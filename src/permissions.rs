//! Permission policy for tool calls. Modes:
//!   bypass  — everything allowed (deny rules still apply)
//!   auto    — reads free; writes inside the workdir free; risky shell commands and anything outside the
//!             workdir ask; network tools free (default)
//!   ask     — every mutating call asks
//!   plan    — read-only: mutating tools denied, shell limited to inspection commands (ask otherwise)
//! Rules: "<tool>" or "<tool>:<glob>" matched against the call's primary argument
//! (bash → cmd, file tools → path, web/download → url, mcp → tool name). Order: deny, allow, ask, mode default.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Mode { Bypass, #[default] Auto, Ask, Plan }
impl Mode {
    pub fn parse(s: &str) -> Option<Mode> { match s.trim().to_lowercase().as_str() { "bypass" | "yolo" => Some(Mode::Bypass), "auto" | "default" => Some(Mode::Auto), "ask" | "strict" => Some(Mode::Ask), "plan" | "readonly" | "read-only" => Some(Mode::Plan), _ => None } }
    pub fn label(&self) -> &'static str { match self { Mode::Bypass => "bypass permissions on", Mode::Auto => "auto permissions", Mode::Ask => "ask before changes", Mode::Plan => "plan mode (read-only)" } }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PermissionsConfig {
    #[serde(default)] pub mode: Mode,
    #[serde(default)] pub allow: Vec<String>,
    #[serde(default)] pub deny: Vec<String>,
    #[serde(default)] pub ask: Vec<String>,
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
const PLAN_OK: &[&str] = &["git status", "git log", "git diff", "git show", "git branch", "git blame", "ls", "cat ", "head ", "tail ", "grep ", "rg ", "find ", "fd ", "wc ", "tree", "pwd", "echo ", "which ", "file ", "stat ", "du ", "df ", "env", "printenv", "cargo check", "cargo metadata", "cargo tree", "python3 -c \"import", "node -e", "jq ", "sed -n", "awk ", "sort", "uniq", "diff "];
const MUTATING: &[&str] = &["write_file", "edit_file", "apply_patch", "bash", "download_file", "extract_archive", "pdf_edit", "memory", "spawn_agent"];

pub struct Policy { pub cfg: PermissionsConfig, pub workdir: PathBuf, session_allow: std::sync::Mutex<Vec<String>>, mode: std::sync::Mutex<Mode> }

impl Policy {
    pub fn new(cfg: PermissionsConfig, workdir: &Path) -> Self {
        let mode = cfg.mode;
        // project-scoped always-rules are merged in automatically
        let mut session = project_rules(workdir);
        session.retain(|r| !cfg.allow.contains(r));
        Self { cfg, workdir: workdir.to_path_buf(), session_allow: std::sync::Mutex::new(session), mode: std::sync::Mutex::new(mode) }
    }
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
            "bash" => g("cmd"),
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

    fn rule_matches(rule: &str, tool: &str, arg: &str) -> bool {
        let (rt, pat) = match rule.split_once(':') { Some((t, p)) => (t.trim(), Some(p.trim())), None => (rule.trim(), None) };
        if !glob_match(rt, tool) { return false; }
        match pat { None => true, Some(p) => glob_match(p, arg) }
    }

    fn outside_workdir(&self, tool: &str, arg: &str) -> bool {
        if !matches!(tool, "write_file" | "edit_file" | "apply_patch" | "extract_archive" | "pdf_edit" | "download_file") { return false; }
        let p = Path::new(arg);
        if !p.is_absolute() { return false; }
        let root = self.workdir.canonicalize().unwrap_or(self.workdir.clone());
        !p.starts_with(&root)
    }

    pub fn check(&self, tool: &str, args: &Value, read_only_tool: bool) -> Decision {
        let arg = Self::primary_arg(tool, args);
        for r in &self.cfg.deny { if Self::rule_matches(r, tool, &arg) { return Decision::Deny(format!("denied by rule '{r}'")); } }
        for r in self.session_allow.lock().unwrap().iter().chain(self.cfg.allow.iter()) { if Self::rule_matches(r, tool, &arg) { return Decision::Allow; } }
        for r in &self.cfg.ask { if Self::rule_matches(r, tool, &arg) { return Decision::Ask(format!("matches rule '{r}'")); } }
        let mutating = MUTATING.contains(&tool) || tool.starts_with("mcp__") && !read_only_tool;
        match self.mode() {
            Mode::Bypass => Decision::Allow,
            Mode::Plan => {
                if read_only_tool || tool == "load_skill" || tool == "web_search" || tool == "web_fetch" { return Decision::Allow; }
                if tool == "bash" { let a = arg.trim(); if PLAN_OK.iter().any(|ok| a.starts_with(ok)) && !a.contains('>') && !a.contains("&&") && !a.contains(';') { return Decision::Allow; } return Decision::Ask("plan mode: shell command that may modify state".into()); }
                if tool == "memory" { return Decision::Allow; }
                Decision::Deny("plan mode is read-only; switch with /permissions auto".into())
            }
            Mode::Ask => { if read_only_tool || !mutating { Decision::Allow } else { Decision::Ask("ask mode".into()) } }
            Mode::Auto => {
                if read_only_tool || !mutating { return Decision::Allow; }
                if tool == "bash" { let a = arg.to_lowercase(); if let Some(r) = RISKY.iter().find(|r| a.contains(*r)) { return Decision::Ask(format!("risky command pattern '{}'", r.trim())); } return Decision::Allow; }
                if self.outside_workdir(tool, &arg) { return Decision::Ask("writes outside the working directory".into()); }
                Decision::Allow
            }
        }
    }
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
pub fn is_trusted(workdir: &Path) -> bool { let w = workdir.canonicalize().unwrap_or(workdir.to_path_buf()).display().to_string(); std::fs::read_to_string(trusted_file()).ok().and_then(|t| serde_json::from_str::<Vec<String>>(&t).ok()).unwrap_or_default().iter().any(|d| *d == w) }
pub fn trust(workdir: &Path) { let w = workdir.canonicalize().unwrap_or(workdir.to_path_buf()).display().to_string(); let mut v: Vec<String> = std::fs::read_to_string(trusted_file()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default(); if !v.contains(&w) { v.push(w); let _ = std::fs::write(trusted_file(), serde_json::to_string_pretty(&v).unwrap_or_default()); } }

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn pol(mode: Mode) -> Policy { Policy::new(PermissionsConfig { mode, allow: vec!["bash:cargo *".into()], deny: vec!["bash:rm -rf /*".into()], ask: vec![] }, Path::new("/tmp")) }
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
        let by = pol(Mode::Bypass);
        assert!(matches!(by.check("bash", &json!({"cmd":"sudo rm -rf build"}), false), Decision::Allow));
    }
}
