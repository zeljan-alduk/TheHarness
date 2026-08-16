//! The core loop: model -> tool calls -> results -> model, with budgets and context compaction.

use crate::events::{Event, Sink};
use crate::llm::{Client, Content, Delta, Message, Usage};
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
    /// Stream tokens (emits ReasoningDelta/AssistantDelta). Requires a server that supports SSE.
    pub stream: bool,
}

/// Precise LLM compaction: summarize everything but the last `keep_last` messages into a dense,
/// exact handoff note (paths, commands, results, decisions, next steps) and replace them with it.
/// Returns (messages removed, summary). Falls back to Err if the model call fails.
pub async fn compact_llm(client: &Client, msgs: &mut Vec<Message>, keep_last: usize, focus: Option<&str>) -> Result<(usize, String)> {
    if msgs.len() < 6 { bail!("nothing to compact"); }
    // choose the cut so the kept tail starts at a user message (never splits tool_calls from results)
    let mut cut = msgs.len().saturating_sub(keep_last).max(1);
    while cut > 1 && msgs[cut].role != "user" { cut -= 1; }
    if cut <= 1 { cut = msgs.len().saturating_sub(2).max(1); while cut > 1 && msgs[cut].role == "tool" { cut -= 1; } }
    if cut <= 1 { bail!("nothing to compact"); }
    let old: Vec<Message> = msgs[1..cut].to_vec();
    let transcript = render_for_summary(&old, 60_000);
    let system = "You compact the working context of an autonomous coding agent mid-session. Write a HANDOFF NOTE so the agent can continue seamlessly without the original messages. Be precise and dense; never invent; keep exact file paths, function/identifier names, commands, numbers, URLs, and error messages verbatim. Use this structure with markdown headings:
## Goal & constraints (the user's requests, key phrases verbatim)
## Done so far (files created/edited with paths and what changed; commands run and their outcomes; tests/eval results)
## Key facts & findings (config values, APIs, gotchas, exact errors)
## Decisions & reasons
## Current state & remaining work (ordered next steps; open questions)
## Notes for the user (anything promised or to report)
Max ~900 words. Output only the note.";
    let mut user = format!("Transcript to compact ({} messages):

{transcript}", old.len());
    if let Some(f) = focus { user.push_str(&format!("

Focus especially on: {f}")); }
    let (reply, _) = client.chat(&[Message::system(system), Message::user(user)], &[]).await?;
    let summary = reply.text().trim().to_string();
    if summary.chars().count() < 80 { bail!("compaction summary too short"); }
    let mut new_msgs = vec![msgs[0].clone(), Message::user(format!("[Context compacted — handoff note replacing {} earlier messages]

{summary}

[Continue from the state above; the most recent messages follow verbatim.]", old.len()))];
    new_msgs.extend_from_slice(&msgs[cut..]);
    let removed = old.len();
    *msgs = new_msgs;
    Ok((removed, summary))
}

fn render_for_summary(msgs: &[Message], max_chars: usize) -> String {
    let mut out = String::new();
    for m in msgs {
        match m.role.as_str() {
            "user" => out.push_str(&format!("### USER
{}
", m.text())),
            "assistant" => {
                let t = m.text(); if !t.trim().is_empty() { out.push_str(&format!("### ASSISTANT
{}
", t)); }
                if let Some(calls) = &m.tool_calls { for c in calls { out.push_str(&format!("### CALL {}
{}
", c.function.name, crate::llm::truncate_for_log(&c.function.arguments, 1500))); } }
            }
            "tool" => out.push_str(&format!("### RESULT {}
{}
", m.name.clone().unwrap_or_default(), crate::llm::truncate_for_log(&m.text(), 2500))),
            _ => {}
        }
    }
    let n = out.chars().count();
    if n > max_chars { let head: String = out.chars().take(max_chars / 3).collect(); let tail: String = out.chars().skip(n - max_chars * 2 / 3).collect(); return format!("{head}
…[{} chars elided]…
{tail}", n - max_chars); }
    out
}

/// After a finished turn: reflect into BRAIN/MEMORY/WORKFLOWS and consolidate if files got long.
pub async fn reflect_after_run(client: &Client, store: &crate::memory::MemoryStore, msgs: &[Message], stats: &RunStats, sink: &dyn Sink) {
    if !store.cfg.auto_reflect || stats.tool_calls < store.cfg.reflect_min_tool_calls || stats.stop_reason != "done" { return; }
    match store.reflect(client, msgs).await {
        Ok(items) => for (file, section, text) in items { sink.emit(&Event::Memory { file, section, text }); },
        Err(e) => sink.emit(&Event::Error { message: format!("memory reflection skipped: {e:#}") }),
    }
    if let Ok(done) = store.maybe_consolidate(client).await { for f in done { sink.emit(&Event::Memory { file: f, section: "consolidated".into(), text: "file was long; merged and de-duplicated".into() }); } }
}

pub fn system_prompt(workdir: &str, tools: &[&str], extra: Option<&str>) -> String {
    system_prompt_with_memory(workdir, tools, extra, None)
}

pub fn system_prompt_with_memory(workdir: &str, tools: &[&str], extra: Option<&str>, memory: Option<&crate::memory::MemoryStore>) -> String {
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
    s.push_str("\n\n"); s.push_str(&crate::setup::summary_line());
    if let Some(m) = memory { s.push_str(&m.prompt_block(std::path::Path::new(workdir))); }
    s
}

/// If a run was interrupted after the model asked for tools but before results were appended,
/// the transcript is invalid for the next request. Patch it with stub results.
pub fn repair_dangling(msgs: &mut Vec<Message>) {
    let mut i = 0;
    while i < msgs.len() {
        if msgs[i].role == "assistant" {
            if let Some(calls) = msgs[i].tool_calls.clone() {
                let mut j = i + 1;
                let mut have = std::collections::HashSet::new();
                while j < msgs.len() && msgs[j].role == "tool" { if let Some(id) = &msgs[j].tool_call_id { have.insert(id.clone()); } j += 1; }
                let mut insert_at = j;
                for c in calls {
                    if !have.contains(&c.id) {
                        msgs.insert(insert_at, Message::tool(c.id.clone(), c.function.name.clone(), "[interrupted by user before this tool ran]"));
                        insert_at += 1;
                    }
                }
                i = insert_at;
                continue;
            }
        }
        i += 1;
    }
}

impl<'a> Agent<'a> {
    /// One-shot: fresh transcript, run to completion.
    pub async fn run(&self, system: &str, task: &str) -> Result<(String, RunStats)> {
        let mut msgs = Vec::new();
        self.run_turn(&mut msgs, system, task).await
    }

    /// Multi-turn: append `task` as a user turn to an existing transcript (created if empty) and run
    /// until the model stops calling tools. The transcript is left ready for the next turn.
    pub async fn run_turn(&self, msgs: &mut Vec<Message>, system: &str, task: &str) -> Result<(String, RunStats)> {
        self.run_turn_message(msgs, system, Message::user(task)).await
    }

    /// Like `run_turn` but the user turn is a prepared message (e.g. text + image parts).
    pub async fn run_turn_message(&self, msgs: &mut Vec<Message>, system: &str, user: Message) -> Result<(String, RunStats)> {
        let start = Instant::now();
        if msgs.is_empty() { msgs.push(Message::system(system)); } else if msgs[0].role == "system" { msgs[0] = Message::system(system); }
        repair_dangling(msgs);
        msgs.push(user);
        let defs = self.registry.defs();
        let mut stats = RunStats::default();
        let mut last_usage = Usage::default();
        let mut truncations = 0u32;
        let mut retries = 0u32;
        self.sink.emit(&Event::RunStarted { model: self.client.model().to_string(), workdir: self.ctx.workdir.display().to_string(), tools: self.registry.names().iter().map(|s| s.to_string()).collect() });

        loop {
            if stats.turns >= self.max_turns {
                stats.stop_reason = "max_turns".into();
                stats.wall_secs = start.elapsed().as_secs_f64();
                self.finish(&stats);
                return Ok((last_text(msgs).unwrap_or_else(|| "(stopped: max turns reached)".into()), stats));
            }
            if last_usage.prompt_tokens > self.context_budget {
                let before = last_usage.prompt_tokens;
                match compact_llm(self.client, msgs, 8, None).await {
                    Ok((n, summary)) => { stats.compactions += 1; self.sink.emit(&Event::Compacted { count: n, prompt_tokens: before, summary }); }
                    Err(_) => { let n = compact(msgs, 6); if n > 0 { stats.compactions += 1; self.sink.emit(&Event::Compacted { count: n, prompt_tokens: before, summary: String::new() }); } }
                }
                last_usage = Usage::default(); // re-measured on the next call
            }
            stats.turns += 1;
            self.sink.emit(&Event::Turn { n: stats.turns });
            let call_start = Instant::now();
            let mut first_token: Option<Instant> = None;
            let res = if self.stream {
                self.client.chat_stream(msgs, &defs, |d| {
                    if first_token.is_none() { first_token = Some(Instant::now()); }
                    match d {
                        Delta::Reasoning(t) => self.sink.emit(&Event::ReasoningDelta { text: t }),
                        Delta::Content(t) => self.sink.emit(&Event::AssistantDelta { text: t }),
                    }
                }).await
            } else {
                self.client.chat(msgs, &defs).await
            };
            let (msg, usage) = match res {
                Ok(x) => x,
                Err(e) => {
                    // Transient server/network trouble (local servers restart, stall, hiccup): retry with backoff.
                    let text = format!("{e:#}");
                    let transient = !text.contains("returned 4") || text.contains("429");
                    if transient && retries < 3 {
                        retries += 1;
                        self.sink.emit(&Event::Error { message: format!("model call failed ({}); retry {retries}/3 in {}s", crate::llm::truncate_for_log(&text, 160), 5 * retries) });
                        tokio::time::sleep(std::time::Duration::from_secs(5 * retries as u64)).await;
                        stats.turns -= 1;
                        continue;
                    }
                    self.sink.emit(&Event::Error { message: text });
                    return Err(e);
                }
            };
            retries = 0;
            let secs = call_start.elapsed().as_secs_f64();
            self.sink.emit(&Event::ModelResponse {
                prompt_tokens: usage.prompt_tokens, completion_tokens: usage.completion_tokens,
                ttft_secs: first_token.map(|t| (t - call_start).as_secs_f64()).unwrap_or(secs), secs,
                tool_calls: msg.tool_calls.as_ref().map(|c| c.len()).unwrap_or(0),
            });
            stats.prompt_tokens += usage.prompt_tokens;
            stats.completion_tokens += usage.completion_tokens;
            last_usage = usage;

            if let Some(r) = &msg.reasoning_content { if !r.trim().is_empty() { self.sink.emit(&Event::Reasoning { text: r.clone() }); } }
            { let c = msg.text(); if !c.trim().is_empty() { self.sink.emit(&Event::Assistant { text: c }); } }

            let calls = msg.tool_calls.clone().unwrap_or_default();
            let mut assistant = msg.clone();
            assistant.reasoning_content = None;
            // A turn cut off by max_tokens with no tool calls is not "done": nudge and continue (bounded).
            let truncated = assistant.text().contains("[output truncated by max_tokens]");
            if calls.is_empty() && truncated && truncations < 3 {
                truncations += 1;
                self.sink.emit(&Event::Error { message: format!("model output hit max_tokens with no tool call (attempt {truncations}/3) — asking it to continue") });
                msgs.push(assistant);
                msgs.push(Message::user("Your previous message was cut off by the output limit before you called any tool. Keep reasoning brief and act: call the next tool now (write_file / edit_file / bash). Do not restate the whole plan."));
                continue;
            }
            if calls.is_empty() {
                let text = assistant.text();
                msgs.push(assistant);
                stats.stop_reason = "done".into();
                stats.wall_secs = start.elapsed().as_secs_f64();
                if text.trim().is_empty() { self.sink.emit(&Event::Error { message: "model returned empty message with no tool calls".into() }); bail!("model returned empty message with no tool calls"); }
                self.finish(&stats);
                return Ok((text, stats));
            }
            msgs.push(assistant);

            let mut pending_images: Vec<(String, Vec<(String, String)>)> = Vec::new();
            for call in calls {
                stats.tool_calls += 1;
                let name = call.function.name.clone();
                let args = call.function.arguments.clone();
                let id = if call.id.is_empty() { format!("call_{}", stats.tool_calls) } else { call.id.clone() };
                self.sink.emit(&Event::ToolCall { id: id.clone(), name: name.clone(), args: args.clone() });
                let t0 = Instant::now();
                let out = self.registry.call(&name, &args, self.ctx).await;
                self.sink.emit(&Event::ToolResult { id: id.clone(), name: name.clone(), result: out.text.clone(), secs: t0.elapsed().as_secs_f64(),
                    images: out.images.iter().map(|(m, b)| format!("data:{m};base64,{b}")).collect() });
                msgs.push(Message::tool(id, name.clone(), out.text));
                if !out.images.is_empty() { pending_images.push((name, out.images)); }
            }
            // Tool results are text-only in the OpenAI protocol; images ride in a follow-up user turn.
            if !pending_images.is_empty() {
                let mut parts = vec![Content::text_part("[harness] image(s) returned by the tool call(s) above:")];
                for (name, imgs) in pending_images {
                    for (mime, b64) in imgs {
                        parts.push(Content::text_part(&format!("(from {name})")));
                        parts.push(Content::image_part(&mime, &b64));
                    }
                }
                msgs.push(Message::user_parts(parts));
            }
        }
    }

    fn finish(&self, s: &RunStats) {
        self.sink.emit(&Event::RunFinished { stop_reason: s.stop_reason.clone(), turns: s.turns, tool_calls: s.tool_calls, prompt_tokens: s.prompt_tokens, completion_tokens: s.completion_tokens, wall_secs: s.wall_secs });
    }
}

fn last_text(msgs: &[Message]) -> Option<String> {
    msgs.iter().rev().find(|m| m.role == "assistant").map(|m| m.text()).filter(|c| !c.trim().is_empty())
}

/// Replace the content of tool results older than the last `keep_last` tool messages with a stub.
fn compact(msgs: &mut [Message], keep_last: usize) -> usize {
    let idxs: Vec<usize> = msgs.iter().enumerate().filter(|(_, m)| m.role == "tool").map(|(i, _)| i).collect();
    if idxs.len() <= keep_last { return 0; }
    let mut n = 0;
    for &i in &idxs[..idxs.len() - keep_last] {
        let m = &mut msgs[i];
        let text = m.text();
        if text.len() > 200 {
            let head: String = text.chars().take(120).collect();
            m.content = Some(Content::Text(format!("{head}… [older tool output compacted; re-run the tool if you need it]")));
            n += 1;
        }
        // Drop image payloads that follow an old tool result (they are the expensive part).
        if i + 1 < msgs.len() && msgs[i + 1].role == "user" && matches!(msgs[i + 1].content, Some(Content::Parts(_))) {
            msgs[i + 1].content = Some(Content::Text("[harness] earlier image(s) removed from context; call view_image again if needed.".into()));
            n += 1;
        }
    }
    n
}
