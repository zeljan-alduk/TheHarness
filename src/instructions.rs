//! Project instruction files, discovered the way every other harness does it.
//!
//! Chain per directory: `AGENTS.md` → `CLAUDE.md` → `HARNESS.md` → `GEMINI.md` → `.cursorrules` →
//! `.github/copilot-instructions.md` (first hit wins), plus `*.local.md` / `*.override.md` additions.
//! Directories are walked from the repo root down to the working directory (most specific last), and
//! global files (`~/.agents/AGENTS.md`, `~/.claude/CLAUDE.md`, `~/.config/harness/HARNESS.md`) come first.
//! Files may pull in others with `@path/to/file` lines (depth ≤ 3).
//!
//! Rules (`.harness/rules/*.md`, `.claude/rules/*.md`, `.cursor/rules/*.mdc`) carry `paths:`/`globs:`
//! frontmatter: they are injected lazily, the first time a tool touches a matching file. Instruction
//! files in sub-directories below the working directory work the same way.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

/// Per-directory instruction file names, highest precedence first.
pub const NAMES: [&str; 6] = ["AGENTS.md", "CLAUDE.md", "HARNESS.md", "GEMINI.md", ".cursorrules", ".github/copilot-instructions.md"];
/// Additions loaded on top of the chosen file (personal / generated overrides).
const EXTRA: [&str; 4] = ["AGENTS.local.md", "AGENTS.override.md", "CLAUDE.local.md", ".harness/HARNESS.md"];
const MAX_DOC_CHARS: usize = 24_000;
const MAX_IMPORT_DEPTH: usize = 3;

#[derive(Debug, Clone)]
pub struct Doc {
    pub path: PathBuf,
    pub text: String,
    /// Set when the doc was pulled in by an `@import` from another file.
    pub imported_by: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct Rule {
    pub path: PathBuf,
    pub name: String,
    pub description: String,
    /// Path globs this rule applies to; empty + `always` = always injected.
    pub globs: Vec<String>,
    pub always: bool,
    pub body: String,
}

/// Everything loaded for one working directory.
pub struct Instructions {
    pub workdir: PathBuf,
    pub docs: Vec<Doc>,
    pub rules: Vec<Rule>,
    /// Paths already injected (either eagerly or on access) — each is shown at most once per session.
    seen: Mutex<HashSet<PathBuf>>,
}

/// Per-workdir cache: loaded once per process so the "inject each file once" bookkeeping is shared
/// between the system prompt and the on-access injection. `reset()` drops it (`/reload`).
static CACHE: OnceLock<Mutex<HashMap<PathBuf, Arc<Instructions>>>> = OnceLock::new();
fn cache() -> &'static Mutex<HashMap<PathBuf, Arc<Instructions>>> { CACHE.get_or_init(Default::default) }

pub fn cached(workdir: &Path) -> Arc<Instructions> {
    let key = workdir.canonicalize().unwrap_or_else(|_| workdir.to_path_buf());
    if let Some(hit) = cache().lock().ok().and_then(|g| g.get(&key).cloned()) { return hit; }
    let ins = Arc::new(Instructions::load(&key));
    if let Ok(mut g) = cache().lock() { g.insert(key, ins.clone()); }
    ins
}

/// Forget everything loaded (after editing an instruction file, or `/reload`).
pub fn reset() { if let Ok(mut g) = cache().lock() { g.clear(); } }

impl Instructions {
    pub fn load(workdir: &Path) -> Instructions {
        let workdir = workdir.canonicalize().unwrap_or_else(|_| workdir.to_path_buf());
        let mut docs = Vec::new();
        let mut seen_files: HashSet<PathBuf> = HashSet::new();

        // global (user-level) instructions
        let home = dirs_home();
        let mut globals: Vec<PathBuf> = vec![crate::setup::config_dir().join("HARNESS.md")];
        if let Some(h) = &home {
            globals.push(h.join(".agents").join("AGENTS.md"));
            globals.push(h.join(".claude").join("CLAUDE.md"));
            globals.push(h.join(".codex").join("AGENTS.md"));
        }
        for g in globals { collect_file(&g, None, &mut docs, &mut seen_files, 0); }

        // project chain: root → … → workdir (most specific last)
        for dir in ancestors(&workdir) { collect_dir(&dir, &mut docs, &mut seen_files); }

        let rules = load_rules(&workdir);
        let seen: HashSet<PathBuf> = docs.iter().map(|d| d.path.clone()).collect();
        Instructions { workdir, docs, rules, seen: Mutex::new(seen) }
    }

    /// The block injected into the system prompt: all eagerly-loaded docs plus always-on rules,
    /// and a one-line index of the path-gated rules so the model knows they exist.
    pub fn prompt_block(&self) -> String {
        let mut s = String::new();
        if !self.docs.is_empty() {
            s.push_str("\n\n# Project instructions\nThese files are the user's standing instructions for this repository. Follow them; they win over your defaults (but never over an explicit user request in this session).\n");
            for d in &self.docs {
                let via = d.imported_by.as_ref().map(|p| format!(" (imported by {})", p.display())).unwrap_or_default();
                s.push_str(&format!("\n--- {}{} ---\n{}\n", d.path.display(), via, d.text.trim_end()));
            }
        }
        let (always, gated): (Vec<&Rule>, Vec<&Rule>) = self.rules.iter().partition(|r| r.always || r.globs.is_empty());
        if !always.is_empty() {
            s.push_str("\n# Rules\n");
            for r in always { s.push_str(&format!("\n--- {} ---\n{}\n", r.path.display(), r.body.trim_end())); if let Ok(mut g) = self.seen.lock() { g.insert(r.path.clone()); } }
        }
        if !gated.is_empty() {
            s.push_str("\n# Path-scoped rules (their text arrives when you touch a matching file)\n");
            for r in gated { s.push_str(&format!("- {} — {} [{}]\n", r.name, crate::llm::truncate_for_log(&r.description, 100), r.globs.join(", "))); }
        }
        s
    }

    /// Text to append to a tool result because the call touched `path`: rules whose globs match and
    /// instruction files living in that file's directory. Each file is returned at most once.
    pub fn on_path(&self, path: &Path) -> Option<String> {
        let abs = if path.is_absolute() { path.to_path_buf() } else { self.workdir.join(path) };
        // the caller may hand us an un-canonicalized path (symlinked temp dirs, /var vs /private/var)
        let abs = abs.parent().and_then(|d| d.canonicalize().ok()).map(|d| d.join(abs.file_name().unwrap_or_default())).unwrap_or(abs);
        let rel = abs.strip_prefix(&self.workdir).unwrap_or(&abs).to_string_lossy().replace('\\', "/");
        let mut out = String::new();
        for r in &self.rules {
            if r.always || r.globs.is_empty() { continue; }
            if !r.globs.iter().any(|g| glob_path(g, &rel)) { continue; }
            if !self.claim(&r.path) { continue; }
            out.push_str(&format!("\n\n[project rule {} — applies to {}]\n{}", r.path.display(), r.globs.join(", "), r.body.trim()));
        }
        // instruction files in sub-directories of the workdir (not part of the eager walk-up)
        if let Some(dir) = abs.parent() {
            if dir.starts_with(&self.workdir) && dir != self.workdir {
                for name in NAMES.iter().chain(EXTRA.iter()) {
                    let p = dir.join(name);
                    if !p.is_file() || !self.claim(&p) { continue; }
                    if let Ok(t) = std::fs::read_to_string(&p) {
                        out.push_str(&format!("\n\n[instructions for {}: {}]\n{}", dir.display(), p.display(), cap(&t)));
                    }
                    break;
                }
            }
        }
        (!out.is_empty()).then_some(out)
    }

    fn claim(&self, p: &Path) -> bool { self.seen.lock().map(|mut g| g.insert(p.to_path_buf())).unwrap_or(false) }
}

/// The primary path argument of a tool call, if it has one (used to trigger path-scoped rules).
pub fn touched_path(tool: &str, args: &serde_json::Value) -> Option<String> {
    let p = match tool {
        "read_file" | "write_file" | "edit_file" | "apply_patch" | "notebook_edit" | "view_image" => args.get("path"),
        "list_dir" | "glob" | "grep" => args.get("path"),
        _ => None,
    }?;
    p.as_str().filter(|s| !s.is_empty()).map(|s| s.to_string())
}

fn cap(t: &str) -> String {
    if t.chars().count() <= MAX_DOC_CHARS { return t.trim_end().to_string(); }
    t.chars().take(MAX_DOC_CHARS).collect::<String>() + "\n…[truncated]"
}

fn dirs_home() -> Option<PathBuf> { std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from) }

/// Directories from the repo root (or 6 levels up) down to `workdir`, deduped.
fn ancestors(workdir: &Path) -> Vec<PathBuf> {
    let mut chain: Vec<PathBuf> = Vec::new();
    let home = dirs_home();
    let mut cur = Some(workdir.to_path_buf());
    let mut hops = 0;
    while let Some(d) = cur {
        chain.push(d.clone());
        hops += 1;
        let is_root = d.join(".git").exists();
        let is_home = home.as_ref().map(|h| &d == h).unwrap_or(false);
        if is_root || is_home || hops >= 6 { break; }
        cur = d.parent().map(|p| p.to_path_buf());
    }
    chain.reverse();
    chain
}

fn collect_dir(dir: &Path, docs: &mut Vec<Doc>, seen: &mut HashSet<PathBuf>) {
    for name in NAMES {
        let p = dir.join(name);
        if p.is_file() { collect_file(&p, None, docs, seen, 0); break; }
    }
    for name in EXTRA { collect_file(&dir.join(name), None, docs, seen, 0); }
}

fn collect_file(p: &Path, imported_by: Option<&Path>, docs: &mut Vec<Doc>, seen: &mut HashSet<PathBuf>, depth: usize) {
    if docs.len() > 24 { return; }
    let real = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    if !real.is_file() || !seen.insert(real.clone()) { return; }
    let Ok(text) = std::fs::read_to_string(&real) else { return };
    if text.trim().is_empty() { return; }
    let imports = if depth < MAX_IMPORT_DEPTH { find_imports(&text, real.parent().unwrap_or(Path::new("."))) } else { vec![] };
    docs.push(Doc { path: real.clone(), text: cap(&text), imported_by: imported_by.map(|p| p.to_path_buf()) });
    for imp in imports { collect_file(&imp, Some(&real), docs, seen, depth + 1); }
}

/// `@path/to/file.md` at the start of a line (Claude Code / Codex style imports). `~` is expanded.
fn find_imports(text: &str, base: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for line in text.lines() {
        let l = line.trim();
        let Some(rest) = l.strip_prefix('@') else { continue };
        let raw = rest.split_whitespace().next().unwrap_or("");
        if raw.is_empty() || raw.starts_with('@') { continue; }
        let p = if let Some(r) = raw.strip_prefix("~/") { match dirs_home() { Some(h) => h.join(r), None => continue } } else if Path::new(raw).is_absolute() { PathBuf::from(raw) } else { base.join(raw) };
        if p.is_file() { out.push(p); }
        if out.len() >= 8 { break; }
    }
    out
}

fn load_rules(workdir: &Path) -> Vec<Rule> {
    let mut out = Vec::new();
    let mut dirs: Vec<PathBuf> = vec![
        workdir.join(".harness").join("rules"),
        workdir.join(".claude").join("rules"),
        workdir.join(".cursor").join("rules"),
        workdir.join(".windsurf").join("rules"),
    ];
    if let Some(h) = dirs_home() { dirs.push(h.join(".claude").join("rules")); }
    dirs.push(crate::setup::config_dir().join("rules"));
    for d in dirs {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        let mut files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().map(|e| e == "md" || e == "mdc").unwrap_or(false)).collect();
        files.sort();
        for f in files {
            let Ok(text) = std::fs::read_to_string(&f) else { continue };
            let (fm, body) = split_frontmatter(&text);
            let get = |k: &str| fm.iter().find(|(a, _)| a == k).map(|(_, v)| v.clone());
            let globs: Vec<String> = get("paths").or_else(|| get("globs")).map(|v| split_list(&v)).unwrap_or_default();
            let always = get("always").or_else(|| get("alwaysApply")).map(|v| matches!(v.trim(), "true" | "yes" | "on")).unwrap_or(globs.is_empty());
            let name = get("name").unwrap_or_else(|| f.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default());
            let description = get("description").unwrap_or_else(|| body.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim_start_matches('#').trim().to_string());
            if body.trim().is_empty() { continue; }
            out.push(Rule { path: f, name, description, globs, always, body: cap(&body) });
            if out.len() >= 40 { return out; }
        }
    }
    out
}

/// `key: value` YAML-ish frontmatter between `---` fences; returns (pairs, body).
pub fn split_frontmatter(text: &str) -> (Vec<(String, String)>, String) {
    let t = text.trim_start_matches('\u{feff}');
    if !t.starts_with("---") { return (vec![], text.to_string()); }
    let mut lines = t.lines();
    lines.next();
    let mut pairs = Vec::new();
    let mut body = String::new();
    let mut in_fm = true;
    for line in lines {
        if in_fm {
            if line.trim() == "---" { in_fm = false; continue; }
            if let Some((k, v)) = line.split_once(':') {
                let v = v.trim().trim_matches('"').trim_matches('\'').to_string();
                pairs.push((k.trim().to_string(), v));
            }
        } else { body.push_str(line); body.push('\n'); }
    }
    if in_fm { return (vec![], text.to_string()); } // no closing fence: not frontmatter
    (pairs, body)
}

/// `a, b` or `["a", "b"]` → ["a", "b"]
pub fn split_list(v: &str) -> Vec<String> {
    v.trim().trim_start_matches('[').trim_end_matches(']')
        .split(',').map(|s| s.trim().trim_matches('"').trim_matches('\'').to_string())
        .filter(|s| !s.is_empty()).collect()
}

/// Path glob with `**` (any depth), `*` (within a segment) and `?`. A bare `*.rs` matches at any depth.
pub fn glob_path(pat: &str, path: &str) -> bool {
    let pat = pat.trim().trim_start_matches("./");
    let path = path.trim_start_matches("./");
    if pat.is_empty() { return false; }
    if to_regex(pat).map(|r| r.is_match(path)).unwrap_or(false) { return true; }
    // a pattern without a separator also matches the basename at any depth
    if !pat.contains('/') { if let Some(base) = path.rsplit('/').next() { return to_regex(pat).map(|r| r.is_match(base)).unwrap_or(false); } }
    false
}

fn to_regex(pat: &str) -> Option<regex::Regex> {
    let mut re = String::from("^");
    let chars: Vec<char> = pat.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '*' => {
                if i + 1 < chars.len() && chars[i + 1] == '*' {
                    // `**/` may match nothing at all; bare `**` matches anything
                    if i + 2 < chars.len() && chars[i + 2] == '/' { re.push_str("(?:.*/)?"); i += 3; continue; }
                    re.push_str(".*"); i += 2; continue;
                }
                re.push_str("[^/]*"); i += 1;
            }
            '?' => { re.push_str("[^/]"); i += 1; }
            c => { re.push_str(&regex::escape(&c.to_string())); i += 1; }
        }
    }
    re.push('$');
    regex::Regex::new(&re).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs() {
        assert!(glob_path("src/**/*.rs", "src/tools/fs.rs"));
        assert!(glob_path("src/**/*.rs", "src/main.rs"));
        assert!(!glob_path("src/**/*.rs", "tests/main.rs"));
        assert!(glob_path("*.md", "docs/GAPS.md"));
        assert!(glob_path("**/*.test.ts", "a/b/c.test.ts"));
        assert!(!glob_path("src/*.rs", "src/a/b.rs"));
    }

    #[test]
    fn frontmatter() {
        let (fm, body) = split_frontmatter("---\nname: x\npaths: src/**/*.rs, tests/*\n---\nbody here\n");
        assert_eq!(fm.iter().find(|(k, _)| k == "name").unwrap().1, "x");
        assert_eq!(split_list(&fm.iter().find(|(k, _)| k == "paths").unwrap().1), vec!["src/**/*.rs", "tests/*"]);
        assert_eq!(body.trim(), "body here");
        let (fm2, body2) = split_frontmatter("# no frontmatter\n---\n");
        assert!(fm2.is_empty()); assert!(body2.starts_with("# no"));
    }

    #[test]
    fn chain_and_imports() {
        let d = std::env::temp_dir().join(format!("harness-instr-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("sub")).unwrap();
        std::fs::create_dir_all(d.join(".git")).unwrap();
        std::fs::create_dir_all(d.join(".harness/rules")).unwrap();
        std::fs::write(d.join("AGENTS.md"), "root rules\n@extra.md\n").unwrap();
        std::fs::write(d.join("CLAUDE.md"), "should be shadowed by AGENTS.md").unwrap();
        std::fs::write(d.join("extra.md"), "imported bit").unwrap();
        std::fs::write(d.join("sub/AGENTS.md"), "sub rules").unwrap();
        std::fs::write(d.join(".harness/rules/rust.md"), "---\npaths: src/**/*.rs\ndescription: rust style\n---\nuse ? not unwrap\n").unwrap();
        let ins = Instructions::load(&d);
        let block = ins.prompt_block();
        assert!(block.contains("root rules"), "{block}");
        assert!(block.contains("imported bit"), "{block}");
        assert!(!block.contains("shadowed"), "{block}");
        assert!(block.contains("rust style"), "{block}");
        assert!(!block.contains("use ? not unwrap"), "path-gated rule must not be eager: {block}");
        let hit = ins.on_path(Path::new("src/tools/fs.rs")).unwrap_or_default();
        assert!(hit.contains("use ? not unwrap"), "{hit}");
        assert!(ins.on_path(Path::new("src/tools/other.rs")).is_none(), "rule injected only once");
        let sub = ins.on_path(&d.join("sub/x.rs")).unwrap_or_default();
        assert!(sub.contains("sub rules"), "{sub}");
        let _ = std::fs::remove_dir_all(&d);
    }
}
