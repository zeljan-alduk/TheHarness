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

/// POSIX single-quote shell quoting: safe to interpolate into `sh -c` strings.
pub fn shq(s: &str) -> String { format!("'{}'", s.replace('\'', "'\\''")) }

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

/// The shell command every tool-spawned process goes through: sandbox wrapper (seatbelt/bwrap if
/// configured), scrubbed env (no secrets), harness PATH, stdin closed, own session (process group),
/// killed on drop. Stdout/stderr are left to the caller.
pub fn build_shell_command(cmd: &str, cwd: &Path) -> Command {
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
        .kill_on_drop(true);
    c.env_clear();
    for (k, v) in std::env::vars() {
        if !is_secret_env(&k) { c.env(k, v); }
    }
    c.env("HARNESS", "1").env("CI", "1").env("GIT_TERMINAL_PROMPT", "0").env("TERM", "dumb");
    c.env("PATH", crate::setup::path_with_bin_dir(cwd));
    // New session => own process group, so we can kill the whole tree on timeout.
    #[cfg(unix)] unsafe { c.pre_exec(|| { libc::setsid(); Ok(()) }); }
    c
}

/// Head+tail byte buffer with a cap: everything beyond `cap` bytes is discarded while reading, so a
/// chatty/never-ending process cannot exhaust memory. `into_string` marks the elision.
struct CappedBuf { head: Vec<u8>, tail: std::collections::VecDeque<u8>, head_cap: usize, tail_cap: usize, dropped: usize }
impl CappedBuf {
    fn new(cap: usize) -> Self { let head_cap = cap * 2 / 3; Self { head: Vec::new(), tail: Default::default(), head_cap, tail_cap: cap - head_cap, dropped: 0 } }
    fn push(&mut self, b: &[u8]) {
        let take = self.head_cap.saturating_sub(self.head.len()).min(b.len());
        self.head.extend_from_slice(&b[..take]);
        for &x in &b[take..] { if self.tail.len() >= self.tail_cap { self.tail.pop_front(); self.dropped += 1; } self.tail.push_back(x); }
    }
    fn into_string(mut self) -> String {
        if self.dropped == 0 { self.head.extend(self.tail); return String::from_utf8_lossy(&self.head).into_owned(); }
        format!("{}\n…[{} bytes elided]…\n{}", String::from_utf8_lossy(&self.head), self.dropped, String::from_utf8_lossy(self.tail.make_contiguous()))
    }
}
async fn read_capped<R: tokio::io::AsyncRead + Unpin>(r: Option<R>, cap: usize) -> CappedBuf {
    use tokio::io::AsyncReadExt;
    let mut out = CappedBuf::new(cap);
    let Some(mut r) = r else { return out };
    let mut buf = [0u8; 8192];
    loop { match r.read(&mut buf).await { Ok(0) | Err(_) => break, Ok(n) => out.push(&buf[..n]) } }
    out
}

pub async fn run_shell(cmd: &str, cwd: &Path, timeout: Duration, max_output: usize) -> Result<ProcOutput> {
    let start = std::time::Instant::now();
    let mut c = build_shell_command(cmd, cwd);
    c.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = c.spawn()?;
    #[cfg(unix)] let pid = child.id().map(|p| p as i32);
    // keep at most ~4×max_output bytes per stream (chars ≤ bytes); truncate_middle finishes the job
    let cap = max_output.saturating_mul(4).max(4096);
    let (so, se) = (child.stdout.take(), child.stderr.take());
    let run = async { let (o, e, st) = tokio::join!(read_capped(so, cap), read_capped(se, cap), child.wait()); st.map(|st| (o, e, st)) };
    match tokio::time::timeout(timeout, run).await {
        Ok(out) => {
            let (o, e, status) = out?;
            Ok(ProcOutput {
                stdout: truncate_middle(&o.into_string(), max_output),
                stderr: truncate_middle(&e.into_string(), max_output),
                code: status.code(),
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
    #[test]
    fn quotes() { assert_eq!(shq("a'b"), "'a'\\''b'"); assert_eq!(shq("x y"), "'x y'"); }
    #[test]
    fn capped_buf_keeps_head_and_tail() {
        let mut b = CappedBuf::new(30);
        for i in 0..10 { b.push(format!("{i:0>10}").as_bytes()); }
        let s = b.into_string();
        assert!(s.starts_with("00000000000000000001"), "{s}"); assert!(s.ends_with("0000000009"), "{s}"); assert!(s.contains("bytes elided"), "{s}");
        let mut b = CappedBuf::new(30); b.push(b"short"); assert_eq!(b.into_string(), "short");
    }
    #[tokio::test]
    async fn caps_huge_output() {
        let o = run_shell("yes | head -c 5000000", Path::new("/tmp"), Duration::from_secs(20), 200).await.unwrap();
        assert!(o.stdout.len() < 2000, "{}", o.stdout.len()); assert!(o.stdout.contains("elided"));
    }
    #[test]
    fn secret_env_detection() { assert!(is_secret_env("OPENAI_API_KEY")); assert!(is_secret_env("aws_secret")); assert!(!is_secret_env("HARNESS_TOKEN_BUDGET")); assert!(!is_secret_env("PATH")); }
}
