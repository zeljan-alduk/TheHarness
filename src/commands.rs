//! Project and user slash commands defined in markdown, the way Claude Code, OpenCode, Kilo and Qwen
//! do it: `.harness/commands/<name>.md` (also `.claude/commands`, and the same under `~`). The file's
//! body is a prompt template:
//!
//!   $ARGUMENTS       everything typed after the command
//!   $1 … $9          individual words
//!   !`git status`    replaced by the output of that shell command, run before the prompt is sent
//!   @path/to/file    replaced by the file's contents
//!
//! Frontmatter: `description` (shown in /help), `model`, `agent` (run it as a sub-agent instead).

use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Command {
    pub name: String,
    pub description: String,
    pub template: String,
    pub source: String,
    pub path: PathBuf,
    pub model: Option<String>,
    pub agent: Option<String>,
}

pub fn dirs(workdir: &Path) -> Vec<(PathBuf, &'static str)> {
    let mut v: Vec<(PathBuf, &'static str)> = vec![
        (workdir.join(".harness").join("commands"), "project"),
        (workdir.join(".claude").join("commands"), "project"),
        (workdir.join(".agents").join("commands"), "project"),
    ];
    v.push((crate::setup::config_dir().join("commands"), "user"));
    let h = crate::setup::home_dir();
    v.push((h.join(".claude").join("commands"), "user"));
    v
}

/// Every markdown command available here (project first, then user, then plugins).
pub fn discover(workdir: &Path) -> Vec<Command> {
    let mut out: Vec<Command> = Vec::new();
    for (dir, source) in dirs(workdir) {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        let mut files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().map(|e| e == "md").unwrap_or(false)).collect();
        files.sort();
        for f in files {
            let Ok(text) = std::fs::read_to_string(&f) else { continue };
            let (fm, body) = crate::instructions::split_frontmatter(&text);
            let get = |k: &str| fm.iter().find(|(a, _)| a.eq_ignore_ascii_case(k)).map(|(_, v)| v.clone());
            let name = get("name").unwrap_or_else(|| f.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default());
            if name.trim().is_empty() || body.trim().is_empty() { continue; }
            if out.iter().any(|c| c.name == name) { continue; }
            out.push(Command {
                description: get("description").unwrap_or_else(|| body.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string()),
                name, template: body.trim().to_string(), source: source.to_string(), path: f,
                model: get("model"), agent: get("agent"),
            });
        }
    }
    if let Ok(p) = crate::plugins::Plugins::open() {
        for c in p.commands() {
            if out.iter().any(|x| x.name == c.name) { continue; }
            out.push(Command { name: c.name, description: c.description, template: c.template, source: format!("plugin {}", c.plugin), path: PathBuf::new(), model: None, agent: None });
        }
    }
    out
}

pub fn find(workdir: &Path, name: &str) -> Option<Command> {
    let n = name.trim().trim_start_matches('/').to_lowercase();
    discover(workdir).into_iter().find(|c| c.name.to_lowercase() == n)
}

/// Expand a template: arguments, `!`shell`` substitutions and `@file` inclusions.
pub async fn expand(cmd: &Command, args: &str, workdir: &Path) -> String {
    let mut out = cmd.template.clone();
    out = out.replace("$ARGUMENTS", args.trim());
    let words: Vec<&str> = args.split_whitespace().collect();
    for i in 1..=9 { out = out.replace(&format!("${i}"), words.get(i - 1).copied().unwrap_or("")); }

    // !`command` → its output
    while let Some(start) = out.find("!`") {
        let Some(rel_end) = out[start + 2..].find('`') else { break };
        let end = start + 2 + rel_end;
        let shell_cmd = out[start + 2..end].to_string();
        let result = match crate::sandbox::run_shell(&shell_cmd, workdir, std::time::Duration::from_secs(60), 20_000).await {
            Ok(o) => { let mut t = o.stdout.trim().to_string(); if !o.success() && !o.stderr.trim().is_empty() { t.push_str(&format!("\n{}", o.stderr.trim())); } t }
            Err(e) => format!("(command failed: {e})"),
        };
        out.replace_range(start..=end, &result);
    }

    // @path → file contents (only inside the workdir)
    let mut expanded = String::with_capacity(out.len());
    for (i, part) in out.split('@').enumerate() {
        if i == 0 { expanded.push_str(part); continue; }
        let token: String = part.chars().take_while(|c| !c.is_whitespace() && *c != ',' && *c != ')').collect();
        let rest = &part[token.len()..];
        let p = workdir.join(&token);
        match (token.is_empty(), std::fs::read_to_string(&p)) {
            (false, Ok(text)) => expanded.push_str(&format!("\n--- {token} ---\n{}\n---\n", crate::sandbox::truncate_middle(&text, 20_000))),
            _ => { expanded.push('@'); expanded.push_str(&token); }
        }
        expanded.push_str(rest);
    }
    expanded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn discovers_and_expands() {
        let d = std::env::temp_dir().join(format!("harness-cmds-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join(".harness/commands")).unwrap();
        std::fs::write(d.join("notes.md"), "the notes body").unwrap();
        std::fs::write(d.join(".harness/commands/review.md"),
            "---\ndescription: Review a diff\n---\nReview $1 for $ARGUMENTS.\nBranch: !`echo main`\nNotes: @notes.md\n").unwrap();
        let cmds = discover(&d);
        let c = cmds.iter().find(|c| c.name == "review").expect("found");
        assert_eq!(c.description, "Review a diff");
        assert_eq!(c.source, "project");
        let out = expand(c, "src/lib.rs bugs and style", &d).await;
        assert!(out.contains("Review src/lib.rs for src/lib.rs bugs and style."), "{out}");
        assert!(out.contains("Branch: main"), "shell substitution: {out}");
        assert!(out.contains("the notes body"), "file inclusion: {out}");
        assert!(!out.contains("@notes.md"), "{out}");
        // an unknown @token is left alone
        let c2 = Command { name: "x".into(), description: String::new(), template: "see @nope.md and user@example.com".into(), source: String::new(), path: PathBuf::new(), model: None, agent: None };
        let out = expand(&c2, "", &d).await;
        assert!(out.contains("@nope.md") && out.contains("user@example.com"), "{out}");
        let _ = std::fs::remove_dir_all(&d);
    }
}
