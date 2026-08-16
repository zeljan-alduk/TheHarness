//! Shell hooks (like Claude Code hooks): commands run before/after tool calls, when the agent stops,
//! and when the user submits a prompt. Each hook gets JSON on stdin. Pre-tool hooks can block a call
//! by exiting with code 2 (their stdout/stderr becomes the error the model sees).
//! Configure in harness.toml:
//!   [hooks]
//!   pre_tool  = ["./scripts/guard.sh", { command = "./scripts/no-rm.sh", matcher = "bash" }]   # exit 2 = block
//!   post_tool = [{ command = "cargo fmt", matcher = "write_file|edit_file|apply_patch" }]
//!   on_stop   = ["..."]                       # {"summary"}
//!   on_prompt = ["..."]                       # {"prompt"} — stdout is appended to the prompt as context
//!   session_start / session_end / subagent_stop / pre_compact / notification = ["..."]

use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;

/// A hook: either a plain command string or a table {command, matcher, timeout_secs}.
/// `matcher` is a glob (or "a|b" alternatives) on the tool name for tool events, or on the event's subject.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum HookSpec { Cmd(String), Full { command: String, #[serde(default)] matcher: Option<String>, #[serde(default)] timeout_secs: Option<u64> } }
impl HookSpec {
    pub fn command(&self) -> &str { match self { HookSpec::Cmd(c) => c, HookSpec::Full { command, .. } => command } }
    pub fn matches(&self, subject: &str) -> bool { match self { HookSpec::Cmd(_) => true, HookSpec::Full { matcher, .. } => matcher.as_deref().map(|m| m.split('|').any(|alt| crate::permissions::glob_match(alt.trim(), subject))).unwrap_or(true) } }
    pub fn timeout(&self, default: u64) -> u64 { match self { HookSpec::Full { timeout_secs: Some(t), .. } => *t, _ => default } }
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct HooksConfig {
    #[serde(default)] pub pre_tool: Vec<HookSpec>,
    #[serde(default)] pub post_tool: Vec<HookSpec>,
    #[serde(default)] pub on_stop: Vec<HookSpec>,
    #[serde(default)] pub on_prompt: Vec<HookSpec>,
    #[serde(default)] pub session_start: Vec<HookSpec>,
    #[serde(default)] pub session_end: Vec<HookSpec>,
    #[serde(default)] pub subagent_stop: Vec<HookSpec>,
    #[serde(default)] pub pre_compact: Vec<HookSpec>,
    #[serde(default)] pub notification: Vec<HookSpec>,
    #[serde(default = "d_timeout")] pub timeout_secs: u64,
}
fn d_timeout() -> u64 { 30 }

async fn run_hook(cmd: &str, input: &serde_json::Value, cwd: &Path, timeout: Duration) -> (i32, String) {
    let (prog, flag) = crate::sandbox::shell_program();
    let mut c = tokio::process::Command::new(prog);
    c.arg(flag).arg(cmd).current_dir(cwd).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).kill_on_drop(true);
    c.env("PATH", crate::setup::path_with_bin_dir(cwd));
    let Ok(mut child) = c.spawn() else { return (1, format!("hook failed to start: {cmd}")) };
    if let Some(mut stdin) = child.stdin.take() { use tokio::io::AsyncWriteExt; let _ = stdin.write_all(input.to_string().as_bytes()).await; }
    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(o)) => (o.status.code().unwrap_or(1), format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)).trim().to_string()),
        Ok(Err(e)) => (1, e.to_string()),
        Err(_) => (1, format!("hook timed out: {cmd}")),
    }
}

/// Returns Some(reason) if a pre_tool hook blocks the call.
pub async fn run_pre_tool(cfg: &HooksConfig, tool: &str, args: &str, cwd: &Path) -> Option<String> {
    for h in cfg.pre_tool.iter().filter(|h| h.matches(tool)) {
        let (code, out) = run_hook(h.command(), &serde_json::json!({"event": "pre_tool", "tool": tool, "args": args, "workdir": cwd.display().to_string()}), cwd, Duration::from_secs(h.timeout(cfg.timeout_secs))).await;
        if code == 2 { return Some(if out.is_empty() { format!("blocked by hook {}", h.command()) } else { out }); }
    }
    None
}
pub async fn run_post_tool(cfg: &HooksConfig, tool: &str, result: &str, cwd: &Path) {
    for h in cfg.post_tool.iter().filter(|h| h.matches(tool)) { let _ = run_hook(h.command(), &serde_json::json!({"event": "post_tool", "tool": tool, "result": crate::llm::truncate_for_log(result, 4000), "workdir": cwd.display().to_string()}), cwd, Duration::from_secs(h.timeout(cfg.timeout_secs))).await; }
}
pub async fn run_on_stop(cfg: &HooksConfig, summary: &str, cwd: &Path) {
    for h in &cfg.on_stop { let _ = run_hook(h.command(), &serde_json::json!({"event": "on_stop", "summary": summary, "workdir": cwd.display().to_string()}), cwd, Duration::from_secs(h.timeout(cfg.timeout_secs))).await; }
}
pub async fn run_on_prompt(cfg: &HooksConfig, prompt: &str, cwd: &Path) -> Option<String> {
    let mut extra = String::new();
    for h in &cfg.on_prompt { let (code, out) = run_hook(h.command(), &serde_json::json!({"event": "on_prompt", "prompt": prompt, "workdir": cwd.display().to_string()}), cwd, Duration::from_secs(h.timeout(cfg.timeout_secs))).await; if code == 0 && !out.is_empty() { extra.push_str(&out); extra.push('\n'); } }
    if extra.is_empty() { None } else { Some(extra) }
}
/// Generic lifecycle event: session_start | session_end | subagent_stop | pre_compact | notification.
/// `subject` is matched against the hook's matcher (e.g. sub-agent label, notification title).
pub async fn run_event(cfg: &HooksConfig, event: &str, subject: &str, payload: serde_json::Value, cwd: &Path) -> Vec<String> {
    let hooks = match event { "session_start" => &cfg.session_start, "session_end" => &cfg.session_end, "subagent_stop" => &cfg.subagent_stop, "pre_compact" => &cfg.pre_compact, "notification" => &cfg.notification, _ => return vec![] };
    let mut outs = Vec::new();
    for h in hooks.iter().filter(|h| h.matches(subject)) {
        let mut p = payload.clone(); p["event"] = serde_json::json!(event); p["workdir"] = serde_json::json!(cwd.display().to_string());
        let (code, out) = run_hook(h.command(), &p, cwd, Duration::from_secs(h.timeout(cfg.timeout_secs))).await;
        if code == 0 && !out.is_empty() { outs.push(out); }
    }
    outs
}
