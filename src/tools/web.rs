//! Internet access: fetch a URL as readable text, and web search over the provider of your choice —
//! Brave, Tavily, Exa, a self-hosted SearXNG, or key-less DuckDuckGo HTML scraping as the fallback.
//! Identical searches inside one session are answered from a small cache.

use super::{arg_str, Tool, ToolCtx, ToolOutput};
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::Duration;

pub struct WebFetch;
pub struct WebSearch;

fn client(ctx: &ToolCtx) -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .timeout(Duration::from_secs(ctx.net.timeout_secs))
        .user_agent(&ctx.net.user_agent)
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()?)
}

async fn get_text(ctx: &ToolCtx, url: &str) -> Result<(String, String)> {
    if !(url.starts_with("http://") || url.starts_with("https://")) { bail!("only http(s) URLs are allowed"); }
    let mut resp = client(ctx)?.get(url).send().await.with_context(|| format!("GET {url}"))?;
    let status = resp.status();
    let ctype = resp.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    // stream and stop at max_fetch_bytes: never buffer an arbitrarily large body
    let max = ctx.net.max_fetch_bytes;
    let mut bytes: Vec<u8> = Vec::new();
    while let Some(chunk) = resp.chunk().await? {
        let room = max.saturating_sub(bytes.len());
        bytes.extend_from_slice(&chunk[..chunk.len().min(room)]);
        if bytes.len() >= max { break; }
    }
    let body = String::from_utf8_lossy(&bytes).to_string();
    if !status.is_success() { bail!("HTTP {status} for {url}\n{}", crate::sandbox::truncate_middle(&body, 500)); }
    Ok((ctype, body))
}

/// Very small HTML -> text: drop script/style, turn block tags into newlines, strip tags, unescape a few entities.
pub fn html_to_text(html: &str) -> String {
    // single pass over a lowercased copy (same byte offsets: ASCII lowering keeps char boundaries)
    let lower = html.to_ascii_lowercase();
    let mut s = String::with_capacity(html.len());
    let mut i = 0;
    'outer: while i < html.len() {
        if lower[i..].starts_with('<') {
            for tag in ["script", "style", "noscript", "svg", "head"] {
                let open = format!("<{tag}");
                if lower[i..].starts_with(&open) && !lower[i + open.len()..].starts_with(|c: char| c.is_ascii_alphanumeric()) {
                    let close = format!("</{tag}>");
                    match lower[i..].find(&close) { Some(b) => { s.push('\n'); i += b + close.len(); continue 'outer; } None => break 'outer }
                }
            }
        }
        let n = html[i..].chars().next().map(char::len_utf8).unwrap_or(1);
        s.push_str(&html[i..i + n]); i += n;
    }
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    let mut tag = String::new();
    for ch in s.chars() {
        if in_tag {
            if ch == '>' {
                in_tag = false;
                let t = tag.trim_start_matches('/').split_whitespace().next().unwrap_or("").to_ascii_lowercase();
                if matches!(t.as_str(), "p"|"div"|"br"|"li"|"tr"|"h1"|"h2"|"h3"|"h4"|"h5"|"h6"|"pre"|"blockquote"|"section"|"article"|"ul"|"ol"|"table"|"hr") { out.push('\n'); }
                if matches!(t.as_str(), "td"|"th") { out.push('\t'); }
                tag.clear();
            } else { tag.push(ch); }
        } else if ch == '<' { in_tag = true; } else { out.push(ch); }
    }
    let out = out.replace("&nbsp;", " ").replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&#x27;", "'");
    // collapse whitespace
    let mut res = String::with_capacity(out.len());
    let mut blank = 0;
    for line in out.lines() {
        let l = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if l.is_empty() { blank += 1; if blank <= 1 { res.push('\n'); } } else { blank = 0; res.push_str(&l); res.push('\n'); }
    }
    res.trim().to_string()
}

#[async_trait]
impl Tool for WebFetch {
    fn read_only(&self) -> bool { true }
    fn name(&self) -> &'static str { "web_fetch" }
    fn description(&self) -> &'static str { "Fetch a URL over HTTP(S) and return its content as readable text (HTML is converted to text; JSON/plain returned as-is). Use for docs, APIs, raw files from GitHub, etc." }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{"url":{"type":"string"},"raw":{"type":"boolean","description":"return raw body without HTML->text conversion"}},"required":["url"]})
    }
    async fn call(&self, args: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let url = arg_str(&args, "url")?;
        let raw = args.get("raw").and_then(|v| v.as_bool()).unwrap_or(false);
        let (ctype, body) = get_text(ctx, url).await?;
        let text = if !raw && (ctype.contains("html") || body.trim_start().starts_with('<')) { html_to_text(&body) } else { body };
        Ok((crate::sandbox::truncate_middle(&text, ctx.max_output)).into())
    }
}

#[async_trait]
impl Tool for WebSearch {
    fn read_only(&self) -> bool { true }
    fn name(&self) -> &'static str { "web_search" }
    fn description(&self) -> &'static str { "Search the web. Returns a list of results (title, URL, snippet). Follow up with web_fetch to read a result." }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{"query":{"type":"string"},"max_results":{"type":"integer","description":"default 8"},"provider":{"type":"string","enum":["auto","brave","tavily","exa","searxng","duckduckgo"],"description":"default: whatever is configured"}},"required":["query"]})
    }
    async fn call(&self, args: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let q = arg_str(&args, "query")?;
        let max = args.get("max_results").and_then(|v| v.as_u64()).unwrap_or(8) as usize;
        // repeated identical searches inside one session are answered from a small cache
        if let Some(hit) = cache_get(q, max) { return Ok(format!("{hit}\n\n[cached result of an identical search in this session]").into()); }
        let provider = args.get("provider").and_then(|v| v.as_str()).map(|s| s.to_string())
            .unwrap_or_else(|| ctx.net.search_provider.clone().unwrap_or_else(|| "auto".into()));
        let (used, results) = search(ctx, &provider, q, max).await?;
        if results.is_empty() {
            return Ok(format!("no results from {used}. Try rephrasing, another provider (web_search {{provider:\"brave|tavily|exa|searxng|duckduckgo\"}}), or web_fetch a known URL.").into());
        }
        let text = format!("results from {used}:\n\n{}", results.iter().enumerate().map(|(i, r)| format!("{}. {}\n   {}\n   {}", i + 1, r.0, r.1, r.2)).collect::<Vec<_>>().join("\n\n"));
        cache_put(q, max, &text);
        Ok(text.into())
    }
}

/// (title, url, snippet)
type Hit = (String, String, String);

/// Search with the configured provider. "auto" uses whichever API key is present, else DuckDuckGo.
async fn search(ctx: &ToolCtx, provider: &str, q: &str, max: usize) -> Result<(String, Vec<Hit>)> {
    let key = |name: &str| ctx.net.search_api_key.clone().filter(|_| true).or_else(|| std::env::var(name).ok()).filter(|k| !k.trim().is_empty());
    let chosen = match provider.trim().to_lowercase().as_str() {
        "auto" | "" => {
            if std::env::var("BRAVE_API_KEY").is_ok() { "brave" }
            else if std::env::var("TAVILY_API_KEY").is_ok() { "tavily" }
            else if std::env::var("EXA_API_KEY").is_ok() { "exa" }
            else if ctx.net.searxng_url.is_some() { "searxng" }
            else { "duckduckgo" }
        }
        other => match other { "brave" => "brave", "tavily" => "tavily", "exa" => "exa", "searxng" | "searx" => "searxng", "ddg" | "duckduckgo" => "duckduckgo", o => anyhow::bail!("unknown search provider '{o}' (brave|tavily|exa|searxng|duckduckgo)") },
    };
    let http = client(ctx)?;
    match chosen {
        "brave" => {
            let k = key("BRAVE_API_KEY").context("BRAVE_API_KEY is not set")?;
            let v: Value = http.get(format!("https://api.search.brave.com/res/v1/web/search?q={}&count={max}", urlencode(q)))
                .header("X-Subscription-Token", k).header("Accept", "application/json").send().await?.error_for_status()?.json().await?;
            Ok(("brave".into(), v["web"]["results"].as_array().map(|a| a.iter().take(max).map(|r| (
                r["title"].as_str().unwrap_or("").to_string(), r["url"].as_str().unwrap_or("").to_string(),
                html_to_text(r["description"].as_str().unwrap_or("")).trim().to_string())).collect()).unwrap_or_default()))
        }
        "tavily" => {
            let k = key("TAVILY_API_KEY").context("TAVILY_API_KEY is not set")?;
            let v: Value = http.post("https://api.tavily.com/search").json(&json!({"api_key": k, "query": q, "max_results": max, "include_answer": false}))
                .send().await?.error_for_status()?.json().await?;
            Ok(("tavily".into(), v["results"].as_array().map(|a| a.iter().take(max).map(|r| (
                r["title"].as_str().unwrap_or("").to_string(), r["url"].as_str().unwrap_or("").to_string(),
                r["content"].as_str().unwrap_or("").trim().to_string())).collect()).unwrap_or_default()))
        }
        "exa" => {
            let k = key("EXA_API_KEY").context("EXA_API_KEY is not set")?;
            let v: Value = http.post("https://api.exa.ai/search").header("x-api-key", k)
                .json(&json!({"query": q, "numResults": max, "contents": {"text": {"maxCharacters": 400}}}))
                .send().await?.error_for_status()?.json().await?;
            Ok(("exa".into(), v["results"].as_array().map(|a| a.iter().take(max).map(|r| (
                r["title"].as_str().unwrap_or("").to_string(), r["url"].as_str().unwrap_or("").to_string(),
                r["text"].as_str().unwrap_or("").trim().chars().take(400).collect())).collect()).unwrap_or_default()))
        }
        "searxng" => {
            let base = ctx.net.searxng_url.clone().or_else(|| std::env::var("SEARXNG_URL").ok()).context("set [net] searxng_url or $SEARXNG_URL")?;
            let v: Value = http.get(format!("{}/search?q={}&format=json", base.trim_end_matches('/'), urlencode(q))).send().await?.error_for_status()?.json().await?;
            Ok(("searxng".into(), v["results"].as_array().map(|a| a.iter().take(max).map(|r| (
                r["title"].as_str().unwrap_or("").to_string(), r["url"].as_str().unwrap_or("").to_string(),
                r["content"].as_str().unwrap_or("").trim().to_string())).collect()).unwrap_or_default()))
        }
        _ => {
            let url = format!("https://html.duckduckgo.com/html/?q={}", urlencode(q));
            let (_, body) = get_text(ctx, &url).await?;
            Ok(("duckduckgo".into(), parse_ddg(&body, max)))
        }
    }
}

/// Tiny per-process search cache: the same query twice in a session costs one request.
fn cache() -> &'static std::sync::Mutex<std::collections::HashMap<String, (std::time::Instant, String)>> {
    static C: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, (std::time::Instant, String)>>> = std::sync::OnceLock::new();
    C.get_or_init(Default::default)
}
fn cache_get(q: &str, max: usize) -> Option<String> {
    let g = cache().lock().ok()?;
    let (at, text) = g.get(&format!("{max}:{q}"))?;
    (at.elapsed() < std::time::Duration::from_secs(900)).then(|| text.clone())
}
fn cache_put(q: &str, max: usize, text: &str) {
    if let Ok(mut g) = cache().lock() {
        if g.len() > 64 { g.clear(); }
        g.insert(format!("{max}:{q}"), (std::time::Instant::now(), text.to_string()));
    }
}

fn urlencode(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => o.push(b as char),
            b' ' => o.push('+'),
            _ => o.push_str(&format!("%{:02X}", b)),
        }
    }
    o
}

/// Parse DuckDuckGo HTML results: <a class="result__a" href="…">title</a> … <a class="result__snippet">…</a>
fn parse_ddg(html: &str, max: usize) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(i) = rest.find("class=\"result__a\"") {
        let seg = &rest[i..];
        let Some(href_i) = seg.find("href=\"") else { break };
        let href_seg = &seg[href_i + 6..];
        let Some(href_end) = href_seg.find('"') else { break };
        let mut href = href_seg[..href_end].to_string();
        // DDG wraps: //duckduckgo.com/l/?uddg=<encoded>&rut=…
        if let Some(u) = href.find("uddg=") {
            let enc = &href[u + 5..];
            let enc = enc.split('&').next().unwrap_or(enc);
            href = urldecode(enc);
        }
        let Some(gt) = href_seg.find('>') else { break };
        let title_seg = &href_seg[gt + 1..];
        let Some(a_end) = title_seg.find("</a>") else { break };
        let title = html_to_text(&title_seg[..a_end]);
        let after = &title_seg[a_end..];
        let snippet = after.find("result__snippet").and_then(|s| {
            let s2 = &after[s..];
            let gt = s2.find('>')?;
            let end = s2[gt..].find("</a>").or_else(|| s2[gt..].find("</div>"))?;
            Some(html_to_text(&s2[gt + 1..gt + end]))
        }).unwrap_or_default();
        if !href.is_empty() && !title.is_empty() { out.push((title, href, snippet)); }
        if out.len() >= max { break; }
        rest = after;
    }
    out
}

fn urldecode(s: &str) -> String {
    let b = s.as_bytes();
    let mut o = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            // byte-wise: `%` followed by two ASCII hex digits (never slices into a multibyte char)
            if let Some(v) = b.get(i + 1..i + 3).filter(|h| h.iter().all(u8::is_ascii_hexdigit)).and_then(|h| u8::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok()) { o.push(v); i += 3; continue; }
        }
        if b[i] == b'+' { o.push(b' '); } else { o.push(b[i]); }
        i += 1;
    }
    String::from_utf8_lossy(&o).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn html_text() {
        let t = html_to_text("<html><head><title>x</title><style>p{}</style></head><body><h1>Hi</h1><p>a &amp; b</p><script>1</script></body></html>");
        assert_eq!(t, "Hi\n\na & b");
    }
    #[test]
    fn html_text_case_and_unclosed() {
        assert_eq!(html_to_text("<SCRIPT>x</Script><P>ok</p><scripts>keep</scripts>"), "ok\nkeep");
        assert_eq!(html_to_text("a<style>never closed"), "a");
        assert_eq!(html_to_text("é<b>ü</b>"), "éü");
    }
    #[test]
    fn urldecode_multibyte_no_panic() {
        assert_eq!(urldecode("%e2%82%ac%zz€"), "€%zz€");
        assert_eq!(urldecode("a+b%2"), "a b%2");
        assert_eq!(urldecode("%C3%A9%"), "é%");
        assert_eq!(urldecode("%€"), "%€");
    }
    #[test]
    fn ddg_parse() {
        let h = r##"<a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fx&amp;rut=1">Ex <b>ample</b></a><a class="result__snippet" href="#">snip here</a>"##;
        let r = parse_ddg(h, 5);
        assert_eq!(r.len(), 1); assert_eq!(r[0].1, "https://example.com/x"); assert_eq!(r[0].0, "Ex ample"); assert_eq!(r[0].2, "snip here");
    }
}
