//! Minimal LSP client (JSON-RPC over stdio, Content-Length framing). One server per (language, workdir),
//! started lazily. Supports diagnostics (publishDiagnostics), definition, references, hover, symbols.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspServerConfig { pub command: String, #[serde(default)] pub args: Vec<String>, pub exts: Vec<String>, #[serde(default)] pub language_ids: HashMap<String, String> }

pub fn default_servers() -> HashMap<String, LspServerConfig> {
    let mut m = HashMap::new();
    m.insert("rust".into(), LspServerConfig { command: "rust-analyzer".into(), args: vec![], exts: vec!["rs".into()], language_ids: HashMap::new() });
    m.insert("python".into(), LspServerConfig { command: "pyright-langserver".into(), args: vec!["--stdio".into()], exts: vec!["py".into()], language_ids: HashMap::new() });
    m.insert("typescript".into(), LspServerConfig { command: "typescript-language-server".into(), args: vec!["--stdio".into()], exts: vec!["ts".into(), "tsx".into(), "js".into(), "jsx".into(), "mjs".into()], language_ids: HashMap::new() });
    m.insert("go".into(), LspServerConfig { command: "gopls".into(), args: vec![], exts: vec!["go".into()], language_ids: HashMap::new() });
    m
}

pub fn language_id(ext: &str) -> &'static str {
    match ext { "rs" => "rust", "py" => "python", "ts" => "typescript", "tsx" => "typescriptreact", "js" | "mjs" | "cjs" => "javascript", "jsx" => "javascriptreact", "go" => "go", "c" => "c", "cpp" | "cc" => "cpp", "java" => "java", "rb" => "ruby", _ => "plaintext" }
}

pub struct LspServer {
    pub name: String,
    pub root: PathBuf,
    child: Mutex<Option<Child>>,
    stdin: tokio::sync::Mutex<ChildStdin>,
    next_id: AtomicU64,
    pending: Mutex<HashMap<u64, tokio::sync::oneshot::Sender<Value>>>,
    diagnostics: Mutex<HashMap<String, Vec<Value>>>,
    diag_notify: tokio::sync::Notify,
    open_docs: Mutex<HashMap<String, i64>>, // uri → version
}

static SERVERS: OnceLock<Mutex<HashMap<String, Arc<LspServer>>>> = OnceLock::new();
fn table() -> &'static Mutex<HashMap<String, Arc<LspServer>>> { SERVERS.get_or_init(|| Mutex::new(HashMap::new())) }
/// Serializes server start-up so concurrent first calls never spawn the same server twice.
static STARTING: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

fn uri_of(p: &Path) -> String { format!("file://{}", p.display()) }

impl LspServer {
    /// An already-running server for (language, root) — never starts one (post-edit diagnostics).
    pub fn running(name: &str, root: &Path) -> Option<Arc<LspServer>> {
        table().lock().ok()?.get(&format!("{name}@{}", root.display())).cloned()
    }

    pub async fn get_or_start(name: &str, cfg: &LspServerConfig, root: &Path) -> Result<Arc<LspServer>> {
        let key = format!("{name}@{}", root.display());
        if let Some(s) = table().lock().unwrap().get(&key) { return Ok(s.clone()); }
        let _starting = STARTING.get_or_init(|| tokio::sync::Mutex::new(())).lock().await;
        if let Some(s) = table().lock().unwrap().get(&key) { return Ok(s.clone()); }
        let mut c = Command::new(&cfg.command);
        c.args(&cfg.args).current_dir(root).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).kill_on_drop(true);
        c.env("PATH", crate::setup::path_with_bin_dir(root));
        let mut child = c.spawn().with_context(|| format!("lsp '{name}': cannot start `{}` (install it: see `harness setup`)", cfg.command))?;
        let stdin = child.stdin.take().context("no stdin")?;
        let stdout = child.stdout.take().context("no stdout")?;
        let server = Arc::new(LspServer { name: name.into(), root: root.to_path_buf(), child: Mutex::new(Some(child)), stdin: tokio::sync::Mutex::new(stdin), next_id: AtomicU64::new(1), pending: Mutex::new(HashMap::new()), diagnostics: Mutex::new(HashMap::new()), diag_notify: tokio::sync::Notify::new(), open_docs: Mutex::new(HashMap::new()) });
        // reader task
        let s2 = server.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut len = 0usize;
                loop {
                    let mut line = String::new();
                    let n = match reader.read_line(&mut line).await { Ok(n) => n, Err(_) => return };
                    if n == 0 { return; }
                    let l = line.trim();
                    if l.is_empty() { break; }
                    if let Some(v) = l.strip_prefix("Content-Length:") { len = v.trim().parse().unwrap_or(0); }
                }
                let mut buf = vec![0u8; len];
                if reader.read_exact(&mut buf).await.is_err() { return; }
                let Ok(msg) = serde_json::from_slice::<Value>(&buf) else { continue };
                if let Some(id) = msg.get("id").and_then(|i| i.as_u64()) {
                    if msg.get("method").is_some() {
                        // server → client request: answer politely
                        let method = msg["method"].as_str().unwrap_or("");
                        let result = match method { "workspace/configuration" => json!([null]), "client/registerCapability" | "client/unregisterCapability" => Value::Null, "window/workDoneProgress/create" => Value::Null, "workspace/workspaceFolders" => json!([{"uri": uri_of(&s2.root), "name": "root"}]), _ => Value::Null };
                        let _ = s2.send(json!({"jsonrpc": "2.0", "id": id, "result": result})).await;
                    } else if let Some(tx) = s2.pending.lock().unwrap().remove(&id) { let _ = tx.send(msg); }
                } else if msg["method"] == "textDocument/publishDiagnostics" {
                    let uri = msg["params"]["uri"].as_str().unwrap_or("").to_string();
                    let diags = msg["params"]["diagnostics"].as_array().cloned().unwrap_or_default();
                    s2.diagnostics.lock().unwrap().insert(uri, diags);
                    s2.diag_notify.notify_waiters();
                }
            }
        });
        // initialize
        let init = server.request("initialize", json!({
            "processId": std::process::id(), "rootUri": uri_of(root), "workspaceFolders": [{"uri": uri_of(root), "name": "root"}],
            "capabilities": {"textDocument": {"publishDiagnostics": {"relatedInformation": true}, "hover": {"contentFormat": ["plaintext", "markdown"]}, "definition": {}, "references": {}, "documentSymbol": {"hierarchicalDocumentSymbolSupport": true}}, "workspace": {"configuration": true, "workspaceFolders": true}},
            "initializationOptions": {}
        }), Duration::from_secs(60)).await.with_context(|| format!("lsp '{name}': initialize failed"))?;
        let _ = init;
        server.notify("initialized", json!({})).await?;
        table().lock().unwrap().insert(key, server.clone());
        Ok(server)
    }

    async fn send(&self, msg: Value) -> Result<()> {
        let body = msg.to_string();
        let mut w = self.stdin.lock().await;
        w.write_all(format!("Content-Length: {}\r\n\r\n{}", body.len(), body).as_bytes()).await?;
        w.flush().await?;
        Ok(())
    }
    pub async fn notify(&self, method: &str, params: Value) -> Result<()> { self.send(json!({"jsonrpc": "2.0", "method": method, "params": params})).await }
    pub async fn request(&self, method: &str, params: Value, timeout: Duration) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})).await?;
        let msg = match tokio::time::timeout(timeout, rx).await {
            Ok(r) => r?,
            Err(_) => { self.pending.lock().unwrap().remove(&id); bail!("lsp '{}': timeout waiting for {method}", self.name) }
        };
        if let Some(e) = msg.get("error") { bail!("lsp error: {e}"); }
        Ok(msg.get("result").cloned().unwrap_or(Value::Null))
    }
    /// Open (or re-sync) a document with its current on-disk content.
    pub async fn sync_doc(&self, path: &Path) -> Result<String> {
        let uri = uri_of(path);
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let version: Option<i64> = { let mut docs = self.open_docs.lock().unwrap(); match docs.get_mut(&uri) { Some(v) => { *v += 1; Some(*v) } None => { docs.insert(uri.clone(), 1); None } } };
        match version {
            Some(ver) => self.notify("textDocument/didChange", json!({"textDocument": {"uri": uri, "version": ver}, "contentChanges": [{"text": text}]})).await?,
            None => self.notify("textDocument/didOpen", json!({"textDocument": {"uri": uri, "languageId": language_id(ext), "version": 1, "text": text}})).await?,
        }
        // servers like rust-analyzer run their checker (cargo check) on save
        self.notify("textDocument/didSave", json!({"textDocument": {"uri": uri}, "text": text})).await?;
        Ok(uri)
    }
    /// Wait for diagnostics for `uri`: returns as soon as a non-empty set arrives, or — after an empty
    /// publish — waits `settle` more for a follow-up (checkers like cargo check publish later), then returns.
    pub async fn wait_diagnostics(&self, uri: &str, timeout: Duration, settle: Duration) -> Vec<Value> {
        let deadline = tokio::time::Instant::now() + timeout;
        { self.diagnostics.lock().unwrap().remove(uri); }
        let mut settle_deadline: Option<tokio::time::Instant> = None;
        loop {
            let cur = { self.diagnostics.lock().unwrap().get(uri).cloned() };
            if let Some(d) = &cur { if !d.is_empty() { return d.clone(); } if settle_deadline.is_none() { settle_deadline = Some(tokio::time::Instant::now() + settle); } }
            let until = settle_deadline.map(|s| s.min(deadline)).unwrap_or(deadline);
            let notified = self.diag_notify.notified();
            if tokio::time::timeout_at(until, notified).await.is_err() { let d = { self.diagnostics.lock().unwrap().get(uri).cloned() }; return d.unwrap_or_default(); }
        }
    }
    pub fn all_diagnostics(&self) -> HashMap<String, Vec<Value>> { self.diagnostics.lock().unwrap().clone() }
    pub async fn shutdown(&self) { let _ = self.request("shutdown", Value::Null, Duration::from_secs(5)).await; let _ = self.notify("exit", Value::Null).await; let child = { self.child.lock().unwrap().take() }; if let Some(mut c) = child { let _ = c.start_kill(); } }
}

pub fn server_for(path: &Path, servers: &HashMap<String, LspServerConfig>) -> Option<(String, LspServerConfig)> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    servers.iter().find(|(_, c)| c.exts.contains(&ext)).map(|(n, c)| (n.clone(), c.clone()))
}

pub fn fmt_location(v: &Value, root: &Path) -> String {
    let uri = v["uri"].as_str().or_else(|| v["targetUri"].as_str()).unwrap_or("");
    let range = if v.get("targetSelectionRange").is_some() { &v["targetSelectionRange"] } else { &v["range"] };
    let p = uri.strip_prefix("file://").unwrap_or(uri);
    let p = Path::new(p).strip_prefix(root).map(|r| r.display().to_string()).unwrap_or(p.to_string());
    format!("{}:{}:{}", p, range["start"]["line"].as_u64().unwrap_or(0) + 1, range["start"]["character"].as_u64().unwrap_or(0) + 1)
}

pub fn fmt_diag(uri: &str, d: &Value, root: &Path) -> String {
    let sev = match d["severity"].as_u64().unwrap_or(1) { 1 => "error", 2 => "warning", 3 => "info", _ => "hint" };
    let p = uri.strip_prefix("file://").unwrap_or(uri);
    let p = Path::new(p).strip_prefix(root).map(|r| r.display().to_string()).unwrap_or(p.to_string());
    format!("{}:{}:{}: {sev}: {}{}", p, d["range"]["start"]["line"].as_u64().unwrap_or(0) + 1, d["range"]["start"]["character"].as_u64().unwrap_or(0) + 1, d["message"].as_str().unwrap_or("").lines().next().unwrap_or(""), d["code"].as_str().map(|c| format!(" [{c}]")).unwrap_or_default())
}
