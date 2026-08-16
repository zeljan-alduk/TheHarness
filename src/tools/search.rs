//! grep (ripgrep-backed, plain-grep fallback) and glob (native walker) — structured, bounded, read-only.

use super::{arg_str, Tool, ToolCtx, ToolOutput};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub struct Grep;
pub struct Glob;

const SKIP_DIRS: &[&str] = &[".git", "target", "node_modules", ".venv", "venv", "__pycache__", "dist", "build", ".next", ".cache", ".harness-memory"];

#[async_trait]
impl Tool for Grep {
    fn read_only(&self) -> bool { true }
    fn name(&self) -> &'static str { "grep" }
    fn description(&self) -> &'static str { "Search file contents with a regex (ripgrep). Returns path:line: text, bounded. Prefer this over bash grep. Use `glob` to restrict file types, `context` for surrounding lines." }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{
            "pattern":{"type":"string","description":"regular expression (ripgrep syntax)"},
            "path":{"type":"string","description":"file or directory to search (default: workdir)"},
            "glob":{"type":"string","description":"only files matching this glob, e.g. *.rs or **/*.{ts,tsx}"},
            "case_insensitive":{"type":"boolean"},
            "context":{"type":"integer","description":"lines of context around matches (default 0)"},
            "max_results":{"type":"integer","description":"default 200"},
            "files_only":{"type":"boolean","description":"list matching files only"}
        },"required":["pattern"]})
    }
    async fn call(&self, args: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let pattern = arg_str(&args, "pattern")?;
        let path = ctx.resolve(args.get("path").and_then(|v| v.as_str()).unwrap_or("."))?;
        let max = args.get("max_results").and_then(|v| v.as_u64()).unwrap_or(200) as usize;
        let ci = args.get("case_insensitive").and_then(|v| v.as_bool()).unwrap_or(false);
        let context = args.get("context").and_then(|v| v.as_u64()).unwrap_or(0);
        let files_only = args.get("files_only").and_then(|v| v.as_bool()).unwrap_or(false);
        let glob = args.get("glob").and_then(|v| v.as_str());
        let has_rg = crate::sandbox::run_shell("command -v rg", &ctx.workdir, std::time::Duration::from_secs(5), 200).await.map(|o| o.success()).unwrap_or(false);
        let mut cmd = if has_rg {
            let mut c = format!("rg --no-heading --line-number --color never --max-count 50 --max-columns 300 --max-columns-preview -e {}", shq(pattern));
            if ci { c.push_str(" -i"); }
            if context > 0 { c.push_str(&format!(" -C {context}")); }
            if files_only { c.push_str(" -l"); }
            if let Some(g) = glob { c.push_str(&format!(" -g {}", shq(g))); }
            for d in SKIP_DIRS { c.push_str(&format!(" -g '!{d}'")); }
            c
        } else {
            let mut c = format!("grep -rn{}{} -E {}", if ci { "i" } else { "" }, if files_only { "l" } else { "" }, shq(pattern));
            for d in SKIP_DIRS { c.push_str(&format!(" --exclude-dir={d}")); }
            if let Some(g) = glob { c.push_str(&format!(" --include={}", shq(g.rsplit('/').next().unwrap_or(g)))); }
            c
        };
        cmd.push_str(&format!(" {} 2>/dev/null | head -n {}", shq(&path.display().to_string()), max + 1));
        let out = crate::sandbox::run_shell(&cmd, &ctx.workdir, ctx.timeout, ctx.max_output * 2).await?;
        let mut lines: Vec<String> = out.stdout.lines().map(|l| l.strip_prefix(&format!("{}/", ctx.workdir.display())).unwrap_or(l).to_string()).collect();
        let truncated = lines.len() > max;
        lines.truncate(max);
        if lines.is_empty() { return Ok(format!("no matches for /{pattern}/ in {}", path.display()).into()); }
        let mut text = lines.join("\n");
        if truncated { text.push_str(&format!("\n… more than {max} results; narrow the pattern, path or glob")); }
        Ok(crate::sandbox::truncate_middle(&text, ctx.max_output).into())
    }
}

#[async_trait]
impl Tool for Glob {
    fn read_only(&self) -> bool { true }
    fn name(&self) -> &'static str { "glob" }
    fn description(&self) -> &'static str { "Find files by glob pattern (e.g. **/*.rs, src/**/test_*.py). Returns paths sorted by modification time (newest first), skipping .git/target/node_modules." }
    fn parameters(&self) -> Value { json!({"type":"object","properties":{"pattern":{"type":"string"},"path":{"type":"string","description":"root directory (default: workdir)"},"max_results":{"type":"integer","description":"default 300"}},"required":["pattern"]}) }
    async fn call(&self, args: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let pattern = arg_str(&args, "pattern")?.trim_start_matches("./").to_string();
        let root = ctx.resolve(args.get("path").and_then(|v| v.as_str()).unwrap_or("."))?;
        let max = args.get("max_results").and_then(|v| v.as_u64()).unwrap_or(300) as usize;
        let root2 = root.clone();
        let found: Vec<(std::time::SystemTime, PathBuf)> = tokio::task::spawn_blocking(move || {
            let mut out = Vec::new();
            walk(&root2, &root2, &pattern, &mut out, 0);
            out
        }).await.context("glob walk")?;
        let mut found = found;
        found.sort_by(|a, b| b.0.cmp(&a.0));
        let total = found.len();
        let lines: Vec<String> = found.into_iter().take(max).map(|(_, p)| p.strip_prefix(&root).map(|r| r.display().to_string()).unwrap_or(p.display().to_string())).collect();
        if lines.is_empty() { return Ok(format!("no files match {} under {}", arg_str(&args, "pattern")?, root.display()).into()); }
        let mut text = lines.join("\n");
        if total > max { text.push_str(&format!("\n… {} more (of {total})", total - max)); }
        Ok(crate::sandbox::truncate_middle(&text, ctx.max_output).into())
    }
}

fn walk(root: &Path, dir: &Path, pattern: &str, out: &mut Vec<(std::time::SystemTime, PathBuf)>, depth: usize) {
    if depth > 24 || out.len() > 20_000 { return; }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if p.is_dir() { if SKIP_DIRS.contains(&name.as_str()) || (name.starts_with('.') && name != ".") { continue; } walk(root, &p, pattern, out, depth + 1); continue; }
        let rel = p.strip_prefix(root).map(|r| r.display().to_string()).unwrap_or_default();
        if glob_path(pattern, &rel) || glob_path(pattern, &name) {
            let mt = e.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
            out.push((mt, p));
        }
    }
}

/// Glob with `**` (any path segments), `*` (within a segment), `?`, and `{a,b}` alternatives.
pub fn glob_path(pat: &str, path: &str) -> bool {
    if let (Some(a), Some(b)) = (pat.find('{'), pat.find('}')) { if a < b { let (pre, rest) = pat.split_at(a); let (alts, post) = rest[1..].split_at(b - a - 1); let post = &post[1..]; return alts.split(',').any(|alt| glob_path(&format!("{pre}{alt}{post}"), path)); } }
    fn rec(p: &[char], t: &[char]) -> bool {
        match p.first() {
            None => t.is_empty(),
            Some('*') if p.get(1) == Some(&'*') => { let mut rest = &p[2..]; if rest.first() == Some(&'/') { rest = &rest[1..]; } (0..=t.len()).any(|i| (i == 0 || t[i - 1] == '/' || i == t.len()) && rec(rest, &t[i..])) }
            Some('*') => (0..=t.len()).take_while(|&i| i == 0 || t[i - 1] != '/').any(|i| rec(&p[1..], &t[i..])),
            Some('?') => !t.is_empty() && t[0] != '/' && rec(&p[1..], &t[1..]),
            Some(c) => t.first() == Some(c) && rec(&p[1..], &t[1..]),
        }
    }
    rec(&pat.chars().collect::<Vec<_>>(), &path.chars().collect::<Vec<_>>())
}

fn shq(s: &str) -> String { format!("'{}'", s.replace('\'', "'\\''")) }

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static N: AtomicU64 = AtomicU64::new(0);
    fn ctx() -> (ToolCtx, PathBuf) {
        let d = std::env::temp_dir().join(format!("harness-search-test-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        (ToolCtx { workdir: d.clone(), timeout: std::time::Duration::from_secs(30), max_output: 16000, net: crate::config::NetConfig::default(), memory: None, subagent: None, redact_secrets: true, hooks: Default::default(), todos: Default::default() }, d)
    }

    #[test]
    fn globs() {
        assert!(glob_path("**/*.rs", "src/tools/mod.rs")); assert!(glob_path("*.rs", "main.rs")); assert!(!glob_path("*.rs", "src/main.rs"));
        assert!(glob_path("src/**/test_*.py", "src/a/b/test_x.py")); assert!(glob_path("**/*.{ts,tsx}", "ui/app.tsx")); assert!(!glob_path("**/*.{ts,tsx}", "ui/app.js"));
        assert!(glob_path("**/*.rs", "lib.rs"));
    }

    #[tokio::test]
    async fn grep_finds_match_with_line_number() {
        let (c, d) = ctx();
        std::fs::write(d.join("alpha.rs"), "fn one() {}\nlet needle = 1;\nfn three() {}\n").unwrap();
        std::fs::write(d.join("beta.txt"), "nothing to see here\n").unwrap();

        let out = Grep.call(json!({"pattern": "needle"}), &c).await.unwrap();
        assert!(out.text.contains("alpha.rs"), "{}", out.text);
        assert!(out.text.contains(":2:"), "expected line number 2 in: {}", out.text);
        assert!(!out.text.contains("beta.txt"), "{}", out.text);

        let none = Grep.call(json!({"pattern": "zz_no_such_token_zz"}), &c).await.unwrap();
        assert!(none.text.contains("no matches"), "{}", none.text);
    }

    #[tokio::test]
    async fn grep_glob_restricts_file_types() {
        let (c, d) = ctx();
        std::fs::write(d.join("alpha.rs"), "let needle = 1;\n").unwrap();
        std::fs::write(d.join("beta.txt"), "let needle = 2;\n").unwrap();

        let out = Grep.call(json!({"pattern": "needle", "glob": "*.rs"}), &c).await.unwrap();
        assert!(out.text.contains("alpha.rs"), "{}", out.text);
        assert!(!out.text.contains("beta.txt"), "glob *.rs should exclude beta.txt: {}", out.text);
    }

    #[tokio::test]
    async fn glob_finds_nested_files() {
        let (c, d) = ctx();
        std::fs::create_dir_all(d.join("a/b")).unwrap();
        std::fs::write(d.join("top.txt"), "t\n").unwrap();
        std::fs::write(d.join("a/b/deep.txt"), "d\n").unwrap();
        std::fs::write(d.join("a/notes.md"), "n\n").unwrap();

        let out = Glob.call(json!({"pattern": "**/*.txt"}), &c).await.unwrap();
        assert!(out.text.contains("top.txt"), "{}", out.text);
        assert!(out.text.contains("a/b/deep.txt"), "nested file should match **/*.txt: {}", out.text);

        let md = Glob.call(json!({"pattern": "*.md"}), &c).await.unwrap();
        assert!(md.text.contains("a/notes.md"), "{}", md.text);
        assert!(!md.text.contains("deep.txt"), "single-segment *.md must not match nested files: {}", md.text);
    }

    #[tokio::test]
    async fn glob_no_match_reports() {
        let (c, d) = ctx();
        std::fs::write(d.join("x.txt"), "t\n").unwrap();
        let out = Glob.call(json!({"pattern": "**/*.nope"}), &c).await.unwrap();
        assert!(out.text.contains("no files match"), "{}", out.text);
    }
}
