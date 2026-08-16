//! Background processes started by the agent (dev servers, watchers, long builds). Output goes to a
//! log file; the `process` tool lists, tails and kills them. Killed on harness exit (kill_on_drop).

use anyhow::{bail, Result};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

pub struct Proc { pub id: u32, pub pid: u32, pub cmd: String, pub log: PathBuf, pub started: Instant, pub child: tokio::process::Child }

static PROCS: OnceLock<Mutex<HashMap<u32, Proc>>> = OnceLock::new();
static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
fn table() -> &'static Mutex<HashMap<u32, Proc>> { PROCS.get_or_init(|| Mutex::new(HashMap::new())) }

pub fn log_dir() -> PathBuf { let d = std::env::temp_dir().join("harness-procs"); let _ = std::fs::create_dir_all(&d); d }

pub async fn start(cmd: &str, cwd: &std::path::Path) -> Result<(u32, PathBuf)> {
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let log = log_dir().join(format!("proc-{}-{id}.log", std::process::id()));
    let f = std::fs::File::create(&log)?;
    // same sandbox / env scrub / own session as foreground `bash` (sandbox::run_shell)
    let mut c = crate::sandbox::build_shell_command(cmd, cwd);
    c.stdout(f.try_clone()?).stderr(f);
    let child = c.spawn()?;
    let pid = child.id().unwrap_or(0);
    table().lock().unwrap().insert(id, Proc { id, pid, cmd: cmd.to_string(), log: log.clone(), started: Instant::now(), child });
    Ok((id, log))
}

pub fn list() -> Vec<(u32, u32, String, String, f64, PathBuf)> {
    let mut t = table().lock().unwrap();
    let mut out = Vec::new();
    for p in t.values_mut() {
        let status = match p.child.try_wait() { Ok(Some(st)) => format!("exited {}", st.code().map(|c| c.to_string()).unwrap_or("signal".into())), Ok(None) => "running".into(), Err(_) => "?".into() };
        out.push((p.id, p.pid, p.cmd.clone(), status, p.started.elapsed().as_secs_f64(), p.log.clone()));
    }
    out.sort_by_key(|x| x.0);
    out
}

/// Last `lines` lines of the log (reads only the tail of the file).
pub fn tail(id: u32, lines: usize) -> Result<String> {
    let log = { let t = table().lock().unwrap(); t.get(&id).map(|p| p.log.clone()) };
    let Some(log) = log else { bail!("no background process #{id}") };
    let bytes = std::fs::read(&log).unwrap_or_default();
    // read only the end of the log: 4 KiB per requested line is plenty, then keep the last `lines`
    let keep = lines.max(1).saturating_mul(4096).max(64 * 1024);
    let start = bytes.len().saturating_sub(keep);
    let text = String::from_utf8_lossy(&bytes[start..]);
    let mut v: std::collections::VecDeque<&str> = std::collections::VecDeque::with_capacity(lines.min(10_000) + 1);
    for l in text.lines() { if v.len() >= lines.max(1) { v.pop_front(); } v.push_back(l); }
    Ok(v.into_iter().collect::<Vec<_>>().join("\n"))
}

pub async fn kill(id: u32) -> Result<String> {
    let pid = { let t = table().lock().unwrap(); t.get(&id).map(|p| p.pid) };
    let Some(pid) = pid else { bail!("no background process #{id}") };
    // never kill(0, …) / kill(-0, …): that would signal our own process group
    if pid > 0 {
        #[cfg(unix)] { unsafe { libc::kill(-(pid as i32), libc::SIGTERM); } tokio::time::sleep(std::time::Duration::from_millis(500)).await; unsafe { libc::kill(-(pid as i32), libc::SIGKILL); } }
        #[cfg(windows)] { let _ = tokio::process::Command::new("taskkill").args(["/PID", &pid.to_string(), "/T", "/F"]).output().await; }
    }
    let mut t = table().lock().unwrap();
    if let Some(p) = t.get_mut(&id) { let _ = p.child.start_kill(); }
    Ok(format!("sent SIGTERM/SIGKILL to process group of #{id} (pid {pid})"))
}
