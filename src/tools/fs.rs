use super::{arg_str, Tool, ToolCtx, ToolOutput};
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct ReadFile;
/// Whole-file reads above this size are refused (use offset/limit to read a window).
const MAX_READ_BYTES: u64 = 8 * 1024 * 1024;
pub struct WriteFile;
pub struct EditFile;
pub struct ListDir;

#[async_trait]
impl Tool for ReadFile {
    fn read_only(&self) -> bool { true }
    fn name(&self) -> &'static str { "read_file" }
    fn description(&self) -> &'static str { "Read a text file. Returns numbered lines. Use offset/limit for large files." }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{
            "path":{"type":"string"},
            "offset":{"type":"integer","description":"1-based first line (default 1)"},
            "limit":{"type":"integer","description":"max lines (default 400)"}
        },"required":["path"]})
    }
    async fn call(&self, args: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let path = ctx.resolve(arg_str(&args, "path")?)?;
        let offset = args.get("offset").and_then(|v| v.as_u64()).unwrap_or(1).max(1) as usize;
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(400) as usize;
        let windowed = args.get("offset").is_some() || args.get("limit").is_some();
        let size = tokio::fs::metadata(&path).await.with_context(|| format!("reading {}", path.display()))?.len();
        if size > MAX_READ_BYTES && !windowed { bail!("{} is {} MiB — too large to read whole; pass offset/limit to read a window, or use grep/bash (head/tail)", path.display(), size >> 20); }
        // stream lines: only the requested window is kept in memory
        use tokio::io::AsyncBufReadExt;
        let f = tokio::fs::File::open(&path).await.with_context(|| format!("reading {}", path.display()))?;
        let mut lines = tokio::io::BufReader::new(f).lines();
        let end = offset.saturating_sub(1).saturating_add(limit);
        let (mut total, mut out) = (0usize, String::new());
        while let Some(line) = lines.next_line().await.with_context(|| format!("reading {}", path.display()))? {
            total += 1;
            if total >= offset && total <= end { out.push_str(&format!("{:>5}\t{}\n", total, line)); }
        }
        if end < total { out.push_str(&format!("…[{} more lines; total {}]\n", total - end, total)); }
        if out.is_empty() { out.push_str("(empty)"); }
        Ok((crate::sandbox::truncate_middle(&out, ctx.max_output)).into())
    }
}

#[async_trait]
impl Tool for WriteFile {
    fn name(&self) -> &'static str { "write_file" }
    fn description(&self) -> &'static str { "Create or overwrite a file with the given content. Creates parent directories." }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"]})
    }
    async fn call(&self, args: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let path = ctx.resolve(arg_str(&args, "path")?)?;
        let content = arg_str(&args, "content")?;
        if let Some(p) = path.parent() { tokio::fs::create_dir_all(p).await?; }
        let existed = tokio::fs::read_to_string(&path).await.ok();
        tokio::fs::write(&path, content).await.with_context(|| format!("writing {}", path.display()))?;
        match existed {
            Some(old) => { let (a, r) = line_delta(&old, content); Ok(format!("overwrote {} ({} lines, +{a} -{r})", path.display(), content.lines().count()).into()) }
            None => Ok(format!("created {} ({} bytes, {} lines)", path.display(), content.len(), content.lines().count()).into()),
        }
    }
}

#[async_trait]
impl Tool for EditFile {
    fn name(&self) -> &'static str { "edit_file" }
    fn description(&self) -> &'static str { "Replace an exact, unique substring `old` with `new` in a file. Fails if `old` is missing or ambiguous; include enough context to be unique." }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{"path":{"type":"string"},"old":{"type":"string"},"new":{"type":"string"}},"required":["path","old","new"]})
    }
    async fn call(&self, args: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let path = ctx.resolve(arg_str(&args, "path")?)?;
        let old = arg_str(&args, "old")?;
        let new = arg_str(&args, "new")?;
        if old.is_empty() { bail!("`old` must not be empty"); }
        let text = tokio::fs::read_to_string(&path).await.with_context(|| format!("reading {}", path.display()))?;
        let n = text.matches(old).count();
        if n == 0 { bail!("`old` not found in {}", path.display()); }
        if n > 1 { bail!("`old` matches {n} times in {}; add context to make it unique", path.display()); }
        let out = text.replacen(old, new, 1);
        tokio::fs::write(&path, &out).await?;
        // show the change as a mini diff (the UI colors +/- lines)
        let line_no = text[..text.find(old).unwrap_or(0)].matches('\n').count() + 1;
        let mut d = format!("edited {} @@ line {}\n", path.display(), line_no);
        for l in old.lines() { d.push_str(&format!("- {l}\n")); }
        for l in new.lines() { d.push_str(&format!("+ {l}\n")); }
        Ok(d.trim_end().to_string().into())
    }
}

/// Rough added/removed line counts between two texts (multiset difference).
fn line_delta(old: &str, new: &str) -> (usize, usize) {
    let mut counts: std::collections::HashMap<&str, i64> = std::collections::HashMap::new();
    for l in old.lines() { *counts.entry(l).or_default() -= 1; }
    for l in new.lines() { *counts.entry(l).or_default() += 1; }
    let added: i64 = counts.values().filter(|v| **v > 0).sum(); let removed: i64 = -counts.values().filter(|v| **v < 0).sum::<i64>();
    (added as usize, removed as usize)
}

#[async_trait]
impl Tool for ListDir {
    fn read_only(&self) -> bool { true }
    fn name(&self) -> &'static str { "list_dir" }
    fn description(&self) -> &'static str { "List a directory (non-recursive). Directories end with '/'." }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{"path":{"type":"string","description":"default '.'"}},"required":[]})
    }
    async fn call(&self, args: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let p = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
        let path = ctx.resolve(p)?;
        let mut rd = tokio::fs::read_dir(&path).await.with_context(|| format!("listing {}", path.display()))?;
        let mut names = Vec::new();
        while let Some(e) = rd.next_entry().await? {
            let mut n = e.file_name().to_string_lossy().to_string();
            if e.file_type().await.map(|t| t.is_dir()).unwrap_or(false) { n.push('/'); }
            names.push(n);
        }
        names.sort();
        if names.is_empty() { return Ok("(empty)".into()); }
        Ok((crate::sandbox::truncate_middle(&names.join("\n"), ctx.max_output)).into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    fn ctx() -> (ToolCtx, PathBuf) {
        let d = std::env::temp_dir().join(format!("harness-fs-test-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        (ToolCtx::basic(d.clone()), d)
    }

    #[tokio::test]
    async fn read_offset_limit_and_huge_limit() {
        let (c, d) = ctx();
        std::fs::write(d.join("f.txt"), "l1\nl2\nl3\nl4\nl5\n").unwrap();
        let t = ReadFile.call(json!({"path": "f.txt", "offset": 2, "limit": 2}), &c).await.unwrap().text;
        assert!(t.contains("    2\tl2\n    3\tl3\n"), "{t}"); assert!(!t.contains("l1") && !t.contains("l4"), "{t}");
        assert!(t.contains("2 more lines; total 5"), "{t}");
        let t = ReadFile.call(json!({"path": "f.txt", "offset": 4, "limit": u64::MAX}), &c).await.unwrap().text;
        assert!(t.contains("l5") && !t.contains("more lines"), "{t}");
        let t = ReadFile.call(json!({"path": "f.txt"}), &c).await.unwrap().text;
        assert!(t.starts_with("    1\tl1\n") && t.ends_with("    5\tl5\n"), "{t}");
        assert!(ReadFile.call(json!({"path": "missing.txt"}), &c).await.is_err());
        assert!(ReadFile.call(json!({"path": "../../etc/passwd"}), &c).await.unwrap_err().to_string().contains("escapes"));
    }

    #[tokio::test]
    async fn write_and_edit() {
        let (c, d) = ctx();
        let t = WriteFile.call(json!({"path": "sub/dir/new.txt", "content": "a\nb\n"}), &c).await.unwrap().text;
        assert!(t.starts_with("created"), "{t}");
        assert_eq!(std::fs::read_to_string(d.join("sub/dir/new.txt")).unwrap(), "a\nb\n");
        let t = WriteFile.call(json!({"path": "sub/dir/new.txt", "content": "a\nc\n"}), &c).await.unwrap().text;
        assert!(t.contains("overwrote") && t.contains("+1 -1"), "{t}");
        // edit: unique / ambiguous / missing / empty
        let t = EditFile.call(json!({"path": "sub/dir/new.txt", "old": "c", "new": "d"}), &c).await.unwrap().text;
        assert!(t.contains("@@ line 2") && t.contains("- c") && t.contains("+ d"), "{t}");
        assert_eq!(std::fs::read_to_string(d.join("sub/dir/new.txt")).unwrap(), "a\nd\n");
        std::fs::write(d.join("dup.txt"), "x x").unwrap();
        assert!(EditFile.call(json!({"path": "dup.txt", "old": "x", "new": "y"}), &c).await.unwrap_err().to_string().contains("matches 2 times"));
        assert!(EditFile.call(json!({"path": "dup.txt", "old": "zzz", "new": "y"}), &c).await.unwrap_err().to_string().contains("not found"));
        assert!(EditFile.call(json!({"path": "dup.txt", "old": "", "new": "y"}), &c).await.is_err());
        assert_eq!(std::fs::read_to_string(d.join("dup.txt")).unwrap(), "x x", "failed edits must not touch the file");
        assert!(WriteFile.call(json!({"path": "/etc/harness-x", "content": ""}), &c).await.is_err());
    }
}
