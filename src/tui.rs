//! Interactive terminal UI — the Claude-Code-style front end for a local model.
//! Everything here is presentation; the agent loop lives in the `harness` library.

use anyhow::Result;
use crossterm::event::{Event as CEvent, EventStream, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};
use futures_util::StreamExt;
use harness::agent::Agent;
use harness::config::Config;
use harness::events::{Event, Sink};
use harness::llm::{Client, Content, Message};
use harness::tools::{Registry, ToolCtx, Toolset};
use ratatui::prelude::*;
use ratatui::widgets::{Axis, Chart, Dataset, Gauge, GraphType, Paragraph, Sparkline};
use ratatui_image::{picker::Picker, protocol::StatefulProtocol, StatefulImage};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use unicode_width::UnicodeWidthChar;

// ───────────────────────── palette (dark / light) ─────────────────────────
#[derive(Clone, Copy)]
struct Pal { orange: Color, dim: Color, ok: Color, err: Color, think: Color, blue: Color, pink: Color, cyan: Color, fg: Color, panel_bg: Color }
static LIGHT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
fn pal() -> Pal {
    if LIGHT.load(std::sync::atomic::Ordering::Relaxed) {
        Pal { orange: Color::Rgb(200, 90, 0), dim: Color::Rgb(110, 116, 130), ok: Color::Rgb(20, 140, 80), err: Color::Rgb(200, 40, 40), think: Color::Rgb(110, 70, 220), blue: Color::Rgb(20, 100, 220), pink: Color::Rgb(200, 40, 90), cyan: Color::Rgb(0, 130, 150), fg: Color::Black, panel_bg: Color::Rgb(225, 228, 235) }
    } else {
        Pal { orange: Color::Rgb(255, 140, 40), dim: Color::Rgb(128, 136, 152), ok: Color::Rgb(76, 195, 138), err: Color::Rgb(255, 107, 107), think: Color::Rgb(167, 139, 250), blue: Color::Rgb(78, 161, 255), pink: Color::Rgb(255, 110, 130), cyan: Color::Rgb(90, 205, 220), fg: Color::White, panel_bg: Color::Rgb(38, 44, 56) }
    }
}
const SPINNER: [&str; 10] = ["✻", "✼", "✽", "✾", "✿", "❀", "✿", "✾", "✽", "✼"];
const WORDS: [&str; 12] = ["Thinking", "Pondering", "Working", "Reasoning", "Cooking", "Tinkering", "Brewing", "Mulling", "Crunching", "Percolating", "Noodling", "Computing"];

enum Msg { Ask(harness::permissions::ApprovalRequest, tokio::sync::oneshot::Sender<harness::permissions::Approval>), Ev(Event), Done(Result<(String, harness::agent::RunStats), String>), Sys(SysSample), CtxLen(u64), Pasted(Result<PathBuf, String>), Frames(Result<(PathBuf, f64, Vec<(f64, PathBuf)>), String>), Toolset(Arc<Toolset>), Catalog(Result<harness::plugins::Catalog, String>), Notice(String) }

/// Video scrubber state (modal over the transcript).
struct VideoPicker { path: PathBuf, duration: f64, frames: Vec<(f64, PathBuf, String)>, cur: usize, selected: std::collections::BTreeSet<usize>, loading: bool, error: Option<String> }

fn video_ext(p: &std::path::Path) -> bool {
    matches!(p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref(), Some("mp4" | "mov" | "m4v" | "webm" | "mkv" | "avi" | "gif" | "mpg" | "mpeg"))
}

/// Probe duration and extract up to `n` evenly spaced JPEG frames (max width 640) with ffmpeg.
async fn extract_frames(video: PathBuf, out_dir: PathBuf, n: usize) -> Result<(PathBuf, f64, Vec<(f64, PathBuf)>), String> {
    let probe = tokio::process::Command::new("ffprobe").args(["-v", "error", "-show_entries", "format=duration", "-of", "default=noprint_wrappers=1:nokey=1"]).arg(&video).output().await.map_err(|e| format!("ffprobe: {e} (install ffmpeg: brew install ffmpeg)"))?;
    let duration: f64 = String::from_utf8_lossy(&probe.stdout).trim().parse().unwrap_or(0.0);
    std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
    let n = n.max(2);
    let step = if duration > 0.0 { duration / n as f64 } else { 1.0 };
    // one ffmpeg call: fps filter to n frames across the duration
    let vf = if duration > 0.0 { format!("fps=1/{:.4},scale='min(640,iw)':-2", step) } else { "fps=1,scale='min(640,iw)':-2".to_string() };
    let pattern = out_dir.join("frame-%03d.jpg");
    let o = tokio::process::Command::new("ffmpeg").args(["-hide_banner", "-loglevel", "error", "-y", "-i"]).arg(&video).args(["-vf", &vf, "-frames:v", &(n + 1).to_string(), "-q:v", "4"]).arg(&pattern).output().await.map_err(|e| format!("ffmpeg: {e}"))?;
    if !o.status.success() { return Err(format!("ffmpeg failed: {}", String::from_utf8_lossy(&o.stderr).trim())); }
    let mut frames: Vec<(f64, PathBuf)> = Vec::new();
    let mut entries: Vec<PathBuf> = std::fs::read_dir(&out_dir).map_err(|e| e.to_string())?.flatten().map(|e| e.path()).filter(|p| p.extension().map(|e| e == "jpg").unwrap_or(false)).collect();
    entries.sort();
    for (i, p) in entries.into_iter().enumerate() { frames.push((i as f64 * step, p)); }
    if frames.is_empty() { return Err("no frames extracted".into()); }
    Ok((video, duration, frames))
}

/// An image attached to the next prompt.
struct Attachment { path: PathBuf, mime: String, b64: String, dims: (u32, u32), img: image::DynamicImage, label: Option<String> }

/// A rendered image slot in the transcript: which key, and its cell size.
struct ImgSlot { key: String, cols: u16, rows: u16 }
struct Placeholder { line: usize, slot: ImgSlot }

fn image_mime(p: &std::path::Path) -> Option<&'static str> {
    match p.extension()?.to_str()?.to_ascii_lowercase().as_str() { "png" => Some("image/png"), "jpg" | "jpeg" => Some("image/jpeg"), "gif" => Some("image/gif"), "webp" => Some("image/webp"), "bmp" => Some("image/bmp"), _ => None }
}

fn load_attachment(path: &std::path::Path) -> Result<Attachment, String> {
    let mime = image_mime(path).ok_or("not an image (png/jpg/gif/webp/bmp)")?;
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    if bytes.len() > 12 * 1024 * 1024 { return Err(format!("image too large ({} MB, max 12)", bytes.len() >> 20)); }
    let img = image::load_from_memory(&bytes).map_err(|e| format!("could not decode image: {e}"))?;
    let dims = (img.width(), img.height());
    use base64::Engine;
    Ok(Attachment { path: path.to_path_buf(), mime: mime.into(), b64: base64::engine::general_purpose::STANDARD.encode(&bytes), dims, img, label: None })
}

/// Terminals that implement an inline-image protocol (and answer capability queries).
fn graphics_terminal() -> bool {
    if let Ok(v) = std::env::var("HARNESS_GRAPHICS") { return v != "0" && v != "off"; }
    let term = std::env::var("TERM").unwrap_or_default();
    let prog = std::env::var("TERM_PROGRAM").unwrap_or_default();
    term.contains("kitty") || std::env::var_os("KITTY_WINDOW_ID").is_some()
        || prog == "WezTerm" || prog == "iTerm.app" || prog == "ghostty" || term.contains("ghostty")
        || std::env::var_os("WEZTERM_PANE").is_some() || std::env::var_os("ITERM_SESSION_ID").is_some()
}

/// macOS clipboard → a file we can attach. Images are saved as PNG under the pastes dir; copied
/// files (Finder ⌘C) resolve to their existing path. Returns Err if the clipboard has neither.
async fn clipboard_image(store: Option<harness::memory::MemoryStore>) -> Result<PathBuf, String> {
    // 1) image data
    let tmp = std::env::temp_dir().join(format!("harness-paste-{}.png", std::process::id()));
    let script = format!("set f to open for access POSIX file \"{}\" with write permission\nset eof of f to 0\nwrite (the clipboard as «class PNGf») to f\nclose access f", tmp.display());
    let o = tokio::process::Command::new("osascript").arg("-e").arg(&script).output().await.map_err(|e| e.to_string())?;
    if o.status.success() {
        let bytes = std::fs::read(&tmp).map_err(|e| e.to_string())?; let _ = std::fs::remove_file(&tmp);
        return match &store { Some(st) => st.save_paste("png", &bytes).map_err(|e| e.to_string()), None => { std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?; Ok(tmp) } };
    }
    let _ = std::fs::remove_file(&tmp);
    // 2) a file reference (copied in Finder)
    let o = tokio::process::Command::new("osascript").arg("-e").arg("POSIX path of (the clipboard as «class furl»)").output().await.map_err(|e| e.to_string())?;
    if o.status.success() {
        let p = PathBuf::from(String::from_utf8_lossy(&o.stdout).trim());
        if p.exists() { return Ok(p); }
    }
    Err("clipboard has no image or file (copy an image or a file, then ctrl+v; or type/drag a path)".into())
}

/// One system sample (1 Hz) from the background sampler.
#[derive(Clone, Debug, Default)]
struct SysSample { cpu: f32, mem_used: u64, mem_total: u64, gpu_util: Option<f32>, gpu_mem: Option<u64>, server_rss: u64, harness_rss: u64 }

const HIST: usize = 120;

/// Everything the dashboard shows.
struct Metrics {
    cpu: VecDeque<u64>, gpu: VecDeque<u64>, mem: VecDeque<u64>, last: SysSample,
    gen_speed: VecDeque<u64>,          // tok/s per model call
    ttft: VecDeque<u64>,               // ms per model call
    completion_per_call: VecDeque<u64>,
    prompt_per_call: VecDeque<u64>,
    live_chars: VecDeque<(Instant, usize)>, // streamed chars for live tok/s estimate
    last_call: Option<(u64, u64, f64, f64)>, // prompt, completion, ttft, secs
    calls: u64, ctx_len: u64,
    turn_start: Option<Instant>, live_peak: f64,
}
impl Metrics {
    fn new(ctx_len: u64) -> Self { Self { cpu: VecDeque::new(), gpu: VecDeque::new(), mem: VecDeque::new(), last: SysSample::default(), gen_speed: VecDeque::new(), ttft: VecDeque::new(), completion_per_call: VecDeque::new(), prompt_per_call: VecDeque::new(), live_chars: VecDeque::new(), last_call: None, calls: 0, ctx_len, turn_start: None, live_peak: 0.0 } }
    fn push(q: &mut VecDeque<u64>, v: u64) { q.push_back(v); while q.len() > HIST { q.pop_front(); } }
    fn on_sys(&mut self, s: SysSample) {
        Self::push(&mut self.cpu, s.cpu.round() as u64);
        if let Some(g) = s.gpu_util { Self::push(&mut self.gpu, g.round() as u64); }
        if s.mem_total > 0 { Self::push(&mut self.mem, (s.mem_used * 100 / s.mem_total) as u64); }
        self.last = s;
    }
    fn on_delta(&mut self, chars: usize) {
        let now = Instant::now();
        self.live_chars.push_back((now, chars));
        while let Some((t, _)) = self.live_chars.front() { if now.duration_since(*t) > Duration::from_secs(3) { self.live_chars.pop_front(); } else { break; } }
        let v = self.live_tps(); if v > self.live_peak { self.live_peak = v; }
    }
    /// ≈ tokens/s over the last 3 s of streaming (≈4 chars per token).
    fn live_tps(&self) -> f64 {
        let Some((t0, _)) = self.live_chars.front() else { return 0.0 };
        let span = Instant::now().duration_since(*t0).as_secs_f64().max(0.5);
        let chars: usize = self.live_chars.iter().map(|(_, c)| *c).sum();
        chars as f64 / 4.0 / span
    }
    fn on_call(&mut self, p: u64, c: u64, ttft: f64, secs: f64) {
        self.calls += 1;
        let gen = if secs > ttft && c > 0 { c as f64 / (secs - ttft) } else { 0.0 };
        Self::push(&mut self.gen_speed, gen.round() as u64);
        Self::push(&mut self.ttft, (ttft * 1000.0) as u64);
        Self::push(&mut self.completion_per_call, c);
        Self::push(&mut self.prompt_per_call, p);
        self.last_call = Some((p, c, ttft, secs));
    }
}

/// Background sampler: CPU/RAM (sysinfo), Apple GPU via IOAccelerator PerformanceStatistics (no root),
/// RSS of the model server (LM Studio / llama-server / ollama) and of this process.
async fn sampler(tx: mpsc::UnboundedSender<Msg>) {
    use sysinfo::{ProcessesToUpdate, System};
    let mut sys = System::new();
    let me = sysinfo::get_current_pid().ok();
    loop {
        sys.refresh_cpu_usage();
        sys.refresh_memory();
        sys.refresh_processes(ProcessesToUpdate::All, true);
        let mut server_rss = 0u64; let mut harness_rss = 0u64;
        for (pid, p) in sys.processes() {
            let exe = p.exe().map(|e| e.display().to_string()).unwrap_or_default();
            let name = p.name().to_string_lossy().to_lowercase();
            if exe.contains("LM Studio") || exe.contains(".lmstudio") || name.contains("llama-server") || name == "ollama" || name.contains("lmstudio") || name.contains("llmster") { server_rss += p.memory(); }
            if Some(*pid) == me { harness_rss = p.memory(); }
        }
        let (gpu_util, gpu_mem) = gpu_stats().await;
        let s = SysSample { cpu: sys.global_cpu_usage(), mem_used: sys.used_memory(), mem_total: sys.total_memory(), gpu_util, gpu_mem, server_rss, harness_rss };
        if tx.send(Msg::Sys(s)).is_err() { return; }
        tokio::time::sleep(Duration::from_millis(1000)).await;
    }
}

async fn gpu_stats() -> (Option<f32>, Option<u64>) {
    #[cfg(target_os = "macos")]
    {
        let out = tokio::process::Command::new("ioreg").args(["-r", "-d", "1", "-c", "IOAccelerator"]).output().await;
        if let Ok(o) = out {
            let s = String::from_utf8_lossy(&o.stdout);
            let util = grab_num(&s, "\"Device Utilization %\"=");
            let mem = grab_num(&s, "\"In use system memory\"=");
            return (util.map(|v| v as f32), mem);
        }
    }
    (None, None)
}
fn grab_num(s: &str, key: &str) -> Option<u64> {
    let i = s.find(key)? + key.len();
    let digits: String = s[i..].chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

/// Detect the model's context length at start (LM Studio / llama.cpp / Ollama).
async fn fetch_ctx_len(base_url: String, model: String, tx: mpsc::UnboundedSender<Msg>) {
    match harness::llm::detect_context_length(&base_url, &model).await {
        Some((n, src)) => { let _ = tx.send(Msg::CtxLen(n)); let _ = tx.send(Msg::Notice(format!("context window: {} tokens ({src})", fmt_k(n)))); }
        None => { let _ = tx.send(Msg::Notice("context window: unknown (server did not report it) — using 60k compaction threshold".into())); }
    }
}

struct TuiSink(mpsc::UnboundedSender<Msg>);
impl Sink for TuiSink { fn emit(&self, e: &Event) { let _ = self.0.send(Msg::Ev(e.clone())); } }

/// Routes permission prompts to the UI and waits for the answer.
struct TuiApprover(mpsc::UnboundedSender<Msg>);
#[async_trait::async_trait]
impl harness::permissions::Approver for TuiApprover {
    async fn ask(&self, req: harness::permissions::ApprovalRequest) -> harness::permissions::Approval {
        let (tx, rx) = tokio::sync::oneshot::channel();
        if self.0.send(Msg::Ask(req, tx)).is_err() { return harness::permissions::Approval::Deny; }
        rx.await.unwrap_or(harness::permissions::Approval::Deny)
    }
}

enum Block {
    Banner(Vec<String>),
    User(String, Vec<String>),
    Assistant { text: String, streaming: bool, folded: bool },
    Reasoning { text: String, streaming: bool, show: Option<bool> },
    Tool { id: String, name: String, args: String, result: Option<String>, secs: f64, images: usize, interrupted: bool, fold: Option<bool> },
    System(String),
    Error(String),
    Finished(String),
    Memory(String),
}

struct App {
    cfg: Config,
    workdir: PathBuf,
    model: String,
    net: bool,
    blocks: Vec<Block>,
    input: String,
    cursor: usize, // char index
    history: Vec<String>,
    hist_idx: Option<usize>,
    hist_draft: String,
    scroll_up: usize,
    running: Option<tokio::task::JoinHandle<()>>,
    run_started: Instant,
    queued: Vec<String>,
    expand_tools: bool,
    show_thinking: bool,
    session: Arc<tokio::sync::Mutex<Vec<Message>>>,
    tx: mpsc::UnboundedSender<Msg>,
    total_prompt: u64,
    total_completion: u64,
    last_prompt_tokens: u64,
    turn_tokens: u64,
    last_ctrl_c: Option<Instant>,
    status_msg: Option<(String, Instant)>,
    quit: bool,
    tick: u64,
    word: usize,
    models: Vec<String>,
    metrics: Metrics,
    panel: Option<bool>, // None = auto by width
    attachments: Vec<Attachment>,
    tool_previews: std::collections::HashMap<String, Vec<String>>, // tool call id → image keys
    picker: Picker,
    images: std::collections::HashMap<String, (StatefulProtocol, (u32, u32))>,
    img_seq: u64,
    think_scroll: usize,
    toolset: Option<Arc<Toolset>>,
    perm_mode: harness::permissions::Mode,
    session_meta: harness::sessions::Meta,
    todos: Arc<std::sync::Mutex<Vec<harness::tools::todo::TodoItem>>>,
    event_log: Option<std::fs::File>,
    pending_ask: Option<(harness::permissions::ApprovalRequest, tokio::sync::oneshot::Sender<harness::permissions::Approval>)>,
    video: Option<VideoPicker>,
    strip_rects: Vec<(Rect, usize)>,
    // geometry from the last draw, for mouse hit-testing
    tr_rect: Rect, panel_rect: Rect, tr_start: usize,
    line_map: Vec<(usize, usize, usize)>, // (first line, last line exclusive, block index)
}

pub async fn run(cfg: Config, resume: Option<String>) -> Result<()> {
    let workdir = std::env::current_dir()?;
    // Detect the terminal's graphics protocol (kitty / iterm2 / sixel) and cell size; fall back to half-blocks.
    // Only query terminals known to answer — a plain pty or Terminal.app would block forever on the query.
    let picker = if graphics_terminal() { Picker::from_query_stdio().unwrap_or_else(|_| Picker::from_fontsize((8, 16))) } else { Picker::from_fontsize((8, 16)) };
    let (tx, mut rx) = mpsc::unbounded_channel::<Msg>();
    let mut app = App {
        model: cfg.llm.model.clone(), net: cfg.net.enabled, cfg, workdir,
        blocks: vec![], input: String::new(), cursor: 0, history: vec![], hist_idx: None, hist_draft: String::new(),
        scroll_up: 0, running: None, run_started: Instant::now(), queued: vec![], expand_tools: false, show_thinking: false,
        session: Arc::new(tokio::sync::Mutex::new(Vec::new())), tx: tx.clone(),
        total_prompt: 0, total_completion: 0, last_prompt_tokens: 0, turn_tokens: 0, last_ctrl_c: None, status_msg: None,
        quit: false, tick: 0, word: 0, models: vec![],
        metrics: Metrics::new(0), panel: None, attachments: vec![], tool_previews: Default::default(),
        picker, images: Default::default(), img_seq: 0,
        think_scroll: 0, toolset: None, perm_mode: harness::permissions::Mode::Auto, session_meta: harness::sessions::Meta::default(), todos: Default::default(), event_log: None, pending_ask: None, video: None, strip_rects: vec![], tr_rect: Rect::default(), panel_rect: Rect::default(), tr_start: 0, line_map: vec![],
    };
    app.metrics.ctx_len = app.cfg.llm.context_budget_tokens.unwrap_or(0);
    app.perm_mode = app.cfg.permissions.mode;
    if app.cfg.ui.event_log { if let Ok(h) = std::env::var("HOME") { let d = std::path::PathBuf::from(h).join(".config/harness/logs").join(harness::memory::today_iso()); let _ = std::fs::create_dir_all(&d); app.event_log = std::fs::OpenOptions::new().create(true).append(true).open(d.join(format!("tui-{}.jsonl", std::process::id()))).ok(); } }
    if app.cfg.ui.theme == "light" { LIGHT.store(true, std::sync::atomic::Ordering::Relaxed); }
    app.banner();
    if let Some(r) = resume { app.resume_session(&r); }
    app.reload_toolset();
    tokio::spawn(sampler(tx.clone()));
    tokio::spawn(fetch_ctx_len(app.cfg.llm.base_url.clone(), app.cfg.llm.model.clone(), tx.clone()));
    // model list in the background
    {
        let tx = tx.clone(); let llm = app.cfg.llm.clone();
        tokio::spawn(async move {
            if let Ok(c) = Client::new(&llm) { if let Ok(m) = c.list_models().await { let _ = tx.send(Msg::Ev(Event::RunStarted { model: m.join("\u{1f}"), workdir: "\u{0}models".into(), tools: vec![] })); } }
        });
    }

    let mut terminal = ratatui::init();
    let _ = crossterm::execute!(std::io::stdout(), crossterm::event::EnableBracketedPaste, crossterm::event::EnableMouseCapture);
    let mut events = EventStream::new();
    let mut ticker = tokio::time::interval(Duration::from_millis(80));
    let res: Result<()> = async {
        loop {
            terminal.draw(|f| draw(f, &mut app))?;
            tokio::select! {
                _ = ticker.tick() => { app.tick += 1; if app.tick % 30 == 0 { app.word = (app.word + 1) % WORDS.len(); } }
                Some(msg) = rx.recv() => { app.on_msg(msg); while let Ok(m) = rx.try_recv() { app.on_msg(m); } }
                Some(ev) = events.next() => { match ev { Ok(ev) => app.on_term(ev), Err(e) => { app.blocks.push(Block::Error(format!("terminal: {e}"))); } } }
            }
            if app.quit { break; }
        }
        Ok(())
    }.await;
    let _ = crossterm::execute!(std::io::stdout(), crossterm::event::DisableMouseCapture, crossterm::event::DisableBracketedPaste);
    ratatui::restore();
    if let Some(h) = app.running.take() { h.abort(); }
    res
}

// ───────────────────────── behaviour ─────────────────────────
impl App {
    fn banner(&mut self) {
        let wd = short_path(&self.workdir);
        self.blocks.push(Block::Banner(vec![
            format!("✻ TheHarness — local coding agent"),
            format!("  model  {}", self.model),
            format!("  server {}", self.cfg.llm.base_url),
            format!("  cwd    {}", wd),
            {
                let st = harness::setup::check();
                let ok = st.iter().filter(|s| s.ok()).count();
                let miss: Vec<&str> = st.iter().filter(|s| !s.ok()).map(|s| s.name).collect();
                format!("  tools  {ok}/{} available{}", st.len(), if miss.is_empty() { String::new() } else { format!(" · missing {} → harness setup --install", miss.join(", ")) })
            },
            String::new(),
            "  /help for commands · esc interrupts · ctrl+o expands tool output · ctrl+t shows thinking · ctrl+p panel".into(),
        ]));
    }

    fn set_status(&mut self, s: impl Into<String>) { self.status_msg = Some((s.into(), Instant::now())); }

    /// (Re)build tools: built-ins + MCP servers from global/project/plugin configs. Async; swaps in when ready.
    fn reload_toolset(&mut self) {
        let tx = self.tx.clone(); let net = self.net; let wd = self.workdir.clone();
        tokio::spawn(async move {
            let ts = harness::tools::build_toolset(net, &wd, true).await;
            for n in &ts.notes { let _ = tx.send(Msg::Notice(n.clone())); }
            let _ = tx.send(Msg::Toolset(Arc::new(ts)));
        });
    }
    fn panel_visible(&self, width: u16) -> bool { self.panel.unwrap_or(width >= 120) }

    fn on_term(&mut self, ev: CEvent) {
        match ev {
            CEvent::Paste(s) => { self.insert_str(&s.replace("\r\n", "\n").replace('\r', "\n")); }
            CEvent::Mouse(m) if self.video.is_some() => {
                match m.kind {
                    MouseEventKind::ScrollUp => { if let Some(v) = &mut self.video { v.cur = v.cur.saturating_sub(1); } }
                    MouseEventKind::ScrollDown => { if let Some(v) = &mut self.video { let n = v.frames.len(); if n > 0 { v.cur = (v.cur + 1).min(n - 1); } } }
                    MouseEventKind::Down(MouseButton::Left) => {
                        if let Some(&(_, idx)) = self.strip_rects.iter().find(|(r, _)| m.column >= r.x && m.column < r.x + r.width && m.row >= r.y && m.row < r.y + r.height) {
                            if let Some(v) = &mut self.video { if v.cur == idx { if !v.selected.remove(&idx) { v.selected.insert(idx); } } else { v.cur = idx; } }
                        }
                    }
                    _ => {}
                }
            }
            CEvent::Mouse(m) => {
                let in_panel = self.panel_rect.width > 0 && m.column >= self.panel_rect.x && m.column < self.panel_rect.x + self.panel_rect.width;
                match m.kind {
                    MouseEventKind::ScrollUp => { if in_panel { self.think_scroll += 3; } else { self.scroll_up += 3; } }
                    MouseEventKind::ScrollDown => { if in_panel { self.think_scroll = self.think_scroll.saturating_sub(3); } else { self.scroll_up = self.scroll_up.saturating_sub(3); } }
                    MouseEventKind::Down(MouseButton::Left) if !in_panel => {
                        let r = self.tr_rect;
                        if m.row >= r.y && m.row < r.y + r.height && m.column >= r.x && m.column < r.x + r.width {
                            let line = self.tr_start + (m.row - r.y) as usize;
                            if let Some(&(_, _, idx)) = self.line_map.iter().find(|(a, b, _)| line >= *a && line < *b) { self.toggle_fold(idx); }
                        }
                    }
                    _ => {}
                }
            }
            CEvent::Key(k) if k.kind == KeyEventKind::Press && self.pending_ask.is_some() => {
                let ans = match k.code {
                    KeyCode::Char('y') | KeyCode::Enter => Some(harness::permissions::Approval::Once),
                    KeyCode::Char('a') => Some(harness::permissions::Approval::Always),
                    KeyCode::Char('n') | KeyCode::Esc => Some(harness::permissions::Approval::Deny),
                    KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => Some(harness::permissions::Approval::Deny),
                    _ => None,
                };
                if let Some(a) = ans { if let Some((req, tx)) = self.pending_ask.take() { let label = match &a { harness::permissions::Approval::Once => "allowed once".to_string(), harness::permissions::Approval::Always => format!("always allow {}", req.suggested_rule), harness::permissions::Approval::Deny => "denied".into() }; self.blocks.push(Block::System(format!("🔒 {label}"))); let _ = tx.send(a); } }
            }
            CEvent::Key(k) if k.kind == KeyEventKind::Press && self.video.is_some() => {
                let n = self.video.as_ref().map(|v| v.frames.len()).unwrap_or(0);
                match k.code {
                    KeyCode::Esc => { self.video = None; self.set_status("video cancelled"); }
                    KeyCode::Enter => self.video_confirm(),
                    KeyCode::Left | KeyCode::Char('h') => { if let Some(v) = &mut self.video { v.cur = v.cur.saturating_sub(1); } }
                    KeyCode::Right | KeyCode::Char('l') => { if let Some(v) = &mut self.video { if n > 0 { v.cur = (v.cur + 1).min(n - 1); } } }
                    KeyCode::Home => { if let Some(v) = &mut self.video { v.cur = 0; } }
                    KeyCode::End => { if let Some(v) = &mut self.video { v.cur = n.saturating_sub(1); } }
                    KeyCode::Char(' ') => { if let Some(v) = &mut self.video { if !v.selected.remove(&v.cur) { v.selected.insert(v.cur); } } }
                    KeyCode::Char('a') => { if let Some(v) = &mut self.video { if v.selected.len() == n { v.selected.clear(); } else { v.selected = (0..n).collect(); } } }
                    KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => { self.video = None; }
                    _ => {}
                }
            }
            CEvent::Key(k) if k.kind == KeyEventKind::Press => {
                let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
                let alt = k.modifiers.contains(KeyModifiers::ALT);
                match (k.code, ctrl, alt) {
                    (KeyCode::Char('c'), true, _) => {
                        if self.running.is_some() { self.interrupt(); }
                        else if !self.input.is_empty() { self.input.clear(); self.cursor = 0; }
                        else if self.last_ctrl_c.map(|t| t.elapsed() < Duration::from_millis(1500)).unwrap_or(false) { self.quit = true; }
                        else { self.last_ctrl_c = Some(Instant::now()); self.set_status("Press ctrl+c again to exit"); }
                    }
                    (KeyCode::Char('d'), true, _) if self.input.is_empty() => self.quit = true,
                    (KeyCode::Esc, _, _) => { if self.running.is_some() { self.interrupt(); } else if !self.input.is_empty() { self.input.clear(); self.cursor = 0; } }
                    (KeyCode::Enter, _, true) | (KeyCode::Char('j'), true, _) => self.insert_str("\n"),
                    (KeyCode::Enter, _, _) => self.submit(),
                    (KeyCode::Char('o'), true, _) => { self.expand_tools = !self.expand_tools; }
                    (KeyCode::Char('t'), true, _) => { self.show_thinking = !self.show_thinking; }
                    (KeyCode::Char('l'), true, _) => { self.scroll_up = 0; }
                    (KeyCode::Char('p'), true, _) => { self.panel = Some(!self.panel_visible(200)); }
                    (KeyCode::Char('n'), true, _) => self.next_task(),
                    (KeyCode::Char('v'), true, _) => { let tx = self.tx.clone(); let store = if self.cfg.memory.enabled { harness::memory::MemoryStore::open(&self.cfg.memory).ok() } else { None }; self.set_status("reading clipboard…"); tokio::spawn(async move { let _ = tx.send(Msg::Pasted(clipboard_image(store).await)); }); }
                    (KeyCode::Char('u'), true, _) => { let c = self.cursor; self.input = self.input.chars().skip(c).collect(); self.cursor = 0; }
                    (KeyCode::Char('a'), true, _) | (KeyCode::Home, _, _) => self.cursor = self.line_start(),
                    (KeyCode::Char('e'), true, _) | (KeyCode::End, _, _) => self.cursor = self.line_end(),
                    (KeyCode::Backspace, _, _) => { if self.cursor > 0 { let mut cs: Vec<char> = self.input.chars().collect(); cs.remove(self.cursor - 1); self.input = cs.into_iter().collect(); self.cursor -= 1; } }
                    (KeyCode::Delete, _, _) => { let mut cs: Vec<char> = self.input.chars().collect(); if self.cursor < cs.len() { cs.remove(self.cursor); self.input = cs.into_iter().collect(); } }
                    (KeyCode::Left, _, _) => { self.cursor = self.cursor.saturating_sub(1); }
                    (KeyCode::Right, _, _) => { self.cursor = (self.cursor + 1).min(self.input.chars().count()); }
                    (KeyCode::Up, true, _) | (KeyCode::PageUp, _, _) => { self.scroll_up += 10; }
                    (KeyCode::Down, true, _) | (KeyCode::PageDown, _, _) => { self.scroll_up = self.scroll_up.saturating_sub(10); }
                    (KeyCode::Up, _, _) => { if !self.input.contains('\n') { self.history_prev(); } }
                    (KeyCode::Down, _, _) => { if !self.input.contains('\n') { self.history_next(); } }
                    (KeyCode::BackTab, _, _) => { use harness::permissions::Mode::*; self.perm_mode = match self.perm_mode { Auto => Ask, Ask => Plan, Plan => Bypass, Bypass => Auto }; self.set_status(format!("permissions → {}", self.perm_mode.label())); }
                    (KeyCode::Tab, _, _) => { self.complete_slash(); }
                    (KeyCode::Char(c), false, false) => { self.insert_str(&c.to_string()); }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn insert_str(&mut self, s: &str) {
        let mut cs: Vec<char> = self.input.chars().collect();
        for (i, c) in s.chars().enumerate() { cs.insert(self.cursor + i, c); }
        self.cursor += s.chars().count();
        self.input = cs.into_iter().collect();
        self.hist_idx = None;
    }
    fn line_start(&self) -> usize { let cs: Vec<char> = self.input.chars().collect(); let mut i = self.cursor; while i > 0 && cs[i - 1] != '\n' { i -= 1; } i }
    fn line_end(&self) -> usize { let cs: Vec<char> = self.input.chars().collect(); let mut i = self.cursor; while i < cs.len() && cs[i] != '\n' { i += 1; } i }
    fn history_prev(&mut self) {
        if self.history.is_empty() { return; }
        let idx = match self.hist_idx { None => { self.hist_draft = self.input.clone(); self.history.len() - 1 } Some(0) => 0, Some(i) => i - 1 };
        self.hist_idx = Some(idx); self.input = self.history[idx].clone(); self.cursor = self.input.chars().count();
    }
    fn history_next(&mut self) {
        let Some(i) = self.hist_idx else { return };
        if i + 1 >= self.history.len() { self.hist_idx = None; self.input = self.hist_draft.clone(); }
        else { self.hist_idx = Some(i + 1); self.input = self.history[i + 1].clone(); }
        self.cursor = self.input.chars().count();
    }
    fn complete_slash(&mut self) {
        if !self.input.starts_with('/') || self.input.contains(' ') { return; }
        let m: Vec<&str> = COMMANDS.iter().map(|c| c.0).filter(|c| c.starts_with(&self.input)).collect();
        if m.len() == 1 { self.input = format!("{} ", m[0]); self.cursor = self.input.chars().count(); }
    }

    fn register_image(&mut self, img: image::DynamicImage) -> String {
        self.img_seq += 1;
        let key = format!("img{}", self.img_seq);
        let dims = (img.width(), img.height());
        let proto = self.picker.new_resize_protocol(img);
        self.images.insert(key.clone(), (proto, dims));
        key
    }

    /// Open the frame scrubber for a video file.
    fn open_video(&mut self, p: &std::path::Path) {
        let path = p.to_path_buf();
        let store = if self.cfg.memory.enabled { harness::memory::MemoryStore::open(&self.cfg.memory).ok() } else { None };
        let base = store.as_ref().map(|s| s.pastes_dir()).unwrap_or(std::env::temp_dir());
        let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or("video".into());
        let out_dir = base.join(format!("{stem}-frames"));
        self.video = Some(VideoPicker { path: path.clone(), duration: 0.0, frames: vec![], cur: 0, selected: Default::default(), loading: true, error: None });
        let tx = self.tx.clone();
        tokio::spawn(async move { let _ = tx.send(Msg::Frames(extract_frames(path, out_dir, 32).await)); });
    }
    /// Attach the selected frames (or the current one) as images with timestamps.
    fn video_confirm(&mut self) {
        let Some(v) = self.video.take() else { return };
        let mut idxs: Vec<usize> = v.selected.iter().cloned().collect();
        if idxs.is_empty() && !v.frames.is_empty() { idxs.push(v.cur); }
        let mut n = self.attachments.len();
        for i in idxs {
            let (ts, p, _) = &v.frames[i];
            if let Ok(mut a) = load_attachment(p) {
                a.label = Some(format!("frame at {:.1}s of {} (duration {:.1}s)", ts, v.path.display(), v.duration));
                n += 1; self.attachments.push(a);
                self.insert_str(&format!("[image #{n} @{:.1}s] ", ts));
            }
        }
        self.set_status(format!("attached {} frame(s) from {}", n, short_path(&v.path)));
    }

    fn attach(&mut self, p: &std::path::Path) {
        match load_attachment(p) {
            Ok(a) => { let n = self.attachments.len() + 1; self.set_status(format!("attached image #{n}: {} ({}×{})", short_path(&a.path), a.dims.0, a.dims.1)); self.attachments.push(a); if !self.input.is_empty() && !self.input.ends_with(' ') { self.insert_str(" "); } self.insert_str(&format!("[image #{n}] ")); }
            Err(e) => self.set_status(format!("cannot attach {}: {e}", p.display())),
        }
    }

    /// Words in the prompt that are paths to existing image files become attachments (drag & drop).
    fn harvest_image_paths(&mut self, text: &str) {
        let toks: Vec<String> = text.split_whitespace().map(|t| t.trim_matches(|c| c == '"' || c == '\'' || c == ',').replace("\\ ", " ")).collect();
        for t in toks {
            let p = if t.starts_with('~') { PathBuf::from(t.replacen('~', &std::env::var("HOME").unwrap_or_default(), 1)) } else if PathBuf::from(&t).is_absolute() { PathBuf::from(&t) } else { self.workdir.join(&t) };
            if image_mime(&p).is_some() && p.is_file() && !self.attachments.iter().any(|a| a.path == p) {
                if let Ok(a) = load_attachment(&p) { self.attachments.push(a); }
            }
        }
    }

    fn submit(&mut self) {
        let text = self.input.trim().to_string();
        if text.is_empty() && self.attachments.is_empty() { return; }
        // a bare video path (or one among the words) opens the scrubber first
        for t in text.split_whitespace() {
            let t = t.trim_matches(|c| c == '"' || c == '\'');
            let p = if t.starts_with('~') { PathBuf::from(t.replacen('~', &std::env::var("HOME").unwrap_or_default(), 1)) } else if PathBuf::from(t).is_absolute() { PathBuf::from(t) } else { self.workdir.join(t) };
            if video_ext(&p) && p.is_file() && !self.input.contains("@") { self.open_video(&p); return; }
        }
        let text = if text.is_empty() { "Look at the attached image(s).".to_string() } else { text };
        self.input.clear(); self.cursor = 0; self.hist_idx = None;
        if self.history.last() != Some(&text) { self.history.push(text.clone()); }
        if text.starts_with('/') { self.command(&text); return; }
        if self.running.is_some() { self.queued.push(text); self.set_status(format!("queued ({} waiting) — will run after the current turn", self.queued.len())); return; }
        self.start_run(text);
    }

    fn command(&mut self, line: &str) {
        let mut parts = line.splitn(2, ' ');
        let cmd = parts.next().unwrap_or("");
        let arg = parts.next().unwrap_or("").trim().to_string();
        match cmd {
            "/help" | "/?" => {
                let mut lines = vec!["Commands".to_string()];
                for (c, d) in COMMANDS { lines.push(format!("  {c:<14} {d}")); }
                lines.push(String::new());
                lines.push("Keys: enter send · alt+enter/ctrl+j newline · esc interrupt · ctrl+c clear/exit · ctrl+o expand tools · ctrl+t thinking · ctrl+p panel · ctrl+v paste image · ctrl+n next task · pgup/pgdn scroll · ↑/↓ history".into());
                lines.push("Mouse: wheel/trackpad scrolls the transcript and the thinking panel; click a tool call, answer or thought to fold/unfold it. Hold shift (or fn/option in Terminal) to select text.".into());
                lines.push("Video: paste/drag a video (mp4/mov/webm/mkv/gif) or /video <path> → frame scrubber; select frames, enter attaches them as images with timestamps.".into());
                lines.push("Images: ctrl+v pastes from the clipboard; typing or dragging an image path attaches it. Previews render as a color mosaic; the model sees the full image.".into());
                self.blocks.push(Block::Banner(lines));
            }
            "/sessions" => {
                match harness::sessions::SessionStore::open() {
                    Ok(store) => { let list = store.list(None); let mut lines = vec![format!("Sessions ({}) — /resume <n|id|last>   · current: {}", list.len(), if self.session_meta.id.is_empty() { "(unsaved)" } else { &self.session_meta.id })]; for (i, m) in list.iter().take(25).enumerate() { lines.push(format!("  {:>2}. {}  {:<50} {:<28} {} turns · {}", i + 1, m.id, truncate(&m.title, 50), short_path(std::path::Path::new(&m.workdir)), m.turns, harness::sessions::fmt_age(m.updated))); } self.blocks.push(Block::Banner(lines)); }
                    Err(e) => self.blocks.push(Block::Error(e.to_string())),
                }
            }
            "/resume" => { if self.running.is_some() { self.set_status("finish or interrupt the current task first"); } else { self.resume_session(&arg); } }
            "/clear" | "/new" => {
                self.session_meta = harness::sessions::Meta::default();
                if let Ok(mut t) = self.todos.lock() { t.clear(); }
                let s = self.session.clone(); tokio::spawn(async move { s.lock().await.clear(); });
                self.blocks.clear(); self.total_prompt = 0; self.total_completion = 0; self.last_prompt_tokens = 0; self.banner();
                self.blocks.push(Block::System("new session".into()));
            }
            "/model" => {
                if arg.is_empty() {
                    let mut lines = vec![format!("current: {}", self.model), "available:".into()];
                    for m in &self.models { lines.push(format!("  {}{}", if *m == self.model { "● " } else { "  " }, m)); }
                    lines.push("usage: /model <name>".into());
                    self.blocks.push(Block::Banner(lines));
                } else { self.model = arg.clone(); self.cfg.llm.model = arg.clone(); self.blocks.push(Block::System(format!("model → {arg}"))); tokio::spawn(fetch_ctx_len(self.cfg.llm.base_url.clone(), arg.clone(), self.tx.clone())); }
            }
            "/cd" => {
                let p = if arg.is_empty() { std::env::var("HOME").unwrap_or_default() } else { arg.clone() };
                let p = if p.starts_with('~') { p.replacen('~', &std::env::var("HOME").unwrap_or_default(), 1) } else { p };
                let p = if PathBuf::from(&p).is_absolute() { PathBuf::from(&p) } else { self.workdir.join(&p) };
                match p.canonicalize() { Ok(p) if p.is_dir() => { self.workdir = p.clone(); let _ = std::env::set_current_dir(&p); self.blocks.push(Block::System(format!("cwd → {}", short_path(&p)))); }
                    _ => self.blocks.push(Block::Error(format!("no such directory: {}", p.display()))) }
            }
            "/pwd" => self.blocks.push(Block::System(self.workdir.display().to_string())),
            "/net" => { match arg.as_str() { "on" => self.net = true, "off" => self.net = false, _ => {} } self.blocks.push(Block::System(format!("internet tools: {}", if self.net { "on" } else { "off" }))); }
            "/tools" => {
                let defs = match &self.toolset { Some(ts) => ts.registry.defs(), None => Registry::defaults(self.net).defs() };
                self.blocks.push(Block::Banner(std::iter::once(format!("Tools ({})", defs.len())).chain(defs.into_iter().map(|d| format!("  {:<28} {}", d.function.name, truncate(&d.function.description, 80)))).collect()));
            }
            "/mcp" => {
                let wd = self.workdir.clone();
                let plugins = harness::plugins::Plugins::open().ok();
                let extra = plugins.as_ref().map(|p| p.mcp_files()).unwrap_or_default();
                let servers = harness::mcp::discover(&wd, &extra);
                let mut lines = vec![format!("MCP servers configured: {}  (edit ~/.config/harness/mcp.json or <project>/.mcp.json, then /reload)", servers.len())];
                for (n, c, f) in servers { lines.push(format!("  {:<18} {} {}   ← {}", n, c.command, c.args.join(" "), short_path(&f))); }
                let live: Vec<String> = self.toolset.as_ref().map(|ts| ts.registry.names().into_iter().filter(|n| n.starts_with("mcp__")).map(String::from).collect()).unwrap_or_default();
                lines.push(format!("live MCP tools: {}", live.len()));
                for t in live.iter().take(40) { lines.push(format!("  {t}")); }
                self.blocks.push(Block::Banner(lines));
            }
            "/reload" => { self.reload_toolset(); self.blocks.push(Block::System("reloading tools, MCP servers and plugins…".into())); }
            "/compact" => {
                if self.running.is_some() { self.set_status("wait for the current turn to finish"); }
                else {
                    let tx = self.tx.clone(); let session = self.session.clone(); let cfg = self.cfg.clone(); let focus = if arg.is_empty() { None } else { Some(arg.clone()) };
                    self.blocks.push(Block::System("compacting context…".into()));
                    tokio::spawn(async move {
                        let sink = TuiSink(tx.clone());
                        let res: Result<(), String> = async {
                            let client = Client::new(&cfg.llm).map_err(|e| e.to_string())?;
                            let mut msgs = session.lock().await;
                            let (n, summary) = harness::agent::compact_llm(&client.aux(), &mut msgs, 4, focus.as_deref()).await.map_err(|e| format!("{e:#}"))?;
                            sink.emit(&Event::Compacted { count: n, prompt_tokens: 0, summary });
                            Ok(())
                        }.await;
                        if let Err(e) = res { sink.emit(&Event::Error { message: format!("compact: {e}") }); }
                    });
                }
            }
            "/plugin" | "/plugins" => self.plugin_command(&arg),
            "/thinking" => { self.show_thinking = !self.show_thinking; self.blocks.push(Block::System(format!("thinking {}", if self.show_thinking { "shown" } else { "hidden" }))); }
            "/expand" => { self.expand_tools = !self.expand_tools; }
            "/panel" => { self.panel = Some(!self.panel_visible(200)); }
            "/cost" | "/stats" => self.blocks.push(Block::System(format!("session tokens: {} prompt + {} completion · last context {} · turns in history {}", self.total_prompt, self.total_completion, self.last_prompt_tokens, self.history.len()))),
            "/config" => self.blocks.push(Block::Banner(vec![format!("server  {}", self.cfg.llm.base_url), format!("model   {}", self.model), format!("context {} · compaction at {} tokens · max_turns {} · tool timeout {}s", fmt_k(self.metrics.ctx_len), fmt_k(self.cfg.llm.effective_budget(if self.metrics.ctx_len > 0 { Some(self.metrics.ctx_len) } else { None })), self.cfg.agent.max_turns, self.cfg.agent.tool_timeout_secs), format!("net {} · segments {}", self.net, self.cfg.net.download_segments)])),
            "/memory" | "/brain" | "/workflows" => {
                let file = match cmd { "/memory" => "MEMORY", "/brain" => "BRAIN", _ => "WORKFLOWS" };
                match harness::memory::MemoryStore::open(&self.cfg.memory).and_then(|s| Ok((s.path(file)?, s.read(file)?))) {
                    Ok((p, doc)) => { let mut lines = vec![format!("{} — edit with any editor", p.display()), String::new()]; lines.extend(doc.lines().map(String::from)); self.blocks.push(Block::Banner(lines)); }
                    Err(e) => self.blocks.push(Block::Error(format!("memory: {e:#}"))),
                }
            }
            "/remember" => {
                if arg.is_empty() { self.blocks.push(Block::Error("usage: /remember <text>   (adds to MEMORY.md › Preferences; use `/remember brain: <text>` or `workflows:` to target another file)".into())); }
                else {
                    let (file, section, text) = if let Some(t) = arg.strip_prefix("brain:") { ("BRAIN", "Lessons", t.trim()) } else if let Some(t) = arg.strip_prefix("workflows:") { ("WORKFLOWS", "Notes", t.trim()) } else { ("MEMORY", "Preferences", arg.as_str()) };
                    match harness::memory::MemoryStore::open(&self.cfg.memory).and_then(|s| s.append(file, section, text)) {
                        Ok(true) => self.blocks.push(Block::Memory(format!("{file} › {section}: {text}"))),
                        Ok(false) => self.blocks.push(Block::System("already in memory".into())),
                        Err(e) => self.blocks.push(Block::Error(format!("memory: {e:#}"))),
                    }
                }
            }
            "/reflect" => {
                if self.running.is_some() { self.set_status("wait for the current turn to finish"); }
                else {
                    let tx = self.tx.clone(); let session = self.session.clone(); let cfg = self.cfg.clone();
                    self.blocks.push(Block::System("reflecting on this session…".into()));
                    tokio::spawn(async move {
                        let sink = TuiSink(tx.clone());
                        let res: Result<(), String> = async {
                            let store = harness::memory::MemoryStore::open(&cfg.memory).map_err(|e| e.to_string())?;
                            let client = Client::new(&cfg.llm).map_err(|e| e.to_string())?;
                            let msgs = session.lock().await.clone();
                            let items = store.reflect(&client, &msgs).await.map_err(|e| e.to_string())?;
                            if items.is_empty() { sink.emit(&Event::Error { message: "reflection: nothing new worth remembering".into() }); }
                            for (f, s, t) in items { sink.emit(&Event::Memory { file: f, section: s, text: t }); }
                            Ok(())
                        }.await;
                        if let Err(e) = res { sink.emit(&Event::Error { message: format!("reflection failed: {e}") }); }
                    });
                }
            }
            "/video" => { if arg.is_empty() { self.blocks.push(Block::Error("usage: /video <path>".into())); } else { let p = if arg.starts_with('~') { PathBuf::from(arg.replacen('~', &std::env::var("HOME").unwrap_or_default(), 1)) } else { PathBuf::from(&arg) }; if p.is_file() { self.open_video(&p); } else { self.blocks.push(Block::Error(format!("no such file: {arg}"))); } } }
            "/permissions" | "/perm" | "/mode" => {
                if arg.is_empty() {
                    let rules = harness::permissions::persisted_rules();
                    let mut lines = vec![format!("permission mode: {} ({})", format!("{:?}", self.perm_mode).to_lowercase(), self.perm_mode.label()), "switch: /permissions bypass|auto|ask|plan   (shift+tab cycles)".into(), format!("config allow: {:?}", self.cfg.permissions.allow), format!("config deny:  {:?}", self.cfg.permissions.deny), format!("always-allowed (this machine): {:?}", rules)];
                    lines.push("Rules are '<tool>' or '<tool>:<glob>' matched on the primary argument (bash cmd, file path, url).".into());
                    self.blocks.push(Block::Banner(lines));
                } else if let Some(m) = harness::permissions::Mode::parse(&arg) { self.perm_mode = m; self.blocks.push(Block::System(format!("permissions → {}", m.label()))); }
                else { self.blocks.push(Block::Error("usage: /permissions [bypass|auto|ask|plan]".into())); }
            }
            "/theme" => { let light = match arg.as_str() { "light" => true, "dark" => false, _ => !LIGHT.load(std::sync::atomic::Ordering::Relaxed) }; LIGHT.store(light, std::sync::atomic::Ordering::Relaxed); self.blocks.push(Block::System(format!("theme → {}", if light { "light" } else { "dark" }))); }
            "/plan" => { self.perm_mode = if self.perm_mode == harness::permissions::Mode::Plan { harness::permissions::Mode::Auto } else { harness::permissions::Mode::Plan }; self.blocks.push(Block::System(format!("permissions → {}", self.perm_mode.label()))); }
            "/queue" => {
                if self.queued.is_empty() { self.blocks.push(Block::System("queue is empty".into())); }
                else { let mut lines = vec![format!("Queued tasks ({}) — /next skips the current one, /queue clear empties the queue", self.queued.len())]; for (i, q) in self.queued.iter().enumerate() { lines.push(format!("  {}. {}", i + 1, truncate(q, 120))); } self.blocks.push(Block::Banner(lines)); }
                if arg == "clear" { self.queued.clear(); self.blocks.push(Block::System("queue cleared".into())); }
            }
            "/next" | "/skip" => self.next_task(),
            "/exit" | "/quit" | "/q" => self.quit = true,
            _ => {
                let name = cmd.trim_start_matches('/');
                let found = harness::plugins::Plugins::open().ok().and_then(|p| p.commands().into_iter().find(|c| c.name == name));
                match found {
                    Some(c) => { let prompt = c.template.replace("$ARGUMENTS", &arg); self.blocks.push(Block::System(format!("/{} (plugin {})", c.name, c.plugin))); if self.running.is_some() { self.queued.push(prompt); } else { self.start_run(prompt); } }
                    None => self.blocks.push(Block::Error(format!("unknown command {cmd} — /help"))),
                }
            }
        }
    }

    fn plugin_command(&mut self, arg: &str) {
        let mut parts = arg.splitn(2, ' ');
        let sub = parts.next().unwrap_or("").trim().to_string();
        let rest = parts.next().unwrap_or("").trim().to_string();
        let tx = self.tx.clone();
        match sub.as_str() {
            "" | "list" | "ls" | "refresh" => {
                let refresh = sub == "refresh";
                self.blocks.push(Block::System(if refresh { "refreshing plugin catalog from GitHub…".into() } else { "loading plugin catalog…".into() }));
                tokio::spawn(async move {
                    let res = async { harness::plugins::Plugins::open()?.catalog(refresh).await }.await.map_err(|e| format!("{e:#}"));
                    let _ = tx.send(Msg::Catalog(res));
                });
            }
            "install" | "add" => {
                if rest.is_empty() { self.blocks.push(Block::Error("usage: /plugin install <owner/repo | git url | local dir>".into())); return; }
                self.blocks.push(Block::System(format!("installing {rest}…")));
                tokio::spawn(async move {
                    let res = async { let p = harness::plugins::Plugins::open()?; let name = p.install(&rest).await?; let info = p.inspect(&p.dir.join(&name)); Ok::<_, anyhow::Error>((name, info)) }.await;
                    match res {
                        Ok((name, info)) => {
                            let mut msg = format!("installed {name}: {} skill(s), {} command(s), {} MCP server(s){}", info.skills.len(), info.commands.len(), info.mcp_servers.len(), if info.ts_only { " — NOTE: TypeScript-only DSH plugin; its code needs the DSH runtime and cannot run here" } else { "" });
                            if !info.skills.is_empty() { msg.push_str(&format!(" · skills: {}", info.skills.iter().map(|s| s.name.clone()).collect::<Vec<_>>().join(", "))); }
                            if !info.commands.is_empty() { msg.push_str(&format!(" · commands: {}", info.commands.iter().map(|c| format!("/{}", c.name)).collect::<Vec<_>>().join(", "))); }
                            let _ = tx.send(Msg::Notice(msg));
                        }
                        Err(e) => { let _ = tx.send(Msg::Notice(format!("install failed: {e:#}"))); }
                    }
                });
                // reload after a short delay so MCP servers from the plugin start
                let tx2 = self.tx.clone(); let net = self.net; let wd = self.workdir.clone();
                tokio::spawn(async move { tokio::time::sleep(Duration::from_secs(8)).await; let ts = harness::tools::build_toolset(net, &wd, true).await; let _ = tx2.send(Msg::Toolset(Arc::new(ts))); });
            }
            "enable" | "disable" | "remove" | "rm" | "update" => {
                if rest.is_empty() { self.blocks.push(Block::Error(format!("usage: /plugin {sub} <name>"))); return; }
                let res: Result<String, String> = (|| {
                    let mut p = harness::plugins::Plugins::open().map_err(|e| e.to_string())?;
                    match sub.as_str() {
                        "enable" => { p.set_enabled(&rest, true).map_err(|e| e.to_string())?; Ok(format!("enabled {rest}")) }
                        "disable" => { p.set_enabled(&rest, false).map_err(|e| e.to_string())?; Ok(format!("disabled {rest}")) }
                        "remove" | "rm" => { p.remove(&rest).map_err(|e| e.to_string())?; Ok(format!("removed {rest}")) }
                        _ => { let tx = tx.clone(); let name = rest.clone(); tokio::spawn(async move { let r = async { harness::plugins::Plugins::open()?.update(&name).await }.await; let _ = tx.send(Msg::Notice(match r { Ok(m) => m, Err(e) => format!("update failed: {e:#}") })); }); Ok(format!("updating {rest}…")) }
                    }
                })();
                match res { Ok(m) => { self.blocks.push(Block::System(m)); self.reload_toolset(); } Err(e) => self.blocks.push(Block::Error(e)) }
            }
            "info" | "show" => {
                match harness::plugins::Plugins::open() {
                    Ok(p) => match p.installed().into_iter().find(|x| x.name == rest || x.path.file_name().map(|n| n.to_string_lossy() == rest).unwrap_or(false)) {
                        Some(pl) => {
                            let mut lines = vec![format!("{} {} — {}", pl.name, pl.version, pl.description), format!("path: {}  origin: {}  enabled: {}", pl.path.display(), pl.origin.clone().unwrap_or_default(), pl.enabled)];
                            for s in &pl.skills { lines.push(format!("  skill   {} — {}", s.name, truncate(&s.description, 90))); }
                            for c in &pl.commands { lines.push(format!("  command /{} — {}", c.name, truncate(&c.description, 90))); }
                            for m in &pl.mcp_servers { lines.push(format!("  mcp     {m}")); }
                            if pl.ts_only { lines.push("  (TypeScript-only DSH plugin: code not runnable here)".into()); }
                            self.blocks.push(Block::Banner(lines));
                        }
                        None => self.blocks.push(Block::Error(format!("no installed plugin '{rest}'"))),
                    },
                    Err(e) => self.blocks.push(Block::Error(e.to_string())),
                }
            }
            _ => self.blocks.push(Block::Error("usage: /plugin [list|refresh] · install <owner/repo|url|dir> · enable|disable|remove|update|info <name>".into())),
        }
    }

    fn show_catalog(&mut self, c: &harness::plugins::Catalog) {
        let plugins = harness::plugins::Plugins::open().ok();
        let installed = plugins.as_ref().map(|p| p.installed()).unwrap_or_default();
        let mut lines = vec![format!("Plugins — ● enabled  ◐ installed (disabled)  ○ downloadable   ({} in catalog from topics {}; /plugin refresh to refetch)", c.entries.len(), harness::plugins::TOPICS.join(", "))];
        lines.push(String::new());
        if !installed.is_empty() {
            lines.push("Installed:".into());
            for p in &installed {
                let dirname = p.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                let mark = if p.enabled { "●" } else { "◐" };
                let what = format!("{}sk {}cmd {}mcp{}", p.skills.len(), p.commands.len(), p.mcp_servers.len(), if p.ts_only { " ts-only" } else { "" });
                lines.push(format!("  {mark} {:<28} {:<22} {}", dirname, what, truncate(&p.description, 70)));
            }
            lines.push(String::new());
        }
        lines.push("Downloadable (/plugin install <owner/repo>):".into());
        for e in c.entries.iter().take(60) {
            let repo = e.full_name.rsplit('/').next().unwrap_or("");
            let inst = installed.iter().find(|p| p.origin.as_deref().map(|o| o.contains(&e.full_name)).unwrap_or(false) || p.path.file_name().map(|n| n.to_string_lossy() == repo).unwrap_or(false));
            let mark = match inst { Some(p) if p.enabled => "●", Some(_) => "◐", None => "○" };
            lines.push(format!("  {mark} {:<40} ★{:<6} {:<10} {}", e.full_name, e.stars, e.language, truncate(&e.description, 60)));
        }
        lines.push(String::new());
        lines.push("Note: DSH plugins are TypeScript modules; this harness loads their skills, commands and MCP servers. Pure-code plugins are marked ts-only after install.".into());
        self.blocks.push(Block::Banner(lines));
    }

    fn start_run(&mut self, text: String) {
        self.fold_previous();
        self.think_scroll = 0;
        self.harvest_image_paths(&text);
        let atts: Vec<Attachment> = std::mem::take(&mut self.attachments);
        let keys: Vec<String> = atts.iter().map(|a| self.register_image(a.img.clone())).collect();
        self.blocks.push(Block::User(text.clone(), keys));
        let user_msg = if atts.is_empty() { Message::user(text.clone()) } else {
            let mut parts = vec![Content::text_part(&text)];
            for (i, a) in atts.iter().enumerate() { parts.push(Content::text_part(&format!("[image #{}: {} ({}×{}){}]", i + 1, a.path.display(), a.dims.0, a.dims.1, a.label.as_ref().map(|l| format!(" — {l}")).unwrap_or_default()))); parts.push(Content::image_part(&a.mime, &a.b64)); }
            Message::user_parts(parts)
        };
        self.scroll_up = 0;
        self.turn_tokens = 0;
        let tx = self.tx.clone(); let session = self.session.clone();
        let mut cfg = self.cfg.clone(); cfg.llm.model = self.model.clone(); cfg.net.enabled = self.net;
        let workdir = self.workdir.clone();
        let toolset = self.toolset.clone();
        let todos = self.todos.clone();
        let perm_mode = self.perm_mode;
        let budget = self.cfg.llm.effective_budget(if self.metrics.ctx_len > 0 { Some(self.metrics.ctx_len) } else { None });
        self.run_started = Instant::now();
        self.metrics.turn_start = Some(Instant::now()); self.metrics.live_peak = 0.0; self.metrics.live_chars.clear();
        let handle = tokio::spawn(async move {
            let res: Result<(String, harness::agent::RunStats), String> = async {
                let client = Client::new(&cfg.llm).map_err(|e| e.to_string())?;
                let store = if cfg.memory.enabled { harness::memory::MemoryStore::open(&cfg.memory).ok() } else { None };
                let fallback = Registry::defaults(cfg.net.enabled);
                let (registry, extra_prompt): (&Registry, String) = match &toolset { Some(ts) => (&ts.registry, ts.prompt_extra.clone()), None => (&fallback, String::new()) };
                let sink: Arc<dyn Sink> = Arc::new(TuiSink(tx.clone()));
                let mut pcfg = cfg.permissions.clone(); pcfg.mode = perm_mode; pcfg.allow.extend(harness::permissions::persisted_rules());
                let policy = Arc::new(harness::permissions::Policy::new(pcfg, &workdir));
                let approver: Arc<dyn harness::permissions::Approver> = Arc::new(TuiApprover(tx.clone()));
                let env = Arc::new(harness::agent::SubAgentEnv::new(client.clone(), registry.clone(), policy.clone(), approver.clone(), sink.clone(), budget, true));
                let ctx = ToolCtx { workdir: workdir.clone(), timeout: Duration::from_secs(cfg.agent.tool_timeout_secs), max_output: cfg.agent.max_tool_output_chars, net: cfg.net.clone(), memory: store.clone(), subagent: Some(env), redact_secrets: cfg.security.redact_secrets, hooks: cfg.hooks.clone(), todos: todos.clone() };
                let agent = Agent { client: &client, registry, ctx: &ctx, max_turns: cfg.agent.max_turns, context_budget: budget, sink: sink.as_ref(), stream: true, policy: &policy, approver: approver.as_ref() };
                let extra = format!("You are in an interactive session: the user can see everything and will reply; keep final answers concise.{extra_prompt}");
                let system = harness::agent::system_prompt_with_memory(&workdir.display().to_string(), &registry.names(), Some(&extra), store.as_ref());
                let mut msgs = session.lock().await;
                let out = agent.run_turn_message(&mut msgs, &system, user_msg).await.map_err(|e| format!("{e:#}"))?;
                Ok(out)
            }.await;
            let _ = tx.send(Msg::Done(res));
        });
        self.running = Some(handle);
    }

    /// Reflection runs *after* Done so the next queued task starts immediately; skipped when tasks are waiting.
    fn spawn_reflection(&mut self, stats: &harness::agent::RunStats) {
        if !self.cfg.memory.enabled || !self.cfg.memory.auto_reflect || stats.tool_calls < self.cfg.memory.reflect_min_tool_calls || stats.stop_reason != "done" { return; }
        if !self.queued.is_empty() { return; }
        let tx = self.tx.clone(); let session = self.session.clone(); let cfg = self.cfg.clone();
        tokio::spawn(async move {
            let sink = TuiSink(tx.clone());
            let Ok(store) = harness::memory::MemoryStore::open(&cfg.memory) else { return };
            let Ok(client) = Client::new(&cfg.llm) else { return };
            let client = client.aux();
            let msgs = session.lock().await.clone();
            match store.reflect(&client, &msgs).await {
                Ok(items) => for (f, s, t) in items { sink.emit(&Event::Memory { file: f, section: s, text: t }); },
                Err(_) => {}
            }
            if let Ok(done) = store.maybe_consolidate(&client).await { for f in done { sink.emit(&Event::Memory { file: f, section: "consolidated".into(), text: "merged and de-duplicated".into() }); } }
        });
    }

    /// Stop the current task (if any) and immediately start the next queued one.
    fn next_task(&mut self) {
        if self.running.is_some() {
            self.interrupt();
            // Done(Err interrupted) arrives asynchronously? No — abort is immediate and no Done is sent; start next now.
        }
        if let Some(next) = if self.queued.is_empty() { None } else { Some(self.queued.remove(0)) } { self.start_run(next); }
        else { self.set_status("no queued task"); }
    }

    /// Persist the transcript (called after every turn and on interrupt).
    fn save_session(&mut self) {
        if self.session_meta.id.is_empty() { self.session_meta.id = harness::sessions::SessionStore::new_id(); }
        self.session_meta.workdir = self.workdir.display().to_string();
        self.session_meta.model = self.model.clone();
        let session = self.session.clone(); let mut meta = self.session_meta.clone(); let tx = self.tx.clone();
        tokio::spawn(async move {
            let msgs = session.lock().await.clone();
            if msgs.len() < 2 { return; }
            if let Ok(store) = harness::sessions::SessionStore::open() { if let Err(e) = store.save(&mut meta, &msgs) { let _ = tx.send(Msg::Notice(format!("session save failed: {e:#}"))); } }
        });
    }

    fn resume_session(&mut self, which: &str) {
        let Ok(store) = harness::sessions::SessionStore::open() else { return };
        let id = if which == "last" || which == "latest" || which.is_empty() { match store.latest_for(&self.workdir.display().to_string()).or_else(|| store.list(None).into_iter().next()) { Some(m) => m.id, None => { self.blocks.push(Block::Error("no saved sessions".into())); return; } } }
            else if let Ok(n) = which.parse::<usize>() { match store.list(None).get(n.saturating_sub(1)) { Some(m) => m.id.clone(), None => { self.blocks.push(Block::Error(format!("no session #{n}"))); return; } } }
            else { which.to_string() };
        match store.load(&id) {
            Ok((meta, msgs)) => {
                self.blocks.clear(); self.banner();
                self.blocks.push(Block::System(format!("resumed session {} — {} · {} · {} turns", meta.id, meta.title, short_path(std::path::Path::new(&meta.workdir)), meta.turns)));
                self.replay(&msgs);
                if std::path::Path::new(&meta.workdir).is_dir() { self.workdir = PathBuf::from(&meta.workdir); let _ = std::env::set_current_dir(&self.workdir); }
                if !meta.model.is_empty() { self.model = meta.model.clone(); }
                self.session_meta = meta;
                let s = self.session.clone(); tokio::spawn(async move { *s.lock().await = msgs; });
                self.reload_toolset();
            }
            Err(e) => self.blocks.push(Block::Error(format!("resume: {e:#}"))),
        }
    }

    /// Rebuild transcript blocks from saved messages.
    fn replay(&mut self, msgs: &[Message]) {
        for m in msgs {
            match m.role.as_str() {
                "user" => { let t = m.text(); if !t.starts_with("[harness]") { self.blocks.push(Block::User(t, vec![])); } }
                "assistant" => {
                    let t = m.text(); if !t.trim().is_empty() { self.blocks.push(Block::Assistant { text: t, streaming: false, folded: true }); }
                    if let Some(calls) = &m.tool_calls { for c in calls { self.blocks.push(Block::Tool { id: c.id.clone(), name: c.function.name.clone(), args: c.function.arguments.clone(), result: None, secs: 0.0, images: 0, interrupted: false, fold: Some(true) }); } }
                }
                "tool" => { if let Some(id) = &m.tool_call_id { let t = m.text(); if let Some(Block::Tool { result, .. }) = self.blocks.iter_mut().rev().find(|b| matches!(b, Block::Tool { id: i, .. } if i == id)) { *result = Some(t); } } }
                _ => {}
            }
        }
    }

    fn interrupt(&mut self) {
        if let Some(h) = self.running.take() {
            h.abort();
            self.save_session();
            for b in self.blocks.iter_mut().rev() {
                match b {
                    Block::Tool { result: None, interrupted, .. } => { *interrupted = true; }
                    Block::Assistant { streaming, .. } | Block::Reasoning { streaming, .. } => { *streaming = false; }
                    _ => {}
                }
            }
            self.blocks.push(Block::System("interrupted".into()));
            self.set_status("Interrupted — the transcript is kept; type to continue");
        }
    }

    fn on_msg(&mut self, m: Msg) {
        match m {
            Msg::Ask(req, tx) => { self.blocks.push(Block::System(format!("🔒 approval needed — {}({}) · {}", req.tool, truncate(&req.summary, 100), req.reason))); self.pending_ask = Some((req, tx)); }
            Msg::Ev(e) => self.on_event(e),
            Msg::Sys(s) => self.metrics.on_sys(s),
            Msg::CtxLen(n) => { self.metrics.ctx_len = n; self.blocks.push(Block::System(format!("auto-compaction at {} tokens ({}% of context); /compact to force", fmt_k(self.cfg.llm.effective_budget(Some(n))), (self.cfg.llm.compact_at_fraction * 100.0) as u64))); }
            Msg::Toolset(ts) => { let n = ts.registry.len(); self.toolset = Some(ts); self.set_status(format!("tools ready: {n}")); }
            Msg::Notice(t) => self.blocks.push(Block::System(t)),
            Msg::Catalog(Ok(c)) => self.show_catalog(&c),
            Msg::Catalog(Err(e)) => self.blocks.push(Block::Error(format!("plugin catalog: {e}"))),
            Msg::Pasted(Ok(p)) => { if image_mime(&p).is_some() { self.attach(&p) } else if video_ext(&p) { self.open_video(&p) } else { let t = format!("{} ", p.display()); self.insert_str(&t); self.set_status(format!("inserted file path {}", short_path(&p))); } }
            Msg::Frames(Ok((path, duration, frames))) => {
                let mut fr = Vec::new();
                for (ts, p) in frames { if let Ok(bytes) = std::fs::read(&p) { if let Ok(img) = image::load_from_memory(&bytes) { let key = self.register_image(img); fr.push((ts, p, key)); } } }
                if let Some(v) = &mut self.video { if v.path == path { v.frames = fr; v.duration = duration; v.loading = false; v.cur = 0; } }
            }
            Msg::Frames(Err(e)) => { if let Some(v) = &mut self.video { v.loading = false; v.error = Some(e); } }
            Msg::Pasted(Err(e)) => self.set_status(e),
            Msg::Done(res) => {
                self.running = None;
                if self.cfg.ui.notify && self.run_started.elapsed() > Duration::from_secs(20) {
                    let title = match &res { Ok(_) => "Harness: task finished", Err(_) => "Harness: task stopped" };
                    let body = truncate(&self.blocks.iter().rev().find_map(|b| if let Block::User(t, _) = b { Some(t.clone()) } else { None }).unwrap_or_default(), 80).replace('"', "'");
                    let script = format!("display notification \"{body}\" with title \"{title}\" sound name \"Glass\"");
                    tokio::spawn(async move { let _ = tokio::process::Command::new("osascript").arg("-e").arg(script).output().await; });
                }
                match &res {
                    Ok((_, stats)) => { self.session_meta.prompt_tokens += stats.prompt_tokens; self.session_meta.completion_tokens += stats.completion_tokens; }
                    Err(_) => {}
                }
                self.save_session();
                match res {
                    Ok((_, stats)) => { self.spawn_reflection(&stats); }
                    Err(e) => { if !e.contains("interrupted") { self.blocks.push(Block::Error(e)); } }
                }
                if !self.queued.is_empty() {
                    let next = self.queued.remove(0);
                    self.set_status(format!("→ next task ({} left in queue)", self.queued.len()));
                    self.start_run(next);
                }
            }
        }
    }

    fn on_event(&mut self, e: Event) {
        if let Some(f) = &mut self.event_log { if !matches!(e, Event::ReasoningDelta { .. } | Event::AssistantDelta { .. } | Event::Turn { .. }) { use std::io::Write; let _ = writeln!(f, "{}", serde_json::to_string(&e).unwrap_or_default()); } }
        match e {
            Event::RunStarted { model, workdir, .. } if workdir == "\u{0}models" => { self.models = model.split('\u{1f}').map(String::from).collect(); }
            Event::RunStarted { .. } | Event::Turn { .. } => {}
            Event::ModelResponse { prompt_tokens, completion_tokens, ttft_secs, secs, .. } => { self.metrics.on_call(prompt_tokens, completion_tokens, ttft_secs, secs); self.last_prompt_tokens = prompt_tokens; }
            Event::ReasoningDelta { text } => {
                self.metrics.on_delta(text.chars().count());
                if let Some(Block::Reasoning { text: t, streaming: true, .. }) = self.blocks.last_mut() { t.push_str(&text); }
                else { self.blocks.push(Block::Reasoning { text, streaming: true, show: None }); }
            }
            Event::Reasoning { text } => {
                if let Some(Block::Reasoning { text: t, streaming, .. }) = self.blocks.last_mut() { *t = text; *streaming = false; }
                else if !text.trim().is_empty() { self.blocks.push(Block::Reasoning { text, streaming: false, show: None }); }
            }
            Event::AssistantDelta { text } => {
                self.metrics.on_delta(text.chars().count());
                if let Some(Block::Assistant { text: t, streaming: true, .. }) = self.blocks.last_mut() { t.push_str(&text); }
                else { self.finish_streaming(); self.blocks.push(Block::Assistant { text, streaming: true, folded: false }); }
            }
            Event::Assistant { text } => {
                if let Some(Block::Assistant { text: t, streaming, .. }) = self.blocks.last_mut() { *t = text; *streaming = false; }
                else if !text.trim().is_empty() { self.blocks.push(Block::Assistant { text, streaming: false, folded: false }); }
            }
            Event::ToolCall { id, name, args } => { self.finish_streaming(); self.blocks.push(Block::Tool { id, name, args, result: None, secs: 0.0, images: 0, interrupted: false, fold: None }); }
            Event::ToolResult { id, result, secs, images, .. } => {
                if !images.is_empty() {
                    use base64::Engine;
                    let mut keys = Vec::new();
                    for du in &images { if let Some(b64) = du.split(',').nth(1) { if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64) { if let Ok(img) = image::load_from_memory(&bytes) { keys.push(self.register_image(img)); } } } }
                    self.tool_previews.insert(id.clone(), keys);
                }
                if let Some(Block::Tool { result: r, secs: s, images: im, .. }) = self.blocks.iter_mut().rev().find(|b| matches!(b, Block::Tool { id: i, .. } if *i == id)) { *r = Some(result); *s = secs; *im = images.len(); }
            }
            Event::Compacted { count, prompt_tokens, summary } => {
                self.blocks.push(Block::System(format!("⟲ context compacted: {count} messages → handoff note (context was {} tokens)", fmt_k(prompt_tokens))));
                if !summary.is_empty() { self.blocks.push(Block::Assistant { text: format!("Handoff note (context compaction)\n{summary}"), streaming: false, folded: true }); }
            }
            Event::RunFinished { stop_reason, turns, tool_calls, prompt_tokens, completion_tokens, wall_secs } => {
                self.finish_streaming();
                self.total_prompt += prompt_tokens; self.total_completion += completion_tokens; self.turn_tokens = completion_tokens;
                let s = format!("{} · {} model call{} · {} tool call{} · {}+{} tokens · {:.0}s", if stop_reason == "done" { "done" } else { &stop_reason }, turns, if turns == 1 { "" } else { "s" }, tool_calls, if tool_calls == 1 { "" } else { "s" }, fmt_k(prompt_tokens), fmt_k(completion_tokens), wall_secs);
                self.blocks.push(Block::Finished(s));
            }
            Event::Error { message } => { self.finish_streaming(); self.blocks.push(Block::Error(message)); }
            Event::Memory { file, section, text } => { self.blocks.push(Block::Memory(format!("{} › {section}: {text}", file.trim_end_matches(".md")))); }
            Event::Permission { tool, summary, decision } => { if decision.starts_with("denied") { self.blocks.push(Block::Error(format!("🔒 {tool}({}) {decision}", truncate(&summary, 80)))); } }
        }
    }
    /// Click on a block: fold/unfold it.
    fn toggle_fold(&mut self, idx: usize) {
        let global_tools = self.expand_tools; let global_think = self.show_thinking;
        if let Some(b) = self.blocks.get_mut(idx) {
            match b {
                Block::Assistant { folded, .. } => *folded = !*folded,
                Block::Reasoning { show, .. } => { let cur = show.unwrap_or(global_think); *show = Some(!cur); }
                Block::Tool { fold, .. } => { let cur = fold.map(|f| !f).unwrap_or(global_tools); *fold = Some(cur); } // cur = expanded? → fold it
                _ => {}
            }
        }
    }
    /// When a new turn starts, collapse the previous turn's outputs (click to expand again).
    fn fold_previous(&mut self) {
        for b in self.blocks.iter_mut() {
            match b {
                Block::Assistant { text, folded, .. } => { if text.lines().count() > 6 { *folded = true; } }
                Block::Reasoning { show, .. } => *show = Some(false),
                Block::Tool { fold, .. } => *fold = Some(true),
                _ => {}
            }
        }
    }

    fn finish_streaming(&mut self) {
        for b in self.blocks.iter_mut().rev().take(3) {
            match b { Block::Assistant { streaming, .. } | Block::Reasoning { streaming, .. } => *streaming = false, _ => {} }
        }
    }
}

const COMMANDS: &[(&str, &str)] = &[
    ("/help", "show commands and keys"),
    ("/clear", "start a new session (forget the transcript)"),
    ("/sessions", "list saved sessions"),
    ("/resume", "resume a saved session: /resume <n|id|last>"),
    ("/model", "show or switch the model: /model <name>"),
    ("/cd", "change working directory"),
    ("/pwd", "print working directory"),
    ("/tools", "list the tools the model can call"),
    ("/net", "internet tools on|off"),
    ("/thinking", "toggle showing the model's reasoning"),
    ("/expand", "toggle expanded tool output (ctrl+o)"),
    ("/panel", "toggle the dashboard panel (ctrl+p)"),
    ("/cost", "token usage for this session"),
    ("/compact", "compact the context into a precise handoff note: /compact [focus]"),
    ("/config", "effective configuration"),
    ("/memory", "show MEMORY.md (settings · preferences · ideas)"),
    ("/brain", "show BRAIN.md (what the agent learned)"),
    ("/workflows", "show WORKFLOWS.md (recipes)"),
    ("/remember", "add a note: /remember <text> | brain: <text> | workflows: <text>"),
    ("/reflect", "ask the model what to remember from this session"),
    ("/video", "open the frame scrubber for a video: /video <path>"),
    ("/plugin", "plugins: list · install <owner/repo> · enable|disable|remove|update|info <name>"),
    ("/mcp", "show configured MCP servers and live MCP tools"),
    ("/reload", "restart tools, MCP servers and plugins"),
    ("/permissions", "show or set permission mode: bypass|auto|ask|plan"),
    ("/plan", "toggle plan mode (read-only)"),
    ("/theme", "switch theme: /theme light|dark"),
    ("/queue", "show queued tasks (/queue clear)"),
    ("/next", "stop the current task and start the next queued one (ctrl+n)"),
    ("/exit", "quit"),
];

// ───────────────────────── rendering ─────────────────────────
fn draw(f: &mut Frame, app: &mut App) {
    let full = f.area();
    let show_panel = app.panel_visible(full.width);
    let (area, panel_area) = if show_panel {
        let pw = (full.width / 3).clamp(36, 56);
        let cols = Layout::horizontal([Constraint::Min(40), Constraint::Length(1), Constraint::Length(pw)]).split(full);
        (cols[0], Some((cols[1], cols[2])))
    } else { (full, None) };
    if let Some((div, pa)) = panel_area {
        let divider: Vec<Line> = (0..div.height).map(|_| Line::from(Span::styled("│", Style::default().fg(pal().dim)))).collect();
        f.render_widget(Paragraph::new(divider), div);
        draw_panel(f, app, pa);
    }
    let width = area.width as usize;
    // input geometry
    let input_lines = wrap_input(&app.input, width.saturating_sub(2).max(1));
    let sugg = suggestions(&app.input);
    let input_h = (input_lines.len().clamp(1, 8) + sugg.len() + if app.attachments.is_empty() { 0 } else { 1 }) as u16;
    // notice line above the box: spinner while running, or a transient status message
    let notice: Option<Vec<Span>> = if let Some((req, _)) = &app.pending_ask {
        Some(vec![Span::styled(" 🔒 ", Style::default().fg(Color::Black).bg(pal().orange)), Span::styled(format!(" {}({}) ", req.tool, truncate(&req.summary, width.saturating_sub(70))), Style::default().fg(Color::Black).bg(pal().orange).bold()),
                  Span::styled(format!("  {} · ", req.reason), Style::default().fg(pal().orange)),
                  Span::styled("[y] allow once  ", Style::default().fg(pal().ok).bold()), Span::styled(format!("[a] always ({})  ", req.suggested_rule), Style::default().fg(pal().cyan)), Span::styled("[n] deny", Style::default().fg(pal().err).bold())])
    } else if app.running.is_some() {
        let sp = SPINNER[(app.tick as usize / 2) % SPINNER.len()];
        let el = app.run_started.elapsed().as_secs();
        let live = app.metrics.live_tps();
        Some(vec![Span::styled(format!("{sp} {}… ", WORDS[app.word]), Style::default().fg(pal().orange)),
                  Span::styled(format!("({el}s · {} tok/s · esc to interrupt{})", if live > 0.0 { format!("{live:.0}") } else { "–".into() }, if app.queued.is_empty() { String::new() } else { format!(" · {} queued", app.queued.len()) }), Style::default().fg(pal().dim))])
    } else if let Some((m, t)) = &app.status_msg { if t.elapsed() < Duration::from_secs(4) { Some(vec![Span::styled(format!("· {m}"), Style::default().fg(pal().orange))]) } else { None } } else { None };
    let notice_h = if notice.is_some() { 1 } else { 0 };
    let chunks = Layout::vertical([Constraint::Min(1), Constraint::Length(notice_h), Constraint::Length(1), Constraint::Length(input_h), Constraint::Length(1), Constraint::Length(1)]).split(area);
    let (tr_area, no_area, top_area, in_area, bot_area, st_area) = (chunks[0], chunks[1], chunks[2], chunks[3], chunks[4], chunks[5]);

    if app.video.is_some() { draw_video(f, app, tr_area); }
    // transcript
    let mut lines: Vec<Line> = Vec::new();
    let mut ph: Vec<Placeholder> = Vec::new();
    let mut line_map: Vec<(usize, usize, usize)> = Vec::new();
    for (i, b) in app.blocks.iter().enumerate() { let a = lines.len(); render_block(b, app, width, &mut lines, &mut ph); line_map.push((a, lines.len(), i)); }
    let total = lines.len();
    let h = tr_area.height as usize;
    let max_up = total.saturating_sub(h);
    if app.scroll_up > max_up { app.scroll_up = max_up; }
    let start = max_up - app.scroll_up;
    app.line_map = line_map; app.tr_rect = tr_area; app.tr_start = start;
    app.panel_rect = panel_area.map(|(_, pa)| pa).unwrap_or_default();
    let visible: Vec<Line> = lines.into_iter().skip(start).take(h).collect();
    if app.video.is_none() { f.render_widget(Paragraph::new(visible), tr_area); }
    // images: draw those whose slot is fully inside the visible window
    for p in ph {
        if app.video.is_some() { break; }
        let rows = p.slot.rows as usize;
        if p.line >= start && p.line + rows <= start + h {
            let indent = if p.slot.key.is_empty() { 0 } else { 0 };
            let rect = Rect { x: tr_area.x + 2 + indent, y: tr_area.y + (p.line - start) as u16, width: p.slot.cols.min(tr_area.width.saturating_sub(2)), height: p.slot.rows };
            if let Some((proto, _)) = app.images.get_mut(&p.slot.key) { f.render_stateful_widget(StatefulImage::default(), rect, proto); }
        }
    }
    if app.scroll_up > 0 {
        let tag = format!(" ↓ {} more lines ", app.scroll_up);
        let r = Rect { x: area.x + area.width.saturating_sub(tag.len() as u16 + 1), y: tr_area.bottom().saturating_sub(1), width: tag.len() as u16, height: 1 };
        f.render_widget(Paragraph::new(Span::styled(tag, Style::default().fg(Color::Black).bg(pal().orange))), r);
    }
    if let Some(n) = notice { f.render_widget(Paragraph::new(Line::from(n)), no_area); }

    // input box: rule / › text / rule
    let rule = Line::from(Span::styled("─".repeat(width), Style::default().fg(pal().dim)));
    f.render_widget(Paragraph::new(rule.clone()), top_area);
    let mut in_lines: Vec<Line> = Vec::new();
    for (i, l) in input_lines.iter().enumerate().take(8) {
        let prompt = if i == 0 { Span::styled("› ", Style::default().fg(pal().fg).bold()) } else { Span::raw("  ") };
        in_lines.push(Line::from(vec![prompt, Span::raw(l.clone())]));
    }
    if app.input.is_empty() {
        in_lines[0] = Line::from(vec![Span::styled("› ", Style::default().fg(pal().fg).bold()), Span::styled(if app.running.is_some() { "type to queue the next message…" } else { "Ask the agent to do something… (/help)" }, Style::default().fg(pal().dim))]);
    }
    for (c, d) in &sugg { in_lines.push(Line::from(vec![Span::raw("  "), Span::styled(format!("{c:<12}"), Style::default().fg(pal().blue)), Span::styled(d.to_string(), Style::default().fg(pal().dim))])); }
    if !app.attachments.is_empty() {
        let mut spans = vec![Span::styled("  📎 ", Style::default().fg(pal().blue))];
        for (i, a) in app.attachments.iter().enumerate() { spans.push(Span::styled(format!("#{} {} {}×{}  ", i + 1, a.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(), a.dims.0, a.dims.1), Style::default().fg(pal().blue))); }
        in_lines.push(Line::from(spans));
    }
    f.render_widget(Paragraph::new(in_lines), in_area);
    f.render_widget(Paragraph::new(rule), bot_area);
    // cursor
    let (crow, ccol) = cursor_pos(&app.input, app.cursor, width.saturating_sub(2).max(1));
    if crow < 8 { f.set_cursor_position((in_area.x + 2 + ccol as u16, in_area.y + crow as u16)); }

    // mode line: ▶▶ bypass permissions on · model · cwd · ctx
    let dot = || Span::styled(" · ", Style::default().fg(pal().dim));
    let (mode_txt, mode_col) = match app.perm_mode { harness::permissions::Mode::Bypass => ("▶▶ bypass permissions on", pal().pink), harness::permissions::Mode::Auto => ("▶▶ auto permissions", pal().cyan), harness::permissions::Mode::Ask => ("▶▶ ask before changes", pal().orange), harness::permissions::Mode::Plan => ("▶▶ plan mode · read-only", pal().think) };
    let mut st = vec![Span::styled(format!("  {mode_txt}"), Style::default().fg(mode_col)), dot(),
        Span::styled(app.model.clone(), Style::default().fg(pal().cyan)), dot(),
        Span::styled(short_path(&app.workdir), Style::default().fg(pal().cyan)), dot(),
        Span::styled(format!("ctx {}", fmt_k(app.last_prompt_tokens)), Style::default().fg(pal().cyan))];
    if !app.net { st.push(dot()); st.push(Span::styled("offline", Style::default().fg(pal().pink))); }
    if !app.queued.is_empty() { st.push(dot()); st.push(Span::styled(format!("{} queued", app.queued.len()), Style::default().fg(pal().cyan))); }
    let lw: usize = st.iter().map(|s| s.content.chars().count()).sum();
    let right = if app.running.is_none() { "? for shortcuts · /help" } else { "esc to interrupt" };
    let pad = width.saturating_sub(lw + right.chars().count() + 1);
    st.push(Span::raw(" ".repeat(pad))); st.push(Span::styled(right, Style::default().fg(pal().dim)));
    f.render_widget(Paragraph::new(Line::from(st)), st_area);
}

// ───────────────────────── video scrubber ─────────────────────────
fn draw_video(f: &mut Frame, app: &mut App, area: Rect) {
    let dim = Style::default().fg(pal().dim);
    f.render_widget(ratatui::widgets::Clear, area);
    let Some(v) = &app.video else { return };
    let title = format!(" 🎞  {}  ·  {:.1}s  ·  {} frames  ·  {} selected ", short_path(&v.path), v.duration, v.frames.len(), v.selected.len());
    let rows = Layout::vertical([Constraint::Length(1), Constraint::Min(8), Constraint::Length(1), Constraint::Length(7), Constraint::Length(2)]).split(area);
    f.render_widget(Paragraph::new(Line::from(vec![Span::styled(title, Style::default().fg(Color::Black).bg(pal().orange).bold())])), rows[0]);
    if v.loading { f.render_widget(Paragraph::new(Span::styled("  extracting frames with ffmpeg…", Style::default().fg(pal().orange))), rows[1]); }
    else if let Some(e) = &v.error { f.render_widget(Paragraph::new(Span::styled(format!("  {e}"), Style::default().fg(pal().err))), rows[1]); }
    // main frame
    let cur = v.cur; let frames: Vec<(f64, String)> = v.frames.iter().map(|(t, _, k)| (*t, k.clone())).collect();
    let selected = v.selected.clone();
    let (cw, ch) = app.picker.font_size();
    if let Some((ts, key)) = frames.get(cur).cloned() {
        if let Some((proto, (iw, ih))) = app.images.get_mut(&key) {
            let (iw, ih) = (*iw as f64, *ih as f64);
            let max_cols = rows[1].width.saturating_sub(4) as f64; let max_rows = rows[1].height.saturating_sub(1) as f64;
            let scale = f64::min(max_cols * cw as f64 / iw, max_rows * ch as f64 / ih);
            let cols = ((iw * scale / cw as f64).floor() as u16).max(1); let rws = ((ih * scale / ch as f64).floor() as u16).max(1);
            let x = rows[1].x + (rows[1].width.saturating_sub(cols)) / 2;
            f.render_stateful_widget(StatefulImage::default(), Rect { x, y: rows[1].y + 1, width: cols, height: rws }, proto);
        }
        let mark = if selected.contains(&cur) { "● selected" } else { "○ not selected" };
        f.render_widget(Paragraph::new(Line::from(vec![Span::styled(format!("  frame {}/{}  ·  t = {:.1}s  ·  ", cur + 1, frames.len(), ts), dim), Span::styled(mark, Style::default().fg(if selected.contains(&cur) { pal().ok } else { pal().dim }))])), rows[2]);
    }
    // strip: thumbnails around cur
    app.strip_rects.clear();
    let tw: u16 = 14; let th: u16 = 6; let gap = 1;
    let per = ((rows[3].width as usize) / (tw as usize + gap)).max(1);
    let first = cur.saturating_sub(per / 2).min(frames.len().saturating_sub(per));
    let mut x = rows[3].x;
    for i in first..(first + per).min(frames.len()) {
        let r = Rect { x, y: rows[3].y, width: tw, height: th };
        let (_, key) = &frames[i];
        if let Some((proto, _)) = app.images.get_mut(key) { f.render_stateful_widget(StatefulImage::default(), Rect { x: r.x + 1, y: r.y, width: tw - 2, height: th - 1 }, proto); }
        let lbl = format!("{}{:.1}s", if selected.contains(&i) { "●" } else { " " }, frames[i].0);
        let st = if i == cur { Style::default().fg(Color::Black).bg(pal().orange).bold() } else if selected.contains(&i) { Style::default().fg(pal().ok) } else { dim };
        f.render_widget(Paragraph::new(Span::styled(format!("{:^w$}", lbl, w = tw as usize), st)), Rect { x: r.x, y: r.y + th - 1, width: tw, height: 1 });
        app.strip_rects.push((r, i));
        x += tw + gap as u16;
    }
    f.render_widget(Paragraph::new(vec![
        Line::from(Span::styled("  ←/→ or wheel: move · space or click: select · a: all · enter: attach selected (or current) frames · esc: cancel", dim)),
        Line::from(Span::styled("  Selected frames are sent to the model as images with their timestamps; frame files are kept in the pastes folder.", dim)),
    ]), rows[4]);
}

// ───────────────────────── dashboard panel ─────────────────────────
fn draw_panel(f: &mut Frame, app: &App, area: Rect) {
    let m = &app.metrics;
    let title = |t: &str| Line::from(vec![Span::styled(format!("── {t} "), Style::default().fg(pal().orange).bold()), Span::styled("─".repeat((area.width as usize).saturating_sub(t.len() + 4)), Style::default().fg(pal().dim))]);
    let dim = Style::default().fg(pal().dim);
    let running = app.running.is_some();
    let todos: Vec<harness::tools::todo::TodoItem> = app.todos.lock().map(|t| t.clone()).unwrap_or_default();
    let todo_h = if todos.is_empty() { 0 } else { (todos.len() as u16).min(8) + 1 };
    let rows = Layout::vertical([
        Constraint::Length(1), Constraint::Min(6),          // thinking
        Constraint::Length(todo_h),                         // tasks
        Constraint::Length(1), Constraint::Length(6),       // tokens
        Constraint::Length(1), Constraint::Length(8),       // speed
        Constraint::Length(1), Constraint::Length(9),       // system
    ]).split(area);
    let (r_tokens_t, r_tokens, r_speed_t, r_speed, r_sys_t, r_sys) = (rows[3], rows[4], rows[5], rows[6], rows[7], rows[8]);
    if todo_h > 0 {
        let done = todos.iter().filter(|t| t.status == "done").count();
        let mut tl: Vec<Line> = vec![title(&format!("Tasks {}/{}", done, todos.len()))];
        for t in todos.iter().take(8) { let (mark, st) = match t.status.as_str() { "done" => ("☑ ", Style::default().fg(pal().ok)), "in_progress" => ("▶ ", Style::default().fg(pal().orange).bold()), _ => ("☐ ", dim) }; tl.push(Line::from(vec![Span::styled(mark, st), Span::styled(truncate(&t.text, area.width.saturating_sub(4) as usize), if t.status == "done" { dim } else { Style::default() })])); }
        f.render_widget(Paragraph::new(tl), rows[2]);
    }

    // ── Thinking ──
    f.render_widget(Paragraph::new(title(&format!("{}{}", if running { "Thinking · live" } else { "Thinking · last" }, if app.think_scroll > 0 { " ↑" } else { "" }))), rows[0]);
    let think = app.blocks.iter().rev().find_map(|b| if let Block::Reasoning { text, .. } = b { Some(text.clone()) } else { None }).unwrap_or_default();
    let tw = rows[1].width as usize;
    let mut tl: Vec<Line> = Vec::new();
    for l in think.lines().filter(|l| !l.trim().is_empty()) { push_wrapped(&mut tl, vec![Span::styled(l.trim().to_string(), Style::default().fg(pal().think))], tw, 0); }
    let th = rows[1].height as usize;
    let max_up = tl.len().saturating_sub(th);
    let up = app.think_scroll.min(max_up);
    let skip = max_up - up;
    let tail: Vec<Line> = tl.into_iter().skip(skip).take(th).collect();
    if tail.is_empty() { f.render_widget(Paragraph::new(Span::styled("(reasoning will stream here)", dim)), rows[1]); } else { f.render_widget(Paragraph::new(tail), rows[1]); }

    // ── Tokens ──
    f.render_widget(Paragraph::new(title("Tokens")), r_tokens_t);
    let tk = Layout::vertical([Constraint::Length(1), Constraint::Length(1), Constraint::Length(1), Constraint::Length(3)]).split(r_tokens);
    let ctx = m.ctx_len.max(1);
    let ratio = (app.last_prompt_tokens as f64 / ctx as f64).clamp(0.0, 1.0);
    let gcolor = if ratio > 0.85 { pal().err } else if ratio > 0.6 { pal().orange } else { pal().ok };
    f.render_widget(Gauge::default().gauge_style(Style::default().fg(gcolor).bg(pal().panel_bg)).ratio(ratio).label(format!("context {} / {} ({:.0}%)", fmt_k(app.last_prompt_tokens), fmt_k(ctx), ratio * 100.0)), tk[0]);
    f.render_widget(Paragraph::new(Line::from(vec![Span::styled("session ", dim), Span::raw(format!("{} in · {} out", fmt_k(app.total_prompt), fmt_k(app.total_completion))), Span::styled(format!(" · {} calls", m.calls), dim)])), tk[1]);
    let (lp, lc) = m.last_call.map(|(p, c, _, _)| (p, c)).unwrap_or((0, 0));
    f.render_widget(Paragraph::new(Line::from(vec![Span::styled("last call ", dim), Span::raw(format!("{} in · {} out", fmt_k(lp), fmt_k(lc))), Span::styled("   out/call ▸", dim)])), tk[2]);
    f.render_widget(Sparkline::default().data(&m.completion_per_call.iter().cloned().collect::<Vec<_>>()).style(Style::default().fg(pal().blue)), tk[3]);

    // ── Speed ──
    f.render_widget(Paragraph::new(title("Speed")), r_speed_t);
    let sp = Layout::vertical([Constraint::Length(1), Constraint::Length(1), Constraint::Length(1), Constraint::Min(3)]).split(r_speed);
    let live = if running { m.live_tps() } else { 0.0 };
    let (ttft, gen, psp) = m.last_call.map(|(p, c, t, s)| (t, if s > t && c > 0 { c as f64 / (s - t) } else { 0.0 }, if t > 0.0 { p as f64 / t } else { 0.0 })).unwrap_or((0.0, 0.0, 0.0));
    f.render_widget(Paragraph::new(Line::from(vec![Span::styled("live ", dim), Span::styled(format!("{live:>5.1} tok/s"), Style::default().fg(if running { pal().orange } else { pal().dim }).bold()), Span::styled(format!("  peak {:.1}", m.live_peak), dim)])), sp[0]);
    f.render_widget(Paragraph::new(Line::from(vec![Span::styled("gen  ", dim), Span::raw(format!("{gen:>5.1} tok/s")), Span::styled("  ttft ", dim), Span::raw(format!("{:.2}s", ttft))])), sp[1]);
    f.render_widget(Paragraph::new(Line::from(vec![Span::styled("prompt ", dim), Span::raw(format!("{psp:>6.0} tok/s")), Span::styled(format!("  turn {}s", m.turn_start.map(|t| t.elapsed().as_secs()).filter(|_| running).unwrap_or(0)), dim)])), sp[2]);
    // chart: gen tok/s per call
    let pts: Vec<(f64, f64)> = m.gen_speed.iter().enumerate().map(|(i, v)| (i as f64, *v as f64)).collect();
    if pts.len() >= 2 {
        let ymax = pts.iter().map(|p| p.1).fold(1.0, f64::max) * 1.15;
        let ds = Dataset::default().name("tok/s").marker(symbols::Marker::Braille).graph_type(GraphType::Line).style(Style::default().fg(pal().ok)).data(&pts);
        let chart = Chart::new(vec![ds])
            .x_axis(Axis::default().bounds([0.0, (pts.len() - 1) as f64]).style(dim))
            .y_axis(Axis::default().bounds([0.0, ymax]).labels(vec![Span::styled("0", dim), Span::styled(format!("{:.0}", ymax), dim)]).style(dim));
        f.render_widget(chart, sp[3]);
    } else {
        f.render_widget(Paragraph::new(Span::styled("(gen tok/s per call chart after 2 calls)", dim)), sp[3]);
    }

    // ── System ──
    f.render_widget(Paragraph::new(title("System")), r_sys_t);
    let sy = Layout::vertical([Constraint::Length(1), Constraint::Length(2), Constraint::Length(1), Constraint::Length(2), Constraint::Length(1), Constraint::Length(2)]).split(r_sys);
    let last = &m.last;
    let cpu = last.cpu;
    f.render_widget(Paragraph::new(Line::from(vec![Span::styled("cpu ", dim), Span::styled(format!("{cpu:>5.1}%"), Style::default().fg(if cpu > 80.0 { pal().err } else { pal().fg })), Span::styled(format!("   harness rss {}", fmt_bytes(last.harness_rss)), dim)])), sy[0]);
    f.render_widget(Sparkline::default().data(&m.cpu.iter().cloned().collect::<Vec<_>>()).max(100).style(Style::default().fg(pal().blue)), sy[1]);
    match (last.gpu_util, last.gpu_mem) {
        (Some(g), gm) => {
            f.render_widget(Paragraph::new(Line::from(vec![Span::styled("gpu ", dim), Span::styled(format!("{g:>5.0}%"), Style::default().fg(if g > 80.0 { pal().orange } else { pal().fg })), Span::styled(format!("   gpu mem {}", gm.map(fmt_bytes).unwrap_or_else(|| "?".into())), dim)])), sy[2]);
            f.render_widget(Sparkline::default().data(&m.gpu.iter().cloned().collect::<Vec<_>>()).max(100).style(Style::default().fg(pal().think)), sy[3]);
        }
        _ => { f.render_widget(Paragraph::new(Span::styled("gpu  n/a on this platform", dim)), sy[2]); }
    }
    let mr = if last.mem_total > 0 { last.mem_used as f64 / last.mem_total as f64 } else { 0.0 };
    f.render_widget(Paragraph::new(Line::from(vec![Span::styled("ram ", dim), Span::raw(format!("{} / {}", fmt_bytes(last.mem_used), fmt_bytes(last.mem_total))), Span::styled(format!("   server rss {}", fmt_bytes(last.server_rss)), dim)])), sy[4]);
    f.render_widget(Gauge::default().gauge_style(Style::default().fg(if mr > 0.9 { pal().err } else { pal().blue }).bg(pal().panel_bg)).ratio(mr.clamp(0.0, 1.0)).label(format!("{:.0}%", mr * 100.0)), Rect { height: 1, ..sy[5] });
}

fn fmt_bytes(n: u64) -> String { if n < 1 << 20 { format!("{} KB", n >> 10) } else if n < 1 << 30 { format!("{:.0} MB", n as f64 / 1048576.0) } else { format!("{:.1} GB", n as f64 / 1073741824.0) } }

fn suggestions(input: &str) -> Vec<(&'static str, &'static str)> {
    if !input.starts_with('/') || input.contains(' ') { return vec![]; }
    COMMANDS.iter().filter(|(c, _)| c.starts_with(input)).take(6).cloned().collect()
}

/// Reserve `rows` blank lines for an image and remember where it goes.
fn image_slot(app: &App, key: &str, max_cols: u16, max_rows: u16, indent: usize, out: &mut Vec<Line<'static>>, ph: &mut Vec<Placeholder>) {
    let Some((_, (iw, ih))) = app.images.get(key) else { return };
    let (cw, ch) = app.picker.font_size();
    let (cw, ch) = (cw.max(1) as f64, ch.max(1) as f64);
    let scale = f64::min(max_cols as f64 * cw / *iw as f64, max_rows as f64 * ch / *ih as f64).min(1.0);
    let cols = ((*iw as f64 * scale / cw).ceil() as u16).clamp(1, max_cols);
    let rows = ((*ih as f64 * scale / ch).ceil() as u16).clamp(1, max_rows);
    ph.push(Placeholder { line: out.len(), slot: ImgSlot { key: key.to_string(), cols, rows } });
    for _ in 0..rows { out.push(Line::from(Span::raw(" ".repeat(indent)))); }
}

fn render_block(b: &Block, app: &App, width: usize, out: &mut Vec<Line<'static>>, ph: &mut Vec<Placeholder>) {
    let w = width.max(10);
    match b {
        Block::Banner(ls) => {
            let inner = ls.iter().map(|l| l.chars().count()).max().unwrap_or(0).min(w.saturating_sub(4));
            let bs = Style::default().fg(pal().orange);
            out.push(Line::from(Span::styled(format!("╭{}╮", "─".repeat(inner + 2)), bs)));
            for l in ls { let t = truncate(l, inner); out.push(Line::from(vec![Span::styled("│ ", bs), Span::raw(format!("{:<inner$}", t)), Span::styled(" │", bs)])); }
            out.push(Line::from(Span::styled(format!("╰{}╯", "─".repeat(inner + 2)), bs)));
            out.push(Line::raw(""));
        }
        Block::User(t, imgs) => {
            out.push(Line::raw(""));
            for (i, l) in t.lines().enumerate() { push_wrapped(out, vec![Span::styled(if i == 0 { "› " } else { "  " }, Style::default().fg(pal().dim)), Span::styled(l.to_string(), Style::default().bold())], w, 2); }
            for k in imgs { image_slot(app, k, (w.saturating_sub(4)).min(60) as u16, 12, 2, out, ph); out.push(Line::raw("")); }
            out.push(Line::raw(""));
        }
        Block::Assistant { text, streaming, folded } => {
            if *folded && !*streaming {
                let n = text.lines().count();
                let first = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("").to_string();
                push_wrapped(out, vec![Span::styled("⏺ ", Style::default().fg(pal().fg)), Span::raw(truncate(&first, w.saturating_sub(30))), Span::styled(format!("  … +{} lines (click)", n.saturating_sub(1)), Style::default().fg(pal().dim).italic())], w, 2);
                out.push(Line::raw(""));
                return;
            }
            let mut first = true;
            for l in text.lines() {
                let bullet = if first { Span::styled("⏺ ", Style::default().fg(pal().fg)) } else { Span::raw("  ") };
                push_wrapped(out, vec![bullet, Span::raw(l.to_string())], w, 2);
                first = false;
            }
            if *streaming { if let Some(last) = out.last_mut() { last.spans.push(Span::styled("▍", Style::default().fg(pal().orange))); } }
            if text.is_empty() && *streaming { out.push(Line::from(vec![Span::styled("⏺ ", Style::default().fg(pal().fg)), Span::styled("▍", Style::default().fg(pal().orange))])); }
            out.push(Line::raw(""));
        }
        Block::Reasoning { text, streaming, show } => {
            let st = Style::default().fg(pal().think).italic();
            if show.unwrap_or(app.show_thinking) {
                let mut first = true;
                for l in text.lines().filter(|l| !l.trim().is_empty()) { push_wrapped(out, vec![Span::styled(if first { "✻ " } else { "  " }, st), Span::styled(l.to_string(), st)], w, 2); first = false; }
                if *streaming { if let Some(last) = out.last_mut() { last.spans.push(Span::styled("▍", st)); } }
            } else {
                let firstline = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string();
                let n = text.chars().count();
                let lbl = if *streaming { format!("✻ Thinking… {}", truncate(&firstline, w.saturating_sub(40))) } else { format!("✻ Thought for {} chars: {}", n, truncate(&firstline, w.saturating_sub(40))) };
                push_wrapped(out, vec![Span::styled(lbl, st), Span::styled("  (click · ctrl+t)", Style::default().fg(pal().dim))], w, 2);
            }
            out.push(Line::raw(""));
        }
        Block::Tool { id, name, args, result, secs, images, interrupted, fold } => {
            let expanded = fold.map(|f| !f).unwrap_or(app.expand_tools);
            let (bullet_style, done) = match (result, interrupted) {
                (Some(r), _) if r.starts_with("error:") => (Style::default().fg(pal().err), true),
                (Some(_), _) => (Style::default().fg(pal().ok), true),
                (None, true) => (Style::default().fg(pal().err), true),
                (None, false) => (Style::default().fg(if (app.tick / 4) % 2 == 0 { pal().orange } else { pal().dim }), false),
            };
            let summary = args_summary(name, args, w.saturating_sub(name.len() + 6));
            push_wrapped(out, vec![Span::styled("⏺ ", bullet_style), Span::styled(name.clone(), Style::default().bold()), Span::styled(format!("({summary})"), Style::default().fg(pal().dim))], w, 2);
            match result {
                None if *interrupted => out.push(Line::from(vec![Span::styled("  ⎿  ", Style::default().fg(pal().dim)), Span::styled("interrupted", Style::default().fg(pal().err))])),
                None => out.push(Line::from(vec![Span::styled("  ⎿  ", Style::default().fg(pal().dim)), Span::styled("running…", Style::default().fg(pal().dim))])),
                Some(r) => {
                    let is_err = r.starts_with("error:");
                    let diffish = matches!(name.as_str(), "edit_file" | "apply_patch" | "write_file") || name.ends_with("edit_file");
                    let lines: Vec<&str> = r.lines().collect();
                    let show = if expanded { lines.len().min(60) } else if diffish && fold.is_none() { lines.len().min(12) } else { 1 };
                    for (i, l) in lines.iter().take(show).enumerate() {
                        let pre = if i == 0 { "  ⎿  " } else { "     " };
                        let lstyle = if is_err { Style::default().fg(pal().err) } else if diffish && l.starts_with("+ ") { Style::default().fg(pal().ok) } else if diffish && l.starts_with("- ") { Style::default().fg(pal().err) } else { Style::default().fg(pal().dim) };
                        let mut spans = vec![Span::styled(pre, Style::default().fg(pal().dim)), Span::styled(l.to_string(), lstyle)];
                        if i == show - 1 && lines.len() > show { spans.push(Span::styled(format!("  … +{} lines (click · ctrl+o)", lines.len() - show), Style::default().fg(pal().dim).italic())); }
                        push_wrapped(out, spans, w, 5);
                    }
                    if lines.is_empty() { out.push(Line::from(vec![Span::styled("  ⎿  ", Style::default().fg(pal().dim)), Span::styled("(no output)", Style::default().fg(pal().dim))])); }
                    if *images > 0 {
                        out.push(Line::from(vec![Span::styled("     ", Style::default()), Span::styled(format!("[{} image{} shown to the model]", images, if *images == 1 { "" } else { "s" }), Style::default().fg(pal().blue))]));
                        if let Some(keys) = app.tool_previews.get(id) { for k in keys { image_slot(app, k, (w.saturating_sub(7)).min(60) as u16, 14, 5, out, ph); } }
                    }
                    let _ = (done, secs);
                }
            }
        }
        Block::System(t) => { push_wrapped(out, vec![Span::styled("· ", Style::default().fg(pal().dim)), Span::styled(t.clone(), Style::default().fg(pal().dim))], w, 2); }
        Block::Memory(t) => { push_wrapped(out, vec![Span::styled("🧠 ", Style::default()), Span::styled(t.clone(), Style::default().fg(pal().ok))], w, 3); }
        Block::Error(t) => { for (i, l) in t.lines().enumerate() { push_wrapped(out, vec![Span::styled(if i == 0 { "✗ " } else { "  " }, Style::default().fg(pal().err)), Span::styled(l.to_string(), Style::default().fg(pal().err))], w, 2); } out.push(Line::raw("")); }
        Block::Finished(t) => { push_wrapped(out, vec![Span::styled("  ✓ ", Style::default().fg(pal().ok)), Span::styled(t.clone(), Style::default().fg(pal().dim))], w, 4); }
    }
}

fn args_summary(name: &str, args: &str, max: usize) -> String {
    let v: serde_json::Value = serde_json::from_str(args).unwrap_or(serde_json::Value::Null);
    let s = match name {
        "bash" => v["cmd"].as_str().unwrap_or(args).to_string(),
        "read_file" | "write_file" | "edit_file" | "list_dir" | "view_image" | "read_pdf" | "extract_archive" => v["path"].as_str().unwrap_or(args).to_string(),
        "web_fetch" | "download_file" => v["url"].as_str().unwrap_or(args).to_string(),
        "web_search" => v["query"].as_str().unwrap_or(args).to_string(),
        _ => args.to_string(),
    };
    truncate(&s.replace('\n', "⏎"), max.max(8))
}

/// Wrap spans to `width`, continuation lines indented by `indent`. Char/width aware.
fn push_wrapped(out: &mut Vec<Line<'static>>, spans: Vec<Span<'static>>, width: usize, indent: usize) {
    let mut cur: Vec<Span<'static>> = Vec::new();
    let mut col = 0usize;
    for sp in spans {
        let style = sp.style;
        let mut buf = String::new();
        for ch in sp.content.chars() {
            let cw = UnicodeWidthChar::width(ch).unwrap_or(1);
            if col + cw > width {
                if !buf.is_empty() { cur.push(Span::styled(std::mem::take(&mut buf), style)); }
                out.push(Line::from(std::mem::take(&mut cur)));
                cur.push(Span::raw(" ".repeat(indent)));
                col = indent;
            }
            buf.push(ch); col += cw;
        }
        if !buf.is_empty() { cur.push(Span::styled(buf, style)); }
    }
    out.push(Line::from(cur));
}

fn wrap_input(input: &str, width: usize) -> Vec<String> {
    let mut rows = Vec::new();
    for line in input.split('\n') {
        let mut cur = String::new(); let mut col = 0;
        for ch in line.chars() {
            let cw = UnicodeWidthChar::width(ch).unwrap_or(1);
            if col + cw > width { rows.push(std::mem::take(&mut cur)); col = 0; }
            cur.push(ch); col += cw;
        }
        rows.push(cur);
    }
    rows
}

fn cursor_pos(input: &str, cursor: usize, width: usize) -> (usize, usize) {
    let mut row = 0; let mut col = 0;
    for (i, ch) in input.chars().enumerate() {
        if i == cursor { break; }
        if ch == '\n' { row += 1; col = 0; continue; }
        let cw = UnicodeWidthChar::width(ch).unwrap_or(1);
        if col + cw > width { row += 1; col = 0; }
        col += cw;
    }
    (row, col)
}

fn truncate(s: &str, n: usize) -> String { if s.chars().count() <= n { s.to_string() } else { format!("{}…", s.chars().take(n.saturating_sub(1)).collect::<String>()) } }
fn fmt_k(n: u64) -> String { if n < 1000 { n.to_string() } else if n < 100_000 { format!("{:.1}k", n as f64 / 1000.0) } else { format!("{}k", n / 1000) } }
fn short_path(p: &std::path::Path) -> String { let s = p.display().to_string(); if let Ok(h) = std::env::var("HOME") { if let Some(r) = s.strip_prefix(&h) { return format!("~{r}"); } } s }
