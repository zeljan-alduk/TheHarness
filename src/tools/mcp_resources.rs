//! mcp_resources: list / read MCP *resources* (the harness bridges MCP tools automatically; resources —
//! files, docs, DB rows, screenshots… exposed by a server — are reached through this tool).
//! Servers are looked up in the process-global registry populated by `crate::mcp::start_all`.

use super::{Tool, ToolCtx, ToolOutput};
use anyhow::{bail, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct McpResources;

const NO_SERVERS: &str = "no MCP servers connected (configure them in .mcp.json / ~/.config/harness/mcp.json; see /mcp)";

#[async_trait]
impl Tool for McpResources {
    fn name(&self) -> &'static str { "mcp_resources" }
    fn description(&self) -> &'static str {
        "Browse MCP resources (data exposed by connected MCP servers, addressed by URI): list {server?} shows `server  uri  name  mimeType  description` for every connected server (or one), templates {server?} lists URI templates, read {server, uri} returns the resource contents (text inline; images are shown to you; other binary blobs are described by size/mime)."
    }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{
            "action":{"type":"string","enum":["list","read","templates"],"description":"default list"},
            "server":{"type":"string","description":"MCP server name (required for read; optional filter for list/templates)"},
            "uri":{"type":"string","description":"resource URI to read (from list)"}
        },"required":["action"]})
    }
    fn read_only(&self) -> bool { true }
    async fn call(&self, args: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("list");
        let server = args.get("server").and_then(|v| v.as_str()).map(|s| s.trim()).filter(|s| !s.is_empty());
        let all = crate::mcp::connected_servers();
        if all.is_empty() { return Ok(NO_SERVERS.into()); }
        let selected: Vec<_> = match server {
            Some(name) => {
                let v: Vec<_> = all.iter().filter(|(n, _)| n == name).cloned().collect();
                if v.is_empty() { bail!("no MCP server named '{name}' (connected: {})", all.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>().join(", ")); }
                v
            }
            None => all.clone(),
        };
        match action {
            "list" | "templates" => {
                let mut lines = Vec::new();
                let mut notes = Vec::new();
                for (name, srv) in &selected {
                    let mut s = srv.lock().await;
                    if !s.supports_resources() { notes.push(format!("{name}: server does not support resources")); continue; }
                    let res = if action == "list" { s.list_resources().await } else { s.list_resource_templates().await };
                    match res {
                        Ok(items) => { let l = format_resource_list(name, &items); if l.is_empty() { notes.push(format!("{name}: no {}", if action == "list" { "resources" } else { "resource templates" })); } else { lines.push(l); } }
                        Err(e) => notes.push(format!("{name}: {e:#}")),
                    }
                }
                let mut out = lines.join("\n");
                if !notes.is_empty() { if !out.is_empty() { out.push('\n'); } out.push_str(&notes.join("\n")); }
                if out.is_empty() { out = format!("no {} on connected servers", if action == "list" { "resources" } else { "resource templates" }); }
                Ok(crate::sandbox::truncate_middle(&out, ctx.max_output).into())
            }
            "read" => {
                let uri = args.get("uri").and_then(|v| v.as_str()).map(|s| s.trim()).filter(|s| !s.is_empty());
                let Some(uri) = uri else { bail!("uri required (use action=list to see resource URIs)") };
                let (name, srv) = match server {
                    Some(_) => selected[0].clone(),
                    None if all.len() == 1 => all[0].clone(),
                    None => bail!("server required (connected: {})", all.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>().join(", ")),
                };
                let mut s = srv.lock().await;
                if !s.supports_resources() { return Ok(format!("{name}: server does not support resources").into()); }
                let contents = s.read_resource(uri, ctx.timeout).await?;
                drop(s);
                let (text, images) = format_read(&contents);
                Ok(ToolOutput { text: crate::sandbox::truncate_middle(&text, ctx.max_output), images })
            }
            _ => bail!("unknown action {action} (list|read|templates)"),
        }
    }
}

/// One line per resource: `server  uri  name  mimeType  description` (empty fields as `-`).
pub fn format_resource_list(server: &str, items: &[Value]) -> String {
    let field = |v: &Value, k: &str| v.get(k).and_then(|x| x.as_str()).map(|s| s.trim()).filter(|s| !s.is_empty()).map(|s| s.replace('\n', " ")).unwrap_or_else(|| "-".to_string());
    items.iter().map(|r| {
        let uri = r.get("uri").or_else(|| r.get("uriTemplate")).and_then(|x| x.as_str()).unwrap_or("-");
        format!("{server}  {uri}  {}  {}  {}", field(r, "name"), field(r, "mimeType"), crate::llm::truncate_for_log(&field(r, "description"), 160))
    }).collect::<Vec<_>>().join("\n")
}

/// Render `resources/read` contents: text inline, image blobs to `images`, other blobs described.
pub fn format_read(contents: &[Value]) -> (String, Vec<(String, String)>) {
    let mut text = String::new();
    let mut images = Vec::new();
    for c in contents {
        let uri = c["uri"].as_str().unwrap_or("");
        let mime = c["mimeType"].as_str().unwrap_or("");
        if contents.len() > 1 { text.push_str(&format!("--- {uri}{}\n", if mime.is_empty() { String::new() } else { format!(" ({mime})") })); }
        if let Some(t) = c["text"].as_str() { text.push_str(t); if !t.ends_with('\n') { text.push('\n'); } }
        else if let Some(b) = c["blob"].as_str() {
            let bytes = b.len() * 3 / 4 - (b.len() - b.trim_end_matches('=').len());
            if mime.starts_with("image/") { images.push((mime.to_string(), b.to_string())); text.push_str(&format!("[image {mime}, ~{bytes} bytes — shown to you]\n")); }
            else { text.push_str(&format!("[binary blob {}, ~{bytes} bytes (base64, not shown)]\n", if mime.is_empty() { "unknown mime" } else { mime })); }
        } else { text.push_str(&format!("[empty content {uri}]\n")); }
    }
    if text.trim().is_empty() && images.is_empty() { text = "(no contents)".to_string(); }
    (text.trim_end().to_string(), images)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_formatting() {
        let items = vec![
            json!({"uri":"file:///a.txt","name":"a","mimeType":"text/plain","description":"first\nline"}),
            json!({"uri":"db://users"}),
            json!({"uriTemplate":"file:///{path}","name":"tmpl"}),
        ];
        let out = format_resource_list("srv", &items);
        assert_eq!(out, "srv  file:///a.txt  a  text/plain  first line\nsrv  db://users  -  -  -\nsrv  file:///{path}  tmpl  -  -");
        assert_eq!(format_resource_list("srv", &[]), "");
    }

    #[test]
    fn read_formatting() {
        let (t, imgs) = format_read(&[json!({"uri":"x","mimeType":"text/plain","text":"hello"})]);
        assert_eq!(t, "hello"); assert!(imgs.is_empty());
        let (t, imgs) = format_read(&[json!({"uri":"x","mimeType":"image/png","blob":"aGVsbG8="})]);
        assert!(t.contains("image image/png")); assert_eq!(imgs.len(), 1); assert_eq!(imgs[0].0, "image/png");
        let (t, imgs) = format_read(&[json!({"uri":"x","mimeType":"application/pdf","blob":"aGVsbG8="})]);
        assert!(t.contains("binary blob application/pdf")); assert!(imgs.is_empty());
    }
}
