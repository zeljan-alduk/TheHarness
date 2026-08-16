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
    let mut c = tokio::process::Command::new("/bin/sh");
    c.arg("-c").arg(cmd).current_dir(cwd).stdin(std::process::Stdio::null()).stdout(f.try_clone()?).stderr(f).kill_on_drop(true);
    c.env("PATH", crate::setup::path_with_bin_dir(cwd)).env("HARNESS", "1").env("CI", "1").env("TERM", "dumb");
    unsafe { c.pre_exec(|| { libc::setsid(); Ok(()) }); }
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

pub fn tail(id: u32, lines: usize) -> Result<String> {
    let log = { let t = table().lock().unwrap(); t.get(&id).map(|p| p.log.clone()) };
    let Some(log) = log else { bail!("no background process #{id}") };
    let text = std::fs::read_to_string(&log).unwrap_or_default();
    let v: Vec<&str> = text.lines().collect();
    let start = v.len().saturating_sub(lines);
    Ok(v[start..].join("\n"))
}

pub async fn kill(id: u32) -> Result<String> {
    let pid = { let t = table().lock().unwrap(); t.get(&id).map(|p| p.pid) };
    let Some(pid) = pid else { bail!("no background process #{id}") };
    unsafe { libc::kill(-(pid as i32), libc::SIGTERM); }
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    unsafe { libc::kill(-(pid as i32), libc::SIGKILL); }
    let mut t = table().lock().unwrap();
    if let Some(p) = t.get_mut(&id) { let _ = p.child.start_kill(); }
    Ok(format!("sent SIGTERM/SIGKILL to process group of #{id} (pid {pid})"))
}
