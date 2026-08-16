//! agents: talk to running sub-agents (list / send a follow-up message / kill / wait). Messages
//! pushed to a sub-agent's inbox reach it before its next model call, so the parent can steer or
//! redirect a running sub-agent instead of killing and re-spawning it.

use super::{arg_str, Tool, ToolCtx, ToolOutput};
use crate::agent::{SubAgentEnv, SubAgentInfo};
use anyhow::{bail, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

pub struct Agents;

/// One status line: `#id label [status] Ns, N tool calls — task (truncated)`.
pub fn status_line(info: &SubAgentInfo) -> String {
    let status = info.status.lock().unwrap().clone();
    let end = info.finished.lock().unwrap().unwrap_or_else(std::time::Instant::now);
    let secs = end.saturating_duration_since(info.started).as_secs();
    let calls = info.tool_calls.load(std::sync::atomic::Ordering::Relaxed);
    let task: String = info.task.lines().next().unwrap_or("").chars().take(80).collect();
    let ell = if info.task.chars().count() > 80 || info.task.contains('\n') { "…" } else { "" };
    format!("#{} {} [{}] {}s, {} tool calls — {}{}", info.id, info.label, status, secs, calls, task, ell)
}

fn find(env: &SubAgentEnv, id: usize) -> Result<Arc<SubAgentInfo>> {
    let all = env.list();
    match all.iter().find(|a| a.id == id) {
        Some(a) => Ok(a.clone()),
        None => bail!("no sub-agent #{id} (known: {})", if all.is_empty() { "none".to_string() } else { all.iter().map(|a| format!("#{}", a.id)).collect::<Vec<_>>().join(", ") }),
    }
}

fn arg_id(args: &Value) -> Result<usize> {
    match args.get("id") {
        Some(v) if v.is_u64() => Ok(v.as_u64().unwrap() as usize),
        Some(v) if v.is_string() => v.as_str().unwrap().trim_start_matches('#').parse::<usize>().map_err(|_| anyhow::anyhow!("id must be a sub-agent number")),
        _ => bail!("missing required parameter: id"),
    }
}

/// Wait until `targets` are all finished or `timeout` elapses; returns their status lines.
pub async fn wait_for(targets: &[Arc<SubAgentInfo>], timeout: Duration) -> (bool, String) {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut timed_out = false;
    while targets.iter().any(|a| a.running()) {
        if tokio::time::Instant::now() >= deadline { timed_out = true; break; }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    (timed_out, targets.iter().map(|a| status_line(a)).collect::<Vec<_>>().join("\n"))
}

#[async_trait]
impl Tool for Agents {
    fn name(&self) -> &'static str { "agents" }
    fn description(&self) -> &'static str { "Manage running sub-agents started with spawn_agent: list them, send a follow-up message to a running one (delivered before its next model call — use to steer, redirect or hand it new info instead of killing and re-spawning), kill one, or wait for one/all to finish (useful when several sub-agents run in parallel)." }
    fn read_only(&self) -> bool { false }
    fn parallel_safe(&self) -> bool { true }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{
            "action":{"type":"string","enum":["list","send","kill","wait"]},
            "id":{"type":"integer","description":"sub-agent number (from list); required for send/kill, optional for wait (default: all running)"},
            "message":{"type":"string","description":"send: the message to deliver to the running sub-agent"},
            "timeout_secs":{"type":"integer","description":"wait: give up after this many seconds (default 120)"}
        },"required":["action"]})
    }
    async fn call(&self, args: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let Some(env) = &ctx.subagent else { bail!("agents is not available here (sub-agents cannot manage other agents)") };
        let action = arg_str(&args, "action")?;
        match action {
            "list" => {
                let all = env.list();
                if all.is_empty() { return Ok("no sub-agents have been spawned yet".into()); }
                Ok(all.iter().map(|a| status_line(a)).collect::<Vec<_>>().join("\n").into())
            }
            "send" => {
                let info = find(env, arg_id(&args)?)?;
                let message = arg_str(&args, "message")?.trim();
                if message.is_empty() { bail!("message is empty"); }
                if !info.running() { bail!("sub-agent #{} {} has finished ({}) — spawn a new agent instead", info.id, info.label, info.status.lock().unwrap()); }
                info.inbox.push("message from parent agent", message);
                Ok(format!("delivered to #{} {} (it will see it before its next model call; {} message(s) pending)", info.id, info.label, info.inbox.len()).into())
            }
            "kill" => {
                let info = find(env, arg_id(&args)?)?;
                if !info.running() { return Ok(format!("sub-agent #{} {} already finished ({})", info.id, info.label, info.status.lock().unwrap()).into()); }
                info.kill();
                Ok(format!("kill requested for #{} {}", info.id, info.label).into())
            }
            "wait" => {
                let timeout = Duration::from_secs(args.get("timeout_secs").and_then(|v| v.as_u64()).unwrap_or(120));
                let targets: Vec<Arc<SubAgentInfo>> = if args.get("id").is_some() { vec![find(env, arg_id(&args)?)?] } else { env.list().into_iter().filter(|a| a.running()).collect() };
                if targets.is_empty() { return Ok("no running sub-agents".into()); }
                let (timed_out, lines) = wait_for(&targets, timeout).await;
                Ok(if timed_out { format!("timed out after {}s; still running:\n{lines}", timeout.as_secs()) } else { format!("finished:\n{lines}") }.into())
            }
            other => bail!("unknown action '{other}' (list|send|kill|wait)"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env() -> Arc<SubAgentEnv> {
        let cfg: crate::config::LlmConfig = serde_json::from_value(json!({"base_url":"http://127.0.0.1:1","model":"test"})).unwrap();
        let client = crate::llm::Client::new(&cfg).unwrap();
        let policy = Arc::new(crate::permissions::Policy::new(Default::default(), std::path::Path::new("/tmp")));
        let approver: Arc<dyn crate::permissions::Approver> = Arc::new(crate::permissions::AutoApprover { yes: true });
        let sink: Arc<dyn crate::events::Sink> = Arc::new(crate::events::JsonlSink);
        Arc::new(SubAgentEnv::new(client, crate::tools::Registry::defaults(false), policy, approver, sink, 1000, false))
    }
    fn ctx(env: &Arc<SubAgentEnv>) -> ToolCtx { let mut c = ToolCtx::basic(std::env::temp_dir()); c.subagent = Some(env.clone()); c }

    #[test]
    fn status_line_format() {
        let e = env();
        let info = e.register("↳1".into(), "do the thing\nmore lines".into());
        info.tool_calls.store(3, std::sync::atomic::Ordering::Relaxed);
        let s = status_line(&info);
        assert!(s.starts_with("#1 ↳1 [running] 0s, 3 tool calls — do the thing…"), "{s}");
        *info.finished.lock().unwrap() = Some(std::time::Instant::now());
        *info.status.lock().unwrap() = "done".into();
        assert!(status_line(&info).contains("[done]"));
    }

    #[tokio::test]
    async fn not_available_without_env() {
        let c = ToolCtx::basic(std::env::temp_dir());
        assert!(Agents.call(json!({"action":"list"}), &c).await.unwrap_err().to_string().contains("not available"));
    }

    #[tokio::test]
    async fn list_send_kill_wait() {
        let e = env();
        let c = ctx(&e);
        assert!(Agents.call(json!({"action":"list"}), &c).await.unwrap().text.contains("no sub-agents"));
        let info = e.register("↳1".into(), "task one".into());
        let out = Agents.call(json!({"action":"list"}), &c).await.unwrap().text;
        assert!(out.contains("#1 ↳1 [running]") && out.contains("task one"), "{out}");
        // send → inbox
        let out = Agents.call(json!({"action":"send","id":1,"message":"focus on tests"}), &c).await.unwrap().text;
        assert!(out.contains("delivered to #1"), "{out}");
        let items = info.inbox.drain();
        assert_eq!(items.len(), 1); assert_eq!(items[0].source, "message from parent agent"); assert_eq!(items[0].text, "focus on tests");
        assert!(Agents.call(json!({"action":"send","id":9,"message":"x"}), &c).await.is_err());
        // wait with timeout while running
        let out = Agents.call(json!({"action":"wait","id":1,"timeout_secs":0}), &c).await.unwrap().text;
        assert!(out.starts_with("timed out"), "{out}");
        // kill → cancelling
        let out = Agents.call(json!({"action":"kill","id":"#1"}), &c).await.unwrap().text;
        assert!(out.contains("kill requested"), "{out}");
        assert!(info.cancel.load(std::sync::atomic::Ordering::Relaxed));
        assert_eq!(*info.status.lock().unwrap(), "cancelling");
        // finish it → send errors, wait returns immediately
        *info.finished.lock().unwrap() = Some(std::time::Instant::now());
        *info.status.lock().unwrap() = "killed".into();
        assert!(Agents.call(json!({"action":"send","id":1,"message":"x"}), &c).await.unwrap_err().to_string().contains("finished"));
        let out = Agents.call(json!({"action":"wait"}), &c).await.unwrap().text;
        assert!(out.contains("no running sub-agents"), "{out}");
        let out = Agents.call(json!({"action":"wait","id":1}), &c).await.unwrap().text;
        assert!(out.starts_with("finished:") && out.contains("[killed]"), "{out}");
    }
}
