//! The core loop: model -> tool calls -> results -> model, with budgets and context compaction.

use crate::events::{Event, Sink};
use crate::llm::{Client, Message, Usage};
use crate::tools::{Registry, ToolCtx};
use anyhow::{bail, Result};
use std::time::Instant;

#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct RunStats {
    pub turns: usize,
    pub tool_calls: usize,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub wall_secs: f64,
    pub compactions: usize,
    pub stop_reason: String,
}

pub struct Agent<'a> {
    pub client: &'a Client,
    pub registry: &'a Registry,
    pub ctx: &'a ToolCtx,
    pub max_turns: usize,
    pub context_budget: u64,
    pub sink: &'a dyn Sink,
}

pub fn system_prompt(workdir: &str, tools: &[&str], extra: Option<&str>) -> String {
    let mut s = format!(
"You are an autonomous software engineering agent running locally with a real toolchain.
Working directory: {workdir}
Tools: {tools}

Rules:
- Act, don't ask. The user is not present; finish the task end-to-end, then reply with a short summary.
- Explore before editing: list_dir / read_file / grep via bash. Never guess file contents.
- Prefer edit_file for small changes; write_file for new files. Keep edits minimal and idiomatic.
- Verify your work: run the build, tests, or the program itself with bash. If it fails, fix it and re-run.
- The working directory is a git repository. Use `git status`, `git diff`, `git log` freely to understand state, and `git checkout -- <file>` / `git revert` to undo mistakes. Commit when a coherent unit of work is done, with a clear message.
- Tool outputs may be truncated in the middle; use offset/limit or grep to see more.
- When done, your final message (with no tool calls) must state what changed and how you verified it.",
        tools = tools.join(", "));
    if let Some(e) = extra { s.push_str("\n\n"); s.push_str(e); }
    s
}

impl<'a> Agent<'a> {
    pub async fn run(&self, system: &str, task: &str) -> Result<(String, RunStats)> {
        let start = Instant::now();
        let mut msgs = vec![Message::system(system), Message::user(task)];
        let defs = self.registry.defs();
        let mut stats = RunStats::default();
        let mut last_usage = Usage::default();
        self.sink.emit(&Event::RunStarted { model: self.client.model().to_string(), workdir: self.ctx.workdir.display().to_string(), tools: self.registry.names().iter().map(|s| s.to_string()).collect() });

        loop {
            if stats.turns >= self.max_turns {
                stats.stop_reason = "max_turns".into();
                stats.wall_secs = start.elapsed().as_secs_f64();
                self.finish(&stats);
                return Ok((last_text(&msgs).unwrap_or_else(|| "(stopped: max turns reached)".into()), stats));
            }
            if last_usage.prompt_tokens > self.context_budget {
                let n = compact(&mut msgs, 6);
                if n > 0 { stats.compactions += 1; self.sink.emit(&Event::Compacted { count: n, prompt_tokens: last_usage.prompt_tokens }); }
            }
            stats.turns += 1;
            self.sink.emit(&Event::Turn { n: stats.turns });
            let (msg, usage) = match self.client.chat(&msgs, &defs).await {
                Ok(x) => x,
                Err(e) => { self.sink.emit(&Event::Error { message: format!("{e:#}") }); return Err(e); }
            };
            stats.prompt_tokens += usage.prompt_tokens;
            stats.completion_tokens += usage.completion_tokens;
            last_usage = usage;

            if let Some(r) = &msg.reasoning_content { if !r.trim().is_empty() { self.sink.emit(&Event::Reasoning { text: r.clone() }); } }
            if let Some(c) = &msg.content { if !c.trim().is_empty() { self.sink.emit(&Event::Assistant { text: c.clone() }); } }

            let calls = msg.tool_calls.clone().unwrap_or_default();
            let mut assistant = msg.clone();
            assistant.reasoning_content = None;
            if calls.is_empty() {
                let text = assistant.content.clone().unwrap_or_default();
                msgs.push(assistant);
                stats.stop_reason = "done".into();
                stats.wall_secs = start.elapsed().as_secs_f64();
                if text.trim().is_empty() { self.sink.emit(&Event::Error { message: "model returned empty message with no tool calls".into() }); bail!("model returned empty message with no tool calls"); }
                self.finish(&stats);
                return Ok((text, stats));
            }
            msgs.push(assistant);

            for call in calls {
                stats.tool_calls += 1;
                let name = call.function.name.clone();
                let args = call.function.arguments.clone();
                let id = if call.id.is_empty() { format!("call_{}", stats.tool_calls) } else { call.id.clone() };
                self.sink.emit(&Event::ToolCall { id: id.clone(), name: name.clone(), args: args.clone() });
                let t0 = Instant::now();
                let result = self.registry.call(&name, &args, self.ctx).await;
                self.sink.emit(&Event::ToolResult { id: id.clone(), name: name.clone(), result: result.clone(), secs: t0.elapsed().as_secs_f64() });
                msgs.push(Message::tool(id, name, result));
            }
        }
    }

    fn finish(&self, s: &RunStats) {
        self.sink.emit(&Event::RunFinished { stop_reason: s.stop_reason.clone(), turns: s.turns, tool_calls: s.tool_calls, prompt_tokens: s.prompt_tokens, completion_tokens: s.completion_tokens, wall_secs: s.wall_secs });
    }
}

fn last_text(msgs: &[Message]) -> Option<String> {
    msgs.iter().rev().find(|m| m.role == "assistant").and_then(|m| m.content.clone()).filter(|c| !c.trim().is_empty())
}

/// Replace the content of tool results older than the last `keep_last` tool messages with a stub.
fn compact(msgs: &mut [Message], keep_last: usize) -> usize {
    let idxs: Vec<usize> = msgs.iter().enumerate().filter(|(_, m)| m.role == "tool").map(|(i, _)| i).collect();
    if idxs.len() <= keep_last { return 0; }
    let mut n = 0;
    for &i in &idxs[..idxs.len() - keep_last] {
        let m = &mut msgs[i];
        let len = m.content.as_ref().map(|c| c.len()).unwrap_or(0);
        if len > 200 {
            let head: String = m.content.as_ref().unwrap().chars().take(120).collect();
            m.content = Some(format!("{head}… [older tool output compacted; re-run the tool if you need it]"));
            n += 1;
        }
    }
    n
}
