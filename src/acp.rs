//! ACP (Agent Client Protocol) server: `harness acp` speaks newline-delimited JSON-RPC 2.0 on stdio,
//! which is what Zed, JetBrains, Neovim, Emacs and the other ACP clients launch an agent as. It maps
//! our Event stream onto `session/update` notifications and our permission prompts onto
//! `session/request_permission`, so the whole harness (tools, checkpoints, MCP, sub-agents) shows up
//! inside the editor with no editor-specific code.
//!
//! Implemented: initialize · session/new · session/load · session/prompt · session/cancel ·
//! session/set_mode, and the agent→client `session/update` + `session/request_permission` calls.

use crate::config::Config;
use crate::events::{Event, Sink};
use crate::llm::{Content, Message};
use crate::permissions::{Approval, ApprovalRequest, Approver};
use anyhow::Result;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

pub const PROTOCOL_VERSION: u64 = 1;

/// The stdio JSON-RPC connection: writes messages, and correlates the requests we send to the client.
pub struct Conn {
    out: Mutex<std::io::Stdout>,
    next_id: AtomicU64,
    pending: Mutex<HashMap<u64, tokio::sync::oneshot::Sender<Value>>>,
}

impl Conn {
    pub fn new() -> Self { Self { out: Mutex::new(std::io::stdout()), next_id: AtomicU64::new(1), pending: Mutex::new(HashMap::new()) } }

    fn send(&self, v: Value) {
        if let Ok(mut o) = self.out.lock() { let _ = writeln!(o, "{v}"); let _ = o.flush(); }
    }
    pub fn notify(&self, method: &str, params: Value) { self.send(json!({"jsonrpc":"2.0","method":method,"params":params})); }
    pub fn reply(&self, id: Value, result: Value) { self.send(json!({"jsonrpc":"2.0","id":id,"result":result})); }
    pub fn reply_err(&self, id: Value, code: i64, message: String) { self.send(json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})); }

    /// Call the client and wait for its answer.
    pub async fn request(&self, method: &str, params: Value) -> Option<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.pending.lock().ok()?.insert(id, tx);
        self.send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}));
        rx.await.ok()
    }
    /// Route an incoming response to whoever is waiting for it.
    fn resolve(&self, id: u64, result: Value) {
        if let Some(tx) = self.pending.lock().ok().and_then(|mut p| p.remove(&id)) { let _ = tx.send(result); }
    }
}
impl Default for Conn { fn default() -> Self { Self::new() } }

/// Translates harness events into ACP `session/update` notifications.
pub struct AcpSink { pub conn: Arc<Conn>, pub session_id: String }

impl AcpSink {
    fn update(&self, u: Value) { self.conn.notify("session/update", json!({"sessionId": self.session_id, "update": u})); }
}

/// ACP tool-call kinds, from our tool names.
pub fn tool_kind(name: &str) -> &'static str {
    match name {
        "read_file" | "list_dir" | "view_image" | "read_pdf" | "notebook_edit" => "read",
        "write_file" | "edit_file" | "apply_patch" | "pdf_edit" => "edit",
        "grep" | "glob" => "search",
        "bash" | "process" | "monitor" | "run_workflow" => "execute",
        "web_fetch" | "web_search" | "download_file" => "fetch",
        "spawn_agent" | "agents" => "think",
        _ => "other",
    }
}

impl Sink for AcpSink {
    fn emit(&self, e: &Event) {
        match e {
            Event::AssistantDelta { text } => self.update(json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":text}})),
            Event::ReasoningDelta { text } => self.update(json!({"sessionUpdate":"agent_thought_chunk","content":{"type":"text","text":text}})),
            Event::ToolCall { id, name, args } => {
                let input: Value = serde_json::from_str(args).unwrap_or(Value::Null);
                let locations: Vec<Value> = input.get("path").and_then(|p| p.as_str()).map(|p| vec![json!({"path": p})]).unwrap_or_default();
                self.update(json!({"sessionUpdate":"tool_call","toolCallId":id,"title":format!("{name} {}", crate::llm::truncate_for_log(&crate::permissions::Policy::primary_arg(name, &input), 80)),
                    "kind":tool_kind(name),"status":"in_progress","rawInput":input,"locations":locations}));
            }
            Event::ToolResult { id, result, .. } => self.update(json!({"sessionUpdate":"tool_call_update","toolCallId":id,
                "status": if result.starts_with("error:") { "failed" } else { "completed" },
                "content":[{"type":"content","content":{"type":"text","text":crate::llm::truncate_for_log(result, 20000)}}]})),
            Event::Compacted { count, .. } => self.update(json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":format!("\n_[context compacted: {count} messages]_\n")}})),
            Event::Error { message } => self.update(json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":format!("\n**error:** {message}\n")}})),
            _ => {}
        }
    }
}

/// Permission prompts become ACP `session/request_permission` calls.
pub struct AcpApprover { pub conn: Arc<Conn>, pub session_id: String }

#[async_trait::async_trait]
impl Approver for AcpApprover {
    fn interactive(&self) -> bool { true }
    async fn ask(&self, req: ApprovalRequest) -> Approval {
        let params = json!({
            "sessionId": self.session_id,
            "toolCall": {"toolCallId": format!("perm-{}", req.tool), "title": format!("{}: {}", req.tool, crate::llm::truncate_for_log(&req.summary, 100)), "kind": tool_kind(&req.tool), "status": "pending"},
            "options": [
                {"optionId":"allow","name":"Allow once","kind":"allow_once"},
                {"optionId":"allow_always","name":format!("Always allow {}", req.suggested_rule),"kind":"allow_always"},
                {"optionId":"reject","name":"Reject","kind":"reject_once"},
            ],
        });
        let Some(resp) = self.conn.request("session/request_permission", params).await else { return Approval::Deny };
        match resp["outcome"]["outcome"].as_str() {
            Some("selected") => match resp["outcome"]["optionId"].as_str() {
                Some("allow") => Approval::Once,
                Some("allow_always") => Approval::Always,
                _ => Approval::Deny,
            },
            _ => Approval::Deny,
        }
    }
}

struct Session {
    workdir: PathBuf,
    msgs: Mutex<Vec<Message>>,
    prepared: Arc<crate::runner::Prepared>,
    cancel: Arc<AtomicBool>,
    meta: Mutex<crate::sessions::Meta>,
}

/// Run the ACP server on stdin/stdout until the client closes the connection.
pub async fn serve(cfg: Config) -> Result<()> {
    use tokio::io::AsyncBufReadExt;
    let conn = Arc::new(Conn::new());
    let sessions: Arc<tokio::sync::Mutex<HashMap<String, Arc<Session>>>> = Default::default();
    let mut lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();
    while let Some(line) = lines.next_line().await? {
        let line = line.trim().to_string();
        if line.is_empty() { continue; }
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
        // a response to something we asked the client
        if msg.get("method").is_none() {
            if let Some(id) = msg["id"].as_u64() { conn.resolve(id, msg.get("result").cloned().unwrap_or(Value::Null)); }
            continue;
        }
        let (conn2, sessions2, cfg2) = (conn.clone(), sessions.clone(), cfg.clone());
        tokio::spawn(async move { handle(conn2, sessions2, cfg2, msg).await; });
    }
    Ok(())
}

async fn handle(conn: Arc<Conn>, sessions: Arc<tokio::sync::Mutex<HashMap<String, Arc<Session>>>>, cfg: Config, msg: Value) {
    let method = msg["method"].as_str().unwrap_or("").to_string();
    let id = msg.get("id").cloned();
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    let result: Result<Value> = match method.as_str() {
        "initialize" => Ok(json!({
            "protocolVersion": PROTOCOL_VERSION,
            "agentCapabilities": {"loadSession": true, "promptCapabilities": {"image": true, "audio": false, "embeddedContext": true}},
            "authMethods": [],
            "agentInfo": {"name": "harness", "version": crate::VERSION},
        })),
        "authenticate" => Ok(json!({})),
        "session/new" => new_session(&conn, &sessions, &cfg, &params, None).await,
        "session/load" => {
            let sid = params["sessionId"].as_str().unwrap_or("").to_string();
            new_session(&conn, &sessions, &cfg, &params, Some(sid)).await
        }
        "session/prompt" => prompt(&conn, &sessions, &params).await,
        "session/cancel" => {
            let sid = params["sessionId"].as_str().unwrap_or("");
            if let Some(s) = sessions.lock().await.get(sid) { s.cancel.store(true, Ordering::Relaxed); }
            Ok(Value::Null)
        }
        "session/set_mode" => {
            let sid = params["sessionId"].as_str().unwrap_or("");
            let mode = params["modeId"].as_str().unwrap_or("");
            match (sessions.lock().await.get(sid), crate::permissions::Mode::parse(mode)) {
                (Some(s), Some(m)) => { s.prepared.policy.set_mode(m); Ok(json!({})) }
                (None, _) => Err(anyhow::anyhow!("unknown session {sid}")),
                (_, None) => Err(anyhow::anyhow!("unknown mode {mode}")),
            }
        }
        _ => Err(anyhow::anyhow!("method not found: {method}")),
    };
    // notifications (no id) get no reply
    let Some(id) = id else { return };
    match result {
        Ok(v) => conn.reply(id, v),
        Err(e) => conn.reply_err(id, -32603, format!("{e:#}")),
    }
}

async fn new_session(conn: &Arc<Conn>, sessions: &Arc<tokio::sync::Mutex<HashMap<String, Arc<Session>>>>, cfg: &Config, params: &Value, load: Option<String>) -> Result<Value> {
    let cwd = params["cwd"].as_str().map(PathBuf::from).unwrap_or(std::env::current_dir()?);
    let workdir = cwd.canonicalize().unwrap_or(cwd);
    let session_id = load.clone().filter(|s| !s.is_empty()).unwrap_or_else(crate::sessions::SessionStore::new_id);
    let sink: Arc<dyn Sink> = Arc::new(AcpSink { conn: conn.clone(), session_id: session_id.clone() });
    let approver: Arc<dyn Approver> = Arc::new(AcpApprover { conn: conn.clone(), session_id: session_id.clone() });
    let mut setup = crate::runner::RunSetup::new(cfg.clone(), workdir.clone(), sink.clone(), approver);
    setup.session_id = Some(session_id.clone());
    setup.prompt_extra = Some("You are running inside the user's editor over ACP: they see your messages, diffs and tool calls live and can reply. Keep final answers short.".to_string());
    let mut prepared = crate::runner::prepare(setup).await?;
    let cancel = Arc::new(AtomicBool::new(false));
    prepared.ctx.cancel = Some(cancel.clone());

    // restore an earlier transcript when the client asks us to load one
    let (mut msgs, mut meta) = (Vec::new(), crate::sessions::Meta { id: session_id.clone(), workdir: workdir.display().to_string(), model: prepared.client.model().to_string(), ..Default::default() });
    if load.is_some() {
        if let Ok(store) = crate::sessions::SessionStore::open() {
            if let Ok((m, old)) = store.load(&session_id) {
                meta = m;
                for msg in &old {
                    let update = match msg.role.as_str() {
                        "user" => Some(json!({"sessionUpdate":"user_message_chunk","content":{"type":"text","text":msg.text()}})),
                        "assistant" if !msg.text().trim().is_empty() => Some(json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":msg.text()}})),
                        _ => None,
                    };
                    if let Some(u) = update { conn.notify("session/update", json!({"sessionId": session_id, "update": u})); }
                }
                msgs = old;
            }
        }
    }
    let s = Arc::new(Session { workdir, msgs: Mutex::new(msgs), prepared: Arc::new(prepared), cancel, meta: Mutex::new(meta) });
    sessions.lock().await.insert(session_id.clone(), s);
    Ok(json!({"sessionId": session_id, "modes": {"currentModeId": cfg.permissions.mode_id(), "availableModes": [
        {"id":"bypass","name":"Bypass permissions","description":"never ask"},
        {"id":"auto","name":"Auto","description":"reads and in-project writes run; risky calls ask"},
        {"id":"ask","name":"Ask","description":"every change asks"},
        {"id":"plan","name":"Plan","description":"read-only"}]}}))
}

async fn prompt(conn: &Arc<Conn>, sessions: &Arc<tokio::sync::Mutex<HashMap<String, Arc<Session>>>>, params: &Value) -> Result<Value> {
    let sid = params["sessionId"].as_str().unwrap_or("").to_string();
    let Some(s) = sessions.lock().await.get(&sid).cloned() else { anyhow::bail!("unknown session {sid}") };
    let (text, images) = prompt_content(&params["prompt"], &s.workdir);
    if text.trim().is_empty() && images.is_empty() { return Ok(json!({"stopReason":"end_turn"})); }
    s.cancel.store(false, Ordering::Relaxed);

    let user = if images.is_empty() { Message::user(text.clone()) } else {
        let mut parts = vec![Content::text_part(&text)];
        for (mime, b64) in &images { parts.push(Content::image_part(mime, b64)); }
        Message::user_parts(parts)
    };
    let mut msgs = { s.msgs.lock().unwrap().clone() };
    let out = if s.prepared.external_backend() {
        s.prepared.run_once(&text, &s.workdir).await.map(|(t, st)| { msgs.push(Message::user(text.clone())); msgs.push(Message { role: "assistant".into(), content: Some(Content::Text(t.clone())), ..Default::default() }); (t, st) })
    } else {
        s.prepared.agent().run_turn_message(&mut msgs, &s.prepared.system, user).await
    };
    let stop = match &out {
        Ok((answer, stats)) => {
            // the final answer is streamed as deltas; send it once more only if nothing was streamed
            if !s.prepared.external_backend() && stats.turns == 0 {
                conn.notify("session/update", json!({"sessionId": sid, "update": {"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":answer}}}));
            }
            match stats.stop_reason.as_str() { "cancelled" => "cancelled", "max_turns" => "max_turn_requests", _ => "end_turn" }
        }
        Err(e) => {
            conn.notify("session/update", json!({"sessionId": sid, "update": {"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":format!("\n**error:** {e:#}\n")}}}));
            "refusal"
        }
    };
    *s.msgs.lock().unwrap() = msgs.clone();
    if let Ok(store) = crate::sessions::SessionStore::open() {
        let mut meta = s.meta.lock().unwrap().clone();
        if let Ok((_, st)) = &out { meta.prompt_tokens += st.prompt_tokens; meta.completion_tokens += st.completion_tokens; }
        let _ = store.save(&mut meta, &msgs);
        *s.meta.lock().unwrap() = meta;
    }
    Ok(json!({"stopReason": stop}))
}

/// ACP content blocks → (text, images). `resource`/`resource_link` blocks are inlined as context.
fn prompt_content(blocks: &Value, workdir: &std::path::Path) -> (String, Vec<(String, String)>) {
    let mut text = String::new();
    let mut images = Vec::new();
    let list = match blocks { Value::Array(a) => a.clone(), Value::String(s) => vec![json!({"type":"text","text":s})], _ => vec![] };
    for b in list {
        match b["type"].as_str().unwrap_or("") {
            "text" => { if !text.is_empty() { text.push('\n'); } text.push_str(b["text"].as_str().unwrap_or("")); }
            "image" => { if let (Some(d), Some(m)) = (b["data"].as_str(), b["mimeType"].as_str()) { images.push((m.to_string(), d.to_string())); } }
            "resource_link" => {
                let uri = b["uri"].as_str().unwrap_or("");
                let path = uri.strip_prefix("file://").unwrap_or(uri);
                let rel = std::path::Path::new(path).strip_prefix(workdir).map(|p| p.display().to_string()).unwrap_or_else(|_| path.to_string());
                text.push_str(&format!("\n@{rel}"));
            }
            "resource" => {
                let r = &b["resource"];
                let uri = r["uri"].as_str().unwrap_or("");
                if let Some(t) = r["text"].as_str() { text.push_str(&format!("\n\n--- {uri} ---\n{t}\n---\n")); }
            }
            _ => {}
        }
    }
    (text, images)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_blocks() {
        let wd = std::path::Path::new("/proj");
        let (t, imgs) = prompt_content(&json!([
            {"type":"text","text":"fix this"},
            {"type":"resource_link","uri":"file:///proj/src/main.rs"},
            {"type":"resource","resource":{"uri":"file:///proj/notes.md","text":"be careful"}},
            {"type":"image","data":"AAAA","mimeType":"image/png"}
        ]), wd);
        assert!(t.contains("fix this") && t.contains("@src/main.rs") && t.contains("be careful"), "{t}");
        assert_eq!(imgs, vec![("image/png".to_string(), "AAAA".to_string())]);
        let (t2, _) = prompt_content(&json!("plain string"), wd);
        assert_eq!(t2, "plain string");
    }

    #[test]
    fn tool_kinds() {
        assert_eq!(tool_kind("read_file"), "read");
        assert_eq!(tool_kind("edit_file"), "edit");
        assert_eq!(tool_kind("bash"), "execute");
        assert_eq!(tool_kind("mcp__x__y"), "other");
    }
}
