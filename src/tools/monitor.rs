//! monitor: run a command in the background and stream its output lines (optionally regex-filtered) into
//! the conversation as inbox events, so the model can react to logs / polled status. Reuses `procs`
//! (log file, kill on exit); a tokio task tails the log, coalesces lines (≤1 inbox item per second) and
//! reports exit / timeout / truncation.

use super::{Tool, ToolCtx, ToolOutput};
use crate::inbox::Inbox;
use anyhow::{bail, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

pub struct Monitor;

struct Mon { proc_id: u32, pid: u32, cmd: String, filter: Option<String>, lines: Arc<AtomicUsize>, done: Arc<AtomicBool>, started: Instant, task: tokio::task::JoinHandle<()> }

static MONS: OnceLock<Mutex<HashMap<u32, Mon>>> = OnceLock::new();
static NEXT: AtomicUsize = AtomicUsize::new(1);
fn table() -> &'static Mutex<HashMap<u32, Mon>> { MONS.get_or_init(|| Mutex::new(HashMap::new())) }

fn short(cmd: &str) -> String { crate::llm::truncate_for_log(cmd.lines().next().unwrap_or(""), 40) }

fn proc_status(proc_id: u32) -> Option<String> { crate::procs::list().into_iter().find(|p| p.0 == proc_id).map(|p| p.3) }

/// Start monitoring `cmd`; returns (monitor id, pid, log path).
pub async fn start(inbox: Arc<Inbox>, cmd: &str, cwd: &std::path::Path, filter: Option<regex::Regex>, timeout: Duration, max_lines: usize) -> Result<(u32, u32, std::path::PathBuf)> {
    let (proc_id, log) = crate::procs::start(cmd, cwd).await?;
    let pid = crate::procs::list().into_iter().find(|p| p.0 == proc_id).map(|p| p.1).unwrap_or(0);
    let id = NEXT.fetch_add(1, Ordering::Relaxed) as u32;
    let source = format!("monitor #{id} ({})", short(cmd));
    let lines = Arc::new(AtomicUsize::new(0));
    let done = Arc::new(AtomicBool::new(false));
    let (lines2, done2, log2) = (lines.clone(), done.clone(), log.clone());
    let filter_str = filter.as_ref().map(|r| r.as_str().to_string());
    let started = Instant::now();
    let task = tokio::spawn(async move {
        let mut file = std::fs::File::open(&log2).ok();
        let mut offset = 0u64;
        let mut partial = String::new();
        let mut pending: Vec<String> = Vec::new();
        let mut last_flush = Instant::now();
        let mut truncated = false;
        loop {
            // read new bytes
            if file.is_none() { file = std::fs::File::open(&log2).ok(); }
            let mut chunk = String::new();
            if let Some(f) = file.as_mut() {
                let mut buf = Vec::new();
                if f.seek(SeekFrom::Start(offset)).is_ok() && f.read_to_end(&mut buf).is_ok() && !buf.is_empty() { offset += buf.len() as u64; chunk = String::from_utf8_lossy(&buf).into_owned(); }
            }
            let status = proc_status(proc_id);
            let exited = status.as_deref().map(|s| s != "running").unwrap_or(true);
            let timed_out = !timeout.is_zero() && started.elapsed() > timeout && !exited;
            if !chunk.is_empty() || (exited && !partial.is_empty()) {
                partial.push_str(&chunk);
                let mut rest = String::new();
                let mut it = partial.split_inclusive('\n').peekable();
                while let Some(l) = it.next() {
                    if !l.ends_with('\n') && !exited { rest = l.to_string(); break; }
                    let line = l.trim_end_matches(['\n', '\r']);
                    if filter.as_ref().map(|r| r.is_match(line)).unwrap_or(true) && !truncated {
                        let n = lines2.fetch_add(1, Ordering::Relaxed) + 1;
                        if n > max_lines { truncated = true; pending.push(format!("…monitor #{id} reached max_lines ({max_lines}); further output not streamed — use `process tail {{id:{proc_id}}}` for the full log")); }
                        else { pending.push(line.to_string()); }
                    }
                }
                partial = rest;
            }
            if !pending.is_empty() && (last_flush.elapsed() >= Duration::from_secs(1) || exited || timed_out) {
                inbox.push(&source, pending.join("\n"));
                pending.clear();
                last_flush = Instant::now();
            }
            if timed_out {
                let _ = crate::procs::kill(proc_id).await;
                inbox.push(&source, format!("monitor #{id} timed out after {}s — process killed", timeout.as_secs()));
                break;
            }
            if exited {
                inbox.push(&source, format!("monitor #{id} {} (log: {})", status.unwrap_or_else(|| "exited".into()), log2.display()));
                break;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        done2.store(true, Ordering::Relaxed);
    });
    table().lock().unwrap().insert(id, Mon { proc_id, pid, cmd: cmd.to_string(), filter: filter_str, lines, done, started, task });
    Ok((id, pid, log))
}

pub async fn stop(id: u32) -> Result<String> {
    let (proc_id, task) = { let mut t = table().lock().unwrap(); let Some(m) = t.get_mut(&id) else { bail!("no monitor #{id}") }; m.done.store(true, Ordering::Relaxed); (m.proc_id, m.task.abort_handle()) };
    task.abort();
    let msg = crate::procs::kill(proc_id).await.unwrap_or_else(|e| e.to_string());
    Ok(format!("monitor #{id} stopped; {msg}"))
}

pub fn list() -> String {
    let t = table().lock().unwrap();
    if t.is_empty() { return "no monitors".into(); }
    let mut v: Vec<_> = t.iter().collect();
    v.sort_by_key(|(id, _)| **id);
    v.into_iter().map(|(id, m)| format!("monitor #{id} pid {} [{}] {:.0}s {} lines{}  {}", m.pid, if m.done.load(Ordering::Relaxed) { "finished" } else { "running" }, m.started.elapsed().as_secs_f64(), m.lines.load(Ordering::Relaxed), m.filter.as_ref().map(|f| format!(" filter=/{f}/")).unwrap_or_default(), crate::llm::truncate_for_log(&m.cmd, 80))).collect::<Vec<_>>().join("\n")
}

#[async_trait]
impl Tool for Monitor {
    fn name(&self) -> &'static str { "monitor" }
    fn description(&self) -> &'static str { "Run a shell command in the background and stream its output lines back to you as they appear (delivered as inbox events before your next turn, even while idle) — for watching logs, builds, test watchers or polling loops. Actions: start {cmd, filter?, cwd?, timeout_secs?, max_lines?} (filter = regex, only matching lines are streamed; timeout_secs kills it, 0 = none; max_lines default 200), stop {id}, list. Output lines are coalesced (~1 event/s); an exit note is sent when the command ends." }
    fn parameters(&self) -> Value { json!({"type":"object","properties":{"action":{"type":"string","enum":["start","stop","list"]},"cmd":{"type":"string","description":"start: shell command to run"},"filter":{"type":"string","description":"start: regex; only matching lines are streamed"},"cwd":{"type":"string","description":"start: working directory (default workdir)"},"timeout_secs":{"type":"integer","description":"start: kill after N seconds (default 0 = never)"},"max_lines":{"type":"integer","description":"start: stop streaming after N lines (default 200)"},"id":{"type":"integer","description":"stop: monitor id"}},"required":["action"]}) }
    fn parallel_safe(&self) -> bool { true }
    async fn call(&self, args: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("list");
        match action {
            "start" => {
                let cmd = args.get("cmd").and_then(|v| v.as_str()).map(str::trim).unwrap_or("");
                if cmd.is_empty() { bail!("cmd required"); }
                let filter = match args.get("filter").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) { Some(f) => Some(regex::Regex::new(f).map_err(|e| anyhow::anyhow!("bad filter regex: {e}"))?), None => None };
                let cwd = match args.get("cwd").and_then(|v| v.as_str()) { Some(c) => { let p = ctx.workdir.join(c); if !p.is_dir() { bail!("cwd {} is not a directory", p.display()); } p } None => ctx.workdir.clone() };
                let timeout = Duration::from_secs(args.get("timeout_secs").and_then(|v| v.as_u64()).unwrap_or(0));
                let max_lines = args.get("max_lines").and_then(|v| v.as_u64()).unwrap_or(200).max(1) as usize;
                let (id, pid, log) = start(ctx.inbox.clone(), cmd, &cwd, filter, timeout, max_lines).await?;
                Ok(format!("monitor #{id} started (pid {pid}, log {}); matching output lines will arrive as inbox events. stop with monitor {{action:stop,id:{id}}}", log.display()).into())
            }
            "stop" => { let Some(id) = args.get("id").and_then(|v| v.as_u64()) else { bail!("id required") }; Ok(stop(id as u32).await?.into()) }
            "list" => Ok(list().into()),
            _ => bail!("unknown action {action}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn streams_filtered_lines_and_exit() {
        let inbox = Arc::new(Inbox::new());
        let cwd = std::env::temp_dir();
        let cmd = "printf 'a\\nb\\nERR c\\n'; sleep 0.3; echo 'ERR d'";
        let (id, _pid, _log) = start(inbox.clone(), cmd, &cwd, Some(regex::Regex::new("ERR").unwrap()), Duration::from_secs(10), 200).await.unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut all = String::new();
        while Instant::now() < deadline {
            for it in inbox.drain() { assert!(it.source.starts_with(&format!("monitor #{id} (")), "{}", it.source); all.push_str(&it.text); all.push('\n'); }
            if all.contains("exited") { break; }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(all.contains("ERR c"), "{all}");
        assert!(all.contains("ERR d"), "{all}");
        assert!(!all.contains("\na\n") && !all.contains("\nb\n"), "unfiltered lines leaked: {all}");
        assert!(all.contains(&format!("monitor #{id} exited 0")), "{all}");
        assert!(list().contains(&format!("monitor #{id}")));
    }

    #[tokio::test]
    async fn timeout_kills() {
        let inbox = Arc::new(Inbox::new());
        let (id, ..) = start(inbox.clone(), "sleep 30", &std::env::temp_dir(), None, Duration::from_secs(1), 200).await.unwrap();
        let deadline = Instant::now() + Duration::from_secs(6);
        let mut all = String::new();
        while Instant::now() < deadline { for it in inbox.drain() { all.push_str(&it.text); } if all.contains("timed out") { break; } tokio::time::sleep(Duration::from_millis(100)).await; }
        assert!(all.contains(&format!("monitor #{id} timed out")), "{all}");
    }
}
