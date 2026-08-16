#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use base64::Engine;
use harness::agent::Agent;
use harness::config::Config;
use harness::events::{Event, Sink};
use harness::llm::Client;
use harness::tools::{Registry, ToolCtx};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};

/// Bridges core events to the webview.
struct TauriSink { app: AppHandle }
impl Sink for TauriSink {
    fn emit(&self, e: &Event) { let _ = self.app.emit("agent-event", e); }
}

#[derive(Default)]
struct RunState { handle: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>, asks: Mutex<std::collections::HashMap<u64, tokio::sync::oneshot::Sender<harness::permissions::Approval>>>, next_ask: Mutex<u64> }

/// Permission prompts go to the webview as `permission-ask`; the UI answers via `answer_permission`.
struct TauriApprover { app: AppHandle }
#[async_trait::async_trait]
impl harness::permissions::Approver for TauriApprover {
    async fn ask(&self, req: harness::permissions::ApprovalRequest) -> harness::permissions::Approval {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let state = self.app.state::<RunState>();
        let id = { let mut n = state.next_ask.lock().unwrap(); *n += 1; *n };
        state.asks.lock().unwrap().insert(id, tx);
        let _ = self.app.emit("permission-ask", &serde_json::json!({"id": id, "tool": req.tool, "summary": req.summary, "rule": req.suggested_rule, "reason": req.reason}));
        rx.await.unwrap_or(harness::permissions::Approval::Deny)
    }
}

#[tauri::command]
fn answer_permission(state: State<'_, RunState>, id: u64, decision: String) -> Result<(), String> {
    let tx = state.asks.lock().unwrap().remove(&id).ok_or("unknown prompt")?;
    let a = match decision.as_str() { "once" | "yes" => harness::permissions::Approval::Once, "always" => harness::permissions::Approval::Always, _ => harness::permissions::Approval::Deny };
    let _ = tx.send(a); Ok(())
}

#[derive(Serialize)]
struct RunFinished { ok: bool, text: String, error: Option<String> }

fn load_config() -> Result<Config, String> {
    let explicit = std::env::var("HARNESS_CONFIG").ok().map(PathBuf::from);
    Config::load(explicit.as_deref()).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
fn get_config() -> Result<serde_json::Value, String> {
    let cfg = load_config()?;
    let mut v = serde_json::to_value(&cfg).map_err(|e| e.to_string())?;
    v["cwd"] = serde_json::json!(std::env::current_dir().map(|p| p.display().to_string()).unwrap_or_default());
    v["home"] = serde_json::json!(std::env::var("HOME").unwrap_or_default());
    Ok(v)
}

#[tauri::command]
async fn list_models() -> Result<Vec<String>, String> {
    let cfg = load_config()?;
    let client = Client::new(&cfg.llm).map_err(|e| e.to_string())?;
    client.list_models().await.map_err(|e| format!("{e:#}"))
}

#[tauri::command]
async fn start_run(app: AppHandle, state: State<'_, RunState>, task: String, workdir: String, model: Option<String>, max_turns: Option<usize>, net: Option<bool>) -> Result<(), String> {
    let mut cfg = load_config()?;
    if let Some(m) = model { if !m.is_empty() { cfg.llm.model = m; } }
    if let Some(n) = max_turns { cfg.agent.max_turns = n; }
    if let Some(n) = net { cfg.net.enabled = n; }
    let workdir = PathBuf::from(&workdir).canonicalize().map_err(|e| format!("workdir: {e}"))?;
    if !workdir.is_dir() { return Err("workdir is not a directory".into()); }
    {
        let guard = state.handle.lock().unwrap();
        if let Some(h) = guard.as_ref() { if !h.inner().is_finished() { return Err("a run is already in progress".into()); } }
    }
    let app2 = app.clone();
    let handle = tauri::async_runtime::spawn(async move {
        let result: Result<String, String> = async {
            let client = Client::new(&cfg.llm).map_err(|e| e.to_string())?;
            let store = if cfg.memory.enabled { harness::memory::MemoryStore::open(&cfg.memory).ok() } else { None };
            if let Some(m) = &store { let _ = m.touch_project(&workdir); }
            let registry = Registry::defaults(cfg.net.enabled);
            let system = harness::agent::system_prompt_with_memory(&workdir.display().to_string(), &registry.names(), None, store.as_ref());
            let sink: std::sync::Arc<dyn Sink> = std::sync::Arc::new(TauriSink { app: app2.clone() });
            let budget = cfg.llm.effective_budget(harness::llm::detect_context_length(&cfg.llm.base_url, &cfg.llm.model).await.map(|d| d.0));
            let mut pcfg = cfg.permissions.clone(); pcfg.allow.extend(harness::permissions::persisted_rules());
            let policy = std::sync::Arc::new(harness::permissions::Policy::new(pcfg, &workdir));
            let approver: std::sync::Arc<dyn harness::permissions::Approver> = std::sync::Arc::new(TauriApprover { app: app2.clone() });
            let env = std::sync::Arc::new(harness::agent::SubAgentEnv::new(client.clone(), registry.clone(), policy.clone(), approver.clone(), sink.clone(), budget, true));
            let ctx = ToolCtx { workdir: workdir.clone(), timeout: Duration::from_secs(cfg.agent.tool_timeout_secs), max_output: cfg.agent.max_tool_output_chars, net: cfg.net.clone(), memory: store.clone(), subagent: Some(env), redact_secrets: cfg.security.redact_secrets, hooks: cfg.hooks.clone(), todos: Default::default(), lsp_servers: cfg.lsp.servers.clone() };
            let agent = Agent { client: &client, registry: &registry, ctx: &ctx, max_turns: cfg.agent.max_turns, context_budget: budget, sink: sink.as_ref(), stream: true, policy: &policy, approver: approver.as_ref() };
            agent.run(&system, &task).await.map(|(t, _)| t).map_err(|e| format!("{e:#}"))
        }.await;
        let payload = match result {
            Ok(text) => RunFinished { ok: true, text, error: None },
            Err(e) => RunFinished { ok: false, text: String::new(), error: Some(e) },
        };
        let _ = app2.emit("run-finished", &payload);
    });
    *state.handle.lock().unwrap() = Some(handle);
    Ok(())
}

#[tauri::command]
fn stop_run(app: AppHandle, state: State<'_, RunState>) -> Result<bool, String> {
    let mut guard = state.handle.lock().unwrap();
    if let Some(h) = guard.take() {
        h.abort();
        let _ = app.emit("run-finished", &RunFinished { ok: false, text: String::new(), error: Some("stopped by user".into()) });
        return Ok(true);
    }
    Ok(false)
}

#[derive(Serialize)]
struct DirEntry { name: String, path: String, is_dir: bool, size: u64 }

#[tauri::command]
fn list_dir(path: String) -> Result<Vec<DirEntry>, String> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(&path).map_err(|e| e.to_string())? {
        let e = e.map_err(|e| e.to_string())?;
        let md = e.metadata().map_err(|e| e.to_string())?;
        let name = e.file_name().to_string_lossy().to_string();
        if name == ".git" || name == "target" || name == "node_modules" { continue; }
        out.push(DirEntry { name, path: e.path().display().to_string(), is_dir: md.is_dir(), size: md.len() });
    }
    out.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    Ok(out)
}

#[derive(Serialize)]
struct FilePreview { kind: String, mime: String, size: u64, text: Option<String>, data_url: Option<String> }

fn mime_of(p: &Path) -> (&'static str, &'static str) {
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "png" => ("image", "image/png"), "jpg" | "jpeg" => ("image", "image/jpeg"), "gif" => ("image", "image/gif"),
        "webp" => ("image", "image/webp"), "svg" => ("image", "image/svg+xml"), "bmp" => ("image", "image/bmp"),
        "mp3" => ("audio", "audio/mpeg"), "wav" => ("audio", "audio/wav"), "ogg" => ("audio", "audio/ogg"),
        "m4a" => ("audio", "audio/mp4"), "flac" => ("audio", "audio/flac"), "aac" => ("audio", "audio/aac"),
        "mp4" => ("video", "video/mp4"), "webm" => ("video", "video/webm"), "mov" => ("video", "video/quicktime"),
        "pdf" => ("pdf", "application/pdf"),
        _ => ("text", "text/plain"),
    }
}

#[tauri::command]
fn read_file(path: String) -> Result<FilePreview, String> {
    let p = Path::new(&path);
    let md = std::fs::metadata(p).map_err(|e| e.to_string())?;
    let (kind, mime) = mime_of(p);
    const MAX_MEDIA: u64 = 64 * 1024 * 1024;
    const MAX_TEXT: u64 = 2 * 1024 * 1024;
    if kind == "text" {
        if md.len() > MAX_TEXT { return Ok(FilePreview { kind: "binary".into(), mime: mime.into(), size: md.len(), text: None, data_url: None }); }
        let bytes = std::fs::read(p).map_err(|e| e.to_string())?;
        if bytes.iter().take(8000).any(|b| *b == 0) {
            return Ok(FilePreview { kind: "binary".into(), mime: "application/octet-stream".into(), size: md.len(), text: None, data_url: None });
        }
        return Ok(FilePreview { kind: "text".into(), mime: mime.into(), size: md.len(), text: Some(String::from_utf8_lossy(&bytes).to_string()), data_url: None });
    }
    if md.len() > MAX_MEDIA { return Ok(FilePreview { kind: "binary".into(), mime: mime.into(), size: md.len(), text: None, data_url: None }); }
    let bytes = std::fs::read(p).map_err(|e| e.to_string())?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(FilePreview { kind: kind.into(), mime: mime.into(), size: md.len(), text: None, data_url: Some(format!("data:{mime};base64,{b64}")) })
}

#[tauri::command]
async fn git_log(workdir: String) -> Result<String, String> {
    let o = harness::sandbox::run_shell("git log --oneline --decorate -20 2>&1; echo; git status --short 2>&1 | head -40", Path::new(&workdir), Duration::from_secs(10), 8000).await.map_err(|e| e.to_string())?;
    Ok(o.stdout)
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(RunState::default())
        .invoke_handler(tauri::generate_handler![get_config, list_models, start_run, stop_run, list_dir, read_file, git_log, answer_permission])
        .run(tauri::generate_context!())
        .expect("error while running TheHarness UI");
}
