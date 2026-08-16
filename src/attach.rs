//! `harness attach <url>`: a thin terminal client for a running `harness serve`. The session lives in
//! the server process — close the client, reconnect later, or attach from another machine (or a phone
//! over SSH) and the run keeps going. Events stream in over SSE; prompts, permission answers and stop
//! requests go back over the same token-authenticated JSON API the web UI uses.

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::io::Write;
use std::sync::{Arc, Mutex};

/// Split a URL like `http://host:7878/?token=abc` into (base, token).
pub fn split_url(url: &str, token: Option<&str>) -> Result<(String, String)> {
    let u = url.trim();
    let u = if u.starts_with("http://") || u.starts_with("https://") { u.to_string() } else { format!("http://{u}") };
    let (base, query) = u.split_once('?').unwrap_or((u.as_str(), ""));
    let base = base.trim_end_matches('/').to_string();
    let tok = token.map(|t| t.to_string())
        .or_else(|| query.split('&').find_map(|kv| kv.strip_prefix("token=")).map(|t| t.to_string()))
        .or_else(|| std::env::var("HARNESS_TOKEN").ok())
        .unwrap_or_default();
    if tok.is_empty() { bail!("no token — pass --token, or use the URL that `harness serve` printed (it carries ?token=…)"); }
    Ok((base, tok))
}

/// Render one server event as a line for the terminal (None = nothing worth printing).
pub fn render_event(name: &str, payload: &Value) -> Option<String> {
    match name {
        "agent-event" => {
            let e = payload;
            match e["type"].as_str().unwrap_or("") {
                "tool_call" => Some(format!("▶ {} {}", e["name"].as_str().unwrap_or(""), crate::llm::truncate_for_log(&e["args"].as_str().unwrap_or("").replace('\n', "\\n"), 160))),
                "tool_result" => Some(format!("◀ {} ({:.1}s) {}", e["name"].as_str().unwrap_or(""), e["secs"].as_f64().unwrap_or(0.0), crate::llm::truncate_for_log(&e["result"].as_str().unwrap_or("").replace('\n', "⏎"), 160))),
                "assistant" => Some(format!("\n{}\n", e["text"].as_str().unwrap_or(""))),
                "error" => Some(format!("✖ {}", e["message"].as_str().unwrap_or(""))),
                "permission" => Some(format!("🔒 {} → {}", e["tool"].as_str().unwrap_or(""), e["decision"].as_str().unwrap_or(""))),
                "compacted" => Some(format!("⟲ compacted {} messages", e["count"].as_u64().unwrap_or(0))),
                "run_finished" => Some(format!("— {} turns, {} tool calls, {:.0}s", e["turns"].as_u64().unwrap_or(0), e["tool_calls"].as_u64().unwrap_or(0), e["wall_secs"].as_f64().unwrap_or(0.0))),
                _ => None,
            }
        }
        "permission-ask" => Some(format!("🔒 {}({}) — {} · answer y / a (always) / n",
            payload["tool"].as_str().unwrap_or(""), crate::llm::truncate_for_log(payload["summary"].as_str().unwrap_or(""), 120), payload["reason"].as_str().unwrap_or(""))),
        "run-finished" => Some(match payload["ok"].as_bool().unwrap_or(false) {
            true => format!("\n✓ {}\n", payload["text"].as_str().unwrap_or("")),
            false => format!("\n✖ {}\n", payload["error"].as_str().unwrap_or("failed")),
        }),
        "warning" => Some(format!("· {}", payload.as_str().unwrap_or(""))),
        _ => None,
    }
}

async fn invoke(http: &reqwest::Client, base: &str, token: &str, cmd: &str, args: Value) -> Result<Value> {
    let host = base.trim_start_matches("http://").trim_start_matches("https://").to_string();
    let r = http.post(format!("{base}/api/invoke"))
        .header("X-Harness-Token", token)
        .header("Host", host)
        .json(&json!({"cmd": cmd, "args": args}))
        .send().await.with_context(|| format!("POST {base}/api/invoke"))?;
    let status = r.status();
    let v: Value = r.json().await.unwrap_or(Value::Null);
    if !status.is_success() { bail!("{cmd}: {status} {}", crate::llm::truncate_for_log(&v.to_string(), 300)); }
    if let Some(e) = v.get("error").and_then(|e| e.as_str()) { bail!("{cmd}: {e}"); }
    Ok(v.get("result").cloned().unwrap_or(v))
}

/// Attach to `url` and drive it from this terminal until stdin closes.
pub async fn attach(url: &str, token: Option<&str>, workdir: Option<String>, first_task: Option<String>) -> Result<()> {
    use tokio::io::AsyncBufReadExt;
    let (base, token) = split_url(url, token)?;
    let http = reqwest::Client::builder().timeout(std::time::Duration::from_secs(30)).build()?;
    let cfg = invoke(&http, &base, &token, "get_config", json!({})).await.context("cannot reach the server (is `harness serve` running, and is the token right?)")?;
    let workdir = workdir.unwrap_or_else(|| cfg["cwd"].as_str().unwrap_or(".").to_string());
    println!("attached to {base} · harness {} · {}", cfg["version"].as_str().unwrap_or("?"), workdir);
    println!("type a task and press enter · /stop interrupts · /quit detaches (the session keeps running)\n");

    // pending permission prompt id, answered by the next line of input
    let pending: Arc<Mutex<Option<u64>>> = Arc::new(Mutex::new(None));

    // events → stdout
    let (b2, t2, p2) = (base.clone(), token.clone(), pending.clone());
    let events = tokio::spawn(async move {
        let http = reqwest::Client::builder().build().unwrap();
        loop {
            let host = b2.trim_start_matches("http://").trim_start_matches("https://").to_string();
            let r = http.get(format!("{b2}/api/events?token={t2}")).header("Host", host).send().await;
            match r {
                Ok(resp) if resp.status().is_success() => {
                    let mut stream = futures_util::StreamExt::boxed(resp.bytes_stream());
                    let mut buf = String::new();
                    while let Some(chunk) = futures_util::StreamExt::next(&mut stream).await {
                        let Ok(chunk) = chunk else { break };
                        buf.push_str(&String::from_utf8_lossy(&chunk));
                        while let Some(pos) = buf.find('\n') {
                            let line = buf[..pos].trim().to_string();
                            buf.drain(..=pos);
                            let Some(data) = line.strip_prefix("data:") else { continue };
                            let Ok(v) = serde_json::from_str::<Value>(data.trim()) else { continue };
                            let name = v["name"].as_str().unwrap_or("");
                            if name == "permission-ask" { *p2.lock().unwrap() = v["payload"]["id"].as_u64(); }
                            if let Some(text) = render_event(name, &v["payload"]) {
                                println!("{text}");
                                let _ = std::io::stdout().flush();
                            }
                        }
                    }
                }
                Ok(resp) => { eprintln!("· event stream: {}", resp.status()); }
                Err(e) => { eprintln!("· event stream lost ({e}); reconnecting in 3s"); }
            }
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        }
    });

    if let Some(t) = first_task {
        invoke(&http, &base, &token, "start_run", json!({"task": t, "workdir": workdir})).await?;
    }

    let mut lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();
    while let Some(line) = lines.next_line().await? {
        let line = line.trim().to_string();
        if line.is_empty() { continue; }
        // answering a permission prompt
        let ask = *pending.lock().unwrap();
        if let Some(id) = ask {
            let decision = match line.chars().next().unwrap_or('n') { 'y' | 'Y' => "once", 'a' | 'A' => "always", _ => "deny" };
            *pending.lock().unwrap() = None;
            invoke(&http, &base, &token, "answer_permission", json!({"id": id, "decision": decision})).await?;
            continue;
        }
        match line.as_str() {
            "/quit" | "/detach" | "/exit" => break,
            "/stop" => { invoke(&http, &base, &token, "stop_run", json!({})).await?; continue; }
            "/status" => { let c = invoke(&http, &base, &token, "get_config", json!({})).await?; println!("· {} · model {} · {}", c["version"].as_str().unwrap_or("?"), c["llm"]["model"].as_str().unwrap_or("?"), c["cwd"].as_str().unwrap_or("")); continue; }
            _ => {}
        }
        if let Err(e) = invoke(&http, &base, &token, "start_run", json!({"task": line, "workdir": workdir})).await { eprintln!("✖ {e:#}"); }
    }
    events.abort();
    println!("detached — the session keeps running on the server");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_and_tokens() {
        let (b, t) = split_url("http://127.0.0.1:7878/?token=abc123", None).unwrap();
        assert_eq!(b, "http://127.0.0.1:7878");
        assert_eq!(t, "abc123");
        let (b, t) = split_url("192.168.1.9:7878", Some("zz")).unwrap();
        assert_eq!((b.as_str(), t.as_str()), ("http://192.168.1.9:7878", "zz"));
        assert!(split_url("http://x:1/", None).is_err(), "a token is required");
    }

    #[test]
    fn renders_events() {
        let e = json!({"type": "tool_call", "name": "bash", "args": "{\"cmd\":\"ls\"}"});
        assert!(render_event("agent-event", &e).unwrap().starts_with("▶ bash"));
        let e = json!({"type": "reasoning_delta", "text": "…"});
        assert!(render_event("agent-event", &e).is_none(), "deltas stay quiet");
        let ask = json!({"id": 3, "tool": "bash", "summary": "rm -rf build", "reason": "risky"});
        assert!(render_event("permission-ask", &ask).unwrap().contains("answer y / a"));
        assert!(render_event("run-finished", &json!({"ok": true, "text": "done"})).unwrap().contains("done"));
    }
}
