//! `terminal`: persistent PTY sessions the model can drive — REPLs, debuggers (gdb/lldb/pdb),
//! interactive installers, `ssh`, `vim`, anything that needs a real terminal instead of a one-shot
//! `bash` call. Sessions outlive the tool call; output is buffered and handed over incrementally.

use super::{arg_str, Tool, ToolCtx, ToolOutput};
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

const MAX_BUFFER: usize = 400_000;

struct Session {
    id: usize,
    cmd: String,
    cwd: String,
    started: Instant,
    /// Everything the process has written so far (ANSI-stripped), capped.
    buf: Arc<Mutex<String>>,
    /// How much of `buf` the model has already been shown.
    cursor: usize,
    writer: Box<dyn Write + Send>,
    master: Box<dyn portable_pty::MasterPty + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    exited: Arc<Mutex<Option<u32>>>,
}

fn sessions() -> &'static Mutex<HashMap<usize, Session>> {
    static S: OnceLock<Mutex<HashMap<usize, Session>>> = OnceLock::new();
    S.get_or_init(Default::default)
}
fn next_id() -> usize {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(1);
    N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Drop ANSI/CSI escapes, carriage returns and other control noise so the model sees plain text.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => {
                match chars.peek() {
                    Some('[') => { chars.next(); while let Some(c) = chars.next() { if ('@'..='~').contains(&c) { break; } } }
                    Some(']') => { // OSC … BEL / ST
                        chars.next();
                        while let Some(c) = chars.next() { if c == '\u{7}' { break; } if c == '\u{1b}' { chars.next(); break; } }
                    }
                    Some(_) => { chars.next(); }
                    None => {}
                }
            }
            '\r' => { if chars.peek() != Some(&'\n') { out.push('\n'); } }
            '\u{7}' | '\u{8}' => {}
            c => out.push(c),
        }
    }
    out
}

pub struct Terminal;

#[async_trait]
impl Tool for Terminal {
    fn name(&self) -> &'static str { "terminal" }
    fn description(&self) -> &'static str {
        "Drive a real terminal (PTY) that stays alive between calls — for REPLs (python, node, psql), debuggers (lldb, gdb, pdb), interactive installers, ssh, or any program that prompts. Actions: open {cmd?, cwd?} starts a session and returns its id and first output; write {id, input} types a line and returns what appeared; read {id, wait_for?, timeout_secs?} returns new output (optionally waiting for a regex); resize {id, rows, cols}; close {id}; list. Use `bash` instead for one-shot commands."
    }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{
            "action":{"type":"string","enum":["open","write","read","resize","close","list"]},
            "id":{"type":"integer","description":"session id from open/list"},
            "cmd":{"type":"string","description":"open: program to run (default: your login shell)"},
            "cwd":{"type":"string","description":"open: working directory (default: the session workdir)"},
            "input":{"type":"string","description":"write: text to type"},
            "enter":{"type":"boolean","description":"write: append a newline (default true)"},
            "wait_for":{"type":"string","description":"read/write: regex to wait for (e.g. a prompt like '>>> $')"},
            "timeout_secs":{"type":"number","description":"read/write: how long to wait for output (default 10)"},
            "rows":{"type":"integer"},"cols":{"type":"integer"}
        },"required":["action"]})
    }

    async fn call(&self, args: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let ctx = ctx.effective();
        let action = arg_str(&args, "action")?.to_string();
        let id = args.get("id").and_then(|v| v.as_u64()).map(|n| n as usize);
        let timeout = Duration::from_secs_f64(args.get("timeout_secs").and_then(|v| v.as_f64()).unwrap_or(10.0).clamp(0.1, 600.0));
        let wait_for = args.get("wait_for").and_then(|v| v.as_str()).map(|s| s.to_string());
        match action.as_str() {
            "open" => {
                let cmd = args.get("cmd").and_then(|v| v.as_str()).map(|s| s.to_string())
                    .unwrap_or_else(|| std::env::var("SHELL").unwrap_or_else(|_| if cfg!(windows) { "powershell.exe".into() } else { "/bin/bash".into() }));
                let cwd = match args.get("cwd").and_then(|v| v.as_str()) { Some(d) => ctx.resolve(d)?, None => ctx.workdir.clone() };
                let rows = args.get("rows").and_then(|v| v.as_u64()).unwrap_or(40) as u16;
                let cols = args.get("cols").and_then(|v| v.as_u64()).unwrap_or(120) as u16;
                let id = open_session(&cmd, &cwd, rows, cols)?;
                let out = collect(id, timeout.min(Duration::from_secs(3)), wait_for.as_deref()).await?;
                Ok(format!("terminal #{id} started: {cmd} (cwd {})\n{}", cwd.display(), out).into())
            }
            "write" => {
                let id = id.context("write needs the session id")?;
                let input = arg_str(&args, "input")?.to_string();
                let enter = args.get("enter").and_then(|v| v.as_bool()).unwrap_or(true);
                {
                    let mut all = sessions().lock().unwrap();
                    let s = all.get_mut(&id).with_context(|| format!("no terminal #{id} (use list)"))?;
                    s.writer.write_all(input.as_bytes())?;
                    if enter { s.writer.write_all(b"\n")?; }
                    s.writer.flush()?;
                }
                let out = collect(id, timeout, wait_for.as_deref()).await?;
                Ok(if out.trim().is_empty() { format!("(no new output within {:.0}s)", timeout.as_secs_f64()) } else { out }.into())
            }
            "read" => {
                let id = id.context("read needs the session id")?;
                let out = collect(id, timeout, wait_for.as_deref()).await?;
                Ok(if out.trim().is_empty() { format!("(no new output within {:.0}s)", timeout.as_secs_f64()) } else { out }.into())
            }
            "resize" => {
                let id = id.context("resize needs the session id")?;
                let (rows, cols) = (args.get("rows").and_then(|v| v.as_u64()).unwrap_or(40) as u16, args.get("cols").and_then(|v| v.as_u64()).unwrap_or(120) as u16);
                let all = sessions().lock().unwrap();
                let s = all.get(&id).with_context(|| format!("no terminal #{id}"))?;
                s.master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })?;
                Ok(format!("terminal #{id} resized to {rows}×{cols}").into())
            }
            "close" => {
                let id = id.context("close needs the session id")?;
                let mut all = sessions().lock().unwrap();
                let mut s = all.remove(&id).with_context(|| format!("no terminal #{id}"))?;
                let _ = s.child.kill();
                Ok(format!("terminal #{id} closed ({} after {:.0}s)", s.cmd, s.started.elapsed().as_secs_f64()).into())
            }
            "list" => {
                let all = sessions().lock().unwrap();
                if all.is_empty() { return Ok("no terminal sessions (open one with terminal {action:\"open\"})".into()); }
                let mut ids: Vec<&usize> = all.keys().collect(); ids.sort();
                let mut out = format!("{} terminal session(s):\n", all.len());
                for i in ids {
                    let s = &all[i];
                    let status = match *s.exited.lock().unwrap() { Some(c) => format!("exited {c}"), None => "running".into() };
                    out.push_str(&format!("  #{:<3} {:<9} {:>5.0}s  {:<40} cwd {}\n", s.id, status, s.started.elapsed().as_secs_f64(), crate::llm::truncate_for_log(&s.cmd, 40), s.cwd));
                }
                Ok(out.into())
            }
            other => bail!("unknown action '{other}' (open|write|read|resize|close|list)"),
        }
    }
}

fn open_session(cmd: &str, cwd: &std::path::Path, rows: u16, cols: u16) -> Result<usize> {
    let sys = NativePtySystem::default();
    let pair = sys.openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 }).context("opening a pty")?;
    let mut builder = if cmd.contains(' ') || cmd.contains('|') || cmd.contains('&') {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
        let mut b = CommandBuilder::new(shell); b.arg("-lc"); b.arg(cmd); b
    } else { CommandBuilder::new(cmd) };
    builder.cwd(cwd);
    builder.env("TERM", "xterm-256color");
    builder.env("PAGER", "cat");
    builder.env("GIT_PAGER", "cat");
    let child = pair.slave.spawn_command(builder).context("starting the program in the pty")?;
    drop(pair.slave);
    let writer = pair.master.take_writer()?;
    let mut reader = pair.master.try_clone_reader()?;
    let buf = Arc::new(Mutex::new(String::new()));
    let exited = Arc::new(Mutex::new(None));
    let (b2, e2) = (buf.clone(), exited.clone());
    std::thread::spawn(move || {
        let mut chunk = [0u8; 8192];
        loop {
            match reader.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let text = strip_ansi(&String::from_utf8_lossy(&chunk[..n]));
                    let mut g = b2.lock().unwrap();
                    g.push_str(&text);
                    if g.len() > MAX_BUFFER { let cut = g.len() - MAX_BUFFER; *g = g[cut..].to_string(); }
                }
            }
        }
        *e2.lock().unwrap() = Some(0);
    });
    let id = next_id();
    sessions().lock().unwrap().insert(id, Session { id, cmd: cmd.to_string(), cwd: cwd.display().to_string(), started: Instant::now(), buf, cursor: 0, writer, master: pair.master, child, exited });
    Ok(id)
}

/// New output for a session, waiting up to `timeout` (or until `wait_for` matches). Advances the cursor.
async fn collect(id: usize, timeout: Duration, wait_for: Option<&str>) -> Result<String> {
    let re = wait_for.map(|p| regex::Regex::new(p)).transpose().context("wait_for is not a valid regex")?;
    let deadline = Instant::now() + timeout;
    let quiet = Duration::from_millis(250);
    let mut last_len = 0usize;
    let mut last_change = Instant::now();
    loop {
        let (text, done) = {
            let all = sessions().lock().unwrap();
            let s = all.get(&id).with_context(|| format!("no terminal #{id}"))?;
            let buf = s.buf.lock().unwrap();
            let text = buf[s.cursor.min(buf.len())..].to_string();
            let done = s.exited.lock().unwrap().is_some();
            (text, done)
        };
        let matched = re.as_ref().map(|r| r.is_match(&text)).unwrap_or(false);
        if text.len() != last_len { last_len = text.len(); last_change = Instant::now(); }
        let settled = re.is_none() && !text.is_empty() && last_change.elapsed() >= quiet;
        if matched || settled || done || Instant::now() >= deadline {
            let mut all = sessions().lock().unwrap();
            if let Some(s) = all.get_mut(&id) { s.cursor += text.len(); }
            let mut out = text;
            if done { out.push_str("\n[process exited — the session is closed]"); all.remove(&id); }
            else if re.is_some() && !matched && Instant::now() >= deadline { out.push_str(&format!("\n[timed out after {:.0}s waiting for /{}/]", timeout.as_secs_f64(), wait_for.unwrap_or(""))); }
            return Ok(out);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ansi_stripping() {
        assert_eq!(strip_ansi("\u{1b}[32mgreen\u{1b}[0m text"), "green text");
        assert_eq!(strip_ansi("a\rb"), "a\nb");
        assert_eq!(strip_ansi("\u{1b}]0;title\u{7}prompt$ "), "prompt$ ");
    }

    #[tokio::test]
    async fn repl_session_round_trip() {
        let d = std::env::temp_dir().join(format!("harness-pty-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let ctx = ToolCtx::basic(d.canonicalize().unwrap());
        let t = Terminal;
        let out = t.call(json!({"action":"open","cmd":"/bin/sh","timeout_secs":3}), &ctx).await.unwrap().text;
        let id: usize = out.split('#').nth(1).unwrap().split(' ').next().unwrap().parse().unwrap();
        let out = t.call(json!({"action":"write","id":id,"input":"echo hello-pty","timeout_secs":5}), &ctx).await.unwrap().text;
        assert!(out.contains("hello-pty"), "{out}");
        // state persists between calls
        let _ = t.call(json!({"action":"write","id":id,"input":"X=42","timeout_secs":3}), &ctx).await.unwrap();
        let out = t.call(json!({"action":"write","id":id,"input":"echo value=$X","timeout_secs":5}), &ctx).await.unwrap().text;
        assert!(out.contains("value=42"), "{out}");
        let list = t.call(json!({"action":"list"}), &ctx).await.unwrap().text;
        assert!(list.contains(&format!("#{id}")), "{list}");
        let closed = t.call(json!({"action":"close","id":id}), &ctx).await.unwrap().text;
        assert!(closed.contains("closed"), "{closed}");
        assert!(t.call(json!({"action":"read","id":id}), &ctx).await.is_err(), "a closed session is gone");
    }
}
