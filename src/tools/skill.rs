//! load_skill: returns a plugin skill's SKILL.md (and its file list) so the model can follow it.

use super::{arg_str, Tool, ToolCtx, ToolOutput};
use anyhow::{bail, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct LoadSkill;

#[async_trait]
impl Tool for LoadSkill {
    fn name(&self) -> &'static str { "load_skill" }
    fn description(&self) -> &'static str { "Load a skill (instructions + reference files) provided by an installed plugin. Call it when a task matches one of the skills listed in your system prompt, then follow the returned instructions." }
    fn parameters(&self) -> Value { json!({"type":"object","properties":{"name":{"type":"string"}},"required":["name"]}) }
    async fn call(&self, args: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let name = arg_str(&args, "name")?;
        let plugins = crate::plugins::Plugins::open()?;
        let Some(sk) = plugins.find_skill(name) else {
            let names: Vec<String> = plugins.enabled().into_iter().flat_map(|p| p.skills).map(|s| s.name).collect();
            bail!("no skill named '{name}'. Available: {}", if names.is_empty() { "(none — install plugins with /plugin)".to_string() } else { names.join(", ") });
        };
        let body = std::fs::read_to_string(&sk.path)?;
        let dir = sk.path.parent().unwrap_or(std::path::Path::new("."));
        let mut files = Vec::new();
        fn walk(d: &std::path::Path, depth: usize, out: &mut Vec<String>) { if depth > 3 { return; } if let Ok(rd) = std::fs::read_dir(d) { for e in rd.flatten() { let p = e.path(); if p.file_name().map(|n| n.to_string_lossy().starts_with('.')).unwrap_or(false) { continue; } if p.is_dir() { walk(&p, depth + 1, out); } else { out.push(p.display().to_string()); } if out.len() > 80 { return; } } } }
        walk(dir, 0, &mut files);
        let mut text = format!("# Skill: {} (plugin {}, dir {})\n\n{}\n", sk.name, sk.plugin, dir.display(), body.trim());
        if files.len() > 1 { text.push_str("\n\nFiles in this skill (read_file them by path as needed):\n"); for f in files { text.push_str(&format!("- {f}\n")); } }
        Ok(crate::sandbox::truncate_middle(&text, ctx.max_output * 2).into())
    }
}
