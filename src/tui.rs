//! Interactive terminal UI — the Claude-Code-style front end for a local model.
//! Everything here is presentation; the agent loop lives in the `harness` library.

use anyhow::{Context, Result};
use crossterm::event::{Event as CEvent, EventStream, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};
use futures_util::StreamExt;
use harness::agent::Agent;
fn config_dir() -> PathBuf { harness::setup::config_dir() }
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

// ───────────────────────── syntax highlighting ─────────────────────────
struct Highlighter { ss: syntect::parsing::SyntaxSet, dark: syntect::highlighting::Theme, light: syntect::highlighting::Theme }
static HL: std::sync::OnceLock<Highlighter> = std::sync::OnceLock::new();
fn hl() -> &'static Highlighter {
    HL.get_or_init(|| {
        let ts = syntect::highlighting::ThemeSet::load_defaults();
        Highlighter { ss: syntect::parsing::SyntaxSet::load_defaults_newlines(), dark: ts.themes["base16-ocean.dark"].clone(), light: ts.themes["InspiredGitHub"].clone() }
    })
}
/// Highlight one line of code in `lang` into styled spans (falls back to plain).
fn highlight_line(lang: &str, line: &str, state: &mut Option<syntect::easy::HighlightLines<'static>>) -> Vec<Span<'static>> {
    let h = hl();
    if state.is_none() {
        let syntax = h.ss.find_syntax_by_token(lang).or_else(|| h.ss.find_syntax_by_extension(lang)).unwrap_or_else(|| h.ss.find_syntax_plain_text());
        let theme = if LIGHT.load(std::sync::atomic::Ordering::Relaxed) { &h.light } else { &h.dark };
        *state = Some(syntect::easy::HighlightLines::new(syntax, theme));
    }
    let hl_state = state.as_mut().unwrap();
    match hl_state.highlight_line(&format!("{line}\n"), &h.ss) {
        Ok(ranges) => ranges.into_iter().map(|(st, txt)| { let c = st.foreground; let mut style = Style::default().fg(Color::Rgb(c.r, c.g, c.b)); if st.font_style.contains(syntect::highlighting::FontStyle::BOLD) { style = style.bold(); } if st.font_style.contains(syntect::highlighting::FontStyle::ITALIC) { style = style.italic(); } Span::styled(txt.trim_end_matches('\n').to_string(), style) }).collect(),
        Err(_) => vec![Span::raw(line.to_string())],
    }
}

// ───────────────────────── /settings ─────────────────────────
/// (key, label, choices, help). Booleans use ["on","off"]. Read-only rows have no choices.
const SETTINGS: &[(&str, &str, &[&str], &str)] = &[
    ("ui.tool_view", "Tool calls", &["summary", "hidden", "full"], "summary = one line per burst (click to expand) · hidden = only thinking + answers · full = every call (ctrl+o cycles)"),
    ("ui.show_thinking", "Show thinking inline", &["off", "on"], "expand the model's reasoning in the transcript (ctrl+t / click)"),
    ("ui.panel", "Dashboard panel", &["auto", "on", "off"], "auto shows it when the window is ≥120 columns (ctrl+p)"),
    ("ui.theme", "Theme", &["dark", "light"], ""),
    ("ui.notify", "Notifications", &["on", "off"], "desktop notification when a long task finishes"),
    ("ui.fold_previous", "Auto-fold previous turn", &["on", "off"], "collapse the last turn's outputs when a new task starts"),
    ("ui.vim", "Vim mode", &["off", "on"], "modal editing in the prompt (/vim)"),
    ("permissions.mode", "Permission mode", &["auto", "ask", "plan", "bypass"], "default for new sessions (shift+tab cycles live)"),
    ("llm.effort", "Effort (Claude backend)", &["medium", "low", "high", "xhigh", "max"], "reasoning effort passed to Claude Code"),
    ("llm.compact_at_fraction", "Auto-compact at", &["0.75", "0.5", "0.6", "0.85", "0.9"], "fraction of the context window that triggers compaction (local/API backends)"),
    ("memory.auto_reflect", "Memory reflection", &["on", "off"], "learn durable facts into BRAIN.md after substantive runs"),
    ("security.redact_secrets", "Redact secrets", &["on", "off"], "mask API keys/tokens in tool outputs"),
    ("net.enabled", "Internet tools", &["on", "off"], "web_fetch / web_search / download_file"),
    ("agent.max_task_secs", "Max task time", &["0", "300", "900", "1800", "3600"], "0 = unlimited; the queue continues afterwards"),
    ("ui.event_log", "Event log", &["on", "off"], "~/.config/harness/logs/<date>/"),
    ("sandbox.mode", "Sandbox", &["none", "seatbelt", "bwrap"], "confine shell writes (macOS seatbelt / Linux bubblewrap)"),
    ("llm.provider", "Backend", &[], "change with /backend"),
    ("llm.model", "Model", &[], "change with /model or /backend"),
];

// ───────────────────────── custom keybindings ─────────────────────────
/// ~/.config/harness/keybindings.toml — [bindings] action = "ctrl+x" | "alt+enter" | "shift+tab" | "f5" | "esc" …
/// Actions: interrupt, next_task, toggle_panel, toggle_thinking, expand_tools, paste, cycle_permissions, newline,
/// quit, scroll_up, scroll_down, clear_line, jump_bottom, complete
#[derive(Clone, Default)]
struct Keymap { map: std::collections::HashMap<String, (KeyCode, KeyModifiers)> }
impl Keymap {
    fn load() -> Self {
        let mut m = std::collections::HashMap::new();
        let defaults = [("interrupt", "esc"), ("next_task", "ctrl+n"), ("toggle_panel", "ctrl+p"), ("toggle_thinking", "ctrl+t"), ("expand_tools", "ctrl+o"), ("paste", "ctrl+v"), ("cycle_permissions", "shift+tab"), ("newline", "ctrl+j"), ("quit", "ctrl+d"), ("scroll_up", "pageup"), ("scroll_down", "pagedown"), ("clear_line", "ctrl+u"), ("jump_bottom", "ctrl+l"), ("complete", "tab")];
        for (a, k) in defaults { if let Some(v) = parse_key(k) { m.insert(a.to_string(), v); } }
        let p = harness::setup::config_dir().join("keybindings.toml");
        if let Ok(t) = std::fs::read_to_string(&p) { if let Ok(v) = t.parse::<toml::Value>() { if let Some(b) = v.get("bindings").and_then(|b| b.as_table()) { for (a, k) in b { if let Some(ks) = k.as_str() { if let Some(v) = parse_key(ks) { m.insert(a.clone(), v); } } } } } }
        else { let _ = std::fs::create_dir_all(p.parent().unwrap()); let _ = std::fs::write(&p, "# Custom keybindings — action = \"key\". Keys: ctrl+x, alt+x, shift+tab, f1..f12, esc, enter, tab, pageup, pagedown, up, down, home, end\n[bindings]\n# next_task = \"ctrl+n\"\n# toggle_panel = \"ctrl+p\"\n"); }
        Self { map: m }
    }
    fn is(&self, action: &str, code: KeyCode, mods: KeyModifiers) -> bool {
        match self.map.get(action) { Some((c, m)) => { let norm = |k: KeyCode| match k { KeyCode::Char(ch) => KeyCode::Char(ch.to_ascii_lowercase()), o => o }; norm(*c) == norm(code) && (m.contains(KeyModifiers::CONTROL) == mods.contains(KeyModifiers::CONTROL)) && (m.contains(KeyModifiers::ALT) == mods.contains(KeyModifiers::ALT)) && (!m.contains(KeyModifiers::SHIFT) || mods.contains(KeyModifiers::SHIFT) || matches!(code, KeyCode::BackTab)) } None => false }
    }
}
fn parse_key(s: &str) -> Option<(KeyCode, KeyModifiers)> {
    let mut mods = KeyModifiers::NONE; let mut key: Option<KeyCode> = None;
    for part in s.split('+') {
        match part.trim().to_lowercase().as_str() {
            "ctrl" | "control" => mods |= KeyModifiers::CONTROL, "alt" | "opt" | "option" | "meta" => mods |= KeyModifiers::ALT, "shift" => mods |= KeyModifiers::SHIFT,
            "esc" | "escape" => key = Some(KeyCode::Esc), "enter" | "return" => key = Some(KeyCode::Enter), "tab" => key = Some(if mods.contains(KeyModifiers::SHIFT) { KeyCode::BackTab } else { KeyCode::Tab }),
            "backtab" => key = Some(KeyCode::BackTab), "pageup" | "pgup" => key = Some(KeyCode::PageUp), "pagedown" | "pgdn" => key = Some(KeyCode::PageDown), "up" => key = Some(KeyCode::Up), "down" => key = Some(KeyCode::Down), "left" => key = Some(KeyCode::Left), "right" => key = Some(KeyCode::Right), "home" => key = Some(KeyCode::Home), "end" => key = Some(KeyCode::End), "space" => key = Some(KeyCode::Char(' ')), "backspace" => key = Some(KeyCode::Backspace), "delete" | "del" => key = Some(KeyCode::Delete),
            f if f.starts_with('f') && f[1..].parse::<u8>().is_ok() => key = Some(KeyCode::F(f[1..].parse().unwrap())),
            c if c.chars().count() == 1 => key = Some(KeyCode::Char(c.chars().next().unwrap())),
            _ => return None,
        }
    }
    if matches!(key, Some(KeyCode::Tab)) && mods.contains(KeyModifiers::SHIFT) { key = Some(KeyCode::BackTab); }
    key.map(|k| (k, mods))
}

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

enum Msg { Title(String), Question(harness::permissions::Question, tokio::sync::oneshot::Sender<harness::permissions::Answer>), SubEnv(Arc<harness::agent::SubAgentEnv>), Policy(Arc<harness::permissions::Policy>), CcSession(Arc<harness::claude_code::ClaudeCodeSession>), CcSid(String), Block(Block), Ask(harness::permissions::ApprovalRequest, tokio::sync::oneshot::Sender<harness::permissions::Approval>), Ev(Event), Done(Result<(String, harness::agent::RunStats), String>), Sys(SysSample), CtxLen(u64), Pasted(Result<PathBuf, String>), Frames(Result<(PathBuf, f64, Vec<(f64, PathBuf)>), String>), Toolset(Arc<Toolset>), Catalog(Result<harness::plugins::Catalog, String>), Notice(String), Improve(harness::selfimprove::Stage) }

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
    let o = if cfg!(target_os = "macos") {
        let script = format!("set f to open for access POSIX file \"{}\" with write permission\nset eof of f to 0\nwrite (the clipboard as «class PNGf») to f\nclose access f", tmp.display());
        tokio::process::Command::new("osascript").arg("-e").arg(&script).output().await.map_err(|e| e.to_string())?
    } else if cfg!(windows) {
        let ps = format!("Add-Type -AssemblyName System.Windows.Forms; $img=[Windows.Forms.Clipboard]::GetImage(); if($img){{ $img.Save('{}', [System.Drawing.Imaging.ImageFormat]::Png); exit 0 }} else {{ exit 1 }}", tmp.display().to_string().replace('\'', "''"));
        tokio::process::Command::new("powershell").args(["-NoProfile", "-Command", &ps]).output().await.map_err(|e| e.to_string())?
    } else {
        // linux: wl-paste (wayland) or xclip (x11)
        let cmd = format!("(command -v wl-paste >/dev/null && wl-paste --type image/png > '{0}') || (command -v xclip >/dev/null && xclip -selection clipboard -t image/png -o > '{0}')", tmp.display());
        tokio::process::Command::new("sh").args(["-c", &cmd]).output().await.map_err(|e| e.to_string())?
    };
    if o.status.success() && std::fs::metadata(&tmp).map(|m| m.len() > 0).unwrap_or(false) {
        let bytes = std::fs::read(&tmp).map_err(|e| e.to_string())?; let _ = std::fs::remove_file(&tmp);
        return match &store { Some(st) => st.save_paste("png", &bytes).map_err(|e| e.to_string()), None => { std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?; Ok(tmp) } };
    }
    let _ = std::fs::remove_file(&tmp);
    // 2) a file reference (copied in Finder / Explorer)
    let o = if cfg!(target_os = "macos") { tokio::process::Command::new("osascript").arg("-e").arg("POSIX path of (the clipboard as «class furl»)").output().await.map_err(|e| e.to_string())? }
        else if cfg!(windows) { tokio::process::Command::new("powershell").args(["-NoProfile", "-Command", "Add-Type -AssemblyName System.Windows.Forms; $f=[Windows.Forms.Clipboard]::GetFileDropList(); if($f.Count -gt 0){ Write-Output $f[0]; exit 0 } else { exit 1 }"]).output().await.map_err(|e| e.to_string())? }
        else { tokio::process::Command::new("sh").args(["-c", "command -v xclip >/dev/null && xclip -selection clipboard -o | head -1 | sed 's#^file://##'"]).output().await.map_err(|e| e.to_string())? };
    if o.status.success() {
        let p = PathBuf::from(String::from_utf8_lossy(&o.stdout).trim());
        if p.exists() { return Ok(p); }
    }
    Err("clipboard has no image or file (copy an image or a file, then ctrl+v; or type/drag a path)".into())
}

/// One system sample (1 Hz) from the background sampler.
#[derive(Clone, Debug, Default)]
struct SysSample { cpu: f32, mem_used: u64, mem_total: u64, gpu_util: Option<f32>, gpu_mem: Option<u64>, server_rss: u64, harness_rss: u64, cpu_temp: Option<f32>, gpu_temp: Option<f32>, cpu_power: Option<f32>, gpu_power: Option<f32>, sys_power: Option<f32> }

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
        let (cpu_temp, gpu_temp, cpu_power, gpu_power, sys_power) = macmon_stats().await;
        let s = SysSample { cpu: sys.global_cpu_usage(), mem_used: sys.used_memory(), mem_total: sys.total_memory(), gpu_util, gpu_mem, server_rss, harness_rss, cpu_temp, gpu_temp, cpu_power, gpu_power, sys_power };
        if tx.send(Msg::Sys(s)).is_err() { return; }
        tokio::time::sleep(Duration::from_millis(1000)).await;
    }
}

/// Temperatures and power via `macmon` (brew install macmon; reads SMC sensors without root). None if absent.
async fn macmon_stats() -> (Option<f32>, Option<f32>, Option<f32>, Option<f32>, Option<f32>) {
    static AVAILABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let avail = *AVAILABLE.get_or_init(|| { let p = std::env::var("PATH").unwrap_or_default(); std::env::split_paths(&p).chain([std::path::PathBuf::from("/opt/homebrew/bin"), std::path::PathBuf::from("/usr/local/bin")]).any(|d| d.join("macmon").is_file()) });
    if !avail { return (None, None, None, None, None); }
    let out = tokio::time::timeout(Duration::from_secs(3), tokio::process::Command::new("macmon").args(["pipe", "-s", "1", "-i", "600"]).env("PATH", format!("{}:/opt/homebrew/bin:/usr/local/bin", std::env::var("PATH").unwrap_or_default())).output()).await;
    let Ok(Ok(o)) = out else { return (None, None, None, None, None) };
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(String::from_utf8_lossy(&o.stdout).lines().last().unwrap_or("").as_bytes()) else { return (None, None, None, None, None) };
    let f = |x: &serde_json::Value| x.as_f64().map(|n| n as f32);
    (f(&v["temp"]["cpu_temp_avg"]), f(&v["temp"]["gpu_temp_avg"]), f(&v["cpu_power"]), f(&v["gpu_power"]), f(&v["sys_power"]))
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
#[allow(dead_code)]
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
    async fn question(&self, q: harness::permissions::Question) -> Option<harness::permissions::Answer> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        if self.0.send(Msg::Question(q, tx)).is_err() { return None; }
        rx.await.ok()
    }
    fn interactive(&self) -> bool { true }
}

enum Block {
    Banner(Vec<String>),
    User(String, Vec<String>),
    Assistant { text: String, streaming: bool, folded: bool },
    Reasoning { text: String, streaming: bool, show: Option<bool>, started: Instant, ended: Option<Instant> },
    Tool { id: String, name: String, args: String, result: Option<String>, secs: f64, images: usize, interrupted: bool, fold: Option<bool> },
    System(String),
    Error(String),
    Finished(String),
    Memory(String),
    /// Context map before/after compaction: (label, tokens) segments.
    CompactMap { before: Vec<(String, u64)>, after: Vec<(String, u64)> },
    /// /context report: segments (label, tokens), window size, measured prompt tokens, top items, hints.
    ContextReport { segments: Vec<(String, u64)>, window: u64, measured: u64, top: Vec<String>, hints: Vec<String> },
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
    /// /restart: after quitting, exec the (possibly rebuilt) harness binary with `--resume <this session>`
    restart: bool,
    /// /improve: background self-improvement job + its cancel flag
    improve: Option<tokio::task::JoinHandle<()>>,
    improve_cancel: Arc<std::sync::atomic::AtomicBool>,
    /// Automatic restart scheduled after an improvement was installed (esc or /cancel aborts it)
    restart_at: Option<Instant>,
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
    vim: bool, vim_normal: bool,
    keymap: Keymap,
    tool_view: String,               // summary | hidden | full
    tool_groups_open: std::collections::HashSet<usize>, // first block index of an expanded tool burst
    settings_open: bool, settings_cursor: usize,
    live_policy: Option<Arc<harness::permissions::Policy>>,
    extra_roots: Vec<PathBuf>,
    /// worktree enter/exit state (shared with the running agent; persists across turns)
    wt_cwd: harness::worktree::CwdCell,
    cc: Option<Arc<harness::claude_code::ClaudeCodeSession>>,
    cc_last_session: Option<String>,
    cc_rate: Option<(String, String, u64)>,
    compact_progress: Option<(f64, String, Instant)>,
    session_meta: harness::sessions::Meta,
    todos: Arc<std::sync::Mutex<Vec<harness::tools::todo::TodoItem>>>,
    inbox: Arc<harness::inbox::Inbox>,
    event_log: Option<std::fs::File>,
    pending_ask: Option<(harness::permissions::ApprovalRequest, tokio::sync::oneshot::Sender<harness::permissions::Approval>)>,
    pending_q: Option<(harness::permissions::Question, tokio::sync::oneshot::Sender<harness::permissions::Answer>, String)>,
    subenv: Option<Arc<harness::agent::SubAgentEnv>>,
    attached: Option<usize>,
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
        quit: false, restart: false, improve: None, improve_cancel: Default::default(), restart_at: None, tick: 0, word: 0, models: vec![],
        metrics: Metrics::new(0), panel: None, attachments: vec![], tool_previews: Default::default(),
        picker, images: Default::default(), img_seq: 0,
        think_scroll: 0, toolset: None, perm_mode: harness::permissions::Mode::Auto, vim: false, vim_normal: false, keymap: Keymap::load(), tool_view: "summary".into(), tool_groups_open: Default::default(), settings_open: false, settings_cursor: 0, live_policy: None, cc_rate: None, extra_roots: vec![], wt_cwd: harness::worktree::new_cell(), cc: None, cc_last_session: None, compact_progress: None, session_meta: harness::sessions::Meta::default(), todos: Default::default(), inbox: Default::default(), event_log: None, pending_ask: None, pending_q: None, subenv: None, attached: None, video: None, strip_rects: vec![], tr_rect: Rect::default(), panel_rect: Rect::default(), tr_start: 0, line_map: vec![],
    };
    app.metrics.ctx_len = app.cfg.llm.context_budget_tokens.unwrap_or(0);
    app.perm_mode = app.cfg.permissions.mode;
    app.tool_view = app.cfg.ui.tool_view.clone(); app.show_thinking = app.cfg.ui.show_thinking; app.vim = app.cfg.ui.vim;
    app.panel = match app.cfg.ui.panel.as_str() { "on" => Some(true), "off" => Some(false), _ => None };
    if app.cfg.ui.event_log { { let d = config_dir().join("logs").join(harness::memory::today_iso()); let _ = std::fs::create_dir_all(&d); app.event_log = std::fs::OpenOptions::new().create(true).append(true).open(d.join(format!("tui-{}.jsonl", std::process::id()))).ok(); } }
    if app.cfg.ui.theme == "light" { LIGHT.store(true, std::sync::atomic::Ordering::Relaxed); }
    app.banner();
    if let Ok(p) = harness::plugins::Plugins::open() { let st = p.stale(7); if !st.is_empty() { app.blocks.push(Block::System(format!("plugins not updated for 7+ days: {} — /plugin update all", st.join(", ")))); } }
    if !harness::permissions::is_trusted(&app.workdir) && app.perm_mode != harness::permissions::Mode::Plan { app.blocks.push(Block::System(format!("first time in {} — tools run here in '{}' mode. /trust remembers this directory; /plan for read-only.", short_path(&app.workdir), app.perm_mode.label()))); }
    // after /restart (or an automatic restart following /improve): restore the previously picked backend/model/effort
    if let Ok(m) = std::env::var("HARNESS_RESTORE_MODEL") { if !m.is_empty() { app.model = m.clone(); app.cfg.llm.model = m; } }
    if let Ok(p) = std::env::var("HARNESS_RESTORE_PROVIDER") { app.cfg.llm.provider = if p.is_empty() { None } else { Some(p) }; }
    if let Ok(e) = std::env::var("HARNESS_RESTORE_EFFORT") { if !e.is_empty() { app.cfg.llm.effort = Some(e); } }
    for k in ["HARNESS_RESTORE_MODEL", "HARNESS_RESTORE_PROVIDER", "HARNESS_RESTORE_EFFORT"] { std::env::remove_var(k); }
    if let Some(r) = resume { app.resume_session(&r); }
    { let h = app.cfg.hooks.clone(); let wd = app.workdir.clone(); if !h.session_start.is_empty() { let tx = app.tx.clone(); tokio::spawn(async move { for o in harness::hooks::run_event(&h, "session_start", "", serde_json::json!({}), &wd).await { let _ = tx.send(Msg::Notice(format!("session_start hook: {}", o.trim()))); } }); } }
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
                _ = ticker.tick() => {
                    app.tick += 1; if app.tick % 30 == 0 { app.word = (app.word + 1) % WORDS.len(); }
                    // cross-session: heartbeat every ~5s, poll our mailbox every ~2s
                    if app.tick % 60 == 0 { if app.session_meta.id.is_empty() { app.session_meta.id = harness::sessions::SessionStore::new_id(); } harness::mailbox::heartbeat(&harness::mailbox::Live { id: app.session_meta.id.clone(), title: if app.session_meta.title.is_empty() { "(new session)".into() } else { app.session_meta.title.clone() }, workdir: app.workdir.display().to_string(), pid: std::process::id(), backend: app.cfg.llm.provider.clone().unwrap_or("local".into()), updated: 0, busy: app.running.is_some() }); }
                    if app.tick % 25 == 0 && !app.session_meta.id.is_empty() { for m in harness::mailbox::take(&app.session_meta.id) { app.blocks.push(Block::System(format!("✉ message from session {}: {}", m.from, truncate(&m.text, 200)))); app.inbox.push(format!("message from session {}", m.from), m.text); } }
                    // wakeups: inbox events (monitor lines, scheduled prompts, messages) start a turn when idle
                    if app.running.is_none() && app.pending_ask.is_none() && app.pending_q.is_none() && !app.inbox.is_empty() && app.tick % 12 == 0 { if let Some(m) = app.inbox.take_message() { app.set_status("inbox event → waking the agent"); app.start_run(m); } }
                    if let Some(at) = app.restart_at { if Instant::now() >= at { if app.running.is_none() && app.pending_ask.is_none() && app.pending_q.is_none() { app.restart_at = None; app.blocks.push(Block::System("↻ restarting into the improved harness — resuming this session".into())); app.restart = true; app.quit = true; } } }
                    let cap = app.cfg.agent.max_task_secs;
                    if cap > 0 && app.running.is_some() && app.run_started.elapsed().as_secs() > cap { app.blocks.push(Block::Error(format!("task exceeded max_task_secs ({cap}s) — stopping and moving on"))); app.next_task(); }
                }
                Some(msg) = rx.recv() => { app.on_msg(msg); while let Ok(m) = rx.try_recv() { app.on_msg(m); } }
                Some(ev) = events.next() => { match ev { Ok(ev) => app.on_term(ev), Err(e) => { app.blocks.push(Block::Error(format!("terminal: {e}"))); } } }
            }
            if app.quit { break; }
        }
        Ok(())
    }.await;
    let _ = crossterm::execute!(std::io::stdout(), crossterm::event::DisableMouseCapture, crossterm::event::DisableBracketedPaste);
    ratatui::restore();
    if !app.session_meta.id.is_empty() { harness::mailbox::unregister(&app.session_meta.id); }
    if !app.cfg.hooks.session_end.is_empty() { let _ = harness::hooks::run_event(&app.cfg.hooks, "session_end", "", serde_json::json!({"session": app.session_meta.id}), &app.workdir).await; }
    if let Some(h) = app.running.take() { h.abort(); }
    if app.restart && res.is_ok() { restart_process(&mut app).await?; }
    res
}

/// Replace this process with the harness binary on disk (picks up a rebuilt/installed binary),
/// resuming the current session. Original CLI args are kept, minus any previous --resume/--continue.
async fn restart_process(app: &mut App) -> Result<()> {
    // persist the transcript synchronously so the new process can resume it
    let msgs = app.session.lock().await.clone();
    let mut resume: Option<String> = None;
    if msgs.len() >= 2 {
        if app.session_meta.id.is_empty() { app.session_meta.id = harness::sessions::SessionStore::new_id(); }
        app.session_meta.workdir = app.workdir.display().to_string();
        app.session_meta.model = app.model.clone();
        app.session_meta.provider = app.cfg.llm.provider.clone(); app.session_meta.effort = app.cfg.llm.effort.clone();
        let store = harness::sessions::SessionStore::open()?;
        store.save(&mut app.session_meta, &msgs)?;
        resume = Some(app.session_meta.id.clone());
    }
    let exe = std::env::var_os("HARNESS_ORIG_EXE").map(PathBuf::from).or_else(|| std::env::current_exe().ok()).context("cannot locate the harness executable")?;
    let mut args: Vec<std::ffi::OsString> = Vec::new();
    let mut skip = false;
    for a in std::env::args_os().skip(1) {
        if skip { skip = false; continue; }
        let s = a.to_string_lossy();
        if s == "--resume" || s == "-r" { skip = true; continue; }
        if s.starts_with("--resume=") || s == "--continue" || s == "-c" || s == "chat" { continue; }
        args.push(a);
    }
    if let Some(id) = &resume { args.push("--resume".into()); args.push(id.into()); }
    eprintln!("· restarting {} {}", exe.display(), args.iter().map(|a| a.to_string_lossy().to_string()).collect::<Vec<_>>().join(" "));
    let mut cmd = std::process::Command::new(&exe);
    // the picked backend/model/effort survive the restart even when the session was too short to be saved
    cmd.args(&args).env_remove("HARNESS_SELF_EXEC").current_dir(&app.workdir)
        .env("HARNESS_RESTORE_MODEL", &app.model).env("HARNESS_RESTORE_PROVIDER", app.cfg.llm.provider.clone().unwrap_or_default()).env("HARNESS_RESTORE_EFFORT", app.cfg.llm.effort.clone().unwrap_or_default());
    #[cfg(unix)] { use std::os::unix::process::CommandExt; let err = cmd.exec(); anyhow::bail!("failed to re-exec {}: {err}", exe.display()) }
    #[cfg(not(unix))] { let st = cmd.status().with_context(|| format!("failed to run {}", exe.display()))?; std::process::exit(st.code().unwrap_or(1)); }
}

// ───────────────────────── behaviour ─────────────────────────
impl App {
    fn banner(&mut self) {
        let wd = short_path(&self.workdir);
        self.blocks.push(Block::Banner(vec![
            format!("✻ TheHarness {} — local coding agent", harness::version()),
            format!("  model  {}", self.model),
            if self.cfg.llm.provider.as_deref() == Some("claude-code") { "  server claude-code (official CLI, your Anthropic subscription; tools bridged over MCP)".to_string() } else { format!("  server {}", self.cfg.llm.base_url) },
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

    fn setting_value(&self, key: &str) -> String {
        let b = |v: bool| if v { "on" } else { "off" }.to_string();
        match key {
            "ui.tool_view" => self.tool_view.clone(), "ui.show_thinking" => b(self.show_thinking), "ui.panel" => match self.panel { Some(true) => "on".into(), Some(false) => "off".into(), None => "auto".into() },
            "ui.theme" => if LIGHT.load(std::sync::atomic::Ordering::Relaxed) { "light".into() } else { "dark".into() }, "ui.notify" => b(self.cfg.ui.notify), "ui.fold_previous" => b(self.cfg.ui.fold_previous), "ui.vim" => b(self.vim),
            "permissions.mode" => format!("{:?}", self.perm_mode).to_lowercase(), "llm.effort" => self.cfg.llm.effort.clone().unwrap_or("medium".into()), "llm.compact_at_fraction" => format!("{}", self.cfg.llm.compact_at_fraction),
            "memory.auto_reflect" => b(self.cfg.memory.auto_reflect), "security.redact_secrets" => b(self.cfg.security.redact_secrets), "net.enabled" => b(self.net), "agent.max_task_secs" => self.cfg.agent.max_task_secs.to_string(), "ui.event_log" => b(self.cfg.ui.event_log), "sandbox.mode" => if self.cfg.sandbox.mode.is_empty() { "none".into() } else { self.cfg.sandbox.mode.clone() },
            "llm.provider" => self.cfg.llm.provider.clone().unwrap_or("local (OpenAI-compatible server)".into()), "llm.model" => self.model.clone(),
            _ => String::new(),
        }
    }
    /// Cycle the selected setting; apply live + persist.
    fn cycle_setting(&mut self, dir: i32) {
        let (key, _, choices, _) = SETTINGS[self.settings_cursor];
        if choices.is_empty() { self.set_status("read-only here — use /backend or /model"); return; }
        let cur = self.setting_value(key);
        let idx = choices.iter().position(|c| *c == cur).unwrap_or(0) as i32;
        let next = choices[((idx + dir).rem_euclid(choices.len() as i32)) as usize];
        self.apply_setting(key, next);
    }
    fn apply_setting(&mut self, key: &str, val: &str) {
        let _ = self.cfg.set_setting(key, val);
        match key {
            "ui.tool_view" => self.tool_view = val.into(),
            "ui.show_thinking" => self.show_thinking = val == "on",
            "ui.panel" => self.panel = match val { "on" => Some(true), "off" => Some(false), _ => None },
            "ui.theme" => LIGHT.store(val == "light", std::sync::atomic::Ordering::Relaxed),
            "ui.vim" => { self.vim = val == "on"; self.vim_normal = false; }
            "permissions.mode" => { if let Some(m) = harness::permissions::Mode::parse(val) { self.set_perm_mode(m); } }
            "net.enabled" => { self.net = val == "on"; self.reload_toolset(); }
            "llm.effort" => { if let Some(cc) = self.cc.take() { tokio::spawn(async move { cc.stop().await; }); } }
            _ => {}
        }
        match harness::config::Config::save_setting(key, val) { Ok(()) => self.set_status(format!("{key} = {val} (saved)")), Err(e) => self.set_status(format!("{key} = {val} (not saved: {e})")) }
    }
    fn set_perm_mode(&mut self, m: harness::permissions::Mode) {
        self.perm_mode = m; if let Some(p) = &self.live_policy { p.set_mode(m); }
        if m == harness::permissions::Mode::Bypass { if let Some((_, tx)) = self.pending_ask.take() { let _ = tx.send(harness::permissions::Approval::Once); self.blocks.push(Block::System("🔒 pending prompt auto-approved (bypass)".into())); } }
    }

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
            CEvent::Key(k) if k.kind == KeyEventKind::Press && self.settings_open => {
                let n = SETTINGS.len();
                match k.code {
                    KeyCode::Esc | KeyCode::Char('q') => self.settings_open = false,
                    KeyCode::Up | KeyCode::Char('k') => self.settings_cursor = self.settings_cursor.saturating_sub(1),
                    KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => self.settings_cursor = (self.settings_cursor + 1).min(n - 1),
                    KeyCode::Right | KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Char('l') => self.cycle_setting(1),
                    KeyCode::Left | KeyCode::Char('h') => self.cycle_setting(-1),
                    KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => self.settings_open = false,
                    _ => {}
                }
            }
            CEvent::Key(k) if k.kind == KeyEventKind::Press && self.pending_q.is_some() => {
                let nopts = self.pending_q.as_ref().map(|(q, _, _)| q.options.len()).unwrap_or(0);
                let buf_empty = self.pending_q.as_ref().map(|(_, _, b)| b.is_empty()).unwrap_or(true);
                let mut answer: Option<harness::permissions::Answer> = None;
                match k.code {
                    KeyCode::Esc => answer = Some(harness::permissions::Answer { declined: true, ..Default::default() }),
                    KeyCode::Char(c) if c.is_ascii_digit() && buf_empty && (c as usize - '0' as usize) >= 1 && (c as usize - '0' as usize) <= nopts => answer = Some(harness::permissions::Answer { choice: Some(c as usize - '1' as usize), ..Default::default() }),
                    KeyCode::Enter => { let t = self.pending_q.as_ref().map(|(_, _, b)| b.clone()).unwrap_or_default(); if !t.trim().is_empty() { answer = Some(harness::permissions::Answer { text: Some(t), ..Default::default() }); } }
                    KeyCode::Backspace => { if let Some((_, _, b)) = &mut self.pending_q { b.pop(); } }
                    KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => answer = Some(harness::permissions::Answer { declined: true, ..Default::default() }),
                    KeyCode::Char(c) => { if let Some((q, _, b)) = &mut self.pending_q { if q.allow_free_text { b.push(c); } } }
                    _ => {}
                }
                if let Some(a) = answer { if let Some((q, tx, _)) = self.pending_q.take() { let label = if a.declined { "declined".to_string() } else if let Some(i) = a.choice { format!("→ {}", q.options.get(i).map(|o| o.label.clone()).unwrap_or_default()) } else { format!("→ {}", a.text.clone().unwrap_or_default()) }; self.blocks.push(Block::System(format!("❓ answer {label}"))); let _ = tx.send(a); } }
            }
            CEvent::Key(k) if k.kind == KeyEventKind::Press && self.pending_ask.is_some() => {
                let ans = match k.code {
                    KeyCode::Char('y') | KeyCode::Enter => Some(harness::permissions::Approval::Once),
                    KeyCode::Char('a') => Some(harness::permissions::Approval::Always),
                    KeyCode::Char('p') => Some(harness::permissions::Approval::AlwaysProject),
                    KeyCode::Char('n') | KeyCode::Esc => Some(harness::permissions::Approval::Deny),
                    KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => Some(harness::permissions::Approval::Deny),
                    _ => None,
                };
                if let Some(a) = ans { if let Some((req, tx)) = self.pending_ask.take() { let label = match &a { harness::permissions::Approval::Once => "allowed once".to_string(), harness::permissions::Approval::Always => format!("always allow {}", req.suggested_rule), harness::permissions::Approval::AlwaysProject => format!("always allow {} (this project)", req.suggested_rule), harness::permissions::Approval::Deny => "denied".into() }; self.blocks.push(Block::System(format!("🔒 {label}"))); let _ = tx.send(a); } }
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
                let km = self.keymap.clone();
                // 1) global actions (custom keybindings) — take precedence over typing
                if km.is("next_task", k.code, k.modifiers) { self.next_task(); return; }
                if km.is("toggle_panel", k.code, k.modifiers) { self.panel = Some(!self.panel_visible(200)); return; }
                if km.is("toggle_thinking", k.code, k.modifiers) { self.show_thinking = !self.show_thinking; return; }
                if km.is("expand_tools", k.code, k.modifiers) { self.tool_view = match self.tool_view.as_str() { "summary" => "full".into(), "full" => "hidden".into(), _ => "summary".into() }; if self.tool_view == "full" { self.expand_tools = !self.expand_tools; } self.set_status(format!("tool calls: {}{}", self.tool_view, if self.tool_view == "full" && self.expand_tools { " (outputs expanded)" } else { "" })); return; }
                if km.is("paste", k.code, k.modifiers) { let tx = self.tx.clone(); let store = if self.cfg.memory.enabled { harness::memory::MemoryStore::open(&self.cfg.memory).ok() } else { None }; self.set_status("reading clipboard…"); tokio::spawn(async move { let _ = tx.send(Msg::Pasted(clipboard_image(store).await)); }); return; }
                if km.is("cycle_permissions", k.code, k.modifiers) { use harness::permissions::Mode::*; let m = match self.perm_mode { Auto => Ask, Ask => Plan, Plan => Bypass, Bypass => Auto }; self.set_perm_mode(m); self.set_status(format!("permissions → {}", self.perm_mode.label())); return; }
                if km.is("newline", k.code, k.modifiers) || (k.code == KeyCode::Enter && alt) { self.insert_str("\n"); return; }
                if km.is("quit", k.code, k.modifiers) && self.input.is_empty() { self.quit = true; return; }
                if km.is("scroll_up", k.code, k.modifiers) || (k.code == KeyCode::Up && ctrl) { self.scroll_up += 10; return; }
                if km.is("scroll_down", k.code, k.modifiers) || (k.code == KeyCode::Down && ctrl) { self.scroll_up = self.scroll_up.saturating_sub(10); return; }
                if km.is("clear_line", k.code, k.modifiers) { let c = self.cursor; self.input = self.input.chars().skip(c).collect(); self.cursor = 0; return; }
                if km.is("jump_bottom", k.code, k.modifiers) { self.scroll_up = 0; return; }
                if km.is("complete", k.code, k.modifiers) { self.complete_slash(); return; }
                if km.is("interrupt", k.code, k.modifiers) && self.running.is_some() { self.interrupt(); return; }
                // 2) vim normal mode
                if self.vim && self.vim_normal {
                    let n = self.input.chars().count();
                    match k.code {
                        KeyCode::Char('i') => self.vim_normal = false,
                        KeyCode::Char('a') => { self.cursor = (self.cursor + 1).min(n); self.vim_normal = false; }
                        KeyCode::Char('A') => { self.cursor = n; self.vim_normal = false; }
                        KeyCode::Char('I') => { self.cursor = 0; self.vim_normal = false; }
                        KeyCode::Char('h') | KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
                        KeyCode::Char('l') | KeyCode::Right => self.cursor = (self.cursor + 1).min(n),
                        KeyCode::Char('0') | KeyCode::Home => self.cursor = self.line_start(),
                        KeyCode::Char('$') | KeyCode::End => self.cursor = self.line_end(),
                        KeyCode::Char('w') => { let cs: Vec<char> = self.input.chars().collect(); let mut i = self.cursor; while i < cs.len() && !cs[i].is_whitespace() { i += 1; } while i < cs.len() && cs[i].is_whitespace() { i += 1; } self.cursor = i; }
                        KeyCode::Char('b') => { let cs: Vec<char> = self.input.chars().collect(); let mut i = self.cursor; while i > 0 && cs[i - 1].is_whitespace() { i -= 1; } while i > 0 && !cs[i - 1].is_whitespace() { i -= 1; } self.cursor = i; }
                        KeyCode::Char('x') => { let mut cs: Vec<char> = self.input.chars().collect(); if self.cursor < cs.len() { cs.remove(self.cursor); self.input = cs.into_iter().collect(); } }
                        KeyCode::Char('d') => { self.input.clear(); self.cursor = 0; } // dd (single d clears the line — simplification)
                        KeyCode::Char('u') => { if let Some(prev) = self.history.last() { self.input = prev.clone(); self.cursor = self.input.chars().count(); } }
                        KeyCode::Enter => self.submit(),
                        KeyCode::Char('j') | KeyCode::Down => self.history_next(),
                        KeyCode::Char('k') | KeyCode::Up => self.history_prev(),
                        KeyCode::Char(':') => { self.insert_str("/"); self.vim_normal = false; }
                        KeyCode::Esc => { if self.restart_at.is_some() { self.cancel_restart(); } else if self.running.is_some() { self.interrupt(); } }
                        _ => {}
                    }
                    return;
                }
                match (k.code, ctrl, alt) {
                    (KeyCode::Char('c'), true, _) => {
                        if self.running.is_some() { self.interrupt(); }
                        else if !self.input.is_empty() { self.input.clear(); self.cursor = 0; }
                        else if self.last_ctrl_c.map(|t| t.elapsed() < Duration::from_millis(1500)).unwrap_or(false) { self.quit = true; }
                        else { self.last_ctrl_c = Some(Instant::now()); self.set_status("Press ctrl+c again to exit"); }
                    }
                    (KeyCode::Esc, _, _) => { if self.restart_at.is_some() { self.cancel_restart(); } else if self.running.is_some() { self.interrupt(); } else if self.vim { self.vim_normal = true; } else if !self.input.is_empty() { self.input.clear(); self.cursor = 0; } }
                    (KeyCode::Enter, _, _) => self.submit(),
                    (KeyCode::Char('a'), true, _) | (KeyCode::Home, _, _) => self.cursor = self.line_start(),
                    (KeyCode::Char('e'), true, _) | (KeyCode::End, _, _) => self.cursor = self.line_end(),
                    (KeyCode::Backspace, _, _) => { if self.cursor > 0 { let mut cs: Vec<char> = self.input.chars().collect(); cs.remove(self.cursor - 1); self.input = cs.into_iter().collect(); self.cursor -= 1; } }
                    (KeyCode::Delete, _, _) => { let mut cs: Vec<char> = self.input.chars().collect(); if self.cursor < cs.len() { cs.remove(self.cursor); self.input = cs.into_iter().collect(); } }
                    (KeyCode::Left, _, _) => { self.cursor = self.cursor.saturating_sub(1); }
                    (KeyCode::Right, _, _) => { self.cursor = (self.cursor + 1).min(self.input.chars().count()); }
                    (KeyCode::Up, _, _) => { if !self.input.contains('\n') { self.history_prev(); } }
                    (KeyCode::Down, _, _) => { if !self.input.contains('\n') { self.history_next(); } }
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
            let p = if t.starts_with('~') { PathBuf::from(t.replacen('~', &harness::setup::home_dir().display().to_string(), 1)) } else if PathBuf::from(&t).is_absolute() { PathBuf::from(&t) } else { self.workdir.join(&t) };
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
            let p = if t.starts_with('~') { PathBuf::from(t.replacen('~', &harness::setup::home_dir().display().to_string(), 1)) } else if PathBuf::from(t).is_absolute() { PathBuf::from(t) } else { self.workdir.join(t) };
            if video_ext(&p) && p.is_file() && !self.input.contains("@") { self.open_video(&p); return; }
        }
        let text = if text.is_empty() { "Look at the attached image(s).".to_string() } else { text };
        self.input.clear(); self.cursor = 0; self.hist_idx = None;
        if self.history.last() != Some(&text) { self.history.push(text.clone()); }
        if text.starts_with('/') { self.command(&text); return; }
        if let Some(id) = self.attached {
            if let Some(a) = self.subenv.as_ref().and_then(|e| e.list().into_iter().find(|a| a.id == id)) {
                if a.running() { a.inbox.push("message from the user (attached)", text.clone()); self.blocks.push(Block::System(format!("→ {} (delivered before its next model call): {}", a.label, truncate(&text, 120)))); return; }
                self.set_status(format!("sub-agent #{id} is finished — /agents detach")); return;
            }
        }
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
            "/msg" | "/send" => {
                let mut it = arg.splitn(2, ' '); let to = it.next().unwrap_or("").to_string(); let text = it.next().unwrap_or("").trim().to_string();
                if to.is_empty() || text.is_empty() { self.blocks.push(Block::Error("usage: /msg <session id|prefix|title|all> <text>   (see /sessions live)".into())); }
                else { match harness::mailbox::send(&to, &self.session_meta.id, &text) { Ok(n) => self.blocks.push(Block::System(format!("✉ delivered to {n} session(s)"))), Err(e) => self.blocks.push(Block::Error(format!("send failed: {e}"))) } }
            }
            "/sessions" if arg == "live" => {
                let l = harness::mailbox::live();
                let mut lines = vec![format!("Live sessions ({}) — /msg <id|prefix|title|all> <text>", l.len())];
                for s in l { lines.push(format!("  {}{}  {:<40} {}  [{}]{}", if s.id == self.session_meta.id { "● " } else { "  " }, s.id, truncate(&s.title, 40), short_path(std::path::Path::new(&s.workdir)), s.backend, if s.busy { " busy" } else { "" })); }
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
                if let Some(cc) = self.cc.take() { tokio::spawn(async move { cc.stop().await; }); } self.cc_last_session = None;
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
                } else { self.model = arg.clone(); self.cfg.llm.model = arg.clone(); self.blocks.push(Block::System(format!("model → {arg}"))); if self.cfg.llm.provider.as_deref() == Some("claude-code") { if let Some(cc) = self.cc.take() { tokio::spawn(async move { cc.stop().await; }); } self.cc_last_session = None; } else { tokio::spawn(fetch_ctx_len(self.cfg.llm.base_url.clone(), arg.clone(), self.tx.clone())); } }
            }
            "/cd" => {
                let p = if arg.is_empty() { harness::setup::home_dir().display().to_string() } else { arg.clone() };
                let p = if p.starts_with('~') { p.replacen('~', &harness::setup::home_dir().display().to_string(), 1) } else { p };
                let p = if PathBuf::from(&p).is_absolute() { PathBuf::from(&p) } else { self.workdir.join(&p) };
                match p.canonicalize() { Ok(p) if p.is_dir() => { self.workdir = p.clone(); *self.wt_cwd.lock().unwrap() = None; let _ = std::env::set_current_dir(&p); self.blocks.push(Block::System(format!("cwd → {}", short_path(&p)))); }
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
                for (n, c, f) in servers { lines.push(format!("  {:<18} {} {}   ← {}", n, if c.command.is_empty() { c.url.clone().unwrap_or_default() } else { c.command.clone() }, c.args.join(" "), short_path(&f))); }
                let live: Vec<String> = self.toolset.as_ref().map(|ts| ts.registry.names().into_iter().filter(|n| n.starts_with("mcp__")).map(String::from).collect()).unwrap_or_default();
                lines.push(format!("live MCP tools: {}", live.len()));
                for t in live.iter().take(40) { lines.push(format!("  {t}")); }
                self.blocks.push(Block::Banner(lines));
            }
            "/reload" => { self.reload_toolset(); self.blocks.push(Block::System("reloading tools, MCP servers and plugins…".into())); }
            "/compact" if self.cfg.llm.provider.as_deref() == Some("claude-code") => {
                if self.running.is_some() { self.set_status("wait for the current turn to finish"); }
                else if let Some(cc) = self.cc.clone() {
                    let tx = self.tx.clone(); let focus = if arg.is_empty() { None } else { Some(arg.clone()) };
                    self.blocks.push(Block::System("asking Claude Code to compact its context…".into()));
                    tokio::spawn(async move { let sink = TuiSink(tx.clone()); match cc.compact(focus.as_deref(), &sink).await { Ok((pre, post)) => { let _ = tx.send(Msg::Notice(format!("Claude Code compacted: {} → {} tokens", fmt_k(pre), fmt_k(post)))); } Err(e) => { sink.emit(&Event::Error { message: format!("compact: {e:#}") }); } } });
                } else { self.blocks.push(Block::System("no Claude session yet — nothing to compact".into())); }
            }
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
                            let (n, summary, mb, ma) = harness::agent::compact_llm_with(&client.aux(), &mut msgs, 4, focus.as_deref(), Some(&sink)).await.map_err(|e| format!("{e:#}"))?;
                            sink.emit(&Event::Compacted { count: n, prompt_tokens: 0, summary, map_before: mb, map_after: ma });
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
            "/config" if !arg.is_empty() => self.blocks.push(Block::Banner(vec![format!("server  {}", self.cfg.llm.base_url), format!("model   {}", self.model), format!("context {} · compaction at {} tokens · max_turns {} · tool timeout {}s", fmt_k(self.metrics.ctx_len), fmt_k(self.cfg.llm.effective_budget(if self.metrics.ctx_len > 0 { Some(self.metrics.ctx_len) } else { None })), self.cfg.agent.max_turns, self.cfg.agent.tool_timeout_secs), format!("net {} · segments {}", self.net, self.cfg.net.download_segments)])),
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
            "/video" => { if arg.is_empty() { self.blocks.push(Block::Error("usage: /video <path>".into())); } else { let p = if arg.starts_with('~') { PathBuf::from(arg.replacen('~', &harness::setup::home_dir().display().to_string(), 1)) } else { PathBuf::from(&arg) }; if p.is_file() { self.open_video(&p); } else { self.blocks.push(Block::Error(format!("no such file: {arg}"))); } } }
            "/permissions" | "/perm" | "/mode" => {
                let mut it = arg.splitn(2, ' '); let sub = it.next().unwrap_or("").to_string(); let rest = it.next().unwrap_or("").trim().to_string();
                if arg.is_empty() {
                    let rules = harness::permissions::persisted_rules(); let proj = harness::permissions::project_rules(&self.workdir);
                    let mut lines = vec![format!("permission mode: {} ({})", format!("{:?}", self.perm_mode).to_lowercase(), self.perm_mode.label()), "switch: /permissions bypass|auto|ask|plan   (shift+tab cycles) · rules: /permissions add <rule> [project] · remove <rule>".into(), format!("config allow: {:?}", self.cfg.permissions.allow), format!("config deny:  {:?}", self.cfg.permissions.deny), format!("always-allowed (this machine): {:?}", rules), format!("always-allowed (this project, .harness/permissions.json): {:?}", proj), format!("directory trusted: {}  (/trust to remember this directory)", harness::permissions::is_trusted(&self.workdir))];
                    lines.push("Rules are '<tool>' or '<tool>:<glob>' matched on the primary argument (bash cmd, file path, url).".into());
                    self.blocks.push(Block::Banner(lines));
                } else if sub == "add" && !rest.is_empty() {
                    let (rule, project) = match rest.strip_suffix(" project") { Some(r) => (r.trim().to_string(), true), None => (rest.clone(), false) };
                    match &self.live_policy { Some(p) => { if project { p.allow_always_project(&rule) } else { p.allow_always(&rule) } } None => { let p = harness::permissions::Policy::new(self.cfg.permissions.clone(), &self.workdir); if project { p.allow_always_project(&rule) } else { p.allow_always(&rule) } } }
                    self.blocks.push(Block::System(format!("rule added{}: {rule}", if project { " (project)" } else { "" })));
                } else if sub == "remove" && !rest.is_empty() {
                    let p = self.live_policy.clone().unwrap_or_else(|| Arc::new(harness::permissions::Policy::new(self.cfg.permissions.clone(), &self.workdir)));
                    let n = p.remove_rule(&rest); self.blocks.push(Block::System(format!("removed {n} rule(s) matching {rest}")));
                } else if let Some(m) = harness::permissions::Mode::parse(&arg) { self.set_perm_mode(m); self.blocks.push(Block::System(format!("permissions → {}", m.label()))); }
                else { self.blocks.push(Block::Error("usage: /permissions [bypass|auto|ask|plan] · add <rule> [project] · remove <rule>".into())); }
            }
            "/trust" => { harness::permissions::trust(&self.workdir); self.blocks.push(Block::System(format!("{} is now a trusted directory", short_path(&self.workdir)))); }
            "/vim" => { self.vim = !self.vim; self.vim_normal = false; self.blocks.push(Block::System(format!("vim mode {} — esc → NORMAL (h/l/w/b/0/$ move, x delete, d clear, i/a/A/I insert, j/k history, : starts a /command, enter sends)", if self.vim { "on" } else { "off" }))); }
            "/theme" => { let light = match arg.as_str() { "light" => true, "dark" => false, _ => !LIGHT.load(std::sync::atomic::Ordering::Relaxed) }; LIGHT.store(light, std::sync::atomic::Ordering::Relaxed); self.blocks.push(Block::System(format!("theme → {}", if light { "light" } else { "dark" }))); }
            "/plan" => { let m = if self.perm_mode == harness::permissions::Mode::Plan { harness::permissions::Mode::Auto } else { harness::permissions::Mode::Plan }; self.set_perm_mode(m); self.blocks.push(Block::System(format!("permissions → {}", self.perm_mode.label()))); }
            "/context" | "/ctx" => {
                let tx = self.tx.clone(); let session = self.session.clone(); let cfg = self.cfg.clone(); let workdir = self.workdir.clone(); let toolset = self.toolset.clone();
                let window = self.metrics.ctx_len; let measured = self.last_prompt_tokens;
                tokio::spawn(async move {
                    let msgs = session.lock().await.clone();
                    let store = if cfg.memory.enabled { harness::memory::MemoryStore::open(&cfg.memory).ok() } else { None };
                    let (defs_json, extra_prompt, names): (usize, String, Vec<String>) = match &toolset { Some(ts) => (serde_json::to_string(&ts.registry.defs()).map(|s| s.len()).unwrap_or(0), ts.prompt_extra.clone(), ts.registry.names().into_iter().map(String::from).collect()), None => { let r = Registry::defaults(cfg.net.enabled); (serde_json::to_string(&r.defs()).map(|s| s.len()).unwrap_or(0), String::new(), r.names().into_iter().map(String::from).collect()) } };
                    let tok = |c: usize| (c / 4) as u64;
                    let base = harness::agent::base_prompt_template().len() + harness::setup::summary_line().len() + names.iter().map(|n| n.len() + 2).sum::<usize>();
                    let mem_block = store.as_ref().map(|m| m.prompt_block(&workdir).len()).unwrap_or(0);
                    let mut segs: Vec<(String, u64)> = vec![("system prompt".into(), tok(base)), ("tool schemas".into(), tok(defs_json)), ("memory files".into(), tok(mem_block)), ("skills/plugins".into(), tok(extra_prompt.len()))];
                    let mut user = 0u64; let mut asst = 0u64; let mut tools = 0u64; let mut imgs = 0u64; let mut note = 0u64;
                    let mut items: Vec<(u64, String)> = Vec::new();
                    for m in msgs.iter().skip(1) {
                        let t = m.text(); let n = tok(t.chars().count());
                        match m.role.as_str() {
                            "user" => { if t.starts_with("[Context compacted") { note += n; items.push((n, "handoff note".into())); } else { user += n; items.push((n, format!("user: {}", truncate(t.lines().next().unwrap_or(""), 50)))); } if let Some(Content::Parts(p)) = &m.content { let k = p.iter().filter(|x| x["type"] == "image_url").count() as u64; imgs += k * 1200; } }
                            "assistant" => { let mut a = n; if let Some(c) = &m.tool_calls { a += c.iter().map(|c| tok(c.function.arguments.chars().count())).sum::<u64>(); } asst += a; if a > 0 { items.push((a, format!("assistant: {}", truncate(t.lines().next().unwrap_or("(tool calls)"), 50)))); } }
                            "tool" => { tools += n; items.push((n, format!("tool result {}: {}", m.name.clone().unwrap_or_default(), truncate(t.lines().next().unwrap_or(""), 44)))); }
                            _ => {}
                        }
                    }
                    segs.push(("handoff notes".into(), note)); segs.push(("user messages".into(), user)); segs.push(("assistant".into(), asst)); segs.push(("tool results".into(), tools)); segs.push(("images".into(), imgs));
                    items.sort_by(|a, b| b.0.cmp(&a.0));
                    let top: Vec<String> = items.iter().take(10).map(|(n, l)| format!("{:>6}  {}", fmt_k(*n), l)).collect();
                    let total: u64 = segs.iter().map(|x| x.1).sum();
                    let mut hints = Vec::new();
                    if window > 0 && total * 100 / window > 60 { hints.push("context above 60% — /compact keeps recent messages verbatim and summarizes the rest".into()); }
                    if tools > total / 2 && total > 5000 { hints.push("tool results dominate — prefer grep/glob with tighter patterns and read_file with offset/limit".into()); }
                    if imgs > 0 { hints.push(format!("{} image(s) ≈ {} tokens; old images are dropped on compaction", imgs / 1200, fmt_k(imgs))); }
                    if defs_json / 4 > 6000 { hints.push("tool schemas are large (many MCP tools) — disable unused MCP servers/plugins to save context per call".into()); }
                    if mem_block / 4 > 4000 { hints.push("memory files are large — /brain, /memory: consolidate or lower [memory] max_inject_chars".into()); }
                    let _ = tx.send(Msg::Block(Block::ContextReport { segments: segs, window, measured, top, hints }));
                });
            }
            "/workflow" | "/wf" => {
                let mut it = arg.splitn(2, ' '); let name = it.next().unwrap_or("").to_string(); let wargs = it.next().unwrap_or("").to_string();
                if name.is_empty() || name == "list" {
                    let l = harness::workflow::list(&self.workdir);
                    let mut lines = vec!["Workflows — /workflow <name> [args]   (files: ~/.config/harness/workflows/*.toml, .harness/workflows/*.toml)".to_string()];
                    for (n, d, _) in l { lines.push(format!("  {:<14} {}", n, truncate(&d, 100))); }
                    self.blocks.push(Block::Banner(lines));
                } else if self.running.is_some() { self.set_status("finish or interrupt the current task first"); }
                else {
                    match harness::workflow::find(&name, &self.workdir) {
                        Err(e) => self.blocks.push(Block::Error(e.to_string())),
                        Ok(wf) => {
                            self.blocks.push(Block::User(format!("/workflow {name} {wargs}"), vec![]));
                            let tx = self.tx.clone(); let cfg = self.cfg.clone(); let workdir = self.workdir.clone(); let toolset = self.toolset.clone(); let perm_mode = self.perm_mode; let todos = self.todos.clone();
                            let budget = self.cfg.llm.effective_budget(if self.metrics.ctx_len > 0 { Some(self.metrics.ctx_len) } else { None });
                            self.run_started = Instant::now(); self.metrics.turn_start = Some(Instant::now());
                            let handle = tokio::spawn(async move {
                                let res: Result<(String, harness::agent::RunStats), String> = async {
                                    let client = Client::new(&cfg.llm).map_err(|e| e.to_string())?;
                                    let store = if cfg.memory.enabled { harness::memory::MemoryStore::open(&cfg.memory).ok() } else { None };
                                    let fallback = Registry::defaults(cfg.net.enabled);
                                    let (registry, extra_prompt): (Registry, String) = match &toolset { Some(ts) => (ts.registry.clone(), ts.prompt_extra.clone()), None => (fallback, String::new()) };
                                    let sink: Arc<dyn Sink> = Arc::new(TuiSink(tx.clone()));
                                    let mut pcfg = cfg.permissions.clone(); pcfg.mode = perm_mode; pcfg.allow.extend(harness::permissions::persisted_rules());
                                    let policy = Arc::new(harness::permissions::Policy::new(pcfg, &workdir));
                                    let approver: Arc<dyn harness::permissions::Approver> = Arc::new(TuiApprover(tx.clone()));
                                    let env = Arc::new(harness::agent::SubAgentEnv::new(client.clone(), registry.clone(), policy.clone(), approver.clone(), sink.clone(), budget, true));
                                    let ctx = ToolCtx { memory: store.clone(), subagent: Some(env.clone()), redact_secrets: cfg.security.redact_secrets, hooks: cfg.hooks.clone(), lsp_servers: cfg.lsp.servers.clone(), todos, timeout: Duration::from_secs(cfg.agent.tool_timeout_secs), max_output: cfg.agent.max_tool_output_chars, net: cfg.net.clone(), ..ToolCtx::basic(workdir.clone()) };
                                    let base_system = harness::agent::system_prompt_with_memory(&workdir.display().to_string(), &registry.names(), Some(&extra_prompt), store.as_ref());
                                    let wenv = harness::workflow::WorkflowEnv { env, ctx, sink: sink.clone(), base_system };
                                    let out = harness::workflow::run(&wf, &wargs, &wenv).await.map_err(|e| format!("{e:#}"))?;
                                    sink.emit(&Event::Assistant { text: format!("Workflow `{}` finished.\n\n{out}", wf.name) });
                                    Ok((out, harness::agent::RunStats { stop_reason: "done".into(), ..Default::default() }))
                                }.await;
                                let _ = tx.send(Msg::Done(res));
                            });
                            self.running = Some(handle);
                        }
                    }
                }
            }
            "/settings" | "/config" if arg.is_empty() => { self.settings_open = true; self.settings_cursor = 0; }
            "/keybindings" | "/keys" | "/shortcuts" => {
                self.blocks.push(Block::Banner(vec![
                    "Keyboard shortcuts".into(),
                    "  enter            send · queue if a task is running      alt+enter / ctrl+j   newline".into(),
                    "  esc              interrupt the current task (context kept) · clear input when idle".into(),
                    "  ctrl+c           interrupt · clear input · press twice to quit     ctrl+d   quit (empty input)".into(),
                    "  ctrl+n           stop current task and start the next queued one".into(),
                    "  ctrl+o           expand/collapse all tool outputs      click a block  fold/unfold it".into(),
                    "  ctrl+t           show/hide thinking inline               ctrl+p         dashboard panel".into(),
                    "  ctrl+v           paste image/file from clipboard        ctrl+u         clear line".into(),
                    "  ctrl+a / ctrl+e  line start / end                       ctrl+l         jump to bottom".into(),
                    "  shift+tab        cycle permission mode (auto → ask → plan → bypass)".into(),
                    "  tab              complete a /command                     ↑ / ↓          input history".into(),
                    "  pgup / pgdn, ctrl+↑/↓, mouse wheel   scroll transcript (wheel over the panel scrolls thinking)".into(),
                    "  y / a / n        answer a permission prompt (once / always / deny)".into(),
                    "  video scrubber:  ←/→ frames · space select · a all · enter attach · esc cancel".into(),
                    "  text selection:  hold shift (kitty/wezterm/iterm) or fn/option (Terminal.app) while dragging".into(),
                ]));
            }
            "/status" => {
                let cc = self.cfg.llm.provider.as_deref() == Some("claude-code");
                let mut lines = vec![
                    format!("version   {}", harness::version()),
                    format!("backend   {}", if cc { format!("Claude Code · {} · effort {}", self.model, self.cfg.llm.effort.clone().unwrap_or("medium".into())) } else if self.cfg.llm.provider.as_deref() == Some("anthropic") { format!("Anthropic API · {}", self.model) } else { format!("{} · {}", self.cfg.llm.base_url, self.model) }),
                    format!("context   {} window · last prompt {} · session {} in / {} out", fmt_k(self.metrics.ctx_len), fmt_k(self.last_prompt_tokens), fmt_k(self.total_prompt), fmt_k(self.total_completion)),
                    format!("session   {} · {} turns · workdir {}", if self.session_meta.id.is_empty() { "(unsaved)".to_string() } else { self.session_meta.id.clone() }, self.history.len(), short_path(&self.workdir)),
                    format!("perms     {} · net {} · tools {}", self.perm_mode.label(), if self.net { "on" } else { "off" }, self.toolset.as_ref().map(|t| t.registry.len()).unwrap_or(0)),
                    format!("queue     {} waiting · running: {}", self.queued.len(), self.running.is_some()),
                ];
                if let Some((st, kind, at)) = &self.cc_rate { let mins = at.saturating_sub(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)) / 60; lines.push(format!("claude    rate limit {st} ({kind}) · window resets in {}h{:02}m · /usage for details", mins / 60, mins % 60)); }
                if let Ok(p) = harness::plugins::Plugins::open() { let en = p.enabled(); lines.push(format!("plugins   {} enabled ({} skills)", en.len(), en.iter().map(|x| x.skills.len()).sum::<usize>())); }
                self.blocks.push(Block::Banner(lines));
            }
            "/doctor" => {
                let st = harness::setup::check();
                let mut lines = vec!["Doctor — external tools".to_string()];
                for t in &st { lines.push(format!("  {} {:<26} {}", if t.ok() { "✓" } else { "·" }, t.name, if t.ok() { t.found.first().map(|(_, p)| p.display().to_string()).unwrap_or_default() } else { format!("missing → {}", t.install.clone().unwrap_or("(system)".into())) })); }
                lines.push(String::new());
                lines.push(format!("  claude CLI  {}", harness::claude_code::claude_bin().map(|p| p.display().to_string()).unwrap_or("not found (needed for /backend claude)".into())));
                lines.push(format!("  config      {}", short_path(&harness::setup::config_dir().join("harness.toml"))));
                lines.push(format!("  memory dir  {}  · sessions {}  · plugins {}", short_path(&harness::setup::config_dir()), short_path(&harness::setup::config_dir().join("sessions")), short_path(&harness::setup::config_dir().join("plugins"))));
                lines.push("  fix missing tools: harness setup --install".into());
                self.blocks.push(Block::Banner(lines));
            }
            "/init" => {
                let prompt = "Analyze this repository and write HARNESS.md at the repo root — project instructions for a coding agent working here: how to build/run/test (exact commands), directory structure and key files, code conventions, common pitfalls, and anything an engineer would tell a new teammate. Base it only on what you find (read README, build files, CI, existing docs, and skim the source). Keep it under 150 lines. If HARNESS.md exists, improve it instead of replacing it.".to_string();
                if self.running.is_some() { self.queued.push(prompt); self.set_status("queued /init"); } else { self.start_run(prompt); }
            }
            "/add-dir" => {
                let p = if arg.starts_with('~') { PathBuf::from(arg.replacen('~', &harness::setup::home_dir().display().to_string(), 1)) } else { PathBuf::from(&arg) };
                match p.canonicalize() { Ok(p) if p.is_dir() => { self.extra_roots.push(p.clone()); self.blocks.push(Block::System(format!("added {} — file tools may now read/write there (this session)", short_path(&p)))); } _ => self.blocks.push(Block::Error(format!("usage: /add-dir <existing directory>  ({arg})"))) }
            }
            "/rename" => { if arg.is_empty() { self.blocks.push(Block::Error("usage: /rename <title>".into())); } else { self.session_meta.title = arg.clone(); self.save_session(); self.blocks.push(Block::System(format!("session renamed: {arg}"))); } }
            "/export" => {
                let session = self.session.clone(); let tx = self.tx.clone(); let name = if arg.is_empty() { format!("session-{}.md", if self.session_meta.id.is_empty() { "unsaved".into() } else { self.session_meta.id.clone() }) } else { arg.clone() };
                let out = harness::setup::config_dir().join("exports").join(&name);
                tokio::spawn(async move {
                    let msgs = session.lock().await.clone();
                    let mut md = String::from("# Harness session export\n\n");
                    for m in msgs.iter().skip(1) { match m.role.as_str() { "user" => md.push_str(&format!("## User\n\n{}\n\n", m.text())), "assistant" => { let t = m.text(); if !t.trim().is_empty() { md.push_str(&format!("## Assistant\n\n{}\n\n", t)); } if let Some(c) = &m.tool_calls { for c in c { md.push_str(&format!("**tool** `{}` `{}`\n\n", c.function.name, truncate(&c.function.arguments, 300))); } } } "tool" => md.push_str(&format!("```\n{}\n```\n\n", truncate(&m.text(), 1500))), _ => {} } }
                    let _ = std::fs::create_dir_all(out.parent().unwrap());
                    let r = std::fs::write(&out, md).map(|_| out.display().to_string()).map_err(|e| e.to_string());
                    let _ = tx.send(Msg::Notice(match r { Ok(p) => format!("exported to {p}"), Err(e) => format!("export failed: {e}") }));
                });
            }
            "/todos" => { let t = self.todos.lock().map(|t| t.clone()).unwrap_or_default(); if t.is_empty() { self.blocks.push(Block::System("no todos (the agent maintains them with the todo tool)".into())); } else { self.blocks.push(Block::Banner(std::iter::once("Todos".to_string()).chain(t.iter().map(|x| format!("  {}", x.line(&t)))).collect())); } }
            "/hooks" => { let h = &self.cfg.hooks; self.blocks.push(Block::Banner(vec!["Hooks (harness.toml [hooks]) — JSON on stdin; pre_tool exit 2 blocks the call".into(), format!("  pre_tool  {:?}", h.pre_tool), format!("  post_tool {:?}", h.post_tool), format!("  on_stop   {:?}", h.on_stop), format!("  on_prompt {:?}", h.on_prompt), format!("  timeout   {}s", h.timeout_secs)])); }
            "/skills" => { match harness::plugins::Plugins::open() { Ok(p) => { let sk: Vec<_> = p.enabled().into_iter().flat_map(|x| x.skills).collect(); if sk.is_empty() { self.blocks.push(Block::System("no skills installed — /plugin list".into())); } else { self.blocks.push(Block::Banner(std::iter::once(format!("Skills ({}) — the model loads them with load_skill", sk.len())).chain(sk.iter().map(|s| format!("  {:<28} {}  [{}]", s.name, truncate(&s.description, 80), s.plugin))).collect())); } } Err(e) => self.blocks.push(Block::Error(e.to_string())) } }
            "/diff" => { let wd = self.workdir.clone(); let tx = self.tx.clone(); tokio::spawn(async move { let o = harness::sandbox::run_shell("git status --short | head -40; echo; git diff --stat HEAD | tail -25", &wd, Duration::from_secs(20), 12000).await; let _ = tx.send(Msg::Block(Block::Banner(std::iter::once("git diff (working tree vs HEAD)".to_string()).chain(o.map(|o| o.stdout).unwrap_or_default().lines().map(String::from)).collect()))); }); }
            "/copy" => {
                let last = self.blocks.iter().rev().find_map(|b| if let Block::Assistant { text, .. } = b { Some(text.clone()) } else { None }).unwrap_or_default();
                if last.is_empty() { self.set_status("nothing to copy"); } else {
                    let tx = self.tx.clone();
                    tokio::spawn(async move {
                        let cmd = if cfg!(target_os = "macos") { "pbcopy" } else if cfg!(windows) { "clip" } else { "xclip -selection clipboard" };
                        let mut c = tokio::process::Command::new("/bin/sh"); c.arg("-c").arg(cmd).stdin(std::process::Stdio::piped());
                        if let Ok(mut ch) = c.spawn() { if let Some(mut si) = ch.stdin.take() { use tokio::io::AsyncWriteExt; let _ = si.write_all(last.as_bytes()).await; } let _ = ch.wait().await; let _ = tx.send(Msg::Notice("last answer copied to the clipboard".into())); }
                    });
                }
            }
            "/usage" if self.cfg.llm.provider.as_deref() == Some("claude-code") => {
                if let Some(cc) = self.cc.clone() {
                    let tx = self.tx.clone(); self.blocks.push(Block::System("asking Claude Code for subscription usage…".into()));
                    tokio::spawn(async move { match cc.command("/usage").await { Ok(t) => { let mut lines = vec!["Claude subscription usage (from Claude Code /usage)".to_string()]; lines.extend(t.lines().map(String::from)); let _ = tx.send(Msg::Block(Block::Banner(lines))); } Err(e) => { let _ = tx.send(Msg::Notice(format!("usage: {e:#}"))); } } });
                } else { self.blocks.push(Block::System("no Claude session yet — send one message first, then /usage".into())); }
            }
            "/usage" => { self.command("/cost"); }
            "/review" => { self.command(&format!("/workflow review {arg}")); }
            "/pr-comments" => { let wd = self.workdir.clone(); let tx = self.tx.clone(); let a = arg.clone(); tokio::spawn(async move { let o = harness::sandbox::run_shell(&format!("gh pr view {a} --comments 2>&1 | head -120", ), &wd, Duration::from_secs(30), 16000).await; let _ = tx.send(Msg::Block(Block::Banner(std::iter::once("PR comments (gh)".to_string()).chain(o.map(|o| o.stdout).unwrap_or_default().lines().map(String::from)).collect()))); }); }
            "/release-notes" | "/changelog" => { let wd = self.workdir.clone(); let tx = self.tx.clone(); tokio::spawn(async move { let o = harness::sandbox::run_shell("git log --oneline -30", &wd, Duration::from_secs(20), 12000).await; let _ = tx.send(Msg::Block(Block::Banner(std::iter::once("Recent commits".to_string()).chain(o.map(|o| o.stdout).unwrap_or_default().lines().map(String::from)).collect()))); }); }
            "/bug" | "/feedback" => { self.blocks.push(Block::Banner(vec!["Report a harness bug: include the session id and the event log".into(), format!("  session   {}", if self.session_meta.id.is_empty() { "(unsaved)".into() } else { self.session_meta.id.clone() }), format!("  log       {}", short_path(&harness::setup::config_dir().join("logs"))), "  repo      docs/GAPS.md · README.md".into()])); }
            "/agents" => {
                let list = self.subenv.as_ref().map(|e| e.list()).unwrap_or_default();
                let mut it = arg.split_whitespace(); let sub = it.next().unwrap_or(""); let which = it.next().unwrap_or("");
                if sub == "attach" || sub == "watch" {
                    match which.parse::<usize>().ok().and_then(|id| list.iter().find(|a| a.id == id).cloned()) {
                        Some(a) => { self.attached = Some(a.id); self.blocks.push(Block::System(format!("attached to sub-agent #{} {} — showing only its events; what you type is delivered to it; /agents detach to return", a.id, a.label))); }
                        None => self.blocks.push(Block::Error("usage: /agents attach <id>".into())),
                    }
                } else if sub == "detach" { self.attached = None; self.blocks.push(Block::System("detached".into())); }
                else if sub == "kill" || sub == "stop" {
                    let targets: Vec<_> = list.iter().filter(|a| a.running() && (which == "all" || which.parse::<usize>().ok() == Some(a.id))).cloned().collect();
                    if targets.is_empty() { self.blocks.push(Block::Error("usage: /agents kill <id|all>  (no matching running sub-agent)".into())); }
                    for a in targets { a.kill(); self.blocks.push(Block::System(format!("killing sub-agent #{} {}", a.id, a.label))); }
                } else {
                    let mut lines = vec![format!("Sub-agents ({} total, {} running) — /agents kill <id|all>", list.len(), list.iter().filter(|a| a.running()).count())];
                    for a in &list { let secs = a.finished.lock().unwrap().map(|f| f.duration_since(a.started)).unwrap_or_else(|| a.started.elapsed()).as_secs(); lines.push(format!("  #{:<2} {:<4} {:<9} {:>4}s  {:>3} tools  {}", a.id, a.label, truncate(&a.status.lock().unwrap(), 9), secs, a.tool_calls.load(std::sync::atomic::Ordering::Relaxed), truncate(&a.task, 70))); }
                    if list.is_empty() { lines.push("  none yet — the model delegates with spawn_agent {task, workdir?, read_only?}; several in one turn run in parallel (also under the Claude Code backend)".into()); }
                    self.blocks.push(Block::Banner(lines));
                }
            }
            "/rewind" | "/undo" => {
                if self.running.is_some() { self.set_status("finish or interrupt first"); }
                else {
                    let session = self.session.clone(); let tx = self.tx.clone(); let wd = self.workdir.clone();
                    // drop the transcript back to before the last user turn
                    let last_user_idx = self.blocks.iter().rposition(|b| matches!(b, Block::User(..)));
                    if let Some(i) = last_user_idx { self.blocks.truncate(i); }
                    tokio::spawn(async move {
                        let mut msgs = session.lock().await;
                        if let Some(i) = msgs.iter().rposition(|m| m.role == "user" && !m.text().starts_with("[harness]")) { msgs.truncate(i); }
                        let n = msgs.len();
                        drop(msgs);
                        let o = harness::sandbox::run_shell("git status --short | head -20", &wd, Duration::from_secs(10), 4000).await.map(|o| o.stdout).unwrap_or_default();
                        let _ = tx.send(Msg::Notice(format!("rewound the conversation to before the last turn ({n} messages kept). Files changed on disk are NOT reverted — review with /diff, undo with `git checkout -- <file>` or ask the agent.{}", if o.trim().is_empty() { String::new() } else { format!("\n{}", o.trim()) })));
                    });
                }
            }
            "/effort" => {
                let lvl = arg.trim().to_lowercase();
                if lvl.is_empty() { self.blocks.push(Block::System(format!("effort: {} (Claude Code backend) — /effort low|medium|high|xhigh|max", self.cfg.llm.effort.clone().unwrap_or("medium (default)".into())))); }
                else if !matches!(lvl.as_str(), "low" | "medium" | "high" | "xhigh" | "max") { self.blocks.push(Block::Error("usage: /effort low|medium|high|xhigh|max".into())); }
                else {
                    self.cfg.llm.effort = Some(lvl.clone());
                    if let Some(cc) = self.cc.take() { tokio::spawn(async move { cc.stop().await; }); }
                    self.blocks.push(Block::System(format!("effort → {lvl}{}", if self.cfg.llm.provider.as_deref() == Some("claude-code") { " (Claude session restarts on the next turn, resuming the conversation)" } else { " (applies when the Claude Code backend is used)" })));
                }
            }
            "/backend" | "/provider" => {
                let mut it = arg.split_whitespace(); let which = it.next().unwrap_or("").to_string(); let model = it.next().map(String::from); let effort = it.next().map(|e| e.to_lowercase());
                match which.as_str() {
                    "" => { self.blocks.push(Block::Banner(vec![format!("backend: {} · model {}", self.cfg.llm.provider.clone().unwrap_or("openai (local/compatible server)".into()), self.model), "switch: /backend local [model]  ·  /backend claude [model] [effort]   (claude = official Claude Code CLI on your subscription, default claude-fable-5; effort low|medium|high|max, also /effort)".into(), "        /backend anthropic <model>  (Anthropic API key from ANTHROPIC_API_KEY)".into()])); }
                    "local" | "lmstudio" => { self.cfg.llm.provider = None; if let Some(m) = model { self.cfg.llm.model = m; } self.model = self.cfg.llm.model.clone(); if let Some(cc) = self.cc.take() { tokio::spawn(async move { cc.stop().await; }); } self.cc_last_session = None; self.blocks.push(Block::System(format!("backend → local server {} · model {}", self.cfg.llm.base_url, self.model))); tokio::spawn(fetch_ctx_len(self.cfg.llm.base_url.clone(), self.model.clone(), self.tx.clone())); }
                    "claude" | "claude-code" | "cc" => { self.cfg.llm.provider = Some("claude-code".into()); self.cfg.llm.model = model.unwrap_or("claude-fable-5".into()); if let Some(e) = effort { if matches!(e.as_str(), "low" | "medium" | "high" | "xhigh" | "max") { self.cfg.llm.effort = Some(e); } } if self.cfg.llm.effort.is_none() { self.cfg.llm.effort = Some("medium".into()); } self.model = self.cfg.llm.model.clone(); if let Some(cc) = self.cc.take() { tokio::spawn(async move { cc.stop().await; }); } self.cc_last_session = None; self.metrics.ctx_len = 0; self.blocks.push(Block::System(format!("backend → Claude Code (subscription) · model {} · tools bridged over MCP · context window reported after the first turn", self.model))); }
                    "gemini" | "openai" | "openrouter" | "groq" | "mistral" | "deepseek" | "xai" | "together" => {
                        let (url, env, default_model) = match which.as_str() { "gemini" => ("https://generativelanguage.googleapis.com/v1beta/openai", "GEMINI_API_KEY", "gemini-2.5-pro"), "openai" => ("https://api.openai.com/v1", "OPENAI_API_KEY", "gpt-5"), "openrouter" => ("https://openrouter.ai/api/v1", "OPENROUTER_API_KEY", "anthropic/claude-sonnet-4.5"), "groq" => ("https://api.groq.com/openai/v1", "GROQ_API_KEY", "llama-3.3-70b-versatile"), "mistral" => ("https://api.mistral.ai/v1", "MISTRAL_API_KEY", "mistral-large-latest"), "deepseek" => ("https://api.deepseek.com/v1", "DEEPSEEK_API_KEY", "deepseek-chat"), "xai" => ("https://api.x.ai/v1", "XAI_API_KEY", "grok-4"), _ => ("https://api.together.xyz/v1", "TOGETHER_API_KEY", "meta-llama/Llama-3.3-70B-Instruct-Turbo") };
                        self.cfg.llm.provider = Some("openai".into()); self.cfg.llm.base_url = url.into(); self.cfg.llm.model = model.unwrap_or(default_model.into()); self.model = self.cfg.llm.model.clone();
                        match std::env::var(env) { Ok(k) if !k.is_empty() => { self.cfg.llm.api_key = Some(k); self.blocks.push(Block::System(format!("backend → {which} ({url}) · model {} · key from ${env}", self.model))); } _ => { self.blocks.push(Block::Error(format!("backend → {which} set, but ${env} is not set — export it and re-run /backend {which}"))); } }
                        if let Some(cc) = self.cc.take() { tokio::spawn(async move { cc.stop().await; }); }
                        self.metrics.ctx_len = 0; tokio::spawn(fetch_ctx_len(self.cfg.llm.base_url.clone(), self.model.clone(), self.tx.clone()));
                    }
                    "anthropic" => { self.cfg.llm.provider = Some("anthropic".into()); self.cfg.llm.base_url = "https://api.anthropic.com".into(); if let Some(m) = model { self.cfg.llm.model = m; } self.model = self.cfg.llm.model.clone(); self.metrics.ctx_len = 200_000; if self.cfg.llm.thinking_budget.is_none() { self.cfg.llm.thinking_budget = Some(8000); } self.blocks.push(Block::System(format!("backend → Anthropic API · model {}", self.model))); }
                    other => self.blocks.push(Block::Error(format!("unknown backend '{other}' (local | claude | anthropic)"))),
                }
            }
            "/queue" => {
                if self.queued.is_empty() { self.blocks.push(Block::System("queue is empty".into())); }
                else { let mut lines = vec![format!("Queued tasks ({}) — /next skips the current one, /queue clear empties the queue", self.queued.len())]; for (i, q) in self.queued.iter().enumerate() { lines.push(format!("  {}. {}", i + 1, truncate(q, 120))); } self.blocks.push(Block::Banner(lines)); }
                if arg == "clear" { self.queued.clear(); self.blocks.push(Block::System("queue cleared".into())); }
            }
            "/next" | "/skip" => self.next_task(),
            "/exit" | "/quit" | "/q" => self.quit = true,
            "/restart" => {
                if self.running.is_some() { self.set_status("finish or interrupt the current task first (esc)"); }
                else { self.restart = true; self.quit = true; }
            }
            "/improve" => self.start_improve(arg.clone()),
            "/cancel" => {
                if self.restart_at.is_some() { self.cancel_restart(); }
                else if let Some(h) = self.improve.take() { self.improve_cancel.store(true, std::sync::atomic::Ordering::Relaxed); h.abort(); self.blocks.push(Block::System("⚙ self-improvement cancelled (branches/worktrees under $TMPDIR/harness-proposals are kept)".into())); }
                else { self.set_status("nothing to cancel"); }
            }
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
            "update" if rest == "all" || rest == "--all" => { let tx = self.tx.clone(); tokio::spawn(async move { if let Ok(p) = harness::plugins::Plugins::open() { for (n, r) in p.update_all().await { let _ = tx.send(Msg::Notice(format!("plugin {n}: {}", match r { Ok(m) => m, Err(e) => format!("failed: {e:#}") }))); } } }); }
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
        if self.cfg.ui.fold_previous { self.fold_previous(); }
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
        let extra_roots = self.extra_roots.clone();
        if self.session_meta.id.is_empty() { self.session_meta.id = harness::sessions::SessionStore::new_id(); }
        let session_id = self.session_meta.id.clone();
        let inbox = self.inbox.clone();
        let cwd = self.wt_cwd.clone();
        let perm_mode = self.perm_mode;
        let cc_existing = self.cc.clone(); let cc_resume = self.cc_last_session.clone();
        let cc_images: Vec<(String, String)> = atts.iter().map(|a| (a.mime.clone(), a.b64.clone())).collect();
        let user_text = text.clone();
        let budget = self.cfg.llm.effective_budget(if self.metrics.ctx_len > 0 { Some(self.metrics.ctx_len) } else { None });
        self.run_started = Instant::now();
        self.metrics.turn_start = Some(Instant::now()); self.metrics.live_peak = 0.0; self.metrics.live_chars.clear(); self.turn_tokens = 0;
        let handle = tokio::spawn(async move {
            let res: Result<(String, harness::agent::RunStats), String> = async {
                let client = Client::new(&cfg.llm).map_err(|e| e.to_string())?;
                let store = if cfg.memory.enabled { harness::memory::MemoryStore::open(&cfg.memory).ok() } else { None };
                let fallback = Registry::defaults(cfg.net.enabled);
                let (registry, extra_prompt): (&Registry, String) = match &toolset { Some(ts) => (&ts.registry, ts.prompt_extra.clone()), None => (&fallback, String::new()) };
                let sink: Arc<dyn Sink> = Arc::new(TuiSink(tx.clone()));
                let mut pcfg = cfg.permissions.clone(); pcfg.mode = perm_mode; pcfg.allow.extend(harness::permissions::persisted_rules());
                let policy = Arc::new(harness::permissions::Policy::new(pcfg, &workdir));
                let _ = tx.send(Msg::Policy(policy.clone()));
                let approver: Arc<dyn harness::permissions::Approver> = Arc::new(TuiApprover(tx.clone()));
                let mut env_ = harness::agent::SubAgentEnv::new(client.clone(), registry.clone(), policy.clone(), approver.clone(), sink.clone(), budget, true); env_.cc_effort = cfg.llm.effort.clone(); let env = Arc::new(env_); let _ = tx.send(Msg::SubEnv(env.clone()));
                let ctx = ToolCtx { workdir: workdir.clone(), timeout: Duration::from_secs(cfg.agent.tool_timeout_secs), max_output: cfg.agent.max_tool_output_chars, net: cfg.net.clone(), memory: store.clone(), subagent: Some(env), redact_secrets: cfg.security.redact_secrets, hooks: cfg.hooks.clone(), todos: todos.clone(), lsp_servers: cfg.lsp.servers.clone(), extra_roots: extra_roots.clone(), approver: Some(approver.clone()), inbox: inbox.clone(), cancel: None, cwd: Some(cwd.clone()), session_id: Some(session_id.clone()) };
                let agent = Agent { client: &client, registry, ctx: &ctx, max_turns: cfg.agent.max_turns, context_budget: budget, sink: sink.as_ref(), stream: true, policy: &policy, approver: approver.as_ref() };
                let extra = format!("You are in an interactive session: the user can see everything and will reply; keep final answers concise.{extra_prompt}");
                let system = harness::agent::system_prompt_with_memory(&workdir.display().to_string(), &registry.names(), Some(&extra), store.as_ref());
                let mut msgs = session.lock().await;
                if client.provider() == harness::llm::Provider::ClaudeCode {
                    // Claude Code backend: our tools bridged over MCP; the claude CLI drives the loop
                    let cc = match cc_existing { Some(c) => c, None => {
                        let host = Arc::new(harness::mcp_bridge::BridgeHost { registry: registry.clone(), ctx: ctx.clone(), policy: policy.clone(), approver: approver.clone(), sink: sink.clone() });
                        let c = harness::claude_code::ClaudeCodeSession::start_with(&workdir, Some(cfg.llm.model.as_str()), Some(cfg.llm.effort.as_deref().unwrap_or("medium")), &system, host, cc_resume.as_deref()).await.map_err(|e| format!("{e:#}"))?;
                        let _ = tx.send(Msg::CcSession(c.clone())); c } };
                    if msgs.is_empty() { msgs.push(Message::system(&system)); }
                    msgs.push(user_msg);
                    let (t, st) = cc.run_turn(&user_text, &cc_images, sink.as_ref()).await.map_err(|e| format!("{e:#}"))?;
                    msgs.push(Message { role: "assistant".into(), content: Some(Content::Text(t.clone())), ..Default::default() });
                    return Ok((t, st));
                }
                let out = agent.run_turn_message(&mut msgs, &system, user_msg).await.map_err(|e| format!("{e:#}"))?;
                Ok(out)
            }.await;
            let _ = tx.send(Msg::Done(res));
        });
        self.running = Some(handle);
    }

    /// Ask the aux model for a short session title after the first turn (local/API backends).
    fn spawn_title(&mut self) {
        if !self.session_meta.title.is_empty() && self.session_meta.title.len() < 40 && !self.session_meta.title.ends_with('…') { return; }
        if self.cfg.llm.provider.as_deref() == Some("claude-code") { return; }
        let session = self.session.clone(); let cfg = self.cfg.clone(); let tx = self.tx.clone();
        tokio::spawn(async move {
            let msgs = session.lock().await.clone();
            let first = msgs.iter().find(|m| m.role == "user").map(|m| m.text()).unwrap_or_default();
            if first.trim().is_empty() { return; }
            let Ok(client) = Client::new(&cfg.llm) else { return };
            let req = vec![Message::system("Reply with a 3–6 word title for a coding session that starts with the following request. Title only, no quotes, no trailing period."), Message::user(truncate(&first, 600))];
            if let Ok((r, _)) = client.aux().chat(&req, &[]).await { let t = r.text().lines().next().unwrap_or("").trim().trim_matches('"').to_string(); if (3..=80).contains(&t.len()) { let _ = tx.send(Msg::Title(t)); } }
        });
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

    // ───────────────────────── /improve (smart self-improvement) ─────────────────────────
    fn start_improve(&mut self, hint: String) {
        if self.improve.as_ref().map(|h| !h.is_finished()).unwrap_or(false) { self.set_status("an /improve job is already running — /cancel to stop it"); return; }
        if self.running.is_some() { self.set_status("finish or interrupt the current task first (esc)"); return; }
        let mut cfg = self.cfg.clone(); cfg.llm.model = self.model.clone(); cfg.permissions.mode = self.perm_mode;
        let smart = harness::selfimprove::is_smart(&cfg);
        let repo = match harness::selfimprove::locate_repo(&cfg) { Ok(r) => r, Err(e) => { self.blocks.push(Block::Error(format!("/improve: {e:#}"))); return; } };
        self.blocks.push(Block::Banner(vec![
            format!("⚙ self-improvement · source {} · backend {}{}", short_path(&repo), self.model, if self.cfg.llm.provider.as_deref() == Some("claude-code") { format!(" (effort {})", self.cfg.llm.effort.clone().unwrap_or("medium".into())) } else { String::new() }),
            if smart { "  gate 1: frontier backend → the plan is implemented automatically; you will be informed of what is planned".into() } else { "  gate 1: you will be asked to confirm the plan before anything is implemented".into() },
            format!("  then: proposal/* branch per item → arbiter ({} eval run/side{}) → merge → install → restart with {}s to cancel", cfg.selfimprove.arbiter_runs, if cfg.selfimprove.skip_arbiter { ", SKIPPED" } else { "" }, cfg.selfimprove.restart_grace_secs),
            if hint.is_empty() { "  focus: agent's choice (README roadmap, docs/GAPS.md, TODO.md, BRAIN lessons) · /cancel stops the job".into() } else { format!("  focus: {hint} · /cancel stops the job") },
        ]));
        if let Ok(exe) = harness::selfimprove::installed_exe() { if exe.starts_with(repo.join("target")) { self.blocks.push(Block::System("⚙ note: this harness runs from the repo's target/ dir, which the arbiter rebuilds — prefer the installed binary (cargo install --path .) for /improve".into())); } }
        let tx = self.tx.clone();
        let sink: Arc<dyn Sink> = Arc::new(harness::agent::PrefixSink { inner: Arc::new(TuiSink(tx.clone())), prefix: "⚙ ".into(), info: None });
        let approver: Arc<dyn harness::permissions::Approver> = Arc::new(TuiApprover(tx.clone()));
        let tx2 = tx.clone();
        let report: Arc<dyn Fn(harness::selfimprove::Stage) + Send + Sync> = Arc::new(move |s| { let _ = tx2.send(Msg::Improve(s)); });
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false)); self.improve_cancel = cancel.clone();
        let job = harness::selfimprove::Job { cfg, hint, sink, approver, report, cancel, assume_yes: false, no_install: false };
        let tx3 = tx.clone();
        self.improve = Some(tokio::spawn(async move {
            if let Err(e) = harness::selfimprove::run(job).await { let _ = tx3.send(Msg::Improve(harness::selfimprove::Stage::Failed(format!("{e:#}")))); }
        }));
    }
    fn on_improve(&mut self, st: harness::selfimprove::Stage) {
        use harness::selfimprove::Stage::*;
        match st {
            Log(l) => self.blocks.push(Block::System(format!("⚙ {l}"))),
            Plan { items, auto } => {
                let mut lines = vec![if auto { format!("⚙ improvements planned ({}) — will be implemented automatically (frontier backend); /cancel to stop", items.len()) } else { format!("⚙ improvements proposed ({}) — confirm below", items.len()) }];
                for (i, p) in items.iter().enumerate() { lines.push(format!("  {}. {} — {}", i + 1, p.title, p.rationale)); if !p.files.is_empty() { lines.push(format!("     files: {}", p.files.join(", "))); } }
                self.blocks.push(Block::Banner(lines));
                if auto && self.cfg.ui.notify { desktop_notify("Harness: improvements planned", &format!("{} improvement(s) will be implemented automatically", items.len())); }
            }
            Approved(v) => self.blocks.push(Block::System(format!("⚙ implementing {} item(s): {}", v.len(), v.iter().map(|p| p.title.clone()).collect::<Vec<_>>().join(" · ")))),
            Item { title, branch, merged, note } => self.blocks.push(if merged { Block::System(format!("⚙ ✓ {title} [{branch}] — {note}")) } else { Block::Error(format!("⚙ ✗ {title} [{branch}] — {note}")) }),
            Installed { summary, exe, grace_secs } => {
                self.blocks.push(Block::Banner(vec![format!("⚙ improved harness installed at {}", short_path(&exe)), format!("  {summary}"), format!("  restarting in {grace_secs}s — the session and the picked model/effort are restored · esc or /cancel keeps the current version until you /restart")]));
                self.restart_at = Some(Instant::now() + Duration::from_secs(grace_secs.max(5)));
                if self.cfg.ui.notify { desktop_notify("Harness: improvement installed", &format!("restarting in {grace_secs}s — esc or /cancel to keep the current version")); }
            }
            Done { summary } => { self.blocks.push(Block::System(format!("⚙ self-improvement finished: {summary}"))); self.improve = None; }
            Failed(e) => { self.blocks.push(Block::Error(format!("⚙ self-improvement failed: {e}"))); self.improve = None; }
        }
    }
    fn cancel_restart(&mut self) {
        self.restart_at = None;
        self.blocks.push(Block::System("↻ automatic restart cancelled — the improved binary is installed; /restart when convenient".into()));
    }

    /// Persist the transcript (called after every turn and on interrupt).
    fn save_session(&mut self) {
        if self.session_meta.id.is_empty() { self.session_meta.id = harness::sessions::SessionStore::new_id(); }
        self.session_meta.workdir = self.workdir.display().to_string();
        self.session_meta.model = self.model.clone();
        self.session_meta.provider = self.cfg.llm.provider.clone(); self.session_meta.effort = self.cfg.llm.effort.clone();
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
            else {
                // fuzzy: exact id, id prefix, or title/workdir substring (case-insensitive); ambiguous → show matches
                let all = store.list(None); let q = which.to_lowercase();
                let hits: Vec<&harness::sessions::Meta> = if let Some(m) = all.iter().find(|m| m.id == which) { vec![m] } else { all.iter().filter(|m| m.id.starts_with(which) || m.title.to_lowercase().contains(&q) || m.workdir.to_lowercase().contains(&q)).collect() };
                match hits.len() { 1 => hits[0].id.clone(), 0 => { self.blocks.push(Block::Error(format!("no session matches '{which}' — /sessions"))); return; } _ => { let mut lines = vec![format!("{} sessions match '{which}' — pick one: /resume <id>", hits.len())]; for m in hits.iter().take(15) { lines.push(format!("  {}  {:<50} {}", m.id, truncate(&m.title, 50), harness::sessions::fmt_age(m.updated))); } self.blocks.push(Block::Banner(lines)); return; } }
            };
        match store.load(&id) {
            Ok((meta, msgs)) => {
                self.blocks.clear(); self.banner();
                self.blocks.push(Block::System(format!("resumed session {} — {} · {} · {} turns", meta.id, meta.title, short_path(std::path::Path::new(&meta.workdir)), meta.turns)));
                self.replay(&msgs);
                if std::path::Path::new(&meta.workdir).is_dir() { self.workdir = PathBuf::from(&meta.workdir); let _ = std::env::set_current_dir(&self.workdir); }
                if !meta.model.is_empty() { self.model = meta.model.clone(); self.cfg.llm.model = meta.model.clone(); }
                if meta.provider.is_some() { self.cfg.llm.provider = meta.provider.clone().filter(|p| !p.is_empty()); }
                if let Some(e) = &meta.effort { if !e.is_empty() { self.cfg.llm.effort = Some(e.clone()); } }
                if let Some(cc) = self.cc.take() { tokio::spawn(async move { cc.stop().await; }); }
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
        if let Some(cc) = self.cc.take() { let tx = self.tx.clone(); tokio::spawn(async move { let sid = cc.session_id.lock().await.clone(); cc.stop().await; if let Some(s) = sid { let _ = tx.send(Msg::Notice(format!("claude session {s} stopped; next turn resumes it"))); } }); }
        if let Some(h) = self.running.take() {
            h.abort();
            self.save_session();
            for b in self.blocks.iter_mut().rev() {
                match b {
                    Block::Tool { result: None, interrupted, .. } => { *interrupted = true; }
                    Block::Assistant { streaming, .. } => { *streaming = false; }
                    Block::Reasoning { streaming, ended, .. } => { if ended.is_none() { *ended = Some(Instant::now()); } *streaming = false; }
                    _ => {}
                }
            }
            self.blocks.push(Block::System("interrupted".into()));
            self.set_status("Interrupted — the transcript is kept; type to continue");
        }
    }

    fn on_msg(&mut self, m: Msg) {
        match m {
            Msg::Block(b) => self.blocks.push(b),
            Msg::Improve(st) => self.on_improve(st),
            Msg::CcSession(s) => { self.cc = Some(s); }
            Msg::Policy(p) => { p.set_mode(self.perm_mode); self.live_policy = Some(p); }
            Msg::SubEnv(e) => { self.subenv = Some(e); }
            Msg::Title(t) => { self.session_meta.title = t; self.save_session(); }
            Msg::Question(q, tx) => {
                let mut lines = vec![format!("❓ {}", q.question)];
                for (i, o) in q.options.iter().enumerate() { lines.push(format!("   [{}] {}{}", i + 1, o.label, if o.description.is_empty() { String::new() } else { format!(" — {}", o.description) })); }
                lines.push(if q.allow_free_text { "   type an answer and press enter · number picks an option · esc declines".into() } else { "   press a number to choose · esc declines".into() });
                self.blocks.push(Block::Banner(lines));
                self.pending_q = Some((q, tx, String::new()));
            }
            Msg::CcSid(id) => { self.cc_last_session = Some(id); }
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
                if let Some(cc) = self.cc.clone() { let tx = self.tx.clone(); tokio::spawn(async move { if let Some(id) = cc.session_id.lock().await.clone() { let _ = tx.send(Msg::CcSid(id)); } }); }
                if self.cfg.ui.notify && self.run_started.elapsed() > Duration::from_secs(20) {
                    let title = match &res { Ok(_) => "Harness: task finished", Err(_) => "Harness: task stopped" };
                    let body = truncate(&self.blocks.iter().rev().find_map(|b| if let Block::User(t, _) = b { Some(t.clone()) } else { None }).unwrap_or_default(), 80).replace('"', "'");
                    let title = title.to_string();
                    { let h = self.cfg.hooks.clone(); let wd = self.workdir.clone(); let (t2, b2) = (title.clone(), body.clone()); if !h.notification.is_empty() { tokio::spawn(async move { let _ = harness::hooks::run_event(&h, "notification", &t2, serde_json::json!({"title": t2, "body": b2}), &wd).await; }); } }
                    tokio::spawn(async move {
                        if cfg!(target_os = "macos") { let script = format!("display notification \"{body}\" with title \"{title}\" sound name \"Glass\""); let _ = tokio::process::Command::new("osascript").arg("-e").arg(script).output().await; }
                        else if cfg!(target_os = "linux") { let _ = tokio::process::Command::new("notify-send").arg(&title).arg(&body).output().await; }
                        else if cfg!(windows) { let ps = format!("[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] > $null; $t=[Windows.UI.Notifications.ToastNotificationManager]::GetTemplateContent(1); $t.GetElementsByTagName('text')[0].AppendChild($t.CreateTextNode('{title}: {body}')) > $null; [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('harness').Show([Windows.UI.Notifications.ToastNotification]::new($t))"); let _ = tokio::process::Command::new("powershell").args(["-NoProfile", "-Command", &ps]).output().await; }
                    });
                }
                self.save_session();
                match res {
                    Ok((_, stats)) => { if self.history.len() <= 1 { self.spawn_title(); } self.spawn_reflection(&stats); }
                    Err(e) => { if !e.contains("interrupted") { self.blocks.push(Block::Error(e)); } }
                }
                if !self.queued.is_empty() {
                    let next = self.queued.remove(0);
                    self.set_status(format!("→ next task ({} left in queue)", self.queued.len()));
                    self.start_run(next);
                } else if let Some(m) = self.inbox.take_message() { self.set_status("inbox event → waking the agent"); self.start_run(m); }
            }
        }
    }

    fn on_event(&mut self, e: Event) {
        if let Some(f) = &mut self.event_log { if !matches!(e, Event::ReasoningDelta { .. } | Event::AssistantDelta { .. } | Event::Turn { .. }) { use std::io::Write; let _ = writeln!(f, "{}", serde_json::to_string(&e).unwrap_or_default()); } }
        match e {
            Event::RunStarted { model, workdir, .. } if workdir == "\u{0}models" => { self.models = model.split('\u{1f}').map(String::from).collect(); }
            Event::RunStarted { .. } | Event::Turn { .. } => {}
            Event::ModelResponse { prompt_tokens, completion_tokens, ttft_secs, secs, .. } => {
                self.metrics.on_call(prompt_tokens, completion_tokens, ttft_secs, secs);
                self.last_prompt_tokens = prompt_tokens;
                // live session totals (per call), so the panel updates while a task runs
                self.total_prompt += prompt_tokens; self.total_completion += completion_tokens; self.turn_tokens += completion_tokens;
                self.session_meta.prompt_tokens += prompt_tokens; self.session_meta.completion_tokens += completion_tokens;
            }
            Event::ReasoningDelta { text } => {
                self.metrics.on_delta(text.chars().count());
                if let Some(Block::Reasoning { text: t, streaming: true, .. }) = self.blocks.last_mut() { t.push_str(&text); }
                else { self.blocks.push(Block::Reasoning { text, streaming: true, show: None, started: Instant::now(), ended: None }); }
            }
            Event::RateLimit { status, kind, resets_at } => { if status != "allowed" { self.blocks.push(Block::Error(format!("Claude subscription rate limit: {status} ({kind}); resets in {}m", resets_at.saturating_sub(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)) / 60))); } self.cc_rate = Some((status, kind, resets_at)); }
            Event::ContextInfo { window, source } => { if window > 0 && window != self.metrics.ctx_len { self.metrics.ctx_len = window; self.blocks.push(Block::System(format!("context window: {} tokens ({source}) · auto-compaction (local backends) at {}", fmt_k(window), fmt_k(self.cfg.llm.effective_budget(Some(window)))))); } }
            Event::ThinkingStatus { est_tokens, done } => {
                let label = |n: u64, d: bool| if d { format!("(reasoning hidden by the provider — ~{} tokens)", fmt_k(n)) } else { format!("(reasoning hidden by the provider — thinking… ~{} tokens so far)", fmt_k(n)) };
                if let Some(Block::Reasoning { text: t, streaming, ended, .. }) = self.blocks.last_mut() { *t = label(est_tokens, done); if done { *streaming = false; if ended.is_none() { *ended = Some(Instant::now()); } } }
                else { self.blocks.push(Block::Reasoning { text: label(est_tokens, done), streaming: !done, show: None, started: Instant::now(), ended: if done { Some(Instant::now()) } else { None } }); }
                self.metrics.on_delta(40);
            }
            Event::Reasoning { text } => {
                if let Some(Block::Reasoning { text: t, streaming, ended, .. }) = self.blocks.last_mut() { *t = text; *streaming = false; *ended = Some(Instant::now()); }
                else if !text.trim().is_empty() { let now = Instant::now(); self.blocks.push(Block::Reasoning { text, streaming: false, show: None, started: now, ended: Some(now) }); }
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
            Event::CompactProgress { fraction, phase } => { self.compact_progress = if fraction >= 1.0 { None } else { Some((fraction, phase, Instant::now())) }; }
            Event::Compacted { count, prompt_tokens, summary, map_before, map_after } => {
                self.compact_progress = None;
                let (tb, ta): (u64, u64) = (map_before.iter().map(|x| x.1).sum(), map_after.iter().map(|x| x.1).sum());
                let pct = if tb > 0 { 100.0 - (ta as f64) * 100.0 / (tb as f64) } else { 0.0 };
                if count == 0 { self.blocks.push(Block::System(format!("⟲ Claude Code compacted its context · {} → {} tokens ({}{:.0}%)", fmt_k(tb), fmt_k(ta), if pct >= 0.0 { "−" } else { "+" }, pct.abs()))); }
                else { self.blocks.push(Block::System(format!("⟲ context compacted: {count} messages → handoff note · ~{} → ~{} tokens ({}{:.0}%){}", fmt_k(tb), fmt_k(ta), if pct >= 0.0 { "−" } else { "+" }, pct.abs(), if prompt_tokens > 0 { format!(" · measured prompt was {}", fmt_k(prompt_tokens)) } else { String::new() }))); }
                if count == 0 { self.last_prompt_tokens = ta; }
                self.blocks.push(Block::CompactMap { before: map_before, after: map_after });
                if !summary.is_empty() { self.blocks.push(Block::Assistant { text: format!("Handoff note (context compaction)\n{summary}"), streaming: false, folded: true }); }
            }
            Event::RunFinished { stop_reason, turns, tool_calls, prompt_tokens, completion_tokens, wall_secs } => {
                self.finish_streaming();
                let _ = (prompt_tokens, completion_tokens); // already accumulated per model call
                let s = format!("{} · {} model call{} · {} tool call{} · {}+{} tokens · {:.0}s", if stop_reason == "done" { "done" } else { &stop_reason }, turns, if turns == 1 { "" } else { "s" }, tool_calls, if tool_calls == 1 { "" } else { "s" }, fmt_k(prompt_tokens), fmt_k(completion_tokens), wall_secs);
                self.blocks.push(Block::Finished(s));
            }
            Event::Error { message } => { self.finish_streaming(); self.blocks.push(Block::Error(message)); }
            Event::Memory { file, section, text } => { self.blocks.push(Block::Memory(format!("{} › {section}: {text}", file.trim_end_matches(".md")))); }
            Event::Permission { tool, summary, decision } => { if decision.starts_with("denied") { self.blocks.push(Block::Error(format!("🔒 {tool}({}) {decision}", truncate(&summary, 80)))); } }
        }
    }
    /// Click on a block: fold/unfold it (a summarized tool burst opens/closes as a group).
    fn toggle_fold(&mut self, idx: usize) {
        if matches!(self.blocks.get(idx), Some(Block::Tool { .. })) && self.tool_view != "full" {
            let mut start = idx; while start > 0 && matches!(self.blocks.get(start - 1), Some(Block::Tool { .. })) { start -= 1; }
            let mut end = idx; while matches!(self.blocks.get(end + 1), Some(Block::Tool { .. })) { end += 1; }
            if self.tool_groups_open.contains(&start) { for k in start..=end { self.tool_groups_open.remove(&k); } } else { for k in start..=end { self.tool_groups_open.insert(k); } }
            return;
        }
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
            match b { Block::Assistant { streaming, .. } => *streaming = false, Block::Reasoning { streaming, ended, .. } => { if *streaming && ended.is_none() { *ended = Some(Instant::now()); } *streaming = false; } _ => {} }
        }
    }
}

const COMMANDS: &[(&str, &str)] = &[
    ("/help", "show commands and keys"),
    ("/clear", "start a new session (forget the transcript)"),
    ("/sessions", "list saved sessions · /sessions live = other running sessions"),
    ("/msg", "message another live session: /msg <id|prefix|title|all> <text>"),
    ("/resume", "resume a saved session: /resume <n|id|last>"),
    ("/model", "show or switch the model: /model <name>"),
    ("/backend", "switch backend: local (LM Studio etc.) | claude [model] [effort] (Claude Code CLI, subscription) | anthropic <model>"),
    ("/effort", "Claude Code backend reasoning effort: /effort low|medium|high|xhigh|max (default medium)"),
    ("/cd", "change working directory"),
    ("/pwd", "print working directory"),
    ("/tools", "list the tools the model can call"),
    ("/net", "internet tools on|off"),
    ("/thinking", "toggle showing the model's reasoning"),
    ("/expand", "toggle expanded tool output (ctrl+o)"),
    ("/panel", "toggle the dashboard panel (ctrl+p)"),
    ("/cost", "token usage for this session"),
    ("/usage", "Claude backend: subscription usage (proxied Claude Code /usage); otherwise same as /cost"),
    ("/compact", "compact the context into a precise handoff note: /compact [focus]"),
    ("/context", "context map: what fills the window (prompt, tools, memory, messages) + heaviest items"),
    ("/settings", "interactive settings panel (also /config)"),
    ("/memory", "show MEMORY.md (settings · preferences · ideas)"),
    ("/brain", "show BRAIN.md (what the agent learned)"),
    ("/workflows", "show WORKFLOWS.md (recipes)"),
    ("/remember", "add a note: /remember <text> | brain: <text> | workflows: <text>"),
    ("/reflect", "ask the model what to remember from this session"),
    ("/video", "open the frame scrubber for a video: /video <path>"),
    ("/plugin", "plugins: list · install <owner/repo> · enable|disable|remove|update|info <name>"),
    ("/mcp", "show configured MCP servers and live MCP tools"),
    ("/reload", "restart tools, MCP servers and plugins"),
    ("/restart", "restart the harness (re-exec the installed binary) and resume this session"),
    ("/improve", "self-improvement loop: /improve [focus] — propose → confirm (auto with a frontier backend) → implement → arbiter → merge → install → restart (60s to cancel)"),
    ("/cancel", "cancel the pending automatic restart or the running /improve job"),
    ("/permissions", "show or set permission mode: bypass|auto|ask|plan"),
    ("/plan", "toggle plan mode (read-only)"),
    ("/trust", "remember this directory as trusted (no first-time notice)"),
    ("/theme", "switch theme: /theme light|dark"),
    ("/vim", "toggle vim-style modal editing in the prompt"),
    ("/workflow", "run a workflow: /workflow <name> [args]  (list with /workflow)"),
    ("/queue", "show queued tasks (/queue clear)"),
    ("/next", "stop the current task and start the next queued one (ctrl+n)"),
    ("/status", "backend, context, session, permissions at a glance"),
    ("/doctor", "check external tools, claude CLI, config paths"),
    ("/init", "have the agent write HARNESS.md project instructions"),
    ("/add-dir", "allow file tools to access another directory this session"),
    ("/rename", "rename the current session"),
    ("/export", "export the transcript to markdown (~/.config/harness/exports)"),
    ("/todos", "show the agent's todo list"),
    ("/hooks", "show configured hooks"),
    ("/skills", "list installed skills"),
    ("/diff", "git status + diff stat of the working tree"),
    ("/copy", "copy the last answer to the clipboard"),
    ("/review", "run the review workflow on the working-tree diff"),
    ("/pr-comments", "show PR comments via gh: /pr-comments [number]"),
    ("/rewind", "drop the last turn from the conversation (files not reverted)"),
    ("/release-notes", "recent commits"),
    ("/agents", "sub-agents: list · attach <id> (watch + message it) · detach · kill <id|all>"),
    ("/keybindings", "list keyboard shortcuts"),
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
    let width = (area.width as usize).saturating_sub(1).max(10); // 1 column reserved for the scrollbar
    // input geometry
    let input_lines = wrap_input(&app.input, width.saturating_sub(2).max(1));
    let sugg = suggestions(&app.input);
    let input_h = (input_lines.len().clamp(1, 8) + sugg.len() + if app.attachments.is_empty() { 0 } else { 1 }) as u16;
    // notice line above the box: spinner while running, or a transient status message
    let notice: Option<Vec<Span>> = if let Some((q, _, buf)) = &app.pending_q {
        Some(vec![Span::styled(" ❓ ", Style::default().fg(Color::Black).bg(pal().cyan)), Span::styled(format!(" {} ", truncate(&q.question, width.saturating_sub(60))), Style::default().fg(Color::Black).bg(pal().cyan).bold()),
                  Span::styled(format!("  {}", if q.options.is_empty() { String::new() } else { format!("[1-{}] choose · ", q.options.len()) }), Style::default().fg(pal().cyan)),
                  Span::styled(if q.allow_free_text { format!("type + enter: {buf}▏") } else { String::new() }, Style::default().fg(pal().fg)), Span::styled("  esc declines", Style::default().fg(pal().dim))])
    } else if let Some((req, _)) = &app.pending_ask {
        Some(vec![Span::styled(" 🔒 ", Style::default().fg(Color::Black).bg(pal().orange)), Span::styled(format!(" {}({}) ", req.tool, truncate(&req.summary, 240)), Style::default().fg(Color::Black).bg(pal().orange).bold()),
                  Span::styled(format!("  {} · ", req.reason), Style::default().fg(pal().orange)),
                  Span::styled("[y] once  ", Style::default().fg(pal().ok).bold()), Span::styled(format!("[a] always ({})  ", req.suggested_rule), Style::default().fg(pal().cyan)), Span::styled("[p] always in this project  ", Style::default().fg(pal().cyan)), Span::styled("[n] deny", Style::default().fg(pal().err).bold())])
    } else if let Some(at) = app.restart_at {
        let left = at.saturating_duration_since(Instant::now()).as_secs();
        Some(vec![Span::styled(format!("↻ improved harness installed — restarting in {left}s and resuming this session  "), Style::default().fg(pal().ok).bold()), Span::styled(if app.running.is_some() { "(waits for the current task) · esc or /cancel to keep the running version".to_string() } else { "esc or /cancel to keep the running version".to_string() }, Style::default().fg(pal().dim))])
    } else if let Some((f, phase, _)) = &app.compact_progress {
        let barw = 30usize; let filled = ((f * barw as f64) as usize).min(barw);
        Some(vec![Span::styled("⟲ compacting context ", Style::default().fg(pal().orange)), Span::styled("█".repeat(filled), Style::default().fg(pal().orange)), Span::styled("░".repeat(barw - filled), Style::default().fg(pal().dim)), Span::styled(format!(" {:>3.0}%  {phase}", f * 100.0), Style::default().fg(pal().dim))])
    } else if app.running.is_some() {
        let sp = SPINNER[(app.tick as usize / 2) % SPINNER.len()];
        let el = app.run_started.elapsed().as_secs();
        let live = app.metrics.live_tps();
        Some(vec![Span::styled(format!("{sp} {}… ", WORDS[app.word]), Style::default().fg(pal().orange)),
                  Span::styled(format!("({el}s · {} tok/s · esc to interrupt{})", if live > 0.0 { format!("{live:.0}") } else { "–".into() }, if app.queued.is_empty() { String::new() } else { format!(" · {} queued", app.queued.len()) }), Style::default().fg(pal().dim))])
    } else if let Some((m, t)) = &app.status_msg { if t.elapsed() < Duration::from_secs(4) { Some(vec![Span::styled(format!("· {m}"), Style::default().fg(pal().orange))]) } else { None } } else { None };
    // wrap the notice into up to 4 lines
    let notice_lines: Vec<Line> = match &notice { Some(spans) => { let mut v = Vec::new(); push_wrapped(&mut v, spans.clone(), width.max(10), 4); v.truncate(4); v } None => vec![] };
    let notice_h = notice_lines.len() as u16;
    let chunks = Layout::vertical([Constraint::Min(1), Constraint::Length(notice_h), Constraint::Length(1), Constraint::Length(input_h), Constraint::Length(1), Constraint::Length(1)]).split(area);
    let (tr_area, no_area, top_area, in_area, bot_area, st_area) = (chunks[0], chunks[1], chunks[2], chunks[3], chunks[4], chunks[5]);

    if app.video.is_some() { draw_video(f, app, tr_area); }
    if app.settings_open { draw_settings(f, app, tr_area); }
    // transcript
    let mut lines: Vec<Line> = Vec::new();
    let mut ph: Vec<Placeholder> = Vec::new();
    let mut line_map: Vec<(usize, usize, usize)> = Vec::new();
    let attach_prefix: Option<String> = app.attached.and_then(|id| app.subenv.as_ref().and_then(|e| e.list().into_iter().find(|a| a.id == id)).map(|a| format!("{} ", a.label)));
    if let Some(pfx) = &attach_prefix { lines.push(Line::from(vec![Span::styled(format!(" attached to sub-agent {} ", pfx.trim()), Style::default().fg(Color::Black).bg(pal().orange).bold()), Span::styled("  its tool calls and report below · type to message it · /agents detach", Style::default().fg(pal().dim))])); }
    let mut i = 0usize;
    while i < app.blocks.len() {
        let b = &app.blocks[i];
        if let Some(pfx) = &attach_prefix { let keep = match b { Block::Tool { name, .. } => name.starts_with(pfx.as_str()), Block::Error(t) | Block::System(t) => t.contains(pfx.trim()), _ => false }; if !keep { i += 1; continue; } }
        // tool bursts: consecutive Tool blocks collapse to one line in summary mode (hidden: nothing, except pending/error)
        if matches!(b, Block::Tool { .. }) && app.tool_view != "full" && attach_prefix.is_none() && !app.tool_groups_open.contains(&i) {
            let start_i = i; let mut j = i; let mut names: Vec<String> = Vec::new(); let mut pending = 0; let mut errors = 0; let mut secs = 0.0;
            while j < app.blocks.len() { if let Block::Tool { name, result, secs: sc, .. } = &app.blocks[j] { names.push(name.split_whitespace().last().unwrap_or(name).to_string()); if result.is_none() { pending += 1; } else if result.as_ref().map(|r| r.starts_with("error:")).unwrap_or(false) { errors += 1; } secs += sc; j += 1; } else { break; } }
            let a = lines.len();
            if app.tool_view == "summary" || pending > 0 || errors > 0 {
                let mut counts: Vec<(String, usize)> = Vec::new(); for n in &names { if let Some(c) = counts.iter_mut().find(|(k, _)| k == n) { c.1 += 1; } else { counts.push((n.clone(), 1)); } }
                let desc = counts.iter().map(|(k, c)| if *c > 1 { format!("{k}×{c}") } else { k.clone() }).collect::<Vec<_>>().join(", ");
                let bullet_style = if pending > 0 { Style::default().fg(if (app.tick / 4) % 2 == 0 { pal().orange } else { pal().dim }) } else if errors > 0 { Style::default().fg(pal().err) } else { Style::default().fg(pal().ok) };
                let mut spans = vec![Span::styled("⚙ ", bullet_style), Span::styled(format!("{} tool call{}", names.len(), if names.len() == 1 { "" } else { "s" }), Style::default().fg(pal().dim).bold()), Span::styled(format!(" · {}", truncate(&desc, width.saturating_sub(40))), Style::default().fg(pal().dim))];
                if pending > 0 { spans.push(Span::styled(" · running…", Style::default().fg(pal().orange))); } else { spans.push(Span::styled(format!(" · {:.1}s", secs), Style::default().fg(pal().dim))); }
                if errors > 0 { spans.push(Span::styled(format!(" · {errors} error{}", if errors == 1 { "" } else { "s" }), Style::default().fg(pal().err))); }
                spans.push(Span::styled("  (click to expand)", Style::default().fg(pal().dim).italic()));
                push_wrapped(&mut lines, spans, width, 2);
                out_line_pad(&mut lines);
            }
            // every block of the burst maps to the summary line so a click expands the group
            for k in start_i..j { line_map.push((a, lines.len(), k)); }
            i = j; continue;
        }
        let a = lines.len(); render_block(b, app, width, &mut lines, &mut ph); line_map.push((a, lines.len(), i));
        i += 1;
    }
    let total = lines.len();
    let h = tr_area.height as usize;
    let max_up = total.saturating_sub(h);
    if app.scroll_up > max_up { app.scroll_up = max_up; }
    let start = max_up - app.scroll_up;
    app.line_map = line_map; app.tr_rect = tr_area; app.tr_start = start;
    app.panel_rect = panel_area.map(|(_, pa)| pa).unwrap_or_default();
    let visible: Vec<Line> = lines.into_iter().skip(start).take(h).collect();
    if app.video.is_none() && !app.settings_open { f.render_widget(Paragraph::new(visible), tr_area); }
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
    if total > h && tr_area.width > 2 {
        use ratatui::widgets::{Scrollbar, ScrollbarOrientation, ScrollbarState};
        let mut st = ScrollbarState::new(max_up).position(start);
        f.render_stateful_widget(Scrollbar::new(ScrollbarOrientation::VerticalRight).begin_symbol(None).end_symbol(None).track_symbol(Some("│")).thumb_symbol("█").track_style(Style::default().fg(pal().panel_bg)).thumb_style(Style::default().fg(pal().dim)), tr_area, &mut st);
    }
    if app.scroll_up > 0 {
        let tag = format!(" ↓ {} more lines ", app.scroll_up);
        let r = Rect { x: area.x + area.width.saturating_sub(tag.len() as u16 + 1), y: tr_area.bottom().saturating_sub(1), width: tag.len() as u16, height: 1 };
        f.render_widget(Paragraph::new(Span::styled(tag, Style::default().fg(Color::Black).bg(pal().orange))), r);
    }
    if !notice_lines.is_empty() { f.render_widget(Paragraph::new(notice_lines), no_area); }

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
        Span::styled(format!("{}{}", app.model, if app.cfg.llm.provider.as_deref() == Some("claude-code") { app.cfg.llm.effort.as_ref().map(|e| format!(" · effort {e}")).unwrap_or_default() } else { String::new() }), Style::default().fg(pal().cyan)), dot(),
        Span::styled(short_path(&app.workdir), Style::default().fg(pal().cyan)), dot(),
        Span::styled(format!("ctx {}", fmt_k(app.last_prompt_tokens)), Style::default().fg(pal().cyan))];
    if !app.net { st.push(dot()); st.push(Span::styled("offline", Style::default().fg(pal().pink))); }
    if !app.queued.is_empty() { st.push(dot()); st.push(Span::styled(format!("{} queued", app.queued.len()), Style::default().fg(pal().cyan))); }
    if let Some(id) = app.attached { st.push(dot()); st.push(Span::styled(format!("attached #{id}"), Style::default().fg(pal().orange))); }
    if app.vim { st.push(dot()); st.push(Span::styled(if app.vim_normal { "-- NORMAL --" } else { "-- INSERT --" }, Style::default().fg(if app.vim_normal { pal().orange } else { pal().ok }).bold())); }
    if let Some(wt) = app.wt_cwd.lock().unwrap().as_ref() { st.push(dot()); st.push(Span::styled(format!("worktree {}", wt.name), Style::default().fg(pal().orange))); }
    let lw: usize = st.iter().map(|s| s.content.chars().count()).sum();
    let right = if app.running.is_none() { "? for shortcuts · /help" } else { "esc to interrupt" };
    let pad = width.saturating_sub(lw + right.chars().count() + 1);
    st.push(Span::raw(" ".repeat(pad))); st.push(Span::styled(right, Style::default().fg(pal().dim)));
    f.render_widget(Paragraph::new(Line::from(st)), st_area);
}

// ───────────────────────── settings panel ─────────────────────────
fn draw_settings(f: &mut Frame, app: &mut App, area: Rect) {
    f.render_widget(ratatui::widgets::Clear, area);
    let dim = Style::default().fg(pal().dim);
    let mut lines: Vec<Line> = vec![Line::from(Span::styled(" Settings  ·  ↑/↓ select · ←/→ or enter change · esc close  ·  saved to ~/.config/harness/settings.toml ", Style::default().fg(Color::Black).bg(pal().orange).bold())), Line::raw("")];
    for (i, (key, label, choices, help)) in SETTINGS.iter().enumerate() {
        let sel = i == app.settings_cursor;
        let val = app.setting_value(key);
        let mut spans = vec![Span::styled(if sel { " ▸ " } else { "   " }, Style::default().fg(pal().orange)), Span::styled(format!("{:<26}", label), if sel { Style::default().bold() } else { Style::default() })];
        if choices.is_empty() { spans.push(Span::styled(truncate(&val, 50), dim)); }
        else { for c in choices.iter() { let on = **c == val; spans.push(Span::styled(format!(" {} ", c), if on { Style::default().fg(Color::Black).bg(if sel { pal().orange } else { pal().dim }).bold() } else { dim })); spans.push(Span::raw(" ")); } }
        lines.push(Line::from(spans));
        if sel && !help.is_empty() { lines.push(Line::from(vec![Span::raw("     "), Span::styled(help.to_string(), dim.italic())])); }
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(format!("   config file: {}   ·   {}", short_path(&harness::setup::config_dir().join("harness.toml")), harness::version()), dim)));
    f.render_widget(Paragraph::new(lines), area);
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
    let agents: Vec<Arc<harness::agent::SubAgentInfo>> = app.subenv.as_ref().map(|e| e.list()).unwrap_or_default();
    let agents_show: Vec<&Arc<harness::agent::SubAgentInfo>> = agents.iter().filter(|a| a.running() || a.finished.lock().unwrap().map(|f| f.elapsed().as_secs() < 120).unwrap_or(false)).collect();
    let agents_h = if agents_show.is_empty() { 0 } else { (agents_show.len() as u16).min(6) + 1 };
    let rows = Layout::vertical([
        Constraint::Length(1), Constraint::Min(6),          // thinking
        Constraint::Length(todo_h),                         // tasks
        Constraint::Length(agents_h),                       // sub-agents
        Constraint::Length(1), Constraint::Length(6),       // tokens
        Constraint::Length(1), Constraint::Length(8),       // speed
        Constraint::Length(1), Constraint::Length(9),       // system
    ]).split(area);
    let (r_tokens_t, r_tokens, r_speed_t, r_speed, r_sys_t, r_sys) = (rows[4], rows[5], rows[6], rows[7], rows[8], rows[9]);
    if agents_h > 0 {
        let running = agents_show.iter().filter(|a| a.running()).count();
        let mut al: Vec<Line> = vec![title(&format!("Agents · {} running", running))];
        for a in agents_show.iter().take(6) { let secs = a.finished.lock().unwrap().map(|f| f.duration_since(a.started)).unwrap_or_else(|| a.started.elapsed()).as_secs(); let st = a.status.lock().unwrap().clone(); let run = a.running(); al.push(Line::from(vec![Span::styled(format!("{} ", if run { "▶" } else { "☑" }), Style::default().fg(if run { pal().orange } else { pal().ok })), Span::styled(format!("{:<3}", a.label), Style::default().bold()), Span::styled(format!(" {:>3}s {:>2}t ", secs, a.tool_calls.load(std::sync::atomic::Ordering::Relaxed)), dim), Span::styled(truncate(&if run { st } else { format!("{st}") }, area.width.saturating_sub(16) as usize), if run { Style::default() } else { dim })])); }
        f.render_widget(Paragraph::new(al), rows[3]);
    }
    if todo_h > 0 {
        let done = todos.iter().filter(|t| t.status == "done").count();
        let mut tl: Vec<Line> = vec![title(&format!("Tasks {}/{}", done, todos.len()))];
        for t in todos.iter().take(8) { let (mark, st) = match t.status.as_str() { "done" => ("☑ ", Style::default().fg(pal().ok)), "in_progress" => ("▶ ", Style::default().fg(pal().orange).bold()), _ => ("☐ ", dim) }; let mut line = Line::from(vec![Span::styled(mark, st), Span::styled(truncate(&t.text, area.width.saturating_sub(4) as usize), if t.status == "done" { dim } else { Style::default() })]); let blk = t.open_blockers(&todos); if !blk.is_empty() { line.spans.push(Span::styled(format!(" ⏳{}", blk.iter().map(|b| format!("#{b}")).collect::<Vec<_>>().join(",")), dim)); } if !t.owner.is_empty() { line.spans.push(Span::styled(format!(" @{}", t.owner), dim)); } tl.push(line); }
        f.render_widget(Paragraph::new(tl), rows[2]);
    }

    // ── Thinking ──
    f.render_widget(Paragraph::new(title(&format!("{}{}", if running { "Thinking · live" } else { "Thinking · last" }, if app.think_scroll > 0 { " ↑" } else { "" }))), rows[0]);
    let think = app.blocks.iter().rev().find_map(|b| if let Block::Reasoning { text, .. } = b { Some(text.clone()) } else { None }).unwrap_or_default();
    let tw = (rows[1].width as usize).saturating_sub(1).max(10);
    let mut tl: Vec<Line> = Vec::new();
    for l in think.lines().filter(|l| !l.trim().is_empty()) { push_wrapped(&mut tl, vec![Span::styled(l.trim().to_string(), Style::default().fg(pal().think))], tw, 0); }
    let th = rows[1].height as usize;
    let max_up = tl.len().saturating_sub(th);
    let up = app.think_scroll.min(max_up);
    let skip = max_up - up;
    let tail: Vec<Line> = tl.into_iter().skip(skip).take(th).collect();
    if tail.is_empty() { f.render_widget(Paragraph::new(Span::styled("(reasoning will stream here)", dim)), rows[1]); } else {
        f.render_widget(Paragraph::new(tail), rows[1]);
        if max_up > 0 { use ratatui::widgets::{Scrollbar, ScrollbarOrientation, ScrollbarState}; let mut st = ScrollbarState::new(max_up).position(skip); f.render_stateful_widget(Scrollbar::new(ScrollbarOrientation::VerticalRight).begin_symbol(None).end_symbol(None).track_symbol(Some("│")).thumb_symbol("█").track_style(Style::default().fg(pal().panel_bg)).thumb_style(Style::default().fg(pal().dim)), rows[1], &mut st); }
    }

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
    let temp_col = |t: f32| if t >= 90.0 { pal().err } else if t >= 75.0 { pal().orange } else { pal().fg };
    let mut cpu_line = vec![Span::styled("cpu ", dim), Span::styled(format!("{cpu:>5.1}%"), Style::default().fg(if cpu > 80.0 { pal().err } else { pal().fg }))];
    match last.cpu_temp { Some(t) => { cpu_line.push(Span::styled(format!("  {t:.0}°C"), Style::default().fg(temp_col(t)))); if let Some(pw) = last.cpu_power { cpu_line.push(Span::styled(format!(" {pw:.1}W"), dim)); } }
                          None => cpu_line.push(Span::styled("  (temp: brew install macmon)", dim)) }
    cpu_line.push(Span::styled(format!("   rss {}", fmt_bytes(last.harness_rss)), dim));
    f.render_widget(Paragraph::new(Line::from(cpu_line)), sy[0]);
    f.render_widget(Sparkline::default().data(&m.cpu.iter().cloned().collect::<Vec<_>>()).max(100).style(Style::default().fg(pal().blue)), sy[1]);
    match (last.gpu_util, last.gpu_mem) {
        (Some(g), gm) => {
            let mut gl = vec![Span::styled("gpu ", dim), Span::styled(format!("{g:>5.0}%"), Style::default().fg(if g > 80.0 { pal().orange } else { pal().fg }))];
            if let Some(t) = last.gpu_temp { gl.push(Span::styled(format!("  {t:.0}°C"), Style::default().fg(temp_col(t)))); }
            if let Some(pw) = last.gpu_power { gl.push(Span::styled(format!(" {pw:.1}W"), dim)); }
            gl.push(Span::styled(format!("   gpu mem {}", gm.map(fmt_bytes).unwrap_or_else(|| "?".into())), dim));
            f.render_widget(Paragraph::new(Line::from(gl)), sy[2]);
            f.render_widget(Sparkline::default().data(&m.gpu.iter().cloned().collect::<Vec<_>>()).max(100).style(Style::default().fg(pal().think)), sy[3]);
        }
        _ => { f.render_widget(Paragraph::new(Span::styled("gpu  n/a on this platform", dim)), sy[2]); }
    }
    let mr = if last.mem_total > 0 { last.mem_used as f64 / last.mem_total as f64 } else { 0.0 };
    let mut rl = vec![Span::styled("ram ", dim), Span::raw(format!("{} / {}", fmt_bytes(last.mem_used), fmt_bytes(last.mem_total))), Span::styled(format!("   server rss {}", fmt_bytes(last.server_rss)), dim)];
    if let Some(sp) = last.sys_power { rl.push(Span::styled(format!("   sys {sp:.0}W"), dim)); }
    f.render_widget(Paragraph::new(Line::from(rl)), sy[4]);
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
            let mut in_code: Option<(String, Option<syntect::easy::HighlightLines<'static>>)> = None;
            let all_lines: Vec<&str> = text.lines().collect();
            let mut li = 0usize;
            while li < all_lines.len() {
                let l = all_lines[li]; li += 1;
                let bullet = if first { Span::styled("⏺ ", Style::default().fg(pal().fg)) } else { Span::raw("  ") };
                first = false;
                // markdown table: consecutive lines starting with '|'
                if in_code.is_none() && l.trim_start().starts_with('|') && l.trim_end().ends_with('|') {
                    let mut rows: Vec<Vec<String>> = vec![parse_row(l)];
                    while li < all_lines.len() && all_lines[li].trim_start().starts_with('|') { rows.push(parse_row(all_lines[li])); li += 1; }
                    let sep_idx: Option<usize> = rows.iter().position(|r| r.iter().all(|c| !c.is_empty() && c.chars().all(|ch| ch == '-' || ch == ':' || ch == ' ')));
                    let cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
                    let mut widths = vec![0usize; cols];
                    for (ri, r) in rows.iter().enumerate() { if Some(ri) == sep_idx { continue; } for (ci, c) in r.iter().enumerate() { widths[ci] = widths[ci].max(c.chars().count()).min(40); } }
                    let mut first_row = true;
                    for (ri, r) in rows.iter().enumerate() {
                        if Some(ri) == sep_idx { let mut spans = vec![bullet.clone()]; for (ci, wdt) in widths.iter().enumerate() { spans.push(Span::styled(format!("{}{}", "─".repeat(wdt + 2), if ci + 1 < cols { "┼" } else { "" }), Style::default().fg(pal().dim))); } out.push(Line::from(spans)); continue; }
                        let mut spans = vec![if first_row { bullet.clone() } else { Span::raw("  ") }];
                        for ci in 0..cols { let cell = r.get(ci).cloned().unwrap_or_default(); let cell = truncate(&cell, widths[ci]); let pad = widths[ci].saturating_sub(cell.chars().count()); let hdr = sep_idx == Some(ri + 1); spans.push(Span::styled(format!(" {cell}{} ", " ".repeat(pad)), if hdr { Style::default().bold().fg(pal().orange) } else { Style::default() })); if ci + 1 < cols { spans.push(Span::styled("│", Style::default().fg(pal().dim))); } }
                        out.push(Line::from(spans)); first_row = false;
                    }
                    continue;
                }
                // lists: "- ", "* ", "1. " and nested (leading spaces)
                if in_code.is_none() { let t = l.trim_start(); let indent = l.len() - t.len(); if let Some(rest) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")).or_else(|| t.strip_prefix("• ")) { let mut spans = vec![bullet.clone(), Span::raw(" ".repeat(indent)), Span::styled("• ", Style::default().fg(pal().orange))]; spans.extend(md_spans(rest)); push_wrapped(out, spans, w, 4 + indent); continue; } if let Some(pos) = t.find(". ") { if pos <= 3 && t[..pos].chars().all(|c| c.is_ascii_digit()) { let mut spans = vec![bullet.clone(), Span::raw(" ".repeat(indent)), Span::styled(format!("{}. ", &t[..pos]), Style::default().fg(pal().orange))]; spans.extend(md_spans(&t[pos + 2..])); push_wrapped(out, spans, w, 4 + indent); continue; } } if let Some(rest) = t.strip_prefix("- [ ] ").or_else(|| t.strip_prefix("- [x] ")).or_else(|| t.strip_prefix("- [X] ")) { let done = t.starts_with("- [x") || t.starts_with("- [X"); let mut spans = vec![bullet.clone(), Span::raw(" ".repeat(indent)), Span::styled(if done { "☑ " } else { "☐ " }, Style::default().fg(if done { pal().ok } else { pal().dim }))]; spans.extend(md_spans(rest)); push_wrapped(out, spans, w, 4 + indent); continue; } if t.starts_with("> ") { let mut spans = vec![bullet.clone(), Span::styled("▍ ", Style::default().fg(pal().dim))]; spans.extend(md_spans(&t[2..]).into_iter().map(|sp| Span::styled(sp.content.to_string(), sp.style.fg(pal().dim).italic()))); push_wrapped(out, spans, w, 4); continue; } if t == "---" || t == "***" { out.push(Line::from(vec![bullet.clone(), Span::styled("─".repeat(w.saturating_sub(4)), Style::default().fg(pal().dim))])); continue; } }
                if let Some(rest) = l.trim_start().strip_prefix("```") {
                    if in_code.is_none() { in_code = Some((rest.trim().split_whitespace().next().unwrap_or("txt").to_string(), None)); out.push(Line::from(vec![bullet, Span::styled(format!("```{}", rest.trim()), Style::default().fg(pal().dim))])); }
                    else { in_code = None; out.push(Line::from(vec![bullet, Span::styled("```", Style::default().fg(pal().dim))])); }
                    continue;
                }
                if let Some((lang, state)) = &mut in_code {
                    let lc = clean(l);
                    let mut spans = vec![bullet, Span::styled("  ", Style::default().bg(pal().panel_bg))];
                    spans.extend(highlight_line(lang, &lc, state).into_iter().map(|sp| Span::styled(sp.content.to_string(), sp.style.bg(pal().panel_bg))));
                    // wrap long code lines instead of overflowing
                    let flat: Vec<Span<'static>> = spans; let mut tmp: Vec<Line<'static>> = Vec::new(); push_wrapped(&mut tmp, flat, w, 4);
                    for mut line in tmp { let used: usize = line.spans.iter().map(|sp| sp.content.chars().count()).sum(); if used < w { line.spans.push(Span::styled(" ".repeat(w - used), Style::default().bg(pal().panel_bg))); } out.push(line); }
                    continue;
                }
                // light markdown: **bold**, `code`
                push_wrapped(out, std::iter::once(bullet).chain(md_spans(l)).collect(), w, 2);
            }
            if *streaming { if let Some(last) = out.last_mut() { last.spans.push(Span::styled("▍", Style::default().fg(pal().orange))); } }
            if text.is_empty() && *streaming { out.push(Line::from(vec![Span::styled("⏺ ", Style::default().fg(pal().fg)), Span::styled("▍", Style::default().fg(pal().orange))])); }
            out.push(Line::raw(""));
        }
        Block::Reasoning { text, streaming, show, started, ended } => {
            let st = Style::default().fg(pal().think).italic();
            let dur = ended.map(|e| e.duration_since(*started)).unwrap_or_else(|| started.elapsed());
            let dur_s = { let s = dur.as_secs(); if s >= 60 { format!("{}m {:02}s", s / 60, s % 60) } else { format!("{s}s") } };
            if show.unwrap_or(app.show_thinking) {
                let mut first = true;
                for l in text.lines().filter(|l| !l.trim().is_empty()) { push_wrapped(out, vec![Span::styled(if first { "✻ " } else { "  " }, st), Span::styled(l.to_string(), st)], w, 2); first = false; }
                if *streaming { if let Some(last) = out.last_mut() { last.spans.push(Span::styled("▍", st)); } }
            } else {
                let firstline = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string();
                let lbl = if *streaming { format!("✻ Thinking… ({dur_s}) {}", truncate(&firstline, w.saturating_sub(44))) } else { format!("✻ Thought for {dur_s}: {}", truncate(&firstline, w.saturating_sub(44))) };
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
        Block::ContextReport { segments, window, measured, top, hints } => {
            let colors = |label: &str| match label { "system prompt" => pal().blue, "tool schemas" => pal().cyan, "memory files" => pal().pink, "skills/plugins" => pal().think, "handoff notes" => pal().orange, "user messages" => pal().fg, "assistant" => pal().ok, "tool results" => pal().dim, "images" => pal().think, _ => pal().dim };
            let total: u64 = segments.iter().map(|x| x.1).sum();
            let win = (*window).max(total).max(1);
            out.push(Line::from(vec![Span::styled("Context map ", Style::default().fg(pal().orange).bold()), Span::styled(format!("≈{} of {} tokens ({}%){}", fmt_k(total), fmt_k(*window), total * 100 / win, if *measured > 0 { format!(" · last measured prompt {}", fmt_k(*measured)) } else { String::new() }), Style::default().fg(pal().dim))]));
            // block grid: 10 rows × 20 cells, each cell = 0.5% of the window
            let cells_total = 200u64;
            let mut cells: Vec<Color> = Vec::new();
            for (label, n) in segments { let k = ((*n * cells_total + win / 2) / win).max(if *n > 0 { 1 } else { 0 }); for _ in 0..k { cells.push(colors(label)); } }
            cells.truncate(cells_total as usize);
            for row in 0..10 {
                let mut spans = vec![Span::raw("  ")];
                for col in 0..20 { let i = row * 20 + col; match cells.get(i) { Some(c) => spans.push(Span::styled("⛁ ", Style::default().fg(*c))), None => spans.push(Span::styled("⛶ ", Style::default().fg(pal().panel_bg))) } }
                out.push(Line::from(spans));
            }
            // rows
            for (label, n) in segments {
                if *n == 0 { continue; }
                let pct = *n as f64 * 100.0 / win as f64;
                let mini = ((*n * 20) / win.max(1)).max(1) as usize;
                out.push(Line::from(vec![Span::raw("  "), Span::styled("■ ", Style::default().fg(colors(label))), Span::styled(format!("{:<15}", label), Style::default()), Span::styled(format!("{:>7} ", fmt_k(*n)), Style::default().bold()), Span::styled(format!("{:>5.1}%  ", pct), Style::default().fg(pal().dim)), Span::styled("▮".repeat(mini.min(20)), Style::default().fg(colors(label)))]));
            }
            let free = win.saturating_sub(total);
            out.push(Line::from(vec![Span::raw("  "), Span::styled("□ free           ", Style::default().fg(pal().dim)), Span::styled(format!("{:>7} ", fmt_k(free)), Style::default().bold()), Span::styled(format!("{:>5.1}%", free as f64 * 100.0 / win as f64), Style::default().fg(pal().dim))]));
            if !top.is_empty() { out.push(Line::from(Span::styled("  heaviest items", Style::default().fg(pal().orange)))); for t in top { out.push(Line::from(vec![Span::raw("   "), Span::styled(t.clone(), Style::default().fg(pal().dim))])); } }
            for h in hints { out.push(Line::from(vec![Span::styled("  ▸ ", Style::default().fg(pal().orange)), Span::raw(h.clone())])); }
            out.push(Line::raw(""));
        }
        Block::CompactMap { before, after } => {
            let colors = |label: &str| match label { "system" => pal().blue, "handoff note" | "claude context (summary)" => pal().orange, "user" => pal().fg, "assistant" => pal().ok, "tool results" => pal().dim, "images" => pal().think, "claude context" => pal().blue, _ => pal().dim };
            let tb: u64 = before.iter().map(|x| x.1).sum::<u64>().max(1);
            let barw = w.saturating_sub(24).clamp(20, 90) as u64;
            for (title, map) in [("before", before), ("after ", after)] {
                let total: u64 = map.iter().map(|x| x.1).sum();
                let mut spans = vec![Span::styled(format!("  {title} "), Style::default().fg(pal().dim)), Span::styled(format!("{:>6} ", fmt_k(total)), Style::default().bold())];
                let mut used = 0u64;
                for (label, n) in map { let cells = ((n * barw) / tb).max(if *n > 0 { 1 } else { 0 }); used += cells; spans.push(Span::styled("█".repeat(cells as usize), Style::default().fg(colors(label)))); }
                if used < barw { spans.push(Span::styled("░".repeat((barw - used) as usize), Style::default().fg(pal().panel_bg))); }
                out.push(Line::from(spans));
            }
            let mut legend = vec![Span::raw("         ")];
            for (label, n) in before.iter().chain(after.iter()).fold(Vec::<(String, u64)>::new(), |mut acc, (l, n)| { if !acc.iter().any(|(x, _)| x == l) { acc.push((l.clone(), *n)); } acc }) { let _ = n; legend.push(Span::styled("■ ", Style::default().fg(colors(&label)))); legend.push(Span::styled(format!("{label}  "), Style::default().fg(pal().dim))); }
            out.push(Line::from(legend));
            out.push(Line::raw(""));
        }
        Block::Memory(t) => { push_wrapped(out, vec![Span::styled("🧠 ", Style::default()), Span::styled(t.clone(), Style::default().fg(pal().ok))], w, 3); }
        Block::Error(t) => { for (i, l) in t.lines().enumerate() { push_wrapped(out, vec![Span::styled(if i == 0 { "✗ " } else { "  " }, Style::default().fg(pal().err)), Span::styled(l.to_string(), Style::default().fg(pal().err))], w, 2); } out.push(Line::raw("")); }
        Block::Finished(t) => { push_wrapped(out, vec![Span::styled("  ✓ ", Style::default().fg(pal().ok)), Span::styled(t.clone(), Style::default().fg(pal().dim))], w, 4); }
    }
}

fn out_line_pad(out: &mut Vec<Line<'static>>) { out.push(Line::raw("")); }

fn parse_row(l: &str) -> Vec<String> { let t = l.trim().trim_start_matches('|').trim_end_matches('|'); t.split('|').map(|c| c.trim().to_string()).collect() }

/// Minimal inline markdown: `code` and **bold** and headings.
fn md_spans(l: &str) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    if let Some(h) = l.strip_prefix("### ").or_else(|| l.strip_prefix("## ")).or_else(|| l.strip_prefix("# ")) { out.push(Span::styled(h.to_string(), Style::default().bold().fg(pal().orange))); return out; }
    let mut rest = l; let mut bold = false;
    while !rest.is_empty() {
        if let Some(i) = rest.find('`') {
            if let Some(j) = rest[i + 1..].find('`') {
                if i > 0 { out.push(Span::styled(rest[..i].to_string(), if bold { Style::default().bold() } else { Style::default() })); }
                out.push(Span::styled(rest[i + 1..i + 1 + j].to_string(), Style::default().fg(pal().cyan)));
                rest = &rest[i + j + 2..]; continue;
            }
        }
        if let Some(i) = rest.find("**") { if i > 0 { out.push(Span::styled(rest[..i].to_string(), if bold { Style::default().bold() } else { Style::default() })); } bold = !bold; rest = &rest[i + 2..]; continue; }
        out.push(Span::styled(rest.to_string(), if bold { Style::default().bold() } else { Style::default() })); break;
    }
    out
}

fn args_summary(name: &str, args: &str, max: usize) -> String {
    let v: serde_json::Value = serde_json::from_str(args).unwrap_or(serde_json::Value::Null);
    let s = match name {
        "bash" => v["cmd"].as_str().unwrap_or(args).to_string(),
        "read_file" | "write_file" | "edit_file" | "list_dir" | "view_image" | "read_pdf" | "pdf_edit" | "extract_archive" => v["path"].as_str().unwrap_or(args).to_string(),
        "web_fetch" | "download_file" => v["url"].as_str().unwrap_or(args).to_string(),
        "web_search" => v["query"].as_str().unwrap_or(args).to_string(),
        _ => args.to_string(),
    };
    truncate(&s.replace('\n', "⏎"), max.max(8))
}

/// Wrap spans to `width`, continuation lines indented by `indent`. Char/width aware.
/// Make text safe for cell-accurate rendering: tabs → 4 spaces, control chars dropped (they would desync width math and spill into neighbouring areas).
fn clean(s: &str) -> String { let mut o = String::with_capacity(s.len()); for ch in s.chars() { match ch { '\t' => o.push_str("    "), c if c.is_control() => {}, c => o.push(c) } } o }

fn push_wrapped(out: &mut Vec<Line<'static>>, spans: Vec<Span<'static>>, width: usize, indent: usize) {
    let mut cur: Vec<Span<'static>> = Vec::new();
    let mut col = 0usize;
    for sp in spans {
        let style = sp.style;
        let mut buf = String::new();
        let content = clean(&sp.content);
        for ch in content.chars() {
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
/// Best-effort desktop notification (macOS osascript / Linux notify-send).
fn desktop_notify(title: &str, body: &str) {
    if std::env::var_os("HARNESS_NO_NOTIFY").is_some() { return; }
    let (title, body) = (title.to_string(), body.replace('"', "'"));
    tokio::spawn(async move {
        if cfg!(target_os = "macos") { let script = format!("display notification \"{body}\" with title \"{title}\" sound name \"Glass\""); let _ = tokio::process::Command::new("osascript").arg("-e").arg(script).output().await; }
        else if cfg!(target_os = "linux") { let _ = tokio::process::Command::new("notify-send").arg(&title).arg(&body).output().await; }
    });
}
fn short_path(p: &std::path::Path) -> String { let s = p.display().to_string(); let h = harness::setup::home_dir().display().to_string(); if let Some(r) = s.strip_prefix(&h) { return format!("~{r}"); } s }
