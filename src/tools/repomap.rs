//! repo_map: a ranked, token-budgeted outline of the repository (see `crate::repomap`).

use super::{Tool, ToolCtx, ToolOutput};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct RepoMap;

#[async_trait]
impl Tool for RepoMap {
    fn read_only(&self) -> bool { true }
    fn name(&self) -> &'static str { "repo_map" }
    fn description(&self) -> &'static str { "Get an outline of the repository: the most-referenced files, each with the symbols it defines and their line numbers, within a token budget. Call this FIRST in an unfamiliar codebase — it replaces a dozen list_dir/grep calls. Use `focus` to bias the map towards a subsystem (a path fragment)." }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{
            "budget_tokens":{"type":"integer","description":"how much context to spend (default 2000)"},
            "focus":{"type":"string","description":"path fragment to prioritise, e.g. \"auth\" or \"src/tools\""},
            "path":{"type":"string","description":"map this sub-directory instead of the whole workdir"}
        }})
    }
    async fn call(&self, args: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let ctx = ctx.effective();
        let root = match args.get("path").and_then(|v| v.as_str()) { Some(p) => ctx.resolve(p)?, None => ctx.workdir.clone() };
        let budget = args.get("budget_tokens").and_then(|v| v.as_u64()).unwrap_or(2000) as usize;
        let focus = args.get("focus").and_then(|v| v.as_str()).map(|s| s.to_string());
        let text = tokio::task::spawn_blocking(move || crate::repomap::render(&root, budget, focus.as_deref())).await?;
        Ok(crate::sandbox::truncate_middle(&text, ctx.max_output * 2).into())
    }
}
