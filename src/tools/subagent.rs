//! spawn_agent: delegate a self-contained task to a nested agent with a fresh context. Several
//! spawn_agent calls in one turn run in parallel. Depth-limited to 1 (sub-agents cannot spawn).

use super::{arg_str, Tool, ToolCtx, ToolOutput};
use anyhow::{bail, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct SpawnAgent;

#[async_trait]
impl Tool for SpawnAgent {
    fn name(&self) -> &'static str { "spawn_agent" }
    fn description(&self) -> &'static str { "Delegate a self-contained sub-task to a fresh agent (own context window, same tools and permissions) and get back its final report. Use for independent chunks of work (research a question, fix one module, run and analyze tests) — issue several spawn_agent calls in ONE turn to run them in parallel. Give complete instructions: the sub-agent does not see this conversation." }
    fn parallel_safe(&self) -> bool { true }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{
            "task":{"type":"string","description":"full, self-contained instructions incl. what to return"},
            "workdir":{"type":"string","description":"optional sub-directory to work in (default: current workdir)"},
            "max_turns":{"type":"integer","description":"default 25"},
            "read_only":{"type":"boolean","description":"true = the sub-agent may not modify files (research/analysis)"},
            "isolation":{"type":"string","enum":["none","worktree"],"description":"worktree = run in a fresh git worktree (own branch wt/agent-N); the report tells you the path/branch to merge from"}
        },"required":["task"]})
    }
    async fn call(&self, args: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let Some(env) = &ctx.subagent else { bail!("spawn_agent is not available in this context (nested sub-agents are not allowed)") };
        let task = arg_str(&args, "task")?.to_string();
        let mut workdir = match args.get("workdir").and_then(|v| v.as_str()) { Some(w) => ctx.resolve(w)?, None => ctx.workdir.clone() };
        if !workdir.is_dir() { bail!("workdir does not exist: {}", workdir.display()); }
        let mut wt_note = String::new();
        if args.get("isolation").and_then(|v| v.as_str()) == Some("worktree") {
            let name = format!("agent-{}", env.next_label());
            workdir = crate::worktree::create(&workdir, &name, None, None)?;
            wt_note = format!("\n[worktree '{name}' at {} — branch wt/{name}; merge or cherry-pick from it, then `worktree remove {{name:\"{name}\"}}`]", workdir.display());
        }
        let max_turns = args.get("max_turns").and_then(|v| v.as_u64()).unwrap_or(25) as usize;
        let read_only = args.get("read_only").and_then(|v| v.as_bool()).unwrap_or(false);
        let registry = env.registry.without("spawn_agent");
        let label = env.next_label();
        let info = env.register(format!("↳{label}"), task.clone());
        let sub_ctx = ToolCtx { workdir: workdir.clone(), timeout: ctx.timeout, max_output: ctx.max_output, net: ctx.net.clone(), memory: ctx.memory.clone(), subagent: None, redact_secrets: ctx.redact_secrets, hooks: ctx.hooks.clone(), todos: ctx.todos.clone(), lsp_servers: ctx.lsp_servers.clone(), extra_roots: ctx.extra_roots.clone(), approver: ctx.approver.clone(), inbox: info.inbox.clone(), cancel: Some(info.cancel.clone()), cwd: None };
        let mut pcfg = env.policy.cfg.clone(); pcfg.mode = env.policy.mode();
        if read_only { pcfg.mode = crate::permissions::Mode::Plan; }
        let policy = crate::permissions::Policy::new(pcfg, &workdir);
        let sink = crate::agent::PrefixSink { inner: env.sink.clone(), prefix: format!("↳{label} "), info: Some(info.clone()) };
        let system = crate::agent::system_prompt_with_memory(&workdir.display().to_string(), &registry.names(), Some("You are a SUB-AGENT working on one delegated task. Do exactly the task, then reply with a concise, complete report of results (facts, file paths, what changed, what failed) — the parent agent only sees this report."), ctx.memory.as_ref());
        let finish = |status: &str| { *info.finished.lock().unwrap() = Some(std::time::Instant::now()); *info.status.lock().unwrap() = status.to_string(); if !ctx.hooks.subagent_stop.is_empty() { let h = ctx.hooks.clone(); let wd = ctx.workdir.clone(); let (l, st) = (info.label.clone(), status.to_string()); tokio::spawn(async move { let _ = crate::hooks::run_event(&h, "subagent_stop", &l, serde_json::json!({"label": l, "status": st}), &wd).await; }); } };
        // Claude Code backend: the sub-agent is another headless claude session with its own tool bridge
        if env.client.provider() == crate::llm::Provider::ClaudeCode {
            let policy = std::sync::Arc::new(policy);
            let host = std::sync::Arc::new(crate::mcp_bridge::BridgeHost { registry: registry.clone(), ctx: sub_ctx.clone(), policy: policy.clone(), approver: env.approver.clone(), sink: std::sync::Arc::new(crate::agent::PrefixSink { inner: env.sink.clone(), prefix: format!("↳{label} "), info: Some(info.clone()) }) });
            let session = match crate::claude_code::ClaudeCodeSession::start_with(&workdir, Some(env.client.model()), env.cc_effort.as_deref().or(Some("medium")), &system, host, None).await { Ok(s) => s, Err(e) => { finish("failed to start"); return Err(e); } };
            *info.cc.lock().unwrap() = Some(session.clone());
            let r = session.run_turn(&task, &[], &sink).await;
            session.stop().await;
            match r {
                Ok((text, stats)) => { finish(&format!("done ({} tool calls, {:.0}s)", stats.tool_calls, stats.wall_secs)); Ok(format!("[sub-agent (claude) finished: {} model calls, {} tool calls, {:.0}s, stop={}]{wt_note}\n{}", stats.turns, stats.tool_calls, stats.wall_secs, stats.stop_reason, text).into()) }
                Err(e) => { finish(if info.cancel.load(std::sync::atomic::Ordering::Relaxed) { "killed" } else { "error" }); if info.cancel.load(std::sync::atomic::Ordering::Relaxed) { Ok("[sub-agent killed by the user]".into()) } else { Err(e) } }
            }
        } else {
            let agent = crate::agent::Agent { client: &env.client, registry: &registry, ctx: &sub_ctx, max_turns, context_budget: env.context_budget, sink: &sink, stream: env.stream, policy: &policy, approver: env.approver.as_ref() };
            match agent.run(&system, &task).await {
                Ok((text, stats)) => { finish(&format!("{} ({} tool calls, {:.0}s)", stats.stop_reason, stats.tool_calls, stats.wall_secs)); Ok(format!("[sub-agent finished: {} turns, {} tool calls, {:.0}s, stop={}]{wt_note}\n{}", stats.turns, stats.tool_calls, stats.wall_secs, stats.stop_reason, text).into()) }
                Err(e) => { finish("error"); Err(e) }
            }
        }
    }
}
