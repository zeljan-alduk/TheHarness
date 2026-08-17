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
            "isolation":{"type":"string","enum":["none","worktree"],"description":"worktree = run in a fresh git worktree (own branch wt/agent-N); the report tells you the path/branch to merge from"},
            "subagent_type":{"type":"string","description":"name of a custom agent (see 'Custom agents' in your system prompt) whose prompt, tools, model and permission mode this sub-agent should use, or \"fork\" to give it a copy of THIS conversation as context (no need to re-explain anything)"},
            "background":{"type":"boolean","description":"return immediately and keep working while it runs; its report arrives in your inbox, or fetch it with agents {action:\"report\", id}"},
            "model":{"type":"string","description":"run this sub-agent on a different model"}
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
            if ctx.hooks.any("worktree_create") { let _ = crate::hooks::run_event(&ctx.hooks, "worktree_create", &name, json!({"name": name, "path": workdir.display().to_string(), "branch": format!("wt/{name}")}), &ctx.workdir).await; }
            wt_note = format!("\n[worktree '{name}' at {} — branch wt/{name}; merge or cherry-pick from it, then `worktree remove {{name:\"{name}\"}}`]", workdir.display());
        }
        // `fork` is not a file-defined agent: it means "inherit this conversation"
        let requested = args.get("subagent_type").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).map(|s| s.to_string());
        let fork = requested.as_deref() == Some("fork");
        // named custom agent (.harness/agents/*.md, .claude/agents/*.md, …)
        let def = match requested.as_deref().filter(|_| !fork) {
            Some(t) => {
                let Some(d) = crate::agentdefs::find(&ctx.workdir, t) else {
                    let mut names: Vec<String> = crate::agentdefs::discover(&ctx.workdir).into_iter().map(|a| a.name).collect();
                    names.push("fork".into());
                    bail!("no agent named '{t}'. Available: {} (add more in .harness/agents/<name>.md)", names.join(", "));
                };
                Some(d)
            }
            None => None,
        };
        if let Some(d) = &def { if d.isolation.as_deref() == Some("worktree") && wt_note.is_empty() {
            let name = format!("agent-{}", env.next_label());
            workdir = crate::worktree::create(&workdir, &name, None, None)?;
            wt_note = format!("\n[worktree '{name}' at {} — branch wt/{name}; merge or cherry-pick from it, then `worktree remove {{name:\"{name}\"}}`]", workdir.display());
        } }
        let max_turns = args.get("max_turns").and_then(|v| v.as_u64()).map(|n| n as usize).or(def.as_ref().and_then(|d| d.max_turns)).unwrap_or(25);
        let read_only = args.get("read_only").and_then(|v| v.as_bool()).unwrap_or(false);
        let registry = match &def { Some(d) => env.registry.only(&d.filter_tools(&env.registry.names())).without("spawn_agent"), None => env.registry.without("spawn_agent") };
        let label = env.next_label();
        let info = env.register(format!("↳{label}"), task.clone());
        let background = args.get("background").and_then(|v| v.as_bool()).unwrap_or(false);
        info.background.store(background, std::sync::atomic::Ordering::Relaxed);
        let mut pcfg = env.policy.cfg.clone(); pcfg.mode = env.policy.mode();
        if let Some(m) = def.as_ref().and_then(|d| d.permission_mode) { pcfg.mode = m; }
        if read_only { pcfg.mode = crate::permissions::Mode::Plan; }
        let policy = std::sync::Arc::new(crate::permissions::Policy::child_of(env.policy.clone(), pcfg, &workdir));
        let sink: std::sync::Arc<dyn crate::events::Sink> = std::sync::Arc::new(crate::agent::PrefixSink { inner: env.sink.clone(), prefix: format!("↳{label} "), info: Some(info.clone()) });
        // a sub-agent may delegate further, until the environment's max depth
        let child_env = env.child(policy.clone(), sink.clone());
        let sub_ctx = ToolCtx { workdir: workdir.clone(), timeout: ctx.timeout, max_output: ctx.max_output, net: ctx.net.clone(), memory: ctx.memory.clone(), subagent: child_env, redact_secrets: ctx.redact_secrets, injection_scan: ctx.injection_scan, hooks: ctx.hooks.clone(), todos: ctx.todos.clone(), lsp_servers: ctx.lsp_servers.clone(), format: ctx.format.clone(), extra_roots: ctx.extra_roots.clone(), approver: ctx.approver.clone(), inbox: info.inbox.clone(), cancel: Some(info.cancel.clone()), cwd: None, session_id: ctx.session_id.clone() };
        let base_extra = crate::agent::prompt_file("subagent", "You are a SUB-AGENT working on one delegated task. Do exactly the task, then reply with a concise, complete report of results (facts, file paths, what changed, what failed) — the parent agent only sees this report.");
        let base_extra = base_extra.trim();
        let mut extra = match &def { Some(d) => format!("{base_extra}\n\n# Your role: {}\n{}", d.name, d.prompt), None => base_extra.to_string() };
        if fork { extra.push_str("\n\nYou are a FORK of the parent agent: the conversation so far is your context. Continue from it."); }
        let system = crate::agent::system_prompt_with_memory(&workdir.display().to_string(), &registry.names(), Some(&extra), ctx.memory.as_ref());
        let client = match args.get("model").and_then(|v| v.as_str()).map(|s| s.to_string()).or_else(|| def.as_ref().and_then(|d| d.model.clone())) {
            Some(m) => env.client.with_model(&m), None => env.client.clone(),
        };
        // `fork` starts from a copy of the parent's transcript instead of an empty one
        let prior: Vec<crate::llm::Message> = if fork { env.forked_transcript() } else { Vec::new() };
        let hooks = ctx.hooks.clone();
        let parent_inbox = ctx.inbox.clone();
        let (env2, info2, workdir2) = (env.clone(), info.clone(), workdir.clone());
        let cc_effort = env.cc_effort.clone();
        let (budget, stream, approver) = (env.context_budget, env.stream, env.approver.clone());
        let hooks_wd = ctx.workdir.clone();

        let job = async move {
            let finish = |status: &str, report: Option<&str>| {
                *info2.finished.lock().unwrap() = Some(std::time::Instant::now());
                *info2.status.lock().unwrap() = status.to_string();
                if let Some(r) = report { *info2.report.lock().unwrap() = Some(r.to_string()); }
                if hooks.any("subagent_stop") {
                    let (h, wd, l, st) = (hooks.clone(), hooks_wd.clone(), info2.label.clone(), status.to_string());
                    tokio::spawn(async move { let _ = crate::hooks::run_event(&h, "subagent_stop", &l, json!({"label": l, "status": st}), &wd).await; });
                }
            };
            // ACP backend: the sub-agent is another session of the external agent
            if let Some(cmd) = client.acp_command() {
                let session = match crate::acp_client::AcpSession::start(&cmd, &workdir2, policy.clone(), approver.clone()).await {
                    Ok(s) => s, Err(e) => { finish("failed to start", None); return Err(e); }
                };
                let r = session.run_turn(&task, sink.clone()).await;
                session.stop().await;
                return match r {
                    Ok((text, stats)) => {
                        let report = format!("[sub-agent (acp {cmd}) finished: {} tool calls, {:.0}s, stop={}]{wt_note}\n{}", stats.tool_calls, stats.wall_secs, stats.stop_reason, text);
                        finish(&format!("done ({} tool calls, {:.0}s)", stats.tool_calls, stats.wall_secs), Some(&report));
                        Ok(report)
                    }
                    Err(e) => { finish("error", None); Err(e) }
                };
            }
            // Claude Code backend: the sub-agent is another headless claude session with its own tool bridge
            if client.provider() == crate::llm::Provider::ClaudeCode {
                let host = std::sync::Arc::new(crate::mcp_bridge::BridgeHost { registry: registry.clone(), ctx: sub_ctx.clone(), policy: policy.clone(), approver: approver.clone(), sink: sink.clone() });
                let session = match crate::claude_code::ClaudeCodeSession::start_with(&workdir2, Some(client.model()), cc_effort.as_deref().or(Some("medium")), &system, host, None).await {
                    Ok(s) => s, Err(e) => { finish("failed to start", None); return Err(e); }
                };
                *info2.cc.lock().unwrap() = Some(session.clone());
                let r = session.run_turn(&task, &[], sink.as_ref()).await;
                session.stop().await;
                match r {
                    Ok((text, stats)) => {
                        let report = format!("[sub-agent (claude) finished: {} model calls, {} tool calls, {:.0}s, stop={}]{wt_note}\n{}", stats.turns, stats.tool_calls, stats.wall_secs, stats.stop_reason, text);
                        finish(&format!("done ({} tool calls, {:.0}s)", stats.tool_calls, stats.wall_secs), Some(&report));
                        Ok(report)
                    }
                    Err(e) => {
                        let killed = info2.cancel.load(std::sync::atomic::Ordering::Relaxed);
                        finish(if killed { "killed" } else { "error" }, killed.then_some("[sub-agent killed by the user]"));
                        if killed { Ok("[sub-agent killed by the user]".to_string()) } else { Err(e) }
                    }
                }
            } else {
                let agent = crate::agent::Agent { client: &client, registry: &registry, ctx: &sub_ctx, max_turns, context_budget: budget, sink: sink.as_ref(), stream, policy: &policy, approver: approver.as_ref() };
                let mut msgs = prior;
                let out = if msgs.is_empty() { agent.run(&system, &task).await } else { agent.run_turn(&mut msgs, &system, &task).await };
                match out {
                    Ok((text, stats)) => {
                        let report = format!("[sub-agent finished: {} turns, {} tool calls, {:.0}s, stop={}]{wt_note}\n{}", stats.turns, stats.tool_calls, stats.wall_secs, stats.stop_reason, text);
                        finish(&format!("{} ({} tool calls, {:.0}s)", stats.stop_reason, stats.tool_calls, stats.wall_secs), Some(&report));
                        Ok(report)
                    }
                    Err(e) => { finish("error", None); Err(e) }
                }
            }
        };

        if background {
            let id = info.id;
            tokio::spawn(async move {
                let text = match job.await { Ok(t) => t, Err(e) => format!("[sub-agent #{id} failed: {e:#}]") };
                parent_inbox.push(format!("sub-agent #{id}"), text);
            });
            let _ = env2;
            return Ok(format!("[sub-agent #{} started in the background — keep working; its report will arrive in your inbox, or collect it with agents {{action:\"report\", id:{}}}]", id, id).into());
        }
        job.await.map(|t| t.into())
    }
}
