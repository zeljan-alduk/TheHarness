//! process: manage background processes started with bash {background:true}.

use super::{Tool, ToolCtx, ToolOutput};
use anyhow::{bail, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct Process;

#[async_trait]
impl Tool for Process {
    fn name(&self) -> &'static str { "process" }
    fn description(&self) -> &'static str { "Manage background processes started with bash {background:true} (dev servers, watchers, long builds): list them, tail a process's log, or kill it." }
    fn parameters(&self) -> Value { json!({"type":"object","properties":{"action":{"type":"string","enum":["list","tail","kill"]},"id":{"type":"integer"},"lines":{"type":"integer","description":"for tail (default 60)"}},"required":["action"]}) }
    async fn call(&self, args: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("list");
        let id = args.get("id").and_then(|v| v.as_u64()).map(|v| v as u32);
        match action {
            "list" => { let l = crate::procs::list(); if l.is_empty() { return Ok("no background processes".into()); } Ok(l.into_iter().map(|(id, pid, cmd, st, secs, log)| format!("#{id} pid {pid} [{st}] {:.0}s  {}  log: {}", secs, crate::llm::truncate_for_log(&cmd, 80), log.display())).collect::<Vec<_>>().join("\n").into()) }
            "tail" => { let Some(id) = id else { bail!("id required") }; let lines = args.get("lines").and_then(|v| v.as_u64()).unwrap_or(60).clamp(1, 10_000) as usize; Ok(crate::sandbox::truncate_middle(&crate::procs::tail(id, lines)?, ctx.max_output).into()) }
            "kill" => { let Some(id) = id else { bail!("id required") }; Ok(crate::procs::kill(id).await?.into()) }
            _ => bail!("unknown action {action}"),
        }
    }
}
