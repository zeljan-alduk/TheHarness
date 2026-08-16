//! The core loop: model -> tool calls -> results -> model, with budgets and context compaction.

use crate::events::{Event, Sink};
use crate::llm::{Client, Content, Delta, Message, Usage};
use crate::tools::{Registry, ToolCtx};
use anyhow::{bail, Result};
use std::sync::Arc;
use std::time::{Duration, Instant};

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
    pub policy: &'a crate::permissions::Policy,
    pub approver: &'a dyn crate::permissions::Approver,
}

/// Precise LLM compaction: summarize everything but the last `keep_last` messages into a dense,
/// exact handoff note (paths, commands, results, decisions, next steps) and replace them with it.
/// Returns (messages removed, summary). Falls back to Err if the model call fails.
/// Context composition as (label, estimated tokens ≈ chars/4) — for the before/after map.
pub fn context_map(msgs: &[Message]) -> Vec<(String, u64)> {
    let (mut sys, mut user, mut asst, mut tool, mut img, mut note) = (0u64, 0u64, 0u64, 0u64, 0u64, 0u64);
    for m in msgs {
        let t = m.text(); let n = (t.chars().count() as u64) / 4;
        match m.role.as_str() {
            "system" => sys += n,
            "user" => { if t.starts_with("[Context compacted") { note += n } else { user += n } if let Some(Content::Parts(p)) = &m.content { img += p.iter().filter(|x| x["type"] == "image_url").count() as u64 * 1200; } }
            "assistant" => { asst += n; if let Some(c) = &m.tool_calls { asst += c.iter().map(|c| c.function.arguments.chars().count() as u64 / 4).sum::<u64>(); } }
            "tool" => tool += n,
            _ => {}
        }
    }
    let mut v = vec![("system".to_string(), sys), ("handoff note".to_string(), note), ("user".to_string(), user), ("assistant".to_string(), asst), ("tool results".to_string(), tool), ("images".to_string(), img)];
    v.retain(|x| x.1 > 0);
    v
}

pub async fn compact_llm(client: &Client, msgs: &mut Vec<Message>, keep_last: usize, focus: Option<&str>) -> Result<(usize, String)> {
    compact_llm_with(client, msgs, keep_last, focus, None).await.map(|(n, s, _, _)| (n, s))
}

/// Compaction with progress + maps. Returns (removed, summary, map_before, map_after).
pub async fn compact_llm_with(client: &Client, msgs: &mut Vec<Message>, keep_last: usize, focus: Option<&str>, sink: Option<&dyn Sink>) -> Result<(usize, String, Vec<(String, u64)>, Vec<(String, u64)>)> {
    if msgs.len() < 6 { bail!("nothing to compact"); }
    let map_before = context_map(msgs);
    let prog = |f: f64, phase: &str| { if let Some(s) = sink { s.emit(&Event::CompactProgress { fraction: f, phase: phase.to_string() }); } };
    prog(0.02, "selecting messages to keep");
    // choose the cut so the kept tail starts at a user message (never splits tool_calls from results)
    let mut cut = msgs.len().saturating_sub(keep_last).max(1);
    while cut > 1 && msgs[cut].role != "user" { cut -= 1; }
    if cut <= 1 { cut = msgs.len().saturating_sub(2).max(1); while cut > 1 && msgs[cut].role == "tool" { cut -= 1; } }
    if cut <= 1 { bail!("nothing to compact"); }
    let old: Vec<Message> = msgs[1..cut].to_vec();
    let old_tokens: u64 = context_map(&old).iter().map(|x| x.1).sum();
    if old_tokens < 1500 { bail!("only ~{old_tokens} tokens would be compacted — too small to gain anything (a handoff note costs a few hundred tokens)"); }
    // size the note to what it replaces: ~1/4 of the compacted content, between 120 and 900 words
    let max_words = (old_tokens / 5).clamp(120, 900);
    let transcript = render_for_summary(&old, 60_000);
    let system_base = "You compact the working context of an autonomous coding agent mid-session. Write a HANDOFF NOTE so the agent can continue seamlessly without the original messages. Be precise and dense; never invent; keep exact file paths, function/identifier names, commands, numbers, URLs, and error messages verbatim. Use this structure with markdown headings:
## Goal & constraints (the user's requests, key phrases verbatim)
## Done so far (files created/edited with paths and what changed; commands run and their outcomes; tests/eval results)
## Key facts & findings (config values, APIs, gotchas, exact errors)
## Decisions & reasons
## Current state & remaining work (ordered next steps; open questions)
## Notes for the user (anything promised or to report)
Output only the note.";
    let system = format!("{system_base}\nHard limit: at most {max_words} words — the note MUST be much shorter than the transcript it replaces; drop detail before exceeding it.");
    let mut user = format!("Transcript to compact ({} messages):

{transcript}", old.len());
    if let Some(f) = focus { user.push_str(&format!("

Focus especially on: {f}")); }
    prog(0.08, &format!("summarizing {} messages (~{} tokens)", old.len(), transcript.chars().count() / 4));
    // stream the note so progress is real. Thinking phase: 5% → 40% (asymptotic in reasoning length);
    // writing phase: 40% → 98% (expected note ≈ 900 words ≈ 5500 chars). Label shows elapsed + tokens.
    let mut got = 0usize; let mut thought = 0usize;
    let expected = (max_words as f64) * 6.0;
    let t0 = Instant::now();
    let mut last_emit = Instant::now();
    let (reply, _) = client.chat_stream(&[Message::system(&system), Message::user(user)], &[], |d| {
        match d { Delta::Content(t) => got += t.chars().count(), Delta::Reasoning(t) => thought += t.chars().count() }
        if last_emit.elapsed() < Duration::from_millis(150) { return; }
        last_emit = Instant::now();
        let secs = t0.elapsed().as_secs();
        if got == 0 { let f = 0.05 + 0.35 * (1.0 - (-(thought as f64) / 3000.0).exp()); prog(f, &format!("thinking about what to keep · {secs}s · ~{} tok", thought / 4)); }
        else { let f = 0.40 + 0.58 * (got as f64 / expected).min(1.0); prog(f, &format!("writing handoff note · {secs}s · ~{} tok", got / 4)); }
    }).await?;
    let summary = reply.text().trim().to_string();
    if summary.chars().count() < 80 { bail!("compaction summary too short"); }
    let note_tokens = (summary.chars().count() / 4) as u64 + 60;
    if note_tokens * 10 >= old_tokens * 8 { bail!("compaction would not shrink the context (note ≈{note_tokens} tokens vs {old_tokens} replaced) — kept the transcript as is"); }
    let mut new_msgs = vec![msgs[0].clone(), Message::user(format!("[Context compacted — handoff note replacing {} earlier messages]\n\n{summary}\n\n[Continue from the state above; the most recent messages follow verbatim.]", old.len()))];
    new_msgs.extend_from_slice(&msgs[cut..]);
    let removed = old.len();
    *msgs = new_msgs;
    let map_after = context_map(msgs);
    prog(1.0, "done");
    Ok((removed, summary, map_before, map_after))
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
    let aux = client.aux();
    match store.reflect(&aux, msgs).await {
        Ok(items) => for (file, section, text) in items { sink.emit(&Event::Memory { file, section, text }); },
        Err(e) => sink.emit(&Event::Error { message: format!("memory reflection skipped: {e:#}") }),
    }
    if let Ok(done) = store.maybe_consolidate(&aux).await { for f in done { sink.emit(&Event::Memory { file: f, section: "consolidated".into(), text: "file was long; merged and de-duplicated".into() }); } }
}

pub fn system_prompt(workdir: &str, tools: &[&str], extra: Option<&str>) -> String {
    system_prompt_with_memory(workdir, tools, extra, None)
}

/// The base system prompt template. If `~/.config/harness/prompts/system.md` exists it is used instead
/// (placeholders: {workdir}, {tools}); it is created from the built-in default on first use so it can be
/// tuned by the user — or proposed by `harness self` and judged by the arbiter.
pub fn base_prompt_template() -> String {
    let default = DEFAULT_PROMPT.to_string();
    let dir = crate::setup::config_dir().join("prompts");
    let p = dir.join("system.md");
    if let Ok(t) = std::fs::read_to_string(&p) { if t.trim().len() > 50 { return t; } }
    let _ = std::fs::create_dir_all(&dir); let _ = std::fs::write(&p, &default);
    default
}

const DEFAULT_PROMPT: &str = "You are an autonomous software engineering agent running locally with a real toolchain.
Working directory: {workdir}
Tools: {tools}

Rules:
- Act, don't ask. The user is not present; finish the task end-to-end, then reply with a short summary.
- Explore before editing: list_dir / read_file / grep / glob. Never guess file contents.
- Prefer edit_file for small changes; apply_patch for multi-hunk changes; write_file for new files. Keep edits minimal and idiomatic.
- Verify your work: run the build, tests, diagnostics, or the program itself with bash. If it fails, fix it and re-run.
- The working directory is a git repository. Use `git status`, `git diff`, `git log` freely to understand state, and `git checkout -- <file>` / `git revert` to undo mistakes. Commit when a coherent unit of work is done, with a clear message.
- For multi-step work use the `todo` tool: set the plan up front, keep exactly one item in_progress (todo start / todo next), and mark items done the moment they finish — the user watches this list live. Delegate independent sub-tasks with spawn_agent (several in one turn run in parallel).
- Tool outputs may be truncated in the middle; use offset/limit or grep to see more.
- When done, your final message (with no tool calls) must state what changed and how you verified it.
- Finish decisively: once the task is verified, stop calling tools and answer. Do not re-verify, re-read, or polish beyond what was asked; the user may have queued the next task.";

pub fn system_prompt_with_memory(workdir: &str, tools: &[&str], extra: Option<&str>, memory: Option<&crate::memory::MemoryStore>) -> String {
    let mut s = base_prompt_template().replace("{workdir}", workdir).replace("{tools}", &tools.join(", "));

    if let Some(e) = extra { s.push_str("\n\n"); s.push_str(e); }
    s.push_str("\n\n"); s.push_str(&crate::setup::summary_line());
    if let Some(m) = memory { s.push_str(&m.prompt_block(std::path::Path::new(workdir))); }
    s
}

/// A running/finished sub-agent, visible to the UI and controllable (kill).
pub struct SubAgentInfo {
    pub id: usize,
    pub label: String,
    pub task: String,
    pub started: Instant,
    pub finished: std::sync::Mutex<Option<Instant>>,
    pub status: std::sync::Mutex<String>,
    pub tool_calls: std::sync::atomic::AtomicUsize,
    pub cancel: Arc<std::sync::atomic::AtomicBool>,
    pub cc: std::sync::Mutex<Option<Arc<crate::claude_code::ClaudeCodeSession>>>,
    /// Messages pushed here (attach mode) reach the sub-agent before its next model call.
    pub inbox: Arc<crate::inbox::Inbox>,
}
impl SubAgentInfo {
    pub fn kill(&self) { self.cancel.store(true, std::sync::atomic::Ordering::Relaxed); *self.status.lock().unwrap() = "cancelling".into(); if let Some(cc) = self.cc.lock().unwrap().clone() { tokio::spawn(async move { cc.stop().await; }); } }
    pub fn running(&self) -> bool { self.finished.lock().unwrap().is_none() }
}

/// Shared, owned environment that lets a tool (spawn_agent) start nested agents.
pub struct SubAgentEnv {
    /// Live registry of sub-agents (this env's children).
    pub agents: std::sync::Mutex<Vec<Arc<SubAgentInfo>>>,
    /// Claude Code backend: effort level for spawned claude sessions.
    pub cc_effort: Option<String>,
    pub client: Client,
    pub registry: crate::tools::Registry,
    pub policy: std::sync::Arc<crate::permissions::Policy>,
    pub approver: std::sync::Arc<dyn crate::permissions::Approver>,
    pub sink: std::sync::Arc<dyn Sink>,
    pub context_budget: u64,
    pub stream: bool,
    counter: std::sync::atomic::AtomicUsize,
}
impl SubAgentEnv {
    pub fn new(client: Client, registry: crate::tools::Registry, policy: std::sync::Arc<crate::permissions::Policy>, approver: std::sync::Arc<dyn crate::permissions::Approver>, sink: std::sync::Arc<dyn Sink>, context_budget: u64, stream: bool) -> Self {
        Self { agents: std::sync::Mutex::new(vec![]), cc_effort: None, client, registry, policy, approver, sink, context_budget, stream, counter: std::sync::atomic::AtomicUsize::new(0) }
    }
    pub fn next_label(&self) -> String { format!("{}", self.counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1) }
    pub fn register(&self, label: String, task: String) -> Arc<SubAgentInfo> {
        let id = self.agents.lock().unwrap().len() + 1;
        let info = Arc::new(SubAgentInfo { id, label, task, started: Instant::now(), finished: std::sync::Mutex::new(None), status: std::sync::Mutex::new("running".into()), tool_calls: std::sync::atomic::AtomicUsize::new(0), cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)), cc: std::sync::Mutex::new(None), inbox: Arc::new(crate::inbox::Inbox::new()) });
        self.agents.lock().unwrap().push(info.clone());
        info
    }
    pub fn list(&self) -> Vec<Arc<SubAgentInfo>> { self.agents.lock().unwrap().clone() }
}

/// Forwards a sub-agent's events to the parent sink with tool names prefixed (e.g. "↳1 bash").
pub struct PrefixSink { pub inner: std::sync::Arc<dyn Sink>, pub prefix: String, pub info: Option<Arc<SubAgentInfo>> }
impl Sink for PrefixSink {
    fn emit(&self, e: &Event) {
        match e {
            Event::ToolCall { id, name, args } => { if let Some(i) = &self.info { i.tool_calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed); *i.status.lock().unwrap() = format!("{name}"); } self.inner.emit(&Event::ToolCall { id: format!("{}{id}", self.prefix), name: format!("{}{name}", self.prefix), args: args.clone() }) }
            Event::ToolResult { id, name, result, secs, images } => self.inner.emit(&Event::ToolResult { id: format!("{}{id}", self.prefix), name: format!("{}{name}", self.prefix), result: result.clone(), secs: *secs, images: images.clone() }),
            Event::Assistant { text } => self.inner.emit(&Event::ToolResult { id: format!("{}final", self.prefix), name: format!("{}report", self.prefix), result: text.clone(), secs: 0.0, images: vec![] }),
            Event::Error { message } => self.inner.emit(&Event::Error { message: format!("{}{message}", self.prefix) }),
            Event::Permission { .. } | Event::Memory { .. } | Event::ModelResponse { .. } => self.inner.emit(e),
            _ => {} // reasoning/deltas/turns of sub-agents stay quiet
        }
    }
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
        let mut last_sig: Vec<String> = Vec::new();
        let mut repeats = 0u32;
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
                match compact_llm_with(&self.client.aux(), msgs, 8, None, Some(self.sink)).await {
                    Ok((n, summary, mb, ma)) => { stats.compactions += 1; self.sink.emit(&Event::Compacted { count: n, prompt_tokens: before, summary, map_before: mb, map_after: ma }); }
                    Err(_) => { let mb = context_map(msgs); let n = compact(msgs, 6); if n > 0 { stats.compactions += 1; self.sink.emit(&Event::Compacted { count: n, prompt_tokens: before, summary: String::new(), map_before: mb, map_after: context_map(msgs) }); } }
                }
                last_usage = Usage::default(); // re-measured on the next call
            }
            if let Some(c) = &self.ctx.cancel { if c.load(std::sync::atomic::Ordering::Relaxed) { stats.stop_reason = "cancelled".into(); stats.wall_secs = start.elapsed().as_secs_f64(); self.finish(&stats); return Ok((last_text(msgs).unwrap_or_else(|| "(cancelled by user)".into()), stats)); } }
            if let Some(m) = self.ctx.inbox.take_message() { msgs.push(Message::user(m)); }
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
                if !self.ctx.hooks.on_stop.is_empty() { crate::hooks::run_on_stop(&self.ctx.hooks, &text, &self.ctx.workdir).await; }
                return Ok((text, stats));
            }
            msgs.push(assistant);

            let mut pending_images: Vec<(String, Vec<(String, String)>)> = Vec::new();
            // loop detection: identical call repeated back-to-back
            let sig: Vec<String> = calls.iter().map(|c| format!("{}:{}", c.function.name, c.function.arguments)).collect();
            if sig == last_sig { repeats += 1; } else { repeats = 0; last_sig = sig; }
            let all_read_only = calls.iter().all(|c| self.registry.is_parallel_safe(&c.function.name));
            let mut prepared: Vec<(String, String, String)> = Vec::new(); // (id, name, args)
            for call in &calls {
                stats.tool_calls += 1;
                let id = if call.id.is_empty() { format!("call_{}", stats.tool_calls) } else { call.id.clone() };
                self.sink.emit(&Event::ToolCall { id: id.clone(), name: call.function.name.clone(), args: call.function.arguments.clone() });
                prepared.push((id, call.function.name.clone(), call.function.arguments.clone()));
            }
            // permissions + pre-tool hooks; tools run in the effective context (worktree enter/exit)
            let ectx = self.ctx.effective();
            let ectx = &ectx;
            let mut blocked: Vec<Option<String>> = Vec::new();
            for (_, name, args) in &prepared {
                if !self.ctx.hooks.pre_tool.is_empty() {
                    if let Some(reason) = crate::hooks::run_pre_tool(&self.ctx.hooks, name, args, &ectx.workdir).await { self.sink.emit(&Event::Permission { tool: name.clone(), summary: crate::llm::truncate_for_log(args, 80), decision: format!("blocked by hook: {reason}") }); blocked.push(Some(format!("error: blocked by a pre-tool hook: {reason}"))); continue; }
                }
                let av: serde_json::Value = serde_json::from_str(args).unwrap_or(serde_json::Value::Null);
                let ro = self.registry.is_read_only(name);
                let d = self.policy.check(name, &av, ro);
                let msg = match d {
                    crate::permissions::Decision::Allow => None,
                    crate::permissions::Decision::Deny(r) => { self.sink.emit(&Event::Permission { tool: name.clone(), summary: crate::permissions::Policy::primary_arg(name, &av), decision: format!("denied: {r}") }); Some(format!("error: blocked by permission policy ({r}). Ask the user or choose another approach.")) }
                    crate::permissions::Decision::Ask(r) => {
                        let arg = crate::permissions::Policy::primary_arg(name, &av);
                        let req = crate::permissions::ApprovalRequest { tool: name.clone(), summary: arg.clone(), suggested_rule: crate::permissions::Policy::suggested_rule(name, &arg), reason: r.clone() };
                        self.sink.emit(&Event::Permission { tool: name.clone(), summary: arg.clone(), decision: format!("asking: {r}") });
                        match self.approver.ask(req.clone()).await {
                            crate::permissions::Approval::Once => { self.sink.emit(&Event::Permission { tool: name.clone(), summary: arg, decision: "allowed once".into() }); None }
                            crate::permissions::Approval::Always => { self.policy.allow_always(&req.suggested_rule); self.sink.emit(&Event::Permission { tool: name.clone(), summary: arg, decision: format!("always allowed ({})", req.suggested_rule) }); None }
                            crate::permissions::Approval::Deny => { self.sink.emit(&Event::Permission { tool: name.clone(), summary: arg, decision: "denied by user".into() }); Some("error: the user declined this action. Do not retry it; ask what to do instead or take a different approach.".to_string()) }
                        }
                    }
                };
                blocked.push(msg);
            }
            let outputs: Vec<(crate::tools::ToolOutput, f64)> = if all_read_only && prepared.len() > 1 {
                // independent reads: run concurrently
                let futs = prepared.iter().zip(blocked.iter()).map(|((_, name, args), b)| async move { if b.is_some() { return (crate::tools::ToolOutput::default(), 0.0); } let t0 = Instant::now(); let o = self.registry.call(name, args, ectx).await; (o, t0.elapsed().as_secs_f64()) });
                futures_util::future::join_all(futs).await
            } else {
                let mut v = Vec::new();
                for ((_, name, args), b) in prepared.iter().zip(blocked.iter()) { if b.is_some() { v.push((crate::tools::ToolOutput::default(), 0.0)); continue; } let t0 = Instant::now(); let o = self.registry.call(name, args, ectx).await; v.push((o, t0.elapsed().as_secs_f64())); }
                v
            };
            for (((id, name, _), (out, secs)), block) in prepared.into_iter().zip(outputs).zip(blocked) {
                let out = match block { Some(m) => crate::tools::ToolOutput { text: m, images: vec![] }, None => out };
                self.sink.emit(&Event::ToolResult { id: id.clone(), name: name.clone(), result: out.text.clone(), secs,
                    images: out.images.iter().map(|(m, b)| format!("data:{m};base64,{b}")).collect() });
                msgs.push(Message::tool(id, name.clone(), out.text));
                if !out.images.is_empty() { pending_images.push((name, out.images)); }
            }
            if repeats == 2 {
                msgs.push(Message::user("[harness] You have issued the exact same tool call three times in a row with the same result. Do not repeat it: change approach, or if the task is complete, stop calling tools and give your final answer now."));
            } else if repeats >= 4 {
                self.sink.emit(&Event::Error { message: "stopping: the model is looping on the same tool call".into() });
                stats.stop_reason = "loop".into(); stats.wall_secs = start.elapsed().as_secs_f64(); self.finish(&stats);
                return Ok((last_text(msgs).unwrap_or_else(|| "(stopped: repeated identical tool calls)".into()), stats));
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
