pub mod archive;
pub mod bash;
pub mod download;
pub mod fs;
pub mod image;
pub mod web;

use crate::llm::ToolDef;
use anyhow::{bail, Result};
use async_trait::async_trait;
use serde_json::Value;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

pub struct ToolCtx {
    pub workdir: PathBuf,
    pub timeout: Duration,
    pub max_output: usize,
    pub net: crate::config::NetConfig,
}

impl ToolCtx {
    /// Resolve a model-supplied path against workdir and refuse escapes.
    /// Symlinks are resolved on the deepest existing ancestor.
    pub fn resolve(&self, p: &str) -> Result<PathBuf> {
        let raw = Path::new(p);
        let joined = if raw.is_absolute() { raw.to_path_buf() } else { self.workdir.join(raw) };
        // lexical normalisation
        let mut norm = PathBuf::new();
        for c in joined.components() {
            match c {
                Component::ParentDir => { norm.pop(); }
                Component::CurDir => {}
                other => norm.push(other.as_os_str()),
            }
        }
        // physical check on the deepest existing ancestor
        let root = self.workdir.canonicalize()?;
        let mut probe = norm.clone();
        while !probe.exists() { if !probe.pop() { break; } }
        let real = probe.canonicalize().unwrap_or(probe);
        if !real.starts_with(&root) {
            bail!("path escapes workdir: {} (workdir is {})", p, root.display());
        }
        Ok(norm)
    }
}

/// What a tool hands back: text for the tool message, plus optional images that the
/// agent loop attaches as a follow-up user message (OpenAI tool results are text-only).
#[derive(Debug, Default)]
pub struct ToolOutput {
    pub text: String,
    /// (mime, base64)
    pub images: Vec<(String, String)>,
}
impl From<String> for ToolOutput { fn from(text: String) -> Self { Self { text, images: vec![] } } }
impl From<&str> for ToolOutput { fn from(text: &str) -> Self { Self { text: text.to_string(), images: vec![] } } }

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn parameters(&self) -> Value;
    async fn call(&self, args: Value, ctx: &ToolCtx) -> Result<ToolOutput>;
}

pub struct Registry {
    tools: Vec<Box<dyn Tool>>,
}

impl Registry {
    pub fn defaults(net_enabled: bool) -> Self {
        let mut tools: Vec<Box<dyn Tool>> = vec![
            Box::new(bash::Bash),
            Box::new(fs::ReadFile),
            Box::new(fs::WriteFile),
            Box::new(fs::EditFile),
            Box::new(fs::ListDir),
            Box::new(image::ViewImage),
            Box::new(archive::ReadPdf),
            Box::new(archive::ExtractArchive),
        ];
        if net_enabled {
            tools.push(Box::new(web::WebFetch));
            tools.push(Box::new(web::WebSearch));
            tools.push(Box::new(download::DownloadFile));
        }
        Self { tools }
    }

    pub fn defs(&self) -> Vec<ToolDef> {
        self.tools.iter().map(|t| ToolDef::new(t.name(), t.description(), t.parameters())).collect()
    }

    pub fn names(&self) -> Vec<&'static str> { self.tools.iter().map(|t| t.name()).collect() }

    /// Errors are returned as text so the model can recover.
    pub async fn call(&self, name: &str, args_json: &str, ctx: &ToolCtx) -> ToolOutput {
        let Some(tool) = self.tools.iter().find(|t| t.name() == name) else {
            return format!("error: unknown tool '{name}'. Available: {:?}", self.names()).into();
        };
        let args: Value = if args_json.trim().is_empty() {
            Value::Object(Default::default())
        } else {
            match serde_json::from_str(args_json) {
                Ok(v) => v,
                Err(e) => return format!("error: tool arguments are not valid JSON ({e}): {args_json}").into(),
            }
        };
        match tool.call(args, ctx).await {
            Ok(s) => s,
            Err(e) => format!("error: {e:#}").into(),
        }
    }
}

pub fn arg_str<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    args.get(key).and_then(|v| v.as_str()).ok_or_else(|| anyhow::anyhow!("missing required string argument '{key}'"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ctx() -> ToolCtx {
        let d = std::env::temp_dir().join(format!("harness-test-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        ToolCtx { workdir: d, timeout: Duration::from_secs(5), max_output: 1000, net: crate::config::NetConfig::default() }
    }
    #[test]
    fn resolve_rejects_escape() {
        let c = ctx();
        assert!(c.resolve("../../etc/passwd").is_err());
        assert!(c.resolve("/etc/passwd").is_err());
        assert!(c.resolve("sub/../ok.txt").is_ok());
        assert!(c.resolve("new/dir/file.txt").is_ok());
    }
}
