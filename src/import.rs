//! Import transcripts from other agents into our session store, so `/resume` and `--resume` can pick
//! up a conversation that started in Claude Code or Codex (and so the eval/memory tooling can mine
//! them). Both formats are line-delimited JSON; we keep the parts that map onto our Message model and
//! skip provider bookkeeping.
//!
//!   Claude Code: ~/.claude/projects/<slug>/<uuid>.jsonl  — {type:"user"|"assistant", message:{…}, cwd}
//!   Codex:       ~/.codex/sessions/<y>/<m>/<d>/rollout-*.jsonl — {type:"response_item", payload:{…}}

use crate::llm::{Content, FunctionCall, Message, ToolCall};
use crate::sessions::{Meta, SessionStore};
use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, serde::Serialize)]
pub struct Imported { pub id: String, pub title: String, pub workdir: String, pub messages: usize, pub source: &'static str, pub file: String }

pub fn claude_root() -> PathBuf { crate::setup::home_dir().join(".claude").join("projects") }
pub fn codex_root() -> PathBuf { crate::setup::home_dir().join(".codex").join("sessions") }

/// Which agent wrote this file (by content, not by path).
pub fn detect(path: &Path) -> Result<&'static str> {
    let text = head(path, 64 * 1024)?;
    for line in text.lines().take(50) {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        match v["type"].as_str() {
            Some("session_meta") | Some("response_item") | Some("event_msg") => return Ok("codex"),
            Some("user") | Some("assistant") if v.get("message").is_some() => return Ok("claude"),
            Some("bridge-session") | Some("file-history-snapshot") | Some("ai-title") => return Ok("claude"),
            _ => {}
        }
    }
    bail!("{} does not look like a Claude Code or Codex transcript", path.display())
}

fn head(path: &Path, n: usize) -> Result<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut buf = vec![0u8; n];
    let read = f.read(&mut buf)?;
    buf.truncate(read);
    Ok(String::from_utf8_lossy(&buf).to_string())
}

/// Parse one transcript into (meta, messages) without saving it.
pub fn parse(path: &Path) -> Result<(Meta, Vec<Message>)> {
    match detect(path)? {
        "codex" => parse_codex(path),
        _ => parse_claude(path),
    }
}

fn text_of(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts.iter().filter_map(|p| match p["type"].as_str() {
            Some("text") | Some("input_text") | Some("output_text") => p["text"].as_str().map(|s| s.to_string()),
            _ => None,
        }).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}

fn parse_claude(path: &Path) -> Result<(Meta, Vec<Message>)> {
    let text = std::fs::read_to_string(path)?;
    let mut msgs: Vec<Message> = Vec::new();
    let mut meta = Meta { id: format!("cc-{}", stem(path)), model: String::new(), ..Default::default() };
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        if meta.workdir.is_empty() { if let Some(cwd) = v["cwd"].as_str() { meta.workdir = cwd.to_string(); } }
        match v["type"].as_str() {
            Some("user") => {
                let content = &v["message"]["content"];
                // tool results arrive as user messages in the Anthropic shape
                let mut had_tool = false;
                if let Value::Array(parts) = content {
                    for p in parts {
                        if p["type"] == "tool_result" {
                            had_tool = true;
                            let id = p["tool_use_id"].as_str().unwrap_or("").to_string();
                            let name = msgs.iter().rev().find_map(|m| m.tool_calls.as_ref().and_then(|c| c.iter().find(|c| c.id == id).map(|c| c.function.name.clone()))).unwrap_or_default();
                            msgs.push(Message::tool(id, name, text_of(&p["content"])));
                        }
                    }
                }
                if had_tool { continue; }
                let t = text_of(content);
                if !t.trim().is_empty() { msgs.push(Message::user(t)); }
            }
            Some("assistant") => {
                if meta.model.is_empty() { if let Some(m) = v["message"]["model"].as_str() { meta.model = m.to_string(); } }
                let content = &v["message"]["content"];
                let t = text_of(content);
                let mut calls: Vec<ToolCall> = Vec::new();
                if let Value::Array(parts) = content {
                    for p in parts {
                        if p["type"] == "tool_use" {
                            calls.push(ToolCall {
                                id: p["id"].as_str().unwrap_or("").to_string(),
                                kind: "function".into(),
                                function: FunctionCall { name: p["name"].as_str().unwrap_or("").to_string(), arguments: p["input"].to_string() },
                            });
                        }
                    }
                }
                if t.trim().is_empty() && calls.is_empty() { continue; }
                msgs.push(Message { role: "assistant".into(), content: (!t.trim().is_empty()).then(|| Content::Text(t)), tool_calls: (!calls.is_empty()).then_some(calls), ..Default::default() });
            }
            _ => {}
        }
    }
    if msgs.is_empty() { bail!("no messages found in {}", path.display()); }
    Ok((meta, msgs))
}

fn parse_codex(path: &Path) -> Result<(Meta, Vec<Message>)> {
    let text = std::fs::read_to_string(path)?;
    let mut msgs: Vec<Message> = Vec::new();
    let mut meta = Meta { id: format!("cx-{}", stem(path)), ..Default::default() };
    let mut call_names: std::collections::HashMap<String, String> = Default::default();
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        match v["type"].as_str() {
            Some("session_meta") => {
                let p = &v["payload"];
                if let Some(c) = p["cwd"].as_str() { meta.workdir = c.to_string(); }
                if let Some(m) = p["model"].as_str().or_else(|| p["model_provider"].as_str()) { meta.model = m.to_string(); }
            }
            Some("response_item") => {
                let p = &v["payload"];
                match p["type"].as_str() {
                    Some("message") => {
                        let role = p["role"].as_str().unwrap_or("");
                        if role != "user" && role != "assistant" { continue; } // developer/system = the harness's own prompt
                        let t = text_of(&p["content"]);
                        if t.trim().is_empty() { continue; }
                        msgs.push(if role == "user" { Message::user(t) } else { Message { role: "assistant".into(), content: Some(Content::Text(t)), ..Default::default() } });
                    }
                    Some("function_call") | Some("custom_tool_call") => {
                        let id = p["call_id"].as_str().or_else(|| p["id"].as_str()).unwrap_or("").to_string();
                        let name = p["name"].as_str().unwrap_or("tool").to_string();
                        let arguments = p["arguments"].as_str().map(|s| s.to_string()).unwrap_or_else(|| p["input"].as_str().map(|s| s.to_string()).unwrap_or_else(|| p["arguments"].to_string()));
                        call_names.insert(id.clone(), name.clone());
                        msgs.push(Message { role: "assistant".into(), content: None, tool_calls: Some(vec![ToolCall { id, kind: "function".into(), function: FunctionCall { name, arguments } }]), ..Default::default() });
                    }
                    Some("function_call_output") | Some("custom_tool_call_output") => {
                        let id = p["call_id"].as_str().unwrap_or("").to_string();
                        let name = call_names.get(&id).cloned().unwrap_or_default();
                        let out = p["output"].as_str().map(|s| s.to_string()).unwrap_or_else(|| p["output"].to_string());
                        msgs.push(Message::tool(id, name, out));
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    if msgs.is_empty() { bail!("no messages found in {}", path.display()); }
    Ok((meta, msgs))
}

fn stem(path: &Path) -> String {
    let s = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    s.rsplit('-').take(2).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("-")
}

/// Import one file into the session store.
pub fn import_file(path: &Path) -> Result<Imported> {
    let source = detect(path)?;
    let (mut meta, msgs) = parse(path)?;
    if meta.workdir.is_empty() { meta.workdir = std::env::current_dir().map(|p| p.display().to_string()).unwrap_or_default(); }
    let store = SessionStore::open()?;
    store.save(&mut meta, &msgs)?;
    Ok(Imported { id: meta.id.clone(), title: meta.title.clone(), workdir: meta.workdir.clone(), messages: msgs.len(), source, file: path.display().to_string() })
}

/// Every transcript under the well-known directories, newest first.
pub fn discover(sources: &[&str]) -> Vec<PathBuf> {
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    let mut walk = |root: PathBuf| {
        fn rec(dir: &Path, depth: usize, out: &mut Vec<(std::time::SystemTime, PathBuf)>) {
            if depth > 5 { return; }
            let Ok(rd) = std::fs::read_dir(dir) else { return };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() { rec(&p, depth + 1, out); }
                else if p.extension().map(|x| x == "jsonl").unwrap_or(false) {
                    let t = e.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
                    out.push((t, p));
                }
            }
        }
        rec(&root, 0, &mut files);
    };
    if sources.contains(&"claude") { walk(claude_root()); }
    if sources.contains(&"codex") { walk(codex_root()); }
    files.sort_by(|a, b| b.0.cmp(&a.0));
    files.into_iter().map(|(_, p)| p).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_both_formats() {
        let d = std::env::temp_dir().join(format!("harness-import-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();

        let cc = d.join("abc-1234.jsonl");
        std::fs::write(&cc, concat!(
            r#"{"type":"bridge-session","sessionId":"x"}"#, "\n",
            r#"{"type":"user","cwd":"/proj","message":{"role":"user","content":"fix the bug"}}"#, "\n",
            r#"{"type":"assistant","message":{"role":"assistant","model":"claude-opus-5","content":[{"type":"text","text":"looking"},{"type":"tool_use","id":"t1","name":"Read","input":{"path":"a.rs"}}]}}"#, "\n",
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":[{"type":"text","text":"fn main(){}"}]}]}}"#, "\n",
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"fixed"}]}}"#, "\n")).unwrap();
        assert_eq!(detect(&cc).unwrap(), "claude");
        let (meta, msgs) = parse(&cc).unwrap();
        assert_eq!(meta.workdir, "/proj");
        assert_eq!(meta.model, "claude-opus-5");
        assert_eq!(msgs.len(), 4);
        assert_eq!(msgs[0].text(), "fix the bug");
        assert_eq!(msgs[1].tool_calls.as_ref().unwrap()[0].function.name, "Read");
        assert_eq!(msgs[2].role, "tool");
        assert_eq!(msgs[2].name.as_deref(), Some("Read"), "the tool name is recovered from the matching call");
        assert_eq!(msgs[3].text(), "fixed");

        let cx = d.join("rollout-2026-08-10T12-00-18-019feb1d.jsonl");
        std::fs::write(&cx, concat!(
            r#"{"type":"session_meta","payload":{"cwd":"/work","model":"gpt-5"}}"#, "\n",
            r#"{"type":"event_msg","payload":{"type":"noise"}}"#, "\n",
            r#"{"type":"response_item","payload":{"type":"message","role":"developer","content":[{"type":"input_text","text":"system prompt"}]}}"#, "\n",
            r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"do it"}]}}"#, "\n",
            r#"{"type":"response_item","payload":{"type":"function_call","call_id":"c1","name":"exec_command","arguments":"{\"cmd\":\"ls\"}"}}"#, "\n",
            r#"{"type":"response_item","payload":{"type":"function_call_output","call_id":"c1","output":"a.rs"}}"#, "\n",
            r#"{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"done"}]}}"#, "\n")).unwrap();
        assert_eq!(detect(&cx).unwrap(), "codex");
        let (meta, msgs) = parse(&cx).unwrap();
        assert_eq!(meta.workdir, "/work");
        assert_eq!(msgs.len(), 4, "the developer prompt is skipped: {msgs:?}");
        assert_eq!(msgs[0].text(), "do it");
        assert_eq!(msgs[1].tool_calls.as_ref().unwrap()[0].function.name, "exec_command");
        assert_eq!(msgs[2].text(), "a.rs");
        assert_eq!(msgs[3].text(), "done");

        std::fs::write(d.join("other.jsonl"), "{\"hello\":1}\n").unwrap();
        assert!(detect(&d.join("other.jsonl")).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }
}
