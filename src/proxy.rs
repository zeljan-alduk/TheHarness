//! Network allow-list proxy. Shell commands and tools run with `HTTP(S)_PROXY` pointed here, so every
//! outbound connection has to name its host up front: hosts that do not match the allow-list are
//! refused with a 403 the agent can read, and credentials are stripped from requests to hosts that
//! were not explicitly told to receive them. It is a containment measure, not an intrusion detector —
//! but it turns "the agent can reach anything" into "the agent can reach these domains".

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProxyConfig {
    /// Route tool/shell traffic through the allow-list proxy.
    #[serde(default)] pub enabled: bool,
    /// Address to listen on (default: an ephemeral loopback port).
    #[serde(default)] pub bind: Option<String>,
    /// Host globs that may be reached: "github.com", "*.githubusercontent.com", "crates.io".
    #[serde(default = "d_allow")] pub allow: Vec<String>,
    /// Hosts that may receive Authorization/Cookie headers (default: the allowed hosts themselves).
    #[serde(default)] pub credentials_to: Vec<String>,
    /// Log every decision to stderr.
    #[serde(default)] pub verbose: bool,
}
fn d_allow() -> Vec<String> {
    ["github.com", "*.github.com", "*.githubusercontent.com", "crates.io", "*.crates.io", "static.crates.io",
     "registry.npmjs.org", "*.npmjs.org", "pypi.org", "*.pypi.org", "files.pythonhosted.org",
     "proxy.golang.org", "sum.golang.org", "localhost", "127.0.0.1"]
        .iter().map(|s| s.to_string()).collect()
}
impl Default for ProxyConfig {
    fn default() -> Self { Self { enabled: false, bind: None, allow: d_allow(), credentials_to: vec![], verbose: false } }
}

impl ProxyConfig {
    pub fn allows(&self, host: &str) -> bool {
        let h = host.trim().trim_end_matches('.').to_lowercase();
        self.allow.iter().any(|p| host_match(p, &h))
    }
    pub fn may_carry_credentials(&self, host: &str) -> bool {
        let h = host.trim().to_lowercase();
        if self.credentials_to.is_empty() { return self.allows(&h); }
        self.credentials_to.iter().any(|p| host_match(p, &h))
    }
}

/// Host glob: `*.example.com` matches sub-domains, `example.com` matches exactly, `*` matches all.
pub fn host_match(pattern: &str, host: &str) -> bool {
    let p = pattern.trim().to_lowercase();
    if p == "*" { return true; }
    if let Some(suffix) = p.strip_prefix("*.") { return host == suffix || host.ends_with(&format!(".{suffix}")); }
    host == p
}

/// A running proxy: keep the handle alive for as long as the session needs it.
pub struct Proxy { pub addr: String, pub cfg: Arc<ProxyConfig>, pub blocked: Arc<AtomicU64>, pub allowed: Arc<AtomicU64>, task: tokio::task::JoinHandle<()> }
impl Drop for Proxy { fn drop(&mut self) { self.task.abort(); } }

impl Proxy {
    /// The `HTTP_PROXY` / `HTTPS_PROXY` value for children.
    pub fn url(&self) -> String { format!("http://{}", self.addr) }
    pub fn stats(&self) -> (u64, u64) { (self.allowed.load(Ordering::Relaxed), self.blocked.load(Ordering::Relaxed)) }
}

static RUNNING: std::sync::OnceLock<Proxy> = std::sync::OnceLock::new();

/// Start the proxy once per process and point every child (and our own HTTP clients) at it.
/// Loopback traffic — the local model server above all — bypasses it via NO_PROXY.
pub async fn configure(cfg: ProxyConfig) -> Result<String> {
    if let Some(p) = RUNNING.get() { return Ok(p.url()); }
    let verbose = cfg.verbose;
    let proxy = start(cfg).await?;
    let url = proxy.url();
    std::env::set_var("HTTP_PROXY", &url);
    std::env::set_var("http_proxy", &url);
    std::env::set_var("HTTPS_PROXY", &url);
    std::env::set_var("https_proxy", &url);
    std::env::set_var("NO_PROXY", "localhost,127.0.0.1,::1");
    std::env::set_var("no_proxy", "localhost,127.0.0.1,::1");
    if verbose { eprintln!("· network allow-list proxy on {url}"); }
    let _ = RUNNING.set(proxy);
    Ok(url)
}

/// The running proxy, if one was started.
pub fn running() -> Option<&'static Proxy> { RUNNING.get() }

pub async fn start(cfg: ProxyConfig) -> Result<Proxy> {
    let bind = cfg.bind.clone().unwrap_or_else(|| "127.0.0.1:0".to_string());
    let listener = TcpListener::bind(&bind).await.with_context(|| format!("binding the proxy to {bind}"))?;
    let addr = listener.local_addr()?.to_string();
    let cfg = Arc::new(cfg);
    let (allowed, blocked) = (Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)));
    let (c2, a2, b2) = (cfg.clone(), allowed.clone(), blocked.clone());
    let task = tokio::spawn(async move {
        loop {
            let Ok((sock, _)) = listener.accept().await else { break };
            let (c, a, b) = (c2.clone(), a2.clone(), b2.clone());
            tokio::spawn(async move { let _ = handle(sock, c, a, b).await; });
        }
    });
    Ok(Proxy { addr, cfg, allowed, blocked, task })
}

async fn deny(sock: &mut TcpStream, host: &str, why: &str) -> Result<()> {
    let body = format!("harness proxy: {why} ({host}). Allowed hosts come from [net.proxy] allow in harness.toml.");
    let resp = format!("HTTP/1.1 403 Forbidden\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    sock.write_all(resp.as_bytes()).await?;
    Ok(())
}

async fn handle(mut sock: TcpStream, cfg: Arc<ProxyConfig>, allowed: Arc<AtomicU64>, blocked: Arc<AtomicU64>) -> Result<()> {
    let mut reader = BufReader::new(&mut sock);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).await? == 0 { return Ok(()); }
    let mut parts = request_line.split_whitespace();
    let (method, target) = (parts.next().unwrap_or("").to_string(), parts.next().unwrap_or("").to_string());
    // headers
    let mut headers: Vec<(String, String)> = Vec::new();
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h).await? == 0 { break; }
        if h == "\r\n" || h == "\n" { break; }
        if let Some((k, v)) = h.split_once(':') { headers.push((k.trim().to_string(), v.trim().to_string())); }
    }
    let host_header = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("host")).map(|(_, v)| v.clone()).unwrap_or_default();

    if method.eq_ignore_ascii_case("CONNECT") {
        let (host, port) = target.rsplit_once(':').unwrap_or((target.as_str(), "443"));
        if !cfg.allows(host) {
            blocked.fetch_add(1, Ordering::Relaxed);
            if cfg.verbose { eprintln!("· proxy: blocked CONNECT {host}"); }
            drop(reader);
            return deny(&mut sock, host, "host is not on the network allow-list").await;
        }
        allowed.fetch_add(1, Ordering::Relaxed);
        if cfg.verbose { eprintln!("· proxy: CONNECT {host}:{port}"); }
        drop(reader);
        let mut upstream = match TcpStream::connect(format!("{host}:{port}")).await {
            Ok(s) => s,
            Err(e) => { let _ = deny(&mut sock, host, &format!("cannot connect: {e}")).await; return Ok(()); }
        };
        sock.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n").await?;
        let _ = tokio::io::copy_bidirectional(&mut sock, &mut upstream).await;
        return Ok(());
    }

    // absolute-form http:// request
    let (host, port, path) = match target.strip_prefix("http://") {
        Some(rest) => {
            let (hostport, path) = rest.split_once('/').map(|(h, p)| (h, format!("/{p}"))).unwrap_or((rest, "/".into()));
            let (h, p) = hostport.rsplit_once(':').unwrap_or((hostport, "80"));
            (h.to_string(), p.to_string(), path)
        }
        None => {
            let (h, p) = host_header.rsplit_once(':').unwrap_or((host_header.as_str(), "80"));
            (h.to_string(), p.to_string(), if target.is_empty() { "/".into() } else { target.clone() })
        }
    };
    if host.is_empty() || !cfg.allows(&host) {
        blocked.fetch_add(1, Ordering::Relaxed);
        if cfg.verbose { eprintln!("· proxy: blocked {method} {host}{path}"); }
        drop(reader);
        return deny(&mut sock, &host, "host is not on the network allow-list").await;
    }
    // credentials only travel to hosts that are supposed to see them
    let strip_credentials = !cfg.may_carry_credentials(&host);
    let mut out = format!("{method} {path} HTTP/1.1\r\n");
    let mut body_len = 0usize;
    let mut stripped: Vec<&str> = Vec::new();
    for (k, v) in &headers {
        let lk = k.to_lowercase();
        if lk == "proxy-connection" || lk == "proxy-authorization" { continue; }
        if strip_credentials && matches!(lk.as_str(), "authorization" | "cookie" | "x-api-key" | "api-key" | "x-auth-token") { stripped.push("credentials"); continue; }
        if lk == "content-length" { body_len = v.parse().unwrap_or(0); }
        out.push_str(&format!("{k}: {v}\r\n"));
    }
    out.push_str("Connection: close\r\n\r\n");
    let mut body = vec![0u8; body_len.min(8 * 1024 * 1024)];
    if body_len > 0 { reader.read_exact(&mut body).await?; }
    // a body carrying an obvious secret never leaves for a host that may not hold credentials
    if strip_credentials {
        let text = String::from_utf8_lossy(&body);
        if crate::security::redact(&text) != text {
            blocked.fetch_add(1, Ordering::Relaxed);
            drop(reader);
            return deny(&mut sock, &host, "the request body contains what looks like a credential and this host is not on credentials_to").await;
        }
    }
    allowed.fetch_add(1, Ordering::Relaxed);
    if cfg.verbose { eprintln!("· proxy: {method} {host}{path}{}", if stripped.is_empty() { String::new() } else { " (credentials stripped)".to_string() }); }
    drop(reader);
    let mut upstream = match TcpStream::connect(format!("{host}:{port}")).await {
        Ok(s) => s,
        Err(e) => { let _ = deny(&mut sock, &host, &format!("cannot connect: {e}")).await; return Ok(()); }
    };
    upstream.write_all(out.as_bytes()).await?;
    if !body.is_empty() { upstream.write_all(&body).await?; }
    let _ = tokio::io::copy_bidirectional(&mut sock, &mut upstream).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_globs() {
        assert!(host_match("github.com", "github.com"));
        assert!(!host_match("github.com", "evil-github.com"));
        assert!(host_match("*.githubusercontent.com", "raw.githubusercontent.com"));
        assert!(host_match("*.githubusercontent.com", "githubusercontent.com"));
        assert!(!host_match("*.githubusercontent.com", "githubusercontent.com.evil.net"));
        assert!(host_match("*", "anything.example"));
        let cfg = ProxyConfig { allow: vec!["example.com".into(), "*.internal".into()], ..Default::default() };
        assert!(cfg.allows("example.com") && cfg.allows("db.internal") && !cfg.allows("other.com"));
        // credentials default to the allow-list, and narrow when credentials_to is set
        assert!(cfg.may_carry_credentials("example.com"));
        let cfg2 = ProxyConfig { allow: vec!["*".into()], credentials_to: vec!["api.example.com".into()], ..Default::default() };
        assert!(cfg2.may_carry_credentials("api.example.com") && !cfg2.may_carry_credentials("evil.com"));
    }

    /// A real request through the proxy: allowed host passes, everything else gets a 403.
    #[tokio::test]
    async fn proxies_and_blocks() {
        // tiny origin server
        let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin_addr = origin.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut s, _)) = origin.accept().await else { break };
                tokio::spawn(async move {
                    let mut buf = [0u8; 2048];
                    let _ = s.read(&mut buf).await;
                    let saw_auth = String::from_utf8_lossy(&buf).to_lowercase().contains("authorization:");
                    let body = if saw_auth { "GOT-AUTH" } else { "hello-from-origin" };
                    let _ = s.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await;
                });
            }
        });
        let cfg = ProxyConfig { enabled: true, allow: vec!["127.0.0.1".into()], credentials_to: vec!["nobody.invalid".into()], ..Default::default() };
        let proxy = start(cfg).await.unwrap();

        let client = reqwest::Client::builder().proxy(reqwest::Proxy::http(proxy.url()).unwrap()).build().unwrap();
        let r = client.get(format!("http://127.0.0.1:{}/x", origin_addr.port())).header("Authorization", "Bearer secret").send().await.unwrap();
        assert_eq!(r.status(), 200);
        assert_eq!(r.text().await.unwrap(), "hello-from-origin", "the Authorization header must not reach a host outside credentials_to");

        let r = client.get("http://not-allowed.example/x").send().await.unwrap();
        assert_eq!(r.status(), 403);
        assert!(r.text().await.unwrap().contains("allow-list"));
        let (ok, blocked) = proxy.stats();
        assert!(ok >= 1 && blocked >= 1, "allowed={ok} blocked={blocked}");
    }
}
