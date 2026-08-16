//! Minimal MCP (Model Context Protocol) client over stdio: JSON-RPC 2.0, newline-delimited.
//! Each configured server is spawned once; its tools are exposed to the model as `mcp__<server>__<tool>`.
//! Config format is the de-facto standard `{"mcpServers": {"name": {"command", "args", "env", "cwd"}}}`
//! (Claude Code / Cursor / DSH-compatible), read from ~/.config/harness/mcp.json, <workdir>/.harness/mcp.json,
//! <workdir>/.mcp.json, and every enabled plugin's mcp.json.

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    /// stdio transport: program to run (omit when `url` is set)
    #[serde(default)]
    pub command: String,
    /// streamable-HTTP transport: endpoint URL (e.g. http://localhost:3000/mcp)
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub disabled: bool,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct McpFile {
    #[serde(default, rename = "mcpServers")]
    pub mcp_servers: HashMap<String, ServerConfig>,
}

/// Collect server configs from all standard locations. Later files override earlier ones by name.
pub fn discover(workdir: &Path, extra_files: &[PathBuf]) -> Vec<(String, ServerConfig, PathBuf)> {
    let mut files: Vec<PathBuf> = Vec::new();
    if let Ok(h) = std::env::var("HOME") { files.push(PathBuf::from(h).join(".config/harness/mcp.json")); }
    files.push(workdir.join(".harness/mcp.json"));
    files.push(workdir.join(".mcp.json"));
    files.extend(extra_files.iter().cloned());
    let mut out: HashMap<String, (ServerConfig, PathBuf)> = HashMap::new();
    for f in files {
        let Ok(text) = std::fs::read_to_string(&f) else { continue };
        let Ok(parsed) = serde_json::from_str::<McpFile>(&text) else { eprintln!("mcp: could not parse {}", f.display()); continue };
        for (name, cfg) in parsed.mcp_servers { if !cfg.disabled { out.insert(name, (cfg, f.clone())); } }
    }
    let mut v: Vec<(String, ServerConfig, PathBuf)> = out.into_iter().map(|(n, (c, f))| (n, c, f)).collect();
    v.sort_by(|a, b| a.0.cmp(&b.0));
    v
}

#[derive(Debug, Clone)]
pub struct McpToolInfo { pub name: String, pub description: String, pub input_schema: Value }

enum Transport {
    Stdio { child: Child, stdin: ChildStdin, reader: BufReader<ChildStdout> },
    Http { http: reqwest::Client, url: String, headers: HashMap<String, String>, session: Option<String> },
}

pub struct McpServer {
    pub name: String,
    transport: Transport,
    next_id: u64,
    pub tools: Vec<McpToolInfo>,
}

impl McpServer {
    pub async fn spawn(name: &str, cfg: &ServerConfig, workdir: &Path) -> Result<Self> {
        if let Some(url) = &cfg.url {
            let http = reqwest::Client::builder().timeout(Duration::from_secs(120)).build()?;
            let mut headers = cfg.headers.clone();
            for v in headers.values_mut() { *v = expand_env(v); }
            let mut s = Self { name: name.to_string(), transport: Transport::Http { http, url: url.clone(), headers, session: None }, next_id: 1, tools: vec![] };
            s.handshake().await?;
            return Ok(s);
        }
        if cfg.command.is_empty() { bail!("mcp '{name}': needs `command` (stdio) or `url` (http)"); }
        let mut c = Command::new(&cfg.command);
        c.args(&cfg.args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true);
        c.env("PATH", crate::setup::path_with_bin_dir(workdir));
        for (k, v) in &cfg.env { c.env(k, expand_env(v)); }
        c.current_dir(cfg.cwd.as_ref().map(|d| PathBuf::from(expand_env(d))).unwrap_or(workdir.to_path_buf()));
        let mut child = c.spawn().with_context(|| format!("mcp '{name}': cannot start `{}` (is it installed? try `harness setup`)", cfg.command))?;
        let stdin = child.stdin.take().context("no stdin")?;
        let stdout = child.stdout.take().context("no stdout")?;
        let mut s = Self { name: name.to_string(), transport: Transport::Stdio { child, stdin, reader: BufReader::new(stdout) }, next_id: 1, tools: vec![] };
        s.handshake().await?;
        Ok(s)
    }

    async fn handshake(&mut self) -> Result<()> {
        let name = self.name.clone();
        self.request("initialize", json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "harness", "version": env!("CARGO_PKG_VERSION")}}), Duration::from_secs(30)).await
            .with_context(|| format!("mcp '{name}': initialize failed"))?;
        self.notify("notifications/initialized", json!({})).await?;
        let list = self.request("tools/list", json!({}), Duration::from_secs(30)).await.with_context(|| format!("mcp '{name}': tools/list failed"))?;
        for t in list["tools"].as_array().cloned().unwrap_or_default() {
            self.tools.push(McpToolInfo {
                name: t["name"].as_str().unwrap_or("").to_string(),
                description: t["description"].as_str().unwrap_or("").to_string(),
                input_schema: t.get("inputSchema").cloned().unwrap_or(json!({"type": "object", "properties": {}})),
            });
        }
        Ok(())
    }

    async fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        let msg = json!({"jsonrpc": "2.0", "method": method, "params": params});
        match &mut self.transport {
            Transport::Stdio { stdin, .. } => { stdin.write_all(format!("{}\n", msg).as_bytes()).await?; stdin.flush().await?; }
            Transport::Http { http, url, headers, session } => {
                let mut req = http.post(url.as_str()).header("Content-Type", "application/json").header("Accept", "application/json, text/event-stream").json(&msg);
                for (k, v) in headers.iter() { req = req.header(k, v); }
                if let Some(sid) = session.as_ref() { req = req.header("Mcp-Session-Id", sid.as_str()); }
                let _ = req.send().await;
            }
        }
        Ok(())
    }

    async fn request(&mut self, method: &str, params: Value, timeout: Duration) -> Result<Value> {
        let id = self.next_id; self.next_id += 1;
        let msg = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let name = self.name.clone();
        match &mut self.transport {
            Transport::Stdio { stdin, reader, .. } => {
                stdin.write_all(format!("{}\n", msg).as_bytes()).await?;
                stdin.flush().await?;
                let deadline = tokio::time::Instant::now() + timeout;
                loop {
                    let mut line = String::new();
                    let n = tokio::time::timeout_at(deadline, reader.read_line(&mut line)).await.map_err(|_| anyhow::anyhow!("timeout waiting for {method}"))??;
                    if n == 0 { bail!("mcp server '{}' closed its stdout", name); }
                    let Ok(v) = serde_json::from_str::<Value>(line.trim()) else { continue }; // logs / non-JSON lines
                    if v.get("id").and_then(|i| i.as_u64()) == Some(id) {
                        if let Some(err) = v.get("error") { bail!("mcp error: {}", err); }
                        return Ok(v.get("result").cloned().unwrap_or(Value::Null));
                    }
                    if v.get("method").is_some() && v.get("id").is_some() {
                        let resp = json!({"jsonrpc": "2.0", "id": v["id"], "error": {"code": -32601, "message": "not supported by harness"}});
                        let _ = stdin.write_all(format!("{}\n", resp).as_bytes()).await;
                    }
                }
            }
            Transport::Http { http, url, headers, session } => {
                let mut req = http.post(url.as_str()).header("Content-Type", "application/json").header("Accept", "application/json, text/event-stream").json(&msg).timeout(timeout);
                for (k, v) in headers.iter() { req = req.header(k, v); }
                if let Some(sid) = session.as_ref() { req = req.header("Mcp-Session-Id", sid.as_str()); }
                let resp = req.send().await.with_context(|| format!("mcp '{name}': POST {url}"))?;
                if let Some(sid) = resp.headers().get("mcp-session-id").and_then(|v| v.to_str().ok()) { *session = Some(sid.to_string()); }
                let status = resp.status();
                let ctype = resp.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
                let body = resp.text().await?;
                if !status.is_success() { bail!("mcp '{name}': HTTP {status}: {}", crate::llm::truncate_for_log(&body, 300)); }
                let candidates: Vec<Value> = if ctype.contains("text/event-stream") {
                    body.lines().filter_map(|l| l.strip_prefix("data:")).filter_map(|d| serde_json::from_str::<Value>(d.trim()).ok()).collect()
                } else { serde_json::from_str::<Value>(&body).ok().into_iter().collect() };
                for v in candidates {
                    if v.get("id").and_then(|i| i.as_u64()) == Some(id) {
                        if let Some(err) = v.get("error") { bail!("mcp error: {}", err); }
                        return Ok(v.get("result").cloned().unwrap_or(Value::Null));
                    }
                }
                bail!("mcp '{name}': no response for {method} (id {id}) in HTTP reply")
            }
        }
    }

    pub async fn call_tool(&mut self, tool: &str, args: Value, timeout: Duration) -> Result<(String, Vec<(String, String)>)> {
        let res = self.request("tools/call", json!({"name": tool, "arguments": args}), timeout).await?;
        let mut text = String::new();
        let mut images = Vec::new();
        for c in res["content"].as_array().cloned().unwrap_or_default() {
            match c["type"].as_str() {
                Some("text") => { text.push_str(c["text"].as_str().unwrap_or("")); text.push('\n'); }
                Some("image") => { images.push((c["mimeType"].as_str().unwrap_or("image/png").to_string(), c["data"].as_str().unwrap_or("").to_string())); text.push_str("[image]\n"); }
                Some("resource") => { if let Some(t) = c["resource"]["text"].as_str() { text.push_str(t); text.push('\n'); } else { text.push_str(&format!("[resource {}]\n", c["resource"]["uri"].as_str().unwrap_or(""))); } }
                _ => { text.push_str(&c.to_string()); text.push('\n'); }
            }
        }
        if res["isError"].as_bool().unwrap_or(false) { text = format!("error: {}", text.trim()); }
        if text.trim().is_empty() && images.is_empty() { text = res.to_string(); }
        Ok((text.trim_end().to_string(), images))
    }

    pub async fn shutdown(mut self) { if let Transport::Stdio { child, .. } = &mut self.transport { let _ = child.kill().await; } }
}

fn expand_env(s: &str) -> String {
    let mut out = s.to_string();
    if let Some(h) = std::env::var_os("HOME") { out = out.replace("${HOME}", &h.to_string_lossy()).replace("~/", &format!("{}/", h.to_string_lossy())); }
    // ${VAR} substitution
    let mut res = String::new(); let mut rest = out.as_str();
    while let Some(i) = rest.find("${") {
        res.push_str(&rest[..i]);
        if let Some(j) = rest[i..].find('}') { let var = &rest[i + 2..i + j]; res.push_str(&std::env::var(var).unwrap_or_default()); rest = &rest[i + j + 1..]; } else { res.push_str(&rest[i..]); rest = ""; }
    }
    res.push_str(rest);
    res
}

/// A live MCP tool exposed through the harness `Tool` trait.
pub struct McpTool { pub server: Arc<Mutex<McpServer>>, pub server_name: String, pub info: McpToolInfo, pub full_name: String, pub description: String }

#[async_trait::async_trait]
impl crate::tools::Tool for McpTool {
    fn name(&self) -> &'static str { Box::leak(self.full_name.clone().into_boxed_str()) }
    fn description(&self) -> &'static str { Box::leak(self.description.clone().into_boxed_str()) }
    fn parameters(&self) -> Value { self.info.input_schema.clone() }
    async fn call(&self, args: Value, ctx: &crate::tools::ToolCtx) -> Result<crate::tools::ToolOutput> {
        let mut s = self.server.lock().await;
        let (text, images) = s.call_tool(&self.info.name, args, ctx.timeout).await?;
        Ok(crate::tools::ToolOutput { text: crate::sandbox::truncate_middle(&text, ctx.max_output), images })
    }
}

/// Start all discovered servers; return the tools (and any startup errors, as strings).
pub async fn start_all(workdir: &Path, extra_files: &[PathBuf]) -> (Vec<Arc<dyn crate::tools::Tool>>, Vec<String>, Vec<Arc<Mutex<McpServer>>>) {
    let mut tools: Vec<Arc<dyn crate::tools::Tool>> = Vec::new();
    let mut errors = Vec::new();
    let mut servers = Vec::new();
    for (name, cfg, _file) in discover(workdir, extra_files) {
        match tokio::time::timeout(Duration::from_secs(60), McpServer::spawn(&name, &cfg, workdir)).await {
            Ok(Ok(server)) => {
                let infos = server.tools.clone();
                let shared = Arc::new(Mutex::new(server));
                for info in infos {
                    let full = format!("mcp__{}__{}", sanitize(&name), sanitize(&info.name));
                    let desc = format!("[MCP {name}] {}", info.description);
                    tools.push(Arc::new(McpTool { server: shared.clone(), server_name: name.clone(), full_name: full, description: desc, info }));
                }
                servers.push(shared);
            }
            Ok(Err(e)) => errors.push(format!("{e:#}")),
            Err(_) => errors.push(format!("mcp '{name}': startup timed out")),
        }
    }
    (tools, errors, servers)
}

fn sanitize(s: &str) -> String { s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect() }
