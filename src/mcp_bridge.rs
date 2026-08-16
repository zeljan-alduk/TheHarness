//! In-process MCP *server* exposing the harness tool registry, plus a tiny stdio proxy so an external
//! client (Claude Code via --mcp-config) can reach it: `harness mcp-proxy <addr>` pumps stdio ↔ socket.
//! Tool calls execute in the hosting process (TUI/CLI) → same permissions, approvals, hooks, redaction.

use crate::permissions::{Approver, Policy};
use crate::tools::{Registry, ToolCtx};
use anyhow::Result;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

pub struct BridgeHost { pub registry: Registry, pub ctx: ToolCtx, pub policy: Arc<Policy>, pub approver: Arc<dyn Approver>, pub sink: Arc<dyn crate::events::Sink> }

/// Address the proxy connects to. Unix socket on unix, TCP loopback elsewhere.
pub fn new_addr() -> String {
    #[cfg(unix)] { format!("unix:{}", std::env::temp_dir().join(format!("harness-bridge-{}-{}.sock", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0))).display()) }
    #[cfg(not(unix))] { "tcp:127.0.0.1:0".to_string() }
}

/// Start listening; returns the address to give the proxy. Runs until the returned handle is dropped/aborted.
pub async fn serve(addr: &str, host: Arc<BridgeHost>) -> Result<(String, tokio::task::JoinHandle<()>)> {
    #[cfg(unix)]
    if let Some(p) = addr.strip_prefix("unix:") {
        let _ = std::fs::remove_file(p);
        let listener = tokio::net::UnixListener::bind(p)?;
        let path = p.to_string();
        let h = tokio::spawn(async move { loop { let Ok((s, _)) = listener.accept().await else { break }; let host = host.clone(); tokio::spawn(async move { let (r, w) = s.into_split(); let _ = handle_conn(BufReader::new(r), w, host).await; }); } });
        return Ok((format!("unix:{path}"), h));
    }
    let bind = addr.strip_prefix("tcp:").unwrap_or("127.0.0.1:0");
    let listener = tokio::net::TcpListener::bind(bind).await?;
    let local = listener.local_addr()?;
    let h = tokio::spawn(async move { loop { let Ok((s, _)) = listener.accept().await else { break }; let host = host.clone(); tokio::spawn(async move { let (r, w) = s.into_split(); let _ = handle_conn(BufReader::new(r), w, host).await; }); } });
    Ok((format!("tcp:{local}"), h))
}

async fn handle_conn<R: tokio::io::AsyncRead + Unpin, W: tokio::io::AsyncWrite + Unpin>(mut reader: BufReader<R>, mut w: W, host: Arc<BridgeHost>) -> Result<()> {
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line).await? == 0 { break; }
        let Ok(msg) = serde_json::from_str::<Value>(line.trim()) else { continue };
        let id = msg.get("id").cloned();
        let method = msg["method"].as_str().unwrap_or("").to_string();
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        let result: Option<Value> = match method.as_str() {
            "initialize" => Some(json!({"protocolVersion": "2025-06-18", "capabilities": {"tools": {}}, "serverInfo": {"name": "harness", "version": env!("CARGO_PKG_VERSION")}})),
            "notifications/initialized" | "notifications/cancelled" => None,
            "ping" => Some(json!({})),
            "tools/list" => Some(json!({"tools": host.registry.defs().into_iter().map(|d| json!({"name": d.function.name, "description": d.function.description, "inputSchema": d.function.parameters})).collect::<Vec<_>>()})),
            "tools/call" => {
                let name = params["name"].as_str().unwrap_or("").to_string();
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                Some(call_tool(&host, &name, args).await)
            }
            _ => { if id.is_some() { let resp = json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": format!("method not found: {method}")}}); w.write_all(format!("{}\n", resp).as_bytes()).await?; } None }
        };
        if let (Some(id), Some(res)) = (id, result) { let resp = json!({"jsonrpc": "2.0", "id": id, "result": res}); w.write_all(format!("{}\n", resp).as_bytes()).await?; w.flush().await?; }
    }
    Ok(())
}

async fn call_tool(host: &BridgeHost, name: &str, args: Value) -> Value {
    use crate::events::Event;
    let id = format!("cc-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0));
    let args_s = args.to_string();
    host.sink.emit(&Event::ToolCall { id: id.clone(), name: name.to_string(), args: args_s.clone() });
    let t0 = std::time::Instant::now();
    // permissions (same policy as the native loop)
    let ro = host.registry.is_read_only(name);
    let blocked: Option<String> = match host.policy.check(name, &args, ro) {
        crate::permissions::Decision::Allow => None,
        crate::permissions::Decision::Deny(r) => Some(format!("error: blocked by permission policy ({r})")),
        crate::permissions::Decision::Ask(r) => {
            let arg = Policy::primary_arg(name, &args);
            let req = crate::permissions::ApprovalRequest { tool: name.to_string(), summary: arg.clone(), suggested_rule: Policy::suggested_rule(name, &arg), reason: r };
            match host.approver.ask(req.clone()).await { crate::permissions::Approval::Once => None, crate::permissions::Approval::Always => { host.policy.allow_always(&req.suggested_rule); None } crate::permissions::Approval::AlwaysProject => { host.policy.allow_always_project(&req.suggested_rule); None } crate::permissions::Approval::Deny => Some("error: the user declined this action.".into()) }
        }
    };
    let out = match blocked { Some(m) => crate::tools::ToolOutput { text: m, images: vec![] }, None => host.registry.call(name, &args_s, &host.ctx).await };
    host.sink.emit(&Event::ToolResult { id, name: name.to_string(), result: out.text.clone(), secs: t0.elapsed().as_secs_f64(), images: out.images.iter().map(|(m, b)| format!("data:{m};base64,{b}")).collect() });
    let mut content = vec![json!({"type": "text", "text": out.text})];
    for (mime, b64) in &out.images { content.push(json!({"type": "image", "data": b64, "mimeType": mime})); }
    json!({"content": content, "isError": out.text.starts_with("error:")})
}

/// `harness mcp-proxy <addr>`: connect and pump stdin→socket, socket→stdout.
pub async fn proxy(addr: &str) -> Result<()> {
    #[cfg(unix)]
    if let Some(p) = addr.strip_prefix("unix:") {
        let s = tokio::net::UnixStream::connect(p).await?;
        let (mut r, mut w) = s.into_split();
        let a = tokio::spawn(async move { let mut stdin = tokio::io::stdin(); let _ = tokio::io::copy(&mut stdin, &mut w).await; });
        let mut stdout = tokio::io::stdout(); let _ = tokio::io::copy(&mut r, &mut stdout).await; a.abort();
        return Ok(());
    }
    let s = tokio::net::TcpStream::connect(addr.strip_prefix("tcp:").unwrap_or(addr)).await?;
    let (mut r, mut w) = s.into_split();
    let a = tokio::spawn(async move { let mut stdin = tokio::io::stdin(); let _ = tokio::io::copy(&mut stdin, &mut w).await; });
    let mut stdout = tokio::io::stdout(); let _ = tokio::io::copy(&mut r, &mut stdout).await; a.abort();
    Ok(())
}
