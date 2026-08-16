//! Hooks: user-supplied logic that runs around the agent's lifecycle — as a shell command, an HTTP
//! call, or an LLM judgement. Every hook receives the event as JSON (on stdin, as a POST body, or as
//! the material for the judgement) and may answer with JSON:
//!
//!   {"decision": "block" | "allow" | "ask", "reason": "...", "updatedInput": {...}, "context": "..."}
//!
//! `block` (or exit code 2 for commands) stops a tool call; `updatedInput` rewrites its arguments;
//! `context` is appended to the model's view. Configure in harness.toml:
//!
//!   [hooks]
//!   pre_tool  = ["./scripts/guard.sh",                                    # exit 2 = block
//!                { command = "./scripts/no-rm.sh", matcher = "bash" },
//!                { url = "http://localhost:9000/guard", matcher = "bash|terminal" },
//!                { prompt = "Block anything that touches production.", matcher = "bash" }]
//!   post_tool = [{ command = "cargo fmt", matcher = "write_file|edit_file", async = true }]
//!   session_start = [{ command = "./scripts/brief.sh", once = true }]
//!
//! Events: pre_tool · post_tool · post_tool_failure · permission_request · permission_denied ·
//! on_stop · on_prompt · session_start · session_end · subagent_stop · pre_compact · post_compact ·
//! before_model · after_model · file_changed · worktree_create · notification.
//! Hooks from `.claude/settings.json` are imported too (see `import_claude_hooks`).

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

/// A hook: a plain command string, or a table selecting one executor
/// ({command} | {url} | {prompt}) plus `matcher`, `timeout_secs`, `once`, `async`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum HookSpec {
    Cmd(String),
    Full {
        #[serde(default)] command: Option<String>,
        /// HTTP executor: POST the event JSON here; the JSON response is the outcome.
        #[serde(default)] url: Option<String>,
        /// LLM executor: judge the event with this instruction (uses the aux model).
        #[serde(default)] prompt: Option<String>,
        #[serde(default)] matcher: Option<String>,
        #[serde(default)] timeout_secs: Option<u64>,
        /// Run at most once per process.
        #[serde(default)] once: bool,
        /// Do not wait for it (never blocks; ignores the outcome).
        #[serde(default, rename = "async")] is_async: bool,
    },
}

impl HookSpec {
    pub fn command(&self) -> &str {
        match self {
            HookSpec::Cmd(c) => c,
            HookSpec::Full { command, url, prompt, .. } => command.as_deref().or(url.as_deref()).or(prompt.as_deref()).unwrap_or(""),
        }
    }
    pub fn kind(&self) -> &'static str {
        match self {
            HookSpec::Cmd(_) => "command",
            HookSpec::Full { command: Some(_), .. } => "command",
            HookSpec::Full { url: Some(_), .. } => "http",
            HookSpec::Full { prompt: Some(_), .. } => "prompt",
            _ => "command",
        }
    }
    pub fn matches(&self, subject: &str) -> bool {
        match self {
            HookSpec::Cmd(_) => true,
            HookSpec::Full { matcher, .. } => matcher.as_deref().map(|m| m.split('|').any(|alt| {
                let alt = alt.trim();
                crate::permissions::glob_match(alt, subject) || regex::Regex::new(alt).map(|r| r.is_match(subject)).unwrap_or(false)
            })).unwrap_or(true),
        }
    }
    /// Seconds this hook may take. A zero/absent config default falls back to 30s — `HooksConfig`
    /// derives Default, which would otherwise make every hook time out immediately.
    pub fn timeout(&self, default: u64) -> u64 {
        let d = if default == 0 { d_timeout() } else { default };
        match self { HookSpec::Full { timeout_secs: Some(t), .. } if *t > 0 => *t, _ => d }
    }
    pub fn is_async(&self) -> bool { matches!(self, HookSpec::Full { is_async: true, .. }) }
    fn once(&self) -> bool { matches!(self, HookSpec::Full { once: true, .. }) }
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct HooksConfig {
    #[serde(default)] pub pre_tool: Vec<HookSpec>,
    #[serde(default)] pub post_tool: Vec<HookSpec>,
    /// A tool call came back as an error.
    #[serde(default)] pub post_tool_failure: Vec<HookSpec>,
    /// The policy is about to ask the user for approval (may decide instead).
    #[serde(default)] pub permission_request: Vec<HookSpec>,
    /// A call was refused (by policy, by the user, or by a hook).
    #[serde(default)] pub permission_denied: Vec<HookSpec>,
    #[serde(default)] pub on_stop: Vec<HookSpec>,
    #[serde(default)] pub on_prompt: Vec<HookSpec>,
    #[serde(default)] pub session_start: Vec<HookSpec>,
    #[serde(default)] pub session_end: Vec<HookSpec>,
    #[serde(default)] pub subagent_stop: Vec<HookSpec>,
    #[serde(default)] pub pre_compact: Vec<HookSpec>,
    #[serde(default)] pub post_compact: Vec<HookSpec>,
    /// Before/after each model call ({messages, tokens} / {text, tokens, secs}).
    #[serde(default)] pub before_model: Vec<HookSpec>,
    #[serde(default)] pub after_model: Vec<HookSpec>,
    /// A file was created or modified by a tool ({path, tool}).
    #[serde(default)] pub file_changed: Vec<HookSpec>,
    #[serde(default)] pub worktree_create: Vec<HookSpec>,
    #[serde(default)] pub notification: Vec<HookSpec>,
    #[serde(default = "d_timeout")] pub timeout_secs: u64,
}
fn d_timeout() -> u64 { 30 }

impl HooksConfig {
    pub fn for_event(&self, event: &str) -> &[HookSpec] {
        match event {
            "pre_tool" => &self.pre_tool,
            "post_tool" => &self.post_tool,
            "post_tool_failure" => &self.post_tool_failure,
            "permission_request" => &self.permission_request,
            "permission_denied" => &self.permission_denied,
            "on_stop" => &self.on_stop,
            "on_prompt" => &self.on_prompt,
            "session_start" => &self.session_start,
            "session_end" => &self.session_end,
            "subagent_stop" => &self.subagent_stop,
            "pre_compact" => &self.pre_compact,
            "post_compact" => &self.post_compact,
            "before_model" => &self.before_model,
            "after_model" => &self.after_model,
            "file_changed" => &self.file_changed,
            "worktree_create" => &self.worktree_create,
            "notification" => &self.notification,
            _ => &[],
        }
    }
    pub fn any(&self, event: &str) -> bool { !self.for_event(event).is_empty() }
    fn slot_mut(&mut self, event: &str) -> Option<&mut Vec<HookSpec>> {
        Some(match event {
            "pre_tool" => &mut self.pre_tool,
            "post_tool" => &mut self.post_tool,
            "post_tool_failure" => &mut self.post_tool_failure,
            "on_stop" => &mut self.on_stop,
            "on_prompt" => &mut self.on_prompt,
            "session_start" => &mut self.session_start,
            "session_end" => &mut self.session_end,
            "subagent_stop" => &mut self.subagent_stop,
            "pre_compact" => &mut self.pre_compact,
            "notification" => &mut self.notification,
            _ => return None,
        })
    }
}

/// What a hook decided.
#[derive(Debug, Clone, Default)]
pub struct Outcome {
    /// "block" | "allow" | "ask" | None (no opinion)
    pub decision: Option<String>,
    pub reason: String,
    /// Rewritten tool arguments (pre_tool only).
    pub updated_input: Option<Value>,
    /// Extra text for the model.
    pub context: String,
}
impl Outcome {
    pub fn blocks(&self) -> bool { self.decision.as_deref() == Some("block") || self.decision.as_deref() == Some("deny") }
    fn from_text(code: i32, text: &str) -> Outcome {
        let mut o = Outcome::default();
        let t = text.trim();
        if let Some(json) = crate::memory::extract_json(t) {
            if let Ok(v) = serde_json::from_str::<Value>(&json) {
                if v.is_object() {
                    o.decision = v["decision"].as_str().map(|s| s.trim().to_lowercase());
                    o.reason = v["reason"].as_str().unwrap_or("").to_string();
                    o.updated_input = v.get("updatedInput").or_else(|| v.get("updated_input")).cloned().filter(|x| x.is_object());
                    o.context = v["context"].as_str().unwrap_or("").to_string();
                    if code == 2 { o.decision = Some("block".into()); }
                    if o.reason.is_empty() && o.blocks() { o.reason = t.to_string(); }
                    return o;
                }
            }
        }
        if code == 2 { o.decision = Some("block".into()); o.reason = t.to_string(); }
        else if code == 0 { o.context = t.to_string(); }
        else { o.reason = t.to_string(); }
        o
    }
}

fn once_fired(key: &str) -> bool {
    static FIRED: Mutex<Option<std::collections::HashSet<String>>> = Mutex::new(None);
    let mut g = FIRED.lock().unwrap();
    let set = g.get_or_insert_with(Default::default);
    !set.insert(key.to_string())
}

async fn run_command(cmd: &str, input: &Value, cwd: &Path, timeout: Duration) -> (i32, String) {
    let (prog, flag) = crate::sandbox::shell_program();
    let mut c = tokio::process::Command::new(prog);
    c.arg(flag).arg(cmd).current_dir(cwd).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).kill_on_drop(true);
    c.env("PATH", crate::setup::path_with_bin_dir(cwd));
    c.env("HARNESS_HOOK_EVENT", input["event"].as_str().unwrap_or(""));
    let Ok(mut child) = c.spawn() else { return (1, format!("hook failed to start: {cmd}")) };
    if let Some(mut stdin) = child.stdin.take() { use tokio::io::AsyncWriteExt; let _ = stdin.write_all(input.to_string().as_bytes()).await; }
    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(o)) => (o.status.code().unwrap_or(1), format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)).trim().to_string()),
        Ok(Err(e)) => (1, e.to_string()),
        Err(_) => (1, format!("hook timed out: {cmd}")),
    }
}

async fn run_http(url: &str, input: &Value, timeout: Duration) -> (i32, String) {
    let client = match reqwest::Client::builder().timeout(timeout).build() { Ok(c) => c, Err(e) => return (1, e.to_string()) };
    match client.post(url).json(input).send().await {
        Ok(r) => {
            let status = r.status();
            let body = r.text().await.unwrap_or_default();
            (if status.is_success() { 0 } else { 1 }, body)
        }
        Err(e) => (1, format!("hook http {url}: {e}")),
    }
}

async fn run_prompt(instruction: &str, input: &Value, client: Option<&crate::llm::Client>) -> (i32, String) {
    let Some(client) = client else { return (1, "prompt hook skipped: no model available in this context".into()) };
    let system = format!(
"You are a policy hook for an autonomous coding agent. Judge the event below against this instruction:\n\n{instruction}\n\nAnswer with JSON only: {{\"decision\": \"allow\"|\"block\"|\"ask\", \"reason\": \"<= 20 words\", \"context\": \"optional note for the agent\"}}. Use \"allow\" when the instruction does not apply. Be conservative: choose \"block\" only when the instruction clearly calls for it.");
    let user = format!("event JSON:\n{}", crate::llm::truncate_for_log(&serde_json::to_string_pretty(input).unwrap_or_default(), 4000));
    match client.role("hook").chat(&[crate::llm::Message::system(system), crate::llm::Message::user(user)], &[]).await {
        Ok((r, _)) => (0, r.text()),
        Err(e) => (1, format!("prompt hook failed: {e:#}")),
    }
}

async fn run_one(h: &HookSpec, event: &str, payload: &Value, cwd: &Path, cfg: &HooksConfig, client: Option<&crate::llm::Client>) -> Outcome {
    let timeout = Duration::from_secs(h.timeout(cfg.timeout_secs));
    let (code, text) = match h {
        HookSpec::Cmd(c) => run_command(c, payload, cwd, timeout).await,
        HookSpec::Full { command: Some(c), .. } => run_command(c, payload, cwd, timeout).await,
        HookSpec::Full { url: Some(u), .. } => run_http(u, payload, timeout).await,
        HookSpec::Full { prompt: Some(p), .. } => run_prompt(p, payload, client).await,
        _ => (1, format!("hook for {event} has no command, url or prompt")),
    };
    Outcome::from_text(code, &text)
}

/// Run every hook registered for `event` whose matcher accepts `subject`.
/// Async hooks are spawned and ignored; the outcomes of the rest are returned in order.
pub async fn run(cfg: &HooksConfig, event: &str, subject: &str, payload: Value, cwd: &Path, client: Option<&crate::llm::Client>) -> Vec<Outcome> {
    let hooks: Vec<HookSpec> = cfg.for_event(event).iter().filter(|h| h.matches(subject)).cloned().collect();
    let mut outs = Vec::new();
    for h in hooks {
        if h.once() && once_fired(&format!("{event}:{}", h.command())) { continue; }
        let mut p = payload.clone();
        p["event"] = json!(event);
        p["subject"] = json!(subject);
        p["workdir"] = json!(cwd.display().to_string());
        if h.is_async() {
            let (cfg2, cwd2, ev) = (cfg.clone(), cwd.to_path_buf(), event.to_string());
            tokio::spawn(async move { let _ = run_one(&h, &ev, &p, &cwd2, &cfg2, None).await; });
            continue;
        }
        outs.push(run_one(&h, event, &p, cwd, cfg, client).await);
    }
    outs
}

/// pre_tool: returns Err(reason) when a hook blocks the call, or Ok(Some(new_args)) when one rewrote them.
pub async fn run_pre_tool_full(cfg: &HooksConfig, tool: &str, args: &str, cwd: &Path, client: Option<&crate::llm::Client>) -> Result<(Option<String>, String), String> {
    let parsed: Value = serde_json::from_str(args).unwrap_or(Value::Null);
    let mut new_args: Option<String> = None;
    let mut context = String::new();
    let mut current = args.to_string();
    for o in run(cfg, "pre_tool", tool, json!({"tool": tool, "args": current, "input": parsed}), cwd, client).await {
        if o.blocks() { return Err(if o.reason.is_empty() { format!("blocked by a pre-tool hook on {tool}") } else { o.reason }); }
        if let Some(u) = o.updated_input { current = u.to_string(); new_args = Some(current.clone()); }
        if !o.context.is_empty() { context.push_str(&o.context); context.push('\n'); }
    }
    Ok((new_args, context))
}

/// Legacy helper: Some(reason) when a pre_tool hook blocks.
pub async fn run_pre_tool(cfg: &HooksConfig, tool: &str, args: &str, cwd: &Path) -> Option<String> {
    run_pre_tool_full(cfg, tool, args, cwd, None).await.err()
}

pub async fn run_post_tool(cfg: &HooksConfig, tool: &str, result: &str, cwd: &Path) {
    let failed = result.starts_with("error:");
    let payload = json!({"tool": tool, "result": crate::llm::truncate_for_log(result, 4000), "is_error": failed});
    let _ = run(cfg, "post_tool", tool, payload.clone(), cwd, None).await;
    if failed && cfg.any("post_tool_failure") { let _ = run(cfg, "post_tool_failure", tool, payload, cwd, None).await; }
}

pub async fn run_on_stop(cfg: &HooksConfig, summary: &str, cwd: &Path) {
    let _ = run(cfg, "on_stop", "", json!({"summary": summary}), cwd, None).await;
}

/// on_prompt: hook output is appended to the user's prompt as extra context.
pub async fn run_on_prompt(cfg: &HooksConfig, prompt: &str, cwd: &Path) -> Option<String> {
    let mut extra = String::new();
    for o in run(cfg, "on_prompt", prompt, json!({"prompt": prompt}), cwd, None).await {
        if !o.context.is_empty() { extra.push_str(&o.context); extra.push('\n'); }
    }
    (!extra.is_empty()).then_some(extra)
}

/// Lifecycle events; returns the non-empty context strings the hooks produced.
pub async fn run_event(cfg: &HooksConfig, event: &str, subject: &str, payload: Value, cwd: &Path) -> Vec<String> {
    run(cfg, event, subject, payload, cwd, None).await.into_iter().filter(|o| !o.context.is_empty()).map(|o| o.context).collect()
}

/// Merge hooks from a Claude Code `settings.json` ({"hooks": {"PreToolUse": [{matcher, hooks:[{type, command}]}]}}).
pub fn import_claude_hooks(cfg: &mut HooksConfig, file: &Path) -> usize {
    let Ok(text) = std::fs::read_to_string(file) else { return 0 };
    let Ok(v) = serde_json::from_str::<Value>(&text) else { return 0 };
    let Some(map) = v["hooks"].as_object() else { return 0 };
    let mut n = 0;
    for (claude_event, entries) in map {
        let event = match claude_event.as_str() {
            "PreToolUse" => "pre_tool",
            "PostToolUse" => "post_tool",
            "PostToolUseFailure" => "post_tool_failure",
            "Stop" => "on_stop",
            "UserPromptSubmit" => "on_prompt",
            "SessionStart" => "session_start",
            "SessionEnd" => "session_end",
            "SubagentStop" => "subagent_stop",
            "PreCompact" => "pre_compact",
            "Notification" => "notification",
            _ => continue,
        };
        for entry in entries.as_array().cloned().unwrap_or_default() {
            let matcher = entry["matcher"].as_str().map(|s| s.to_string());
            for h in entry["hooks"].as_array().cloned().unwrap_or_default() {
                let Some(command) = h["command"].as_str() else { continue };
                let spec = HookSpec::Full { command: Some(command.to_string()), url: None, prompt: None, matcher: matcher.clone(), timeout_secs: h["timeout"].as_u64(), once: false, is_async: false };
                if let Some(slot) = cfg.slot_mut(event) { slot.push(spec); n += 1; }
            }
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_with(pre: Vec<HookSpec>) -> HooksConfig { HooksConfig { pre_tool: pre, ..Default::default() } }

    #[tokio::test]
    async fn command_hook_blocks_and_rewrites() {
        let cwd = std::env::temp_dir();
        // exit 2 blocks
        let cfg = cfg_with(vec![HookSpec::Full { command: Some("echo nope >&2; exit 2".into()), url: None, prompt: None, matcher: Some("bash".into()), timeout_secs: None, once: false, is_async: false }]);
        let r = run_pre_tool_full(&cfg, "bash", "{\"cmd\":\"rm -rf x\"}", &cwd, None).await;
        assert_eq!(r.unwrap_err(), "nope");
        // matcher keeps other tools out
        assert!(run_pre_tool_full(&cfg, "read_file", "{}", &cwd, None).await.is_ok());
        // updatedInput rewrites the arguments
        let cfg = cfg_with(vec![HookSpec::Cmd("echo '{\"decision\":\"allow\",\"updatedInput\":{\"cmd\":\"ls -l\"},\"context\":\"rewritten\"}'".into())]);
        let (args, ctx) = run_pre_tool_full(&cfg, "bash", "{\"cmd\":\"ls\"}", &cwd, None).await.unwrap();
        assert_eq!(args.as_deref(), Some("{\"cmd\":\"ls -l\"}"));
        assert_eq!(ctx.trim(), "rewritten");
    }

    #[tokio::test]
    async fn once_and_matcher_regex() {
        let cwd = std::env::temp_dir();
        let cfg = HooksConfig { session_start: vec![HookSpec::Full { command: Some("echo hi".into()), url: None, prompt: None, matcher: None, timeout_secs: None, once: true, is_async: false }], ..Default::default() };
        assert_eq!(run_event(&cfg, "session_start", "", json!({}), &cwd).await, vec!["hi".to_string()]);
        assert!(run_event(&cfg, "session_start", "", json!({}), &cwd).await.is_empty(), "a `once` hook fires only the first time");
        let h = HookSpec::Full { command: Some("x".into()), url: None, prompt: None, matcher: Some("write_.*|edit_file".into()), timeout_secs: None, once: false, is_async: false };
        assert!(h.matches("write_file") && h.matches("edit_file") && !h.matches("bash"));
    }

    #[test]
    fn imports_claude_settings() {
        let d = std::env::temp_dir().join(format!("harness-hooks-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join("settings.json");
        std::fs::write(&f, r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"./guard.sh"}]}],"Stop":[{"hooks":[{"type":"command","command":"./done.sh"}]}]}}"#).unwrap();
        let mut cfg = HooksConfig::default();
        assert_eq!(import_claude_hooks(&mut cfg, &f), 2);
        assert_eq!(cfg.pre_tool.len(), 1);
        assert!(cfg.pre_tool[0].matches("Bash"));
        assert_eq!(cfg.on_stop.len(), 1);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn outcome_parsing() {
        let o = Outcome::from_text(0, "{\"decision\":\"block\",\"reason\":\"no prod\"}");
        assert!(o.blocks() && o.reason == "no prod");
        let o = Outcome::from_text(2, "plain refusal");
        assert!(o.blocks() && o.reason == "plain refusal");
        let o = Outcome::from_text(0, "just some context");
        assert!(!o.blocks() && o.context == "just some context");
    }
}
