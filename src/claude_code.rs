//! Backend `provider = "claude-code"`: run the official `claude` CLI headlessly (uses the user's
//! Anthropic subscription through the official client) with the harness's tools exposed over an MCP
//! bridge. One long-lived process per session (stream-json in/out); events are mapped onto the
//! harness Event stream so every UI works unchanged.

use crate::events::{Event, Sink};
use crate::agent::RunStats;
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{mpsc, Mutex};

pub struct ClaudeCodeSession {
    child: Mutex<Option<Child>>,
    stdin: Mutex<ChildStdin>,
    events: Mutex<mpsc::UnboundedReceiver<Value>>,
    pub session_id: Mutex<Option<String>>,
    _bridge: tokio::task::JoinHandle<()>,
    mcp_cfg: PathBuf,
}

pub fn claude_bin() -> Option<PathBuf> {
    crate::setup::which("claude").or_else(|| { let p = crate::setup::home_dir().join(".local/bin/claude"); p.is_file().then_some(p) })
}

impl ClaudeCodeSession {
    /// Start `claude` with our tools bridged in. `system` is our full system prompt.
    pub async fn start(workdir: &Path, model: Option<&str>, system: &str, host: Arc<crate::mcp_bridge::BridgeHost>, resume: Option<&str>) -> Result<Arc<Self>> {
        let bin = claude_bin().context("`claude` CLI not found — install Claude Code and log in (https://claude.com/claude-code)")?;
        let (addr, bridge) = crate::mcp_bridge::serve(&crate::mcp_bridge::new_addr(), host).await?;
        let me = std::env::var_os("HARNESS_ORIG_EXE").map(PathBuf::from).unwrap_or(std::env::current_exe()?);
        let mcp_cfg = std::env::temp_dir().join(format!("harness-cc-mcp-{}.json", std::process::id()));
        std::fs::write(&mcp_cfg, json!({"mcpServers": {"harness": {"command": me.display().to_string(), "args": ["mcp-proxy", addr]}}}).to_string())?;
        let mut c = Command::new(&bin);
        c.args(["--print", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose", "--include-partial-messages", "--permission-mode", "bypassPermissions", "--strict-mcp-config", "--mcp-config"]).arg(&mcp_cfg)
         .args(["--tools", "", "--system-prompt"]).arg(system);
        if let Some(m) = model { if !m.is_empty() { c.args(["--model", m]); } }
        if let Some(r) = resume { c.args(["--resume", r]); }
        c.current_dir(workdir).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
        c.env_remove("CLAUDECODE"); // allow nesting when the harness itself runs inside Claude Code
        let mut child = c.spawn().with_context(|| format!("starting {}", bin.display()))?;
        let stdin = child.stdin.take().context("no stdin")?;
        let stdout = child.stdout.take().context("no stdout")?;
        let stderr = child.stderr.take();
        let (tx, rx) = mpsc::unbounded_channel::<Value>();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(l)) = lines.next_line().await { if let Ok(v) = serde_json::from_str::<Value>(&l) { if tx.send(v).is_err() { break; } } }
        });
        if let Some(err) = stderr { let tx2 = tx_err_channel(); tokio::spawn(async move { let mut lines = BufReader::new(err).lines(); while let Ok(Some(l)) = lines.next_line().await { let _ = tx2.send(l); } }); }
        Ok(Arc::new(Self { child: Mutex::new(Some(child)), stdin: Mutex::new(stdin), events: Mutex::new(rx), session_id: Mutex::new(None), _bridge: bridge, mcp_cfg }))
    }

    /// Send a user turn (text + optional images) and stream events until the turn's `result`.
    pub async fn run_turn(&self, text: &str, images: &[(String, String)], sink: &dyn Sink) -> Result<(String, RunStats)> {
        let mut content = vec![json!({"type": "text", "text": text})];
        for (mime, b64) in images { content.push(json!({"type": "image", "source": {"type": "base64", "media_type": mime, "data": b64}})); }
        let msg = json!({"type": "user", "message": {"role": "user", "content": content}});
        { let mut w = self.stdin.lock().await; w.write_all(format!("{}\n", msg).as_bytes()).await?; w.flush().await?; }
        let start = std::time::Instant::now();
        let mut stats = RunStats::default();
        let mut final_text = String::new();
        let mut cur_text = String::new();
        let mut cur_think = String::new();
        let mut think_est: u64 = 0; let mut thinking_hidden = false;
        let mut rx = self.events.lock().await;
        loop {
            let Some(ev) = rx.recv().await else { bail!("claude process ended unexpectedly{}", drain_stderr()) };
            match ev["type"].as_str().unwrap_or("") {
                "system" if ev["subtype"] == "thinking_tokens" => { if let Some(n) = ev["estimated_tokens"].as_u64() { think_est = think_est.max(n); thinking_hidden = true; sink.emit(&Event::ThinkingStatus { est_tokens: n, done: false }); } }
                "system" if ev["subtype"] == "status" => { if ev["status"] == "compacting" { sink.emit(&Event::CompactProgress { fraction: 0.15, phase: "Claude Code is compacting its context…".into() }); } else if ev["compact_result"].is_string() { sink.emit(&Event::CompactProgress { fraction: 1.0, phase: "done".into() }); } }
                "system" if ev["subtype"] == "compact_boundary" => {
                    let m = &ev["compact_metadata"]; let pre = m["pre_tokens"].as_u64().unwrap_or(0); let post = m["post_tokens"].as_u64().unwrap_or(0);
                    sink.emit(&Event::Compacted { count: 0, prompt_tokens: pre, summary: String::new(), map_before: vec![("claude context".into(), pre)], map_after: vec![("claude context (summary)".into(), post)] });
                }
                "system" => { if ev["subtype"] == "init" { if let Some(id) = ev["session_id"].as_str() { *self.session_id.lock().await = Some(id.to_string()); } sink.emit(&Event::RunStarted { model: ev["model"].as_str().unwrap_or("claude").to_string(), workdir: ev["cwd"].as_str().unwrap_or("").to_string(), tools: ev["tools"].as_array().map(|a| a.iter().filter_map(|t| t.as_str().map(String::from)).collect()).unwrap_or_default() }); } }
                "stream_event" => {
                    let e = &ev["event"];
                    match e["type"].as_str().unwrap_or("") {
                        "message_start" => { stats.turns += 1; sink.emit(&Event::Turn { n: stats.turns }); }
                        "content_block_delta" => match e["delta"]["type"].as_str().unwrap_or("") {
                            "text_delta" => { let t = e["delta"]["text"].as_str().unwrap_or(""); cur_text.push_str(t); sink.emit(&Event::AssistantDelta { text: t.to_string() }); }
                            "thinking_delta" => {
                                let t = e["delta"]["thinking"].as_str().unwrap_or("");
                                if !t.is_empty() { cur_think.push_str(t); sink.emit(&Event::ReasoningDelta { text: t.to_string() }); }
                                else { if let Some(n) = e["delta"]["estimated_tokens"].as_u64() { think_est = think_est.max(n); } else { think_est += 40; } thinking_hidden = true; sink.emit(&Event::ThinkingStatus { est_tokens: think_est, done: false }); }
                            }
                            _ => {}
                        },
                        "content_block_stop" => {
                            if !cur_think.is_empty() { sink.emit(&Event::Reasoning { text: std::mem::take(&mut cur_think) }); }
                            else if thinking_hidden { sink.emit(&Event::ThinkingStatus { est_tokens: think_est, done: true }); thinking_hidden = false; think_est = 0; }
                        }
                        "message_delta" => { if let Some(u) = e["usage"].as_object() { let o = u.get("output_tokens").and_then(|x| x.as_u64()).unwrap_or(0); stats.completion_tokens += o; } }
                        _ => {}
                    }
                }
                "assistant" => {
                    // full assistant message: text blocks (final for this step) and tool_use (executed by claude via our bridge)
                    let mut txt = String::new();
                    for b in ev["message"]["content"].as_array().cloned().unwrap_or_default() { if b["type"] == "text" { txt.push_str(b["text"].as_str().unwrap_or("")); } else if b["type"] == "tool_use" { stats.tool_calls += 1; } }
                    if !txt.trim().is_empty() { sink.emit(&Event::Assistant { text: txt.clone() }); final_text = txt; }
                    cur_text.clear();
                    if let Some(u) = ev["message"]["usage"].as_object() { stats.prompt_tokens += u.get("input_tokens").and_then(|x| x.as_u64()).unwrap_or(0) + u.get("cache_read_input_tokens").and_then(|x| x.as_u64()).unwrap_or(0) + u.get("cache_creation_input_tokens").and_then(|x| x.as_u64()).unwrap_or(0); }
                }
                "user" => {} // tool results — already rendered by the bridge
                "result" => {
                    let secs = start.elapsed().as_secs_f64();
                    if let Some(r) = ev["result"].as_str() { if !r.trim().is_empty() { final_text = r.to_string(); } }
                    let ok = ev["subtype"] == "success" && !ev["is_error"].as_bool().unwrap_or(false);
                    stats.stop_reason = if ok { "done".into() } else { format!("error: {}", ev["subtype"].as_str().unwrap_or("?")) };
                    stats.wall_secs = secs;
                    if let Some(mu) = ev["modelUsage"].as_object() { for (m, v) in mu { if let Some(w) = v["contextWindow"].as_u64() { sink.emit(&Event::ContextInfo { window: w, source: format!("Claude Code ({m})") }); } } }
                    if let Some(th) = ev["usage"]["output_tokens_details"]["thinking_tokens"].as_u64() { if th > 0 { sink.emit(&Event::ThinkingStatus { est_tokens: th, done: true }); } }
                    if let Some(u) = ev["usage"].as_object() { let inp = u.get("input_tokens").and_then(|x| x.as_u64()).unwrap_or(0) + u.get("cache_read_input_tokens").and_then(|x| x.as_u64()).unwrap_or(0) + u.get("cache_creation_input_tokens").and_then(|x| x.as_u64()).unwrap_or(0); if inp > 0 { stats.prompt_tokens = inp; } if let Some(o) = u.get("output_tokens").and_then(|x| x.as_u64()) { stats.completion_tokens = o; } }
                    sink.emit(&Event::ModelResponse { prompt_tokens: stats.prompt_tokens, completion_tokens: stats.completion_tokens, ttft_secs: 0.0, secs, tool_calls: stats.tool_calls });
                    if let Some(c) = ev["total_cost_usd"].as_f64() { sink.emit(&Event::Memory { file: "claude-code".into(), section: "cost".into(), text: format!("this turn ≈ ${c:.4} (subscription: informational)") }); }
                    sink.emit(&Event::RunFinished { stop_reason: stats.stop_reason.clone(), turns: stats.turns.max(1), tool_calls: stats.tool_calls, prompt_tokens: stats.prompt_tokens, completion_tokens: stats.completion_tokens, wall_secs: secs });
                    if !ok { bail!("claude: {}{}", ev["result"].as_str().unwrap_or("error"), drain_stderr()); }
                    return Ok((final_text, stats));
                }
                _ => {}
            }
        }
    }

    /// Ask Claude Code to compact its own context (built-in /compact); progress/boundary events are emitted.
    pub async fn compact(&self, focus: Option<&str>, sink: &dyn Sink) -> Result<(u64, u64)> {
        let cmd = match focus { Some(f) if !f.trim().is_empty() => format!("/compact {f}"), _ => "/compact".to_string() };
        let msg = json!({"type": "user", "message": {"role": "user", "content": [{"type": "text", "text": cmd}]}});
        { let mut w = self.stdin.lock().await; w.write_all(format!("{}\n", msg).as_bytes()).await?; w.flush().await?; }
        sink.emit(&Event::CompactProgress { fraction: 0.05, phase: "asking Claude Code to compact…".into() });
        let start = std::time::Instant::now();
        let mut rx = self.events.lock().await;
        let (mut pre, mut post) = (0u64, 0u64);
        loop {
            let ev = match tokio::time::timeout(std::time::Duration::from_secs(300), rx.recv()).await { Ok(Some(e)) => e, Ok(None) => bail!("claude process ended"), Err(_) => bail!("compaction timed out") };
            match (ev["type"].as_str().unwrap_or(""), ev["subtype"].as_str().unwrap_or("")) {
                ("system", "status") => { if ev["status"] == "compacting" { let f = 0.15 + 0.7 * (1.0 - (-(start.elapsed().as_secs_f64()) / 20.0).exp()); sink.emit(&Event::CompactProgress { fraction: f, phase: format!("Claude Code is compacting… {}s", start.elapsed().as_secs()) }); } }
                ("system", "compact_boundary") => { let m = &ev["compact_metadata"]; pre = m["pre_tokens"].as_u64().unwrap_or(0); post = m["post_tokens"].as_u64().unwrap_or(0); }
                ("result", _) => {
                    sink.emit(&Event::CompactProgress { fraction: 1.0, phase: "done".into() });
                    if pre > 0 { sink.emit(&Event::Compacted { count: 0, prompt_tokens: pre, summary: String::new(), map_before: vec![("claude context".into(), pre)], map_after: vec![("claude context (summary)".into(), post)] }); }
                    return Ok((pre, post));
                }
                _ => {}
            }
        }
    }

    pub async fn stop(&self) { if let Some(mut c) = self.child.lock().await.take() { let _ = c.kill().await; } let _ = std::fs::remove_file(&self.mcp_cfg); }
}

static STDERR: std::sync::OnceLock<std::sync::Mutex<Vec<String>>> = std::sync::OnceLock::new();
fn tx_err_channel() -> mpsc::UnboundedSender<String> { let (tx, mut rx) = mpsc::unbounded_channel::<String>(); tokio::spawn(async move { while let Some(l) = rx.recv().await { let m = STDERR.get_or_init(|| std::sync::Mutex::new(vec![])); let mut g = m.lock().unwrap(); g.push(l); if g.len() > 50 { g.remove(0); } } }); tx }
fn drain_stderr() -> String { STDERR.get().map(|m| { let mut g = m.lock().unwrap(); let s = g.join("\n"); g.clear(); if s.is_empty() { String::new() } else { format!("\nclaude stderr:\n{}", crate::llm::truncate_for_log(&s, 800)) } }).unwrap_or_default() }
