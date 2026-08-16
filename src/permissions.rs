//! Permission policy for tool calls. Modes:
//!   bypass  — everything allowed (deny rules still apply)
//!   auto    — reads free; writes inside the workdir free; risky shell commands and anything outside the
//!             workdir ask; network tools free (default)
//!   ask     — every mutating call asks
//!   plan    — read-only: mutating tools denied, shell limited to inspection commands (ask otherwise)
//! Rules: "<tool>", "<tool>:<glob>" or the Claude-Code form "Tool(<glob>)", matched against the call's
//! primary argument (bash → cmd, file tools → path, web/download → url, mcp → tool name). A pattern of
//! the form "<arg>:<glob>" matches one JSON argument instead — "Agent(subagent_type:review*)" — and
//! "domain:<glob>" matches the host of a URL argument.
//! Order: deny rules, allow rules (config/session/parent), built-in guards (catastrophic shell commands
//! and credential files are refused unless an allow rule covers them), ask rules, mode default.
//! In auto mode a call that would ask can first be decided by an LLM classifier — see `classify`.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Mode { Bypass, #[default] Auto, Ask, Plan }
impl Mode {
    pub fn parse(s: &str) -> Option<Mode> { match s.trim().to_lowercase().as_str() { "bypass" | "yolo" => Some(Mode::Bypass), "auto" | "default" => Some(Mode::Auto), "ask" | "strict" => Some(Mode::Ask), "plan" | "readonly" | "read-only" => Some(Mode::Plan), _ => None } }
    /// Stable id used in protocols (ACP session modes, settings files).
    pub fn id(&self) -> &'static str { match self { Mode::Bypass => "bypass", Mode::Auto => "auto", Mode::Ask => "ask", Mode::Plan => "plan" } }
    pub fn label(&self) -> &'static str { match self { Mode::Bypass => "bypass permissions on", Mode::Auto => "auto permissions", Mode::Ask => "ask before changes", Mode::Plan => "plan mode (read-only)" } }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PermissionsConfig {
    #[serde(default)] pub mode: Mode,
    #[serde(default)] pub allow: Vec<String>,
    #[serde(default)] pub deny: Vec<String>,
    #[serde(default)] pub ask: Vec<String>,
    /// Auto mode: what the LLM classifier should wave through, ask about, or refuse.
    #[serde(default)] pub auto: AutoConfig,
}

/// `[permissions.auto]` — the classifier that decides borderline calls in auto mode instead of
/// stopping the run with a prompt. Instructions are plain English, one rule per entry.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AutoConfig {
    /// None = on when an aux model is configured, off otherwise. false disables it entirely.
    #[serde(default)] pub classifier: Option<bool>,
    /// "safe, reversible things in the project" …
    #[serde(default)] pub allow: Vec<String>,
    /// Always stop and ask about these, whatever the classifier thinks.
    #[serde(default)] pub ask: Vec<String>,
    /// Refuse outright.
    #[serde(default)] pub deny: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum Decision { Allow, Deny(String), Ask(String) }

#[derive(Debug, Clone)]
pub struct ApprovalRequest { pub tool: String, pub summary: String, pub suggested_rule: String, pub reason: String }
#[derive(Debug, Clone)]
pub enum Approval { Once, Always, AlwaysProject, Deny }

/// A question the model asks the user (ask_user tool): multiple choice and/or free text.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Question { pub question: String, pub options: Vec<QuestionOption>, pub allow_free_text: bool, pub timeout_secs: Option<u64> }
#[derive(Debug, Clone, serde::Serialize)]
pub struct QuestionOption { pub label: String, pub description: String }
/// How the user answered. `choice` is 0-based into `options`; `text` is free text (or notes).
#[derive(Debug, Clone, Default)]
pub struct Answer { pub choice: Option<usize>, pub text: Option<String>, pub declined: bool, pub timed_out: bool }

#[async_trait::async_trait]
pub trait Approver: Send + Sync {
    async fn ask(&self, req: ApprovalRequest) -> Approval;
    /// Ask the user a question; `None` = no user is available (headless run) — the model must decide itself.
    async fn question(&self, _q: Question) -> Option<Answer> { None }
    /// The user's answers may be prompted for interactively.
    fn interactive(&self) -> bool { false }
}
/// Non-interactive: deny (unless `yes`).
pub struct AutoApprover { pub yes: bool }
#[async_trait::async_trait]
impl Approver for AutoApprover { async fn ask(&self, _req: ApprovalRequest) -> Approval { if self.yes { Approval::Once } else { Approval::Deny } } }

const RISKY: &[&str] = &[
    "rm -rf", "rm -r ", "rm -fr", "sudo ", "su ", "git push --force", "git push -f", "git reset --hard", "git clean -f", "git checkout -- .", "git restore .",
    "chmod -R", "chown -R", "mkfs", "dd if=", "diskutil", ":(){", "shutdown", "reboot", "killall", "pkill ", "kill -9",
    "| sh", "| bash", "|sh", "|bash", "curl ", "wget ", "brew uninstall", "brew remove", "npm install -g", "npm i -g", "pip install", "pip3 install", "cargo install",
    "> ~/", "> /etc", "> /usr", "> /System", "launchctl", "defaults write", "osascript", "crontab", "ssh ", "scp ", "rsync ", "docker ", "kubectl ", "terraform ", "aws ", "gcloud ",
    "git commit --amend", "git rebase", "git branch -D", "git tag -d", "gh pr merge", "gh release",
    "del /s", "del /q", "rmdir /s", "rd /s", "format ", "remove-item -recurse", "rm -recurse", "diskpart", "reg delete", "schtasks", "net user",
];
/// Never allowed outside bypass mode: destroys data outside the project or the machine itself.
const CATASTROPHIC: &[&str] = &["rm -rf /", "rm -rf /*", "rm -fr /", "rm -rf ~", "rm -rf $HOME", "rm -rf .. ", ":(){", "mkfs", "dd if=/dev/zero of=/dev/", "dd if=/dev/random of=/dev/", "> /dev/sda", "chmod -R 777 /", "chown -R / ", "shutdown -h", "diskutil eraseDisk", "format c:", "del /f /s /q c:\\"];
/// Files never read into the model's context (an explicit allow rule still wins).
const SENSITIVE: &[&str] = &["**/.env", "**/.env.*", "**/id_rsa*", "**/id_ed25519*", "**/.ssh/**", "**/*.pem", "**/*.key", "**/.netrc", "**/.npmrc", "**/.pypirc", "**/.aws/credentials", "**/.config/gh/hosts.yml", "**/credentials.json", "**/service-account*.json"];
/// …except these, which are meant to be shared.
const SENSITIVE_OK: &[&str] = &["**/.env.example", "**/.env.sample", "**/.env.template", "**/.env.defaults", "**/*.pub"];
/// Claude-Code / Codex tool names accepted in rules, mapped to ours.
const TOOL_ALIASES: &[(&str, &str)] = &[("bash", "bash"), ("shell", "bash"), ("read", "read_file"), ("write", "write_file"), ("edit", "edit_file"), ("multiedit", "edit_file"), ("ls", "list_dir"), ("glob", "glob"), ("grep", "grep"), ("webfetch", "web_fetch"), ("websearch", "web_search"), ("task", "spawn_agent"), ("agent", "spawn_agent"), ("todowrite", "todo"), ("notebookedit", "notebook_edit"), ("askuserquestion", "ask_user")];

const PLAN_OK: &[&str] = &["git status", "git log", "git diff", "git show", "git branch", "git blame", "ls", "cat ", "head ", "tail ", "grep ", "rg ", "find ", "fd ", "wc ", "tree", "pwd", "echo ", "which ", "file ", "stat ", "du ", "df ", "env", "printenv", "cargo check", "cargo metadata", "cargo tree", "python3 -c \"import", "node -e", "jq ", "sed -n", "awk ", "sort", "uniq", "diff "];
/// Tools that are not read-only per the registry but never touch files/state outside the harness itself.
const BENIGN: &[&str] = &["todo", "ask_user", "notify"];
/// Tools whose primary argument is a shell command (RISKY-pattern checked in auto mode).
const SHELL_TOOLS: &[&str] = &["bash", "monitor", "run_workflow"];

pub struct Policy { pub cfg: PermissionsConfig, pub workdir: PathBuf, session_allow: std::sync::Mutex<Vec<String>>, mode: std::sync::Mutex<Mode>, /// parent policy (sub-agents): if the parent is in bypass, so are we
    pub parent: Option<std::sync::Arc<Policy>> }

impl PermissionsConfig {
    /// The current mode's protocol id (ACP `session/new` → modes.currentModeId).
    pub fn mode_id(&self) -> &'static str { self.mode.id() }
}

impl Policy {
    pub fn new(cfg: PermissionsConfig, workdir: &Path) -> Self {
        let mode = cfg.mode;
        // project-scoped always-rules are merged in automatically
        let mut session = project_rules(workdir);
        session.retain(|r| !cfg.allow.contains(r));
        Self { cfg, workdir: workdir.to_path_buf(), session_allow: std::sync::Mutex::new(session), mode: std::sync::Mutex::new(mode), parent: None }
    }
    /// A child policy (sub-agent): its own mode, but the parent's live bypass/allow-rules always apply.
    pub fn child_of(parent: std::sync::Arc<Policy>, cfg: PermissionsConfig, workdir: &Path) -> Self { let mut p = Self::new(cfg, workdir); p.parent = Some(parent); p }
    fn effective_mode(&self) -> Mode { if let Some(p) = &self.parent { if p.mode() == Mode::Bypass { return Mode::Bypass; } } self.mode() }
    pub fn session_rules(&self) -> Vec<String> { self.session_allow.lock().unwrap().clone() }
    /// Current mode (live: `set_mode` takes effect for running sessions too).
    pub fn mode(&self) -> Mode { *self.mode.lock().unwrap() }
    pub fn set_mode(&self, m: Mode) { *self.mode.lock().unwrap() = m; }
    pub fn allow_always(&self, rule: &str) { self.session_allow.lock().unwrap().push(rule.to_string()); persist_rule(rule); }
    /// Persist an allow rule for this project only (<workdir>/.harness/permissions.json).
    pub fn allow_always_project(&self, rule: &str) { self.session_allow.lock().unwrap().push(rule.to_string()); persist_project_rule(&self.workdir, rule); }
    pub fn remove_rule(&self, rule: &str) -> usize { let mut n = 0; { let mut v = self.session_allow.lock().unwrap(); let before = v.len(); v.retain(|r| r != rule); n += before - v.len(); } n += remove_persisted(rule, None); n += remove_persisted(rule, Some(&self.workdir)); n }

    /// Primary argument used for rule matching and the human-readable summary.
    pub fn primary_arg(tool: &str, args: &Value) -> String {
        let g = |k: &str| args.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
        match tool {
            "bash" | "monitor" => g("cmd"),
            "run_workflow" => format!("{} {}", g("name"), g("args")),
            "read_file" | "write_file" | "edit_file" | "list_dir" | "view_image" | "read_pdf" | "pdf_edit" | "extract_archive" | "apply_patch" => g("path"),
            "web_fetch" | "download_file" => g("url"),
            "web_search" => g("query"),
            "memory" => format!("{} {}", g("action"), g("file")),
            _ => args.to_string(),
        }
    }

    pub fn suggested_rule(tool: &str, arg: &str) -> String {
        match tool {
            "bash" => { let words: Vec<&str> = arg.split_whitespace().collect(); if words.len() >= 2 && matches!(words[0], "git" | "cargo" | "npm" | "pnpm" | "yarn" | "python3" | "python" | "pip" | "brew" | "docker" | "make" | "go" | "gh") { format!("bash:{} {} *", words[0], words[1]) } else if let Some(w) = words.first() { format!("bash:{w} *") } else { "bash".into() } }
            _ => tool.to_string(),
        }
    }

    /// Rules accept `tool`, `tool:<glob>` and the Claude-Code form `Tool(<glob>)`; the pattern may be
    /// `<arg>:<glob>` to match one JSON argument (`Agent(model:*opus*)`) or `domain:<glob>` for URLs.
    pub fn parse_rule(rule: &str) -> (String, Option<String>) {
        let r = rule.trim();
        if let Some(open) = r.find('(') {
            if r.ends_with(')') {
                let tool = r[..open].trim();
                let pat = r[open + 1..r.len() - 1].trim();
                return (canonical_tool(tool), (!pat.is_empty()).then(|| pat.to_string()));
            }
        }
        match r.split_once(':') {
            // mcp__server__tool has no pattern part
            Some((t, p)) if !t.is_empty() && !r.starts_with("mcp__") => (canonical_tool(t), Some(p.trim().to_string())),
            _ => (canonical_tool(r), None),
        }
    }

    fn rule_matches_args(rule: &str, tool: &str, arg: &str, args: &Value) -> bool {
        let (rt, pat) = Self::parse_rule(rule);
        if !glob_match(&rt, tool) { return false; }
        let Some(p) = pat else { return true };
        // `<arg>:<glob>` — match a named JSON argument (domain: matches the host of a URL argument)
        if let Some((key, val)) = p.split_once(':') {
            let (key, val) = (key.trim(), val.trim());
            if key == "domain" { return url_host(arg).map(|h| glob_match(val, &h) || h.ends_with(&format!(".{val}"))).unwrap_or(false); }
            if args.get(key).is_some() {
                let a = args.get(key).map(|v| match v { Value::String(s) => s.clone(), other => other.to_string() }).unwrap_or_default();
                return glob_match(val, &a);
            }
        }
        glob_match(&p, arg)
    }

    #[cfg(test)]
    fn rule_matches(rule: &str, tool: &str, arg: &str) -> bool { Self::rule_matches_args(rule, tool, arg, &Value::Null) }

    fn outside_workdir(&self, tool: &str, arg: &str) -> bool {
        if !matches!(tool, "write_file" | "edit_file" | "apply_patch" | "extract_archive" | "pdf_edit" | "download_file") { return false; }
        let p = Path::new(arg);
        if !p.is_absolute() { return false; }
        let root = self.workdir.canonicalize().unwrap_or(self.workdir.clone());
        !p.starts_with(&root)
    }

    pub fn check(&self, tool: &str, args: &Value, read_only_tool: bool) -> Decision {
        let arg = Self::primary_arg(tool, args);
        for r in &self.cfg.deny { if Self::rule_matches_args(r, tool, &arg, args) { return Decision::Deny(format!("denied by rule '{r}'")); } }
        // an explicit allow rule (config, session or parent) wins over the built-in guards below
        let explicit_allow = self.session_allow.lock().unwrap().iter().chain(self.cfg.allow.iter()).any(|r| Self::rule_matches_args(r, tool, &arg, args))
            || self.parent.as_ref().map(|p| p.session_rules().iter().any(|r| Self::rule_matches_args(r, tool, &arg, args))).unwrap_or(false);
        if explicit_allow { return Decision::Allow; }
        if self.effective_mode() != Mode::Bypass {
            if SHELL_TOOLS.contains(&tool) {
                let a = arg.to_lowercase();
                if let Some(r) = CATASTROPHIC.iter().find(|r| a.contains(*r)) { return Decision::Deny(format!("refused: '{}' destroys data outside this project (allow it explicitly in [permissions].allow if you really mean it)", r.trim())); }
            }
            if let Some(p) = secret_file(tool, &arg) { return Decision::Deny(format!("{p} holds credentials — it is never read into the model's context (add an allow rule to override)")); }
        }
        for r in &self.cfg.ask { if Self::rule_matches_args(r, tool, &arg, args) { return Decision::Ask(format!("matches rule '{r}'")); } }
        // mutating iff the registry does not declare the tool read-only (benign UI tools excepted)
        let mutating = !read_only_tool && !BENIGN.contains(&tool);
        match self.effective_mode() {
            Mode::Bypass => Decision::Allow,
            Mode::Plan => {
                if read_only_tool || tool == "load_skill" || tool == "web_search" || tool == "web_fetch" { return Decision::Allow; }
                if tool == "bash" { if plan_safe_shell(&arg) { return Decision::Allow; } return Decision::Ask("plan mode: shell command that may modify state".into()); }
                if tool == "memory" { return Decision::Allow; }
                Decision::Deny("plan mode is read-only; switch with /permissions auto".into())
            }
            Mode::Ask => { if read_only_tool || !mutating { Decision::Allow } else { Decision::Ask("ask mode".into()) } }
            Mode::Auto => {
                if read_only_tool || !mutating { return Decision::Allow; }
                if SHELL_TOOLS.contains(&tool) { let a = arg.to_lowercase(); if let Some(r) = RISKY.iter().find(|r| a.contains(*r)) { return Decision::Ask(format!("risky command pattern '{}'", r.trim())); } return Decision::Allow; }
                if self.outside_workdir(tool, &arg) { return Decision::Ask("writes outside the working directory".into()); }
                Decision::Allow
            }
        }
    }

    /// Auto mode's LLM classifier: decide a borderline call instead of interrupting the user.
    /// Returns None when the classifier is off or unavailable — the caller then asks the user
    /// (fail-closed: an unparseable or hedging answer never means "allow").
    pub async fn classify(&self, client: &crate::llm::Client, tool: &str, args: &Value, reason: &str) -> Option<(Decision, String)> {
        if self.effective_mode() != Mode::Auto { return None; }
        let a = &self.cfg.auto;
        if a.classifier == Some(false) { return None; }
        if a.classifier.is_none() && !client.has_aux() { return None; }
        let arg = Self::primary_arg(tool, args);
        let mut instr = String::new();
        for x in &a.deny { instr.push_str(&format!("- DENY: {x}\n")); }
        for x in &a.ask { instr.push_str(&format!("- ASK: {x}\n")); }
        for x in &a.allow { instr.push_str(&format!("- ALLOW: {x}\n")); }
        if instr.is_empty() { instr.push_str("- ALLOW: reversible work inside the project (builds, tests, formatters, package scripts, git commands that do not rewrite history or push)\n- ASK: anything that leaves the project directory, touches the network with side effects, installs software globally, or deletes data\n- DENY: anything that exfiltrates secrets or damages the machine\n"); }
        let system = format!(
"You are the permission classifier of a local coding agent. The agent works autonomously in {}; the user is not watching. Decide whether this tool call may run.\nHouse rules (top to bottom, later ones do not override DENY):\n{instr}\nAnswer with JSON only: {{\"decision\": \"allow\"|\"ask\"|\"deny\", \"reason\": \"<= 15 words\"}}.\nRules: choose \"ask\" whenever you are unsure — a wrong \"allow\" is far worse than a needless question. Never allow: deleting or overwriting data outside the project, writing to system paths, credentials/secret exfiltration, `sudo`, disabling tests or safety checks to make something pass, force-pushing, or destroying git history.",
            self.workdir.display());
        let user = format!("tool: {tool}\nprimary argument: {}\nfull arguments: {}\nwhy the heuristic hesitated: {reason}\n\nJSON:", crate::llm::truncate_for_log(&arg, 400), crate::llm::truncate_for_log(&args.to_string(), 1200));
        let req = vec![crate::llm::Message::system(system), crate::llm::Message::user(user)];
        let (reply, _) = client.aux().chat(&req, &[]).await.ok()?;
        let text = reply.text();
        let v: Value = serde_json::from_str(crate::memory::extract_json(&text)?.trim()).ok()?;
        let why = v["reason"].as_str().unwrap_or("").trim().to_string();
        match v["decision"].as_str().map(|d| d.trim().to_lowercase()).as_deref() {
            Some("allow") => Some((Decision::Allow, why)),
            Some("deny") => Some((Decision::Deny(format!("classifier: {why}")), why)),
            _ => None,
        }
    }

}

/// Plan mode: a shell line is safe if every segment (split on ; && || |) starts with an inspection command,
/// there is no output redirection, and no obviously mutating word.
pub fn plan_safe_shell(cmd: &str) -> bool {
    let a = cmd.trim();
    if a.contains('>') || a.contains("<<") { return false; }
    let lower = a.to_lowercase();
    if ["rm ", "mv ", "cp ", "mkdir", "touch", "chmod", "chown", "git commit", "git push", "git checkout", "git reset", "git add", "git rm", "npm i", "npm install", "pip install", "cargo build", "cargo run", "cargo test", "make", "sed -i", "tee ", "install ", "curl ", "wget ", "python3 -c", "python -c", "node -e", "eval "].iter().any(|w| lower.contains(w)) { return false; }
    let segments: Vec<&str> = a.split(|c| c == ';' || c == '|' || c == '&' || c == '\n').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
    if segments.is_empty() { return false; }
    segments.iter().all(|seg| { let seg = seg.trim_start_matches("cd ").trim(); PLAN_OK.iter().any(|ok| seg.starts_with(ok)) || seg.starts_with("cd ") || ["xargs", "cut ", "tr ", "head", "tail", "wc", "grep", "rg", "sort", "uniq", "awk", "sed -n", "cat", "less", "more", "basename", "dirname", "realpath", "test ", "[ ", "true", "false", "printf", "date", "whoami", "id", "uname", "sw_vers", "sysctl", "nproc", "column", "paste", "comm", "diff", "od", "xxd", "hexdump", "strings", "file", "stat", "ls", "find", "fd", "git ", "cargo metadata", "cargo tree", "cargo check", "python3 --version", "node --version", "which", "type ", "command -v", "env", "echo"].iter().any(|ok| seg.starts_with(ok)) })
}

/// Simple glob: `*` matches any sequence, `?` one char; case-sensitive.
pub fn glob_match(pat: &str, text: &str) -> bool {
    fn rec(p: &[char], t: &[char]) -> bool {
        match (p.first(), t.first()) {
            (None, None) => true,
            (Some('*'), _) => rec(&p[1..], t) || (!t.is_empty() && rec(p, &t[1..])),
            (Some('?'), Some(_)) => rec(&p[1..], &t[1..]),
            (Some(a), Some(b)) if a == b => rec(&p[1..], &t[1..]),
            _ => false,
        }
    }
    let p: Vec<char> = pat.chars().collect(); let t: Vec<char> = text.chars().collect();
    rec(&p, &t)
}

/// Rule tool names are case-insensitive and accept the Claude-Code spelling (`Bash`, `WebFetch`, …).
fn canonical_tool(t: &str) -> String {
    let k = t.trim().to_lowercase();
    TOOL_ALIASES.iter().find(|(a, _)| *a == k).map(|(_, c)| c.to_string()).unwrap_or(k)
}

fn url_host(u: &str) -> Option<String> {
    let rest = u.split_once("://").map(|(_, r)| r).unwrap_or(u);
    let host = rest.split(['/', '?', '#']).next()?.split('@').next_back()?.split(':').next()?;
    (!host.is_empty()).then(|| host.to_lowercase())
}

/// The path a call would read, if it is a credentials file the model should never see.
fn secret_file(tool: &str, arg: &str) -> Option<String> {
    if !matches!(tool, "read_file" | "view_image" | "read_pdf" | "notebook_edit" | "edit_file" | "write_file" | "apply_patch") { return None; }
    let p = arg.trim();
    if p.is_empty() { return None; }
    let norm = p.replace('\\', "/");
    if SENSITIVE_OK.iter().any(|g| crate::instructions::glob_path(g, &norm)) { return None; }
    SENSITIVE.iter().any(|g| crate::instructions::glob_path(g, &norm)).then(|| norm)
}

fn rules_file() -> PathBuf { crate::setup::config_dir().join("permissions.json") }
/// "Always allow" rules persist across sessions.
pub fn persisted_rules() -> Vec<String> {
    std::fs::read_to_string(rules_file()).ok().and_then(|t| serde_json::from_str::<Vec<String>>(&t).ok()).unwrap_or_default()
}
fn persist_rule(rule: &str) {
    let mut v = persisted_rules();
    if !v.iter().any(|r| r == rule) { v.push(rule.to_string()); let _ = std::fs::write(rules_file(), serde_json::to_string_pretty(&v).unwrap_or_default()); }
}
fn project_rules_file(workdir: &Path) -> PathBuf { workdir.join(".harness").join("permissions.json") }
pub fn project_rules(workdir: &Path) -> Vec<String> { std::fs::read_to_string(project_rules_file(workdir)).ok().and_then(|t| serde_json::from_str::<Vec<String>>(&t).ok()).unwrap_or_default() }
fn persist_project_rule(workdir: &Path, rule: &str) {
    let mut v = project_rules(workdir);
    if !v.iter().any(|r| r == rule) { v.push(rule.to_string()); let _ = std::fs::create_dir_all(workdir.join(".harness")); let _ = std::fs::write(project_rules_file(workdir), serde_json::to_string_pretty(&v).unwrap_or_default()); }
}
fn remove_persisted(rule: &str, workdir: Option<&Path>) -> usize {
    let (file, mut v) = match workdir { Some(w) => (project_rules_file(w), project_rules(w)), None => (rules_file(), persisted_rules()) };
    let before = v.len(); v.retain(|r| r != rule);
    if v.len() != before { let _ = std::fs::write(file, serde_json::to_string_pretty(&v).unwrap_or_default()); }
    before - v.len()
}

/// Directory trust: remembered directories the user has accepted working in (non-blocking notice otherwise).
fn trusted_file() -> PathBuf { crate::setup::config_dir().join("trusted.json") }
pub fn is_trusted(workdir: &Path) -> bool { let w = workdir.canonicalize().unwrap_or(workdir.to_path_buf()).display().to_string(); std::fs::read_to_string(trusted_file()).ok().and_then(|t| serde_json::from_str::<Vec<String>>(&t).ok()).unwrap_or_default().contains(&w) }
pub fn trust(workdir: &Path) { let w = workdir.canonicalize().unwrap_or(workdir.to_path_buf()).display().to_string(); let mut v: Vec<String> = std::fs::read_to_string(trusted_file()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default(); if !v.contains(&w) { v.push(w); let _ = std::fs::write(trusted_file(), serde_json::to_string_pretty(&v).unwrap_or_default()); } }

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn pol(mode: Mode) -> Policy { Policy::new(PermissionsConfig { mode, allow: vec!["bash:cargo *".into()], deny: vec!["bash:rm -rf /*".into()], ..Default::default() }, Path::new("/tmp")) }
    #[test]
    fn globs() { assert!(glob_match("bash:git *", "bash:git status")); assert!(glob_match("*", "anything")); assert!(!glob_match("git *", "cargo test")); }
    #[test]
    fn modes() {
        let p = pol(Mode::Auto);
        assert!(matches!(p.check("read_file", &json!({"path":"x"}), true), Decision::Allow));
        assert!(matches!(p.check("bash", &json!({"cmd":"ls -la"}), false), Decision::Allow));
        assert!(matches!(p.check("bash", &json!({"cmd":"rm -rf build"}), false), Decision::Ask(_)));
        assert!(matches!(p.check("bash", &json!({"cmd":"rm -rf /"}), false), Decision::Deny(_)));
        assert!(matches!(p.check("bash", &json!({"cmd":"cargo install foo"}), false), Decision::Allow)); // allow rule wins over risky
        assert!(matches!(p.check("write_file", &json!({"path":"/etc/hosts"}), false), Decision::Ask(_)));
        let plan = pol(Mode::Plan);
        assert!(matches!(plan.check("write_file", &json!({"path":"a"}), false), Decision::Deny(_)));
        assert!(matches!(plan.check("bash", &json!({"cmd":"git status"}), false), Decision::Allow));
        assert!(matches!(plan.check("bash", &json!({"cmd":"touch x"}), false), Decision::Ask(_)));
        assert!(matches!(plan.check("bash", &json!({"cmd":"ls -l /tmp | head -30; sed -n 5,10p a.rs"}), false), Decision::Allow));
        assert!(matches!(plan.check("bash", &json!({"cmd":"ls > out.txt"}), false), Decision::Ask(_)));
        let child = Policy::child_of(std::sync::Arc::new(pol(Mode::Bypass)), PermissionsConfig { mode: Mode::Plan, ..Default::default() }, Path::new("/tmp"));
        assert!(matches!(child.check("write_file", &json!({"path":"a"}), false), Decision::Allow), "parent bypass wins");
        let by = pol(Mode::Bypass);
        assert!(matches!(by.check("bash", &json!({"cmd":"sudo rm -rf build"}), false), Decision::Allow));
    }
    #[test]
    fn rule_syntax_and_guards() {
        // Claude-Code spelling, argument matchers and domain rules
        assert!(Policy::rule_matches("Bash(git * main)", "bash", "git push main"));
        assert!(Policy::rule_matches("Read(src/**)", "read_file", "src/a/b.rs"));
        assert!(Policy::rule_matches_args("WebFetch(domain:example.com)", "web_fetch", "https://api.example.com/x", &json!({"url":"https://api.example.com/x"})));
        assert!(!Policy::rule_matches_args("WebFetch(domain:example.com)", "web_fetch", "https://evil.com/x", &json!({"url":"https://evil.com/x"})));
        assert!(Policy::rule_matches_args("Agent(subagent_type:review*)", "spawn_agent", "", &json!({"subagent_type":"reviewer"})));
        assert!(Policy::rule_matches("mcp__chrome-devtools__*", "mcp__chrome-devtools__click", ""));

        let p = pol(Mode::Auto);
        // credentials are never read, in any mode but bypass
        assert!(matches!(p.check("read_file", &json!({"path":".env"}), true), Decision::Deny(_)));
        assert!(matches!(p.check("read_file", &json!({"path":"app/.env.production"}), true), Decision::Deny(_)));
        assert!(matches!(p.check("read_file", &json!({"path":".env.example"}), true), Decision::Allow));
        assert!(matches!(p.check("read_file", &json!({"path":"/home/u/.ssh/id_rsa"}), true), Decision::Deny(_)));
        assert!(matches!(pol(Mode::Bypass).check("read_file", &json!({"path":".env"}), true), Decision::Allow));
        let allowed = Policy::new(PermissionsConfig { mode: Mode::Auto, allow: vec!["Read(.env)".into()], ..Default::default() }, Path::new("/tmp"));
        assert!(matches!(allowed.check("read_file", &json!({"path":".env"}), true), Decision::Allow), "an explicit allow rule overrides the guard");

        // catastrophic commands are refused outright, not merely asked about
        assert!(matches!(p.check("bash", &json!({"cmd":"rm -rf ~"}), false), Decision::Deny(_)));
        assert!(matches!(p.check("bash", &json!({"cmd":"sudo mkfs.ext4 /dev/sda1"}), false), Decision::Deny(_)));
        assert!(matches!(p.check("bash", &json!({"cmd":"rm -rf build"}), false), Decision::Ask(_)));
        assert!(matches!(pol(Mode::Bypass).check("bash", &json!({"cmd":"rm -rf ~"}), false), Decision::Allow));
    }

    #[test]
    fn non_read_only_tools_are_mutating() {
        let p = pol(Mode::Auto);
        assert!(matches!(p.check("monitor", &json!({"cmd":"rm -rf x"}), false), Decision::Ask(_)));
        assert!(matches!(p.check("monitor", &json!({"cmd":"tail -f log"}), false), Decision::Allow));
        assert!(matches!(p.check("todo", &json!({"action":"list"}), false), Decision::Allow));
        let ask = pol(Mode::Ask);
        assert!(matches!(ask.check("monitor", &json!({"cmd":"tail -f log"}), false), Decision::Ask(_)));
        assert!(matches!(ask.check("worktree", &json!({"action":"enter"}), false), Decision::Ask(_)));
        assert!(matches!(ask.check("glob", &json!({"pattern":"*.rs"}), true), Decision::Allow));
    }
}
