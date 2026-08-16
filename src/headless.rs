//! Headless runs: `harness run` with Claude-Code-compatible stream-json in and out, plus structured
//! output against a JSON schema. This is the programmatic surface other tools drive us through
//! (the same shape we already consume from the `claude` CLI in `claude_code.rs`).
//!
//! Output formats:
//!   text        — events on stderr, the final answer on stdout (default)
//!   json        — events on stderr, one JSON object on stdout: {result, is_error, num_turns, usage…}
//!   stream-json — one JSON object per line on stdout: system/init, assistant, user (tool results),
//!                 result. Input may be the same shape (`--input-format stream-json`), one user
//!                 message per line, which makes the run multi-turn and interactive over pipes.

use crate::config::Config;
use crate::events::{Event, Sink};
use crate::llm::Message;
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat { Text, Json, StreamJson }

impl OutputFormat {
    pub fn parse(s: &str) -> Option<OutputFormat> {
        match s.trim().to_lowercase().replace('_', "-").as_str() {
            "text" | "" => Some(OutputFormat::Text),
            "json" => Some(OutputFormat::Json),
            "stream-json" | "streamjson" | "jsonl" => Some(OutputFormat::StreamJson),
            _ => None,
        }
    }
}

pub struct Options {
    pub output: OutputFormat,
    /// Keep working until an aux-model checker says this condition holds (max 12 rounds).
    pub goal: Option<String>,
    /// Read user turns as stream-json objects from stdin instead of taking one task argument.
    pub input_stream: bool,
    /// Constrain the final answer to this JSON schema (also injected into the system prompt).
    pub json_schema: Option<Value>,
    pub verbose: bool,
    pub yes: bool,
    pub max_turns: Option<usize>,
}
impl Default for Options {
    fn default() -> Self { Self { output: OutputFormat::Text, goal: None, input_stream: false, json_schema: None, verbose: false, yes: false, max_turns: None } }
}

/// Claude-Code-compatible stream-json on stdout.
pub struct StreamJsonSink { pub session_id: String, out: Mutex<std::io::Stdout> }
impl StreamJsonSink {
    pub fn new(session_id: &str) -> Self { Self { session_id: session_id.to_string(), out: Mutex::new(std::io::stdout()) } }
    pub fn line(&self, v: Value) {
        if let Ok(mut o) = self.out.lock() { let _ = writeln!(o, "{v}"); let _ = o.flush(); }
    }
}

impl Sink for StreamJsonSink {
    fn emit(&self, e: &Event) {
        let sid = self.session_id.clone();
        match e {
            Event::RunStarted { model, workdir, tools } => self.line(json!({"type":"system","subtype":"init","session_id":sid,"model":model,"cwd":workdir,"tools":tools,"permissionMode":"default"})),
            Event::Assistant { text } if !text.trim().is_empty() =>
                self.line(json!({"type":"assistant","session_id":sid,"message":{"role":"assistant","content":[{"type":"text","text":text}]}})),
            Event::ToolCall { id, name, args } => {
                let input: Value = serde_json::from_str(args).unwrap_or_else(|_| json!({"raw": args}));
                self.line(json!({"type":"assistant","session_id":sid,"message":{"role":"assistant","content":[{"type":"tool_use","id":id,"name":name,"input":input}]}}));
            }
            Event::ToolResult { id, result, .. } =>
                self.line(json!({"type":"user","session_id":sid,"message":{"role":"user","content":[{"type":"tool_result","tool_use_id":id,"content":[{"type":"text","text":result}]}]}})),
            Event::Reasoning { text } if !text.trim().is_empty() =>
                self.line(json!({"type":"assistant","session_id":sid,"message":{"role":"assistant","content":[{"type":"thinking","thinking":text}]}})),
            Event::Permission { tool, summary, decision } =>
                self.line(json!({"type":"system","subtype":"permission","session_id":sid,"tool":tool,"summary":summary,"decision":decision})),
            Event::Compacted { count, prompt_tokens, .. } =>
                self.line(json!({"type":"system","subtype":"compact_boundary","session_id":sid,"compact_metadata":{"trigger":"auto","pre_tokens":prompt_tokens,"messages":count}})),
            Event::Error { message } => self.line(json!({"type":"system","subtype":"error","session_id":sid,"message":message})),
            Event::Memory { file, section, text } => self.line(json!({"type":"system","subtype":"memory","session_id":sid,"file":file,"section":section,"text":text})),
            Event::ContextInfo { window, source } => self.line(json!({"type":"system","subtype":"context","session_id":sid,"window":window,"source":source})),
            _ => {}
        }
    }
}

/// Parse `--json-schema`: inline JSON or a path to a .json file.
pub fn load_schema(spec: &str) -> Result<Value> {
    let t = spec.trim();
    if t.starts_with('{') { return serde_json::from_str(t).context("--json-schema is not valid JSON"); }
    let text = std::fs::read_to_string(t).with_context(|| format!("reading schema file {t}"))?;
    serde_json::from_str(&text).with_context(|| format!("{t} is not valid JSON"))
}

/// A user turn from a stream-json input line (`{"type":"user","message":{...}}`); None = not a turn.
pub fn user_text_from_line(line: &str) -> Option<String> {
    let v: Value = serde_json::from_str(line.trim()).ok()?;
    match v["type"].as_str() {
        Some("user") | None => {}
        _ => return None,
    }
    let content = &v["message"]["content"];
    let text = match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts.iter().filter(|p| p["type"] == "text").filter_map(|p| p["text"].as_str()).collect::<Vec<_>>().join("\n"),
        _ => v["message"].as_str().or_else(|| v["prompt"].as_str()).unwrap_or("").to_string(),
    };
    (!text.trim().is_empty()).then_some(text)
}

/// Shallow JSON-schema check: type, required keys and enum values at the top level. Enough to catch a
/// model that answered with prose or forgot a field; deep validation is deliberately out of scope.
pub fn schema_errors(schema: &Value, value: &Value) -> Vec<String> {
    let mut errs = Vec::new();
    if let Some(t) = schema["type"].as_str() {
        let ok = match t { "object" => value.is_object(), "array" => value.is_array(), "string" => value.is_string(), "number" => value.is_number(), "integer" => value.is_i64() || value.is_u64(), "boolean" => value.is_boolean(), "null" => value.is_null(), _ => true };
        if !ok { errs.push(format!("top level must be a JSON {t}")); }
    }
    for k in schema["required"].as_array().map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>()).unwrap_or_default() {
        if value.get(k).is_none() { errs.push(format!("missing required field '{k}'")); }
    }
    if let (Some(props), Some(obj)) = (schema["properties"].as_object(), value.as_object()) {
        for (k, spec) in props {
            let Some(v) = obj.get(k) else { continue };
            if let Some(t) = spec["type"].as_str() {
                let ok = match t { "object" => v.is_object(), "array" => v.is_array(), "string" => v.is_string(), "number" => v.is_number(), "integer" => v.is_i64() || v.is_u64(), "boolean" => v.is_boolean(), _ => true };
                if !ok { errs.push(format!("field '{k}' must be a {t}")); }
            }
            if let Some(vals) = spec["enum"].as_array() { if !vals.contains(v) { errs.push(format!("field '{k}' must be one of {}", serde_json::to_string(vals).unwrap_or_default())); } }
        }
    }
    errs
}

/// Run headless. Returns the process exit code (0 ok, 1 error / schema violation).
pub async fn run(mut cfg: Config, workdir: PathBuf, task: Option<String>, opts: Options) -> Result<i32> {
    if let Some(n) = opts.max_turns { cfg.agent.max_turns = n; }
    let session_id = crate::sessions::SessionStore::new_id();
    let stream_sink = Arc::new(StreamJsonSink::new(&session_id));
    let sink: Arc<dyn Sink> = match opts.output {
        OutputFormat::StreamJson => stream_sink.clone(),
        OutputFormat::Json | OutputFormat::Text => Arc::new(crate::events::StderrSink { verbose: opts.verbose }),
    };
    let approver: Arc<dyn crate::permissions::Approver> = Arc::new(crate::permissions::AutoApprover { yes: opts.yes });

    let mut setup = crate::runner::RunSetup::new(cfg.clone(), workdir.clone(), sink.clone(), approver.clone());
    setup.session_id = Some(session_id.clone());
    if let Some(schema) = &opts.json_schema {
        setup.prompt_extra = Some(format!(
"\n\n# Structured output\nYour FINAL message (the one with no tool calls) must be a single JSON value matching this schema and nothing else — no prose, no markdown fences, no explanation:\n{}\n",
            serde_json::to_string_pretty(schema).unwrap_or_default()));
    }
    let prepared = crate::runner::prepare(setup).await?;
    let started = std::time::Instant::now();
    let mut msgs: Vec<Message> = Vec::new();
    let mut exit = 0;
    let mut total = crate::agent::RunStats::default();

    // one task argument, or a stream of user turns on stdin
    use tokio::io::AsyncBufReadExt;
    let mut lines: Option<tokio::io::Lines<tokio::io::BufReader<tokio::io::Stdin>>> =
        opts.input_stream.then(|| tokio::io::BufReader::new(tokio::io::stdin()).lines());
    let mut pending = task;

    loop {
        let prompt = match pending.take() {
            Some(p) => p,
            None => match &mut lines {
                Some(l) => {
                    match l.next_line().await? {
                        Some(line) if line.trim().is_empty() => continue,
                        Some(line) => match user_text_from_line(&line) { Some(t) => t, None => continue },
                        None => break,
                    }
                }
                None => break,
            },
        };

        let turn_started = std::time::Instant::now();
        let agent = prepared.agent();
        let res = if prepared.client.provider() == crate::llm::Provider::ClaudeCode {
            prepared.run_once(&prompt, &workdir).await
        } else {
            agent.run_turn(&mut msgs, &prepared.system, &prompt).await
        };
        match res {
            Ok((mut text, mut stats)) => {
                // --goal: keep going until the checker is satisfied
                if let Some(goal) = &opts.goal {
                    let mut round = 0;
                    loop {
                        let (met, reason) = crate::agent::goal_check(&prepared.client, goal, &text, &msgs).await;
                        if met { sink.emit(&Event::Assistant { text: format!("[goal met] {reason}") }); break; }
                        round += 1;
                        if round > 12 { sink.emit(&Event::Error { message: format!("goal not met after 12 rounds: {reason}") }); exit = 1; break; }
                        sink.emit(&Event::Assistant { text: format!("[goal round {round}] not met: {reason} — continuing") });
                        let cont = format!("[goal] Not satisfied yet: {reason}\nKeep working until this is true, then verify it: {goal}");
                        match prepared.agent().run_turn(&mut msgs, &prepared.system, &cont).await {
                            Ok((t, st)) => { text = t; stats.turns += st.turns; stats.tool_calls += st.tool_calls; stats.prompt_tokens += st.prompt_tokens; stats.completion_tokens += st.completion_tokens; }
                            Err(e) => { sink.emit(&Event::Error { message: format!("{e:#}") }); exit = 1; break; }
                        }
                    }
                }
                total.turns += stats.turns; total.tool_calls += stats.tool_calls;
                total.prompt_tokens += stats.prompt_tokens; total.completion_tokens += stats.completion_tokens;
                let mut structured: Option<Value> = None;
                if let Some(schema) = &opts.json_schema {
                    match enforce_schema(&prepared, &mut msgs, schema, &text).await {
                        Ok((v, t)) => { structured = Some(v); text = t; }
                        Err(e) => {
                            exit = 1;
                            sink.emit(&Event::Error { message: format!("structured output: {e:#}") });
                        }
                    }
                }
                emit_result(&opts, &stream_sink, &session_id, &text, structured.as_ref(), &stats, turn_started, exit != 0);
            }
            Err(e) => {
                exit = 1;
                sink.emit(&Event::Error { message: format!("{e:#}") });
                if opts.output == OutputFormat::StreamJson {
                    stream_sink.line(json!({"type":"result","subtype":"error_during_execution","is_error":true,"session_id":session_id,"result":format!("{e:#}"),"duration_ms":turn_started.elapsed().as_millis() as u64}));
                } else if opts.output == OutputFormat::Json {
                    println!("{}", json!({"result":format!("{e:#}"),"is_error":true,"session_id":session_id}));
                }
                if lines.is_none() { break; }
            }
        }
        if lines.is_none() { break; }
    }

    // the transcript is the run log — always persist it
    if !msgs.is_empty() {
        if let Ok(store) = crate::sessions::SessionStore::open() {
            let mut meta = crate::sessions::Meta { id: session_id.clone(), workdir: workdir.display().to_string(), model: prepared.client.model().to_string(), prompt_tokens: total.prompt_tokens, completion_tokens: total.completion_tokens, ..Default::default() };
            let _ = store.save(&mut meta, &msgs);
            if opts.output == OutputFormat::Text { eprintln!("· session saved: {session_id} (harness --resume {session_id})"); }
        }
        if let Some(m) = &prepared.store { crate::agent::reflect_after_run(&prepared.client, m, &msgs, &total, sink.as_ref()).await; }
    }
    let _ = started;
    Ok(exit)
}

fn emit_result(opts: &Options, stream: &StreamJsonSink, session_id: &str, text: &str, structured: Option<&Value>, stats: &crate::agent::RunStats, t0: std::time::Instant, is_error: bool) {
    match opts.output {
        OutputFormat::Text => println!("\n{text}"),
        OutputFormat::Json => {
            let mut v = json!({"type":"result","subtype": if is_error { "error_during_execution" } else { "success" }, "is_error": is_error, "session_id": session_id,
                "duration_ms": t0.elapsed().as_millis() as u64, "num_turns": stats.turns, "stop_reason": stats.stop_reason,
                "usage": {"input_tokens": stats.prompt_tokens, "output_tokens": stats.completion_tokens}, "tool_calls": stats.tool_calls});
            v["result"] = match structured { Some(s) => s.clone(), None => Value::String(text.to_string()) };
            println!("{v}");
        }
        OutputFormat::StreamJson => {
            let mut v = json!({"type":"result","subtype": if is_error { "error_during_execution" } else { "success" }, "is_error": is_error, "session_id": session_id,
                "duration_ms": t0.elapsed().as_millis() as u64, "num_turns": stats.turns, "stop_reason": stats.stop_reason,
                "usage": {"input_tokens": stats.prompt_tokens, "output_tokens": stats.completion_tokens}, "tool_calls": stats.tool_calls});
            v["result"] = match structured { Some(s) => s.clone(), None => Value::String(text.to_string()) };
            stream.line(v);
        }
    }
}

/// Parse the final answer as JSON and check it against the schema; one corrective turn if it is off.
async fn enforce_schema(prepared: &crate::runner::Prepared, msgs: &mut Vec<Message>, schema: &Value, text: &str) -> Result<(Value, String)> {
    for attempt in 0..2 {
        let candidate = crate::memory::extract_json(text).unwrap_or_else(|| text.trim().to_string());
        if let Ok(v) = serde_json::from_str::<Value>(&candidate) {
            let errs = schema_errors(schema, &v);
            if errs.is_empty() { return Ok((v, candidate)); }
            if attempt == 1 { anyhow::bail!("answer does not match the schema: {}", errs.join("; ")); }
            let fix = format!("[harness] Your answer must be a single JSON value matching the schema. Problems: {}. Reply with the corrected JSON only.", errs.join("; "));
            let (t, _) = prepared.agent().run_turn(msgs, &prepared.system, &fix).await?;
            return Box::pin(enforce_schema(prepared, msgs, schema, &t)).await;
        }
        if attempt == 1 { anyhow::bail!("the final answer is not valid JSON"); }
        let fix = "[harness] Your answer was not valid JSON. Reply with the JSON value only — no prose, no markdown fences.";
        let (t, _) = prepared.agent().run_turn(msgs, &prepared.system, fix).await?;
        return Box::pin(enforce_schema(prepared, msgs, schema, &t)).await;
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_json_input() {
        assert_eq!(user_text_from_line(r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"hi there"}]}}"#).as_deref(), Some("hi there"));
        assert_eq!(user_text_from_line(r#"{"type":"user","message":{"role":"user","content":"plain"}}"#).as_deref(), Some("plain"));
        assert_eq!(user_text_from_line(r#"{"type":"result","result":"x"}"#), None);
        assert_eq!(user_text_from_line("not json"), None);
    }

    #[test]
    fn schema_check() {
        let schema = json!({"type":"object","required":["title","score"],"properties":{"title":{"type":"string"},"score":{"type":"integer"},"level":{"enum":["low","high"]}}});
        assert!(schema_errors(&schema, &json!({"title":"a","score":3})).is_empty());
        assert_eq!(schema_errors(&schema, &json!({"title":"a"})), vec!["missing required field 'score'"]);
        assert_eq!(schema_errors(&schema, &json!(["a"])).len(), 3);
        assert!(schema_errors(&schema, &json!({"title":"a","score":1,"level":"mid"}))[0].contains("one of"));
    }

    #[test]
    fn output_format_parsing() {
        assert_eq!(OutputFormat::parse("stream-json"), Some(OutputFormat::StreamJson));
        assert_eq!(OutputFormat::parse("JSON"), Some(OutputFormat::Json));
        assert_eq!(OutputFormat::parse("nope"), None);
    }
}
