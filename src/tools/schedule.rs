//! schedule: session-scoped one-shot / recurring prompts delivered to the model later ("in 10 min re-run the
//! tests and report"). Entries live in a global registry (a tokio task per entry holds a clone of the inbox);
//! when due, the prompt is pushed as an inbox event (`schedule #N`), which the agent loop drains before its
//! next model call and which wakes an idle session up. Not persisted: schedules die with the process
//! (session-scoped; not restored on resume).

use super::{Tool, ToolCtx, ToolOutput};
use crate::inbox::Inbox;
use anyhow::{bail, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

pub struct Schedule;

const MAX_ENTRIES: usize = 20;
const MAX_FIRINGS: usize = 200;
const MIN_EVERY_SECS: u64 = 10;

struct Entry { prompt: String, next: Arc<Mutex<Instant>>, every: Option<Duration>, fired: Arc<AtomicUsize>, task: tokio::task::JoinHandle<()> }

static ENTRIES: OnceLock<Mutex<BTreeMap<u32, Entry>>> = OnceLock::new();
static NEXT: AtomicUsize = AtomicUsize::new(1);
fn table() -> &'static Mutex<BTreeMap<u32, Entry>> { ENTRIES.get_or_init(|| Mutex::new(BTreeMap::new())) }

/// Local wall-clock seconds since midnight (unix: libc localtime; elsewhere UTC).
fn local_secs_of_day() -> u64 {
    #[cfg(unix)]
    unsafe {
        let t: libc::time_t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        if !libc::localtime_r(&t, &mut tm).is_null() { return (tm.tm_hour as u64) * 3600 + (tm.tm_min as u64) * 60 + tm.tm_sec as u64; }
    }
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() % 86400).unwrap_or(0)
}

/// Parse `at`: seconds from now ("90"), or a local `HH:MM` (today, or tomorrow if already past). Returns delay.
fn parse_at(at: &str) -> Result<Duration> {
    let at = at.trim();
    if let Ok(n) = at.parse::<u64>() { return Ok(Duration::from_secs(n)); }
    if let Some((h, m)) = at.split_once(':') {
        let (h, m) = (h.trim().parse::<u64>().ok(), m.trim().parse::<u64>().ok());
        if let (Some(h), Some(m)) = (h, m) {
            if h < 24 && m < 60 {
                let target = h * 3600 + m * 60;
                let now = local_secs_of_day();
                let delay = if target > now { target - now } else { target + 86400 - now };
                return Ok(Duration::from_secs(delay));
            }
        }
    }
    bail!("cannot parse at={at:?}: use delay_secs, seconds from now, or local HH:MM")
}

/// Register a schedule; returns (id, initial delay). (The tool enforces the minimum interval; this API does not.)
pub fn add(inbox: Arc<Inbox>, prompt: &str, delay: Duration, every: Option<Duration>) -> Result<(u32, Duration)> {
    if prompt.trim().is_empty() { bail!("prompt required"); }
    if table().lock().unwrap().len() >= MAX_ENTRIES { bail!("too many schedules ({MAX_ENTRIES}); remove or clear some first"); }
    let id = NEXT.fetch_add(1, Ordering::Relaxed) as u32;
    let next = Arc::new(Mutex::new(Instant::now() + delay));
    let fired = Arc::new(AtomicUsize::new(0));
    let (next2, fired2, prompt2) = (next.clone(), fired.clone(), prompt.to_string());
    let source = format!("schedule #{id}");
    let task = tokio::spawn(async move {
        loop {
            let due = *next2.lock().unwrap();
            tokio::time::sleep_until(tokio::time::Instant::from_std(due)).await;
            let n = fired2.fetch_add(1, Ordering::Relaxed) + 1;
            let mut text = prompt2.clone();
            match every {
                Some(e) if n < MAX_FIRINGS => {
                    text.push_str(&format!("\n(recurring every {}s, firing {n}; `schedule {{action:remove,id:{id}}}` to stop)", e.as_secs()));
                    inbox.push(&source, text);
                    *next2.lock().unwrap() = Instant::now() + e;
                }
                Some(_) => {
                    text.push_str(&format!("\n(recurring schedule #{id} reached the {MAX_FIRINGS}-firing limit and was removed)"));
                    inbox.push(&source, text);
                    table().lock().unwrap().remove(&id);
                    break;
                }
                None => {
                    inbox.push(&source, text);
                    table().lock().unwrap().remove(&id);
                    break;
                }
            }
        }
    });
    table().lock().unwrap().insert(id, Entry { prompt: prompt.to_string(), next, every, fired, task });
    Ok((id, delay))
}

pub fn remove(id: u32) -> Result<String> {
    let Some(e) = table().lock().unwrap().remove(&id) else { bail!("no schedule #{id}") };
    e.task.abort();
    Ok(format!("schedule #{id} removed ({} firings)", e.fired.load(Ordering::Relaxed)))
}

pub fn clear() -> String {
    let all: Vec<_> = std::mem::take(&mut *table().lock().unwrap()).into_iter().collect();
    let n = all.len();
    for (_, e) in all { e.task.abort(); }
    format!("cleared {n} schedule(s)")
}

pub fn list() -> String {
    let t = table().lock().unwrap();
    if t.is_empty() { return "no schedules".into(); }
    let now = Instant::now();
    t.iter().map(|(id, e)| {
        let next = e.next.lock().unwrap().saturating_duration_since(now).as_secs();
        let kind = match e.every { Some(ev) => format!("every {}s", ev.as_secs()), None => "once".into() };
        format!("schedule #{id}: next in {next}s ({kind}), fired {}  — {}", e.fired.load(Ordering::Relaxed), crate::llm::truncate_for_log(&e.prompt, 100))
    }).collect::<Vec<_>>().join("\n")
}

fn fmt_delay(d: Duration) -> String { let s = d.as_secs(); if s >= 3600 { format!("{}h{:02}m", s / 3600, (s % 3600) / 60) } else if s >= 60 { format!("{}m{:02}s", s / 60, s % 60) } else { format!("{s}s") } }

#[async_trait]
impl Tool for Schedule {
    fn name(&self) -> &'static str { "schedule" }
    fn description(&self) -> &'static str { "Schedule a prompt to be delivered to yourself later (as an inbox event before your next turn; wakes you if idle) — one-shot or recurring, for this session only. Typical use: \"check the build again in 5 minutes\", \"poll every 60s until the server responds — remember to remove it when done\". Actions: add {prompt, delay_secs? | at? (local HH:MM or seconds), every_secs? (recurring, min 10s, max 200 firings)}, list, remove {id}, clear. Max 20 schedules." }
    fn parameters(&self) -> Value { json!({"type":"object","properties":{"action":{"type":"string","enum":["add","list","remove","clear"]},"prompt":{"type":"string","description":"add: the instruction to deliver to yourself when due"},"delay_secs":{"type":"integer","description":"add: fire after N seconds (default 60 unless `at` given)"},"at":{"type":"string","description":"add: local time HH:MM (today, or tomorrow if past) or seconds from now"},"every_secs":{"type":"integer","description":"add: repeat every N seconds after the first firing (min 10)"},"id":{"type":"integer","description":"remove: schedule id"}},"required":["action"]}) }
    fn parallel_safe(&self) -> bool { true }
    async fn call(&self, args: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("list");
        match action {
            "add" => {
                let prompt = args.get("prompt").and_then(|v| v.as_str()).map(str::trim).unwrap_or("");
                if prompt.is_empty() { bail!("prompt required"); }
                let delay = match (args.get("delay_secs").and_then(|v| v.as_u64()), args.get("at").and_then(|v| v.as_str())) {
                    (Some(d), _) => Duration::from_secs(d),
                    (None, Some(at)) => parse_at(at)?,
                    (None, None) => match args.get("every_secs").and_then(|v| v.as_u64()) { Some(e) => Duration::from_secs(e), None => Duration::from_secs(60) },
                };
                let every = args.get("every_secs").and_then(|v| v.as_u64()).map(Duration::from_secs);
                if let Some(e) = every { if e.as_secs() < MIN_EVERY_SECS { bail!("every_secs must be ≥ {MIN_EVERY_SECS}"); } }
                let (id, delay) = add(ctx.inbox.clone(), prompt, delay, every)?;
                let kind = match every { Some(e) => format!("then every {}s", e.as_secs()), None => "once".into() };
                Ok(format!("scheduled #{id}: in {} ({kind}) — {}", fmt_delay(delay), crate::llm::truncate_for_log(prompt, 120)).into())
            }
            "list" => Ok(list().into()),
            "remove" => { let Some(id) = args.get("id").and_then(|v| v.as_u64()) else { bail!("id required") }; Ok(remove(id as u32)?.into()) }
            "clear" => Ok(clear().into()),
            _ => bail!("unknown action {action}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn one_shot_fires_and_disappears() {
        let ctx = ToolCtx::basic(std::env::temp_dir());
        assert!(Schedule.call(json!({"action":"add","prompt":"x","every_secs":1}), &ctx).await.is_err());
        let out = Schedule.call(json!({"action":"add","prompt":"re-run the tests","delay_secs":1}), &ctx).await.unwrap();
        assert!(out.text.starts_with("scheduled #"), "{}", out.text);
        let id: u32 = out.text["scheduled #".len()..].split(':').next().unwrap().parse().unwrap();
        assert!(list().contains(&format!("schedule #{id}: next in")));
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut got = None;
        while Instant::now() < deadline && got.is_none() {
            got = ctx.inbox.drain().into_iter().find(|it| it.source == format!("schedule #{id}"));
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let it = got.expect("prompt delivered");
        assert_eq!(it.text, "re-run the tests");
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!list().contains(&format!("schedule #{id}:")), "{}", list());
    }

    #[tokio::test]
    async fn recurring_fires_until_removed() {
        let inbox = Arc::new(Inbox::new());
        // the internal API allows sub-10s intervals (the tool enforces the minimum)
        let (id, _) = add(inbox.clone(), "poll", Duration::from_millis(100), Some(Duration::from_millis(200))).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut n = 0;
        while Instant::now() < deadline && n < 2 { n += inbox.drain().iter().filter(|it| it.source == format!("schedule #{id}") && it.text.starts_with("poll\n(recurring")).count(); tokio::time::sleep(Duration::from_millis(30)).await; }
        assert!(n >= 2, "{n}");
        assert!(list().contains(&format!("schedule #{id}: next in")) && list().contains("every 0s"), "{}", list());
        let msg = remove(id).unwrap();
        assert!(msg.contains("firings"), "{msg}");
        // no more firings after removal
        inbox.drain();
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(inbox.drain().iter().all(|it| it.source != format!("schedule #{id}")));
        assert!(!list().contains(&format!("schedule #{id}:")));
        assert!(remove(id).is_err());
    }

    #[test]
    fn parses_at() {
        assert_eq!(parse_at("90").unwrap(), Duration::from_secs(90));
        let d = parse_at("23:59").unwrap().as_secs();
        assert!(d <= 86400, "{d}");
        assert!(parse_at("nope").is_err());
        assert!(parse_at("25:00").is_err());
    }
}
