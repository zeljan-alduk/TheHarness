//! Local process execution with timeout, process-group kill, env scrubbing and output caps.
//! NOTE: this is *supervision*, not isolation. For true isolation run the harness itself
//! inside a container/VM. That is deliberate for v0 (local processes, per design).

use anyhow::Result;

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

#[derive(Debug)]
pub struct ProcOutput {
    pub stdout: String,
    pub stderr: String,
    pub code: Option<i32>,
    pub timed_out: bool,
    pub elapsed: Duration,
}

impl ProcOutput {
    pub fn success(&self) -> bool { !self.timed_out && self.code == Some(0) }
}

fn is_secret_env(k: &str) -> bool {
    let u = k.to_ascii_uppercase();
    ["KEY", "TOKEN", "SECRET", "PASSWORD", "PASSWD", "CREDENTIAL", "AUTH"].iter().any(|s| u.contains(s))
        && !u.starts_with("HARNESS_")
}

static SEATBELT: std::sync::OnceLock<Option<(bool, Vec<String>)>> = std::sync::OnceLock::new();
/// Enable macOS seatbelt for all shell commands (call once at startup).
pub fn configure_seatbelt(enabled: bool, deny_network: bool, allow_write: Vec<String>) {
    let _ = SEATBELT.set(if enabled { Some((deny_network, allow_write)) } else { None });
}
fn seatbelt_profile(cwd: &Path, deny_network: bool, extra: &[String]) -> String {
    let home = crate::setup::home_dir().display().to_string();
    let mut writable = vec![cwd.canonicalize().unwrap_or(cwd.to_path_buf()).display().to_string(), std::env::temp_dir().display().to_string(), "/private/tmp".into(), "/tmp".into(), format!("{home}/.config/harness"), format!("{home}/.cargo/registry"), format!("{home}/.cargo/git"), format!("{home}/.cache"), format!("{home}/.npm"), "/dev".into()];
    writable.extend(extra.iter().cloned());
    let allows: String = writable.iter().map(|p| format!("(subpath \"{}\")", p.replace('"', ""))).collect::<Vec<_>>().join(" ");
    let net = if deny_network { "(deny network*) (allow network* (local ip \"localhost:*\")) (allow network* (remote ip \"localhost:*\"))" } else { "" };
    format!("(version 1) (allow default) (deny file-write*) (allow file-write* {allows}) (allow file-write* (literal \"/dev/null\") (literal \"/dev/tty\") (regex #\"^/dev/tty\")) {net}")
}

/// The shell used for `bash` tool commands: /bin/sh on unix; on Windows Git Bash (`bash -c`) if
/// installed (POSIX semantics the prompts assume), else `cmd /C`.
pub fn shell_program() -> (String, &'static str) {
    if cfg!(windows) {
        for cand in ["C:\\Program Files\\Git\\bin\\bash.exe", "C:\\Program Files\\Git\\usr\\bin\\bash.exe"] { if Path::new(cand).exists() { return (cand.to_string(), "-c"); } }
        if let Some(p) = crate::setup::which("bash") { return (p.display().to_string(), "-c"); }
        return ("cmd".to_string(), "/C");
    }
    ("/bin/sh".to_string(), "-c")
}

pub async fn run_shell(cmd: &str, cwd: &Path, timeout: Duration, max_output: usize) -> Result<ProcOutput> {
    let start = std::time::Instant::now();
    let seatbelt = SEATBELT.get().cloned().flatten().filter(|_| cfg!(target_os = "macos") && Path::new("/usr/bin/sandbox-exec").exists());
    let (prog, flag) = shell_program();
    let bwrap = SEATBELT.get().cloned().flatten().filter(|_| cfg!(target_os = "linux") && crate::setup::which("bwrap").is_some());
    let mut c = match (&seatbelt, &bwrap) {
        (Some((deny_net, extra)), _) => { let mut c = Command::new("/usr/bin/sandbox-exec"); c.arg("-p").arg(seatbelt_profile(cwd, *deny_net, extra)).arg(&prog); c }
        (_, Some((deny_net, extra))) => {
            // bubblewrap: read-only root, writable workdir/temp/harness config (+extra), private /dev,/proc; optional no network
            let mut c = Command::new("bwrap");
            c.args(["--ro-bind", "/", "/", "--dev", "/dev", "--proc", "/proc", "--tmpfs", "/tmp", "--die-with-parent"]);
            let home = crate::setup::home_dir();
            let mut rw: Vec<PathBuf> = vec![cwd.canonicalize().unwrap_or(cwd.to_path_buf()), std::env::temp_dir(), home.join(".config/harness"), home.join(".cargo"), home.join(".cache")];
            rw.extend(extra.iter().map(PathBuf::from));
            for p in rw { if p.exists() { let ps = p.display().to_string(); c.args(["--bind", &ps, &ps]); } }
            if *deny_net { c.arg("--unshare-net"); }
            c.arg("--").arg(&prog); c
        }
        _ => Command::new(&prog),
    };
    c.arg(flag).arg(cmd)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    c.env_clear();
    for (k, v) in std::env::vars() {
        if !is_secret_env(&k) { c.env(k, v); }
    }
    c.env("HARNESS", "1").env("CI", "1").env("GIT_TERMINAL_PROMPT", "0").env("TERM", "dumb");
    c.env("PATH", crate::setup::path_with_bin_dir(cwd));
    // New session => own process group, so we can kill the whole tree on timeout.
    #[cfg(unix)] unsafe { c.pre_exec(|| { libc::setsid(); Ok(()) }); }

    let child = c.spawn()?;
    #[cfg(unix)] let pid = child.id().map(|p| p as i32);
    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(out) => {
            let out = out?;
            Ok(ProcOutput {
                stdout: truncate_middle(&String::from_utf8_lossy(&out.stdout), max_output),
                stderr: truncate_middle(&String::from_utf8_lossy(&out.stderr), max_output),
                code: out.status.code(),
                timed_out: false,
                elapsed: start.elapsed(),
            })
        }
        Err(_) => {
            #[cfg(unix)] { if let Some(pid) = pid { unsafe { libc::kill(-pid, libc::SIGKILL); } } }
            // (windows: kill_on_drop terminates the shell when the future is dropped)
            Ok(ProcOutput { stdout: String::new(), stderr: format!("killed after {}s timeout", timeout.as_secs()), code: None, timed_out: true, elapsed: start.elapsed() })
        }
    }
}

/// Keep head and tail; mark the elision. Char-safe.
pub fn truncate_middle(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if n <= max { return s.to_string(); }
    let head = max * 2 / 3;
    let tail = max - head;
    let h: String = s.chars().take(head).collect();
    let t: String = s.chars().skip(n - tail).collect();
    format!("{h}\n…[{} chars elided]…\n{t}", n - max)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[tokio::test]
    async fn runs_and_captures() {
        let o = run_shell("echo hi; echo err >&2; exit 3", Path::new("/tmp"), Duration::from_secs(5), 1000).await.unwrap();
        assert_eq!(o.stdout.trim(), "hi"); assert_eq!(o.stderr.trim(), "err"); assert_eq!(o.code, Some(3));
    }
    #[tokio::test]
    async fn kills_on_timeout() {
        let t = std::time::Instant::now();
        let o = run_shell("sleep 30", Path::new("/tmp"), Duration::from_millis(300), 1000).await.unwrap();
        assert!(o.timed_out); assert!(t.elapsed() < Duration::from_secs(5));
    }
    #[test]
    fn truncates_middle() {
        let s = "a".repeat(100);
        let t = truncate_middle(&s, 30);
        assert!(t.contains("elided")); assert!(t.len() < 100);
        assert_eq!(truncate_middle("short", 30), "short");
    }
}
