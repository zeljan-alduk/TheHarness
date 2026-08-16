//! `harness serve`: a small local HTTP server that hosts the same web UI as the Tauri app.
//! Commands: POST /api/invoke {cmd,args} · events: GET /api/events (SSE) · static UI embedded.
//! Hand-rolled HTTP/1.1 on tokio (no extra deps); binds 127.0.0.1 by default.

use crate::agent::Agent;
use crate::events::{Event, Sink};
use crate::llm::Client;
use crate::tools::{Registry, ToolCtx};
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;

const INDEX: &str = include_str!("../ui/dist/index.html");
const APP_JS: &str = include_str!("../ui/dist/app.js");
const STYLE: &str = include_str!("../ui/dist/style.css");
const SHIM: &str = r#"
(function(){
  const es = new EventSource('/api/events');
  const listeners = {};
  es.onmessage = (e) => { try { const m = JSON.parse(e.data); (listeners[m.name]||[]).forEach(cb => cb({payload: m.payload})); } catch(err) {} };
  window.__TAURI__ = {
    core: { invoke: async (cmd, args) => { const r = await fetch('/api/invoke', {method:'POST', headers:{'Content-Type':'application/json'}, body: JSON.stringify({cmd, args: args||{}})}); const j = await r.json(); if (j.error) throw j.error; return j.result; } },
    event: { listen: async (name, cb) => { (listeners[name] ||= []).push(cb); return () => {}; } },
    dialog: { open: async (o) => window.prompt('Directory path:', (o && o.defaultPath) || '') }
  };
})();
"#;

struct Shared {
    cfg: crate::config::Config,
    events: broadcast::Sender<String>,
    run: Mutex<Option<tokio::task::JoinHandle<()>>>,
    asks: Mutex<std::collections::HashMap<u64, tokio::sync::oneshot::Sender<crate::permissions::Approval>>>,
    next_ask: Mutex<u64>,
}

struct SseSink { tx: broadcast::Sender<String> }
impl Sink for SseSink { fn emit(&self, e: &Event) { let _ = self.tx.send(json!({"name": "agent-event", "payload": e}).to_string()); } }

struct WebApprover { shared: Arc<Shared> }
#[async_trait::async_trait]
impl crate::permissions::Approver for WebApprover {
    async fn ask(&self, req: crate::permissions::ApprovalRequest) -> crate::permissions::Approval {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let id = { let mut n = self.shared.next_ask.lock().unwrap(); *n += 1; *n };
        self.shared.asks.lock().unwrap().insert(id, tx);
        let _ = self.shared.events.send(json!({"name": "permission-ask", "payload": {"id": id, "tool": req.tool, "summary": req.summary, "rule": req.suggested_rule, "reason": req.reason}}).to_string());
        rx.await.unwrap_or(crate::permissions::Approval::Deny)
    }
}

pub async fn serve(cfg: crate::config::Config, bind: &str) -> Result<()> {
    let listener = TcpListener::bind(bind).await.with_context(|| format!("binding {bind}"))?;
    let (tx, _) = broadcast::channel::<String>(1024);
    let shared = Arc::new(Shared { cfg, events: tx, run: Mutex::new(None), asks: Mutex::new(Default::default()), next_ask: Mutex::new(0) });
    eprintln!("harness web UI on http://{bind}/  (ctrl+c to stop)");
    loop {
        let (sock, _) = listener.accept().await?;
        let sh = shared.clone();
        tokio::spawn(async move { let _ = handle(sock, sh).await; });
    }
}

async fn handle(mut sock: TcpStream, sh: Arc<Shared>) -> Result<()> {
    let (r, mut w) = sock.split();
    let mut reader = BufReader::new(r);
    let mut line = String::new();
    reader.read_line(&mut line).await?;
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("/").to_string();
    let mut content_length = 0usize;
    loop { let mut h = String::new(); reader.read_line(&mut h).await?; if h == "\r\n" || h == "\n" || h.is_empty() { break; } if let Some(v) = h.to_ascii_lowercase().strip_prefix("content-length:") { content_length = v.trim().parse().unwrap_or(0); } }
    let mut body = vec![0u8; content_length];
    if content_length > 0 { reader.read_exact(&mut body).await?; }
    fn respond(status: &str, ctype: &str, body: &[u8]) -> Vec<u8> {
        let head = format!("HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n", body.len());
        let mut v = head.into_bytes(); v.extend_from_slice(body); v
    }
    match (method.as_str(), path.split('?').next().unwrap_or("/")) {
        ("GET", "/") | ("GET", "/index.html") => { let html = INDEX.replace("<script src=\"app.js\"></script>", "<script src=\"shim.js\"></script><script src=\"app.js\"></script>"); w.write_all(&respond("200 OK", "text/html; charset=utf-8", html.as_bytes())).await?; }
        ("GET", "/app.js") => { w.write_all(&respond("200 OK", "application/javascript", APP_JS.as_bytes())).await?; }
        ("GET", "/shim.js") => { w.write_all(&respond("200 OK", "application/javascript", SHIM.as_bytes())).await?; }
        ("GET", "/style.css") => { w.write_all(&respond("200 OK", "text/css", STYLE.as_bytes())).await?; }
        ("GET", "/api/events") => {
            w.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n\r\n").await?;
            w.write_all(b": connected\n\n").await?;
            let mut rx = sh.events.subscribe();
            loop {
                match tokio::time::timeout(Duration::from_secs(15), rx.recv()).await {
                    Ok(Ok(msg)) => { if w.write_all(format!("data: {msg}\n\n").as_bytes()).await.is_err() { break; } }
                    Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
                    Ok(Err(_)) => break,
                    Err(_) => { if w.write_all(b": ping\n\n").await.is_err() { break; } }
                }
            }
        }
        ("POST", "/api/invoke") => {
            let req: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
            let cmd = req["cmd"].as_str().unwrap_or("").to_string();
            let args = req["args"].clone();
            let out = match invoke(&sh, &cmd, args).await { Ok(v) => json!({"result": v}), Err(e) => json!({"error": format!("{e:#}")}) };
            w.write_all(&respond("200 OK", "application/json", out.to_string().as_bytes())).await?;
        }
        _ => { w.write_all(&respond("404 Not Found", "text/plain", b"not found")).await?; }
    }
    Ok(())
}

async fn invoke(sh: &Arc<Shared>, cmd: &str, args: Value) -> Result<Value> {
    let cfg = &sh.cfg;
    match cmd {
        "get_config" => { let mut v = serde_json::to_value(cfg)?; v["cwd"] = json!(std::env::current_dir().map(|p| p.display().to_string()).unwrap_or_default()); v["home"] = json!(std::env::var("HOME").unwrap_or_default()); Ok(v) }
        "list_models" => { let c = Client::new(&cfg.llm)?; Ok(json!(c.list_models().await?)) }
        "list_dir" => {
            let path = args["path"].as_str().unwrap_or(".");
            let mut out = Vec::new();
            for e in std::fs::read_dir(path)? { let e = e?; let md = e.metadata()?; let name = e.file_name().to_string_lossy().to_string(); if matches!(name.as_str(), ".git" | "target" | "node_modules") { continue; } out.push(json!({"name": name, "path": e.path().display().to_string(), "is_dir": md.is_dir(), "size": md.len()})); }
            out.sort_by(|a, b| b["is_dir"].as_bool().cmp(&a["is_dir"].as_bool()).then(a["name"].as_str().unwrap_or("").to_lowercase().cmp(&b["name"].as_str().unwrap_or("").to_lowercase())));
            Ok(json!(out))
        }
        "read_file" => {
            let p = PathBuf::from(args["path"].as_str().unwrap_or(""));
            let md = std::fs::metadata(&p)?;
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
            let (kind, mime) = match ext.as_str() { "png" => ("image", "image/png"), "jpg" | "jpeg" => ("image", "image/jpeg"), "gif" => ("image", "image/gif"), "webp" => ("image", "image/webp"), "svg" => ("image", "image/svg+xml"), "mp3" => ("audio", "audio/mpeg"), "wav" => ("audio", "audio/wav"), "m4a" => ("audio", "audio/mp4"), "mp4" => ("video", "video/mp4"), "webm" => ("video", "video/webm"), "mov" => ("video", "video/quicktime"), "pdf" => ("pdf", "application/pdf"), _ => ("text", "text/plain") };
            if kind == "text" { if md.len() > 2 << 20 { return Ok(json!({"kind": "binary", "mime": mime, "size": md.len()})); } let bytes = std::fs::read(&p)?; if bytes.iter().take(8000).any(|b| *b == 0) { return Ok(json!({"kind": "binary", "mime": "application/octet-stream", "size": md.len()})); } return Ok(json!({"kind": "text", "mime": mime, "size": md.len(), "text": String::from_utf8_lossy(&bytes)})); }
            if md.len() > 64 << 20 { return Ok(json!({"kind": "binary", "mime": mime, "size": md.len()})); }
            use base64::Engine; let b64 = base64::engine::general_purpose::STANDARD.encode(std::fs::read(&p)?);
            Ok(json!({"kind": kind, "mime": mime, "size": md.len(), "data_url": format!("data:{mime};base64,{b64}")}))
        }
        "git_log" => { let o = crate::sandbox::run_shell("git log --oneline --decorate -20 2>&1; echo; git status --short 2>&1 | head -40", std::path::Path::new(args["workdir"].as_str().unwrap_or(".")), Duration::from_secs(10), 8000).await?; Ok(json!(o.stdout)) }
        "stop_run" => { let h = sh.run.lock().unwrap().take(); if let Some(h) = h { h.abort(); let _ = sh.events.send(json!({"name": "run-finished", "payload": {"ok": false, "text": "", "error": "stopped by user"}}).to_string()); Ok(json!(true)) } else { Ok(json!(false)) } }
        "answer_permission" => { let id = args["id"].as_u64().unwrap_or(0); let tx = sh.asks.lock().unwrap().remove(&id).context("unknown prompt")?; let a = match args["decision"].as_str().unwrap_or("deny") { "once" | "yes" => crate::permissions::Approval::Once, "always" => crate::permissions::Approval::Always, _ => crate::permissions::Approval::Deny }; let _ = tx.send(a); Ok(json!(null)) }
        "start_run" => {
            { let g = sh.run.lock().unwrap(); if let Some(h) = g.as_ref() { if !h.is_finished() { anyhow::bail!("a run is already in progress"); } } }
            let task = args["task"].as_str().unwrap_or("").to_string();
            let workdir = PathBuf::from(args["workdir"].as_str().unwrap_or(".")).canonicalize().context("workdir")?;
            let mut cfg = cfg.clone();
            if let Some(m) = args["model"].as_str() { if !m.is_empty() { cfg.llm.model = m.to_string(); } }
            if let Some(n) = args["maxTurns"].as_u64() { cfg.agent.max_turns = n as usize; }
            if let Some(n) = args["net"].as_bool() { cfg.net.enabled = n; }
            let sh2 = sh.clone();
            let handle = tokio::spawn(async move {
                let res: std::result::Result<String, String> = async {
                    let client = Client::new(&cfg.llm).map_err(|e| e.to_string())?;
                    let store = if cfg.memory.enabled { crate::memory::MemoryStore::open(&cfg.memory).ok() } else { None };
                    if let Some(m) = &store { let _ = m.touch_project(&workdir); }
                    let ts = crate::tools::build_toolset(cfg.net.enabled, &workdir, true).await;
                    let registry: Registry = ts.registry.clone();
                    let sink: Arc<dyn Sink> = Arc::new(SseSink { tx: sh2.events.clone() });
                    let budget = cfg.llm.effective_budget(crate::llm::detect_context_length(&cfg.llm.base_url, &cfg.llm.model).await.map(|d| d.0));
                    let mut pcfg = cfg.permissions.clone(); pcfg.allow.extend(crate::permissions::persisted_rules());
                    let policy = Arc::new(crate::permissions::Policy::new(pcfg, &workdir));
                    let approver: Arc<dyn crate::permissions::Approver> = Arc::new(WebApprover { shared: sh2.clone() });
                    let env = Arc::new(crate::agent::SubAgentEnv::new(client.clone(), registry.clone(), policy.clone(), approver.clone(), sink.clone(), budget, true));
                    let ctx = ToolCtx { workdir: workdir.clone(), timeout: Duration::from_secs(cfg.agent.tool_timeout_secs), max_output: cfg.agent.max_tool_output_chars, net: cfg.net.clone(), memory: store.clone(), subagent: Some(env), redact_secrets: cfg.security.redact_secrets, hooks: cfg.hooks.clone(), todos: Default::default(), lsp_servers: cfg.lsp.servers.clone(), extra_roots: vec![], approver: Some(approver.clone()), inbox: Default::default(), cancel: None };
                    let system = crate::agent::system_prompt_with_memory(&workdir.display().to_string(), &registry.names(), Some(&ts.prompt_extra), store.as_ref());
                    let agent = Agent { client: &client, registry: &registry, ctx: &ctx, max_turns: cfg.agent.max_turns, context_budget: budget, sink: sink.as_ref(), stream: true, policy: &policy, approver: approver.as_ref() };
                    agent.run(&system, &task).await.map(|(t, _)| t).map_err(|e| format!("{e:#}"))
                }.await;
                let payload = match res { Ok(text) => json!({"ok": true, "text": text, "error": null}), Err(e) => json!({"ok": false, "text": "", "error": e}) };
                let _ = sh2.events.send(json!({"name": "run-finished", "payload": payload}).to_string());
            });
            *sh.run.lock().unwrap() = Some(handle);
            Ok(json!(null))
        }
        _ => anyhow::bail!("unknown command {cmd}"),
    }
}
