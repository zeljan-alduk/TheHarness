//! Structured events emitted by the agent loop. The core never prints; a frontend
//! (CLI today, HTTP/WebSocket + web UI or Tauri later) subscribes and renders.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    RunStarted { model: String, workdir: String, tools: Vec<String> },
    Turn { n: usize },
    /// Streaming increments (only when the client streams). Final `Reasoning`/`Assistant` still follow.
    ReasoningDelta { text: String },
    AssistantDelta { text: String },
    Reasoning { text: String },
    Assistant { text: String },
    ToolCall { id: String, name: String, args: String },
    /// `images` are data: URLs (only for tools that return images, e.g. view_image).
    ToolResult { id: String, name: String, result: String, secs: f64, images: Vec<String> },
    /// Context was compacted. `summary` is the handoff summary that replaced older messages ("" for the cheap fallback).
    Compacted { count: usize, prompt_tokens: u64, summary: String },
    /// One model call finished: exact token counts and timing (ttft = time to first streamed token).
    ModelResponse { prompt_tokens: u64, completion_tokens: u64, ttft_secs: f64, secs: f64, tool_calls: usize },
    RunFinished { stop_reason: String, turns: usize, tool_calls: usize, prompt_tokens: u64, completion_tokens: u64, wall_secs: f64 },
    Error { message: String },
    /// A tool call was blocked or is awaiting approval.
    Permission { tool: String, summary: String, decision: String },
    /// Something was written to persistent memory (by the tool or by reflection).
    Memory { file: String, section: String, text: String },
}

pub trait Sink: Send + Sync {
    fn emit(&self, e: &Event);
}

/// Human-readable rendering to stderr (used by the CLI).
pub struct StderrSink { pub verbose: bool }

impl Sink for StderrSink {
    fn emit(&self, e: &Event) {
        use crate::llm::truncate_for_log as t;
        match e {
            Event::RunStarted { model, workdir, tools } => eprintln!("model={model} workdir={workdir} tools={tools:?}"),
            Event::Turn { .. } | Event::ReasoningDelta { .. } | Event::AssistantDelta { .. } => {}
            Event::ModelResponse { prompt_tokens, completion_tokens, ttft_secs, secs, .. } => if self.verbose {
                let gen = if *secs > *ttft_secs && *completion_tokens > 0 { *completion_tokens as f64 / (*secs - *ttft_secs) } else { 0.0 };
                eprintln!("⏱ {prompt_tokens}+{completion_tokens} tok · ttft {ttft_secs:.1}s · {gen:.1} tok/s");
            },
            Event::Reasoning { text } => if self.verbose { eprintln!("💭 {}", t(text.trim(), 400)) },
            Event::Assistant { text } => if self.verbose { eprintln!("🗣 {}", t(text.trim(), 800)) },
            Event::ToolCall { name, args, .. } => eprintln!("▶ {name} {}", t(&args.replace('\n', "\\n"), 300)),
            Event::ToolResult { name, result, secs, .. } => if self.verbose { eprintln!("◀ {name} ({secs:.1}s) {}", t(&result.replace('\n', "⏎"), 300)) },
            Event::Compacted { count, prompt_tokens, summary } => eprintln!("⟲ compacted {count} messages (prompt was {prompt_tokens} tokens){}", if summary.is_empty() { String::new() } else { format!(" — summary {} chars", summary.chars().count()) }),
            Event::RunFinished { stop_reason, turns, tool_calls, prompt_tokens, completion_tokens, wall_secs } =>
                eprintln!("— {turns} turns, {tool_calls} tool calls, {prompt_tokens}+{completion_tokens} tokens, {wall_secs:.0}s, stop={stop_reason}"),
            Event::Error { message } => eprintln!("✖ {message}"),
            Event::Memory { file, section, text } => eprintln!("🧠 {file} › {section}: {text}"),
            Event::Permission { tool, summary, decision } => eprintln!("🔒 {tool}({}) → {decision}", crate::llm::truncate_for_log(summary, 100)),
        }
    }
}

/// One JSON object per line to stdout — for piping into another process/UI.
pub struct JsonlSink;
impl Sink for JsonlSink {
    fn emit(&self, e: &Event) { println!("{}", serde_json::to_string(e).unwrap()); }
}
