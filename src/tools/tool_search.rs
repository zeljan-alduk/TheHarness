//! tool_search: with many MCP servers connected the tool catalogue can outweigh the conversation, so
//! rarely-used tools are held back and surfaced on demand. The model searches by keyword, gets the
//! matching tools' schemas, and from then on can call them normally.

use super::{arg_str, Registry, Tool, ToolCtx, ToolOutput};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

pub struct ToolSearch { pub registry: Arc<std::sync::Mutex<Option<Registry>>> }

/// Rank deferred tools against a query: name matches beat description matches, all terms must appear.
pub fn rank<'a>(catalogue: &[(&'a str, &'a str)], query: &str, max: usize) -> Vec<&'a str> {
    let terms: Vec<String> = query.to_lowercase().split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-').filter(|t| t.len() > 1).map(|t| t.to_string()).collect();
    let mut scored: Vec<(i64, &str)> = Vec::new();
    for (name, desc) in catalogue {
        let (n, d) = (name.to_lowercase(), desc.to_lowercase());
        if terms.is_empty() { scored.push((0, name)); continue; }
        let mut score = 0i64;
        let mut missing = false;
        for t in &terms {
            if n.contains(t) { score += 10; }
            else if d.contains(t) { score += 3; }
            else { missing = true; }
        }
        if missing && score == 0 { continue; }
        if missing { score -= 2; }
        scored.push((score, name));
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored.into_iter().take(max).map(|(_, n)| n).collect()
}

#[async_trait]
impl Tool for ToolSearch {
    fn read_only(&self) -> bool { true }
    fn name(&self) -> &'static str { "tool_search" }
    fn description(&self) -> &'static str { "Find tools that are not in your tool list yet (MCP servers with large catalogues are loaded on demand). Search by keyword — \"browser click\", \"jira issue\", \"postgres query\" — and the matching tools become callable with their full schemas." }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{"query":{"type":"string","description":"what you need to do"},"max":{"type":"integer","description":"default 5"}},"required":["query"]})
    }
    async fn call(&self, args: Value, _ctx: &ToolCtx) -> Result<ToolOutput> {
        let query = arg_str(&args, "query")?;
        let max = args.get("max").and_then(|v| v.as_u64()).unwrap_or(5).clamp(1, 20) as usize;
        let reg = self.registry.lock().unwrap().clone();
        let Some(reg) = reg else { return Ok("no deferred tools in this session".into()) };
        let catalogue = reg.deferred_catalogue();
        if catalogue.is_empty() { return Ok("every tool is already in your tool list".into()); }
        let hits = rank(&catalogue, query, max);
        if hits.is_empty() { return Ok(format!("no deferred tool matches '{query}'. {} are available; try other words.", catalogue.len()).into()); }
        reg.activate(&hits.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        let mut out = format!("{} tool(s) are now callable:\n", hits.len());
        for name in &hits {
            if let Some(t) = reg.get(name) {
                out.push_str(&format!("\n## {}\n{}\nparameters: {}\n", t.name(), crate::llm::truncate_for_log(t.description(), 600), crate::llm::truncate_for_log(&t.parameters().to_string(), 1200)));
            }
        }
        Ok(out.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranks_by_name_then_description() {
        let cat = vec![
            ("mcp__chrome__click", "Click an element on the page"),
            ("mcp__jira__create_issue", "Create a Jira issue in a project"),
            ("mcp__pg__query", "Run a SQL query against Postgres"),
        ];
        assert_eq!(rank(&cat, "jira issue", 5)[0], "mcp__jira__create_issue");
        assert_eq!(rank(&cat, "click page", 5)[0], "mcp__chrome__click");
        assert_eq!(rank(&cat, "sql postgres", 5)[0], "mcp__pg__query");
        assert!(rank(&cat, "kubernetes helm", 5).is_empty(), "no false positives");
        assert_eq!(rank(&cat, "issue", 1).len(), 1, "max is respected");
    }
}
