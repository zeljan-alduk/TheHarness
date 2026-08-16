//! Persistent scheduler: named jobs (a prompt + a working directory + a cadence) stored in
//! `~/.config/harness/schedules.json` and executed by `harness daemon`. Unlike the in-session
//! `schedule` tool — which delivers a reminder to the running agent — these survive restarts and run
//! the agent themselves, headless, with the output kept next to the job (`harness schedule log`).

use crate::config::Config;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

pub const MAX_JOBS: usize = 64;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    /// What to run.
    pub prompt: String,
    pub workdir: String,
    /// Repeat every N seconds (mutually exclusive with `at`).
    #[serde(default)] pub every_secs: Option<u64>,
    /// Local daily time "HH:MM".
    #[serde(default)] pub at: Option<String>,
    /// Unix seconds of the next run.
    pub next_at: u64,
    #[serde(default = "d_true")] pub enabled: bool,
    #[serde(default)] pub runs: usize,
    #[serde(default)] pub last_run: u64,
    #[serde(default)] pub last_status: String,
    #[serde(default)] pub max_turns: Option<usize>,
    #[serde(default)] pub created: u64,
}
fn d_true() -> bool { true }

impl Job {
    pub fn cadence(&self) -> String {
        match (&self.at, self.every_secs) {
            (Some(t), _) => format!("daily at {t}"),
            (None, Some(e)) => format!("every {}", fmt_secs(e)),
            _ => "once".into(),
        }
    }
    pub fn due(&self, now: u64) -> bool { self.enabled && self.next_at <= now }
    /// When it should run next after firing at `now` (None = one-shot, remove it).
    pub fn reschedule(&self, now: u64) -> Option<u64> {
        match (&self.at, self.every_secs) {
            (Some(t), _) => next_daily(t, now).ok(),
            (None, Some(e)) => Some(now + e.max(30)),
            _ => None,
        }
    }
}

pub fn now() -> u64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) }

pub fn fmt_secs(s: u64) -> String {
    if s % 86400 == 0 && s >= 86400 { format!("{}d", s / 86400) }
    else if s % 3600 == 0 && s >= 3600 { format!("{}h", s / 3600) }
    else if s >= 60 { format!("{}m", s / 60) }
    else { format!("{s}s") }
}

/// "30s" "10m" "2h" "1d" or a bare number of seconds.
pub fn parse_every(s: &str) -> Result<u64> {
    let t = s.trim().to_lowercase();
    let (num, mult) = match t.chars().last() {
        Some('s') => (&t[..t.len() - 1], 1),
        Some('m') => (&t[..t.len() - 1], 60),
        Some('h') => (&t[..t.len() - 1], 3600),
        Some('d') => (&t[..t.len() - 1], 86400),
        _ => (t.as_str(), 1),
    };
    let n: u64 = num.trim().parse().with_context(|| format!("cannot parse interval {s:?} (use 30s, 10m, 2h, 1d)"))?;
    let secs = n * mult;
    if secs < 30 { bail!("interval must be at least 30s"); }
    Ok(secs)
}

/// Local seconds since midnight (unix: localtime_r; elsewhere UTC).
pub fn local_secs_of_day() -> u64 {
    #[cfg(unix)]
    unsafe {
        let t: libc::time_t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        if !libc::localtime_r(&t, &mut tm).is_null() { return (tm.tm_hour as u64) * 3600 + (tm.tm_min as u64) * 60 + tm.tm_sec as u64; }
    }
    now() % 86400
}

/// Unix time of the next local "HH:MM" strictly after `from`.
pub fn next_daily(hhmm: &str, from: u64) -> Result<u64> {
    let (h, m) = hhmm.trim().split_once(':').context("time must look like HH:MM")?;
    let (h, m): (u64, u64) = (h.trim().parse().context("bad hour")?, m.trim().parse().context("bad minute")?);
    if h > 23 || m > 59 { bail!("{hhmm} is not a valid time"); }
    let target = h * 3600 + m * 60;
    let today = local_secs_of_day();
    let delta = if target > today { target - today } else { target + 86400 - today };
    Ok(from + delta)
}

pub struct Store { pub path: PathBuf }

impl Store {
    pub fn open() -> Result<Store> {
        let dir = crate::setup::config_dir();
        std::fs::create_dir_all(&dir)?;
        Ok(Store { path: dir.join("schedules.json") })
    }
    pub fn list(&self) -> Vec<Job> {
        std::fs::read_to_string(&self.path).ok().and_then(|t| serde_json::from_str::<Vec<Job>>(&t).ok()).unwrap_or_default()
    }
    fn save(&self, jobs: &[Job]) -> Result<()> {
        std::fs::write(&self.path, serde_json::to_string_pretty(jobs)?)?;
        Ok(())
    }
    pub fn get(&self, id: &str) -> Option<Job> {
        let jobs = self.list();
        jobs.iter().find(|j| j.id == id).cloned().or_else(|| { let hits: Vec<&Job> = jobs.iter().filter(|j| j.id.starts_with(id)).collect(); (hits.len() == 1).then(|| hits[0].clone()) })
    }
    pub fn add(&self, mut job: Job) -> Result<Job> {
        let mut jobs = self.list();
        if jobs.len() >= MAX_JOBS { bail!("too many scheduled jobs ({MAX_JOBS})"); }
        if jobs.iter().any(|j| j.id == job.id) { bail!("a job named '{}' already exists", job.id); }
        if job.created == 0 { job.created = now(); }
        jobs.push(job.clone());
        self.save(&jobs)?;
        Ok(job)
    }
    pub fn remove(&self, id: &str) -> Result<Job> {
        let job = self.get(id).with_context(|| format!("no scheduled job '{id}'"))?;
        let mut jobs = self.list();
        jobs.retain(|j| j.id != job.id);
        self.save(&jobs)?;
        Ok(job)
    }
    pub fn set_enabled(&self, id: &str, on: bool) -> Result<Job> {
        self.update(id, |j| { j.enabled = on; if on && j.next_at <= now() { j.next_at = j.reschedule(now()).unwrap_or(now() + 60); } })
    }
    pub fn update(&self, id: &str, f: impl FnOnce(&mut Job)) -> Result<Job> {
        let target = self.get(id).with_context(|| format!("no scheduled job '{id}'"))?;
        let mut jobs = self.list();
        let j = jobs.iter_mut().find(|j| j.id == target.id).unwrap();
        f(j);
        let out = j.clone();
        self.save(&jobs)?;
        Ok(out)
    }
    pub fn due(&self, at: u64) -> Vec<Job> { self.list().into_iter().filter(|j| j.due(at)).collect() }

    /// Record the outcome of a run and move the job forward (one-shot jobs are removed).
    pub fn finish(&self, id: &str, status: &str) -> Result<()> {
        let t = now();
        let job = self.get(id).with_context(|| format!("no scheduled job '{id}'"))?;
        match job.reschedule(t) {
            Some(next) => { self.update(id, |j| { j.runs += 1; j.last_run = t; j.last_status = status.to_string(); j.next_at = next; })?; }
            None => { self.remove(id)?; }
        }
        Ok(())
    }
    pub fn log_path(&self, id: &str) -> PathBuf { crate::setup::config_dir().join("schedules").join(format!("{id}.log")) }
}

/// Run one job now (headless, non-interactive) and record the outcome.
pub async fn run_job(cfg: &Config, store: &Store, job: &Job) -> Result<String> {
    let workdir = PathBuf::from(&job.workdir);
    if !workdir.is_dir() { let msg = format!("workdir {} is gone", job.workdir); store.finish(&job.id, &msg)?; bail!(msg); }
    let mut cfg = cfg.clone();
    if let Some(n) = job.max_turns { cfg.agent.max_turns = n; }
    let sink: std::sync::Arc<dyn crate::events::Sink> = std::sync::Arc::new(crate::events::StderrSink { verbose: false });
    let approver: std::sync::Arc<dyn crate::permissions::Approver> = std::sync::Arc::new(crate::permissions::AutoApprover { yes: true });
    let mut setup = crate::runner::RunSetup::new(cfg, workdir.clone(), sink, approver);
    setup.prompt_extra = Some(format!("You are running unattended from the harness scheduler (job '{}'). Nobody is watching: do not ask questions, finish the work, and end with a short report of what you found or changed.", job.id));
    setup.session_id = Some(format!("cron-{}-{}", job.id, now()));
    let started = std::time::Instant::now();
    let out = crate::runner::start_run(setup, job.prompt.clone()).await;
    let (status, text) = match &out {
        Ok(t) => (format!("ok in {:.0}s", started.elapsed().as_secs_f64()), t.clone()),
        Err(e) => (format!("failed: {}", crate::llm::truncate_for_log(&format!("{e:#}"), 120)), format!("{e:#}")),
    };
    let log = store.log_path(&job.id);
    if let Some(p) = log.parent() { let _ = std::fs::create_dir_all(p); }
    let entry = format!("\n===== {} — {status}\n{}\n", stamp(), crate::llm::truncate_for_log(&text, 8000));
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&log) { let _ = f.write_all(entry.as_bytes()); }
    store.finish(&job.id, &status)?;
    out
}

fn stamp() -> String {
    let t = now();
    format!("{} {:02}:{:02}:{:02} UTC", crate::memory::today_iso(), (t % 86400) / 3600, (t % 3600) / 60, t % 60)
}

/// The daemon loop: check every `tick` seconds and run whatever is due, one at a time.
pub async fn daemon(cfg: Config, tick: Duration, once: bool) -> Result<()> {
    let store = Store::open()?;
    eprintln!("harness daemon: {} job(s) in {} (tick {}s)", store.list().len(), store.path.display(), tick.as_secs());
    loop {
        let due = store.due(now());
        for job in due {
            eprintln!("· running '{}' ({}) in {}", job.id, job.cadence(), job.workdir);
            match run_job(&cfg, &store, &job).await {
                Ok(t) => eprintln!("· '{}' done: {}", job.id, crate::llm::truncate_for_log(t.trim(), 160)),
                Err(e) => eprintln!("· '{}' failed: {e:#}", job.id),
            }
        }
        if once { return Ok(()); }
        tokio::time::sleep(tick).await;
    }
}

/// Table for `harness schedule list` / the TUI.
pub fn render(jobs: &[Job]) -> Vec<String> {
    if jobs.is_empty() { return vec!["no scheduled jobs — harness schedule add <name> --every 1h \"<prompt>\"".into()]; }
    let t = now();
    let mut lines = vec![format!("{:<16} {:<12} {:>9}  {:<7} {}", "job", "cadence", "next", "runs", "prompt")];
    for j in jobs {
        let next = if !j.enabled { "paused".to_string() } else if j.next_at <= t { "due".to_string() } else { fmt_secs(j.next_at - t) };
        lines.push(format!("{:<16} {:<12} {:>9}  {:<7} {}", crate::llm::truncate_for_log(&j.id, 16), j.cadence(), next, j.runs, crate::llm::truncate_for_log(&j.prompt, 60)));
        if !j.last_status.is_empty() { lines.push(format!("{:<16} last: {}", "", j.last_status)); }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_at(dir: &std::path::Path) -> Store { Store { path: dir.join("schedules.json") } }

    #[test]
    fn intervals_and_daily_times() {
        assert_eq!(parse_every("90").unwrap(), 90);
        assert_eq!(parse_every("10m").unwrap(), 600);
        assert_eq!(parse_every("2h").unwrap(), 7200);
        assert_eq!(parse_every("1d").unwrap(), 86400);
        assert!(parse_every("5s").is_err(), "too frequent");
        assert!(parse_every("nope").is_err());
        let n = next_daily("03:00", 1_000_000).unwrap();
        assert!(n > 1_000_000 && n <= 1_000_000 + 86400);
        assert!(next_daily("99:00", 0).is_err());
        assert_eq!(fmt_secs(3600), "1h");
        assert_eq!(fmt_secs(90), "1m");
    }

    #[test]
    fn crud_and_rescheduling() {
        let d = std::env::temp_dir().join(format!("harness-sched-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let s = store_at(&d);
        let t = now();
        let job = Job { id: "nightly".into(), prompt: "run the evals".into(), workdir: "/tmp".into(), every_secs: Some(3600), at: None, next_at: t - 1, enabled: true, runs: 0, last_run: 0, last_status: String::new(), max_turns: None, created: 0 };
        s.add(job.clone()).unwrap();
        assert!(s.add(job.clone()).is_err(), "duplicate names are refused");
        assert_eq!(s.due(t).len(), 1);
        s.finish("nightly", "ok in 3s").unwrap();
        let j = s.get("nightly").unwrap();
        assert_eq!(j.runs, 1);
        assert!(j.next_at > t, "a recurring job moves forward");
        assert!(s.due(t).is_empty());
        s.set_enabled("night", false).unwrap();          // prefix lookup
        assert!(!s.get("nightly").unwrap().enabled);
        assert!(s.due(j.next_at + 10).is_empty(), "a paused job is never due");

        // one-shot jobs disappear after they run
        let one = Job { id: "once".into(), prompt: "x".into(), workdir: "/tmp".into(), every_secs: None, at: None, next_at: t, enabled: true, runs: 0, last_run: 0, last_status: String::new(), max_turns: None, created: 0 };
        s.add(one).unwrap();
        s.finish("once", "ok").unwrap();
        assert!(s.get("once").is_none());
        assert!(render(&s.list()).len() >= 2);
        let _ = std::fs::remove_dir_all(&d);
    }
}
