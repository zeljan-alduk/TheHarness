//! Internet access: fetch a URL as readable text, and a key-less web search (DuckDuckGo HTML).

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
    let resp = client(ctx)?.get(url).send().await.with_context(|| format!("GET {url}"))?;
    let status = resp.status();
    let ctype = resp.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let bytes = resp.bytes().await?;
    let bytes = if bytes.len() > ctx.net.max_fetch_bytes { &bytes[..ctx.net.max_fetch_bytes] } else { &bytes[..] };
    let body = String::from_utf8_lossy(bytes).to_string();
    if !status.is_success() { bail!("HTTP {status} for {url}\n{}", crate::sandbox::truncate_middle(&body, 500)); }
    Ok((ctype, body))
}

/// Very small HTML -> text: drop script/style, turn block tags into newlines, strip tags, unescape a few entities.
pub fn html_to_text(html: &str) -> String {
    let mut s = html.to_string();
    for tag in ["script", "style", "noscript", "svg", "head"] {
        loop {
            let lower = s.to_ascii_lowercase();
            let Some(a) = lower.find(&format!("<{tag}")) else { break };
            let Some(b) = lower[a..].find(&format!("</{tag}>")) else { s.truncate(a); break; };
            s.replace_range(a..a + b + tag.len() + 3, "\n");
        }
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
        json!({"type":"object","properties":{"query":{"type":"string"},"max_results":{"type":"integer","description":"default 8"}},"required":["query"]})
    }
    async fn call(&self, args: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let q = arg_str(&args, "query")?;
        let max = args.get("max_results").and_then(|v| v.as_u64()).unwrap_or(8) as usize;
        let url = format!("https://html.duckduckgo.com/html/?q={}", urlencode(q));
        let (_, body) = get_text(ctx, &url).await?;
        let results = parse_ddg(&body, max);
        if results.is_empty() {
            return Ok(format!("no results parsed (page was {} bytes). Try rephrasing, or web_fetch a known URL.", body.len()).into());
        }
        Ok((results.iter().enumerate().map(|(i, r)| format!("{}. {}\n   {}\n   {}", i + 1, r.0, r.1, r.2)).collect::<Vec<_>>().join("\n\n")).into())
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
        if b[i] == b'%' && i + 2 < b.len() + 0 && i + 2 <= b.len() - 1 {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) { o.push(v); i += 3; continue; }
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
    fn ddg_parse() {
        let h = r##"<a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fx&amp;rut=1">Ex <b>ample</b></a><a class="result__snippet" href="#">snip here</a>"##;
        let r = parse_ddg(h, 5);
        assert_eq!(r.len(), 1); assert_eq!(r[0].1, "https://example.com/x"); assert_eq!(r[0].0, "Ex ample"); assert_eq!(r[0].2, "snip here");
    }
}
