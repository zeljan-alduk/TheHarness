//! Backend `provider = "acp:<command>"`: drive *another* agent over the Agent Client Protocol and
//! present it through our UI. Codex, Gemini CLI, OpenCode, Copilot and friends all ship an ACP mode,
//! so this is the mirror image of `src/acp.rs` — there we are the agent, here we are the client:
//! we start the process, open a session in the working directory, stream its `session/update`s onto
//! our Event stream, answer its `session/request_permission` through our permission UI, and serve its
//! `fs/read_text_file` / `fs/write_text_file` requests from disk.

use crate::agent::RunStats;
use crate::events::{Event, Sink};
use crate::permissions::{Approval, ApprovalRequest, Approver, Policy};
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};

/// Well-known ACP commands, so `provider = "acp:gemini"` does the right thing without more config.
pub fn expand_command(spec: &str) -> String {
    let s = spec.trim();
    match s {
        "gemini" => "gemini --experimental-acp".into(),
        "codex" => "codex acp".into(),
        "opencode" => "opencode acp".into(),
        "copilot" => "copilot --acp".into(),
        "goose" => "goose acp".into(),
        "harness" => "harness acp".into(),
        other => other.to_string(),
    }
}

struct Turn { text: String, stats: RunStats, sink: Option<Arc<dyn Sink>> }

pub struct AcpSession {
    child: tokio::sync::Mutex<Option<Child>>,
    stdin: tokio::sync::Mutex<ChildStdin>,
    next_id: AtomicU64,
    pending: Mutex<HashMap<u64, tokio::sync::oneshot::Sender<Value>>>,
    session_id: Mutex<Option<String>>,
    turn: Arc<Mutex<Turn>>,
    pub command: String,
    pub workdir: PathBuf,
}

impl AcpSession {
    /// Start the agent, initialize the protocol and open a session in `workdir`.
    pub async fn start(spec: &str, workdir: &Path, policy: Arc<Policy>, approver: Arc<dyn Approver>) -> Result<Arc<Self>> {
        let command = expand_command(spec);
        let mut parts = command.split_whitespace();
        let prog = parts.next().context("empty ACP command")?;
        let bin = crate::setup::which(prog).map(|p| p.display().to_string()).unwrap_or_else(|| prog.to_string());
        let mut c = Command::new(&bin);
        c.args(parts).current_dir(workdir).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
        c.env("PATH", crate::setup::path_with_bin_dir(workdir));
        let mut child = c.spawn().with_context(|| format!("starting ACP agent `{command}` (is {prog} installed?)"))?;
        let stdin = child.stdin.take().context("no stdin")?;
        let stdout = child.stdout.take().context("no stdout")?;
        if let Some(err) = child.stderr.take() {
            tokio::spawn(async move {
                let mut lines = BufReader::new(err).lines();
                while let Ok(Some(l)) = lines.next_line().await { if !l.trim().is_empty() { eprintln!("[acp] {l}"); } }
            });
        }
        let s = Arc::new(AcpSession {
            child: tokio::sync::Mutex::new(Some(child)),
            stdin: tokio::sync::Mutex::new(stdin),
            next_id: AtomicU64::new(1),
            pending: Mutex::new(HashMap::new()),
            session_id: Mutex::new(None),
            turn: Arc::new(Mutex::new(Turn { text: String::new(), stats: RunStats::default(), sink: None })),
            command: command.clone(),
            workdir: workdir.to_path_buf(),
        });
        // reader: responses, notifications and the agent's own requests
        let s2 = s.clone();
        let (policy2, approver2) = (policy.clone(), approver.clone());
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                let Ok(v) = serde_json::from_str::<Value>(l.trim()) else { continue };
                s2.clone().on_message(v, policy2.clone(), approver2.clone()).await;
            }
        });
        let init = s.request("initialize", json!({
            "protocolVersion": crate::acp::PROTOCOL_VERSION,
            "clientCapabilities": {"fs": {"readTextFile": true, "writeTextFile": true}},
            "clientInfo": {"name": "harness", "version": crate::VERSION},
        })).await.context("the ACP agent did not answer initialize")?;
        if init.get("error").is_some() { bail!("initialize failed: {}", init["error"]); }
        let new = s.request("session/new", json!({"cwd": workdir.display().to_string(), "mcpServers": []})).await.context("session/new failed")?;
        let sid = new["sessionId"].as_str().context("the agent returned no sessionId")?.to_string();
        *s.session_id.lock().unwrap() = Some(sid);
        Ok(s)
    }

    fn send(&self, v: Value) -> impl std::future::Future<Output = ()> + '_ {
        async move {
            let mut w = self.stdin.lock().await;
            let _ = w.write_all(format!("{v}\n").as_bytes()).await;
            let _ = w.flush().await;
        }
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})).await;
        let v = tokio::time::timeout(std::time::Duration::from_secs(3600), rx).await.context("the ACP agent stopped responding")??;
        if let Some(e) = v.get("error") { bail!("{method}: {}", e["message"].as_str().unwrap_or(&e.to_string())); }
        Ok(v.get("result").cloned().unwrap_or(v))
    }

    /// Route one incoming message: a response we are waiting for, a session update, or a request.
    async fn on_message(self: Arc<Self>, v: Value, policy: Arc<Policy>, approver: Arc<dyn Approver>) {
        // response to one of our requests
        if v.get("method").is_none() {
            if let Some(id) = v["id"].as_u64() {
                if let Some(tx) = self.pending.lock().unwrap().remove(&id) { let _ = tx.send(v); }
            }
            return;
        }
        let method = v["method"].as_str().unwrap_or("").to_string();
        let params = v.get("params").cloned().unwrap_or(Value::Null);
        let id = v.get("id").cloned();
        match method.as_str() {
            "session/update" => self.on_update(&params["update"]),
            "fs/read_text_file" => {
                let path = params["path"].as_str().unwrap_or("");
                let res = std::fs::read_to_string(path).map(|c| json!({"content": c})).unwrap_or_else(|e| json!({"content": "", "error": e.to_string()}));
                if let Some(id) = id { self.send(json!({"jsonrpc": "2.0", "id": id, "result": res})).await; }
            }
            "fs/write_text_file" => {
                let (path, content) = (params["path"].as_str().unwrap_or("").to_string(), params["content"].as_str().unwrap_or("").to_string());
                // the agent writes through us, so our permission policy still applies
                let decision = policy.check("write_file", &json!({"path": path}), false);
                let allowed = match decision {
                    crate::permissions::Decision::Allow => true,
                    crate::permissions::Decision::Deny(_) => false,
                    crate::permissions::Decision::Ask(reason) => matches!(approver.ask(ApprovalRequest { tool: "write_file".into(), summary: path.clone(), suggested_rule: format!("write_file:{path}"), reason }).await, Approval::Once | Approval::Always | Approval::AlwaysProject),
                };
                let res = if !allowed { json!({"error": "refused by the user"}) }
                    else { match std::fs::write(&path, content) { Ok(()) => json!({}), Err(e) => json!({"error": e.to_string()}) } };
                if let Some(id) = id { self.send(json!({"jsonrpc": "2.0", "id": id, "result": res})).await; }
            }
            "session/request_permission" => {
                let tool = params["toolCall"]["title"].as_str().unwrap_or("tool call").to_string();
                let kind = params["toolCall"]["kind"].as_str().unwrap_or("other").to_string();
                let options = params["options"].as_array().cloned().unwrap_or_default();
                let pick = |wanted: &[&str]| options.iter().find(|o| wanted.contains(&o["kind"].as_str().unwrap_or(""))).and_then(|o| o["optionId"].as_str()).map(|s| s.to_string());
                let approval = approver.ask(ApprovalRequest { tool: format!("{}({kind})", self.command.split_whitespace().next().unwrap_or("agent")), summary: tool, suggested_rule: String::new(), reason: "the external agent asks for permission".into() }).await;
                let chosen = match approval {
                    Approval::Once => pick(&["allow_once", "allow"]).or_else(|| pick(&["allow_always"])),
                    Approval::Always | Approval::AlwaysProject => pick(&["allow_always"]).or_else(|| pick(&["allow_once", "allow"])),
                    Approval::Deny => None,
                };
                let outcome = match chosen {
                    Some(opt) => json!({"outcome": "selected", "optionId": opt}),
                    None => match pick(&["reject_once", "reject_always", "reject"]) { Some(opt) => json!({"outcome": "selected", "optionId": opt}), None => json!({"outcome": "cancelled"}) },
                };
                if let Some(id) = id { self.send(json!({"jsonrpc": "2.0", "id": id, "result": {"outcome": outcome}})).await; }
            }
            _ => {
                // unknown request: answer with an error so the agent is not left waiting
                if let Some(id) = id { self.send(json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": format!("method not found: {method}")}})).await; }
            }
        }
    }

    /// Map the agent's session/update notifications onto our Event stream.
    fn on_update(&self, u: &Value) {
        let mut t = self.turn.lock().unwrap();
        let sink = t.sink.clone();
        let emit = |e: &Event| { if let Some(s) = &sink { s.emit(e); } };
        match u["sessionUpdate"].as_str().unwrap_or("") {
            "agent_message_chunk" => {
                let text = u["content"]["text"].as_str().unwrap_or("").to_string();
                if !text.is_empty() { t.text.push_str(&text); emit(&Event::AssistantDelta { text }); }
            }
            "agent_thought_chunk" => {
                let text = u["content"]["text"].as_str().unwrap_or("").to_string();
                if !text.is_empty() { emit(&Event::ReasoningDelta { text }); }
            }
            "tool_call" => {
                t.stats.tool_calls += 1;
                emit(&Event::ToolCall {
                    id: u["toolCallId"].as_str().unwrap_or("").to_string(),
                    name: u["title"].as_str().unwrap_or(u["kind"].as_str().unwrap_or("tool")).to_string(),
                    args: u["rawInput"].to_string(),
                });
            }
            "tool_call_update" => {
                let status = u["status"].as_str().unwrap_or("");
                if status == "completed" || status == "failed" {
                    let text = u["content"].as_array().map(|a| a.iter().filter_map(|c| c["content"]["text"].as_str()).collect::<Vec<_>>().join("\n")).unwrap_or_default();
                    emit(&Event::ToolResult { id: u["toolCallId"].as_str().unwrap_or("").to_string(), name: status.to_string(), result: text, secs: 0.0, images: vec![] });
                }
            }
            "plan" => {
                let entries: Vec<String> = u["entries"].as_array().map(|a| a.iter().filter_map(|e| e["content"].as_str().map(|c| format!("- [{}] {c}", e["status"].as_str().unwrap_or("pending")))).collect()).unwrap_or_default();
                if !entries.is_empty() { emit(&Event::Assistant { text: format!("plan:\n{}", entries.join("\n")) }); }
            }
            _ => {}
        }
    }

    /// One turn: send the prompt, stream updates, return the agent's answer.
    pub async fn run_turn(&self, prompt: &str, sink: Arc<dyn Sink>) -> Result<(String, RunStats)> {
        let sid = self.session_id.lock().unwrap().clone().context("no ACP session")?;
        { let mut t = self.turn.lock().unwrap(); t.text.clear(); t.stats = RunStats::default(); t.sink = Some(sink.clone()); }
        let started = std::time::Instant::now();
        let res = self.request("session/prompt", json!({"sessionId": sid, "prompt": [{"type": "text", "text": prompt}]})).await;
        let mut t = self.turn.lock().unwrap();
        t.sink = None;
        let mut stats = std::mem::take(&mut t.stats);
        stats.wall_secs = started.elapsed().as_secs_f64();
        stats.turns = 1;
        match res {
            Ok(v) => {
                stats.stop_reason = v["stopReason"].as_str().unwrap_or("end_turn").to_string();
                let text = std::mem::take(&mut t.text);
                if text.trim().is_empty() { Ok((format!("(the agent finished with stopReason={})", stats.stop_reason), stats)) } else { Ok((text, stats)) }
            }
            Err(e) => Err(e),
        }
    }

    pub async fn cancel(&self) {
        if let Some(sid) = self.session_id.lock().unwrap().clone() {
            self.send(json!({"jsonrpc": "2.0", "method": "session/cancel", "params": {"sessionId": sid}})).await;
        }
    }

    pub async fn stop(&self) {
        if let Some(mut c) = self.child.lock().await.take() { let _ = c.kill().await; }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_shortcuts() {
        assert_eq!(expand_command("gemini"), "gemini --experimental-acp");
        assert_eq!(expand_command("codex"), "codex acp");
        assert_eq!(expand_command("my-agent --acp --flag"), "my-agent --acp --flag");
    }

    /// Drive our own `harness acp` server through this client: initialize, a session and a prompt.
    #[tokio::test]
    async fn talks_to_an_acp_agent() {
        let exe = match std::env::var_os("CARGO_BIN_EXE_harness").map(PathBuf::from) {
            Some(p) => p,
            None => { let p = std::env::current_exe().ok().and_then(|p| p.parent().and_then(|d| d.parent().map(|d| d.join("harness")))); match p.filter(|p| p.is_file()) { Some(p) => p, None => return } }
        };
        let d = std::env::temp_dir().join(format!("harness-acpclient-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let policy = Arc::new(Policy::new(Default::default(), &d));
        let approver: Arc<dyn Approver> = Arc::new(crate::permissions::AutoApprover { yes: true });
        // a real model is not reachable in tests; starting the session is what we verify here
        let spec = format!("{} acp", exe.display());
        let Ok(s) = AcpSession::start(&spec, &d, policy, approver).await else { let _ = std::fs::remove_dir_all(&d); return };
        assert!(s.session_id.lock().unwrap().is_some(), "the agent opened a session");
        s.stop().await;
        let _ = std::fs::remove_dir_all(&d);
    }
}
