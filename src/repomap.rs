//! Repository map: a token-budgeted outline of the codebase — the files that matter most, each with
//! the symbols it defines. Small-context models (the ones this harness targets) cannot read a repo
//! to orient themselves; a ranked map costs a couple of thousand tokens and replaces a dozen
//! exploratory tool calls. Same idea as Aider's repo map, without the tree-sitter dependency:
//! per-language definition regexes plus a reference-count ranking over the whole tree.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

const MAX_FILE_BYTES: u64 = 256 * 1024;
const MAX_TOTAL_BYTES: u64 = 12 * 1024 * 1024;
const MAX_FILES: usize = 4000;
const SKIP_DIRS: [&str; 14] = [".git", "node_modules", "target", "dist", "build", "vendor", "__pycache__", ".venv", "venv", ".next", ".cache", "coverage", ".mypy_cache", ".pytest_cache"];

#[derive(Debug, Clone)]
pub struct FileMap { pub path: String, pub lang: &'static str, pub symbols: Vec<Symbol>, pub lines: usize, pub score: f64 }
#[derive(Debug, Clone)]
pub struct Symbol { pub kind: String, pub name: String, pub line: usize }

/// Language of a file by extension, or None if we do not map it.
pub fn lang_of(path: &Path) -> Option<&'static str> {
    match path.extension()?.to_str()? {
        "rs" => Some("rust"),
        "py" | "pyi" => Some("python"),
        "ts" | "tsx" | "mts" => Some("typescript"),
        "js" | "jsx" | "mjs" | "cjs" => Some("javascript"),
        "go" => Some("go"),
        "java" | "kt" => Some("java"),
        "c" | "h" => Some("c"),
        "cc" | "cpp" | "hpp" | "cxx" => Some("cpp"),
        "rb" => Some("ruby"),
        "php" => Some("php"),
        "swift" => Some("swift"),
        "cs" => Some("csharp"),
        "lua" => Some("lua"),
        "sh" | "bash" => Some("shell"),
        _ => None,
    }
}

/// Definition patterns per language: (regex, kind, capture group holding the name).
fn patterns(lang: &str) -> Vec<(regex::Regex, &'static str, usize)> {
    let p: Vec<(&str, &'static str, usize)> = match lang {
        "rust" => vec![
            (r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:unsafe\s+)?fn\s+([A-Za-z_]\w*)", "fn", 1),
            (r"^\s*(?:pub(?:\([^)]*\))?\s+)?struct\s+([A-Za-z_]\w*)", "struct", 1),
            (r"^\s*(?:pub(?:\([^)]*\))?\s+)?enum\s+([A-Za-z_]\w*)", "enum", 1),
            (r"^\s*(?:pub(?:\([^)]*\))?\s+)?trait\s+([A-Za-z_]\w*)", "trait", 1),
            (r"^\s*impl(?:<[^>]*>)?\s+(?:[A-Za-z_][\w:<>, ]*\s+for\s+)?([A-Za-z_]\w*)", "impl", 1),
            (r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:const|static)\s+([A-Z_][A-Z0-9_]*)", "const", 1),
            (r"^\s*macro_rules!\s+([A-Za-z_]\w*)", "macro", 1),
        ],
        "python" => vec![
            (r"^\s*class\s+([A-Za-z_]\w*)", "class", 1),
            (r"^(?:\s{0,4})(?:async\s+)?def\s+([A-Za-z_]\w*)", "def", 1),
        ],
        "typescript" | "javascript" => vec![
            (r"^\s*(?:export\s+)?(?:default\s+)?(?:async\s+)?function\s*\*?\s*([A-Za-z_$][\w$]*)", "fn", 1),
            (r"^\s*(?:export\s+)?(?:abstract\s+)?class\s+([A-Za-z_$][\w$]*)", "class", 1),
            (r"^\s*(?:export\s+)?interface\s+([A-Za-z_$][\w$]*)", "interface", 1),
            (r"^\s*(?:export\s+)?type\s+([A-Za-z_$][\w$]*)", "type", 1),
            (r"^\s*(?:export\s+)?enum\s+([A-Za-z_$][\w$]*)", "enum", 1),
            (r"^\s*(?:export\s+)?(?:const|let|var)\s+([A-Za-z_$][\w$]*)\s*[:=]\s*(?:async\s*)?(?:\(|function|\w+\s*=>)", "fn", 1),
        ],
        "go" => vec![
            (r"^func\s+(?:\([^)]*\)\s*)?([A-Za-z_]\w*)", "func", 1),
            (r"^type\s+([A-Za-z_]\w*)", "type", 1),
        ],
        "java" => vec![
            (r"^\s*(?:public|private|protected|internal)?\s*(?:abstract\s+|final\s+|open\s+|data\s+)?(?:class|interface|object|enum)\s+([A-Za-z_]\w*)", "class", 1),
            (r"^\s*(?:public|private|protected|internal)?\s*(?:static\s+)?(?:suspend\s+)?fun\s+([A-Za-z_]\w*)", "fun", 1),
            (r"^\s{2,}(?:public|private|protected)\s+(?:static\s+)?[\w<>\[\], ]+\s+([A-Za-z_]\w*)\s*\(", "method", 1),
        ],
        "c" | "cpp" => vec![
            (r"^[A-Za-z_][\w \*&:<>,]*\s+\*?([A-Za-z_]\w*)\s*\([^;]*\)\s*\{", "fn", 1),
            (r"^\s*(?:typedef\s+)?(?:struct|class|enum|union)\s+([A-Za-z_]\w*)", "type", 1),
            (r"^\s*#define\s+([A-Z_][A-Z0-9_]*)", "define", 1),
        ],
        "ruby" => vec![(r"^\s*class\s+([A-Za-z_]\w*)", "class", 1), (r"^\s*module\s+([A-Za-z_]\w*)", "module", 1), (r"^\s*def\s+([A-Za-z_][\w?!]*)", "def", 1)],
        "php" => vec![(r"^\s*(?:abstract\s+|final\s+)?class\s+([A-Za-z_]\w*)", "class", 1), (r"^\s*(?:public|private|protected|static|\s)*function\s+([A-Za-z_]\w*)", "fn", 1)],
        "swift" => vec![(r"^\s*(?:public\s+|open\s+|internal\s+)?(?:final\s+)?(?:class|struct|enum|protocol|extension)\s+([A-Za-z_]\w*)", "type", 1), (r"^\s*(?:public\s+|private\s+)?(?:static\s+)?func\s+([A-Za-z_]\w*)", "func", 1)],
        "csharp" => vec![(r"^\s*(?:public|private|internal|protected)?\s*(?:sealed\s+|abstract\s+|static\s+)?(?:class|interface|struct|enum|record)\s+([A-Za-z_]\w*)", "type", 1)],
        "lua" => vec![(r"^\s*(?:local\s+)?function\s+([A-Za-z_][\w.:]*)", "function", 1)],
        "shell" => vec![(r"^\s*(?:function\s+)?([A-Za-z_]\w*)\s*\(\)\s*\{", "fn", 1)],
        _ => vec![],
    };
    p.into_iter().filter_map(|(re, k, g)| regex::Regex::new(re).ok().map(|r| (r, k, g))).collect()
}

/// Files considered for the map (git-tracked when possible, else a bounded walk).
fn candidate_files(root: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let tracked = std::process::Command::new("git").arg("-C").arg(root).args(["ls-files", "-z", "--cached", "--others", "--exclude-standard"]).output().ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).split('\0').filter(|s| !s.is_empty()).map(|s| root.join(s)).collect::<Vec<_>>());
    match tracked {
        Some(files) if !files.is_empty() => out = files,
        _ => walk(root, root, 0, &mut out),
    }
    out.retain(|p| lang_of(p).is_some());
    out.truncate(MAX_FILES);
    out
}

fn walk(dir: &Path, root: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > 8 || out.len() >= MAX_FILES { return; }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if name.starts_with('.') && name != "." { if SKIP_DIRS.contains(&name.as_str()) || p.is_dir() { continue; } }
        if p.is_dir() { if SKIP_DIRS.contains(&name.as_str()) { continue; } walk(&p, root, depth + 1, out); }
        else { out.push(p); }
        if out.len() >= MAX_FILES { return; }
    }
}

/// Parse every candidate file, then rank files by how often the symbols they define are mentioned
/// elsewhere in the tree (a cheap stand-in for graph rank).
pub fn scan(root: &Path) -> Vec<FileMap> {
    let files = candidate_files(root);
    let mut maps: Vec<FileMap> = Vec::new();
    let mut mentions: HashMap<String, usize> = HashMap::new();
    let mut own: HashMap<(usize, String), usize> = HashMap::new();
    let mut budget = MAX_TOTAL_BYTES as i64;
    let mut pats: HashMap<&'static str, Vec<(regex::Regex, &'static str, usize)>> = HashMap::new();

    for path in files {
        let Some(lang) = lang_of(&path) else { continue };
        let Ok(md) = std::fs::metadata(&path) else { continue };
        if md.len() > MAX_FILE_BYTES { continue; }
        budget -= md.len() as i64;
        if budget < 0 { break; }
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let idx = maps.len();
        // identifier frequency over the whole tree, and per file (to subtract self-mentions)
        for ident in identifiers(&text) {
            *mentions.entry(ident.clone()).or_insert(0) += 1;
            *own.entry((idx, ident)).or_insert(0) += 1;
        }
        let ps = pats.entry(lang).or_insert_with(|| patterns(lang));
        let mut symbols = Vec::new();
        for (n, line) in text.lines().enumerate() {
            if line.len() > 400 { continue; }
            for (re, kind, group) in ps.iter() {
                if let Some(c) = re.captures(line) {
                    if let Some(m) = c.get(*group) {
                        symbols.push(Symbol { kind: kind.to_string(), name: m.as_str().to_string(), line: n + 1 });
                        break;
                    }
                }
            }
            if symbols.len() > 400 { break; }
        }
        let rel = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
        maps.push(FileMap { path: rel, lang, lines: text.lines().count(), symbols, score: 0.0 });
    }

    for (i, m) in maps.iter_mut().enumerate() {
        let mut score = 0.0;
        for s in &m.symbols {
            if s.name.len() < 3 { continue; }
            let total = *mentions.get(&s.name).unwrap_or(&0) as f64;
            let mine = *own.get(&(i, s.name.clone())).unwrap_or(&0) as f64;
            let external = (total - mine).max(0.0);
            // a symbol used in many other files makes its file important; log-damped
            score += (1.0 + external).ln();
        }
        // slight bias towards files that define something at all, and away from huge files
        m.score = score / (1.0 + (m.lines as f64 / 800.0));
    }
    maps.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    maps
}

fn identifiers(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in text.chars() {
        if c.is_alphanumeric() || c == '_' { cur.push(c); }
        else if !cur.is_empty() { if cur.len() >= 3 && !cur.chars().next().unwrap().is_numeric() { out.push(std::mem::take(&mut cur)); } else { cur.clear(); } }
    }
    if cur.len() >= 3 { out.push(cur); }
    out
}

/// Render the map within a token budget (≈ 4 chars/token). `focus` boosts files whose path matches.
pub fn render(root: &Path, budget_tokens: usize, focus: Option<&str>) -> String {
    let mut maps = scan(root);
    // names defined in many files are boilerplate (trait methods like `name`/`call`, `main`, `new`);
    // they say nothing about what a file is, so they do not earn a slot in the outline
    let mut defined_in: HashMap<&str, usize> = HashMap::new();
    for m in &maps { let mut seen = std::collections::HashSet::new(); for s in &m.symbols { if seen.insert(s.name.as_str()) { *defined_in.entry(s.name.as_str()).or_insert(0) += 1; } } }
    let boilerplate: std::collections::HashSet<String> = defined_in.iter().filter(|(_, &n)| n >= 6).map(|(k, _)| k.to_string()).collect();
    if let Some(f) = focus {
        let f = f.to_lowercase();
        for m in maps.iter_mut() { if m.path.to_lowercase().contains(&f) { m.score *= 5.0; } }
        maps.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    }
    let cap = budget_tokens.clamp(200, 40_000) * 4;
    let mut out = format!("# Repository map — {} ({} mapped files, most referenced first)\n", root.display(), maps.len());
    let mut used = out.len();
    for m in &maps {
        if used >= cap { out.push_str(&format!("\n… {} more files not shown (raise budget_tokens, or grep/glob for them)\n", maps.len())); break; }
        let mut names: Vec<String> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for s in &m.symbols {
            if !seen.insert(s.name.clone()) { continue; }
            if boilerplate.contains(&s.name) && m.symbols.len() > 4 { continue; }
            names.push(format!("{}:{}", s.name, s.line));
            if names.len() >= 24 { break; }
        }
        let line = if names.is_empty() { format!("\n{} ({} lines)\n", m.path, m.lines) } else { format!("\n{} ({} lines) — {}\n", m.path, m.lines, names.join(" ")) };
        used += line.len();
        out.push_str(&line);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_and_ranks_a_small_tree() {
        let d = std::env::temp_dir().join(format!("harness-repomap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::create_dir_all(d.join("node_modules")).unwrap();
        std::fs::write(d.join("src/core.rs"), "pub struct Engine;\npub fn important_helper() {}\nfn private_thing() {}\n").unwrap();
        std::fs::write(d.join("src/user_a.rs"), "use crate::core::Engine;\nfn a() { important_helper(); important_helper(); }\n").unwrap();
        std::fs::write(d.join("src/user_b.rs"), "fn b() { let _e = Engine; important_helper(); }\n").unwrap();
        std::fs::write(d.join("src/app.py"), "class Widget:\n    def render(self):\n        pass\n").unwrap();
        std::fs::write(d.join("node_modules/ignored.rs"), "pub fn ignored_symbol() {}\n").unwrap();
        let maps = scan(&d);
        let paths: Vec<&str> = maps.iter().map(|m| m.path.as_str()).collect();
        assert!(paths.contains(&"src/core.rs"), "{paths:?}");
        assert!(!paths.iter().any(|p| p.contains("node_modules")), "vendored trees are skipped: {paths:?}");
        assert_eq!(maps[0].path, "src/core.rs", "the most-referenced file ranks first: {paths:?}");
        let py = maps.iter().find(|m| m.path.ends_with("app.py")).unwrap();
        assert!(py.symbols.iter().any(|s| s.name == "Widget" && s.kind == "class"));
        assert!(py.symbols.iter().any(|s| s.name == "render"));

        let text = render(&d, 400, None);
        assert!(text.contains("src/core.rs") && text.contains("important_helper:2"), "{text}");
        let tiny = render(&d, 200, None);
        assert!(tiny.len() < 200 * 4 + 400, "budget respected: {} chars", tiny.len());
        let _ = std::fs::remove_dir_all(&d);
    }
}
