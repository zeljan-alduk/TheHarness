use super::{arg_str, Tool, ToolCtx};
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct ReadFile;
pub struct WriteFile;
pub struct EditFile;
pub struct ListDir;

#[async_trait]
impl Tool for ReadFile {
    fn name(&self) -> &'static str { "read_file" }
    fn description(&self) -> &'static str { "Read a text file. Returns numbered lines. Use offset/limit for large files." }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{
            "path":{"type":"string"},
            "offset":{"type":"integer","description":"1-based first line (default 1)"},
            "limit":{"type":"integer","description":"max lines (default 400)"}
        },"required":["path"]})
    }
    async fn call(&self, args: Value, ctx: &ToolCtx) -> Result<String> {
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
        Ok(crate::sandbox::truncate_middle(&out, ctx.max_output))
    }
}

#[async_trait]
impl Tool for WriteFile {
    fn name(&self) -> &'static str { "write_file" }
    fn description(&self) -> &'static str { "Create or overwrite a file with the given content. Creates parent directories." }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"]})
    }
    async fn call(&self, args: Value, ctx: &ToolCtx) -> Result<String> {
        let path = ctx.resolve(arg_str(&args, "path")?)?;
        let content = arg_str(&args, "content")?;
        if let Some(p) = path.parent() { tokio::fs::create_dir_all(p).await?; }
        tokio::fs::write(&path, content).await.with_context(|| format!("writing {}", path.display()))?;
        Ok(format!("wrote {} bytes to {}", content.len(), path.display()))
    }
}

#[async_trait]
impl Tool for EditFile {
    fn name(&self) -> &'static str { "edit_file" }
    fn description(&self) -> &'static str { "Replace an exact, unique substring `old` with `new` in a file. Fails if `old` is missing or ambiguous; include enough context to be unique." }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{"path":{"type":"string"},"old":{"type":"string"},"new":{"type":"string"}},"required":["path","old","new"]})
    }
    async fn call(&self, args: Value, ctx: &ToolCtx) -> Result<String> {
        let path = ctx.resolve(arg_str(&args, "path")?)?;
        let old = arg_str(&args, "old")?;
        let new = arg_str(&args, "new")?;
        if old.is_empty() { bail!("`old` must not be empty"); }
        let text = tokio::fs::read_to_string(&path).await.with_context(|| format!("reading {}", path.display()))?;
        let n = text.matches(old).count();
        if n == 0 { bail!("`old` not found in {}", path.display()); }
        if n > 1 { bail!("`old` matches {n} times in {}; add context to make it unique", path.display()); }
        let out = text.replacen(old, new, 1);
        tokio::fs::write(&path, out).await?;
        Ok(format!("edited {}", path.display()))
    }
}

#[async_trait]
impl Tool for ListDir {
    fn name(&self) -> &'static str { "list_dir" }
    fn description(&self) -> &'static str { "List a directory (non-recursive). Directories end with '/'." }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{"path":{"type":"string","description":"default '.'"}},"required":[]})
    }
    async fn call(&self, args: Value, ctx: &ToolCtx) -> Result<String> {
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
        Ok(crate::sandbox::truncate_middle(&names.join("\n"), ctx.max_output))
    }
}
