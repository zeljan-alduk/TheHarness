//! Skills discovered from the standard directories every harness now uses, plus plugin skills.
//!
//! Layout: `<dir>/<name>/SKILL.md` (or `<dir>/<name>.md`) with `---` frontmatter:
//! `name`, `description`, `allowed-tools`, `model`, `effort`, `context: fork`, `paths:` (only offer the
//! skill when the project contains a matching file). Searched, project first:
//! `.harness/skills`, `.agents/skills`, `.claude/skills`, then `~/.config/harness/skills`,
//! `~/.agents/skills`, `~/.claude/skills`, then installed plugins.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct SkillDef {
    pub name: String,
    pub description: String,
    pub path: PathBuf,
    /// Where it came from, for display: "project", "user", or "plugin <name>".
    pub source: String,
    pub allowed_tools: Vec<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    /// `context: fork` — run the skill in a sub-agent instead of inline.
    pub fork: bool,
    /// Only offer this skill when the project has a file matching one of these globs.
    pub paths: Vec<String>,
}

impl SkillDef {
    pub fn body(&self) -> String {
        let t = std::fs::read_to_string(&self.path).unwrap_or_default();
        crate::instructions::split_frontmatter(&t).1
    }
    pub fn dir(&self) -> PathBuf { self.path.parent().unwrap_or(Path::new(".")).to_path_buf() }
}

/// Standard skill directories, project-scoped first.
pub fn dirs(workdir: &Path) -> Vec<(PathBuf, &'static str)> {
    let mut v: Vec<(PathBuf, &'static str)> = vec![
        (workdir.join(".harness").join("skills"), "project"),
        (workdir.join(".agents").join("skills"), "project"),
        (workdir.join(".claude").join("skills"), "project"),
    ];
    v.push((crate::setup::config_dir().join("skills"), "user"));
    if let Some(h) = home() {
        v.push((h.join(".agents").join("skills"), "user"));
        v.push((h.join(".claude").join("skills"), "user"));
    }
    v
}

/// Every skill available here (standard dirs + enabled plugins), de-duplicated by name
/// (project beats user beats plugin).
pub fn discover(workdir: &Path) -> Vec<SkillDef> {
    let mut out: Vec<SkillDef> = Vec::new();
    for (dir, source) in dirs(workdir) {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            let file = if p.is_dir() { p.join("SKILL.md") } else if p.extension().map(|e| e == "md").unwrap_or(false) { p.clone() } else { continue };
            if !file.is_file() { continue; }
            if let Some(s) = parse(&file, source) { push(&mut out, s); }
        }
    }
    if let Ok(pl) = crate::plugins::Plugins::open() {
        for p in pl.enabled() {
            for sk in p.skills {
                let mut def = parse(&sk.path, "plugin").unwrap_or(SkillDef { name: sk.name.clone(), description: sk.description.clone(), path: sk.path.clone(), source: String::new(), allowed_tools: vec![], model: None, effort: None, fork: false, paths: vec![] });
                def.name = sk.name; def.source = format!("plugin {}", sk.plugin);
                push(&mut out, def);
            }
        }
    }
    out
}

fn push(out: &mut Vec<SkillDef>, s: SkillDef) {
    if out.iter().any(|x| x.name.eq_ignore_ascii_case(&s.name)) { return; }
    out.push(s);
}

fn parse(file: &Path, source: &str) -> Option<SkillDef> {
    let text = std::fs::read_to_string(file).ok()?;
    let (fm, body) = crate::instructions::split_frontmatter(&text);
    let get = |k: &str| fm.iter().find(|(a, _)| a.eq_ignore_ascii_case(k)).map(|(_, v)| v.clone());
    let name = get("name").unwrap_or_else(|| {
        let stem = file.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        if stem == "SKILL" { file.parent().and_then(|d| d.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or(stem) } else { stem }
    });
    let description = get("description").unwrap_or_else(|| body.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim_start_matches('#').trim().to_string());
    if name.trim().is_empty() { return None; }
    Some(SkillDef {
        name: name.trim().to_string(),
        description,
        path: file.to_path_buf(),
        source: source.to_string(),
        allowed_tools: get("allowed-tools").or_else(|| get("allowed_tools")).or_else(|| get("tools")).map(|v| crate::instructions::split_list(&v)).unwrap_or_default(),
        model: get("model"),
        effort: get("effort"),
        fork: get("context").map(|v| v.trim() == "fork").unwrap_or(false),
        paths: get("paths").or_else(|| get("globs")).map(|v| crate::instructions::split_list(&v)).unwrap_or_default(),
    })
}

pub fn find(workdir: &Path, name: &str) -> Option<SkillDef> {
    let n = name.trim().to_lowercase();
    let all = discover(workdir);
    all.iter().find(|s| s.name.to_lowercase() == n)
        .or_else(|| all.iter().find(|s| s.name.to_lowercase().replace(' ', "-") == n.replace(' ', "-")))
        .or_else(|| all.iter().find(|s| s.name.to_lowercase().ends_with(&format!(":{n}"))))
        .cloned()
}

/// The system-prompt listing. Path-gated skills are only listed when the project has a matching file.
pub fn prompt_block(workdir: &Path) -> String {
    let skills = discover(workdir);
    if skills.is_empty() { return String::new(); }
    let files = project_files(workdir);
    let visible: Vec<&SkillDef> = skills.iter().filter(|s| s.paths.is_empty() || s.paths.iter().any(|g| files.iter().any(|f| crate::instructions::glob_path(g, f)))).collect();
    if visible.is_empty() { return String::new(); }
    let mut s = String::from("\n\n# Skills\nPackaged instructions for specific kinds of work. When a task matches one, call load_skill {name} FIRST and follow what it says.\n");
    for sk in visible { s.push_str(&format!("- {} — {} [{}]\n", sk.name, crate::llm::truncate_for_log(&sk.description, 160), sk.source)); }
    s
}

fn home() -> Option<PathBuf> { std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from) }

/// Repo-relative file list (bounded) used for `paths:` gating.
fn project_files(workdir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    fn walk(dir: &Path, base: &Path, depth: usize, out: &mut Vec<String>) {
        if depth > 4 || out.len() > 3000 { return; }
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            if name.starts_with('.') || matches!(name.as_str(), "node_modules" | "target" | "dist" | "build" | "vendor" | "__pycache__") { continue; }
            if p.is_dir() { walk(&p, base, depth + 1, out); }
            else if let Ok(r) = p.strip_prefix(base) { out.push(r.to_string_lossy().replace('\\', "/")); }
            if out.len() > 3000 { return; }
        }
    }
    walk(workdir, workdir, 0, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn discovers_and_gates() {
        let d = std::env::temp_dir().join(format!("harness-skills-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join(".harness/skills/pdf-forms")).unwrap();
        std::fs::create_dir_all(d.join(".claude/skills/rusty")).unwrap();
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("src/main.rs"), "fn main(){}").unwrap();
        std::fs::write(d.join(".harness/skills/pdf-forms/SKILL.md"), "---\nname: pdf-forms\ndescription: Fill PDF forms\nallowed-tools: bash, read_file\n---\nsteps\n").unwrap();
        std::fs::write(d.join(".claude/skills/rusty/SKILL.md"), "---\nname: rusty\ndescription: Rust review\npaths: **/*.py\n---\nbody\n").unwrap();
        let all = discover(&d);
        assert!(all.iter().any(|s| s.name == "pdf-forms" && s.allowed_tools == vec!["bash", "read_file"]), "{all:?}");
        assert!(all.iter().any(|s| s.name == "rusty"));
        let block = prompt_block(&d);
        assert!(block.contains("pdf-forms"), "{block}");
        assert!(!block.contains("rusty"), "path-gated skill must stay hidden: {block}");
        assert_eq!(find(&d, "pdf-forms").unwrap().body().trim(), "steps");
        let _ = std::fs::remove_dir_all(&d);
    }
}
