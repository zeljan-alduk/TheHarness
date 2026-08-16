use super::{arg_str, Tool, ToolCtx, ToolOutput};
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct ReadFile;
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
        let text = tokio::fs::read_to_string(&path).await.with_context(|| format!("reading {}", path.display()))?;
        let offset = args.get("offset").and_then(|v| v.as_u64()).unwrap_or(1).max(1) as usize;
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(400) as usize;
        let total = text.lines().count();
        let mut out = String::new();
        for (i, line) in text.lines().enumerate().skip(offset - 1).take(limit) {
            out.push_str(&format!("{:>5}\t{}\n", i + 1, line));
        }
        if offset - 1 + limit < total { out.push_str(&format!("…[{} more lines; total {}]\n", total - (offset - 1 + limit), total)); }
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
