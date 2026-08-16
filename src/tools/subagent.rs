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
            "read_only":{"type":"boolean","description":"true = the sub-agent may not modify files (research/analysis)"}
        },"required":["task"]})
    }
    async fn call(&self, args: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let Some(env) = &ctx.subagent else { bail!("spawn_agent is not available in this context (nested sub-agents are not allowed)") };
        let task = arg_str(&args, "task")?.to_string();
        let workdir = match args.get("workdir").and_then(|v| v.as_str()) { Some(w) => ctx.resolve(w)?, None => ctx.workdir.clone() };
        if !workdir.is_dir() { bail!("workdir does not exist: {}", workdir.display()); }
        let max_turns = args.get("max_turns").and_then(|v| v.as_u64()).unwrap_or(25) as usize;
        let read_only = args.get("read_only").and_then(|v| v.as_bool()).unwrap_or(false);
        let registry = env.registry.without("spawn_agent");
        let sub_ctx = ToolCtx { workdir: workdir.clone(), timeout: ctx.timeout, max_output: ctx.max_output, net: ctx.net.clone(), memory: ctx.memory.clone(), subagent: None, redact_secrets: ctx.redact_secrets, hooks: ctx.hooks.clone(), todos: ctx.todos.clone() };
        let mut pcfg = env.policy.cfg.clone();
        if read_only { pcfg.mode = crate::permissions::Mode::Plan; }
        let policy = crate::permissions::Policy::new(pcfg, &workdir);
        let sink = crate::agent::PrefixSink { inner: env.sink.clone(), prefix: format!("↳{} ", env.next_label()) };
        let agent = crate::agent::Agent { client: &env.client, registry: &registry, ctx: &sub_ctx, max_turns, context_budget: env.context_budget, sink: &sink, stream: env.stream, policy: &policy, approver: env.approver.as_ref() };
        let system = crate::agent::system_prompt_with_memory(&workdir.display().to_string(), &registry.names(), Some("You are a SUB-AGENT working on one delegated task. Do exactly the task, then reply with a concise, complete report of results (facts, file paths, what changed, what failed) — the parent agent only sees this report."), ctx.memory.as_ref());
        let (text, stats) = agent.run(&system, &task).await?;
        Ok(format!("[sub-agent finished: {} turns, {} tool calls, {:.0}s, stop={}]\n{}", stats.turns, stats.tool_calls, stats.wall_secs, stats.stop_reason, text).into())
    }
}
