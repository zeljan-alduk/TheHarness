//! `harness mcp-serve`: expose this harness *as* an MCP server, so another agent (Claude Code, Codex,
//! Cursor, anything that speaks MCP) can hand work to it. Two tools: `harness` runs a task to
//! completion in a working directory and returns the answer; `harness_ask` answers a question about a
//! repository without changing anything.

use crate::config::Config;
use anyhow::Result;
use serde_json::{json, Value};
use std::io::Write;
use std::path::PathBuf;

fn tools(default_dir: &str) -> Value {
    json!([
        {
            "name": "harness",
            "description": "Delegate a coding task to TheHarness: it explores the repository, edits files, runs commands and tests, and returns a report of what it did. Use for work that needs several steps in a real working directory.",
            "inputSchema": {"type": "object", "properties": {
                "task": {"type": "string", "description": "full instructions, as you would give a colleague"},
                "workdir": {"type": "string", "description": format!("working directory (default {default_dir})")},
                "max_turns": {"type": "integer", "description": "default 40"}
            }, "required": ["task"]}
        },
        {
            "name": "harness_ask",
            "description": "Ask TheHarness about a repository without changing anything (read-only): where something is implemented, how a subsystem works, what a failure means.",
            "inputSchema": {"type": "object", "properties": {
                "question": {"type": "string"},
                "workdir": {"type": "string", "description": format!("default {default_dir}")}
            }, "required": ["question"]}
        }
    ])
}

fn out(v: Value) {
    let mut o = std::io::stdout();
    let _ = writeln!(o, "{v}");
    let _ = o.flush();
}

/// Serve MCP over stdio until the client closes it.
pub async fn serve(cfg: Config) -> Result<()> {
    use tokio::io::AsyncBufReadExt;
    let default_dir = std::env::current_dir().map(|p| p.display().to_string()).unwrap_or_else(|_| ".".into());
    let mut lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();
    eprintln!("harness mcp server on stdio (tools: harness, harness_ask) — cwd {default_dir}");
    while let Some(line) = lines.next_line().await? {
        let Ok(v) = serde_json::from_str::<Value>(line.trim()) else { continue };
        let id = v.get("id").cloned();
        let method = v["method"].as_str().unwrap_or("");
        let params = v.get("params").cloned().unwrap_or(Value::Null);
        match method {
            "initialize" => out(json!({"jsonrpc":"2.0","id":id,"result":{
                "protocolVersion": params["protocolVersion"].as_str().unwrap_or("2025-06-18"),
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "harness", "version": crate::VERSION}}})),
            "notifications/initialized" | "notifications/cancelled" => {}
            "tools/list" => out(json!({"jsonrpc":"2.0","id":id,"result":{"tools": tools(&default_dir)}})),
            "ping" => out(json!({"jsonrpc":"2.0","id":id,"result":{}})),
            "tools/call" => {
                let name = params["name"].as_str().unwrap_or("").to_string();
                let args = params.get("arguments").cloned().unwrap_or(Value::Null);
                let cfg = cfg.clone();
                let dir = default_dir.clone();
                // one task at a time: the caller is waiting on this response anyway
                let text = match run_tool(&cfg, &name, &args, &dir).await {
                    Ok(t) => t,
                    Err(e) => { out(json!({"jsonrpc":"2.0","id":id,"result":{"content":[{"type":"text","text":format!("error: {e:#}")}],"isError":true}})); continue; }
                };
                out(json!({"jsonrpc":"2.0","id":id,"result":{"content":[{"type":"text","text":text}]}}));
            }
            other => { if id.is_some() { out(json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":format!("{other} is not supported")}})); } }
        }
    }
    Ok(())
}

async fn run_tool(cfg: &Config, name: &str, args: &Value, default_dir: &str) -> Result<String> {
    let workdir = PathBuf::from(args["workdir"].as_str().unwrap_or(default_dir)).canonicalize()?;
    let sink: std::sync::Arc<dyn crate::events::Sink> = std::sync::Arc::new(crate::events::StderrSink { verbose: false });
    let approver: std::sync::Arc<dyn crate::permissions::Approver> = std::sync::Arc::new(crate::permissions::AutoApprover { yes: true });
    let mut setup = crate::runner::RunSetup::new(cfg.clone(), workdir.clone(), sink, approver);
    setup.session_id = Some(format!("mcp-{}", crate::scheduler::now()));
    let prompt = match name {
        "harness" => {
            if let Some(n) = args["max_turns"].as_u64() { setup.cfg.agent.max_turns = n as usize; }
            setup.prompt_extra = Some("You were called through MCP by another agent. Do the work, then answer with a report it can act on: what changed (paths), what you ran, what the result was, and anything it must decide.".into());
            args["task"].as_str().unwrap_or("").to_string()
        }
        "harness_ask" => {
            setup.perm_mode = Some(crate::permissions::Mode::Plan);
            setup.prompt_extra = Some("You were called through MCP to answer a question about this repository. Read what you need, change nothing, and answer with specifics (paths, line numbers, exact commands).".into());
            args["question"].as_str().unwrap_or("").to_string()
        }
        other => anyhow::bail!("unknown tool '{other}'"),
    };
    if prompt.trim().is_empty() { anyhow::bail!("empty task/question"); }
    crate::runner::start_run(setup, prompt).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advertises_both_tools() {
        let t = tools("/proj");
        let names: Vec<&str> = t.as_array().unwrap().iter().map(|x| x["name"].as_str().unwrap()).collect();
        assert_eq!(names, vec!["harness", "harness_ask"]);
        assert!(t[0]["inputSchema"]["required"].as_array().unwrap().contains(&json!("task")));
        assert!(t[1]["inputSchema"]["properties"]["workdir"]["description"].as_str().unwrap().contains("/proj"));
    }
}
