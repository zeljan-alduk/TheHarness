//! lsp: language-server-backed code intelligence (diagnostics, definition, references, hover, symbols).

use super::{arg_str, Tool, ToolCtx, ToolOutput};
use anyhow::{bail, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::Duration;

pub struct Lsp;

#[async_trait]
impl Tool for Lsp {
    fn read_only(&self) -> bool { true }
    fn name(&self) -> &'static str { "lsp" }
    fn description(&self) -> &'static str { "Code intelligence via the language server (rust-analyzer, pyright, typescript-language-server, gopls): diagnostics {path} (errors/warnings for a file, incl. type errors), definition/references/hover {path, line, col} (1-based), symbols {path}. Prefer this over grep for 'where is X defined/used'." }
    fn parameters(&self) -> Value { json!({"type":"object","properties":{"action":{"type":"string","enum":["diagnostics","definition","references","hover","symbols","status"]},"path":{"type":"string"},"line":{"type":"integer"},"col":{"type":"integer"}},"required":["action"]}) }
    async fn call(&self, args: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let action = arg_str(&args, "action")?;
        let servers = if ctx.lsp_servers.is_empty() { crate::lsp::default_servers() } else { ctx.lsp_servers.clone() };
        if action == "status" { return Ok(servers.iter().map(|(n, c)| format!("{n}: {} {}  ({})", c.command, c.args.join(" "), c.exts.join(","))).collect::<Vec<_>>().join("\n").into()); }
        let path = ctx.resolve(arg_str(&args, "path")?)?;
        if !path.is_file() { bail!("not a file: {}", path.display()); }
        let Some((name, cfg)) = crate::lsp::server_for(&path, &servers) else { bail!("no language server configured for {} (see lsp status / [lsp] in harness.toml)", path.display()) };
        let root = ctx.workdir.canonicalize().unwrap_or(ctx.workdir.clone());
        let server = crate::lsp::LspServer::get_or_start(&name, &cfg, &root).await?;
        let uri = server.sync_doc(&path).await?;
        let pos = || -> Result<Value> { let line = args.get("line").and_then(|v| v.as_u64()).ok_or_else(|| anyhow::anyhow!("line required (1-based)"))?; let col = args.get("col").and_then(|v| v.as_u64()).unwrap_or(1); Ok(json!({"line": line.saturating_sub(1), "character": col.saturating_sub(1)})) };
        match action {
            "diagnostics" => {
                let (wait, settle) = if name == "rust" { (60, 25) } else { (15, 4) };
                let d = server.wait_diagnostics(&uri, Duration::from_secs(wait), Duration::from_secs(settle)).await;
                if d.is_empty() { return Ok(format!("no diagnostics for {} ({name})", path.display()).into()); }
                let mut lines: Vec<String> = d.iter().map(|x| crate::lsp::fmt_diag(&uri, x, &root)).collect();
                lines.truncate(80);
                Ok(crate::sandbox::truncate_middle(&lines.join("\n"), ctx.max_output).into())
            }
            "definition" | "references" => {
                let method = if action == "definition" { "textDocument/definition" } else { "textDocument/references" };
                let mut params = json!({"textDocument": {"uri": uri}, "position": pos()?});
                if action == "references" { params["context"] = json!({"includeDeclaration": true}); }
                let res = server.request(method, params, Duration::from_secs(30)).await?;
                let items: Vec<Value> = match res { Value::Array(a) => a, Value::Null => vec![], other => vec![other] };
                if items.is_empty() { return Ok(format!("no {action} found").into()); }
                Ok(items.iter().take(100).map(|l| crate::lsp::fmt_location(l, &root)).collect::<Vec<_>>().join("\n").into())
            }
            "hover" => {
                let res = server.request("textDocument/hover", json!({"textDocument": {"uri": uri}, "position": pos()?}), Duration::from_secs(30)).await?;
                let text = match &res["contents"] { Value::String(s) => s.clone(), Value::Object(o) => o.get("value").and_then(|v| v.as_str()).unwrap_or("").to_string(), Value::Array(a) => a.iter().map(|x| x.as_str().map(String::from).or_else(|| x["value"].as_str().map(String::from)).unwrap_or_default()).collect::<Vec<_>>().join("\n"), _ => String::new() };
                Ok(if text.trim().is_empty() { "no hover info".into() } else { crate::sandbox::truncate_middle(&text, ctx.max_output).into() })
            }
            "symbols" => {
                let res = server.request("textDocument/documentSymbol", json!({"textDocument": {"uri": uri}}), Duration::from_secs(30)).await?;
                let mut out = Vec::new();
                fn walk(v: &Value, depth: usize, out: &mut Vec<String>) { let name = v["name"].as_str().unwrap_or(""); let kind = v["kind"].as_u64().unwrap_or(0); let line = v["selectionRange"]["start"]["line"].as_u64().or_else(|| v["location"]["range"]["start"]["line"].as_u64()).unwrap_or(0) + 1; out.push(format!("{}{} {} (line {line})", "  ".repeat(depth), kind_name(kind), name)); for c in v["children"].as_array().cloned().unwrap_or_default() { walk(&c, depth + 1, out); } }
                for s in res.as_array().cloned().unwrap_or_default() { walk(&s, 0, &mut out); }
                if out.is_empty() { return Ok("no symbols".into()); }
                Ok(crate::sandbox::truncate_middle(&out.join("\n"), ctx.max_output).into())
            }
            _ => bail!("unknown action {action}"),
        }
    }
}

fn kind_name(k: u64) -> &'static str { match k { 1 => "file", 2 => "module", 3 => "namespace", 4 => "package", 5 => "class", 6 => "method", 7 => "property", 8 => "field", 9 => "constructor", 10 => "enum", 11 => "interface", 12 => "function", 13 => "variable", 14 => "constant", 23 => "struct", 22 => "enum-member", 26 => "type-param", _ => "symbol" } }
