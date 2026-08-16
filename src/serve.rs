//! `harness serve`: a small local HTTP server that hosts the same web UI as the Tauri app.
//! Commands: POST /api/invoke {cmd,args} · events: GET /api/events (SSE) · static UI embedded.
//! Hand-rolled HTTP/1.1 on tokio (no extra deps); binds 127.0.0.1 by default.
//! Auth: a per-launch random token (printed in the URL) must accompany every /api call
//! (`X-Harness-Token` header, or `?token=` for the SSE stream); Host/Origin are pinned to the local origin.

use crate::events::{Event, Sink};
use crate::llm::Client;
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;

const INDEX: &str = include_str!("../ui/dist/index.html");
const APP_JS: &str = include_str!("../ui/dist/app.js");
const STYLE: &str = include_str!("../ui/dist/style.css");
const MAX_BODY: usize = 8 << 20;
const SHIM: &str = r#"
(function(){
  const qs = new URLSearchParams(location.search).get('token');
  if (qs) { sessionStorage.setItem('harness_token', qs); history.replaceState(null, '', location.pathname); }
  const token = sessionStorage.getItem('harness_token') || '';
  const es = new EventSource('/api/events?token=' + encodeURIComponent(token));
  const listeners = {};
  es.onmessage = (e) => { try { const m = JSON.parse(e.data); (listeners[m.name]||[]).forEach(cb => cb({payload: m.payload})); } catch(err) {} };
  window.__TAURI__ = {
    core: { invoke: async (cmd, args) => { const r = await fetch('/api/invoke', {method:'POST', headers:{'Content-Type':'application/json', 'X-Harness-Token': token}, body: JSON.stringify({cmd, args: args||{}})}); if (r.status === 401) throw 'unauthorized: open the URL printed by `harness serve` (it carries the access token)'; const j = await r.json(); if (j.error) throw j.error; return j.result; } },
    event: { listen: async (name, cb) => { (listeners[name] ||= []).push(cb); return () => {}; } },
    dialog: { open: async (o) => window.prompt('Directory path:', (o && o.defaultPath) || '') }
  };
})();
"#;

struct Shared {
    cfg: crate::config::Config,
    token: String,
    /// Directories the file browser may read: the launch cwd plus every run's workdir.
    roots: Mutex<Vec<PathBuf>>,
    events: broadcast::Sender<String>,
    run: Mutex<Option<tokio::task::JoinHandle<()>>>,
    asks: Mutex<std::collections::HashMap<u64, tokio::sync::oneshot::Sender<crate::permissions::Approval>>>,
    next_ask: Mutex<u64>, allow_remote: bool }

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

/// A LAN URL for this bind address (so the printed link works from a phone), if we can find one.
fn lan_address(bind: &str) -> Option<String> {
    let port = bind.rsplit(':').next().unwrap_or("7878").to_string();
    let host = bind.rsplit_once(':').map(|(h, _)| h).unwrap_or("");
    if !matches!(host, "0.0.0.0" | "::" | "[::]") { return None; }
    // ask the OS for the address that would be used to reach the internet
    let out = std::process::Command::new("sh").arg("-c").arg("ipconfig getifaddr en0 2>/dev/null || hostname -I 2>/dev/null | awk '{print $1}'").output().ok()?;
    let ip = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!ip.is_empty()).then(|| format!("http://{ip}:{port}"))
}

/// Print a QR code for the URL when `qrencode` is around — scanning beats typing a token on a phone.
fn print_qr(url: &str) {
    if crate::setup::which("qrencode").is_none() { return; }
    if let Ok(o) = std::process::Command::new("qrencode").args(["-t", "ANSIUTF8", "-m", "1", url]).output() {
        if o.status.success() { eprint!("{}", String::from_utf8_lossy(&o.stdout)); }
    }
}

/// 128-bit random hex token: /dev/urandom, else sha256(time ‖ pid ‖ stack address).
fn new_token() -> String {
    let mut buf = [0u8; 16];
    let ok = std::fs::File::open("/dev/urandom").and_then(|mut f| std::io::Read::read_exact(&mut f, &mut buf)).is_ok();
    if !ok {
        use sha2::Digest;
        let mut h = sha2::Sha256::new();
        h.update(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0).to_le_bytes());
        h.update(std::process::id().to_le_bytes());
        h.update((&buf as *const _ as usize).to_le_bytes());
        buf.copy_from_slice(&h.finalize()[..16]);
    }
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// `path` canonicalized and inside one of `roots`, else an error (file browser sandbox).
fn allowed_path(sh: &Shared, p: &str) -> Result<PathBuf> {
    let full = Path::new(p).canonicalize().with_context(|| format!("{p}: not found"))?;
    let roots = sh.roots.lock().unwrap();
    if roots.iter().any(|r| full.starts_with(r)) { Ok(full) } else { anyhow::bail!("{p}: outside the allowed directories ({})", roots.iter().map(|r| r.display().to_string()).collect::<Vec<_>>().join(", ")) }
}

pub async fn serve(cfg: crate::config::Config, bind: &str) -> Result<()> { serve_with(cfg, bind, false).await }

/// `allow_remote` lifts the loopback Host/Origin pin so another machine (or a phone) can attach with
/// the token. Only do that on a network you trust: the token is the only thing between the outside and
/// an agent with tools.
pub async fn serve_with(cfg: crate::config::Config, bind: &str, allow_remote: bool) -> Result<()> {
    let listener = TcpListener::bind(bind).await.with_context(|| format!("binding {bind}"))?;
    let (tx, _) = broadcast::channel::<String>(1024);
    let token = new_token();
    let cwd = std::env::current_dir()?.canonicalize()?;
    let shared = Arc::new(Shared { cfg, token: token.clone(), roots: Mutex::new(vec![cwd]), events: tx, run: Mutex::new(None), asks: Mutex::new(Default::default()), next_ask: Mutex::new(0), allow_remote });
    let url = format!("http://{bind}/?token={token}");
    eprintln!("harness web UI on {url}  (ctrl+c to stop; the token is required)");
    if allow_remote {
        let lan = lan_address(bind);
        eprintln!("remote access is ON — attach from another machine with:\n  harness attach {}", lan.as_deref().unwrap_or(&url));
        if let Some(u) = &lan { print_qr(u); }
    }
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
    let (mut content_length, mut host, mut origin, mut token) = (0usize, String::new(), None::<String>, String::new());
    loop {
        let mut h = String::new(); reader.read_line(&mut h).await?;
        if h == "\r\n" || h == "\n" || h.is_empty() { break; }
        let Some((k, v)) = h.split_once(':') else { continue };
        let v = v.trim();
        match k.trim().to_ascii_lowercase().as_str() { "content-length" => content_length = v.parse().unwrap_or(0), "host" => host = v.to_string(), "origin" => origin = Some(v.to_string()), "x-harness-token" => token = v.to_string(), "authorization" => token = v.strip_prefix("Bearer ").unwrap_or(v).to_string(), _ => {} }
    }
    fn respond(status: &str, ctype: &str, body: &[u8]) -> Vec<u8> {
        let head = format!("HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n", body.len());
        let mut v = head.into_bytes(); v.extend_from_slice(body); v
    }
    // Local-origin pinning: Host must be a loopback name; a present Origin must be our own origin (blocks CSRF / DNS rebinding).
    let host_name = host.rsplit_once(':').map(|(h, _)| h).unwrap_or(&host).trim_matches(|c| c == '[' || c == ']');
    if !sh.allow_remote && (!matches!(host_name, "localhost" | "127.0.0.1" | "::1") || origin.as_deref().is_some_and(|o| o != format!("http://{host}"))) {
        w.write_all(&respond("403 Forbidden", "text/plain", b"forbidden: bad Host/Origin")).await?; return Ok(());
    }
    if content_length > MAX_BODY { w.write_all(&respond("413 Payload Too Large", "text/plain", b"body too large")).await?; return Ok(()); }
    let mut body = vec![0u8; content_length];
    if content_length > 0 { reader.read_exact(&mut body).await?; }
    let (route, query) = path.split_once('?').unwrap_or((&path, ""));
    if route.starts_with("/api/") {
        if token.is_empty() { token = query.split('&').find_map(|kv| kv.strip_prefix("token=")).unwrap_or("").to_string(); }
        if token != sh.token { w.write_all(&respond("401 Unauthorized", "text/plain", b"unauthorized: missing or invalid token")).await?; return Ok(()); }
    }
    match (method.as_str(), route) {
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
                    Ok(Err(broadcast::error::RecvError::Lagged(n))) => { if w.write_all(format!("data: {}\n\n", json!({"name": "warning", "payload": format!("events dropped ({n})")})).as_bytes()).await.is_err() { break; } }
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
        // webhook trigger: POST /api/hook/<job> (token-protected like every /api route) runs a scheduled job now
        ("POST", r) if r.starts_with("/api/hook/") => {
            let name = r.trim_start_matches("/api/hook/").to_string();
            let store = crate::scheduler::Store::open();
            match store.as_ref().ok().and_then(|st| st.get(&name)) {
                Some(job) => {
                    let (cfg, id) = (sh.cfg.clone(), job.id.clone());
                    tokio::spawn(async move {
                        if let Ok(store) = crate::scheduler::Store::open() {
                            let r = crate::scheduler::run_job(&cfg, &store, &job).await;
                            eprintln!("· webhook '{id}': {}", match r { Ok(t) => crate::llm::truncate_for_log(t.trim(), 160), Err(e) => format!("failed: {e:#}") });
                        }
                    });
                    w.write_all(&respond("202 Accepted", "application/json", json!({"started": name}).to_string().as_bytes())).await?;
                }
                None => { w.write_all(&respond("404 Not Found", "application/json", json!({"error": format!("no scheduled job '{name}'")}).to_string().as_bytes())).await?; }
            }
        }
        _ => { w.write_all(&respond("404 Not Found", "text/plain", b"not found")).await?; }
    }
    Ok(())
}

async fn invoke(sh: &Arc<Shared>, cmd: &str, args: Value) -> Result<Value> {
    let cfg = &sh.cfg;
    match cmd {
        "get_config" => { let mut v = serde_json::to_value(cfg)?; v["version"] = json!(crate::version()); v["cwd"] = json!(std::env::current_dir().map(|p| p.display().to_string()).unwrap_or_default()); v["home"] = json!(std::env::var("HOME").unwrap_or_default()); Ok(v) }
        "list_models" => { let c = Client::new(&cfg.llm)?; Ok(json!(c.list_models().await?)) }
        "list_dir" => {
            let path = allowed_path(sh, args["path"].as_str().unwrap_or("."))?;
            let mut out = Vec::new();
            for e in std::fs::read_dir(&path)? { let e = e?; let md = e.metadata()?; let name = e.file_name().to_string_lossy().to_string(); if matches!(name.as_str(), ".git" | "target" | "node_modules") { continue; } out.push(json!({"name": name, "path": e.path().display().to_string(), "is_dir": md.is_dir(), "size": md.len()})); }
            out.sort_by(|a, b| b["is_dir"].as_bool().cmp(&a["is_dir"].as_bool()).then(a["name"].as_str().unwrap_or("").to_lowercase().cmp(&b["name"].as_str().unwrap_or("").to_lowercase())));
            Ok(json!(out))
        }
        "read_file" => {
            let p = allowed_path(sh, args["path"].as_str().unwrap_or(""))?;
            let md = std::fs::metadata(&p)?;
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
            let (kind, mime) = match ext.as_str() { "png" => ("image", "image/png"), "jpg" | "jpeg" => ("image", "image/jpeg"), "gif" => ("image", "image/gif"), "webp" => ("image", "image/webp"), "svg" => ("image", "image/svg+xml"), "mp3" => ("audio", "audio/mpeg"), "wav" => ("audio", "audio/wav"), "m4a" => ("audio", "audio/mp4"), "mp4" => ("video", "video/mp4"), "webm" => ("video", "video/webm"), "mov" => ("video", "video/quicktime"), "pdf" => ("pdf", "application/pdf"), _ => ("text", "text/plain") };
            if kind == "text" { if md.len() > 2 << 20 { return Ok(json!({"kind": "binary", "mime": mime, "size": md.len()})); } let bytes = std::fs::read(&p)?; if bytes.iter().take(8000).any(|b| *b == 0) { return Ok(json!({"kind": "binary", "mime": "application/octet-stream", "size": md.len()})); } return Ok(json!({"kind": "text", "mime": mime, "size": md.len(), "text": String::from_utf8_lossy(&bytes)})); }
            if md.len() > 64 << 20 { return Ok(json!({"kind": "binary", "mime": mime, "size": md.len()})); }
            use base64::Engine; let b64 = base64::engine::general_purpose::STANDARD.encode(std::fs::read(&p)?);
            Ok(json!({"kind": kind, "mime": mime, "size": md.len(), "data_url": format!("data:{mime};base64,{b64}")}))
        }
        "git_log" => { let o = crate::sandbox::run_shell("git log --oneline --decorate -20 2>&1; echo; git status --short 2>&1 | head -40", &allowed_path(sh, args["workdir"].as_str().unwrap_or("."))?, Duration::from_secs(10), 8000).await?; Ok(json!(o.stdout)) }
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
            { let mut roots = sh.roots.lock().unwrap(); if !roots.contains(&workdir) { roots.push(workdir.clone()); } }
            let sh2 = sh.clone();
            let handle = tokio::spawn(async move {
                let sink: Arc<dyn Sink> = Arc::new(SseSink { tx: sh2.events.clone() });
                let approver: Arc<dyn crate::permissions::Approver> = Arc::new(WebApprover { shared: sh2.clone() });
                let res = crate::runner::start_run(crate::runner::RunSetup::new(cfg, workdir, sink, approver), task).await;
                let payload = match res { Ok(text) => json!({"ok": true, "text": text, "error": null}), Err(e) => json!({"ok": false, "text": "", "error": format!("{e:#}")}) };
                let _ = sh2.events.send(json!({"name": "run-finished", "payload": payload}).to_string());
            });
            *sh.run.lock().unwrap() = Some(handle);
            Ok(json!(null))
        }
        _ => anyhow::bail!("unknown command {cmd}"),
    }
}
