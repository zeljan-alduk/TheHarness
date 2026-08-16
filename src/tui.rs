//! Interactive terminal UI — the Claude-Code-style front end for a local model.
//! Everything here is presentation; the agent loop lives in the `harness` library.

use anyhow::Result;
use crossterm::event::{Event as CEvent, EventStream, KeyCode, KeyEventKind, KeyModifiers};
use futures_util::StreamExt;
use harness::agent::{system_prompt, Agent};
use harness::config::Config;
use harness::events::{Event, Sink};
use harness::llm::{Client, Message};
use harness::tools::{Registry, ToolCtx};
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;
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
const SPINNER: [&str; 10] = ["✻", "✼", "✽", "✾", "✿", "❀", "✿", "✾", "✽", "✼"];
const WORDS: [&str; 12] = ["Thinking", "Pondering", "Working", "Reasoning", "Cooking", "Tinkering", "Brewing", "Mulling", "Crunching", "Percolating", "Noodling", "Computing"];

enum Msg { Ev(Event), Done(Result<String, String>) }

struct TuiSink(mpsc::UnboundedSender<Msg>);
impl Sink for TuiSink { fn emit(&self, e: &Event) { let _ = self.0.send(Msg::Ev(e.clone())); } }

enum Block {
    Banner(Vec<String>),
    User(String),
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
}

pub async fn run(cfg: Config) -> Result<()> {
    let workdir = std::env::current_dir()?;
    let (tx, mut rx) = mpsc::unbounded_channel::<Msg>();
    let mut app = App {
        model: cfg.llm.model.clone(), net: cfg.net.enabled, cfg, workdir,
        blocks: vec![], input: String::new(), cursor: 0, history: vec![], hist_idx: None, hist_draft: String::new(),
        scroll_up: 0, running: None, run_started: Instant::now(), queued: vec![], expand_tools: false, show_thinking: false,
        session: Arc::new(tokio::sync::Mutex::new(Vec::new())), tx: tx.clone(),
        total_prompt: 0, total_completion: 0, last_prompt_tokens: 0, turn_tokens: 0, last_ctrl_c: None, status_msg: None,
        quit: false, tick: 0, word: 0, models: vec![],
    };
    app.banner();
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

    fn submit(&mut self) {
        let text = self.input.trim().to_string();
        if text.is_empty() { return; }
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
                lines.push("Keys: enter send · alt+enter/ctrl+j newline · esc interrupt · ctrl+c clear/exit · ctrl+o expand tools · ctrl+t thinking · pgup/pgdn scroll · ↑/↓ history".into());
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
            "/cost" | "/stats" => self.blocks.push(Block::System(format!("session tokens: {} prompt + {} completion · last context {} · turns in history {}", self.total_prompt, self.total_completion, self.last_prompt_tokens, self.history.len()))),
            "/config" => self.blocks.push(Block::Banner(vec![format!("server  {}", self.cfg.llm.base_url), format!("model   {}", self.model), format!("ctx budget {} tokens · max_turns {} · tool timeout {}s", self.cfg.llm.context_budget_tokens, self.cfg.agent.max_turns, self.cfg.agent.tool_timeout_secs), format!("net {} · segments {}", self.net, self.cfg.net.download_segments)])),
            "/exit" | "/quit" | "/q" => self.quit = true,
            _ => self.blocks.push(Block::Error(format!("unknown command {cmd} — /help"))),
        }
    }

    fn start_run(&mut self, text: String) {
        self.blocks.push(Block::User(text.clone()));
        self.scroll_up = 0;
        self.turn_tokens = 0;
        let tx = self.tx.clone(); let session = self.session.clone();
        let mut cfg = self.cfg.clone(); cfg.llm.model = self.model.clone(); cfg.net.enabled = self.net;
        let workdir = self.workdir.clone();
        self.run_started = Instant::now();
        let handle = tokio::spawn(async move {
            let res: Result<String, String> = async {
                let client = Client::new(&cfg.llm).map_err(|e| e.to_string())?;
                let ctx = ToolCtx { workdir: workdir.clone(), timeout: Duration::from_secs(cfg.agent.tool_timeout_secs), max_output: cfg.agent.max_tool_output_chars, net: cfg.net.clone() };
                let registry = Registry::defaults(cfg.net.enabled);
                let sink = TuiSink(tx.clone());
                let agent = Agent { client: &client, registry: &registry, ctx: &ctx, max_turns: cfg.agent.max_turns, context_budget: cfg.llm.context_budget_tokens, sink: &sink, stream: true };
                let system = system_prompt(&workdir.display().to_string(), &registry.names(), Some("You are in an interactive session: the user can see everything and will reply; keep final answers concise."));
                let mut msgs = session.lock().await;
                agent.run_turn(&mut msgs, &system, &text).await.map(|(t, _)| t).map_err(|e| format!("{e:#}"))
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
            Event::ReasoningDelta { text } => {
                if let Some(Block::Reasoning { text: t, streaming: true }) = self.blocks.last_mut() { t.push_str(&text); }
                else { self.blocks.push(Block::Reasoning { text, streaming: true }); }
            }
            Event::Reasoning { text } => {
                if let Some(Block::Reasoning { text: t, streaming }) = self.blocks.last_mut() { *t = text; *streaming = false; }
                else if !text.trim().is_empty() { self.blocks.push(Block::Reasoning { text, streaming: false }); }
            }
            Event::AssistantDelta { text } => {
                if let Some(Block::Assistant { text: t, streaming: true }) = self.blocks.last_mut() { t.push_str(&text); }
                else { self.finish_streaming(); self.blocks.push(Block::Assistant { text, streaming: true }); }
            }
            Event::Assistant { text } => {
                if let Some(Block::Assistant { text: t, streaming }) = self.blocks.last_mut() { *t = text; *streaming = false; }
                else if !text.trim().is_empty() { self.blocks.push(Block::Assistant { text, streaming: false }); }
            }
            Event::ToolCall { id, name, args } => { self.finish_streaming(); self.blocks.push(Block::Tool { id, name, args, result: None, secs: 0.0, images: 0, interrupted: false }); }
            Event::ToolResult { id, result, secs, images, .. } => {
                if let Some(Block::Tool { result: r, secs: s, images: im, .. }) = self.blocks.iter_mut().rev().find(|b| matches!(b, Block::Tool { id: i, .. } if *i == id)) { *r = Some(result); *s = secs; *im = images.len(); }
            }
            Event::Compacted { count, prompt_tokens } => self.blocks.push(Block::System(format!("compacted {count} old tool results (context was {prompt_tokens} tokens)"))),
            Event::RunFinished { stop_reason, turns, tool_calls, prompt_tokens, completion_tokens, wall_secs } => {
                self.finish_streaming();
                self.total_prompt += prompt_tokens; self.total_completion += completion_tokens; self.turn_tokens = completion_tokens;
                self.last_prompt_tokens = prompt_tokens / turns.max(1) as u64;
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
    ("/cost", "token usage for this session"),
    ("/config", "effective configuration"),
    ("/exit", "quit"),
];

// ───────────────────────── rendering ─────────────────────────
fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let width = area.width as usize;
    // input geometry
    let input_lines = wrap_input(&app.input, width.saturating_sub(2).max(1));
    let sugg = suggestions(&app.input);
    let input_h = (input_lines.len().clamp(1, 8) + sugg.len()) as u16;
    let chunks = Layout::vertical([Constraint::Min(1), Constraint::Length(1), Constraint::Length(input_h), Constraint::Length(1)]).split(area);
    let (tr_area, sep_area, in_area, st_area) = (chunks[0], chunks[1], chunks[2], chunks[3]);

    // transcript
    let mut lines: Vec<Line> = Vec::new();
    for b in &app.blocks { render_block(b, app, width, &mut lines); }
    let total = lines.len();
    let h = tr_area.height as usize;
    let max_up = total.saturating_sub(h);
    if app.scroll_up > max_up { app.scroll_up = max_up; }
    let start = max_up - app.scroll_up;
    let visible: Vec<Line> = lines.into_iter().skip(start).take(h).collect();
    f.render_widget(Paragraph::new(visible), tr_area);
    if app.scroll_up > 0 {
        let tag = format!(" ↓ {} more lines ", app.scroll_up);
        let r = Rect { x: area.width.saturating_sub(tag.len() as u16 + 1), y: tr_area.bottom().saturating_sub(1), width: tag.len() as u16, height: 1 };
        f.render_widget(Paragraph::new(Span::styled(tag, Style::default().fg(Color::Black).bg(ORANGE))), r);
    }

    // separator
    f.render_widget(Paragraph::new(Span::styled("─".repeat(width), Style::default().fg(DIM))), sep_area);

    // input (+ suggestions)
    let mut in_lines: Vec<Line> = Vec::new();
    for (i, l) in input_lines.iter().enumerate().take(8) {
        let prompt = if i == 0 { Span::styled("› ", Style::default().fg(ORANGE).bold()) } else { Span::raw("  ") };
        in_lines.push(Line::from(vec![prompt, Span::raw(l.clone())]));
    }
    if app.input.is_empty() {
        in_lines[0] = Line::from(vec![Span::styled("› ", Style::default().fg(ORANGE).bold()), Span::styled(if app.running.is_some() { "type to queue the next message…" } else { "Ask the agent to do something… (/help)" }, Style::default().fg(DIM))]);
    }
    for (c, d) in &sugg { in_lines.push(Line::from(vec![Span::raw("  "), Span::styled(format!("{c:<12}"), Style::default().fg(BLUE)), Span::styled(d.to_string(), Style::default().fg(DIM))])); }
    f.render_widget(Paragraph::new(in_lines), in_area);
    // cursor
    let (crow, ccol) = cursor_pos(&app.input, app.cursor, width.saturating_sub(2).max(1));
    if crow < 8 { f.set_cursor_position((in_area.x + 2 + ccol as u16, in_area.y + crow as u16)); }

    // status
    let left: Vec<Span> = if app.running.is_some() {
        let sp = SPINNER[(app.tick as usize / 2) % SPINNER.len()];
        let el = app.run_started.elapsed().as_secs();
        vec![Span::styled(format!("{sp} {}… ", WORDS[app.word]), Style::default().fg(ORANGE)), Span::styled(format!("({el}s · esc to interrupt{})", if app.queued.is_empty() { String::new() } else { format!(" · {} queued", app.queued.len()) }), Style::default().fg(DIM))]
    } else if let Some((m, t)) = &app.status_msg { if t.elapsed() < Duration::from_secs(4) { vec![Span::styled(m.clone(), Style::default().fg(ORANGE))] } else { vec![Span::styled("? for shortcuts", Style::default().fg(DIM))] } }
    else { vec![Span::styled("/help for commands", Style::default().fg(DIM))] };
    let right = format!("{} · {} · ctx {} · {}{} ", app.model, short_path(&app.workdir), fmt_k(app.last_prompt_tokens), fmt_k(app.total_prompt + app.total_completion), if app.net { "" } else { " · offline" });
    let lw: usize = left.iter().map(|s| s.content.chars().count()).sum();
    let pad = width.saturating_sub(lw + right.chars().count());
    let mut st = left; st.push(Span::raw(" ".repeat(pad))); st.push(Span::styled(right, Style::default().fg(DIM)));
    f.render_widget(Paragraph::new(Line::from(st)), st_area);
}

fn suggestions(input: &str) -> Vec<(&'static str, &'static str)> {
    if !input.starts_with('/') || input.contains(' ') { return vec![]; }
    COMMANDS.iter().filter(|(c, _)| c.starts_with(input)).take(6).cloned().collect()
}

fn render_block(b: &Block, app: &App, width: usize, out: &mut Vec<Line<'static>>) {
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
        Block::User(t) => {
            out.push(Line::raw(""));
            for (i, l) in t.lines().enumerate() { push_wrapped(out, vec![Span::styled(if i == 0 { "› " } else { "  " }, Style::default().fg(DIM)), Span::styled(l.to_string(), Style::default().bold())], w, 2); }
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
        Block::Tool { name, args, result, secs, images, interrupted, .. } => {
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
                    if *images > 0 { out.push(Line::from(vec![Span::styled("     ", Style::default()), Span::styled(format!("[{} image{} shown to the model]", images, if *images == 1 { "" } else { "s" }), Style::default().fg(BLUE))])); }
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
