//! Structured events emitted by the agent loop. The core never prints; a frontend
//! (CLI today, HTTP/WebSocket + web UI or Tauri later) subscribes and renders.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    RunStarted { model: String, workdir: String, tools: Vec<String> },
    Turn { n: usize },
    Reasoning { text: String },
    Assistant { text: String },
    ToolCall { id: String, name: String, args: String },
    ToolResult { id: String, name: String, result: String, secs: f64 },
    Compacted { count: usize, prompt_tokens: u64 },
    RunFinished { stop_reason: String, turns: usize, tool_calls: usize, prompt_tokens: u64, completion_tokens: u64, wall_secs: f64 },
    Error { message: String },
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
            Event::Turn { .. } => {}
            Event::Reasoning { text } => if self.verbose { eprintln!("💭 {}", t(text.trim(), 400)) },
            Event::Assistant { text } => if self.verbose { eprintln!("🗣 {}", t(text.trim(), 800)) },
            Event::ToolCall { name, args, .. } => eprintln!("▶ {name} {}", t(&args.replace('\n', "\\n"), 300)),
            Event::ToolResult { name, result, secs, .. } => if self.verbose { eprintln!("◀ {name} ({secs:.1}s) {}", t(&result.replace('\n', "⏎"), 300)) },
            Event::Compacted { count, prompt_tokens } => eprintln!("⟲ compacted {count} old tool results (prompt was {prompt_tokens} tokens)"),
            Event::RunFinished { stop_reason, turns, tool_calls, prompt_tokens, completion_tokens, wall_secs } =>
                eprintln!("— {turns} turns, {tool_calls} tool calls, {prompt_tokens}+{completion_tokens} tokens, {wall_secs:.0}s, stop={stop_reason}"),
            Event::Error { message } => eprintln!("✖ {message}"),
        }
    }
}

/// One JSON object per line to stdout — for piping into another process/UI.
pub struct JsonlSink;
impl Sink for JsonlSink {
    fn emit(&self, e: &Event) { println!("{}", serde_json::to_string(e).unwrap()); }
}
