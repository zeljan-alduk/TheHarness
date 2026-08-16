//! Interactive terminal UI — the Claude-Code-style front end for a local model.
//! Everything here is presentation; the agent loop lives in the `harness` library.

use anyhow::Result;
use crossterm::event::{Event as CEvent, EventStream, KeyCode, KeyEventKind, KeyModifiers};
use futures_util::StreamExt;
use harness::agent::{system_prompt, Agent};
use harness::config::Config;
use harness::events::{Event, Sink};
use harness::llm::{Client, Content, Message};
use harness::tools::{Registry, ToolCtx};
use ratatui::prelude::*;
use ratatui::widgets::{Axis, Chart, Dataset, Gauge, GraphType, Paragraph, Sparkline};
use ratatui_image::{picker::Picker, protocol::StatefulProtocol, StatefulImage};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use unicode_width::UnicodeWidthChar;

// ───────────────────────── palette ─────────────────────────
const ORANGE: Color = Color::Rgb(255, 140, 40);
const DIM: Color = Color::Rgb(128, 136, 152);
const OK: Color = Color::Rgb(76, 195, 138);
const ERR: Color = Color::Rgb(255, 107, 107);
const THINK: Color = Color::Rgb(167, 139, 250);
const BLUE: Color = Color::Rgb(78, 161, 255);
const PINK: Color = Color::Rgb(255, 110, 130);
const CYAN: Color = Color::Rgb(90, 205, 220);
const SPINNER: [&str; 10] = ["✻", "✼", "✽", "✾", "✿", "❀", "✿", "✾", "✽", "✼"];
const WORDS: [&str; 12] = ["Thinking", "Pondering", "Working", "Reasoning", "Cooking", "Tinkering", "Brewing", "Mulling", "Crunching", "Percolating", "Noodling", "Computing"];

enum Msg { Ev(Event), Done(Result<String, String>), Sys(SysSample), CtxLen(u64), Pasted(Result<PathBuf, String>) }

/// An image attached to the next prompt.
struct Attachment { path: PathBuf, mime: String, b64: String, dims: (u32, u32), img: image::DynamicImage }

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
    Ok(Attachment { path: path.to_path_buf(), mime: mime.into(), b64: base64::engine::general_purpose::STANDARD.encode(&bytes), dims, img })
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

/// macOS: dump a clipboard image (PNG) to a temp file via AppleScript. Returns Err if no image on the clipboard.
async fn clipboard_image() -> Result<PathBuf, String> {
    let out = std::env::temp_dir().join(format!("harness-paste-{}.png", std::process::id() as u64 * 1000 + (Instant::now().elapsed().as_millis() as u64 % 1000)));
    let script = format!("set f to open for access POSIX file \"{}\" with write permission\nset eof of f to 0\nwrite (the clipboard as «class PNGf») to f\nclose access f", out.display());
    let o = tokio::process::Command::new("osascript").arg("-e").arg(&script).output().await.map_err(|e| e.to_string())?;
    if !o.status.success() { let _ = std::fs::remove_file(&out); return Err("no image on the clipboard (copy an image, then ctrl+v; or type/drag an image path)".into()); }
    Ok(out)
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

/// Ask LM Studio for the loaded context length (best effort; falls back to the config budget).
async fn fetch_ctx_len(base_url: String, model: String, tx: mpsc::UnboundedSender<Msg>) {
    let root = base_url.trim_end_matches("/v1").trim_end_matches('/').to_string();
    let Ok(http) = reqwest::Client::builder().timeout(Duration::from_secs(3)).build() else { return };
    if let Ok(r) = http.get(format!("{root}/api/v0/models")).send().await {
        if let Ok(v) = r.json::<serde_json::Value>().await {
            for m in v["data"].as_array().cloned().unwrap_or_default() {
                if m["id"].as_str() == Some(model.as_str()) {
                    if let Some(n) = m["loaded_context_length"].as_u64().or_else(|| m["max_context_length"].as_u64()) { let _ = tx.send(Msg::CtxLen(n)); }
                }
            }
        }
    }
}

struct TuiSink(mpsc::UnboundedSender<Msg>);
impl Sink for TuiSink { fn emit(&self, e: &Event) { let _ = self.0.send(Msg::Ev(e.clone())); } }

enum Block {
    Banner(Vec<String>),
    User(String, Vec<String>),
    Assistant { text: String, streaming: bool },
    Reasoning { text: String, streaming: bool },
    Tool { id: String, name: String, args: String, result: Option<String>, secs: f64, images: usize, interrupted: bool },
    System(String),
    Error(String),
    Finished(String),
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
}

pub async fn run(cfg: Config) -> Result<()> {
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
    };
    app.metrics.ctx_len = app.cfg.llm.context_budget_tokens;
    app.banner();
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
    let _ = crossterm::execute!(std::io::stdout(), crossterm::event::EnableBracketedPaste);
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
    let _ = crossterm::execute!(std::io::stdout(), crossterm::event::DisableBracketedPaste);
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
            String::new(),
            "  /help for commands · esc interrupts · ctrl+o expands tool output · ctrl+t shows thinking".into(),
        ]));
    }

    fn set_status(&mut self, s: impl Into<String>) { self.status_msg = Some((s.into(), Instant::now())); }
    fn panel_visible(&self, width: u16) -> bool { self.panel.unwrap_or(width >= 120) }

    fn on_term(&mut self, ev: CEvent) {
        match ev {
            CEvent::Paste(s) => { self.insert_str(&s.replace("\r\n", "\n").replace('\r', "\n")); }
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
                    (KeyCode::Char('v'), true, _) => { let tx = self.tx.clone(); self.set_status("reading clipboard image…"); tokio::spawn(async move { let _ = tx.send(Msg::Pasted(clipboard_image().await)); }); }
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
                lines.push("Keys: enter send · alt+enter/ctrl+j newline · esc interrupt · ctrl+c clear/exit · ctrl+o expand tools · ctrl+t thinking · ctrl+p panel · ctrl+v paste image · pgup/pgdn scroll · ↑/↓ history".into());
                lines.push("Images: ctrl+v pastes from the clipboard; typing or dragging an image path attaches it. Previews render as a color mosaic; the model sees the full image.".into());
                self.blocks.push(Block::Banner(lines));
            }
            "/clear" | "/new" => {
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
                } else { self.model = arg.clone(); self.blocks.push(Block::System(format!("model → {arg}"))); }
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
                let r = Registry::defaults(self.net);
                self.blocks.push(Block::Banner(std::iter::once("Tools".to_string()).chain(r.defs().into_iter().map(|d| format!("  {:<14} {}", d.function.name, truncate(&d.function.description, 90)))).collect()));
            }
            "/thinking" => { self.show_thinking = !self.show_thinking; self.blocks.push(Block::System(format!("thinking {}", if self.show_thinking { "shown" } else { "hidden" }))); }
            "/expand" => { self.expand_tools = !self.expand_tools; }
            "/panel" => { self.panel = Some(!self.panel_visible(200)); }
            "/cost" | "/stats" => self.blocks.push(Block::System(format!("session tokens: {} prompt + {} completion · last context {} · turns in history {}", self.total_prompt, self.total_completion, self.last_prompt_tokens, self.history.len()))),
            "/config" => self.blocks.push(Block::Banner(vec![format!("server  {}", self.cfg.llm.base_url), format!("model   {}", self.model), format!("ctx budget {} tokens · max_turns {} · tool timeout {}s", self.cfg.llm.context_budget_tokens, self.cfg.agent.max_turns, self.cfg.agent.tool_timeout_secs), format!("net {} · segments {}", self.net, self.cfg.net.download_segments)])),
            "/exit" | "/quit" | "/q" => self.quit = true,
            _ => self.blocks.push(Block::Error(format!("unknown command {cmd} — /help"))),
        }
    }

    fn start_run(&mut self, text: String) {
        self.harvest_image_paths(&text);
        let atts: Vec<Attachment> = std::mem::take(&mut self.attachments);
        let keys: Vec<String> = atts.iter().map(|a| self.register_image(a.img.clone())).collect();
        self.blocks.push(Block::User(text.clone(), keys));
        let user_msg = if atts.is_empty() { Message::user(text.clone()) } else {
            let mut parts = vec![Content::text_part(&text)];
            for (i, a) in atts.iter().enumerate() { parts.push(Content::text_part(&format!("[image #{}: {} {}×{}]", i + 1, a.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(), a.dims.0, a.dims.1))); parts.push(Content::image_part(&a.mime, &a.b64)); }
            Message::user_parts(parts)
        };
        self.scroll_up = 0;
        self.turn_tokens = 0;
        let tx = self.tx.clone(); let session = self.session.clone();
        let mut cfg = self.cfg.clone(); cfg.llm.model = self.model.clone(); cfg.net.enabled = self.net;
        let workdir = self.workdir.clone();
        self.run_started = Instant::now();
        self.metrics.turn_start = Some(Instant::now()); self.metrics.live_peak = 0.0; self.metrics.live_chars.clear();
        let handle = tokio::spawn(async move {
            let res: Result<String, String> = async {
                let client = Client::new(&cfg.llm).map_err(|e| e.to_string())?;
                let ctx = ToolCtx { workdir: workdir.clone(), timeout: Duration::from_secs(cfg.agent.tool_timeout_secs), max_output: cfg.agent.max_tool_output_chars, net: cfg.net.clone() };
                let registry = Registry::defaults(cfg.net.enabled);
                let sink = TuiSink(tx.clone());
                let agent = Agent { client: &client, registry: &registry, ctx: &ctx, max_turns: cfg.agent.max_turns, context_budget: cfg.llm.context_budget_tokens, sink: &sink, stream: true };
                let system = system_prompt(&workdir.display().to_string(), &registry.names(), Some("You are in an interactive session: the user can see everything and will reply; keep final answers concise."));
                let mut msgs = session.lock().await;
                agent.run_turn_message(&mut msgs, &system, user_msg).await.map(|(t, _)| t).map_err(|e| format!("{e:#}"))
            }.await;
            let _ = tx.send(Msg::Done(res));
        });
        self.running = Some(handle);
    }

    fn interrupt(&mut self) {
        if let Some(h) = self.running.take() {
            h.abort();
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
            Msg::Ev(e) => self.on_event(e),
            Msg::Sys(s) => self.metrics.on_sys(s),
            Msg::CtxLen(n) => { self.metrics.ctx_len = n; }
            Msg::Pasted(Ok(p)) => self.attach(&p),
            Msg::Pasted(Err(e)) => self.set_status(e),
            Msg::Done(res) => {
                self.running = None;
                if let Err(e) = res { if !e.contains("interrupted") { self.blocks.push(Block::Error(e)); } }
                if !self.queued.is_empty() { let next = self.queued.remove(0); self.start_run(next); }
            }
        }
    }

    fn on_event(&mut self, e: Event) {
        match e {
            Event::RunStarted { model, workdir, .. } if workdir == "\u{0}models" => { self.models = model.split('\u{1f}').map(String::from).collect(); }
            Event::RunStarted { .. } | Event::Turn { .. } => {}
            Event::ModelResponse { prompt_tokens, completion_tokens, ttft_secs, secs, .. } => { self.metrics.on_call(prompt_tokens, completion_tokens, ttft_secs, secs); self.last_prompt_tokens = prompt_tokens; }
            Event::ReasoningDelta { text } => {
                self.metrics.on_delta(text.chars().count());
                if let Some(Block::Reasoning { text: t, streaming: true }) = self.blocks.last_mut() { t.push_str(&text); }
                else { self.blocks.push(Block::Reasoning { text, streaming: true }); }
            }
            Event::Reasoning { text } => {
                if let Some(Block::Reasoning { text: t, streaming }) = self.blocks.last_mut() { *t = text; *streaming = false; }
                else if !text.trim().is_empty() { self.blocks.push(Block::Reasoning { text, streaming: false }); }
            }
            Event::AssistantDelta { text } => {
                self.metrics.on_delta(text.chars().count());
                if let Some(Block::Assistant { text: t, streaming: true }) = self.blocks.last_mut() { t.push_str(&text); }
                else { self.finish_streaming(); self.blocks.push(Block::Assistant { text, streaming: true }); }
            }
            Event::Assistant { text } => {
                if let Some(Block::Assistant { text: t, streaming }) = self.blocks.last_mut() { *t = text; *streaming = false; }
                else if !text.trim().is_empty() { self.blocks.push(Block::Assistant { text, streaming: false }); }
            }
            Event::ToolCall { id, name, args } => { self.finish_streaming(); self.blocks.push(Block::Tool { id, name, args, result: None, secs: 0.0, images: 0, interrupted: false }); }
            Event::ToolResult { id, result, secs, images, .. } => {
                if !images.is_empty() {
                    use base64::Engine;
                    let mut keys = Vec::new();
                    for du in &images { if let Some(b64) = du.split(',').nth(1) { if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64) { if let Ok(img) = image::load_from_memory(&bytes) { keys.push(self.register_image(img)); } } } }
                    self.tool_previews.insert(id.clone(), keys);
                }
                if let Some(Block::Tool { result: r, secs: s, images: im, .. }) = self.blocks.iter_mut().rev().find(|b| matches!(b, Block::Tool { id: i, .. } if *i == id)) { *r = Some(result); *s = secs; *im = images.len(); }
            }
            Event::Compacted { count, prompt_tokens } => self.blocks.push(Block::System(format!("compacted {count} old tool results (context was {prompt_tokens} tokens)"))),
            Event::RunFinished { stop_reason, turns, tool_calls, prompt_tokens, completion_tokens, wall_secs } => {
                self.finish_streaming();
                self.total_prompt += prompt_tokens; self.total_completion += completion_tokens; self.turn_tokens = completion_tokens;
                let s = format!("{} · {} model call{} · {} tool call{} · {}+{} tokens · {:.0}s", if stop_reason == "done" { "done" } else { &stop_reason }, turns, if turns == 1 { "" } else { "s" }, tool_calls, if tool_calls == 1 { "" } else { "s" }, fmt_k(prompt_tokens), fmt_k(completion_tokens), wall_secs);
                self.blocks.push(Block::Finished(s));
            }
            Event::Error { message } => { self.finish_streaming(); self.blocks.push(Block::Error(message)); }
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
    ("/model", "show or switch the model: /model <name>"),
    ("/cd", "change working directory"),
    ("/pwd", "print working directory"),
    ("/tools", "list the tools the model can call"),
    ("/net", "internet tools on|off"),
    ("/thinking", "toggle showing the model's reasoning"),
    ("/expand", "toggle expanded tool output (ctrl+o)"),
    ("/panel", "toggle the dashboard panel (ctrl+p)"),
    ("/cost", "token usage for this session"),
    ("/config", "effective configuration"),
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
        let divider: Vec<Line> = (0..div.height).map(|_| Line::from(Span::styled("│", Style::default().fg(DIM)))).collect();
        f.render_widget(Paragraph::new(divider), div);
        draw_panel(f, app, pa);
    }
    let width = area.width as usize;
    // input geometry
    let input_lines = wrap_input(&app.input, width.saturating_sub(2).max(1));
    let sugg = suggestions(&app.input);
    let input_h = (input_lines.len().clamp(1, 8) + sugg.len() + if app.attachments.is_empty() { 0 } else { 1 }) as u16;
    // notice line above the box: spinner while running, or a transient status message
    let notice: Option<Vec<Span>> = if app.running.is_some() {
        let sp = SPINNER[(app.tick as usize / 2) % SPINNER.len()];
        let el = app.run_started.elapsed().as_secs();
        let live = app.metrics.live_tps();
        Some(vec![Span::styled(format!("{sp} {}… ", WORDS[app.word]), Style::default().fg(ORANGE)),
                  Span::styled(format!("({el}s · {} tok/s · esc to interrupt{})", if live > 0.0 { format!("{live:.0}") } else { "–".into() }, if app.queued.is_empty() { String::new() } else { format!(" · {} queued", app.queued.len()) }), Style::default().fg(DIM))])
    } else if let Some((m, t)) = &app.status_msg { if t.elapsed() < Duration::from_secs(4) { Some(vec![Span::styled(format!("· {m}"), Style::default().fg(ORANGE))]) } else { None } } else { None };
    let notice_h = if notice.is_some() { 1 } else { 0 };
    let chunks = Layout::vertical([Constraint::Min(1), Constraint::Length(notice_h), Constraint::Length(1), Constraint::Length(input_h), Constraint::Length(1), Constraint::Length(1)]).split(area);
    let (tr_area, no_area, top_area, in_area, bot_area, st_area) = (chunks[0], chunks[1], chunks[2], chunks[3], chunks[4], chunks[5]);

    // transcript
    let mut lines: Vec<Line> = Vec::new();
    let mut ph: Vec<Placeholder> = Vec::new();
    for b in &app.blocks { render_block(b, app, width, &mut lines, &mut ph); }
    let total = lines.len();
    let h = tr_area.height as usize;
    let max_up = total.saturating_sub(h);
    if app.scroll_up > max_up { app.scroll_up = max_up; }
    let start = max_up - app.scroll_up;
    let visible: Vec<Line> = lines.into_iter().skip(start).take(h).collect();
    f.render_widget(Paragraph::new(visible), tr_area);
    // images: draw those whose slot is fully inside the visible window
    for p in ph {
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
        f.render_widget(Paragraph::new(Span::styled(tag, Style::default().fg(Color::Black).bg(ORANGE))), r);
    }
    if let Some(n) = notice { f.render_widget(Paragraph::new(Line::from(n)), no_area); }

    // input box: rule / › text / rule
    let rule = Line::from(Span::styled("─".repeat(width), Style::default().fg(DIM)));
    f.render_widget(Paragraph::new(rule.clone()), top_area);
    let mut in_lines: Vec<Line> = Vec::new();
    for (i, l) in input_lines.iter().enumerate().take(8) {
        let prompt = if i == 0 { Span::styled("› ", Style::default().fg(Color::White).bold()) } else { Span::raw("  ") };
        in_lines.push(Line::from(vec![prompt, Span::raw(l.clone())]));
    }
    if app.input.is_empty() {
        in_lines[0] = Line::from(vec![Span::styled("› ", Style::default().fg(Color::White).bold()), Span::styled(if app.running.is_some() { "type to queue the next message…" } else { "Ask the agent to do something… (/help)" }, Style::default().fg(DIM))]);
    }
    for (c, d) in &sugg { in_lines.push(Line::from(vec![Span::raw("  "), Span::styled(format!("{c:<12}"), Style::default().fg(BLUE)), Span::styled(d.to_string(), Style::default().fg(DIM))])); }
    if !app.attachments.is_empty() {
        let mut spans = vec![Span::styled("  📎 ", Style::default().fg(BLUE))];
        for (i, a) in app.attachments.iter().enumerate() { spans.push(Span::styled(format!("#{} {} {}×{}  ", i + 1, a.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(), a.dims.0, a.dims.1), Style::default().fg(BLUE))); }
        in_lines.push(Line::from(spans));
    }
    f.render_widget(Paragraph::new(in_lines), in_area);
    f.render_widget(Paragraph::new(rule), bot_area);
    // cursor
    let (crow, ccol) = cursor_pos(&app.input, app.cursor, width.saturating_sub(2).max(1));
    if crow < 8 { f.set_cursor_position((in_area.x + 2 + ccol as u16, in_area.y + crow as u16)); }

    // mode line: ▶▶ bypass permissions on · model · cwd · ctx
    let dot = || Span::styled(" · ", Style::default().fg(DIM));
    let mut st = vec![Span::styled("  ▶▶ bypass permissions on", Style::default().fg(PINK)), dot(),
        Span::styled(app.model.clone(), Style::default().fg(CYAN)), dot(),
        Span::styled(short_path(&app.workdir), Style::default().fg(CYAN)), dot(),
        Span::styled(format!("ctx {}", fmt_k(app.last_prompt_tokens)), Style::default().fg(CYAN))];
    if !app.net { st.push(dot()); st.push(Span::styled("offline", Style::default().fg(PINK))); }
    if !app.queued.is_empty() { st.push(dot()); st.push(Span::styled(format!("{} queued", app.queued.len()), Style::default().fg(CYAN))); }
    let lw: usize = st.iter().map(|s| s.content.chars().count()).sum();
    let right = if app.running.is_none() { "? for shortcuts · /help" } else { "esc to interrupt" };
    let pad = width.saturating_sub(lw + right.chars().count() + 1);
    st.push(Span::raw(" ".repeat(pad))); st.push(Span::styled(right, Style::default().fg(DIM)));
    f.render_widget(Paragraph::new(Line::from(st)), st_area);
}

// ───────────────────────── dashboard panel ─────────────────────────
fn draw_panel(f: &mut Frame, app: &App, area: Rect) {
    let m = &app.metrics;
    let title = |t: &str| Line::from(vec![Span::styled(format!("── {t} "), Style::default().fg(ORANGE).bold()), Span::styled("─".repeat((area.width as usize).saturating_sub(t.len() + 4)), Style::default().fg(DIM))]);
    let dim = Style::default().fg(DIM);
    let running = app.running.is_some();
    let rows = Layout::vertical([
        Constraint::Length(1), Constraint::Min(6),          // thinking
        Constraint::Length(1), Constraint::Length(6),       // tokens
        Constraint::Length(1), Constraint::Length(8),       // speed
        Constraint::Length(1), Constraint::Length(9),       // system
    ]).split(area);

    // ── Thinking ──
    f.render_widget(Paragraph::new(title(if running { "Thinking · live" } else { "Thinking · last" })), rows[0]);
    let think = app.blocks.iter().rev().find_map(|b| if let Block::Reasoning { text, .. } = b { Some(text.clone()) } else { None }).unwrap_or_default();
    let tw = rows[1].width as usize;
    let mut tl: Vec<Line> = Vec::new();
    for l in think.lines().filter(|l| !l.trim().is_empty()) { push_wrapped(&mut tl, vec![Span::styled(l.trim().to_string(), Style::default().fg(THINK))], tw, 0); }
    let th = rows[1].height as usize;
    let skip = tl.len().saturating_sub(th);
    let tail: Vec<Line> = tl.into_iter().skip(skip).collect();
    if tail.is_empty() { f.render_widget(Paragraph::new(Span::styled("(reasoning will stream here)", dim)), rows[1]); } else { f.render_widget(Paragraph::new(tail), rows[1]); }

    // ── Tokens ──
    f.render_widget(Paragraph::new(title("Tokens")), rows[2]);
    let tk = Layout::vertical([Constraint::Length(1), Constraint::Length(1), Constraint::Length(1), Constraint::Length(3)]).split(rows[3]);
    let ctx = m.ctx_len.max(1);
    let ratio = (app.last_prompt_tokens as f64 / ctx as f64).clamp(0.0, 1.0);
    let gcolor = if ratio > 0.85 { ERR } else if ratio > 0.6 { ORANGE } else { OK };
    f.render_widget(Gauge::default().gauge_style(Style::default().fg(gcolor).bg(Color::Rgb(38, 44, 56))).ratio(ratio).label(format!("context {} / {} ({:.0}%)", fmt_k(app.last_prompt_tokens), fmt_k(ctx), ratio * 100.0)), tk[0]);
    f.render_widget(Paragraph::new(Line::from(vec![Span::styled("session ", dim), Span::raw(format!("{} in · {} out", fmt_k(app.total_prompt), fmt_k(app.total_completion))), Span::styled(format!(" · {} calls", m.calls), dim)])), tk[1]);
    let (lp, lc) = m.last_call.map(|(p, c, _, _)| (p, c)).unwrap_or((0, 0));
    f.render_widget(Paragraph::new(Line::from(vec![Span::styled("last call ", dim), Span::raw(format!("{} in · {} out", fmt_k(lp), fmt_k(lc))), Span::styled("   out/call ▸", dim)])), tk[2]);
    f.render_widget(Sparkline::default().data(&m.completion_per_call.iter().cloned().collect::<Vec<_>>()).style(Style::default().fg(BLUE)), tk[3]);

    // ── Speed ──
    f.render_widget(Paragraph::new(title("Speed")), rows[4]);
    let sp = Layout::vertical([Constraint::Length(1), Constraint::Length(1), Constraint::Length(1), Constraint::Min(3)]).split(rows[5]);
    let live = if running { m.live_tps() } else { 0.0 };
    let (ttft, gen, psp) = m.last_call.map(|(p, c, t, s)| (t, if s > t && c > 0 { c as f64 / (s - t) } else { 0.0 }, if t > 0.0 { p as f64 / t } else { 0.0 })).unwrap_or((0.0, 0.0, 0.0));
    f.render_widget(Paragraph::new(Line::from(vec![Span::styled("live ", dim), Span::styled(format!("{live:>5.1} tok/s"), Style::default().fg(if running { ORANGE } else { DIM }).bold()), Span::styled(format!("  peak {:.1}", m.live_peak), dim)])), sp[0]);
    f.render_widget(Paragraph::new(Line::from(vec![Span::styled("gen  ", dim), Span::raw(format!("{gen:>5.1} tok/s")), Span::styled("  ttft ", dim), Span::raw(format!("{:.2}s", ttft))])), sp[1]);
    f.render_widget(Paragraph::new(Line::from(vec![Span::styled("prompt ", dim), Span::raw(format!("{psp:>6.0} tok/s")), Span::styled(format!("  turn {}s", m.turn_start.map(|t| t.elapsed().as_secs()).filter(|_| running).unwrap_or(0)), dim)])), sp[2]);
    // chart: gen tok/s per call
    let pts: Vec<(f64, f64)> = m.gen_speed.iter().enumerate().map(|(i, v)| (i as f64, *v as f64)).collect();
    if pts.len() >= 2 {
        let ymax = pts.iter().map(|p| p.1).fold(1.0, f64::max) * 1.15;
        let ds = Dataset::default().name("tok/s").marker(symbols::Marker::Braille).graph_type(GraphType::Line).style(Style::default().fg(OK)).data(&pts);
        let chart = Chart::new(vec![ds])
            .x_axis(Axis::default().bounds([0.0, (pts.len() - 1) as f64]).style(dim))
            .y_axis(Axis::default().bounds([0.0, ymax]).labels(vec![Span::styled("0", dim), Span::styled(format!("{:.0}", ymax), dim)]).style(dim));
        f.render_widget(chart, sp[3]);
    } else {
        f.render_widget(Paragraph::new(Span::styled("(gen tok/s per call chart after 2 calls)", dim)), sp[3]);
    }

    // ── System ──
    f.render_widget(Paragraph::new(title("System")), rows[6]);
    let sy = Layout::vertical([Constraint::Length(1), Constraint::Length(2), Constraint::Length(1), Constraint::Length(2), Constraint::Length(1), Constraint::Length(2)]).split(rows[7]);
    let last = &m.last;
    let cpu = last.cpu;
    f.render_widget(Paragraph::new(Line::from(vec![Span::styled("cpu ", dim), Span::styled(format!("{cpu:>5.1}%"), Style::default().fg(if cpu > 80.0 { ERR } else { Color::White })), Span::styled(format!("   harness rss {}", fmt_bytes(last.harness_rss)), dim)])), sy[0]);
    f.render_widget(Sparkline::default().data(&m.cpu.iter().cloned().collect::<Vec<_>>()).max(100).style(Style::default().fg(BLUE)), sy[1]);
    match (last.gpu_util, last.gpu_mem) {
        (Some(g), gm) => {
            f.render_widget(Paragraph::new(Line::from(vec![Span::styled("gpu ", dim), Span::styled(format!("{g:>5.0}%"), Style::default().fg(if g > 80.0 { ORANGE } else { Color::White })), Span::styled(format!("   gpu mem {}", gm.map(fmt_bytes).unwrap_or_else(|| "?".into())), dim)])), sy[2]);
            f.render_widget(Sparkline::default().data(&m.gpu.iter().cloned().collect::<Vec<_>>()).max(100).style(Style::default().fg(THINK)), sy[3]);
        }
        _ => { f.render_widget(Paragraph::new(Span::styled("gpu  n/a on this platform", dim)), sy[2]); }
    }
    let mr = if last.mem_total > 0 { last.mem_used as f64 / last.mem_total as f64 } else { 0.0 };
    f.render_widget(Paragraph::new(Line::from(vec![Span::styled("ram ", dim), Span::raw(format!("{} / {}", fmt_bytes(last.mem_used), fmt_bytes(last.mem_total))), Span::styled(format!("   server rss {}", fmt_bytes(last.server_rss)), dim)])), sy[4]);
    f.render_widget(Gauge::default().gauge_style(Style::default().fg(if mr > 0.9 { ERR } else { BLUE }).bg(Color::Rgb(38, 44, 56))).ratio(mr.clamp(0.0, 1.0)).label(format!("{:.0}%", mr * 100.0)), Rect { height: 1, ..sy[5] });
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
            let bs = Style::default().fg(ORANGE);
            out.push(Line::from(Span::styled(format!("╭{}╮", "─".repeat(inner + 2)), bs)));
            for l in ls { let t = truncate(l, inner); out.push(Line::from(vec![Span::styled("│ ", bs), Span::raw(format!("{:<inner$}", t)), Span::styled(" │", bs)])); }
            out.push(Line::from(Span::styled(format!("╰{}╯", "─".repeat(inner + 2)), bs)));
            out.push(Line::raw(""));
        }
        Block::User(t, imgs) => {
            out.push(Line::raw(""));
            for (i, l) in t.lines().enumerate() { push_wrapped(out, vec![Span::styled(if i == 0 { "› " } else { "  " }, Style::default().fg(DIM)), Span::styled(l.to_string(), Style::default().bold())], w, 2); }
            for k in imgs { image_slot(app, k, (w.saturating_sub(4)).min(60) as u16, 12, 2, out, ph); out.push(Line::raw("")); }
            out.push(Line::raw(""));
        }
        Block::Assistant { text, streaming } => {
            let mut first = true;
            for l in text.lines() {
                let bullet = if first { Span::styled("⏺ ", Style::default().fg(Color::White)) } else { Span::raw("  ") };
                push_wrapped(out, vec![bullet, Span::raw(l.to_string())], w, 2);
                first = false;
            }
            if *streaming { if let Some(last) = out.last_mut() { last.spans.push(Span::styled("▍", Style::default().fg(ORANGE))); } }
            if text.is_empty() && *streaming { out.push(Line::from(vec![Span::styled("⏺ ", Style::default().fg(Color::White)), Span::styled("▍", Style::default().fg(ORANGE))])); }
            out.push(Line::raw(""));
        }
        Block::Reasoning { text, streaming } => {
            let st = Style::default().fg(THINK).italic();
            if app.show_thinking {
                let mut first = true;
                for l in text.lines().filter(|l| !l.trim().is_empty()) { push_wrapped(out, vec![Span::styled(if first { "✻ " } else { "  " }, st), Span::styled(l.to_string(), st)], w, 2); first = false; }
                if *streaming { if let Some(last) = out.last_mut() { last.spans.push(Span::styled("▍", st)); } }
            } else {
                let firstline = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string();
                let n = text.chars().count();
                let lbl = if *streaming { format!("✻ Thinking… {}", truncate(&firstline, w.saturating_sub(40))) } else { format!("✻ Thought for {} chars: {}", n, truncate(&firstline, w.saturating_sub(40))) };
                push_wrapped(out, vec![Span::styled(lbl, st), Span::styled("  (ctrl+t)", Style::default().fg(DIM))], w, 2);
            }
            out.push(Line::raw(""));
        }
        Block::Tool { id, name, args, result, secs, images, interrupted } => {
            let (bullet_style, done) = match (result, interrupted) {
                (Some(r), _) if r.starts_with("error:") => (Style::default().fg(ERR), true),
                (Some(_), _) => (Style::default().fg(OK), true),
                (None, true) => (Style::default().fg(ERR), true),
                (None, false) => (Style::default().fg(if (app.tick / 4) % 2 == 0 { ORANGE } else { DIM }), false),
            };
            let summary = args_summary(name, args, w.saturating_sub(name.len() + 6));
            push_wrapped(out, vec![Span::styled("⏺ ", bullet_style), Span::styled(name.clone(), Style::default().bold()), Span::styled(format!("({summary})"), Style::default().fg(DIM))], w, 2);
            match result {
                None if *interrupted => out.push(Line::from(vec![Span::styled("  ⎿  ", Style::default().fg(DIM)), Span::styled("interrupted", Style::default().fg(ERR))])),
                None => out.push(Line::from(vec![Span::styled("  ⎿  ", Style::default().fg(DIM)), Span::styled("running…", Style::default().fg(DIM))])),
                Some(r) => {
                    let is_err = r.starts_with("error:");
                    let lines: Vec<&str> = r.lines().collect();
                    let show = if app.expand_tools { lines.len().min(60) } else { 1 };
                    for (i, l) in lines.iter().take(show).enumerate() {
                        let pre = if i == 0 { "  ⎿  " } else { "     " };
                        let mut spans = vec![Span::styled(pre, Style::default().fg(DIM)), Span::styled(l.to_string(), Style::default().fg(if is_err { ERR } else { DIM }))];
                        if i == show - 1 && lines.len() > show { spans.push(Span::styled(format!("  … +{} lines (ctrl+o)", lines.len() - show), Style::default().fg(DIM).italic())); }
                        push_wrapped(out, spans, w, 5);
                    }
                    if lines.is_empty() { out.push(Line::from(vec![Span::styled("  ⎿  ", Style::default().fg(DIM)), Span::styled("(no output)", Style::default().fg(DIM))])); }
                    if *images > 0 {
                        out.push(Line::from(vec![Span::styled("     ", Style::default()), Span::styled(format!("[{} image{} shown to the model]", images, if *images == 1 { "" } else { "s" }), Style::default().fg(BLUE))]));
                        if let Some(keys) = app.tool_previews.get(id) { for k in keys { image_slot(app, k, (w.saturating_sub(7)).min(60) as u16, 14, 5, out, ph); } }
                    }
                    let _ = (done, secs);
                }
            }
        }
        Block::System(t) => { push_wrapped(out, vec![Span::styled("· ", Style::default().fg(DIM)), Span::styled(t.clone(), Style::default().fg(DIM))], w, 2); }
        Block::Error(t) => { for (i, l) in t.lines().enumerate() { push_wrapped(out, vec![Span::styled(if i == 0 { "✗ " } else { "  " }, Style::default().fg(ERR)), Span::styled(l.to_string(), Style::default().fg(ERR))], w, 2); } out.push(Line::raw("")); }
        Block::Finished(t) => { push_wrapped(out, vec![Span::styled("  ✓ ", Style::default().fg(OK)), Span::styled(t.clone(), Style::default().fg(DIM))], w, 4); }
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
