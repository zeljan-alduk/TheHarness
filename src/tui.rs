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
    ("ui.steer", "Enter steers a running task", &["on", "off"], "on = messages typed while a task runs reach the agent at its next tool boundary; off = they queue"),
    ("permissions.mode", "Permission mode", &["auto", "ask", "plan", "bypass"], "default for new sessions (shift+tab cycles live)"),
    ("llm.effort", "Effort (Claude backend)", &["medium", "low", "high", "xhigh", "max"], "reasoning effort passed to Claude Code"),
    ("llm.compact_at_fraction", "Auto-compact at", &["0.75", "0.5", "0.6", "0.85", "0.9"], "fraction of the context window that triggers compaction (local/API backends)"),
    ("memory.auto_reflect", "Memory reflection", &["on", "off"], "learn durable facts into BRAIN.md after substantive runs"),
    ("security.redact_secrets", "Redact secrets", &["on", "off"], "mask API keys/tokens in tool outputs"),
    ("net.enabled", "Internet tools", &["on", "off"], "web_fetch / web_search / download_file"),
    ("agent.max_task_secs", "Max task time", &["0", "300", "900", "1800", "3600"], "0 = unlimited; the queue continues afterwards"),
    ("ui.event_log", "Event log", &["on", "off"], "~/.config/harness/logs/<date>/"),
    ("ui.font_size", "Font size (pt)", &["0", "11", "12", "13", "14", "15", "16", "18", "20"], "0 = leave the terminal alone · ctrl+= / ctrl+- / ctrl+0 (kitty, iTerm2, Terminal.app)"),
    ("sandbox.mode", "Sandbox", &["none", "seatbelt", "bwrap"], "confine shell writes (macOS seatbelt / Linux bubblewrap)"),
    ("format.enabled", "Format after edits", &["on", "off"], "run the project formatter (rustfmt, ruff, prettier, gofmt…) on files the agent writes"),
    ("format.diagnostics_after_edit", "Diagnostics after edits", &["on", "off"], "report language-server errors for the edited file (only when a server is already running)"),
    ("checkpoints.enabled", "File checkpoints", &["on", "off"], "snapshot the working tree before every change so /undo and /rewind can restore it"),
    ("llm.tool_shim", "Tool-call shim", &["auto", "on", "off"], "text <tool_call> protocol for servers/models without function calling (auto switches on demand)"),
    ("llm.provider", "Backend", &[], "change with /backend"),
    ("llm.model", "Model", &[], "change with /model or /backend"),
];

// ───────────────────────── custom keybindings ─────────────────────────
/// ~/.config/harness/keybindings.toml — [bindings] action = "ctrl+x" | "alt+enter" | "shift+tab" | "f5" | "esc" …
/// The Claude Code models the pickers offer, in the order a picker shows them — the cursor starts on the
/// first, which is why fable is there. Effort is a separate knob (/effort).
const CLAUDE_MODELS: &[(&str, &str)] = &[
    ("claude-fable-5", "the default — fast, strong at code"),
    ("claude-opus-5", "deepest reasoning, slowest"),
    ("claude-sonnet-5", "balanced"),
    ("claude-haiku-4-5", "cheapest, quickest"),
];

/// What to *show* for a model id. `mlx_lm.server` answers /v1/models with the filesystem path it was
/// started with, which is long and mostly noise on screen; requests still carry the real id, so only the
/// display is shortened. Publisher-style ids ("qwen/qwen3.6-35b") keep their slash.
fn model_label(id: &str) -> &str {
    if id.starts_with('/') { id.trim_end_matches('/').rsplit('/').next().unwrap_or(id) } else { id }
}

/// Actions: interrupt, next_task, toggle_panel, toggle_thinking, expand_tools, paste, cycle_permissions, newline,
/// quit, scroll_up, scroll_down, clear_line, jump_bottom, complete
#[derive(Clone, Default)]
struct Keymap { map: std::collections::HashMap<String, (KeyCode, KeyModifiers)> }
impl Keymap {
    fn load() -> Self {
        let mut m = std::collections::HashMap::new();
        let defaults = [("interrupt", "esc"), ("next_task", "ctrl+n"), ("toggle_panel", "ctrl+p"), ("toggle_thinking", "ctrl+t"), ("expand_tools", "ctrl+o"), ("paste", "ctrl+v"), ("cycle_permissions", "shift+tab"), ("newline", "ctrl+j"), ("quit", "ctrl+d"), ("scroll_up", "pageup"), ("scroll_down", "pagedown"), ("clear_line", "ctrl+u"), ("jump_bottom", "ctrl+l"), ("complete", "tab"), ("font_bigger", "ctrl+="), ("font_smaller", "ctrl+-"), ("font_reset", "ctrl+0")];
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
/// A palette loaded from ~/.config/harness/themes/<name>.json (keys: orange, dim, ok, err, think,
/// blue, pink, cyan, fg, panel_bg; values "#rrggbb"). Set with /theme <name>.
static CUSTOM: std::sync::Mutex<Option<Pal>> = std::sync::Mutex::new(None);

fn hex_color(s: &str) -> Option<Color> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 { return None; }
    let n = u32::from_str_radix(h, 16).ok()?;
    Some(Color::Rgb((n >> 16) as u8, (n >> 8) as u8, n as u8))
}

/// Theme names available: the built-ins plus every JSON file in the themes directory.
fn theme_names() -> Vec<String> {
    let mut v = vec!["dark".to_string(), "light".to_string()];
    if let Ok(rd) = std::fs::read_dir(harness::setup::config_dir().join("themes")) {
        for e in rd.flatten() { if e.path().extension().map(|x| x == "json").unwrap_or(false) { if let Some(n) = e.path().file_stem() { v.push(n.to_string_lossy().to_string()); } } }
    }
    v.sort(); v.dedup(); v
}

/// Load a custom theme file over the current base palette. Returns what it could not parse.
fn load_theme(name: &str) -> Result<(), String> {
    let path = harness::setup::config_dir().join("themes").join(format!("{name}.json"));
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let base = { let dark = v["base"].as_str().unwrap_or("dark") == "dark"; LIGHT.store(!dark, std::sync::atomic::Ordering::Relaxed); builtin_pal() };
    let g = |k: &str, d: Color| v[k].as_str().and_then(hex_color).unwrap_or(d);
    let p = Pal {
        orange: g("orange", base.orange), dim: g("dim", base.dim), ok: g("ok", base.ok), err: g("err", base.err),
        think: g("think", base.think), blue: g("blue", base.blue), pink: g("pink", base.pink), cyan: g("cyan", base.cyan),
        fg: g("fg", base.fg), panel_bg: g("panel_bg", base.panel_bg),
    };
    *CUSTOM.lock().unwrap() = Some(p);
    Ok(())
}

fn pal() -> Pal {
    if let Some(p) = *CUSTOM.lock().unwrap() { return p; }
    builtin_pal()
}

fn builtin_pal() -> Pal {
    if LIGHT.load(std::sync::atomic::Ordering::Relaxed) {
        Pal { orange: Color::Rgb(200, 90, 0), dim: Color::Rgb(110, 116, 130), ok: Color::Rgb(20, 140, 80), err: Color::Rgb(200, 40, 40), think: Color::Rgb(110, 70, 220), blue: Color::Rgb(20, 100, 220), pink: Color::Rgb(200, 40, 90), cyan: Color::Rgb(0, 130, 150), fg: Color::Black, panel_bg: Color::Rgb(225, 228, 235) }
    } else {
        Pal { orange: Color::Rgb(255, 140, 40), dim: Color::Rgb(128, 136, 152), ok: Color::Rgb(76, 195, 138), err: Color::Rgb(255, 107, 107), think: Color::Rgb(167, 139, 250), blue: Color::Rgb(78, 161, 255), pink: Color::Rgb(255, 110, 130), cyan: Color::Rgb(90, 205, 220), fg: Color::White, panel_bg: Color::Rgb(38, 44, 56) }
    }
}
const SPINNER: [&str; 10] = ["✻", "✼", "✽", "✾", "✿", "❀", "✿", "✾", "✽", "✼"];
const WORDS: [&str; 12] = ["Thinking", "Pondering", "Working", "Reasoning", "Cooking", "Tinkering", "Brewing", "Mulling", "Crunching", "Percolating", "Noodling", "Computing"];

enum Msg { Toast(String), Title(String), Question(harness::permissions::Question, tokio::sync::oneshot::Sender<harness::permissions::Answer>), SubEnv(Arc<harness::agent::SubAgentEnv>), Policy(Arc<harness::permissions::Policy>), CcSession(Arc<harness::claude_code::ClaudeCodeSession>), CcSid(String), Block(Block), Ask(harness::permissions::ApprovalRequest, tokio::sync::oneshot::Sender<harness::permissions::Approval>), Ev(Event), Done(Result<(String, harness::agent::RunStats), String>), Sys(SysSample), CtxLen(u64), Pasted(Result<PathBuf, String>), Frames(Result<Extracted, String>), Toolset(Arc<Toolset>), Catalog(Result<harness::plugins::Catalog, String>), Notice(String), Improve(harness::selfimprove::Stage), GoalCheck(bool, String), Review(String), AcpSession(Arc<harness::acp_client::AcpSession>), RunTask(String), QueueTask(String), StatusLine(String), Dictated(String),
    /// First-run model bootstrap: what the configured local endpoint can actually do.
    LocalProbe(harness::localmodel::Endpoint),
    /// Weights coming down: aggregate progress across the repo's files.
    ModelDl(harness::localmodel::Progress),
    /// The download finished (Ok) or gave up (Err) — a partial download is resumable, not lost.
    ModelDlDone(Result<String, String>),
    /// The MLX server answered on this base_url with this model id, or failed to come up.
    MlxUp(Result<(String, String, &'static str), String>),
    /// The server on our own MLX port turned out to be mlx_lm (text-only) while vision is wanted.
    MlxTextOnly(String),
    /// Whether the Claude Code CLI can run a turn (installed + signed in).
    ClaudeAuth(harness::claude_code::Auth),
    /// /update: what GitHub says the latest release is (the TUI only reports; installing happens on start).
    Update(Result<harness::update::Release, String>) }

/// One hunk of the working-tree diff, as `/review-diff` shows it.
#[derive(Clone, Debug)]
struct Hunk { file: String, header: String, body: Vec<String>, plus: usize, minus: usize, reverted: bool, comment: Option<String> }

impl Hunk {
    /// A standalone patch for this hunk alone (what `git apply -R` needs to revert it).
    fn patch(&self) -> String {
        format!("--- a/{}\n+++ b/{}\n{}\n{}\n", self.file, self.file, self.header, self.body.join("\n"))
    }
}

/// Split `git diff` output into per-file hunks.
fn parse_hunks(diff: &str) -> Vec<Hunk> {
    let mut out: Vec<Hunk> = Vec::new();
    let mut file = String::new();
    let mut cur: Option<Hunk> = None;
    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("+++ b/") { file = rest.trim().to_string(); continue; }
        if line.starts_with("diff --git ") { if let Some(f) = line.split(" b/").nth(1) { file = f.trim().to_string(); } continue; }
        if line.starts_with("@@") {
            if let Some(h) = cur.take() { out.push(h); }
            cur = Some(Hunk { file: file.clone(), header: line.to_string(), body: vec![], plus: 0, minus: 0, reverted: false, comment: None });
            continue;
        }
        if let Some(h) = cur.as_mut() {
            if line.starts_with("+++") || line.starts_with("---") || line.starts_with("index ") || line.starts_with("new file") || line.starts_with("deleted file") || line.starts_with("similarity ") { continue; }
            if line.starts_with('+') { h.plus += 1; } else if line.starts_with('-') { h.minus += 1; }
            h.body.push(line.to_string());
        }
    }
    if let Some(h) = cur.take() { out.push(h); }
    out
}

/// `/review-diff` modal state.
struct DiffReview { hunks: Vec<Hunk>, cursor: usize, scroll: usize, comment: Option<String> }

/// Video scrubber state (modal over the transcript).
struct VideoPicker {
    path: PathBuf,
    duration: f64,
    /// (timestamp, frame file, image key once it has been decoded) — decoding is lazy so a video with
    /// hundreds of frames does not turn into hundreds of live image protocols.
    frames: Vec<(f64, PathBuf, Option<String>)>,
    cur: usize,
    selected: std::collections::BTreeSet<usize>,
    loading: bool,
    error: Option<String>,
    /// How the frames were sampled ("32 keyframes", "750 frames (no keyframes)", …).
    note: String,
    /// Image key of the large preview and which frame it holds — kept separate from the strip's
    /// thumbnail protocol, which is resized to a dozen cells. Carries the frame's pixel size and
    /// file size for the info line.
    preview: Option<(usize, String, (u32, u32), u64)>,
    /// One line about the source video: resolution, codec, fps, bit rate, file size.
    source: String,
}

fn video_ext(p: &std::path::Path) -> bool {
    matches!(p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref(), Some("mp4" | "mov" | "m4v" | "webm" | "mkv" | "avi" | "gif" | "mpg" | "mpeg"))
}

/// Probe duration and extract up to `n` evenly spaced JPEG frames (max width 640) with ffmpeg.
/// What `extract_frames` produced: the frames plus enough about the source to describe it on screen.
struct Extracted { path: PathBuf, duration: f64, frames: Vec<(f64, PathBuf)>, note: String, source: String }

/// Frames of a video, in the order a person would want to scrub them: the keyframes (what the codec
/// itself considers a scene) when there are any, otherwise every single frame. Returns
/// (video, duration, [(timestamp, file)], how it was sampled).
async fn extract_frames(video: PathBuf, out_dir: PathBuf, max_frames: usize) -> Result<Extracted, String> {
    // a feature film has ~1500–2500 keyframes and they only cost a decode each, so keep them all;
    // the every-frame fallback is the one that has to be thinned (140k frames for the same film)
    let max_keyframes = max_frames.max(4000);
    let probe = |args: Vec<String>| {
        let video = video.clone();
        async move {
            let o = tokio::process::Command::new("ffprobe").args(&args).arg(&video).output().await.map_err(|e| format!("ffprobe: {e} (install ffmpeg: brew install ffmpeg)"))?;
            Ok::<String, String>(String::from_utf8_lossy(&o.stdout).to_string())
        }
    };
    let sarg = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<String>>();
    let duration: f64 = probe(sarg(&["-v", "error", "-show_entries", "format=duration", "-of", "default=noprint_wrappers=1:nokey=1"])).await?.trim().parse().unwrap_or(0.0);
    // one line describing the source itself, for the info line under the preview
    let src_raw = probe(sarg(&["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=width,height,codec_name,avg_frame_rate,bit_rate", "-of", "default=noprint_wrappers=1"])).await.unwrap_or_default();
    let field_of = |raw: &str, k: &str| raw.lines().find_map(|l| l.strip_prefix(&format!("{k}="))).map(|v| v.trim().to_string()).unwrap_or_default();
    let src_fps = field_of(&src_raw, "avg_frame_rate").split_once('/').and_then(|(a, b)| Some(a.parse::<f64>().ok()? / b.parse::<f64>().ok()?)).filter(|f| f.is_finite() && *f > 0.0);
    let file_bytes = std::fs::metadata(&video).map(|m| m.len()).unwrap_or(0);
    let source = format!("{}×{} · {} · {}{} · {}",
        field_of(&src_raw, "width"), field_of(&src_raw, "height"),
        field_of(&src_raw, "codec_name"),
        src_fps.map(|f| format!("{f:.2} fps")).unwrap_or_else(|| "? fps".into()),
        match field_of(&src_raw, "bit_rate").parse::<f64>() { Ok(b) if b > 0.0 => format!(" · {:.1} Mb/s", b / 1e6), _ => String::new() },
        fmt_bytes(file_bytes));
    let _ = std::fs::remove_dir_all(&out_dir);
    std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
    let scale = "scale='min(1280,iw)':-2";

    // 1. keyframe timestamps (I-frames); ffmpeg ≥ 4 answers best_effort_timestamp_time
    let mut keys: Vec<f64> = Vec::new();
    for entry in ["frame=best_effort_timestamp_time", "frame=pkt_pts_time"] {
        let out = probe(sarg(&["-v", "error", "-select_streams", "v:0", "-skip_frame", "nokey", "-show_entries", entry, "-of", "csv=p=0"])).await.unwrap_or_default();
        keys = out.lines().filter_map(|l| l.trim().trim_end_matches(',').parse::<f64>().ok()).collect();
        if keys.len() > 1 { break; }
    }

    let run = |args: Vec<String>| {
        let video = video.clone();
        async move {
            let o = tokio::process::Command::new("ffmpeg").args(["-hide_banner", "-loglevel", "error", "-y"]).args(&args[..args.iter().position(|a| a == "-i").unwrap_or(0)])
                .arg("-i").arg(&video).args(&args[args.iter().position(|a| a == "-i").map(|i| i + 1).unwrap_or(0)..])
                .output().await.map_err(|e| format!("ffmpeg: {e}"))?;
            if !o.status.success() { return Err(format!("ffmpeg failed: {}", String::from_utf8_lossy(&o.stderr).trim())); }
            Ok::<(), String>(())
        }
    };
    let pattern = out_dir.join("frame-%05d.jpg").display().to_string();
    let collect = || -> Result<Vec<PathBuf>, String> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(&out_dir).map_err(|e| e.to_string())?.flatten().map(|e| e.path())
            .filter(|p| p.extension().map(|e| e == "jpg").unwrap_or(false)).collect();
        v.sort();
        Ok(v)
    };

    // a handful of keyframes over a long clip is not a scrub strip — that is when every frame wins
    let keyframes_useful = keys.len() >= 8 && (duration <= 0.0 || keys.len() as f64 >= duration / 8.0);
    if keyframes_useful {
        // decode only the keyframes: fast, and exactly the frames the codec marked
        run(sarg(&["-skip_frame", "nokey", "-i", "-vsync", "0", "-q:v", "3", "-vf", scale, &pattern])).await?;
        let files = collect()?;
        if !files.is_empty() {
            let mut frames: Vec<(f64, PathBuf)> = files.into_iter().enumerate().map(|(i, p)| (keys.get(i).copied().unwrap_or(i as f64), p)).collect();
            let total = frames.len();
            if total > max_keyframes { let stride = (total + max_keyframes - 1) / max_keyframes; frames = frames.into_iter().step_by(stride).collect(); }
            let note = if total > frames.len() { format!("{} of {total} keyframes", frames.len()) } else { format!("{total} keyframes") };
            return Ok(Extracted { path: video, duration, frames, note, source });
        }
    }

    // 2. no keyframes to speak of: every frame, thinned only if there are absurdly many
    // avg_frame_rate is the real average (r_frame_rate is often just the container timebase, e.g.
    // 90000/1 on a variable-rate screen recording); nb_frames, when the container knows it, beats both
    let info = probe(sarg(&["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=avg_frame_rate,r_frame_rate,nb_frames", "-of", "default=noprint_wrappers=1"])).await.unwrap_or_default();
    let field = |k: &str| info.lines().find_map(|l| l.strip_prefix(&format!("{k}="))).map(|v| v.trim().to_string()).unwrap_or_default();
    let ratio = |v: &str| v.split_once('/').and_then(|(a, b)| Some(a.trim().parse::<f64>().ok()? / b.trim().parse::<f64>().ok()?)).filter(|f| f.is_finite() && *f > 0.05 && *f < 480.0);
    let counted: Option<usize> = field("nb_frames").parse().ok().filter(|n: &usize| *n > 0);
    let fps = ratio(&field("avg_frame_rate")).or_else(|| ratio(&field("r_frame_rate")))
        .or_else(|| counted.filter(|_| duration > 0.0).map(|n| n as f64 / duration))
        .unwrap_or(30.0);
    let estimated = counted.unwrap_or(if duration > 0.0 { (duration * fps) as usize } else { max_frames });
    let (vf, note_prefix) = if estimated > max_frames {
        let want_fps = (max_frames as f64 / duration.max(0.001)).max(0.1);
        (format!("fps={want_fps:.3},{scale}"), format!("{max_frames} of ~{estimated} frames"))
    } else { (scale.to_string(), String::new()) };
    run(sarg(&["-i", "-vsync", "0", "-q:v", "3", "-vf", &vf, "-frames:v", &max_frames.to_string(), &pattern])).await?;
    let files = collect()?;
    if files.is_empty() { return Err("no frames extracted".into()); }
    let step = if estimated > max_frames { duration / files.len().max(1) as f64 } else { 1.0 / fps };
    let frames: Vec<(f64, PathBuf)> = files.into_iter().enumerate().map(|(i, p)| (i as f64 * step, p)).collect();
    let note = if note_prefix.is_empty() { format!("{} frames (no keyframes)", frames.len()) } else { format!("{note_prefix} (no keyframes)") };
    Ok(Extracted { path: video, duration, frames, note, source })
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

/// Send one kitty remote-control command on our own terminal — the same DCS packet `kitten @` writes,
/// which needs no socket (a fixed `--listen-on` path is unlinked by the next kitty that binds it, which
/// is how this broke) and no `kitten` binary. `no_response` keeps kitty quiet on success; a *refusal* is
/// answered anyway, which is why kitty_remote_control_ok() below asks the question once, up front.
fn kitty_cmd(json: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut o = std::io::stdout();
    o.write_all(format!("\x1bP@kitty-cmd{{\"version\":[0,26,0],\"no_response\":true,{json}}}\x1b\\").as_bytes())?;
    o.flush()
}
static KITTY_RC: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0); // 0 unknown, 1 allowed, 2 refused
/// Is remote control allowed on this terminal? Ask with a no-op (font size × 1) and listen: silence means
/// yes, an error packet means no. The reply would otherwise be decoded as key presses and typed into the
/// prompt, so this runs once at start-up — in raw mode, before the event loop owns the input stream.
fn kitty_remote_control_ok() -> bool {
    use crossterm::event::{poll, read};
    while poll(Duration::ZERO).unwrap_or(false) { let _ = read(); } // leftovers from earlier terminal queries
    if kitty_cmd("\"cmd\":\"set-font-size\",\"payload\":{\"size\":1,\"increment_op\":\"*\"}").is_err() { return false; }
    let refused = poll(Duration::from_millis(150)).unwrap_or(false);
    while poll(Duration::ZERO).unwrap_or(false) { let _ = read(); } // swallow the refusal, whatever it was
    KITTY_RC.store(if refused { 2 } else { 1 }, std::sync::atomic::Ordering::Relaxed);
    !refused
}

/// Drive the terminal's font size. Inside kitty we speak its remote-control protocol ourselves, on our
/// own terminal; iTerm2 and Terminal.app go through AppleScript. Returns the terminal name on success.
async fn set_terminal_font_size(pt: u32) -> Result<&'static str, String> {
    if std::env::var_os("KITTY_WINDOW_ID").is_some() {
        if KITTY_RC.load(std::sync::atomic::Ordering::Relaxed) == 2 { return Err("kitty: remote control is off — add `allow_remote_control yes` to kitty.conf and restart kitty".into()); }
        kitty_cmd(&format!("\"cmd\":\"set-font-size\",\"payload\":{{\"size\":{pt}}}")).map_err(|e| e.to_string())?;
        return Ok("kitty");
    }
    let prog = std::env::var("TERM_PROGRAM").unwrap_or_default();
    if prog == "iTerm.app" {
        let script = format!("tell application \"iTerm2\" to tell current session of current window to set font size to {pt}");
        let o = tokio::process::Command::new("osascript").arg("-e").arg(script).output().await.map_err(|e| e.to_string())?;
        return if o.status.success() { Ok("iTerm2") } else { Err(String::from_utf8_lossy(&o.stderr).trim().to_string()) };
    }
    if prog == "Apple_Terminal" {
        let script = format!("tell application \"Terminal\" to set font size of front window to {pt}");
        let o = tokio::process::Command::new("osascript").arg("-e").arg(script).output().await.map_err(|e| e.to_string())?;
        return if o.status.success() { Ok("Terminal") } else { Err(String::from_utf8_lossy(&o.stderr).trim().to_string()) };
    }
    if prog == "WezTerm" { return Err("WezTerm: use its own ctrl+= / ctrl+- (no remote font control)".into()); }
    Err("this terminal cannot be resized from the app (kitty, iTerm2, Terminal.app supported)".into())
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
    /// The start-up card: the same content as a banner, but it types itself in when first drawn
    /// (the clock starts at the first paint — see `App::banner_at`).
    Startup(Vec<String>),
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
    /// `/goal`: keep working until an aux-model checker says this condition is met.
    goal: Option<String>,
    goal_rounds: usize,
    /// ctrl+r reverse history search: (query, saved input, match index).
    hist_search: Option<(String, String, usize)>,
    /// Which slash-command suggestion is highlighted (↑/↓ while the list is open).
    sugg_idx: Option<usize>,
    /// ctrl+g: the main loop suspends the TUI and opens $EDITOR on the prompt.
    edit_external: bool,
    /// `/review-diff`: per-hunk accept/revert/comment over the working tree.
    review: Option<DiffReview>,
    /// External ACP agent driving this session (provider = "acp:…").
    acp: Option<Arc<harness::acp_client::AcpSession>>,
    /// Output of the user's [ui] statusline command, refreshed every couple of seconds.
    statusline: Option<String>,
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
    // mouse text selection (left-drag in the transcript) + toast
    sel_anchor: Option<(u16, u16)>, sel_cur: Option<(u16, u16)>, sel_dragging: bool,
    visible_text: Vec<String>,
    toast: Option<(String, Instant)>,
    tool_view: String,               // summary | hidden | full
    tool_groups_open: std::collections::HashSet<usize>, // first block index of an expanded tool burst
    settings_open: bool, settings_cursor: usize,
    /// /sessions picker: (all sessions, cursor into the filtered view, filter text, first visible row, row rect for clicks)
    sessions_pick: Option<SessionPicker>,
    /// Generic list picker (/tools, /skills, /commands, …).
    pick: Option<ListPicker>,
    /// Last window title written, so it is only rewritten when something changed.
    title_shown: Option<String>,
    /// How many frames the start-up card has been on screen — its animation clock. Counting frames
    /// rather than milliseconds means the reveal is always seen, even when start-up work stalls the
    /// loop for a moment.
    banner_step: u32,
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
    /// First-run model bootstrap: live download progress, which build it is, and the MLX server we own.
    dl: Option<harness::localmodel::Progress>,
    dl_build: Option<&'static harness::localmodel::Build>,
    dl_cancel: Option<tokio::task::JoinHandle<()>>,
    /// No model can serve a turn: the mode line says so instead of naming one that cannot answer.
    no_model: bool,
    /// The Claude offer is made once per session, whichever no-model path gets there first.
    claude_asked: bool,
    /// The first-run picker is open; closing it (however) still owes the user the Claude question.
    first_run_pending: bool,
    /// The "waiting for Qwen3.8" explanation is worth saying once, not on every picker close.
    said_waiting: bool,
    /// A command that needs the terminal to itself (the Claude sign-in): the loop suspends the UI,
    /// runs it, and comes back — the same trick as ctrl+g's $EDITOR.
    run_external: Option<String>,
}

pub async fn run(cfg: Config, resume: Option<String>, update_note: Option<String>) -> Result<()> {
    let workdir = std::env::current_dir()?;
    // Detect the terminal's graphics protocol (kitty / iterm2 / sixel) and cell size; fall back to half-blocks.
    // Only query terminals known to answer — a plain pty or Terminal.app would block forever on the query.
    let picker = if graphics_terminal() { Picker::from_query_stdio().unwrap_or_else(|_| Picker::from_fontsize((8, 16))) } else { Picker::from_fontsize((8, 16)) };
    let (tx, mut rx) = mpsc::unbounded_channel::<Msg>();
    let mut app = App {
        model: cfg.llm.model.clone(), net: cfg.net.enabled, cfg, workdir,
        blocks: vec![], input: String::new(), cursor: 0, history: vec![], hist_idx: None, hist_draft: String::new(),
        scroll_up: 0, running: None, run_started: Instant::now(), queued: vec![], goal: None, goal_rounds: 0, hist_search: None, sugg_idx: None, edit_external: false, review: None, acp: None, statusline: None, expand_tools: false, show_thinking: false,
        session: Arc::new(tokio::sync::Mutex::new(Vec::new())), tx: tx.clone(),
        total_prompt: 0, total_completion: 0, last_prompt_tokens: 0, turn_tokens: 0, last_ctrl_c: None, status_msg: None,
        quit: false, restart: false, improve: None, improve_cancel: Default::default(), restart_at: None, tick: 0, word: 0, models: vec![],
        metrics: Metrics::new(0), panel: None, attachments: vec![], tool_previews: Default::default(),
        picker, images: Default::default(), img_seq: 0,
        think_scroll: 0, toolset: None, perm_mode: harness::permissions::Mode::Auto, vim: false, vim_normal: false, keymap: Keymap::load(), sel_anchor: None, sel_cur: None, sel_dragging: false, visible_text: vec![], toast: None, tool_view: "summary".into(), tool_groups_open: Default::default(), settings_open: false, settings_cursor: 0, sessions_pick: None, pick: None, title_shown: None, banner_step: 0, live_policy: None, cc_rate: None, extra_roots: vec![], wt_cwd: harness::worktree::new_cell(), cc: None, cc_last_session: None, compact_progress: None, session_meta: harness::sessions::Meta::default(), todos: Default::default(), inbox: Default::default(), event_log: None, pending_ask: None, pending_q: None, subenv: None, attached: None, video: None, strip_rects: vec![], tr_rect: Rect::default(), panel_rect: Rect::default(), tr_start: 0, line_map: vec![], dl: None, dl_build: None, dl_cancel: None, no_model: false, claude_asked: false, first_run_pending: false, said_waiting: false, run_external: None,
    };
    app.metrics.ctx_len = app.cfg.llm.context_budget_tokens.unwrap_or(0);
    app.perm_mode = app.cfg.permissions.mode;
    app.tool_view = app.cfg.ui.tool_view.clone(); app.show_thinking = app.cfg.ui.show_thinking; app.vim = app.cfg.ui.vim;
    app.panel = match app.cfg.ui.panel.as_str() { "on" => Some(true), "off" => Some(false), _ => None };
    if app.cfg.ui.event_log { { let d = config_dir().join("logs").join(harness::memory::today_iso()); let _ = std::fs::create_dir_all(&d); app.event_log = std::fs::OpenOptions::new().create(true).append(true).open(d.join(format!("tui-{}.jsonl", std::process::id()))).ok(); } }
    if app.cfg.ui.theme == "light" { LIGHT.store(true, std::sync::atomic::Ordering::Relaxed); }
    app.banner();
    // The start-up update pass (main.rs) either exec'd us from a fresh install — say so once — or left a note.
    if let Ok(from) = std::env::var("HARNESS_UPDATED_FROM") { std::env::remove_var("HARNESS_UPDATED_FROM"); if !from.is_empty() { app.blocks.push(Block::System(format!("⬆ updated to {} (from {from}) — the previous binary is kept as harness.prev; `harness update --rollback` goes back", harness::update::Version::current()))); } }
    if let Some(n) = update_note { app.blocks.push(Block::System(n)); }
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

    if !matches!(app.cfg.ui.theme.as_str(), "dark" | "light") { let _ = load_theme(&app.cfg.ui.theme.clone()); }
    let mut terminal = ratatui::init();
    let _ = crossterm::execute!(std::io::stdout(), crossterm::event::EnableBracketedPaste, crossterm::event::EnableMouseCapture);
    let kbd_enh = crossterm::terminal::supports_keyboard_enhancement().unwrap_or(false);
    if kbd_enh { let _ = crossterm::execute!(std::io::stdout(), crossterm::event::PushKeyboardEnhancementFlags(crossterm::event::KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)); }
    // ctrl+= / ctrl+- drive kitty's font size over remote control: settle now whether it is allowed, while
    // the input stream is still ours to read (see kitty_remote_control_ok).
    if std::env::var_os("KITTY_WINDOW_ID").is_some() && !kitty_remote_control_ok() { app.blocks.push(Block::System("kitty: remote control is off, so ctrl+= / ctrl+- cannot change the font size — add `allow_remote_control yes` to kitty.conf".into())); }
    if app.cfg.ui.font_size > 0 { let sz = app.cfg.ui.font_size; tokio::spawn(async move { let _ = set_terminal_font_size(sz).await; }); }
    // First run has no model: ask the local endpoint before offering to download one (see on_local_probe).
    if app.cfg.llm.provider.is_none() && app.cfg.local_model.first_run_prompt {
        let (tx2, base) = (tx.clone(), app.cfg.llm.base_url.clone());
        tokio::spawn(async move {
            let up = harness::localmodel::probe(&base).await;
            // the picker warns when a build wants more RAM than the machine has, and that number comes
            // from the first sampler tick — wait for it rather than print advice without it
            tokio::time::sleep(Duration::from_millis(1200)).await;
            let _ = tx2.send(Msg::LocalProbe(up));
        });
    }
    let mut events = EventStream::new();
    let mut ticker = tokio::time::interval(Duration::from_millis(80));
    let res: Result<()> = async {
        loop {
            terminal.draw(|f| draw(f, &mut app))?;
            tokio::select! {
                _ = ticker.tick() => {
                    app.tick += 1; if app.tick % 30 == 0 { app.word = (app.word + 1) % WORDS.len(); }
                    app.update_title();
                    if app.tick % 2 == 0 { if let Some(p) = &mut app.sessions_pick { p.marquee.1 += 1; } }
                    // cross-session: heartbeat every ~5s, poll our mailbox every ~2s
                    if app.tick % 60 == 0 { if app.session_meta.id.is_empty() { app.session_meta.id = harness::sessions::SessionStore::new_id(); } harness::mailbox::heartbeat(&harness::mailbox::Live { id: app.session_meta.id.clone(), title: if app.session_meta.title.is_empty() { "(new session)".into() } else { app.session_meta.title.clone() }, workdir: app.workdir.display().to_string(), pid: std::process::id(), backend: app.cfg.llm.provider.clone().unwrap_or("local".into()), updated: 0, busy: app.running.is_some() }); }
                    if app.tick % 25 == 3 && !app.cfg.ui.statusline.trim().is_empty() { app.refresh_statusline(); }
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
            // ctrl+g: suspend the UI, edit the prompt in $EDITOR, come back
            if app.edit_external {
                app.edit_external = false;
                let _ = crossterm::execute!(std::io::stdout(), crossterm::event::DisableMouseCapture, crossterm::event::DisableBracketedPaste);
                ratatui::restore();
                let edited = tokio::task::block_in_place(|| external_edit(&app.input));
                terminal = ratatui::init();
                let _ = crossterm::execute!(std::io::stdout(), crossterm::event::EnableBracketedPaste, crossterm::event::EnableMouseCapture);
                let _ = terminal.clear();
                match edited {
                    Ok(t) => { app.input = t; app.cursor = app.input.chars().count(); app.set_status("prompt edited in $EDITOR"); }
                    Err(e) => app.set_status(format!("editor: {e:#}")),
                }
            }
            // A command that wants the raw terminal (claude auth login): step out of the UI, run it, return.
            if let Some(cmd) = app.run_external.take() {
                let _ = crossterm::execute!(std::io::stdout(), crossterm::event::DisableMouseCapture, crossterm::event::DisableBracketedPaste);
                ratatui::restore();
                println!("\n· {cmd}\n");
                let status = tokio::task::block_in_place(|| std::process::Command::new("sh").arg("-c").arg(&cmd).status());
                terminal = ratatui::init();
                let _ = crossterm::execute!(std::io::stdout(), crossterm::event::EnableBracketedPaste, crossterm::event::EnableMouseCapture);
                let _ = terminal.clear();
                match status {
                    Ok(st) if st.success() => { app.set_status("checking Claude again"); let tx2 = app.tx.clone(); tokio::spawn(async move { let _ = tx2.send(Msg::ClaudeAuth(harness::claude_code::auth().await)); }); }
                    Ok(st) => app.blocks.push(Block::Error(format!("`{cmd}` exited with {st} — the harness will use Qwen3.8 once it has downloaded"))),
                    Err(e) => app.blocks.push(Block::Error(format!("could not run `{cmd}`: {e}"))),
                }
            }
            if app.quit { break; }
        }
        Ok(())
    }.await;
    { use std::io::Write; let mut o = std::io::stdout(); let _ = write!(o, "\x1b]0;\x07"); let _ = o.flush(); } // hand the title back to the shell
    if kbd_enh { let _ = crossterm::execute!(std::io::stdout(), crossterm::event::PopKeyboardEnhancementFlags); }
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
        self.banner_step = 0;
        self.blocks.push(Block::Startup(vec![
            format!("✻ TheHarness {} — local coding agent", harness::version()),
            format!("  model  {}", model_label(&self.model)),
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

    /// Terminal window/tab title: where you are, what is driving it, and whether it is busy — the
    /// things you want to see when the window is in the background. OSC 0 sets both icon and title.
    fn title_text(&self) -> String {
        let dir = self.workdir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| short_path(&self.workdir));
        let mut parts: Vec<String> = Vec::new();
        if self.running.is_some() {
            let secs = self.run_started.elapsed().as_secs();
            parts.push(format!("{} {}", SPINNER[(self.tick as usize / 4) % SPINNER.len()], if secs >= 60 { format!("{}m{:02}s", secs / 60, secs % 60) } else { format!("{secs}s") }));
        }
        parts.push(dir);
        parts.push(self.model.clone());
        if self.cfg.llm.provider.as_deref() == Some("claude-code") { if let Some(e) = &self.cfg.llm.effort { parts.push(format!("effort {e}")); } }
        match self.perm_mode {
            harness::permissions::Mode::Bypass => parts.push("bypass".into()),
            harness::permissions::Mode::Plan => parts.push("plan".into()),
            _ => {}
        }
        if !self.queued.is_empty() { parts.push(format!("{} queued", self.queued.len())); }
        if let Some(g) = &self.goal { parts.push(format!("goal: {}", truncate(g, 24))); }
        format!("{} — harness", parts.join(" · "))
    }

    /// Write the title when it changed (a redraw every 80ms would otherwise spam the terminal).
    fn update_title(&mut self) {
        let t = self.title_text();
        if self.title_shown.as_deref() == Some(t.as_str()) { return; }
        use std::io::Write;
        let mut o = std::io::stdout();
        let _ = write!(o, "\x1b]0;{t}\x07");
        let _ = o.flush();
        self.title_shown = Some(t);
    }

    fn set_status(&mut self, s: impl Into<String>) { self.status_msg = Some((s.into(), Instant::now())); }

    fn setting_value(&self, key: &str) -> String {
        let b = |v: bool| if v { "on" } else { "off" }.to_string();
        match key {
            "ui.tool_view" => self.tool_view.clone(), "ui.show_thinking" => b(self.show_thinking), "ui.panel" => match self.panel { Some(true) => "on".into(), Some(false) => "off".into(), None => "auto".into() },
            "ui.theme" => if LIGHT.load(std::sync::atomic::Ordering::Relaxed) { "light".into() } else { "dark".into() }, "ui.notify" => b(self.cfg.ui.notify), "ui.fold_previous" => b(self.cfg.ui.fold_previous), "ui.vim" => b(self.vim),
            "permissions.mode" => format!("{:?}", self.perm_mode).to_lowercase(), "llm.effort" => self.cfg.llm.effort.clone().unwrap_or("medium".into()), "llm.compact_at_fraction" => format!("{}", self.cfg.llm.compact_at_fraction),
            "memory.auto_reflect" => b(self.cfg.memory.auto_reflect), "security.redact_secrets" => b(self.cfg.security.redact_secrets), "net.enabled" => b(self.net), "agent.max_task_secs" => self.cfg.agent.max_task_secs.to_string(), "ui.event_log" => b(self.cfg.ui.event_log), "sandbox.mode" => if self.cfg.sandbox.mode.is_empty() { "none".into() } else { self.cfg.sandbox.mode.clone() },
            "llm.provider" => self.cfg.llm.provider.clone().unwrap_or("local (OpenAI-compatible server)".into()), "llm.model" => self.model.clone(), "ui.font_size" => self.cfg.ui.font_size.to_string(),
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
            "ui.font_size" => { if let Ok(n) = val.parse::<u32>() { if n > 0 { let tx = self.tx.clone(); tokio::spawn(async move { let _ = tx.send(Msg::Toast(match set_terminal_font_size(n).await { Ok(t) => format!("font size {n}pt ({t})"), Err(e) => e })); }); } } }
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
            CEvent::Mouse(m) if self.pick.is_some() => {
                let mut run: Option<String> = None;
                if let MouseEventKind::Down(MouseButton::Left) = m.kind {
                    if let Some(p) = &mut self.pick {
                        let rows = p.rows;
                        if m.row >= rows.y && m.row < rows.y + rows.height {
                            let idx = p.top + (m.row - rows.y) as usize;
                            if idx < p.matches().len() {
                                // click selects, clicking the selected row again runs it
                                if p.cursor == idx { run = p.selected().and_then(|i| i.run.clone()); } else { p.cursor = idx; }
                            }
                        }
                    }
                }
                if let Some(cmd) = run { self.pick = None; self.command(&cmd); }
            }
            CEvent::Mouse(m) if self.sessions_pick.is_some() => {
                let mut resume: Option<String> = None;
                if let Some(p) = &mut self.sessions_pick {
                    match m.kind {
                        MouseEventKind::ScrollUp => p.mv(-1), MouseEventKind::ScrollDown => p.mv(1),
                        MouseEventKind::Down(MouseButton::Left) => {
                            let r = p.rows;
                            if m.column >= r.x && m.column < r.x + r.width && m.row >= r.y && m.row < r.y + r.height {
                                let idx = p.top + (m.row - r.y) as usize;
                                if idx < p.filtered().len() { if p.cursor == idx { resume = p.selected_id(); } else { p.cursor = idx; } }
                            }
                        }
                        _ => {}
                    }
                }
                if let Some(id) = resume { self.sessions_pick = None; if self.running.is_some() { self.set_status("finish or interrupt the current task first"); } else { self.resume_session(&id); } }
            }
            CEvent::Mouse(m) => {
                let in_panel = self.panel_rect.width > 0 && m.column >= self.panel_rect.x && m.column < self.panel_rect.x + self.panel_rect.width;
                match m.kind {
                    MouseEventKind::ScrollUp => { if in_panel { self.think_scroll += 3; } else { self.scroll_up += 3; } }
                    MouseEventKind::ScrollDown => { if in_panel { self.think_scroll = self.think_scroll.saturating_sub(3); } else { self.scroll_up = self.scroll_up.saturating_sub(3); } }
                    MouseEventKind::Down(MouseButton::Left) => { self.sel_anchor = Some((m.column, m.row)); self.sel_cur = Some((m.column, m.row)); self.sel_dragging = false; }
                    MouseEventKind::Drag(MouseButton::Left) => { if self.sel_anchor.is_some() { self.sel_cur = Some((m.column, m.row)); self.sel_dragging = true; } }
                    MouseEventKind::Up(MouseButton::Left) => {
                        if let (Some(a), Some(c)) = (self.sel_anchor, self.sel_cur) {
                            if self.sel_dragging && a != c { self.copy_selection(a, c); }
                            else if !in_panel {
                                let r = self.tr_rect;
                                if m.row >= r.y && m.row < r.y + r.height { let line = self.tr_start + (m.row - r.y) as usize; if let Some(&(_, _, idx)) = self.line_map.iter().find(|(x, y, _)| line >= *x && line < *y) { self.toggle_fold(idx); } }
                            }
                        }
                        self.sel_anchor = None; self.sel_cur = None; self.sel_dragging = false;
                    }
                    _ => {}
                }
            }
            CEvent::Key(k) if k.kind == KeyEventKind::Press && self.pick.is_some() => {
                let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
                let (mut run, mut close) = (None::<String>, false);
                if let Some(p) = &mut self.pick {
                    let empty_filter = p.filter.is_empty();
                    match k.code {
                        KeyCode::Esc => close = true,
                        KeyCode::Char('c') | KeyCode::Char('d') if ctrl => close = true,
                        KeyCode::Char('q') if empty_filter => close = true,
                        KeyCode::Up => p.mv(-1),
                        KeyCode::Down | KeyCode::Tab => p.mv(1),
                        KeyCode::Char('k') if empty_filter => p.mv(-1),
                        KeyCode::Char('j') if empty_filter => p.mv(1),
                        KeyCode::PageUp => p.mv(-10),
                        KeyCode::PageDown => p.mv(10),
                        KeyCode::Home => p.cursor = 0,
                        KeyCode::End => p.cursor = p.matches().len().saturating_sub(1),
                        KeyCode::Enter => { run = p.selected().and_then(|i| i.run.clone()); close = run.is_some(); }
                        KeyCode::Backspace => { p.filter.pop(); p.cursor = 0; }
                        KeyCode::Char('u') if ctrl => { p.filter.clear(); p.cursor = 0; }
                        KeyCode::Char(c) if !ctrl && !k.modifiers.contains(KeyModifiers::ALT) => { p.filter.push(c); p.cursor = 0; }
                        _ => {}
                    }
                }
                let closed = close || run.is_some();
                if close { self.pick = None; }
                if let Some(cmd) = run { self.command(&cmd); }
                // The first-run picker is a fork in the road either way: whichever branch was taken (a
                // build to download, "not now", or esc), the user still needs to be asked about Claude.
                if closed && std::mem::take(&mut self.first_run_pending) { self.offer_claude_meanwhile(); }
                else if closed && self.claude_asked && self.no_model && self.cfg.llm.provider.is_none() && !self.said_waiting {
                    self.said_waiting = true;
                    let dl = self.dl.is_some() || self.dl_cancel.is_some();
                    self.blocks.push(Block::System(if dl {
                        "no Claude, then — the harness will use Qwen3.8-27B as soon as the download finishes (⌃P watches it); turns before that will fail".into()
                    } else {
                        "no backend yet — /localmodel downloads Qwen3.8-27B, /backend claude uses Claude, or set [llm] base_url to a server you run".to_string()
                    }));
                }
            }
            CEvent::Key(k) if k.kind == KeyEventKind::Press && self.sessions_pick.is_some() => {
                let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
                let mut resume: Option<String> = None;
                if let Some(p) = &mut self.sessions_pick {
                    match k.code {
                        KeyCode::Esc => self.sessions_pick = None,
                        KeyCode::Char('c') | KeyCode::Char('d') if ctrl => self.sessions_pick = None,
                        KeyCode::Char('q') if p.filter.is_empty() => self.sessions_pick = None,
                        KeyCode::Up => p.mv(-1), KeyCode::Down | KeyCode::Tab => p.mv(1),
                        KeyCode::Char('k') if p.filter.is_empty() => p.mv(-1), KeyCode::Char('j') if p.filter.is_empty() => p.mv(1),
                        KeyCode::PageUp => p.mv(-10), KeyCode::PageDown => p.mv(10),
                        KeyCode::Home => p.cursor = 0, KeyCode::End => { p.cursor = p.filtered().len().saturating_sub(1); }
                        KeyCode::Enter => { resume = p.selected_id(); }
                        KeyCode::Backspace => { p.filter.pop(); p.cursor = 0; }
                        KeyCode::Char('u') if ctrl => { p.filter.clear(); p.cursor = 0; }
                        KeyCode::Char(c) if !ctrl && !k.modifiers.contains(KeyModifiers::ALT) => { p.filter.push(c); p.cursor = 0; }
                        _ => {}
                    }
                }
                if let Some(id) = resume { self.sessions_pick = None; if self.running.is_some() { self.set_status("finish or interrupt the current task first"); } else { self.resume_session(&id); } }
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
            CEvent::Key(k) if k.kind == KeyEventKind::Press && self.review.is_some() => {
                let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
                let n = self.review.as_ref().map(|r| r.hunks.len()).unwrap_or(0);
                // typing a comment for the current hunk
                if self.review.as_ref().map(|r| r.comment.is_some()).unwrap_or(false) {
                    let mut done: Option<String> = None;
                    if let Some(r) = &mut self.review {
                        match k.code {
                            KeyCode::Esc => { r.comment = None; }
                            KeyCode::Enter => { done = r.comment.take(); }
                            KeyCode::Backspace => { if let Some(c) = &mut r.comment { c.pop(); } }
                            KeyCode::Char(c) if !ctrl => { if let Some(b) = &mut r.comment { b.push(c); } }
                            _ => {}
                        }
                    }
                    if let Some(text) = done {
                        if let Some(r) = &mut self.review { let i = r.cursor; if !text.trim().is_empty() { r.hunks[i].comment = Some(text.trim().to_string()); } }
                        self.set_status("comment saved — q sends the review to the agent");
                    }
                    return;
                }
                match k.code {
                    KeyCode::Esc | KeyCode::Char('q') => self.close_review(k.code == KeyCode::Char('q')),
                    KeyCode::Char('c') if ctrl => self.close_review(false),
                    KeyCode::Up | KeyCode::Char('k') => { if let Some(r) = &mut self.review { r.cursor = r.cursor.saturating_sub(1); r.scroll = 0; } }
                    KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => { if let Some(r) = &mut self.review { if n > 0 { r.cursor = (r.cursor + 1).min(n - 1); r.scroll = 0; } } }
                    KeyCode::PageDown => { if let Some(r) = &mut self.review { r.scroll += 10; } }
                    KeyCode::PageUp => { if let Some(r) = &mut self.review { r.scroll = r.scroll.saturating_sub(10); } }
                    KeyCode::Char('a') => { if let Some(r) = &mut self.review { if n > 0 { let i = r.cursor; r.hunks[i].reverted = false; r.cursor = (i + 1).min(n - 1); } } }
                    KeyCode::Char('r') => self.revert_hunk(),
                    KeyCode::Char('m') => { if let Some(r) = &mut self.review { r.comment = Some(String::new()); } }
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
                if km.is("complete", k.code, k.modifiers) {
                    match self.selected_suggestion() {
                        Some(cmd) => { self.input = format!("{cmd} "); self.cursor = self.input.chars().count(); self.sugg_idx = None; }
                        None => self.complete_slash(),
                    }
                    return;
                }
                if km.is("interrupt", k.code, k.modifiers) && self.running.is_some() { self.interrupt(); return; }
                if km.is("font_bigger", k.code, k.modifiers) || (k.code == KeyCode::Char('+') && ctrl) { self.adjust_font(1); return; }
                if km.is("font_smaller", k.code, k.modifiers) || (k.code == KeyCode::Char('_') && ctrl) { self.adjust_font(-1); return; }
                if km.is("font_reset", k.code, k.modifiers) { self.adjust_font(0); return; }
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
                // ctrl+r: reverse history search (a mode of its own — keys mean different things)
                if self.hist_search.is_some() {
                    match (k.code, ctrl) {
                        (KeyCode::Esc, _) | (KeyCode::Char('c'), true) => { if let Some((_, saved, _)) = self.hist_search.take() { self.input = saved; self.cursor = self.input.chars().count(); } self.set_status(""); }
                        (KeyCode::Enter, _) => { self.hist_search = None; self.set_status(""); }
                        (KeyCode::Char('r'), true) => { if let Some((_, _, n)) = &mut self.hist_search { *n += 1; } self.apply_hist_search(); }
                        (KeyCode::Backspace, _) => { if let Some((q, _, n)) = &mut self.hist_search { q.pop(); *n = 0; } self.apply_hist_search(); }
                        (KeyCode::Char(c), false) => { if let Some((q, _, n)) = &mut self.hist_search { q.push(c); *n = 0; } self.apply_hist_search(); }
                        _ => {}
                    }
                    return;
                }
                match (k.code, ctrl, alt) {
                    (KeyCode::Char('r'), true, _) => { self.hist_search = Some((String::new(), self.input.clone(), 0)); self.apply_hist_search(); }
                    (KeyCode::Char('g'), true, _) => { self.edit_external = true; }
                    (KeyCode::Char('c'), true, _) => {
                        if self.running.is_some() { self.interrupt(); }
                        else if !self.input.is_empty() { self.input.clear(); self.cursor = 0; }
                        else if self.last_ctrl_c.map(|t| t.elapsed() < Duration::from_millis(1500)).unwrap_or(false) { self.quit = true; }
                        else { self.last_ctrl_c = Some(Instant::now()); self.set_status("Press ctrl+c again to exit"); }
                    }
                    (KeyCode::Esc, _, _) if self.sugg_idx.is_some() => { self.sugg_idx = None; }
                    (KeyCode::Esc, _, _) => { if self.restart_at.is_some() { self.cancel_restart(); } else if self.running.is_some() { self.interrupt(); } else if self.vim { self.vim_normal = true; } else if !self.input.is_empty() { self.input.clear(); self.cursor = 0; } }
                    (KeyCode::Enter, _, _) => {
                        // a highlighted suggestion is what enter runs
                        if let Some(cmd) = self.selected_suggestion() { self.input = cmd; self.cursor = self.input.chars().count(); self.sugg_idx = None; }
                        self.submit();
                    }
                    (KeyCode::Char('a'), true, _) | (KeyCode::Home, _, _) => self.cursor = self.line_start(),
                    (KeyCode::Char('e'), true, _) | (KeyCode::End, _, _) => self.cursor = self.line_end(),
                    (KeyCode::Backspace, _, _) => { self.sugg_idx = None; if self.cursor > 0 { let mut cs: Vec<char> = self.input.chars().collect(); cs.remove(self.cursor - 1); self.input = cs.into_iter().collect(); self.cursor -= 1; } }
                    (KeyCode::Delete, _, _) => { let mut cs: Vec<char> = self.input.chars().collect(); if self.cursor < cs.len() { cs.remove(self.cursor); self.input = cs.into_iter().collect(); } }
                    (KeyCode::Left, _, _) => { self.cursor = self.cursor.saturating_sub(1); }
                    (KeyCode::Right, _, _) => { self.cursor = (self.cursor + 1).min(self.input.chars().count()); }
                    (KeyCode::Up, _, _) => {
                        let n = suggestions(&self.input).len();
                        if n > 0 { self.sugg_idx = Some(match self.sugg_idx { Some(0) | None => n - 1, Some(i) => i - 1 }); }
                        else if !self.input.contains('\n') { self.history_prev(); }
                    }
                    (KeyCode::Down, _, _) => {
                        let n = suggestions(&self.input).len();
                        if n > 0 { self.sugg_idx = Some(match self.sugg_idx { Some(i) if i + 1 < n => i + 1, _ => 0 }); }
                        else if !self.input.contains('\n') { self.history_next(); }
                    }
                    (KeyCode::Char(c), false, false) => { self.insert_str(&c.to_string()); }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    /// The command the user has highlighted with ↑/↓, if the list is open.
    fn selected_suggestion(&self) -> Option<String> {
        let i = self.sugg_idx?;
        suggestions(&self.input).get(i).map(|(c, _)| c.to_string())
    }

    fn insert_str(&mut self, s: &str) {
        let mut cs: Vec<char> = self.input.chars().collect();
        for (i, c) in s.chars().enumerate() { cs.insert(self.cursor + i, c); }
        self.cursor += s.chars().count();
        self.input = cs.into_iter().collect();
        self.hist_idx = None;
        self.sugg_idx = None;
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
    /// Run the user's `[ui] statusline` command with a JSON snapshot on stdin; its first line is shown
    /// on the right of the mode line (like Claude Code's statusline hook).
    fn refresh_statusline(&mut self) {
        let cmd = self.cfg.ui.statusline.clone();
        if cmd.trim().is_empty() { return; }
        let payload = serde_json::json!({
            "model": self.model,
            "provider": self.cfg.llm.provider.clone().unwrap_or_else(|| "local".into()),
            "workdir": self.workdir.display().to_string(),
            "session_id": self.session_meta.id,
            "permission_mode": self.perm_mode.id(),
            "running": self.running.is_some(),
            "queued": self.queued.len(),
            "tokens": {"prompt": self.total_prompt, "completion": self.total_completion, "context": self.last_prompt_tokens, "window": self.metrics.ctx_len},
            "cost_usd": harness::pricing::spent_usd(),
            "version": harness::VERSION,
        });
        let (wd, tx) = (self.workdir.clone(), self.tx.clone());
        tokio::spawn(async move {
            let (prog, flag) = harness::sandbox::shell_program();
            let mut c = tokio::process::Command::new(prog);
            c.arg(flag).arg(&cmd).current_dir(&wd).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).kill_on_drop(true);
            let Ok(mut child) = c.spawn() else { return };
            if let Some(mut si) = child.stdin.take() { use tokio::io::AsyncWriteExt; let _ = si.write_all(payload.to_string().as_bytes()).await; }
            if let Ok(Ok(out)) = tokio::time::timeout(Duration::from_secs(5), child.wait_with_output()).await {
                let text = String::from_utf8_lossy(&out.stdout).lines().next().unwrap_or("").to_string();
                let _ = tx.send(Msg::StatusLine(text));
            }
        });
    }

    /// Open the hunk-by-hunk review over a `git diff` blob.
    fn open_review(&mut self, diff: String) {
        let hunks = parse_hunks(&diff);
        if hunks.is_empty() { self.blocks.push(Block::System("nothing to review — the working tree matches HEAD".into())); return; }
        self.set_status("");
        self.review = Some(DiffReview { hunks, cursor: 0, scroll: 0, comment: None });
    }

    /// `r`: revert the selected hunk on disk with `git apply -R`.
    fn revert_hunk(&mut self) {
        let Some(r) = &mut self.review else { return };
        if r.hunks.is_empty() { return; }
        let i = r.cursor;
        if r.hunks[i].reverted { self.set_status("already reverted"); return; }
        let patch = r.hunks[i].patch();
        let file = r.hunks[i].file.clone();
        let path = std::env::temp_dir().join(format!("harness-hunk-{}.patch", std::process::id()));
        if let Err(e) = std::fs::write(&path, &patch) { self.set_status(format!("cannot write the patch: {e}")); return; }
        let out = std::process::Command::new("git").arg("-C").arg(&self.workdir).args(["apply", "-R", "--recount", "--unidiff-zero"]).arg(&path).output();
        let _ = std::fs::remove_file(&path);
        match out {
            Ok(o) if o.status.success() => {
                if let Some(r) = &mut self.review { r.hunks[i].reverted = true; let n = r.hunks.len(); r.cursor = (i + 1).min(n - 1); }
                self.set_status(format!("reverted a hunk in {file}"));
            }
            Ok(o) => self.set_status(format!("git apply -R failed: {}", truncate(String::from_utf8_lossy(&o.stderr).trim(), 160))),
            Err(e) => self.set_status(format!("git apply -R: {e}")),
        }
    }

    /// Close the review; with `send`, hand the comments and reverts to the agent as a new turn.
    fn close_review(&mut self, send: bool) {
        let Some(r) = self.review.take() else { return };
        let comments: Vec<&Hunk> = r.hunks.iter().filter(|h| h.comment.is_some()).collect();
        let reverted: Vec<&Hunk> = r.hunks.iter().filter(|h| h.reverted).collect();
        if reverted.is_empty() && comments.is_empty() { self.set_status("review closed — nothing changed"); return; }
        let mut lines = vec![format!("review: {} hunk(s) reverted, {} comment(s)", reverted.len(), comments.len())];
        for h in &reverted { lines.push(format!("  reverted {} {}", h.file, h.header)); }
        for h in &comments { lines.push(format!("  {} {} — {}", h.file, h.header, h.comment.clone().unwrap_or_default())); }
        self.blocks.push(Block::Banner(lines));
        if !send || comments.is_empty() {
            if !comments.is_empty() { self.set_status("comments kept in the transcript (q sends them to the agent)"); }
            return;
        }
        let mut prompt = String::from("I reviewed your changes hunk by hunk. Address these comments (and do not undo my reverts):\n");
        for h in &comments {
            prompt.push_str(&format!("\n## {} {}\n{}\ncomment: {}\n", h.file, h.header, h.body.iter().take(40).cloned().collect::<Vec<_>>().join("\n"), h.comment.clone().unwrap_or_default()));
        }
        if !reverted.is_empty() {
            prompt.push_str("\nHunks I reverted on disk (they are gone; do not reapply them unless I ask):\n");
            for h in &reverted { prompt.push_str(&format!("- {} {}\n", h.file, h.header)); }
        }
        self.start_run(prompt);
    }

    /// ctrl+r: show the newest history entry containing the query (ctrl+r again cycles older ones).
    fn apply_hist_search(&mut self) {
        let Some((q, saved, n)) = self.hist_search.clone() else { return };
        let hits: Vec<&String> = if q.is_empty() { self.history.iter().rev().collect() } else { self.history.iter().rev().filter(|h| h.to_lowercase().contains(&q.to_lowercase())).collect() };
        if hits.is_empty() {
            self.input = saved.clone(); self.cursor = self.input.chars().count();
            self.set_status(format!("(reverse-i-search) '{q}': no match — esc cancels"));
            return;
        }
        let idx = n % hits.len();
        self.input = hits[idx].clone();
        self.cursor = self.input.chars().count();
        self.set_status(format!("(reverse-i-search) '{q}' [{}/{}] — ctrl+r older · enter accepts · esc cancels", idx + 1, hits.len()));
    }

    /// tab: complete a leading /command, or an @path anywhere in the prompt.
    fn complete_slash(&mut self) {
        if self.input.starts_with('/') && !self.input.contains(' ') {
            let m: Vec<&str> = COMMANDS.iter().map(|c| c.0).filter(|c| c.starts_with(&self.input)).collect();
            if m.len() == 1 { self.input = format!("{} ", m[0]); self.cursor = self.input.chars().count(); }
            else if m.len() > 1 {
                if let Some(common) = common_prefix(&m) { if common.len() > self.input.len() { self.input = common; self.cursor = self.input.chars().count(); } }
                self.set_status(m.iter().take(12).cloned().collect::<Vec<_>>().join("  "));
            }
            return;
        }
        self.complete_path();
    }

    /// `@src/to` → completes against the working directory (files and dirs, ignoring dot/vendored ones).
    fn complete_path(&mut self) {
        let cs: Vec<char> = self.input.chars().collect();
        let mut start = self.cursor;
        while start > 0 && !cs[start - 1].is_whitespace() { start -= 1; }
        let token: String = cs[start..self.cursor].iter().collect();
        let Some(frag) = token.strip_prefix('@') else { return };
        let (dir_part, file_part) = match frag.rsplit_once('/') { Some((d, f)) => (d.to_string(), f.to_string()), None => (String::new(), frag.to_string()) };
        let base = if dir_part.is_empty() { self.workdir.clone() } else { self.workdir.join(&dir_part) };
        let Ok(rd) = std::fs::read_dir(&base) else { self.set_status(format!("no such directory: {}", base.display())); return };
        let mut names: Vec<String> = rd.flatten().filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || matches!(name.as_str(), "node_modules" | "target" | "dist" | "build") { return None; }
            if !name.to_lowercase().starts_with(&file_part.to_lowercase()) { return None; }
            Some(if e.path().is_dir() { format!("{name}/") } else { name })
        }).collect();
        names.sort();
        if names.is_empty() { self.set_status(format!("no match for @{frag}")); return; }
        let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
        let completion = if names.len() == 1 { names[0].clone() } else { common_prefix(&refs).unwrap_or_else(|| file_part.clone()) };
        let full = if dir_part.is_empty() { completion.clone() } else { format!("{dir_part}/{completion}") };
        let mut new_input: String = cs[..start].iter().collect();
        new_input.push('@');
        new_input.push_str(&full);
        let tail: String = cs[self.cursor..].iter().collect();
        self.cursor = new_input.chars().count();
        self.input = format!("{new_input}{tail}");
        if names.len() > 1 { self.set_status(names.iter().take(12).cloned().collect::<Vec<_>>().join("  ")); }
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
        self.video = Some(VideoPicker { path: path.clone(), duration: 0.0, frames: vec![], cur: 0, selected: Default::default(), loading: true, error: None, note: String::new(), preview: None, source: String::new() });
        let tx = self.tx.clone();
        tokio::spawn(async move { let _ = tx.send(Msg::Frames(extract_frames(path, out_dir, 1200).await)); });
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
            if video_ext(&p) && p.is_file() && !self.input.contains("@") {
                // `/video <path>`: the command is consumed; the frame labels the scrubber inserts must land in an
                // empty prompt, not after "/video …" (which would re-run /video with a mangled path on enter).
                if text.starts_with("/video") { self.input.clear(); self.cursor = 0; }
                self.open_video(&p); return;
            }
        }
        let text = if text.is_empty() { "Look at the attached image(s).".to_string() } else { text };
        self.input.clear(); self.cursor = 0; self.hist_idx = None;
        if self.history.last() != Some(&text) { self.history.push(text.clone()); }
        if text.starts_with('/') { self.command(&text); return; }
        // `!cmd` runs a shell command here and shows the output; the model never sees it
        if let Some(cmd) = text.strip_prefix('!') {
            let cmd = cmd.trim().to_string();
            if cmd.is_empty() { self.blocks.push(Block::System("!<command> runs a shell command in the working directory (the model does not see it)".into())); return; }
            self.blocks.push(Block::User(format!("!{cmd}"), vec![]));
            let (wd, tx) = (self.workdir.clone(), self.tx.clone());
            tokio::spawn(async move {
                let o = harness::sandbox::run_shell(&cmd, &wd, Duration::from_secs(120), 20000).await;
                let body = match o { Ok(o) => { let mut t = o.stdout.clone(); if !o.stderr.trim().is_empty() { t.push_str(&format!("\n{}", o.stderr)); } if !o.success() { t.push_str(&format!("\n[exit {}]", o.code.unwrap_or(-1))); } t } Err(e) => format!("{e:#}") };
                let lines: Vec<String> = std::iter::once(format!("$ {cmd}")).chain(body.lines().take(200).map(String::from)).collect();
                let _ = tx.send(Msg::Block(Block::Banner(lines)));
            });
            return;
        }
        if let Some(id) = self.attached {
            if let Some(a) = self.subenv.as_ref().and_then(|e| e.list().into_iter().find(|a| a.id == id)) {
                if a.running() { a.inbox.push("message from the user (attached)", text.clone()); self.blocks.push(Block::System(format!("→ {} (delivered before its next model call): {}", a.label, truncate(&text, 120)))); return; }
                self.set_status(format!("sub-agent #{id} is finished — /agents detach")); return;
            }
        }
        if self.running.is_some() {
            if self.cfg.ui.steer {
                // steer: the running agent picks this up at its next tool boundary
                self.blocks.push(Block::User(format!("[steering] {text}"), vec![]));
                self.inbox.push("message from the user", text);
                self.set_status("steering the running task (it sees this before its next model call) — /queue <text> to queue instead");
            } else {
                self.queued.push(text);
                self.set_status(format!("queued ({} waiting) — will run after the current turn", self.queued.len()));
            }
            return;
        }
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
                lines.push("Mouse: wheel/trackpad scrolls; click a block to fold/unfold; left-drag selects text and copies it to the clipboard (toast confirms). Shift-drag (or fn/option in Terminal.app) uses the terminal's own selection.".into());
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
            "/sessions" if arg.is_empty() || arg == "pick" => {
                if let Some(q) = arg.strip_prefix("search ") {
                    let q = q.trim().to_string();
                    match harness::sessions::SessionStore::open() {
                        Ok(store) => {
                            let hits = store.search(&q, None, 25);
                            if hits.is_empty() { self.blocks.push(Block::System(format!("no session mentions '{q}'"))); }
                            else {
                                let mut lines = vec![format!("{} session(s) mention '{q}' — /resume <id>", hits.len())];
                                for (m, line) in hits { lines.push(format!("  {}  {:<40} {}", m.id, truncate(&m.title, 40), harness::sessions::fmt_age(m.updated))); lines.push(format!("      {}", truncate(&line, 110))); }
                                self.blocks.push(Block::Banner(lines));
                            }
                        }
                        Err(e) => self.blocks.push(Block::Error(e.to_string())),
                    }
                    return;
                }
                match harness::sessions::SessionStore::open() {
                    Ok(store) => { let all = store.list(None); if all.is_empty() { self.blocks.push(Block::Banner(vec!["no saved sessions yet".into()])); } else { self.sessions_pick = Some(SessionPicker { all, cursor: 0, filter: String::new(), top: 0, rows: Rect::default(), marquee: (0, 0) }); } }
                    Err(e) => self.blocks.push(Block::Error(format!("sessions: {e}"))),
                }
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
                    // Everything you could switch to, in one list: Claude (when its CLI is installed), the
                    // weights the harness downloaded itself, and whatever the local server is offering.
                    // Entries switch *backend as well as model*, so picking Claude from a local session — or
                    // the reverse — does the whole job rather than leaving a model the backend cannot serve.
                    let cur = self.model.clone();
                    let on_claude = self.cfg.llm.provider.as_deref() == Some("claude-code");
                    let effort = self.cfg.llm.effort.clone().unwrap_or_else(|| "high".into());
                    let mut items: Vec<PickItem> = Vec::new();
                    if harness::claude_code::claude_bin().is_some() {
                        items.extend(CLAUDE_MODELS.iter().map(|(m, note)| PickItem {
                            label: (*m).to_string(),
                            desc: format!("Claude · {note}{}", if on_claude && *m == cur { " · ● current" } else { "" }),
                            detail: format!("{m} through the official Claude Code CLI on your Anthropic subscription, with the \
                                             harness's tools bridged over MCP.\n\nenter switches the backend to Claude at effort {effort} (/effort changes it)"),
                            run: Some(format!("/backend claude {m} {effort}")),
                        }));
                    }
                    // Our own build, when the weights are complete but nothing is serving them yet.
                    if let Some(b) = harness::localmodel::by_name(&self.cfg.local_model.build) {
                        if matches!(harness::localmodel::state_of(b), harness::localmodel::ModelState::Ready { .. }) && !self.models.iter().any(|m| m.contains(b.name())) {
                            items.push(PickItem {
                                label: b.name().to_string(),
                                desc: format!("local · {} GB on disk · MLX server not running", b.bytes / 1_000_000_000),
                                detail: format!("{}\n\nThe weights are in ~/.config/harness/models. enter starts the MLX server \
                                                 ({}) on 127.0.0.1:{} and points the harness at it — loading takes a moment.",
                                                b.repo, self.cfg.local_model.server, self.cfg.local_model.port),
                                run: Some("/localmodel serve".into()),
                            });
                        }
                    }
                    items.extend(self.models.iter().map(|m| PickItem {
                        label: model_label(m).to_string(),
                        desc: format!("local · {}{}", self.cfg.llm.base_url.trim_start_matches("http://").trim_start_matches("https://").trim_end_matches("/v1"), if !on_claude && *m == cur { " · ● current" } else { "" }),
                        detail: format!("{}\n\nenter switches to this model on {}{}", m, self.cfg.llm.base_url,
                            harness::pricing::price_of(m).map(|p| format!("\nprice: ${}/1M in · ${}/1M out", p.input, p.output)).unwrap_or_else(|| "\nno published price — treated as free in /cost".into())),
                        run: Some(format!("/backend local {m}")),
                    }));
                    if items.is_empty() {
                        self.blocks.push(Block::System(format!("current: {} — nothing to switch to: {} has not answered /models, no Claude CLI, no local build. /localmodel downloads one.", self.model, self.cfg.llm.base_url)));
                        return;
                    }
                    self.pick = Some(ListPicker::new("Models", "enter switches backend + model · type to filter · esc closes", items));
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
                let items = defs.into_iter().map(|d| PickItem {
                    label: d.function.name.clone(),
                    desc: d.function.description.lines().next().unwrap_or("").to_string(),
                    detail: format!("{}\n\nparameters:\n{}", d.function.description, serde_json::to_string_pretty(&d.function.parameters).unwrap_or_default()),
                    run: None,
                }).collect();
                self.pick = Some(ListPicker::new("Tools the model can call", "↑/↓ move · type to filter · esc closes", items));
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
                            let (n, summary, mb, ma) = harness::agent::compact_llm_with(&client.role("compaction"), &mut msgs, 4, focus.as_deref(), Some(&sink)).await.map_err(|e| format!("{e:#}"))?;
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
            "/cost" | "/stats" => {
                let mut lines = vec![format!("session tokens: {} prompt + {} completion · last context {} · turns in history {}", self.total_prompt, self.total_completion, self.last_prompt_tokens, self.history.len())];
                match harness::pricing::price_of(&self.model) {
                    Some(p) => {
                        let spent = harness::pricing::spent_usd();
                        lines.push(format!("cost: {} so far ({} in / {} out per 1M tokens for {}){}",
                            harness::pricing::fmt_usd(spent), harness::pricing::fmt_usd(p.input), harness::pricing::fmt_usd(p.output), self.model,
                            harness::pricing::budget_usd().map(|b| format!(" · budget {}", harness::pricing::fmt_usd(b))).unwrap_or_default()));
                        if !arg.trim().is_empty() {
                            match arg.trim().parse::<f64>() {
                                Ok(b) => { harness::pricing::set_budget(Some(b)); lines.push(format!("budget set: the agent stops once this session costs {}", harness::pricing::fmt_usd(b))); }
                                Err(_) => lines.push("usage: /cost <max-usd> to set a budget".into()),
                            }
                        }
                    }
                    None => lines.push(format!("cost: {} runs locally — no token cost (add [llm.pricing] to price a hosted model)", self.model)),
                }
                self.blocks.push(Block::Banner(lines));
            }
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
            "/theme" => {
                let want = arg.trim().to_string();
                match want.as_str() {
                    "" => { let light = !LIGHT.load(std::sync::atomic::Ordering::Relaxed); *CUSTOM.lock().unwrap() = None; LIGHT.store(light, std::sync::atomic::Ordering::Relaxed); self.blocks.push(Block::System(format!("theme → {} · available: {}", if light { "light" } else { "dark" }, theme_names().join(", ")))); }
                    "light" | "dark" => { *CUSTOM.lock().unwrap() = None; LIGHT.store(want == "light", std::sync::atomic::Ordering::Relaxed); self.blocks.push(Block::System(format!("theme → {want}"))); }
                    "list" => self.blocks.push(Block::Banner(vec![format!("themes: {}", theme_names().join(", ")), format!("custom themes are JSON files in {} — keys: base (dark|light), orange, dim, ok, err, think, blue, pink, cyan, fg, panel_bg (\"#rrggbb\")", harness::setup::config_dir().join("themes").display())])),
                    name => match load_theme(name) {
                        Ok(()) => self.blocks.push(Block::System(format!("theme → {name}"))),
                        Err(e) => self.blocks.push(Block::Error(format!("theme {name}: {e} · /theme list"))),
                    },
                }
            }
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
                    if l.is_empty() { self.blocks.push(Block::System("no workflows — add ~/.config/harness/workflows/*.toml or .harness/workflows/*.toml".into())); return; }
                    let items = l.into_iter().map(|(n, d, path)| PickItem {
                        desc: d.clone(),
                        detail: format!("{}\n\n{}", path.display(), std::fs::read_to_string(&path).unwrap_or_default().trim()),
                        run: Some(format!("/workflow {n}")),
                        label: n,
                    }).collect();
                    self.pick = Some(ListPicker::new("Workflows", "enter runs one · type to filter · esc closes", items));
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
                                    let ctx = ToolCtx { memory: store.clone(), subagent: Some(env.clone()), redact_secrets: cfg.security.redact_secrets, injection_scan: cfg.security.injection_scan, hooks: cfg.hooks.clone(), lsp_servers: cfg.lsp.servers.clone(), todos, timeout: Duration::from_secs(cfg.agent.tool_timeout_secs), max_output: cfg.agent.max_tool_output_chars, net: cfg.net.clone(), ..ToolCtx::basic(workdir.clone()) };
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
                    "  left-drag        select text anywhere → copied to the clipboard (toast)".into(),
                    "  ctrl+= / ctrl+-  bigger / smaller terminal font (kitty, iTerm2, Terminal.app) · ctrl+0 reset · remembered".into(),
                    "  text selection:  shift-drag (kitty/wezterm/iterm) or fn/option (Terminal.app) = terminal-native selection".into(),
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
            "/export" | "/share" => {
                let as_html = cmd == "/share" || arg.contains("html");
                let gist = arg.contains("gist");
                let session = self.session.clone(); let tx = self.tx.clone();
                let mut meta = self.session_meta.clone();
                meta.workdir = self.workdir.display().to_string(); meta.model = self.model.clone();
                tokio::spawn(async move {
                    let msgs = session.lock().await.clone();
                    meta.turns = msgs.iter().filter(|m| m.role == "user").count();
                    let r = harness::export::write(&meta, &msgs, as_html);
                    let msg = match r {
                        Err(e) => format!("export failed: {e}"),
                        Ok(path) => {
                            let mut m = format!("exported to {}", path.display());
                            if gist {
                                let o = tokio::process::Command::new("gh").args(["gist", "create", "--desc", "harness session"]).arg(&path).output().await;
                                match o {
                                    Ok(o) if o.status.success() => m.push_str(&format!(" · gist: {}", String::from_utf8_lossy(&o.stdout).trim())),
                                    Ok(o) => m.push_str(&format!(" · gh gist failed: {}", String::from_utf8_lossy(&o.stderr).trim())),
                                    Err(e) => m.push_str(&format!(" · gh not available ({e})")),
                                }
                            } else if as_html {
                                m.push_str(" — open it in a browser, or /share gist to upload it with gh");
                            }
                            m
                        }
                    };
                    let _ = tx.send(Msg::Notice(msg));
                });
            }
            "/import" => {
                let arg2 = arg.clone(); let tx = self.tx.clone();
                tokio::spawn(async move {
                    let files = tokio::task::spawn_blocking(move || {
                        let p = std::path::PathBuf::from(&arg2);
                        if !arg2.is_empty() && p.is_file() { return vec![p]; }
                        let sources: Vec<&str> = match arg2.trim() { "claude" => vec!["claude"], "codex" => vec!["codex"], _ => vec!["claude", "codex"] };
                        let mut v = harness::import::discover(&sources); v.truncate(10); v
                    }).await.unwrap_or_default();
                    if files.is_empty() { let _ = tx.send(Msg::Notice("no Claude Code / Codex transcripts found".into())); return; }
                    let mut lines = vec![format!("Imported sessions (newest {}): resume one with /resume <id>", files.len())];
                    for f in files {
                        match harness::import::import_file(&f) {
                            Ok(i) => lines.push(format!("  {:<7} {:<24} {:>4} msgs  {}", i.source, i.id, i.messages, truncate(&i.title, 60))),
                            Err(e) => lines.push(format!("  skip {}: {e:#}", f.display())),
                        }
                    }
                    let _ = tx.send(Msg::Block(Block::Banner(lines)));
                });
            }
            "/todos" => { let t = self.todos.lock().map(|t| t.clone()).unwrap_or_default(); if t.is_empty() { self.blocks.push(Block::System("no todos (the agent maintains them with the todo tool)".into())); } else { self.blocks.push(Block::Banner(std::iter::once("Todos".to_string()).chain(t.iter().map(|x| format!("  {}", x.line(&t)))).collect())); } }
            "/hooks" => { let h = &self.cfg.hooks; self.blocks.push(Block::Banner(vec!["Hooks (harness.toml [hooks]) — JSON on stdin; pre_tool exit 2 blocks the call".into(), format!("  pre_tool  {:?}", h.pre_tool), format!("  post_tool {:?}", h.post_tool), format!("  on_stop   {:?}", h.on_stop), format!("  on_prompt {:?}", h.on_prompt), format!("  timeout   {}s", h.timeout_secs)])); }
            "/skills" => {
                let sk = harness::skills::discover(&self.workdir);
                if sk.is_empty() { self.blocks.push(Block::System("no skills found — add .harness/skills/<name>/SKILL.md, ~/.claude/skills/…, or install a plugin (/plugin list)".into())); return; }
                let items = sk.into_iter().map(|x| PickItem {
                    desc: format!("{}  [{}]", x.description, x.source),
                    detail: format!("{}\n{}\n{}", x.path.display(), if x.allowed_tools.is_empty() { String::new() } else { format!("tools: {}", x.allowed_tools.join(", ")) }, x.body().trim()),
                    label: x.name,
                    run: None,
                }).collect();
                self.pick = Some(ListPicker::new("Skills", "the model loads one with load_skill · type to filter · esc closes", items));
            }
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
            "/undo" | "/redo" => {
                if self.running.is_some() { self.set_status("finish or interrupt first"); return; }
                let steps = arg.trim().parse::<usize>().unwrap_or(1);
                let (sid, wd, tx, redo) = (self.session_meta.id.clone(), self.workdir.clone(), self.tx.clone(), cmd == "/redo");
                if sid.is_empty() { self.blocks.push(Block::Error("no checkpoints yet in this session".into())); return; }
                tokio::spawn(async move {
                    let msg = tokio::task::spawn_blocking(move || {
                        let Some(cp) = harness::checkpoints::for_session(&sid, &wd) else { return "file checkpoints are disabled ([checkpoints] enabled = false)".to_string() };
                        match if redo { cp.redo(steps) } else { cp.undo(steps) } {
                            Ok((c, n)) => format!("{} to checkpoint #{} — {} ({} file(s) restored). {} to go the other way.", if redo { "redone" } else { "undone" }, cp.cursor() + 1, c.label, n, if redo { "/undo" } else { "/redo" }),
                            Err(e) => format!("{e:#}"),
                        }
                    }).await.unwrap_or_else(|e| format!("{e}"));
                    let _ = tx.send(Msg::Notice(msg));
                });
            }
            "/checkpoints" | "/rewind" => {
                if self.running.is_some() { self.set_status("finish or interrupt first"); return; }
                let a = arg.trim().to_string();
                // conversation-only rewind (the old /rewind behaviour)
                if cmd == "/rewind" && (a == "conv" || a == "conversation" || a == "chat") {
                    let session = self.session.clone(); let tx = self.tx.clone();
                    if let Some(i) = self.blocks.iter().rposition(|b| matches!(b, Block::User(..))) { self.blocks.truncate(i); }
                    tokio::spawn(async move {
                        let mut msgs = session.lock().await;
                        if let Some(i) = msgs.iter().rposition(|m| m.role == "user" && !m.text().starts_with("[harness]")) { msgs.truncate(i); }
                        let n = msgs.len();
                        let _ = tx.send(Msg::Notice(format!("rewound the conversation to before the last turn ({n} messages kept); files were not touched (/undo reverts files)")));
                    });
                    return;
                }
                let sid = self.session_meta.id.clone();
                let Some(cp) = (if sid.is_empty() { None } else { harness::checkpoints::for_session(&sid, &self.workdir) }) else {
                    self.blocks.push(Block::System("no checkpoints in this session yet (they are taken before every file-changing tool call; disable with [checkpoints] enabled = false)".into())); return;
                };
                let list = cp.list();
                if a.is_empty() {
                    if list.is_empty() { self.blocks.push(Block::System("no checkpoints yet in this session".into())); return; }
                    let cursor = cp.cursor();
                    let items: Vec<PickItem> = list.iter().enumerate().map(|(i, c)| PickItem {
                        label: format!("#{}", i + 1),
                        desc: format!("{}{:<9} {:>3} file(s)  {}", if i == cursor { "▸ current  " } else { "           " }, harness::sessions::fmt_age(c.time), c.changed, c.label),
                        detail: format!("{}\n{} · {} file(s) changed · {}\n\nenter rewinds files AND the conversation to this point\n/rewind code {} restores only the files · /undo · /redo", c.label, &c.id[..8.min(c.id.len())], c.changed, harness::sessions::fmt_age(c.time), i + 1),
                        run: Some(format!("/rewind {}", i + 1)),
                    }).rev().collect();
                    self.pick = Some(ListPicker::new("Checkpoints", "enter rewinds here · type to filter · esc closes", items));
                    return;
                }
                let (code_only, which) = match a.strip_prefix("code ") { Some(r) => (true, r.trim().to_string()), None => (false, a.clone()) };
                let (tx, session) = (self.tx.clone(), self.session.clone());
                let target = list.iter().enumerate().find(|(i, c)| which.parse::<usize>() == Ok(i + 1) || c.id.starts_with(&which)).map(|(_, c)| c.clone());
                let Some(target) = target else { self.blocks.push(Block::Error(format!("no checkpoint '{which}' — /checkpoints"))); return };
                if !code_only && target.msgs > 0 { if let Some(i) = self.blocks.iter().rposition(|b| matches!(b, Block::User(..))) { let _ = i; } self.blocks.push(Block::System(format!("rewinding to checkpoint — {}", truncate(&target.label, 70)))); }
                let keep = target.msgs;
                tokio::spawn(async move {
                    let restored = tokio::task::spawn_blocking(move || match cp.restore(&which) { Ok((c, n)) => format!("files restored to '{}' ({n} changed)", truncate(&c.label, 60)), Err(e) => format!("restore failed: {e:#}") }).await.unwrap_or_else(|e| e.to_string());
                    let mut extra = String::new();
                    if !code_only && keep > 0 {
                        let mut msgs = session.lock().await;
                        if keep < msgs.len() { msgs.truncate(keep); extra = format!(" · conversation truncated to {keep} messages"); }
                    }
                    let _ = tx.send(Msg::Notice(format!("{restored}{extra}")));
                });
            }
            "/fork" => {
                if self.running.is_some() { self.set_status("finish or interrupt first"); return; }
                let old = self.session_meta.id.clone();
                self.session_meta.id = harness::sessions::SessionStore::new_id();
                self.session_meta.title = format!("{} (fork)", truncate(&self.session_meta.title, 60));
                self.save_session();
                self.blocks.push(Block::System(format!("forked session {old} → {} — this branch is saved separately; the original is untouched (/resume {old} to go back)", self.session_meta.id)));
            }
            "/arena" => {
                if self.running.is_some() { self.set_status("finish or interrupt the current task first"); return; }
                let (spec, task) = match arg.split_once("--") { Some((m, t)) => (m.trim().to_string(), t.trim().to_string()), None => (String::new(), arg.clone()) };
                if task.is_empty() {
                    self.blocks.push(Block::Banner(vec![
                        "/arena <models> -- <task>   run the same task on several models in isolated worktrees, then judge".into(),
                        "  /arena -- fix the failing test            three runs of the current model (best-of-3)".into(),
                        "  /arena a,b -- add a --json flag           one run each on models a and b".into(),
                        "  /arena qwen3.8-27b-mlx x3 -- <task>       three runs of one model".into(),
                    ]));
                    return;
                }
                let spec = if spec.is_empty() { format!("{} x3", self.model) } else { spec };
                let models = harness::arena::parse_models(&spec, &self.model);
                if models.len() < 2 { self.blocks.push(Block::Error("an arena needs at least two contenders".into())); return; }
                self.blocks.push(Block::System(format!("arena: {} contenders ({}) — each in its own worktree; this takes a while", models.len(), models.join(", "))));
                let (cfg, wd, tx) = (self.cfg.clone(), self.workdir.clone(), self.tx.clone());
                tokio::spawn(async move {
                    let sink: Arc<dyn Sink> = Arc::new(TuiSink(tx.clone()));
                    match harness::arena::run(&cfg, &wd, &task, &models, sink, true).await {
                        Ok(r) => { let _ = tx.send(Msg::Block(Block::Banner(harness::arena::render(&r)))); }
                        Err(e) => { let _ = tx.send(Msg::Block(Block::Error(format!("arena: {e:#}")))); }
                    }
                });
            }
            "/review-diff" | "/hunks" => {
                if self.running.is_some() { self.set_status("finish or interrupt the current task first"); return; }
                let (wd, tx) = (self.workdir.clone(), self.tx.clone());
                self.set_status("collecting the diff…");
                tokio::spawn(async move {
                    let o = harness::sandbox::run_shell("git diff HEAD -- . 2>/dev/null || git diff", &wd, Duration::from_secs(30), 400_000).await;
                    let diff = o.map(|o| o.stdout).unwrap_or_default();
                    let _ = tx.send(Msg::Review(diff));
                });
            }
            "/dream" => {
                let (cfg, tx) = (self.cfg.clone(), self.tx.clone());
                self.set_status("consolidating memory…");
                self.blocks.push(Block::System("dreaming: merging duplicates, dropping stale notes, drafting skills from what was learned…".into()));
                tokio::spawn(async move {
                    let Ok(store) = harness::memory::MemoryStore::open(&cfg.memory) else { let _ = tx.send(Msg::Notice("memory is disabled".into())); return };
                    let Ok(client) = Client::new(&cfg.llm) else { return };
                    let msg = match store.dream(&client.role("memory")).await {
                        Ok((files, skills)) => {
                            let mut m = if files.is_empty() { "memory was already tidy".to_string() } else { format!("consolidated: {}", files.join(", ")) };
                            if !skills.is_empty() { m.push_str(&format!(" · drafted skill(s): {} (load_skill them, or edit under {}/skills)", skills.join(", "), store.dir.display())); }
                            m
                        }
                        Err(e) => format!("dream failed: {e:#}"),
                    };
                    let _ = tx.send(Msg::Notice(msg));
                });
            }
            "/voice" => {
                let secs: u64 = arg.trim().parse().unwrap_or(8);
                let tx = self.tx.clone();
                let (rec, transcribe) = (voice_record_command(), voice_transcribe_command());
                let (Some(rec), Some(transcribe)) = (rec, transcribe) else {
                    self.blocks.push(Block::Error("voice needs a recorder (ffmpeg or sox/arecord) and whisper.cpp (`whisper-cli`/`whisper`) or openai-whisper on PATH".into()));
                    return;
                };
                self.set_status(format!("recording {secs}s… (speak now)"));
                let wd = self.workdir.clone();
                tokio::spawn(async move {
                    let wav = std::env::temp_dir().join(format!("harness-voice-{}.wav", std::process::id()));
                    let w = wav.display().to_string();
                    let r = harness::sandbox::run_shell(&rec.replace("{secs}", &secs.to_string()).replace("{out}", &w), &wd, Duration::from_secs(secs + 20), 4000).await;
                    if r.map(|o| !wav.is_file() || o.timed_out).unwrap_or(true) { let _ = tx.send(Msg::Notice("recording failed".into())); return; }
                    let t = harness::sandbox::run_shell(&transcribe.replace("{in}", &w), &wd, Duration::from_secs(180), 20000).await;
                    let _ = std::fs::remove_file(&wav);
                    match t {
                        Ok(o) if o.success() => {
                            let text = o.stdout.lines().filter(|l| !l.trim().is_empty() && !l.starts_with('[')).collect::<Vec<_>>().join(" ").trim().to_string();
                            let _ = tx.send(if text.is_empty() { Msg::Notice("nothing transcribed".into()) } else { Msg::Dictated(text) });
                        }
                        Ok(o) => { let _ = tx.send(Msg::Notice(format!("transcription failed: {}", truncate(o.stderr.trim(), 160)))); }
                        Err(e) => { let _ = tx.send(Msg::Notice(format!("transcription failed: {e:#}"))); }
                    }
                });
            }
            "/spec" => {
                let a = arg.trim().to_string();
                if a.is_empty() {
                    let dir = self.workdir.join(".harness/specs");
                    let existing: Vec<String> = std::fs::read_dir(&dir).into_iter().flatten().flatten().filter(|e| e.path().is_dir()).map(|e| e.file_name().to_string_lossy().to_string()).collect();
                    let mut lines = vec!["/spec <feature>            write requirements, design and tasks for it (.harness/specs/<slug>/)".to_string(),
                                         "/spec implement <slug>     work through that spec's tasks, ticking them off".to_string()];
                    if !existing.is_empty() { lines.push(format!("existing specs: {}", existing.join(", "))); }
                    self.blocks.push(Block::Banner(lines));
                    return;
                }
                let prompt = if let Some(slug) = a.strip_prefix("implement ") {
                    let slug = slug.trim();
                    format!(
"Implement the spec in .harness/specs/{slug}/.\n\n1. Read requirements.md, design.md and tasks.md there.\n2. Load tasks.md into the todo tool (one item per unchecked task, in order).\n3. Work through them one at a time: implement, run the tests, tick the checkbox in tasks.md as each is done.\n4. If a task turns out to be wrong or missing, fix the spec file too and say so.\n5. Finish with what was built, what was verified, and anything left.")
                } else {
                    let slug: String = a.to_lowercase().chars().map(|c| if c.is_alphanumeric() { c } else { '-' }).collect::<String>().split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-").chars().take(40).collect();
                    format!(
"Write a spec for: {a}\n\nExplore the codebase first — a spec that ignores what exists is fiction. Then write three files under .harness/specs/{slug}/:\n\nrequirements.md — numbered requirements in EARS form (\"When <trigger>, the system shall <response>\"; \"While <state>…\"; \"If <condition>, then…\"), each testable, plus explicit non-goals.\ndesign.md — how it fits this codebase: the files and functions you would touch, data flow, error handling, migration/compatibility concerns, and the alternatives you rejected with reasons.\ntasks.md — an ordered checklist of implementable steps (`- [ ] …`), each small enough to verify on its own, each naming the requirement it satisfies and how it will be tested.\n\nAsk me about anything genuinely ambiguous with ask_user rather than guessing. Do not implement anything yet; finish by listing the three files and the open questions.")
                };
                if self.running.is_some() { self.queued.push(prompt); self.set_status("queued /spec"); } else { self.start_run(prompt); }
            }
            "/learn" => {
                if arg.trim().is_empty() { self.blocks.push(Block::System("/learn <url|path> [as <name>] — read it and write a skill you can load later".into())); return; }
                let (src, name) = match arg.split_once(" as ") { Some((s, n)) => (s.trim().to_string(), Some(n.trim().to_string())), None => (arg.trim().to_string(), None) };
                let dir = harness::memory::MemoryStore::open(&self.cfg.memory).map(|s| s.dir.join("skills")).unwrap_or_else(|_| harness::setup::config_dir().join("skills"));
                let prompt = format!(
"Learn from {src} and write a skill.\n\n1. Read it: {} for a URL, otherwise read_file / list_dir (and grep) for a path.\n2. Work out what a future agent would actually need: when this applies, the exact steps, commands, file paths, gotchas — not a summary of the prose.\n3. Write it to {}/SKILL.md with frontmatter (name, description: one line saying when to use it), then confirm what you wrote in one sentence.",
                    if src.starts_with("http") { "web_fetch" } else { "read_file" },
                    dir.join(name.unwrap_or_else(|| "<kebab-case-name>".into())).display());
                if self.running.is_some() { self.queued.push(prompt); self.set_status("queued /learn"); } else { self.start_run(prompt); }
            }
            "/prompt" | "/mcp-prompt" => {
                let (name, rest) = arg.split_once(' ').map(|(a, b)| (a.trim().to_string(), b.trim().to_string())).unwrap_or((arg.trim().to_string(), String::new()));
                let tx = self.tx.clone();
                let busy = self.running.is_some();
                tokio::spawn(async move {
                    let servers = harness::mcp::connected_servers();
                    if servers.is_empty() { let _ = tx.send(Msg::Notice("no MCP servers connected (/mcp)".into())); return; }
                    // no name: list what the servers offer
                    if name.is_empty() {
                        let mut lines = vec!["MCP prompts — run one with /prompt <server>:<name> [arguments]".to_string()];
                        for (sname, s) in servers {
                            let mut g = s.lock().await;
                            if !g.has_prompts() { continue; }
                            for p in g.list_prompts().await.unwrap_or_default() {
                                lines.push(format!("  /prompt {sname}:{:<24} {}", p["name"].as_str().unwrap_or(""), truncate(p["description"].as_str().unwrap_or(""), 70)));
                            }
                        }
                        if lines.len() == 1 { lines.push("  (no server advertises prompts)".into()); }
                        let _ = tx.send(Msg::Block(Block::Banner(lines)));
                        return;
                    }
                    let (want_server, want_prompt) = name.split_once(':').map(|(a, b)| (a.to_string(), b.to_string())).unwrap_or((String::new(), name.clone()));
                    for (sname, s) in servers {
                        if !want_server.is_empty() && sname != want_server { continue; }
                        let mut g = s.lock().await;
                        if !g.has_prompts() { continue; }
                        let found = g.list_prompts().await.unwrap_or_default().into_iter().find(|p| p["name"].as_str() == Some(want_prompt.as_str()));
                        let Some(spec) = found else { continue };
                        // positional words fill the prompt's declared arguments in order
                        let mut args = serde_json::Map::new();
                        let words: Vec<&str> = rest.split_whitespace().collect();
                        for (i, a) in spec["arguments"].as_array().cloned().unwrap_or_default().iter().enumerate() {
                            if let Some(k) = a["name"].as_str() {
                                let v = if i + 1 == spec["arguments"].as_array().map(|x| x.len()).unwrap_or(0) { words[i.min(words.len())..].join(" ") } else { words.get(i).copied().unwrap_or("").to_string() };
                                if !v.is_empty() { args.insert(k.to_string(), serde_json::Value::String(v)); }
                            }
                        }
                        match g.get_prompt(&want_prompt, serde_json::Value::Object(args)).await {
                            Ok(text) => { let _ = tx.send(if busy { Msg::QueueTask(text) } else { Msg::RunTask(text) }); }
                            Err(e) => { let _ = tx.send(Msg::Notice(format!("prompt {want_prompt}: {e:#}"))); }
                        }
                        return;
                    }
                    let _ = tx.send(Msg::Notice(format!("no MCP prompt '{name}' (/prompt lists them)")));
                });
            }
            "/commands" => {
                let cmds = harness::commands::discover(&self.workdir);
                if cmds.is_empty() {
                    self.blocks.push(Block::System("no markdown commands — add .harness/commands/<name>.md (body = prompt template; $ARGUMENTS, $1…$9, !`shell`, @file)".into()));
                    return;
                }
                let items = cmds.into_iter().map(|c| PickItem {
                    desc: format!("{}  [{}]", c.description, c.source),
                    detail: format!("{}\n\n{}", if c.path.as_os_str().is_empty() { "(plugin command)".to_string() } else { c.path.display().to_string() }, c.template.trim()),
                    run: Some(format!("/{}", c.name)),
                    label: format!("/{}", c.name),
                }).collect();
                self.pick = Some(ListPicker::new("Markdown commands", "enter runs one · type to filter · esc closes", items));
            }
            "/btw" | "/side" => {
                if arg.trim().is_empty() { self.blocks.push(Block::System("/btw <question> — answers from the session's context with the aux model, without adding anything to the conversation".into())); return; }
                let (q, session, cfg, tx) = (arg.clone(), self.session.clone(), self.cfg.clone(), self.tx.clone());
                self.blocks.push(Block::User(format!("[btw] {q}"), vec![]));
                self.set_status("asking on the side…");
                tokio::spawn(async move {
                    let msgs = session.lock().await.clone();
                    let Ok(client) = Client::new(&cfg.llm) else { return };
                    let tail = harness::agent::render_tail(&msgs, 8000);
                    let req = vec![
                        Message::system("You answer a side question about an ongoing coding session. Use the transcript excerpt as context, answer in a few sentences, and do not propose actions — this is a question asked on the side, not a task."),
                        Message::user(format!("Session so far:\n{tail}\n\nQuestion: {q}")),
                    ];
                    let text = match client.role("aux").chat(&req, &[]).await { Ok((r, _)) => r.text(), Err(e) => format!("(aux model unavailable: {e:#})") };
                    let _ = tx.send(Msg::Block(Block::Banner(std::iter::once("[btw — not part of the conversation]".to_string()).chain(text.lines().map(String::from)).collect())));
                });
            }
            "/recap" => {
                let (session, cfg, tx) = (self.session.clone(), self.cfg.clone(), self.tx.clone());
                self.set_status("summarising the session…");
                tokio::spawn(async move {
                    let msgs = session.lock().await.clone();
                    let Ok(client) = Client::new(&cfg.llm) else { return };
                    let tail = harness::agent::render_tail(&msgs, 12000);
                    let req = vec![
                        Message::system("Recap an ongoing coding session for the user in at most 12 short bullets: what was asked, what was actually done (files, commands, results), what is still open. No preamble."),
                        Message::user(tail),
                    ];
                    let text = match client.role("aux").chat(&req, &[]).await { Ok((r, _)) => r.text(), Err(e) => format!("(aux model unavailable: {e:#})") };
                    let _ = tx.send(Msg::Block(Block::Banner(std::iter::once("Recap".to_string()).chain(text.lines().map(String::from)).collect())));
                });
            }
            "/find" | "/search" => {
                if arg.trim().is_empty() { self.blocks.push(Block::System("/find <text> — search this transcript".into())); return; }
                let needle = arg.to_lowercase();
                let mut hits: Vec<String> = Vec::new();
                for (i, b) in self.blocks.iter().enumerate() {
                    let (kind, text) = match b {
                        Block::User(t, _) => ("user", t.clone()),
                        Block::Assistant { text, .. } => ("assistant", text.clone()),
                        Block::Reasoning { text, .. } => ("thinking", text.clone()),
                        Block::System(t) => ("system", t.clone()),
                        Block::Error(t) => ("error", t.clone()),
                        Block::Banner(l) => ("banner", l.join(" ")),
                        Block::Startup(lines) => ("banner", lines.join(" ")),
                        _ => continue,
                    };
                    for line in text.lines() {
                        if line.to_lowercase().contains(&needle) { hits.push(format!("  {:>3} {:<9} {}", i, kind, truncate(line.trim(), 110))); }
                        if hits.len() >= 60 { break; }
                    }
                    if hits.len() >= 60 { break; }
                }
                if hits.is_empty() { self.blocks.push(Block::System(format!("no match for '{arg}' in this session"))); }
                else { self.blocks.push(Block::Banner(std::iter::once(format!("{} match(es) for '{arg}' — block numbers on the left", hits.len())).chain(hits).collect())); }
            }
            "/jobs" => {
                let store = harness::scheduler::Store::open();
                match store {
                    Ok(st) => {
                        let a = arg.trim().to_string();
                        if let Some(name) = a.strip_prefix("run ") {
                            let (name, cfg, tx) = (name.trim().to_string(), self.cfg.clone(), self.tx.clone());
                            self.blocks.push(Block::System(format!("running scheduled job '{name}' in the background")));
                            tokio::spawn(async move {
                                let Ok(store) = harness::scheduler::Store::open() else { return };
                                let msg = match store.get(&name) {
                                    None => format!("no scheduled job '{name}'"),
                                    Some(j) => match harness::scheduler::run_job(&cfg, &store, &j).await { Ok(t) => format!("job '{name}': {}", truncate(t.trim(), 300)), Err(e) => format!("job '{name}' failed: {e:#}") },
                                };
                                let _ = tx.send(Msg::Notice(msg));
                            });
                        } else if let Some(name) = a.strip_prefix("remove ") {
                            match st.remove(name.trim()) { Ok(j) => self.blocks.push(Block::System(format!("removed job '{}'", j.id))), Err(e) => self.blocks.push(Block::Error(format!("{e:#}"))) }
                        } else {
                            let jobs = st.list();
                            if jobs.is_empty() {
                                self.blocks.push(Block::Banner(vec!["no scheduled jobs".into(), "add one with: harness schedule add <name> --every 1h|--at 03:00 \"<prompt>\" · run them with: harness daemon".into()]));
                                return;
                            }
                            let now = harness::scheduler::now();
                            let items = jobs.into_iter().map(|j| PickItem {
                                desc: format!("{:<12} next {:<8} {} run(s)  {}", j.cadence(), if !j.enabled { "paused".into() } else if j.next_at <= now { "due".into() } else { harness::scheduler::fmt_secs(j.next_at - now) }, j.runs, truncate(&j.prompt, 60)),
                                detail: format!("{}\n\nworkdir: {}\ncadence: {} · runs: {}{}\n\nenter runs it now · /jobs remove {} deletes it", j.prompt, j.workdir, j.cadence(), j.runs, if j.last_status.is_empty() { String::new() } else { format!("\nlast: {}", j.last_status) }, j.id),
                                run: Some(format!("/jobs run {}", j.id)),
                                label: j.id,
                            }).collect();
                            self.pick = Some(ListPicker::new("Scheduled jobs", "enter runs one now · harness daemon runs them on schedule · esc closes", items));
                        }
                    }
                    Err(e) => self.blocks.push(Block::Error(format!("{e:#}"))),
                }
            }
            "/goal" => {
                let a = arg.trim().to_string();
                if a.is_empty() {
                    match &self.goal {
                        Some(g) => self.blocks.push(Block::Banner(vec![format!("goal (round {}/12): {g}", self.goal_rounds), "the agent keeps working until a checker model says it is met · /goal off to stop".into()])),
                        None => self.blocks.push(Block::System("no goal set — /goal <condition> works until that condition holds (checked by the aux model after every turn)".into())),
                    }
                } else if a == "off" || a == "clear" || a == "stop" {
                    self.goal = None; self.goal_rounds = 0;
                    self.blocks.push(Block::System("goal cleared".into()));
                } else {
                    self.goal = Some(a.clone()); self.goal_rounds = 0;
                    self.blocks.push(Block::System(format!("goal set: {a} — working until it is met (max 12 rounds; /goal off stops)")));
                    if self.running.is_none() { self.start_run(format!("Work until this is true: {a}\n\nStart now and keep going until it holds; verify it yourself before claiming success.")); }
                }
            }
            "/localmodel" | "/qwen" => {
                use harness::localmodel::{self as lm, ModelState};
                let a = arg.trim();
                match a {
                    "" => {
                        if let Some(p) = &self.dl { let line = p.line(); let pct = p.percent(); self.blocks.push(Block::System(format!("downloading {} — {pct:.1}% · {line} · /localmodel cancel stops it (resumable)", self.dl_build.map(|b| b.name()).unwrap_or("model")))); }
                        else { self.pick = Some(self.model_picker()); }
                    }
                    "4" | "6" | "8" => match lm::by_bits(a.parse().unwrap_or(0)) {
                        Some(b) => self.start_model_download(b),
                        None => self.blocks.push(Block::Error("usage: /localmodel 4|6|8".into())),
                    },
                    // Opt-in build by id (abliterated-mxfp4 | abliterated-6bit | heretic-gguf | …)
                    _ if lm::by_name(a).map(|b| b.is_extra()).unwrap_or(false) => {
                        let b = lm::by_name(a).unwrap();
                        self.blocks.push(Block::System(format!("⚠ {} is uncensored/abliterated — safety refusals removed. Your responsibility what you do with it.", b.label)));
                        if b.is_gguf() && lm::llama_server_bin().is_none() {
                            self.blocks.push(Block::System("this is a GGUF — the harness will install llama.cpp (llama-server) with Homebrew when it starts serving".into()));
                        }
                        self.start_model_download(b);
                    }
                    "resume" => match lm::by_name(&self.cfg.local_model.build) {
                        Some(b) => self.start_model_download(b),
                        None => self.blocks.push(Block::Error("nothing to resume — /localmodel picks a build".into())),
                    },
                    "serve" | "start" | "restart" => self.start_mlx_ex(a == "restart"),
                    "vision" | "text" | "auto" => {
                        let kind = match a { "vision" => "mlx-vlm", "text" => "mlx-lm", _ => "auto" };
                        self.cfg.local_model.server = kind.into();
                        let _ = harness::config::Config::save_setting("local_model.server", kind);
                        self.blocks.push(Block::System(format!("local_model.server → {kind} · restarting the MLX server ({}{})", lm::server_plan(kind).first().copied().unwrap_or("?"), if a == "vision" || a == "auto" { " — the same weights with the vision tower, so images and video frames work" } else { " — text-only" })));
                        self.start_mlx_ex(true);
                    }
                    _ if a == "draft" || a.starts_with("draft ") => {
                        let val = a.strip_prefix("draft").unwrap_or("").trim().to_string();
                        if val.is_empty() {
                            let d = &self.cfg.local_model.draft_model;
                            self.blocks.push(Block::System(if d.is_empty() {
                                "speculative decoding: off. /localmodel draft z-lab/Qwen3.5-27B-DFlash turns it on (a DFlash drafter matched to the 27B; ~1.4–1.6× on code) — or any drafter for the same family; /localmodel draft off disables it. mlx_vlm only.".into()
                            } else {
                                format!("speculative decoding: on · drafter {d} · kind {} · /localmodel draft off to disable", self.cfg.local_model.draft_kind)
                            }));
                        } else if val == "off" || val == "none" {
                            self.cfg.local_model.draft_model.clear();
                            let _ = harness::config::Config::save_setting("local_model.draft_model", "");
                            self.blocks.push(Block::System("speculative decoding off · restarting the MLX server".into()));
                            self.start_mlx_ex(true);
                        } else {
                            self.cfg.local_model.draft_model = val.clone();
                            let _ = harness::config::Config::save_setting("local_model.draft_model", &val);
                            self.blocks.push(Block::System(format!("speculative decoding on · drafter {val} · restarting the MLX server (it downloads the drafter on first serve if it is an HF id — watch ~/.config/harness/logs/mlx-server.log)")));
                            self.start_mlx_ex(true);
                        }
                    }
                    _ if a == "kv" || a.starts_with("kv ") => {
                        let val = a.strip_prefix("kv").unwrap_or("").trim().to_string();
                        if val.is_empty() {
                            let b = self.cfg.local_model.kv_bits;
                            self.blocks.push(Block::System(if b == 8 || b == 4 {
                                format!("KV-cache quantization: {b}-bit · /localmodel kv off to disable")
                            } else {
                                "KV-cache quantization: off (fp16). /localmodel kv 8 (or 4) shrinks the KV the context holds — less memory pressure, faster long-context decode, small quality cost. mlx_vlm only.".into()
                            }));
                        } else {
                            let bits: u8 = match val.as_str() { "off"|"none"|"0"|"16"|"fp16" => 0, "8" => 8, "4" => 4, _ => 255 };
                            if bits == 255 { self.blocks.push(Block::Error("usage: /localmodel kv 8 | 4 | off".into())); }
                            else {
                                self.cfg.local_model.kv_bits = bits;
                                let _ = harness::config::Config::save_setting("local_model.kv_bits", &bits.to_string());
                                self.blocks.push(Block::System(format!("KV-cache quantization → {} · restarting the MLX server", if bits == 0 { "off (fp16)".into() } else { format!("{bits}-bit") })));
                                self.start_mlx_ex(true);
                            }
                        }
                    }
                    "cancel" | "stop" | "unload" | "kill" => {
                        // `stop`/`cancel` cancel a running download first; otherwise (and always for
                        // `unload`/`kill`) they shut the MLX server down and reclaim its ~16–30GB of RAM.
                        if a != "unload" && a != "kill" {
                            if let Some(h) = self.dl_cancel.take() { h.abort(); self.dl = None; self.blocks.push(Block::System("download stopped — the bytes already on disk stay, /localmodel resume continues from there".into())); return; }
                        }
                        let (tx, port) = (self.tx.clone(), self.cfg.local_model.port);
                        self.set_status(format!("stopping the MLX server on port {port}"));
                        tokio::spawn(async move {
                            let stopped = harness::localmodel::stop_on_port(port).await;
                            let _ = tx.send(Msg::Notice(if stopped {
                                format!("MLX server stopped — memory reclaimed. It will not autostart again this session; /localmodel serve starts it, or set [local_model] autostart = false to keep it off (port {port})")
                            } else {
                                "no MLX server of ours was running on that port".into()
                            }));
                        });
                        // don't let this session immediately re-launch it
                        self.cfg.local_model.autostart = false;
                    }
                    "status" => {
                        let mut lines = vec![format!("runtime: {} · vision server (mlx-vlm): {}", lm::mlx_python().map(|p| p.display().to_string()).unwrap_or("missing — re-run the installer".into()), if lm::has_mlx_vlm() { "installed" } else { "missing — installed automatically on the next /localmodel serve" })];
                        for b in lm::BUILDS {
                            let s = match lm::state_of(b) {
                                ModelState::Ready { bytes } => format!("complete ({})", harness::tools::download::human(bytes)),
                                ModelState::Partial { bytes } => format!("partial ({} of {}) — /localmodel resume", harness::tools::download::human(bytes), harness::tools::download::human(b.bytes)),
                                ModelState::Missing => "not downloaded".into(),
                            };
                            lines.push(format!("  {}-bit  {s}{}", b.bits, if self.cfg.local_model.build == b.name() { "  ← selected" } else { "" }));
                        }
                        lines.push(format!("server: {} → {} on 127.0.0.1:{} · /localmodel vision|text|auto switches (mlx_vlm sees images, mlx_lm is text-only)", self.cfg.local_model.server, lm::server_plan(&self.cfg.local_model.server).join(" then "), self.cfg.local_model.port));
                        lines.push(format!("speculative decoding: {} · /localmodel draft <hf-id|off>", if self.cfg.local_model.draft_model.is_empty() { "off".to_string() } else { format!("on · drafter {} (kind {})", self.cfg.local_model.draft_model, self.cfg.local_model.draft_kind) }));
                        lines.push(format!("KV-cache quant: {} · /localmodel kv 8|4|off", if self.cfg.local_model.kv_bits == 8 || self.cfg.local_model.kv_bits == 4 { format!("{}-bit", self.cfg.local_model.kv_bits) } else { "off (fp16)".to_string() }));
                        self.blocks.push(Block::Banner(lines));
                    }
                    _ => self.blocks.push(Block::Error("usage: /localmodel [4|6|8|resume|serve|stop|restart|status|vision|text|draft <id|off>|kv <8|4|off>]".into())),
                }
            }
            "/delegate" => {
                // Claude orchestrates, the local model does the delegated work: [llm.roles] subagent.
                let on = matches!(arg.trim(), "" | "on" | "local" | "qwen");
                if arg.trim() == "off" || arg.trim() == "none" {
                    self.cfg.llm.roles.remove("subagent");
                    self.blocks.push(Block::System("delegation off — sub-agents run on the main model again".into()));
                } else if on {
                    let Some(build) = harness::localmodel::by_name(&self.cfg.local_model.build) else {
                        self.blocks.push(Block::Error("nothing to delegate to yet — /localmodel downloads a local model first".into()));
                        return;
                    };
                    if !matches!(harness::localmodel::state_of(build), harness::localmodel::ModelState::Ready { .. }) {
                        self.blocks.push(Block::Error(format!("{} is not fully downloaded yet — /localmodel status", build.name())));
                        return;
                    }
                    let model = build.dir().display().to_string();
                    let base = format!("http://127.0.0.1:{}/v1", self.cfg.local_model.port);
                    self.cfg.llm.roles.insert("subagent".into(), harness::config::RoleConfig::Full { model: Some(model.clone()), base_url: Some(base.clone()), api_key: None, temperature: None });
                    self.blocks.push(Block::Banner(vec![
                        "delegation on — every sub-agent runs on the local model".into(),
                        format!("  orchestrator: {} · delegate: {} on {}", self.model, build.name(), base),
                        "  spawn_agent / the team tool now cost nothing but electricity; /delegate off reverts".into(),
                        "  permanent: put [llm.roles] subagent = { model = \"…\", base_url = \"…\" } in harness.toml".into(),
                    ]));
                } else { self.blocks.push(Block::Error("usage: /delegate [on|off]".into())); }
            }
            "/claude-login" => {
                let install = arg.trim() == "install";
                self.run_external = Some(if install {
                    "curl -fsSL https://claude.ai/install.sh | bash && claude auth login".into()
                } else { harness::claude_code::LOGIN_COMMAND.to_string() });
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
                self.no_model = false;   // the user is choosing a backend explicitly; stop claiming nothing is loaded
                let mut it = arg.split_whitespace(); let which = it.next().unwrap_or("").to_string(); let model = it.next().map(String::from); let effort = it.next().map(|e| e.to_lowercase());
                match which.as_str() {
                    "" => { self.blocks.push(Block::Banner(vec![format!("backend: {} · model {}", self.cfg.llm.provider.clone().unwrap_or("openai (local/compatible server)".into()), self.model), "switch: /backend local [model]  ·  /backend claude [model] [effort]   (claude = official Claude Code CLI on your subscription, default claude-fable-5; effort low|medium|high|max, also /effort)".into(), "        /backend anthropic <model>  (Anthropic API key from ANTHROPIC_API_KEY)".into(),
                        "        /backend acp <gemini|codex|opencode|copilot|goose|\"<command>\">  (drive another ACP agent)".into()])); }
                    "local" | "lmstudio" => { self.cfg.llm.provider = None; if let Some(m) = model { self.cfg.llm.model = m; } self.model = self.cfg.llm.model.clone(); if let Some(cc) = self.cc.take() { tokio::spawn(async move { cc.stop().await; }); } self.cc_last_session = None; self.blocks.push(Block::System(format!("backend → local server {} · model {}", self.cfg.llm.base_url, self.model))); tokio::spawn(fetch_ctx_len(self.cfg.llm.base_url.clone(), self.model.clone(), self.tx.clone())); }
                    "claude" | "claude-code" | "cc" => { self.cfg.llm.provider = Some("claude-code".into()); self.cfg.llm.model = model.unwrap_or("claude-fable-5".into()); if let Some(e) = effort { if matches!(e.as_str(), "low" | "medium" | "high" | "xhigh" | "max") { self.cfg.llm.effort = Some(e); } } if self.cfg.llm.effort.is_none() { self.cfg.llm.effort = Some("medium".into()); } self.model = self.cfg.llm.model.clone(); if let Some(cc) = self.cc.take() { tokio::spawn(async move { cc.stop().await; }); } self.cc_last_session = None; self.metrics.ctx_len = 0; self.blocks.push(Block::System(format!("backend → Claude Code (subscription) · model {} · tools bridged over MCP · context window reported after the first turn", self.model))); }
                    "acp" => {
                        let spec = arg.split_whitespace().skip(1).collect::<Vec<_>>().join(" ");
                        if spec.is_empty() { self.blocks.push(Block::Banner(vec!["/backend acp <agent>  — drive another Agent Client Protocol agent from this UI".into(), "  known shortcuts: gemini · codex · opencode · copilot · goose · harness".into(), "  or a full command: /backend acp my-agent --acp".into()])); return; }
                        self.cfg.llm.provider = Some(format!("acp:{spec}"));
                        self.model = harness::acp_client::expand_command(&spec);
                        if let Some(a) = self.acp.take() { tokio::spawn(async move { a.stop().await; }); }
                        if let Some(cc) = self.cc.take() { tokio::spawn(async move { cc.stop().await; }); }
                        self.metrics.ctx_len = 0;
                        self.blocks.push(Block::System(format!("backend → ACP agent `{}` — it brings its own tools; permissions and file writes still come through this UI", self.model)));
                    }
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
                if !arg.is_empty() && arg != "clear" { self.queued.push(arg.clone()); self.set_status(format!("queued ({} waiting)", self.queued.len())); return; }
                if self.queued.is_empty() { self.blocks.push(Block::System("queue is empty".into())); }
                else { let mut lines = vec![format!("Queued tasks ({}) — /next skips the current one, /queue clear empties the queue", self.queued.len())]; for (i, q) in self.queued.iter().enumerate() { lines.push(format!("  {}. {}", i + 1, truncate(q, 120))); } self.blocks.push(Block::Banner(lines)); }
                if arg == "clear" { self.queued.clear(); self.blocks.push(Block::System("queue cleared".into())); }
            }
            "/next" | "/skip" => self.next_task(),
            "/factory-reset" | "/factory_reset" | "/reset" => self.factory_reset(arg.trim() == "confirm"),
            "/uninstall" => self.uninstall(arg.trim() == "confirm"),
            "/exit" | "/quit" | "/q" => self.quit = true,
            "/update" => {
                // Only a check: the binary is never replaced under a running session — starting harness is what updates.
                self.set_status(format!("asking github.com/{} for the latest release", harness::update::REPO));
                let tx = self.tx.clone();
                tokio::spawn(async move { let r = harness::update::latest().await.map_err(|e| format!("{e:#}")); let _ = tx.send(Msg::Update(r)); });
            }
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
                let name = cmd.trim_start_matches('/').to_string();
                match harness::commands::find(&self.workdir, &name) {
                    Some(c) => {
                        self.blocks.push(Block::System(format!("/{} ({})", c.name, c.source)));
                        let (wd, tx, arg2) = (self.workdir.clone(), self.tx.clone(), arg.clone());
                        let busy = self.running.is_some();
                        tokio::spawn(async move {
                            let prompt = harness::commands::expand(&c, &arg2, &wd).await;
                            let prompt = match &c.agent { Some(a) => format!("Use spawn_agent with subagent_type \"{a}\" for this:\n\n{prompt}"), None => prompt };
                            let _ = tx.send(if busy { Msg::QueueTask(prompt) } else { Msg::RunTask(prompt) });
                        });
                    }
                    None => self.blocks.push(Block::Error(format!("unknown command {cmd} — /help · /commands lists the markdown ones"))),
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
        let acp_existing = self.acp.clone();
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
                let mut env_ = harness::agent::SubAgentEnv::new(client.clone(), registry.clone(), policy.clone(), approver.clone(), sink.clone(), budget, true); env_.cc_effort = cfg.llm.effort.clone(); env_.max_depth = cfg.agent.max_subagent_depth.max(1); let env = Arc::new(env_); let _ = tx.send(Msg::SubEnv(env.clone()));
                let ctx = ToolCtx { workdir: workdir.clone(), timeout: Duration::from_secs(cfg.agent.tool_timeout_secs), max_output: cfg.agent.max_tool_output_chars, net: cfg.net.clone(), memory: store.clone(), subagent: Some(env), redact_secrets: cfg.security.redact_secrets, injection_scan: cfg.security.injection_scan, hooks: cfg.hooks.clone(), todos: todos.clone(), lsp_servers: cfg.lsp.servers.clone(), format: cfg.format.clone(), extra_roots: extra_roots.clone(), approver: Some(approver.clone()), inbox: inbox.clone(), cancel: None, cwd: Some(cwd.clone()), session_id: Some(session_id.clone()) };
                let agent = Agent { client: &client, registry, ctx: &ctx, max_turns: cfg.agent.max_turns, context_budget: budget, sink: sink.as_ref(), stream: true, policy: &policy, tool_history_keep: cfg.agent.tool_history_keep, tool_history_chars: cfg.agent.tool_history_max_chars, approver: approver.as_ref() };
                let extra = format!("You are in an interactive session: the user can see everything and will reply; keep final answers concise.{extra_prompt}");
                let system = harness::agent::system_prompt_with_memory(&workdir.display().to_string(), &registry.names(), Some(&extra), store.as_ref());
                // provider = "acp:<cmd>": another agent runs the turn; we stream its updates
                if let Some(cmd) = client.acp_command() {
                    let mut msgs = session.lock().await;
                    if msgs.is_empty() { msgs.push(Message::system(&system)); }
                    msgs.push(user_msg);
                    let session_acp = match acp_existing {
                        Some(a) => a,
                        None => {
                            let a = harness::acp_client::AcpSession::start(&cmd, &workdir, policy.clone(), approver.clone()).await.map_err(|e| format!("{e:#}"))?;
                            let _ = tx.send(Msg::AcpSession(a.clone())); a
                        }
                    };
                    let (t, st) = session_acp.run_turn(&user_text, sink.clone()).await.map_err(|e| format!("{e:#}"))?;
                    sink.emit(&Event::Assistant { text: t.clone() });
                    msgs.push(Message { role: "assistant".into(), content: Some(Content::Text(t.clone())), ..Default::default() });
                    return Ok((t, st));
                }
                let mut msgs = session.lock().await;
                // turn boundary checkpoint: anchors /rewind (code + conversation) to "before this prompt"
                {
                    let (n, sid, wd) = (msgs.len(), session_id.clone(), workdir.clone());
                    let label = format!("before turn: {}", harness::llm::truncate_for_log(&user_text, 60));
                    let _ = tokio::task::spawn_blocking(move || { if let Some(cp) = harness::checkpoints::for_session(&sid, &wd) { let _ = cp.snapshot(&label, n); } }).await;
                }
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
            if let Ok((r, _)) = client.role("title").chat(&req, &[]).await { let t = r.text().lines().next().unwrap_or("").trim().trim_matches('"').to_string(); if (3..=80).contains(&t.len()) { let _ = tx.send(Msg::Title(t)); } }
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
            Msg::Update(r) => {
                let cur = harness::update::Version::current();
                match r {
                    Ok(rel) if rel.is_newer() => {
                        let mut lines = vec![format!("⬆ TheHarness {} is available (you have {cur})", rel.version)];
                        let h = harness::update::headline(&rel.notes); if !h.is_empty() { lines.push(format!("  {h}")); }
                        lines.push(if self.cfg.update.mode == "auto" { "  quit and start harness again — it updates itself on start (or run `harness update` in another terminal; this session keeps running on the current version)".into() } else { "  run `harness update` (this session keeps running on the current version)".into() });
                        self.blocks.push(Block::Banner(lines));
                    }
                    Ok(rel) => self.blocks.push(Block::System(format!("⬆ up to date: {cur} is the latest release{}", if rel.version < cur { format!(" (the release is {}; this build is ahead of it)", rel.version) } else { String::new() }))),
                    Err(e) => self.blocks.push(Block::Error(format!("/update: {e}"))),
                }
            }
            Msg::CcSession(s) => { self.cc = Some(s); }
            Msg::Policy(p) => { p.set_mode(self.perm_mode); self.live_policy = Some(p); }
            Msg::SubEnv(e) => { self.subenv = Some(e); }
            Msg::Title(t) => { self.session_meta.title = t; self.save_session(); }
            Msg::Toast(t) => { self.toast = Some((t, Instant::now())); }
            Msg::Dictated(t) => { if !self.input.is_empty() && !self.input.ends_with(' ') { self.insert_str(" "); } self.insert_str(&t); self.set_status("dictated — edit and press enter"); }
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
            Msg::Frames(Ok(ex)) => {
                let fr: Vec<(f64, PathBuf, Option<String>)> = ex.frames.into_iter().map(|(ts, p)| (ts, p, None)).collect();
                if let Some(v) = &mut self.video { if v.path == ex.path { v.frames = fr; v.duration = ex.duration; v.loading = false; v.cur = 0; v.note = ex.note; v.source = ex.source; v.preview = None; } }
            }
            Msg::Frames(Err(e)) => { if let Some(v) = &mut self.video { v.loading = false; v.error = Some(e); } }
            Msg::Pasted(Err(e)) => self.set_status(e),
            Msg::Review(diff) => self.open_review(diff),
            Msg::RunTask(t) => { if self.running.is_some() { self.queued.push(t); } else { self.start_run(t); } }
            Msg::QueueTask(t) => { self.queued.push(t); self.set_status(format!("queued ({} waiting)", self.queued.len())); }
            Msg::AcpSession(s) => self.acp = Some(s),
            Msg::StatusLine(t) => self.statusline = (!t.trim().is_empty()).then(|| t.trim().to_string()),
            Msg::LocalProbe(up) => self.on_local_probe(up),
            Msg::ClaudeAuth(a) => self.on_claude_auth(a),
            Msg::ModelDl(p) => self.dl = Some(p),
            Msg::ModelDlDone(r) => {
                self.dl_cancel = None;
                match r {
                    Ok(dir) => {
                        self.dl = None;
                        let gguf = self.dl_build.map(|b| b.is_gguf()).unwrap_or(false);
                        self.blocks.push(Block::System(format!("model downloaded → {dir} · starting {}", if gguf { "llama-server (installing llama.cpp if needed)" } else { "the MLX server" })));
                        self.start_mlx();
                    }
                    Err(e) => {
                        // The bytes on disk are kept: /localmodel resumes from exactly here.
                        self.blocks.push(Block::Error(format!("model download stopped: {e}\nnothing is lost — /localmodel resumes it")));
                        self.dl = None;
                    }
                }
            }
            Msg::MlxTextOnly(base_url) => {
                self.blocks.push(Block::System(format!("{base_url} is served by mlx_lm.server, which is text-only (images get \"Only 'text' content type is supported\") — restarting it as mlx_vlm.server, same weights plus the vision tower; back in a moment")));
                self.start_mlx_ex(true);
            }
            Msg::MlxUp(r) => match r {
                Ok((base_url, model, module)) => {
                    let vision = module == "mlx_vlm.server";
                    self.no_model = false;
                    let claude = self.cfg.llm.provider.as_deref() == Some("claude-code");
                    self.cfg.llm.base_url = base_url.clone();
                    let _ = harness::config::Config::save_setting("llm.base_url", &base_url);
                    if claude {
                        // Working on Claude already: this is the moment to ask, not to switch under them.
                        self.pick = Some(ListPicker::new(
                            "Qwen3.8-27B is ready",
                            "enter chooses · esc keeps things as they are",
                            vec![
                                PickItem { label: "Switch to the local model".into(), desc: format!("{model} on {base_url}"), detail: "Everything runs locally from now on: no subscription usage, no network. Claude stays one /backend claude away.".into(), run: Some(format!("/backend local {model}")) },
                                PickItem { label: "Keep Claude, delegate to Qwen3.8".into(), desc: "Claude orchestrates · the local model does the work".into(), detail: "Claude plans and drives the session while every sub-agent (spawn_agent, /agents) runs on the local model. Cheap parallel work, Claude's judgement on top.".into(), run: Some("/delegate on".into()) },
                                PickItem { label: "Stay on Claude for now".into(), desc: "the local model is there when you want it".into(), detail: "Nothing changes. /backend local switches whenever you like; the MLX server keeps running.".into(), run: None },
                            ]));
                    } else {
                        self.cfg.llm.model = model.clone(); self.model = model.clone();
                        let note = if module == "llama-server" { " (GGUF via llama.cpp)" } else if vision { " (vision: images and video frames work)" } else { " (text-only — /localmodel vision restarts it with the vision tower)" };
                        self.blocks.push(Block::System(format!("local model ready: {} on {base_url} · {module}{note}", model_label(&model))));
                        tokio::spawn(fetch_ctx_len(base_url, model, self.tx.clone()));
                    }
                }
                Err(e) => self.blocks.push(Block::Error(format!("MLX server: {e}"))),
            },
            Msg::GoalCheck(met, reason) => {
                let Some(goal) = self.goal.clone() else { return };
                if met {
                    self.goal = None; self.goal_rounds = 0;
                    self.blocks.push(Block::System(format!("goal met: {goal}{}", if reason.is_empty() { String::new() } else { format!(" — {reason}") })));
                } else if self.running.is_none() {
                    self.goal_rounds += 1;
                    self.blocks.push(Block::System(format!("goal not met yet ({}): {reason} — continuing (round {}/12, /goal off stops)", goal, self.goal_rounds)));
                    self.start_run(format!("[goal] Not satisfied yet: {reason}\nKeep working until this is true, then verify it: {goal}"));
                }
            }
            Msg::Done(res) => {
                self.running = None;
                if let Some(cc) = self.cc.clone() { let tx = self.tx.clone(); tokio::spawn(async move { if let Some(id) = cc.session_id.lock().await.clone() { let _ = tx.send(Msg::CcSid(id)); } }); }
                if self.cfg.ui.notify && self.run_started.elapsed() > Duration::from_secs(20) {
                    let title = match &res { Ok(_) => "Harness: task finished", Err(_) => "Harness: task stopped" };
                    let body = truncate(&self.blocks.iter().rev().find_map(|b| if let Block::User(t, _) = b { Some(t.clone()) } else { None }).unwrap_or_default(), 80).replace('"', "'");
                    let title = title.to_string();
                    { let h = self.cfg.hooks.clone(); let wd = self.workdir.clone(); let (t2, b2) = (title.clone(), body.clone()); if !h.notification.is_empty() { tokio::spawn(async move { let _ = harness::hooks::run_event(&h, "notification", &t2, serde_json::json!({"title": t2, "body": b2}), &wd).await; }); } }
                    // OSC 9: terminals that support it (kitty, iTerm2, WezTerm, Ghostty) show a native
                    // notification even when the window is in the background; the bell is opt-in.
                    { use std::io::Write; let mut o = std::io::stdout(); let _ = write!(o, "\x1b]9;{title}: {body}\x07"); if self.cfg.ui.sound { let _ = write!(o, "\x07"); } let _ = o.flush(); }
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
                else if let Some(goal) = self.goal.clone() {
                    if self.goal_rounds >= 12 { self.goal = None; self.blocks.push(Block::System("goal: giving up after 12 rounds — /goal <condition> to try again".into())); }
                    else {
                        self.set_status("checking whether the goal is met…");
                        let (session, cfg, tx) = (self.session.clone(), self.cfg.clone(), self.tx.clone());
                        let last = self.blocks.iter().rev().find_map(|b| if let Block::Assistant { text, .. } = b { Some(text.clone()) } else { None }).unwrap_or_default();
                        tokio::spawn(async move {
                            let msgs = session.lock().await.clone();
                            let Ok(client) = Client::new(&cfg.llm) else { return };
                            let (met, reason) = harness::agent::goal_check(&client, &goal, &last, &msgs).await;
                            let _ = tx.send(Msg::GoalCheck(met, reason));
                        });
                    }
                }
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
            Event::Reasoning { text } => finalize_reasoning(&mut self.blocks, text),
            Event::AssistantDelta { text } => {
                self.metrics.on_delta(text.chars().count());
                if let Some(Block::Assistant { text: t, streaming: true, .. }) = self.blocks.last_mut() { t.push_str(&text); }
                else { self.finish_streaming(); self.blocks.push(Block::Assistant { text, streaming: true, folded: false }); }
            }
            Event::Assistant { text } => finalize_assistant(&mut self.blocks, text),
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
            Event::Error { message } => {
                self.finish_streaming();
                let text_only = message.contains("Only 'text' content type is supported");
                self.blocks.push(Block::Error(message));
                if text_only { self.blocks.push(Block::System("that is mlx_lm.server refusing the image parts — it is text-only. /localmodel vision restarts the local model with mlx_vlm.server (same weights + the vision tower); then send the images again".into())); }
            }
            Event::Memory { file, section, text } => { self.blocks.push(Block::Memory(format!("{} › {section}: {text}", file.trim_end_matches(".md")))); }
            Event::Permission { tool, summary, decision } => { if decision.starts_with("denied") { self.blocks.push(Block::Error(format!("🔒 {tool}({}) {decision}", truncate(&summary, 80)))); } }
        }
    }
    // ───────────────────────── first run: getting a model at all ─────────────────────────

    /// What to do about the local endpoint. A server that merely *answers* is not a model that can
    /// reply — LM Studio lists everything it has downloaded — so say what was actually found, then get
    /// the user to a working model: our own weights if they are here, the download picker if not, and the
    /// Claude question either way.
    fn on_local_probe(&mut self, ep: harness::localmodel::Endpoint) {
        use harness::localmodel::{self as lm, Endpoint, ModelState};
        // The start-up card was printed before we knew any of this; it named a model on the strength of the
        // config file alone. Correct it in place rather than leave a card that promises a working model.
        if !ep.ready() {
            for b in self.blocks.iter_mut() {
                if let Block::Startup(lines) = b {
                    for l in lines.iter_mut() {
                        if l.starts_with("  model  ") && !l.contains("not loaded") { l.push_str("   ← not loaded"); }
                    }
                    break;
                }
            }
        }
        match &ep {
            Endpoint::Ready { model } => {
                // Whatever is loaded there is what answers, whatever the config file claims.
                if !model.is_empty() && model != &self.model {
                    self.blocks.push(Block::System(format!("{} is loaded on {} — using it", model_label(model), self.cfg.llm.base_url)));
                    self.cfg.llm.model = model.clone(); self.model = model.clone();
                }
                // Our own port, our own weights, but the text-only server (left over from an older harness or
                // an earlier session)? Then images would fail — swap it for the vision server now, while idle.
                let ours = self.cfg.llm.base_url.contains(&format!(":{}/", self.cfg.local_model.port)) || self.cfg.llm.base_url.ends_with(&format!(":{}", self.cfg.local_model.port));
                let wants_vision = lm::server_plan(&self.cfg.local_model.server).first() == Some(&"mlx_vlm.server");
                if ours && wants_vision && self.cfg.local_model.autostart && lm::by_name(&self.cfg.local_model.build).map(|b| matches!(lm::state_of(b), ModelState::Ready { .. })).unwrap_or(false) {
                    let (tx, base) = (self.tx.clone(), self.cfg.llm.base_url.clone());
                    tokio::spawn(async move { if lm::server_kind(&base).await == Some("mlx_lm.server") { let _ = tx.send(Msg::MlxTextOnly(base)); } });
                }
                return;
            }
            Endpoint::Idle { listed } if *listed > 0 => {
                self.no_model = true;
                self.blocks.push(Block::Error(format!(
                    "{} is running but has no model loaded — it lists {listed} downloaded model{}, none of them resident, so a turn would fail with \"No models loaded\".",
                    self.cfg.llm.base_url, if *listed == 1 { "" } else { "s" })));
            }
            Endpoint::Idle { .. } => {
                self.no_model = true;
                self.blocks.push(Block::Error(format!("{} is running but serving no models.", self.cfg.llm.base_url)));
            }
            Endpoint::Down => {
                self.no_model = true;
                self.blocks.push(Block::Error(format!("nothing is serving on {} — no model to talk to yet.", self.cfg.llm.base_url)));
            }
        }
        // Our own weights beat any other server: this is the model the harness downloaded and controls.
        if let Some(b) = lm::by_name(&self.cfg.local_model.build) {
            match lm::state_of(b) {
                ModelState::Ready { .. } => {
                    if self.cfg.local_model.autostart {
                        self.blocks.push(Block::System(format!("{} is on disk · starting the MLX server", b.name())));
                        self.start_mlx();
                        return;
                    }
                }
                ModelState::Partial { bytes } => {
                    self.blocks.push(Block::System(format!("{} is {} of the way down — /localmodel resume continues it (nothing re-downloads)",
                        b.name(), harness::tools::download::human(bytes))));
                    self.offer_claude_meanwhile();
                    return;
                }
                ModelState::Missing => {}
            }
        }
        self.first_run_pending = true;      // closing the picker still owes the user the Claude question
        self.pick = Some(self.model_picker());
    }

    /// The three Qwen3.8-27B MLX builds, plus the ways to work without one.
    fn model_picker(&self) -> ListPicker {
        use harness::localmodel as lm;
        let ram = self.metrics.last.mem_total / 1_000_000_000;   // 0 until the first sampler tick
        let mut items: Vec<PickItem> = lm::BUILDS.iter().map(|b| {
            let gb = b.bytes / 1_000_000_000;
            let tight = ram > 0 && b.ram_gb() > ram;
            let runtime = if b.is_gguf() { "llama-server (llama.cpp)" } else { "mlx_vlm.server" };
            PickItem {
                label: b.label.to_string(),
                desc: format!("{gb} GB download · {}{}", b.note, if tight { " · tight on this machine" } else { "" }),
                detail: format!(
                    "{}\n\nDownloads {} from Hugging Face into ~/.config/harness/models, resuming where it stopped if \
                     interrupted. Served by {runtime}. Wants about {} GB of RAM resident{}.{}\n\n\
                     You keep working on Claude meanwhile; when it lands, the harness asks whether to switch.",
                    b.repo, if b.is_gguf() { format!("{gb} GB ({})", b.file) } else { format!("{gb} GB") }, b.ram_gb(),
                    if ram > 0 { format!(" (this Mac has {ram} GB)") } else { String::new() },
                    if b.is_extra() { "\n\n⚠ Uncensored / abliterated — safety refusals removed. Your responsibility what you do with it." } else { "" }),
                run: Some(format!("/localmodel {}", b.id)),
            }
        }).collect();
        items.push(PickItem {
            label: "Not now".into(),
            desc: "no local model · point the harness at any OpenAI-compatible server".into(),
            detail: "Nothing is downloaded. Set [llm] base_url/model in ~/.config/harness/harness.toml to use LM Studio, \
                     llama-server or a hosted API, or run /localmodel later to pick a build.".into(),
            run: None,
        });
        ListPicker::new("No local model yet — pick one to download", "enter downloads · esc decides later", items)
    }

    /// Start the segmented, resumable download of `build` and, if possible, put the user on Claude while
    /// it runs. This is the only place that begins a download.
    fn start_model_download(&mut self, build: &'static harness::localmodel::Build) {
        if self.dl_cancel.is_some() { self.set_status("a model download is already running — /localmodel cancel stops it"); return; }
        self.cfg.local_model.build = build.name().to_string();
        let _ = harness::config::Config::save_setting("local_model.build", build.name());
        self.dl_build = Some(build);
        self.dl = Some(harness::localmodel::Progress { total: build.bytes, ..Default::default() });
        self.blocks.push(Block::System(format!("downloading {} ({} GB) in {} segments per file — progress in the panel (⌃P); it resumes if interrupted",
            build.repo, build.bytes / 1_000_000_000, self.cfg.local_model.download_segments)));

        let (tx, segments) = (self.tx.clone(), self.cfg.local_model.download_segments);
        let progress_tx = tx.clone();
        self.dl_cancel = Some(tokio::spawn(async move {
            let on_progress = std::sync::Arc::new(move |p: harness::localmodel::Progress| { let _ = progress_tx.send(Msg::ModelDl(p)); });
            let r = harness::localmodel::fetch(build, segments, on_progress).await;
            let _ = tx.send(Msg::ModelDlDone(r.map(|d| d.display().to_string()).map_err(|e| format!("{e:#}"))));
        }));
        self.offer_claude_meanwhile();
    }

    /// Give the user something to work with while 16–30 GB comes down. Asking the CLI takes ~0.2s, so it
    /// happens off the UI thread and lands as Msg::ClaudeAuth.
    fn offer_claude_meanwhile(&mut self) {
        if self.cfg.llm.provider.is_some() || self.claude_asked { return; }   // already answered, or on another backend
        self.claude_asked = true;
        let tx = self.tx.clone();
        tokio::spawn(async move { let _ = tx.send(Msg::ClaudeAuth(harness::claude_code::auth().await)); });
    }

    /// Claude if it is signed in; an offer to sign in if it is not; and if there is no usable Claude at
    /// all — or the user would rather not — say plainly that the harness waits for Qwen3.8.
    fn on_claude_auth(&mut self, auth: harness::claude_code::Auth) {
        use harness::claude_code::Auth;
        let downloading = self.dl.is_some() || self.dl_cancel.is_some();
        match auth {
            Auth::Ready { .. } => {
                // Signed in, so the choice is the user's: which model, at effort high, or none at all.
                // Fable is first, which is where the cursor starts.
                let who = auth.who();
                let mut items: Vec<PickItem> = CLAUDE_MODELS.iter().map(|(m, note)| PickItem {
                    label: (*m).to_string(),
                    desc: format!("{note} · effort high"),
                    detail: format!("Runs turns through the official Claude Code CLI on your subscription ({who}), with the \
                                     harness's own tools bridged over MCP — permissions, hooks, memory and redaction all still apply.\n\n\
                                     Effort starts at high; /effort changes it, /backend local switches away.{}",
                                     if downloading { "\n\nThe Qwen3.8 download keeps running in the background either way." } else { "" }),
                    run: Some(format!("/backend claude {m} high")),
                }).collect();
                items.push(PickItem {
                    label: "No Claude".into(),
                    desc: if downloading { "wait for Qwen3.8 to finish downloading".into() } else { "no backend until a local model exists".into() },
                    detail: if downloading {
                        "Nothing runs until the weights land — the harness will use Qwen3.8-27B as soon as they do, and turns \
                         attempted before then will fail.".into()
                    } else {
                        "No usable backend is configured. /localmodel downloads one, or point [llm] base_url at a server you run.".into()
                    },
                    run: None,
                });
                self.pick = Some(ListPicker::new(
                    format!("No local model yet — work on Claude? ({who})"),
                    "enter picks a model at effort high · esc declines",
                    items));
            }
            Auth::LoggedOut | Auth::Missing => {
                let missing = auth == Auth::Missing;
                let mut items = Vec::new();
                if missing {
                    items.push(PickItem {
                        label: "Install the Claude Code CLI".into(),
                        desc: "then sign in — the harness suspends while you do".into(),
                        detail: "Runs `curl -fsSL https://claude.ai/install.sh | bash`, then `claude auth login`. Your Anthropic \
                                 subscription is used through the official client; the harness bridges its own tools to it over MCP.".into(),
                        run: Some("/claude-login install".into()),
                    });
                } else {
                    items.push(PickItem {
                        label: "Sign in to Claude now".into(),
                        desc: format!("runs `{}` — the harness steps aside, then comes back", harness::claude_code::LOGIN_COMMAND),
                        detail: "The TUI suspends, the official CLI takes the terminal for the browser sign-in, and the harness \
                                 resumes on your subscription. Nothing about your account is stored by the harness.".into(),
                        run: Some("/claude-login".into()),
                    });
                }
                items.push(PickItem {
                    label: "No Claude — wait for Qwen3.8".into(),
                    desc: if downloading { "the harness starts working when the weights land".into() } else { "nothing to run until a model exists".into() },
                    detail: if downloading {
                        "The download continues; you can watch it in the panel (⌃P). The harness has no model to talk to until it \
                         finishes, so turns will fail until then — everything else (tools, /commands, files) still works.".into()
                    } else {
                        "No backend is configured. /localmodel picks a build to download, or set [llm] base_url in \
                         ~/.config/harness/harness.toml to use a server you already run.".into()
                    },
                    run: None,
                });
                self.pick = Some(ListPicker::new(
                    if missing { "Claude Code is not installed" } else { "Claude Code is installed but not signed in" },
                    "enter chooses · esc waits for the local model",
                    items));
            }
        }
    }

    /// `/factory-reset`: back to first-run state. Shows the plan; `confirm` performs it, stops the MLX
    /// server, and restarts the harness so the next launch is a clean first run.
    fn factory_reset(&mut self, confirmed: bool) {
        use harness::reset;
        let plan = reset::factory_reset_plan();
        if plan.is_empty() { self.blocks.push(Block::System("factory reset: nothing to remove — the harness is already at first-run state".into())); return; }
        let total: u64 = plan.iter().map(|i| i.bytes).sum();
        if !confirmed {
            let mut lines = vec![format!("⚠ /factory-reset will delete {} across {} item(s), returning the harness to first-run state:", reset::human(total), plan.len())];
            for i in &plan { lines.push(format!("  · {} — {}", i.label, reset::human(i.bytes))); }
            lines.push("Kept: the MLX runtime, the source checkout, tool links (the installer parts). The model re-downloads on next launch.".into());
            lines.push("Type  /factory-reset confirm  to proceed. This cannot be undone.".into());
            self.blocks.push(Block::Banner(lines));
            return;
        }
        let port = self.cfg.local_model.port;
        tokio::spawn(async move { harness::localmodel::stop_on_port(port).await; });
        let (removed, errs) = reset::execute(&plan);
        let _ = reset::seed_after_reset();
        self.blocks.push(Block::System(format!("factory reset: removed {removed} item(s), freed {}", reset::human(total))));
        for e in &errs { self.blocks.push(Block::Error(format!("  could not remove {e}"))); }
        self.blocks.push(Block::System("restarting into a clean first run…".into()));
        self.restart = true; self.quit = true;
    }

    /// `/uninstall`: remove everything the harness owns. Shows the plan; `confirm` performs it and quits.
    fn uninstall(&mut self, confirmed: bool) {
        use harness::reset;
        let plan = reset::uninstall_plan();
        let total: u64 = plan.iter().map(|i| i.bytes).sum();
        if !confirmed {
            let mut lines = vec![format!("⚠ /uninstall will remove {} — everything the harness owns:", reset::human(total))];
            for i in &plan { lines.push(format!("  · {} ({})", i.path.display(), reset::human(i.bytes))); }
            let shared = reset::shared_tools_left();
            if !shared.is_empty() {
                lines.push("Left in place (shared tools the installer also set up — remove by hand if you want):".into());
                for (n, p) in &shared { lines.push(format!("  · {n}: {}", p.display())); }
            }
            lines.push("Type  /uninstall confirm  to proceed. The harness quits when done; this cannot be undone.".into());
            self.blocks.push(Block::Banner(lines));
            return;
        }
        let port = self.cfg.local_model.port;
        // stop the server synchronously-ish before the runtime dir is deleted under it
        tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(harness::localmodel::stop_on_port(port)));
        let (removed, errs) = reset::execute(&plan);
        self.blocks.push(Block::System(format!("uninstall: removed {removed} item(s), freed {}", reset::human(total))));
        for e in &errs { self.blocks.push(Block::Error(format!("  could not remove {e}"))); }
        self.blocks.push(Block::System("TheHarness is uninstalled. Reinstall any time: curl -fsSL https://zeljan-alduk.github.io/TheHarness/install.sh | sh".into()));
        // do NOT restart — the binary is gone. Just quit.
        self.restart = false; self.quit = true;
    }

    /// Start (or reuse) the MLX server for the configured build and report where it landed.
    /// `force` restarts even a healthy server — for `/localmodel restart` and switching vision↔text.
    fn start_mlx(&mut self) { self.start_mlx_ex(false); }
    fn start_mlx_ex(&mut self, force: bool) {
        use harness::localmodel as lm;
        let Some(build) = lm::by_name(&self.cfg.local_model.build) else { self.set_status("no local build chosen — /localmodel"); return };
        let gguf = build.is_gguf();
        if !gguf && lm::mlx_python().is_none() {
            self.blocks.push(Block::Error("no MLX runtime in ~/.config/harness/runtime/mlx — re-run the installer (or NO_MLX=0 sh install.sh)".into()));
            return;
        }
        let (tx, port, kind) = (self.tx.clone(), self.cfg.local_model.port, self.cfg.local_model.server.clone());
        let opts = lm::ServeOpts::from_cfg(&self.cfg.local_model);
        let draft = opts.draft.clone();
        let kv = opts.kv_bits;
        let server_name = if gguf { "llama-server".to_string() } else { lm::server_plan(&kind).first().copied().unwrap_or("mlx_lm.server").to_string() };
        self.set_status(format!("{} {server_name} on port {port}{}{}", if force { "restarting" } else { "starting" }, if draft.on() && !gguf { format!(" · speculative draft {}", draft.model) } else { String::new() }, if (kv == 8 || kv == 4) && !gguf { format!(" · kv {kv}-bit") } else { String::new() }));
        tokio::spawn(async move {
            // Keep it ready: a warm server of the right runtime from an earlier session is reused as-is —
            // no reload of the weights, the prefix cache survives. `force` skips this to restart.
            if !force {
                if let Some(model) = lm::running_build(&build, port, &kind).await {
                    let module = lm::server_kind(&format!("http://127.0.0.1:{port}/v1")).await.unwrap_or(if gguf { "llama-server" } else { "mlx_vlm.server" });
                    let _ = tx.send(Msg::MlxUp(Ok((format!("http://127.0.0.1:{port}/v1"), model, module))));
                    return;
                }
            }
            // GGUF needs llama.cpp — the harness installs it with Homebrew when it is missing.
            if gguf {
                match lm::ensure_llama_server().await {
                    Ok(Some(note)) => { let _ = tx.send(Msg::Notice(note)); }
                    Ok(None) => {}
                    Err(e) => { let _ = tx.send(Msg::MlxUp(Err(format!("{e:#}")))); return; }
                }
            }
            // The runtime may predate the vision server (an updated harness on an old venv): fetch it first.
            if !gguf && !matches!(kind.as_str(), "mlx-lm" | "mlx_lm" | "text") {
                match lm::ensure_mlx_vlm().await {
                    Ok(Some(note)) => { let _ = tx.send(Msg::Notice(note)); }
                    Ok(None) => {}
                    Err(e) => { let _ = tx.send(Msg::Notice(format!("{e:#} — starting the text-only server meanwhile"))); }
                }
            }
            // ours from an earlier session (or the other server kind) may still hold the port: replace it
            lm::stop_on_port(port).await;
            let _ = tx.send(Msg::Notice(format!("loading {} GB of weights — a moment", build.bytes / 1_000_000_000)));
            let msg = match lm::serve_build(&build, port, &kind, &opts).await {
                Ok(s) => Msg::MlxUp(Ok((s.base_url.clone(), s.model.clone(), s.module))),
                Err(e) => Msg::MlxUp(Err(format!("{e:#}"))),
            };
            let _ = tx.send(msg);
        });
    }

    /// Change the terminal font size (kitty / iTerm2 / Terminal.app), persist it, toast it. delta 0 = reset to 13.
    fn adjust_font(&mut self, delta: i32) {
        let cur = if self.cfg.ui.font_size > 0 { self.cfg.ui.font_size as i32 } else { 13 };
        let next = if delta == 0 { 13 } else { (cur + delta).clamp(6, 40) };
        self.cfg.ui.font_size = next as u32;
        let _ = harness::config::Config::save_setting("ui.font_size", &next.to_string());
        let tx = self.tx.clone();
        tokio::spawn(async move { let _ = tx.send(Msg::Toast(match set_terminal_font_size(next as u32).await { Ok(t) => format!("font size {next}pt ({t})"), Err(e) => format!("font size: {e}") })); });
    }

    /// Extract the selected text from the visible transcript rows and put it on the clipboard.
    fn copy_selection(&mut self, a: (u16, u16), c: (u16, u16)) {
        let (mut p1, mut p2) = (a, c); if (p1.1, p1.0) > (p2.1, p2.0) { std::mem::swap(&mut p1, &mut p2); }
        // rectangle vs stream: a drag within one column range across rows behaves like a stream selection (like terminals)
        let mut out = Vec::new();
        for row in p1.1..=p2.1 {
            let Some(text) = self.visible_text.get(row as usize) else { continue };
            let chars: Vec<char> = text.chars().collect();
            let from = if row == p1.1 { p1.0 as usize } else { 0 };
            let to = if row == p2.1 { (p2.0 as usize + 1).min(chars.len()) } else { chars.len() };
            if from < to { out.push(chars[from..to].iter().collect::<String>().trim_end().to_string()); } else { out.push(String::new()); }
        }
        let text = out.join("\n").trim_end().to_string();
        if text.is_empty() { return; }
        let n = text.chars().count(); let lines = out.len();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let cmd = if cfg!(target_os = "macos") { "pbcopy" } else if cfg!(windows) { "clip" } else { "xclip -selection clipboard 2>/dev/null || wl-copy" };
            let (prog, flag) = harness::sandbox::shell_program();
            let mut c = tokio::process::Command::new(prog); c.arg(flag).arg(cmd).stdin(std::process::Stdio::piped());
            match c.spawn() { Ok(mut ch) => { if let Some(mut si) = ch.stdin.take() { use tokio::io::AsyncWriteExt; let _ = si.write_all(text.as_bytes()).await; } let ok = ch.wait().await.map(|s| s.success()).unwrap_or(false); let _ = tx.send(Msg::Toast(if ok { format!("✓ copied {n} chars, {lines} line{}", if lines == 1 { "" } else { "s" }) } else { "✗ clipboard tool failed".into() })); } Err(e) => { let _ = tx.send(Msg::Toast(format!("✗ clipboard: {e}"))); } }
        });
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

/// Longest common prefix of the candidates (tab completion).
fn common_prefix(items: &[&str]) -> Option<String> {
    let first = items.first()?;
    let mut len = first.len();
    for it in &items[1..] {
        len = len.min(it.len());
        while len > 0 && (!first.is_char_boundary(len) || !it.is_char_boundary(len) || first[..len] != it[..len]) { len -= 1; }
    }
    (len > 0).then(|| first[..len].to_string())
}

/// ctrl+g: hand the prompt to $EDITOR and take back whatever was saved.
fn external_edit(current: &str) -> Result<String> {
    let editor = std::env::var("VISUAL").or_else(|_| std::env::var("EDITOR")).unwrap_or_else(|_| "vi".into());
    let path = std::env::temp_dir().join(format!("harness-prompt-{}.md", std::process::id()));
    std::fs::write(&path, current)?;
    let mut parts = editor.split_whitespace();
    let prog = parts.next().unwrap_or("vi");
    let status = std::process::Command::new(prog).args(parts).arg(&path).status()?;
    if !status.success() { anyhow::bail!("{editor} exited with {status}"); }
    let text = std::fs::read_to_string(&path)?;
    let _ = std::fs::remove_file(&path);
    Ok(text.trim_end().to_string())
}

#[cfg(test)]
mod tui_tests {
    use super::*;

    const DIFF: &str = "diff --git a/src/a.rs b/src/a.rs\nindex 111..222 100644\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,3 +1,4 @@\n fn main() {\n-    old();\n+    new();\n+    extra();\n }\n@@ -20,2 +21,2 @@\n-    gone();\n+    kept();\ndiff --git a/b.txt b/b.txt\n--- a/b.txt\n+++ b/b.txt\n@@ -1 +1 @@\n-one\n+two\n";

    #[test]
    fn splits_diff_into_hunks() {
        let h = parse_hunks(DIFF);
        assert_eq!(h.len(), 3, "{h:#?}");
        assert_eq!(h[0].file, "src/a.rs");
        assert_eq!((h[0].plus, h[0].minus), (2, 1));
        assert_eq!(h[1].file, "src/a.rs");
        assert_eq!(h[2].file, "b.txt");
        assert!(!h[0].body.iter().any(|l| l.starts_with("index ") || l.starts_with("+++")), "file headers are not part of the hunk body");
        let patch = h[2].patch();
        assert!(patch.starts_with("--- a/b.txt\n+++ b/b.txt\n@@ -1 +1 @@"), "{patch}");
        assert!(patch.contains("-one") && patch.contains("+two"));
        assert!(parse_hunks("").is_empty());
    }

    #[test]
    fn suggestion_windowing() {
        // everything fits: no scrolling
        assert_eq!(suggestion_window(4, Some(3), 6), (0, 4));
        // long list, nothing selected yet: from the top
        assert_eq!(suggestion_window(20, None, 6), (0, 6));
        // selection near the top stays at the top, in the middle centres, at the end sticks
        assert_eq!(suggestion_window(20, Some(1), 6), (0, 6));
        assert_eq!(suggestion_window(20, Some(10), 6), (7, 13));
        assert_eq!(suggestion_window(20, Some(19), 6), (14, 20));
        // the typed prefix filters, and every match is available to the arrows
        assert!(suggestions("/co").iter().all(|(c, _)| c.starts_with("/co")));
        assert!(suggestions("/").len() > 6, "the list is not truncated to the visible rows");
        assert!(suggestions("hello").is_empty() && suggestions("/model x").is_empty());
    }

    #[test]
    fn one_turn_renders_one_answer_even_when_reasoning_streams_first() {
        // The sequence mlx_lm.server produces: reasoning deltas, then content deltas, then the agent's
        // end-of-call full Reasoning and Assistant events. Finalising by tail alone appended copies of both.
        let now = Instant::now();
        let mut blocks: Vec<Block> = vec![
            Block::Reasoning { text: "We need".into(), streaming: true, show: None, started: now, ended: None },
            Block::Assistant { text: "Hello".into(), streaming: true, folded: false },
        ];
        finalize_reasoning(&mut blocks, "We need to answer".into());
        finalize_assistant(&mut blocks, "Hello there".into());
        assert_eq!(blocks.len(), 2, "the streamed blocks were finalised, not duplicated");
        match &blocks[0] { Block::Reasoning { text, streaming, ended, .. } => { assert_eq!(text, "We need to answer"); assert!(!streaming); assert!(ended.is_some()); } _ => panic!("block 0 should be the reasoning") }
        match &blocks[1] { Block::Assistant { text, streaming, .. } => { assert_eq!(text, "Hello there"); assert!(!streaming); } _ => panic!("block 1 should be the answer") }

        // A turn that streamed nothing (a provider that hides thinking) still gets its blocks.
        let mut fresh: Vec<Block> = Vec::new();
        finalize_reasoning(&mut fresh, "thought".into());
        finalize_assistant(&mut fresh, "answer".into());
        assert_eq!(fresh.len(), 2);

        // An earlier turn's finished answer must not be overwritten by the next turn's.
        let mut two: Vec<Block> = vec![Block::Assistant { text: "first answer".into(), streaming: false, folded: false }];
        finalize_assistant(&mut two, "second answer".into());
        assert_eq!(two.len(), 2, "an unrelated finished answer is left alone");
        // Empty finals never create a block.
        let mut none: Vec<Block> = Vec::new();
        finalize_assistant(&mut none, "   ".into());
        finalize_reasoning(&mut none, String::new());
        assert!(none.is_empty());
    }

    #[test]
    fn model_label_shortens_paths_only() {
        // mlx_lm.server reports the directory it was started with
        assert_eq!(model_label("/Users/a/.config/harness/models/Qwen3.8-27B-MLX-4bit"), "Qwen3.8-27B-MLX-4bit");
        assert_eq!(model_label("/Users/a/models/Qwen3.8-27B-MLX-4bit/"), "Qwen3.8-27B-MLX-4bit");
        // publisher-style and plain ids are already what a person wants to read
        assert_eq!(model_label("qwen/qwen3.6-35b-a3b"), "qwen/qwen3.6-35b-a3b");
        assert_eq!(model_label("claude-fable-5"), "claude-fable-5");
        assert_eq!(model_label(""), "");
    }

    #[test]
    fn completion_prefix() {
        assert_eq!(common_prefix(&["/compact", "/config", "/context"]).as_deref(), Some("/co"));
        assert_eq!(common_prefix(&["only"]).as_deref(), Some("only"));
        assert_eq!(common_prefix(&["ab", "cd"]), None);
        assert_eq!(common_prefix(&[]), None);
    }
}

/// How to record a few seconds of microphone audio on this machine ({secs}, {out} are substituted).
fn voice_record_command() -> Option<String> {
    if cfg!(target_os = "macos") && harness::setup::which("ffmpeg").is_some() {
        return Some("ffmpeg -hide_banner -loglevel error -f avfoundation -i :default -t {secs} -ar 16000 -ac 1 -y {out}".into());
    }
    if harness::setup::which("arecord").is_some() { return Some("arecord -q -f S16_LE -r 16000 -c 1 -d {secs} {out}".into()); }
    if harness::setup::which("sox").is_some() { return Some("sox -q -d -r 16000 -c 1 {out} trim 0 {secs}".into()); }
    if harness::setup::which("ffmpeg").is_some() { return Some("ffmpeg -hide_banner -loglevel error -f alsa -i default -t {secs} -ar 16000 -ac 1 -y {out}".into()); }
    None
}

/// How to turn that wav into text ({in} is substituted): whisper.cpp, then openai-whisper.
fn voice_transcribe_command() -> Option<String> {
    for bin in ["whisper-cli", "whisper-cpp", "main"] {
        if harness::setup::which(bin).is_some() {
            let model = harness::setup::home_dir().join(".config/harness/models/ggml-base.en.bin");
            if model.is_file() { return Some(format!("{bin} -m {} -f {{in}} -nt -np", model.display())); }
        }
    }
    if harness::setup::which("whisper").is_some() { return Some("whisper {in} --model base.en --output_format txt --output_dir /tmp --fp16 False 2>/dev/null && cat /tmp/$(basename {in} .wav).txt".into()); }
    None
}

const COMMANDS: &[(&str, &str)] = &[
    ("/help", "show commands and keys"),
    ("/clear", "start a new session (forget the transcript)"),
    ("/sessions", "pick a saved session (↑/↓/click, enter) · /sessions list · /sessions live · /sessions search <text>"),
    ("/msg", "message another live session: /msg <id|prefix|title|all> <text>"),
    ("/resume", "resume a saved session: /resume <n|id|last>"),
    ("/model", "browse every model you can switch to — Claude, the downloaded MLX build, and this server (enter switches backend + model); /model <name> switches directly"),
    ("/backend", "switch backend: local (LM Studio etc.) | claude [model] [effort] (Claude Code CLI, subscription) | anthropic <model>"),
    ("/localmodel", "the local Qwen3.8-27B: download 4/6/8-bit · serve · stop · restart · status · vision|text · draft <hf-id|off> (speculative decoding) · kv <8|4|off> (KV-cache quantization: less RAM, faster long context)"),
    ("/delegate", "Claude orchestrates while the local model runs the delegated work (sub-agents): /delegate on|off"),
    ("/effort", "Claude Code backend reasoning effort: /effort low|medium|high|xhigh|max (default medium)"),
    ("/cd", "change working directory"),
    ("/pwd", "print working directory"),
    ("/tools", "browse the tools the model can call (arrows, type to filter, full schema below)"),
    ("/net", "internet tools on|off"),
    ("/thinking", "toggle showing the model's reasoning"),
    ("/expand", "toggle expanded tool output (ctrl+o)"),
    ("/panel", "toggle the dashboard panel (ctrl+p)"),
    ("/cost", "token usage and $ for this session · /cost <max-usd> caps the spend"),
    ("/usage", "Claude backend: subscription usage (proxied Claude Code /usage); otherwise same as /cost"),
    ("/compact", "compact the context into a precise handoff note: /compact [focus]"),
    ("/context", "context map: what fills the window (prompt, tools, memory, messages) + heaviest items"),
    ("/settings", "interactive settings panel (also /config)"),
    ("/memory", "show MEMORY.md (settings · preferences · ideas)"),
    ("/brain", "show BRAIN.md (what the agent learned)"),
    ("/workflows", "show WORKFLOWS.md (recipes)"),
    ("/remember", "add a note: /remember <text> | brain: <text> | workflows: <text>"),
    ("/reflect", "ask the model what to remember from this session"),
    ("/dream", "consolidate memory: merge duplicates, drop stale notes, draft skills from what was learned"),
    ("/spec", "spec-driven work: /spec <feature> writes requirements/design/tasks · /spec implement <slug>"),
    ("/learn", "learn a URL or directory into a skill: /learn <url|path> [as <name>]"),
    ("/video", "open the frame scrubber for a video: /video <path>"),
    ("/plugin", "plugins: list · install <owner/repo> · enable|disable|remove|update|info <name>"),
    ("/mcp", "show configured MCP servers and live MCP tools"),
    ("/reload", "restart tools, MCP servers and plugins"),
    ("/restart", "restart the harness (re-exec the installed binary) and resume this session"),
    ("/factory-reset", "wipe user state + the downloaded model → first-run state (keeps the MLX runtime/source); /factory-reset confirm"),
    ("/uninstall", "remove everything the harness owns (config, model, runtime, app, binary); /uninstall confirm — then it quits"),
    ("/update", "check GitHub for a newer release (installing happens when harness starts, never under a running session; `harness update` on demand)"),
    ("/improve", "self-improvement loop: /improve [focus] — propose → confirm (auto with a frontier backend) → implement → arbiter → merge → install → restart (60s to cancel)"),
    ("/cancel", "cancel the pending automatic restart or the running /improve job"),
    ("/permissions", "show or set permission mode: bypass|auto|ask|plan"),
    ("/plan", "toggle plan mode (read-only)"),
    ("/trust", "remember this directory as trusted (no first-time notice)"),
    ("/theme", "switch theme: /theme light|dark|<name> · /theme list (custom themes are JSON in ~/.config/harness/themes)"),
    ("/vim", "toggle vim-style modal editing in the prompt"),
    ("/voice", "dictate the prompt: /voice [seconds] (needs ffmpeg/sox + whisper.cpp)"),
    ("/workflow", "run a workflow: /workflow <name> [args]  (list with /workflow)"),
    ("/queue", "queue a task instead of steering: /queue <text> · show the queue · /queue clear"),
    ("/jobs", "browse scheduled jobs - enter runs one now - /jobs remove <name>"),
    ("/arena", "best-of-n: /arena [models] -- <task> runs it in parallel worktrees and judges the results"),
    ("/goal", "keep working until a condition holds: /goal <condition> · /goal off (checked by the aux model each turn)"),
    ("/next", "stop the current task and start the next queued one (ctrl+n)"),
    ("/status", "backend, context, session, permissions at a glance"),
    ("/doctor", "check external tools, claude CLI, config paths"),
    ("/init", "have the agent write HARNESS.md project instructions"),
    ("/add-dir", "allow file tools to access another directory this session"),
    ("/rename", "rename the current session"),
    ("/export", "export the transcript to markdown (/export html for a page)"),
    ("/share", "export a self-contained HTML page of this session (/share gist uploads it with gh)"),
    ("/import", "import Claude Code / Codex transcripts into the session store (/import claude|codex|<file>)"),
    ("/todos", "show the agent's todo list"),
    ("/hooks", "show configured hooks"),
    ("/skills", "browse skills (arrows, filter; the full SKILL.md shows below)"),
    ("/prompt", "run a prompt offered by an MCP server: /prompt <server>:<name> [args] (bare /prompt lists them)"),
    ("/commands", "browse markdown slash commands - enter runs the highlighted one"),
    ("/btw", "ask a side question about this session without touching the conversation"),
    ("/recap", "summarise the session so far (aux model)"),
    ("/find", "search this session's transcript: /find <text>"),
    ("/diff", "git status + diff stat of the working tree"),
    ("/review-diff", "review the working tree hunk by hunk: keep · revert · comment, then send the comments to the agent"),
    ("/copy", "copy the last answer to the clipboard"),
    ("/review", "run the review workflow on the working-tree diff"),
    ("/pr-comments", "show PR comments via gh: /pr-comments [number]"),
    ("/rewind", "rewind to a checkpoint: /rewind <n> (files + conversation) · /rewind code <n> · /rewind conv"),
    ("/checkpoints", "browse file checkpoints - enter rewinds files and conversation to that point"),
    ("/undo", "restore the files to the previous checkpoint (/undo <n> steps)"),
    ("/redo", "re-apply what /undo reverted"),
    ("/fork", "continue this conversation as a new, separately saved session"),
    ("/release-notes", "recent commits"),
    ("/agents", "sub-agents: list · attach <id> (watch + message it) · detach · kill <id|all>"),
    ("/keybindings", "list keyboard shortcuts"),
    ("/exit", "quit"),
];

// ───────────────────────── rendering ─────────────────────────
fn draw(f: &mut Frame, app: &mut App) {
    let full = f.area();
    // the start-up card animates one step per rendered frame
    if app.blocks.iter().any(|b| matches!(b, Block::Startup(_))) { app.banner_step = app.banner_step.saturating_add(1); }
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
    const SUGG_ROWS: usize = 6;
    let sugg_shown = sugg.len().min(SUGG_ROWS) + usize::from(sugg.len() > SUGG_ROWS);
    let input_h = (input_lines.len().clamp(1, 8) + sugg_shown + if app.attachments.is_empty() { 0 } else { 1 }) as u16;
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
    if app.sessions_pick.is_some() { draw_sessions(f, app, tr_area); }
    if app.pick.is_some() { draw_pick(f, app, tr_area); }
    if app.review.is_some() { draw_review(f, app, tr_area); }
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
    if app.video.is_none() && !app.settings_open && app.sessions_pick.is_none() && app.pick.is_none() && app.review.is_none() { f.render_widget(Paragraph::new(visible), tr_area); }
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
    let (from, to) = suggestion_window(sugg.len(), app.sugg_idx, SUGG_ROWS);
    for (i, (c, d)) in sugg.iter().enumerate().take(to).skip(from) {
        let selected = app.sugg_idx == Some(i);
        let (marker, cmd_style, desc_style) = if selected {
            ("▸ ", Style::default().fg(Color::Black).bg(pal().blue).bold(), Style::default().fg(pal().fg))
        } else {
            ("  ", Style::default().fg(pal().blue), Style::default().fg(pal().dim))
        };
        let mut spans = vec![Span::styled(marker, Style::default().fg(pal().blue)), Span::styled(format!("{c:<12}"), cmd_style), Span::styled(d.to_string(), desc_style)];
        if selected { spans.push(Span::styled("   enter runs it · tab edits it", Style::default().fg(pal().dim))); }
        in_lines.push(Line::from(spans));
    }
    if sugg.len() > SUGG_ROWS {
        in_lines.push(Line::from(Span::styled(format!("  {}/{} — ↑/↓ to move", app.sugg_idx.map(|i| i + 1).unwrap_or(0), sugg.len()), Style::default().fg(pal().dim))));
    }
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
        // A model name here is a promise that a turn will work; when nothing is loaded, say that instead.
        Span::styled(
            if app.no_model && app.cfg.llm.provider.is_none() { "no model loaded".to_string() }
            else { format!("{}{}", model_label(&app.model), if app.cfg.llm.provider.as_deref() == Some("claude-code") { app.cfg.llm.effort.as_ref().map(|e| format!(" · effort {e}")).unwrap_or_default() } else { String::new() }) },
            Style::default().fg(if app.no_model && app.cfg.llm.provider.is_none() { pal().err } else { pal().cyan })), dot(),
        Span::styled(short_path(&app.workdir), Style::default().fg(pal().cyan))];
    if !app.net { st.push(dot()); st.push(Span::styled("offline", Style::default().fg(pal().pink))); }
    if !app.queued.is_empty() { st.push(dot()); st.push(Span::styled(format!("{} queued", app.queued.len()), Style::default().fg(pal().cyan))); }
    if let Some(id) = app.attached { st.push(dot()); st.push(Span::styled(format!("attached #{id}"), Style::default().fg(pal().orange))); }
    if app.vim { st.push(dot()); st.push(Span::styled(if app.vim_normal { "-- NORMAL --" } else { "-- INSERT --" }, Style::default().fg(if app.vim_normal { pal().orange } else { pal().ok }).bold())); }
    if let Some(wt) = app.wt_cwd.lock().unwrap().as_ref() { st.push(dot()); st.push(Span::styled(format!("worktree {}", wt.name), Style::default().fg(pal().orange))); }
    let lw: usize = st.iter().map(|s| s.content.chars().count()).sum();
    let custom = app.statusline.clone().unwrap_or_default();
    let right_owned = match (app.running.is_none(), custom.is_empty()) {
        (_, false) => custom,
        (false, true) => "esc to interrupt".to_string(),
        _ => String::new(),
    };
    let right = right_owned.as_str();
    let pad = width.saturating_sub(lw + right.chars().count() + 1);
    st.push(Span::raw(" ".repeat(pad))); st.push(Span::styled(right, Style::default().fg(pal().dim)));
    f.render_widget(Paragraph::new(Line::from(st)), st_area);

    // ── whole-screen text snapshot (for drag-to-copy) + selection highlight + toast ──
    let full_area = f.area();
    {
        let buf = f.buffer_mut();
        let mut rows: Vec<String> = Vec::with_capacity(full_area.height as usize);
        for y in 0..full_area.height { let mut row = String::new(); for x in 0..full_area.width { let sym = buf[(x, y)].symbol(); if sym.is_empty() { row.push(' '); } else { row.push_str(sym); } } rows.push(row); }
        app.visible_text = rows;
        if let (Some(a), Some(c)) = (app.sel_anchor, app.sel_cur) {
            if app.sel_dragging && a != c {
                let (mut p1, mut p2) = (a, c); if (p1.1, p1.0) > (p2.1, p2.0) { std::mem::swap(&mut p1, &mut p2); }
                for y in p1.1..=p2.1.min(full_area.height.saturating_sub(1)) {
                    let x0 = if y == p1.1 { p1.0 } else { 0 }; let x1 = if y == p2.1 { p2.0 } else { full_area.width.saturating_sub(1) };
                    for x in x0..=x1.min(full_area.width.saturating_sub(1)) { buf[(x, y)].set_style(Style::default().fg(Color::Black).bg(pal().cyan)); }
                }
            }
        }
    }
    if let Some((t, at)) = &app.toast { if at.elapsed() < Duration::from_millis(2500) { let w = (t.chars().count() as u16 + 4).min(full_area.width); let r = Rect { x: full_area.width.saturating_sub(w + 1), y: full_area.height.saturating_sub(4), width: w, height: 1 }; f.render_widget(Paragraph::new(Span::styled(format!("  {t}  "), Style::default().fg(Color::Black).bg(pal().ok).bold())), r); } else { app.toast = None; } }
}

// ───────────────────────── settings panel ─────────────────────────
/// One row of the generic list picker.
#[derive(Clone, Debug)]
struct PickItem {
    label: String,
    /// One-line summary shown next to the label (truncated to fit the row).
    desc: String,
    /// The full text shown in the detail pane — never truncated by the list width.
    detail: String,
    /// Slash command run when enter is pressed (None = the entry is informational).
    run: Option<String>,
}

/// A filterable list with a detail pane: `/tools`, `/skills`, `/commands`, `/jobs`, `/checkpoints`,
/// `/model`, `/workflow`, `/agents`. The point is that nothing is cut off — the selected entry's full
/// description (schema, body, log) is readable underneath the list.
struct ListPicker { title: String, hint: String, items: Vec<PickItem>, cursor: usize, top: usize, filter: String, rows: Rect }

impl ListPicker {
    fn new(title: impl Into<String>, hint: impl Into<String>, items: Vec<PickItem>) -> Self {
        Self { title: title.into(), hint: hint.into(), items, cursor: 0, top: 0, filter: String::new(), rows: Rect::default() }
    }
    /// Indices of the entries matching the filter (label and summary are searched).
    fn matches(&self) -> Vec<usize> {
        if self.filter.is_empty() { return (0..self.items.len()).collect(); }
        let f = self.filter.to_lowercase();
        (0..self.items.len()).filter(|i| { let it = &self.items[*i]; it.label.to_lowercase().contains(&f) || it.desc.to_lowercase().contains(&f) }).collect()
    }
    fn mv(&mut self, d: isize) {
        let n = self.matches().len();
        if n == 0 { self.cursor = 0; return; }
        self.cursor = (self.cursor as isize + d).rem_euclid(n as isize) as usize;
    }
    fn selected(&self) -> Option<&PickItem> { self.matches().get(self.cursor).map(|i| &self.items[*i]) }
}

struct SessionPicker { all: Vec<harness::sessions::Meta>, cursor: usize, filter: String, top: usize, rows: Rect, /// marquee: (row the offset belongs to, tick counter) — reset when the cursor moves
    marquee: (usize, u32) }
/// Marquee window over `s`: if it fits in `w` cells, pad it; otherwise show a `w`-wide slice that starts at `off`
/// (wrapping around after a gap so it reads like a ticker), pausing briefly at the start.
fn marquee(s: &str, w: usize, tick: u32) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= w { return format!("{:<w$}", s, w = w); }
    let gap = 4usize; let cycle = chars.len() + gap; let pause = 6u32;
    let off = if tick < pause { 0 } else { ((tick - pause) as usize) % cycle };
    let mut out = String::with_capacity(w);
    for i in 0..w { let j = (off + i) % cycle; out.push(if j < chars.len() { chars[j] } else { ' ' }); }
    out
}
impl SessionPicker {
    fn filtered(&self) -> Vec<&harness::sessions::Meta> {
        let q = self.filter.to_lowercase();
        self.all.iter().filter(|m| q.is_empty() || m.id.contains(&q) || m.title.to_lowercase().contains(&q) || m.workdir.to_lowercase().contains(&q)).collect()
    }
    fn selected_id(&self) -> Option<String> { self.filtered().get(self.cursor).map(|m| m.id.clone()) }
    fn mv(&mut self, d: i32) { let n = self.filtered().len(); if n == 0 { self.cursor = 0; return; } self.cursor = (self.cursor as i32 + d).clamp(0, n as i32 - 1) as usize; }
}

/// The generic list picker: filterable list on top, the selected entry in full underneath.
fn draw_pick(f: &mut Frame, app: &mut App, area: Rect) {
    f.render_widget(ratatui::widgets::Clear, area);
    let dim = Style::default().fg(pal().dim);
    let Some(p) = &mut app.pick else { return };
    let w = area.width as usize;
    let detail_h = (area.height / 3).clamp(4, 12) as usize;
    let list_h = (area.height as usize).saturating_sub(detail_h + 3).max(1);
    let idx = p.matches();
    let n = idx.len();
    if p.cursor >= n { p.cursor = n.saturating_sub(1); }
    if p.cursor < p.top { p.top = p.cursor; }
    if p.cursor >= p.top + list_h { p.top = p.cursor + 1 - list_h; }

    let mut lines: Vec<Line> = vec![
        Line::from(Span::styled(format!(" {} ({}{})  ·  {} ", p.title, n, if p.filter.is_empty() { String::new() } else { format!(" of {}", p.items.len()) }, p.hint), Style::default().fg(Color::Black).bg(pal().orange).bold())),
        Line::from(vec![Span::styled(" filter: ", dim), Span::raw(p.filter.clone()), Span::styled("▏", Style::default().fg(pal().orange))]),
    ];
    let label_w = idx.iter().map(|i| p.items[*i].label.chars().count()).max().unwrap_or(12).clamp(8, 32);
    for (row, i) in idx.iter().enumerate().skip(p.top).take(list_h) {
        let it = &p.items[*i];
        let sel = row == p.cursor;
        let st = if sel { Style::default().fg(Color::Black).bg(pal().orange).bold() } else { Style::default().fg(pal().blue) };
        let text = format!("{:<label_w$}  {}", truncate(&it.label, label_w), truncate(&it.desc, w.saturating_sub(label_w + 8)));
        lines.push(Line::from(vec![
            Span::styled(if sel { " ▸ " } else { "   " }, Style::default().fg(pal().orange)),
            Span::styled(format!("{:<width$}", text, width = w.saturating_sub(4)), st),
        ]));
    }
    if n == 0 { lines.push(Line::from(Span::styled("   nothing matches the filter", dim.italic()))); }
    lines.push(Line::from(Span::styled("─".repeat(w.min(200)), dim)));
    if let Some(it) = idx.get(p.cursor).and_then(|i| p.items.get(*i)) {
        let mut shown = 0usize;
        'outer: for para in it.detail.lines() {
            for chunk in wrap_text(para, w.saturating_sub(3)) {
                if shown >= detail_h { lines.push(Line::from(Span::styled("  …", dim))); break 'outer; }
                lines.push(Line::from(vec![Span::raw("  "), Span::styled(chunk, Style::default().fg(pal().fg))]));
                shown += 1;
            }
        }
    }
    p.rows = Rect { x: area.x, y: area.y + 2, width: area.width, height: list_h as u16 };
    f.render_widget(Paragraph::new(lines), area);
}

fn draw_sessions(f: &mut Frame, app: &mut App, area: Rect) {
    f.render_widget(ratatui::widgets::Clear, area);
    let dim = Style::default().fg(pal().dim);
    let cur_id = app.session_meta.id.clone();
    let Some(p) = &mut app.sessions_pick else { return };
    let avail = area.height.saturating_sub(4) as usize;
    if p.cursor < p.top { p.top = p.cursor; } if avail > 0 && p.cursor >= p.top + avail { p.top = p.cursor + 1 - avail; }
    if p.marquee.0 != p.cursor { p.marquee = (p.cursor, 0); }
    let items = p.filtered();
    let n = items.len();
    let hdr = format!(" Sessions ({}{})  ·  ↑/↓ or click select · enter / click again resume · type to filter · esc close ", n, if p.filter.is_empty() { String::new() } else { format!(" of {}", p.all.len()) });
    let mut lines: Vec<Line> = vec![Line::from(Span::styled(hdr, Style::default().fg(Color::Black).bg(pal().orange).bold()))];
    lines.push(Line::from(vec![Span::styled(" filter: ", dim), Span::raw(p.filter.clone()), Span::styled("▏", Style::default().fg(pal().orange)), Span::styled(if cur_id.is_empty() { String::new() } else { format!("      current: {cur_id}") }, dim)]));
    let top = p.top; let cursor = p.cursor;
    let tick = p.marquee.1;
    let w = area.width as usize;
    let title_w = w.saturating_sub(82).clamp(10, 70);
    for (i, m) in items.iter().enumerate().skip(top).take(avail.max(1)) {
        let sel = i == cursor;
        let is_cur = m.id == cur_id;
        let wd = short_path(std::path::Path::new(&m.workdir));
        let (t, d) = if sel { (marquee(&m.title, title_w, tick), marquee(&wd, 28, tick)) } else { (format!("{:<tw$}", truncate(&m.title, title_w), tw = title_w), format!("{:<28}", truncate(&wd, 28))) };
        let row = format!("{:>3}. {}  {}  {} {:>3} turns · {}", i + 1, m.id, t, d, m.turns, harness::sessions::fmt_age(m.updated));
        let st = if sel { Style::default().fg(Color::Black).bg(pal().orange).bold() } else if is_cur { Style::default().fg(pal().orange) } else { Style::default() };
        lines.push(Line::from(vec![Span::styled(if sel { " ▸ " } else { "   " }, Style::default().fg(pal().orange)), Span::styled(format!("{:<w2$}", row, w2 = w.saturating_sub(4)), st)]));
    }
    if n == 0 { lines.push(Line::from(Span::styled("   no sessions match", dim.italic()))); }
    drop(items);
    p.rows = Rect { x: area.x, y: area.y + 2, width: area.width, height: avail as u16 };
    f.render_widget(Paragraph::new(lines), area);
    if n > avail && avail > 0 {
        let sb_area = Rect { x: area.x + area.width.saturating_sub(1), y: area.y + 2, width: 1, height: avail as u16 };
        let mut st = ratatui::widgets::ScrollbarState::new(n.saturating_sub(avail)).position(top);
        f.render_stateful_widget(ratatui::widgets::Scrollbar::new(ratatui::widgets::ScrollbarOrientation::VerticalRight).thumb_style(Style::default().fg(pal().dim)).track_style(Style::default().fg(pal().dim)), sb_area, &mut st);
    }
}

fn draw_review(f: &mut Frame, app: &mut App, area: Rect) {
    f.render_widget(ratatui::widgets::Clear, area);
    let dim = Style::default().fg(pal().dim);
    let Some(r) = &mut app.review else { return };
    let n = r.hunks.len();
    let reverted = r.hunks.iter().filter(|h| h.reverted).count();
    let commented = r.hunks.iter().filter(|h| h.comment.is_some()).count();
    let hdr = format!(" Review diff — hunk {}/{} · {reverted} reverted · {commented} commented ", r.cursor + 1, n);
    let mut lines: Vec<Line> = vec![
        Line::from(Span::styled(hdr, Style::default().fg(Color::Black).bg(pal().orange).bold())),
        Line::from(Span::styled(" j/k move · a keep · r revert this hunk on disk · m comment · q send the review to the agent · esc close ", dim)),
    ];
    // hunk list (compact) — a window around the cursor
    let list_h = 6usize;
    let start = r.cursor.saturating_sub(list_h / 2).min(n.saturating_sub(list_h.min(n)));
    for (i, h) in r.hunks.iter().enumerate().skip(start).take(list_h) {
        let sel = i == r.cursor;
        let mark = if h.reverted { "⨯" } else if h.comment.is_some() { "✎" } else { " " };
        let row = format!("{mark} {:<40} {:<22} +{} -{}", truncate(&h.file, 40), truncate(&h.header, 22), h.plus, h.minus);
        let st = if sel { Style::default().fg(Color::Black).bg(pal().orange).bold() } else if h.reverted { dim } else { Style::default() };
        lines.push(Line::from(vec![Span::styled(if sel { " ▸ " } else { "   " }, Style::default().fg(pal().orange)), Span::styled(row, st)]));
    }
    lines.push(Line::from(Span::styled(" ─────", dim)));
    // the selected hunk itself
    if let Some(h) = r.hunks.get(r.cursor) {
        let avail = (area.height as usize).saturating_sub(lines.len() + 3);
        for l in h.body.iter().skip(r.scroll).take(avail) {
            let st = if l.starts_with('+') { Style::default().fg(pal().ok) } else if l.starts_with('-') { Style::default().fg(pal().err) } else { dim };
            lines.push(Line::from(Span::styled(format!("   {}", truncate(l, area.width as usize - 4)), st)));
        }
        if let Some(c) = &h.comment { lines.push(Line::from(Span::styled(format!("   ✎ {c}"), Style::default().fg(pal().cyan)))); }
    }
    if let Some(buf) = r.comment.as_ref() {
        lines.push(Line::from(vec![Span::styled(" comment: ", Style::default().fg(Color::Black).bg(pal().cyan)), Span::raw(buf.clone()), Span::styled("▏", Style::default().fg(pal().cyan)), Span::styled("  enter saves · esc cancels", dim)]));
    }
    f.render_widget(Paragraph::new(lines), area);
}

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
    // decode only what is on screen: the current frame and the strip around it
    ensure_frame_images(app, area);
    let Some(v) = &app.video else { return };
    let title = format!(" 🎞  {}  ·  {:.1}s  ·  {}  ·  {} selected ", short_path(&v.path), v.duration, if v.note.is_empty() { format!("{} frames", v.frames.len()) } else { v.note.clone() }, v.selected.len());
    // the preview takes everything the strip and the legend do not need
    let rows = Layout::vertical([Constraint::Length(1), Constraint::Min(8), Constraint::Length(1), Constraint::Length(6), Constraint::Length(2)]).split(area);
    f.render_widget(Paragraph::new(Line::from(vec![Span::styled(title, Style::default().fg(Color::Black).bg(pal().orange).bold())])), rows[0]);
    if v.loading { f.render_widget(Paragraph::new(Span::styled("  extracting frames with ffmpeg…", Style::default().fg(pal().orange))), rows[1]); }
    else if let Some(e) = &v.error { f.render_widget(Paragraph::new(Span::styled(format!("  {e}"), Style::default().fg(pal().err))), rows[1]); }
    let cur = v.cur;
    let frames: Vec<(f64, Option<String>)> = v.frames.iter().map(|(t, _, k)| (*t, k.clone())).collect();
    let selected = v.selected.clone();
    let (cw, ch) = app.picker.font_size();
    // main frame: as large as the pane allows, from its own protocol
    let preview = app.video.as_ref().and_then(|v| v.preview.clone());
    let source = app.video.as_ref().map(|v| v.source.clone()).unwrap_or_default();
    if let Some((ts, _)) = frames.get(cur).cloned() {
        // (rendered rect of the image, so the info can sit directly under it)
        let mut shown: Option<(Rect, (u32, u32), u64)> = None;
        if let Some((_, key, dims, bytes)) = &preview {
            if let Some((proto, (iw, ih))) = app.images.get_mut(key) {
                let (iw, ih) = (*iw as f64, *ih as f64);
                let max_cols = rows[1].width.saturating_sub(2) as f64;
                let max_rows = rows[1].height.saturating_sub(2) as f64; // two rows for the info lines
                let scale = f64::min(max_cols * cw as f64 / iw, max_rows * ch as f64 / ih);
                let cols = ((iw * scale / cw as f64).floor() as u16).max(1).min(rows[1].width);
                let rws = ((ih * scale / ch as f64).floor() as u16).max(1).min(rows[1].height.saturating_sub(2));
                let x = rows[1].x + (rows[1].width.saturating_sub(cols)) / 2;
                let y = rows[1].y + (rows[1].height.saturating_sub(rws + 2)) / 2;
                let r = Rect { x, y, width: cols, height: rws };
                f.render_stateful_widget(StatefulImage::default(), r, proto);
                shown = Some((r, *dims, *bytes));
            }
        }
        // info directly under the image: which frame, what it is, and what the source was
        let mark = if selected.contains(&cur) { "● selected" } else { "○ not selected" };
        let mut l1 = vec![
            Span::styled(format!("frame {}/{}", cur + 1, frames.len()), Style::default().fg(pal().fg).bold()),
            Span::styled(format!("  ·  t = {}  ·  ", fmt_timecode(ts)), dim),
            Span::styled(mark, Style::default().fg(if selected.contains(&cur) { pal().ok } else { pal().dim })),
        ];
        let mut l2: Vec<Span> = Vec::new();
        if let Some((r, (fw, fh), bytes)) = shown {
            // the image renderer fits without upscaling, so the size on screen is capped at 1:1
            let (cell_w, cell_h) = (r.width as f64 * cw as f64, r.height as f64 * ch as f64);
            let zoom = f64::min(f64::min(cell_w / fw.max(1) as f64, cell_h / fh.max(1) as f64), 1.0);
            let file = frames.get(cur).and_then(|_| app.video.as_ref().map(|v| v.frames[cur].1.clone()))
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string())).unwrap_or_default();
            l1.push(Span::styled(format!("   {fw}×{fh} px · {} · {:.2}:1", fmt_bytes(bytes), fw as f64 / fh.max(1) as f64), dim));
            l2.push(Span::styled(format!("shown {}×{} cells ≈ {}×{} px ({:.0}%{})", r.width, r.height, (fw as f64 * zoom) as u32, (fh as f64 * zoom) as u32, zoom * 100.0, if zoom >= 0.999 { ", 1:1" } else { "" }), dim));
            if !source.is_empty() { l2.push(Span::styled(format!("   ·   source {source}"), dim)); }
            if !file.is_empty() { l2.push(Span::styled(format!("   ·   {file}"), dim)); }
            // centre both lines under the image when it is narrower than the pane
            let pad = |line: &Vec<Span>| -> u16 {
                let w: usize = line.iter().map(|s| s.content.chars().count()).sum();
                let mid = r.x + r.width / 2;
                mid.saturating_sub((w / 2) as u16).max(rows[1].x)
            };
            let (p1, p2) = (pad(&l1), pad(&l2));
            let y = r.y + r.height;
            if y + 1 < rows[1].y + rows[1].height {
                f.render_widget(Paragraph::new(Line::from(l1)), Rect { x: p1, y, width: rows[1].width.saturating_sub(p1 - rows[1].x), height: 1 });
                f.render_widget(Paragraph::new(Line::from(l2)), Rect { x: p2, y: y + 1, width: rows[1].width.saturating_sub(p2 - rows[1].x), height: 1 });
            }
        } else {
            f.render_widget(Paragraph::new(Line::from(l1)), rows[2]);
        }
    }
    // strip: thumbnails around the cursor
    app.strip_rects.clear();
    let tw: u16 = 14; let th: u16 = 6; let gap = 1;
    let per = ((rows[3].width as usize) / (tw as usize + gap)).max(1);
    let first = cur.saturating_sub(per / 2).min(frames.len().saturating_sub(per.min(frames.len())));
    let mut x = rows[3].x;
    for i in first..(first + per).min(frames.len()) {
        let r = Rect { x, y: rows[3].y, width: tw, height: th };
        if let Some((proto, _)) = frames[i].1.as_ref().and_then(|k| app.images.get_mut(k)) {
            f.render_stateful_widget(StatefulImage::default(), Rect { x: r.x + 1, y: r.y, width: tw - 2, height: th - 1 }, proto);
        }
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

/// Decode the frames that are about to be drawn (current + the strip window) and drop the protocols
/// for frames far away, so scrubbing a long video stays flat in memory.
fn ensure_frame_images(app: &mut App, area: Rect) {
    let Some(v) = &app.video else { return };
    if v.frames.is_empty() { return; }
    let per = ((area.width as usize) / 15).max(1);
    let half = per / 2 + 2;
    let (lo, hi) = (v.cur.saturating_sub(half), (v.cur + half).min(v.frames.len() - 1));
    let want: Vec<usize> = (lo..=hi).collect();
    // the big preview gets a protocol of its own, rebuilt whenever the cursor moves
    let (cur, cur_path, have_preview) = (v.cur, v.frames[v.cur].1.clone(), v.preview.as_ref().map(|(i, ..)| *i) == Some(v.cur));
    if !have_preview {
        let old_key = app.video.as_ref().and_then(|v| v.preview.as_ref().map(|(_, k, ..)| k.clone()));
        if let Ok(bytes) = std::fs::read(&cur_path) {
            if let Ok(img) = image::load_from_memory(&bytes) {
                let dims = (img.width(), img.height());
                let key = app.register_image(img);
                if let Some(v) = &mut app.video { v.preview = Some((cur, key, dims, bytes.len() as u64)); }
                if let Some(k) = old_key { app.images.remove(&k); }
            }
        }
    }
    let Some(v) = &app.video else { return };
    let paths: Vec<(usize, PathBuf, bool)> = want.iter().map(|i| (*i, v.frames[*i].1.clone(), v.frames[*i].2.is_some())).collect();
    for (i, path, have) in paths {
        if have { continue; }
        let Ok(bytes) = std::fs::read(&path) else { continue };
        let Ok(img) = image::load_from_memory(&bytes) else { continue };
        let key = app.register_image(img);
        if let Some(v) = &mut app.video { v.frames[i].2 = Some(key); }
    }
    // evict what is no longer near the cursor
    let mut drop_keys: Vec<String> = Vec::new();
    if let Some(v) = &mut app.video {
        for (i, fr) in v.frames.iter_mut().enumerate() {
            if (lo..=hi).contains(&i) { continue; }
            if let Some(k) = fr.2.take() { drop_keys.push(k); }
        }
    }
    for k in drop_keys { app.images.remove(&k); }
}

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
    let dl_h = if app.dl.is_some() { 3 } else { 0 };     // title + bar + bytes/speed/ETA
    let rows = Layout::vertical([
        Constraint::Length(1), Constraint::Min(6),          // thinking
        Constraint::Length(todo_h),                         // tasks
        Constraint::Length(agents_h),                       // sub-agents
        Constraint::Length(dl_h),                           // model download
        Constraint::Length(1), Constraint::Length(6),       // tokens
        Constraint::Length(1), Constraint::Length(8),       // speed
        Constraint::Length(1), Constraint::Length(9),       // system
    ]).split(area);
    let (r_tokens_t, r_tokens, r_speed_t, r_speed, r_sys_t, r_sys) = (rows[5], rows[6], rows[7], rows[8], rows[9], rows[10]);

    // ── Model download: the one thing a first run is waiting on ──
    if let Some(p) = &app.dl {
        let w = (area.width as usize).saturating_sub(2).max(8);
        let filled = ((p.percent() / 100.0) * w as f64).round() as usize;
        let name = app.dl_build.map(|b| b.name()).unwrap_or("model");
        f.render_widget(Paragraph::new(vec![
            title(&format!("{name} · {:.1}%", p.percent())),
            Line::from(vec![
                Span::styled("█".repeat(filled.min(w)), Style::default().fg(pal().orange)),
                Span::styled("░".repeat(w.saturating_sub(filled)), dim),
            ]),
            Line::from(vec![Span::styled(truncate(&p.line(), w), dim)]),
        ]), rows[4]);
    }
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

/// Every command matching what has been typed (the renderer shows a window of them).
fn suggestions(input: &str) -> Vec<(&'static str, &'static str)> {
    if !input.starts_with('/') || input.contains(' ') { return vec![]; }
    COMMANDS.iter().filter(|(c, _)| c.starts_with(input)).take(60).cloned().collect()
}

/// Which slice of the suggestion list to draw: at most `rows`, kept around the selection.
fn suggestion_window(total: usize, selected: Option<usize>, rows: usize) -> (usize, usize) {
    if total <= rows { return (0, total); }
    let sel = selected.unwrap_or(0);
    let start = sel.saturating_sub(rows / 2).min(total - rows);
    (start, start + rows)
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
        Block::Startup(lines) => {
            // reveal: the border sweeps out, then each line types itself in; the ✻ spins until it settles
            const START: u32 = 2;        // frames before the first line starts
            const STAGGER: u32 = 2;      // frames between lines
            const CHARS_PER_FRAME: usize = 7;
            let step = app.banner_step;
            let inner = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0).min(w.saturating_sub(4));
            let bs = Style::default().fg(pal().orange);
            let border = |open: bool| -> Line<'static> {
                let full = inner + 2;
                let shown = ((step as usize * full) / 3).min(full);
                let bar = "─".repeat(shown);
                let (l, r) = if open { ("╭", "╮") } else { ("╰", "╯") };
                Line::from(Span::styled(if shown >= full { format!("{l}{bar}{r}") } else { format!("{l}{bar}") }, bs))
            };
            out.push(border(true));
            let mut animating = false;
            for (i, l) in lines.iter().enumerate() {
                let start = START + i as u32 * STAGGER;
                let text = truncate(l, inner);
                let chars: Vec<char> = text.chars().collect();
                let shown = if step <= start { 0 } else { ((step - start) as usize * CHARS_PER_FRAME).min(chars.len()) };
                let done = shown >= chars.len();
                if !done { animating = true; }
                let mut visible: String = chars[..shown].iter().collect();
                if !done && shown > 0 { visible.push('▌'); } // a cursor rides the line being typed
                if i == 0 && !done {
                    // the ✻ is multi-byte: swap the first *character*, not the first byte
                    if let Some(first) = visible.chars().next() { visible.replace_range(0..first.len_utf8(), SPINNER[(step as usize / 2) % SPINNER.len()]); }
                }
                let style = if i == 0 { Style::default().fg(pal().orange).bold() } else { Style::default() };
                out.push(Line::from(vec![Span::styled("│ ", bs), Span::styled(format!("{:<inner$}", visible), style), Span::styled(" │", bs)]));
            }
            let _ = animating;
            out.push(border(false));
            out.push(Line::raw(""));
        }
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

/// Break a paragraph into lines of at most `width` cells, on word boundaries where it can.
fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let width = width.max(8);
    let (mut out, mut cur) = (Vec::new(), String::new());
    for word in text.split_whitespace() {
        let wl = word.chars().count();
        if wl > width { // one very long token (a path, a JSON blob): hard-split it
            if !cur.is_empty() { out.push(std::mem::take(&mut cur)); }
            let chars: Vec<char> = word.chars().collect();
            for chunk in chars.chunks(width) { out.push(chunk.iter().collect()); }
            continue;
        }
        if cur.chars().count() + wl + 1 > width { out.push(std::mem::take(&mut cur)); }
        if !cur.is_empty() { cur.push(' '); }
        cur.push_str(word);
    }
    if !cur.is_empty() { out.push(cur); }
    if out.is_empty() { out.push(String::new()); }
    out
}

/// mm:ss.mmm for a timestamp in seconds (h:mm:ss.mmm past an hour).
fn fmt_timecode(t: f64) -> String {
    let total = t.max(0.0);
    let (h, m, sec) = ((total / 3600.0) as u64, ((total % 3600.0) / 60.0) as u64, total % 60.0);
    if h > 0 { format!("{h}:{m:02}:{sec:06.3}") } else { format!("{m}:{sec:06.3}") }
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

/// Finalise the reasoning this turn streamed with the complete text.
///
/// The streamed block is not necessarily the *last* block: a server that streams reasoning and then
/// content — `mlx_lm.server` does — leaves an Assistant block on the end, so finalising by tail alone
/// appended a second copy and every answer appeared twice. Identify it instead: the newest Reasoning block
/// that is still streaming, or whose partial text is a prefix of what just arrived.
fn finalize_reasoning(blocks: &mut Vec<Block>, text: String) {
    match blocks.iter_mut().rev().find_map(|b| match b {
        Block::Reasoning { text: t, streaming, ended, .. } if *streaming || (!t.is_empty() && text.starts_with(t.as_str())) => Some((t, streaming, ended)),
        _ => None,
    }) {
        Some((t, streaming, ended)) => { *t = text; *streaming = false; *ended = Some(Instant::now()); }
        None => if !text.trim().is_empty() {
            let now = Instant::now();
            blocks.push(Block::Reasoning { text, streaming: false, show: None, started: now, ended: Some(now) });
        },
    }
}

/// The same, for the answer itself.
fn finalize_assistant(blocks: &mut Vec<Block>, text: String) {
    match blocks.iter_mut().rev().find_map(|b| match b {
        Block::Assistant { text: t, streaming, .. } if *streaming || (!t.is_empty() && text.starts_with(t.as_str())) => Some((t, streaming)),
        _ => None,
    }) {
        Some((t, streaming)) => { *t = text; *streaming = false; }
        None => if !text.trim().is_empty() { blocks.push(Block::Assistant { text, streaming: false, folded: false }); },
    }
}


