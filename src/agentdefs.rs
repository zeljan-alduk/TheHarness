//! Named custom sub-agents defined in markdown, the way Claude Code / Cursor / Copilot / OpenCode do it.
//!
//! `<project>/.harness/agents/*.md` (also `.claude/agents`, `.agents/agents`, `.cursor/agents`) and the
//! same directories under `~`. Frontmatter: `name`, `description`, `tools` (allow-list, `*` = all),
//! `model`, `effort`, `permission-mode` (bypass|auto|ask|plan), `isolation` (none|worktree),
//! `max-turns`. The body is the agent's system prompt.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct AgentDef {
    pub name: String,
    pub description: String,
    pub path: PathBuf,
    pub source: String,
    /// Allowed tool names; empty or `["*"]` = inherit everything the parent has.
    pub tools: Vec<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub permission_mode: Option<crate::permissions::Mode>,
    pub isolation: Option<String>,
    pub max_turns: Option<usize>,
    /// System prompt body (markdown after the frontmatter).
    pub prompt: String,
}

impl AgentDef {
    /// Tools this agent may use, given what the parent has.
    pub fn filter_tools(&self, available: &[&'static str]) -> Vec<String> {
        if self.tools.is_empty() || self.tools.iter().any(|t| t == "*") { return available.iter().map(|s| s.to_string()).collect(); }
        available.iter().filter(|a| self.tools.iter().any(|t| crate::permissions::glob_match(t, a))).map(|s| s.to_string()).collect()
    }
}

pub fn dirs(workdir: &Path) -> Vec<(PathBuf, &'static str)> {
    let mut v: Vec<(PathBuf, &'static str)> = vec![
        (workdir.join(".harness").join("agents"), "project"),
        (workdir.join(".claude").join("agents"), "project"),
        (workdir.join(".agents").join("agents"), "project"),
        (workdir.join(".cursor").join("agents"), "project"),
    ];
    v.push((crate::setup::config_dir().join("agents"), "user"));
    if let Some(h) = home() {
        v.push((h.join(".claude").join("agents"), "user"));
        v.push((h.join(".agents").join("agents"), "user"));
    }
    v
}

pub fn discover(workdir: &Path) -> Vec<AgentDef> {
    let mut out: Vec<AgentDef> = Vec::new();
    for (dir, source) in dirs(workdir) {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        let mut files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().map(|e| e == "md").unwrap_or(false)).collect();
        files.sort();
        for f in files {
            if let Some(a) = parse(&f, source) {
                if !out.iter().any(|x| x.name.eq_ignore_ascii_case(&a.name)) { out.push(a); }
            }
        }
    }
    out
}

pub fn find(workdir: &Path, name: &str) -> Option<AgentDef> {
    let n = name.trim().to_lowercase();
    discover(workdir).into_iter().find(|a| a.name.to_lowercase() == n || a.name.to_lowercase().replace(' ', "-") == n.replace(' ', "-"))
}

fn parse(file: &Path, source: &str) -> Option<AgentDef> {
    let text = std::fs::read_to_string(file).ok()?;
    let (fm, body) = crate::instructions::split_frontmatter(&text);
    let get = |k: &str| fm.iter().find(|(a, _)| a.eq_ignore_ascii_case(k)).map(|(_, v)| v.clone());
    let name = get("name").unwrap_or_else(|| file.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default());
    if name.trim().is_empty() || body.trim().is_empty() { return None; }
    Some(AgentDef {
        name: name.trim().to_string(),
        description: get("description").unwrap_or_else(|| body.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string()),
        path: file.to_path_buf(),
        source: source.to_string(),
        tools: get("tools").map(|v| crate::instructions::split_list(&v)).unwrap_or_default(),
        model: get("model"),
        effort: get("effort"),
        permission_mode: get("permission-mode").or_else(|| get("permission_mode")).and_then(|v| crate::permissions::Mode::parse(&v)),
        isolation: get("isolation"),
        max_turns: get("max-turns").or_else(|| get("max_turns")).and_then(|v| v.trim().parse().ok()),
        prompt: body.trim().to_string(),
    })
}

/// System-prompt listing so the model knows which `subagent_type` values exist.
pub fn prompt_block(workdir: &Path) -> String {
    let defs = discover(workdir);
    if defs.is_empty() { return String::new(); }
    let mut s = String::from("\n\n# Custom agents\nPass one of these as `subagent_type` to spawn_agent to delegate with that agent's own prompt, tools and model:\n");
    for a in defs { s.push_str(&format!("- {} — {} [{}]\n", a.name, crate::llm::truncate_for_log(&a.description, 160), a.source)); }
    s
}

fn home() -> Option<PathBuf> { std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from) }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_agent_files() {
        let d = std::env::temp_dir().join(format!("harness-agentdefs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join(".harness/agents")).unwrap();
        std::fs::write(d.join(".harness/agents/reviewer.md"), "---\nname: reviewer\ndescription: Reviews diffs\ntools: read_file, grep, glob\npermission-mode: plan\nmax-turns: 12\n---\nYou review code.\n").unwrap();
        let a = find(&d, "reviewer").unwrap();
        assert_eq!(a.tools, vec!["read_file", "grep", "glob"]);
        assert_eq!(a.permission_mode, Some(crate::permissions::Mode::Plan));
        assert_eq!(a.max_turns, Some(12));
        assert_eq!(a.prompt, "You review code.");
        assert_eq!(a.filter_tools(&["read_file", "bash", "grep"]), vec!["read_file", "grep"]);
        assert!(prompt_block(&d).contains("reviewer"));
        let _ = std::fs::remove_dir_all(&d);
    }
}
