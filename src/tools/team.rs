//! team: run several named sub-agents on one goal at the same time, sharing the todo list as their
//! task board and each other's mailboxes for coordination. It is a thin primitive on purpose — the
//! members are ordinary sub-agents, so everything that governs one (permissions, checkpoints, custom
//! agent definitions, worktree isolation) governs a team member too.

use super::{arg_str, Tool, ToolCtx, ToolOutput};
use anyhow::{bail, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct Team;

#[async_trait]
impl Tool for Team {
    fn name(&self) -> &'static str { "team" }
    fn description(&self) -> &'static str {
        "Put several agents on one goal at the same time. Each member gets a role and works from the shared todo list (claim an item with todo start {id, owner}, mark it done when it is), and members can message each other with `agents send`. Use when a goal splits into parts that touch different files; use spawn_agent for a single delegated task. Returns every member's report."
    }
    fn parallel_safe(&self) -> bool { false }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{
            "goal":{"type":"string","description":"what the team is trying to achieve, in full"},
            "members":{"type":"array","description":"2–5 members","items":{"type":"object","properties":{
                "name":{"type":"string","description":"short handle, e.g. api, ui, tests"},
                "role":{"type":"string","description":"what this member owns"},
                "subagent_type":{"type":"string","description":"optional custom agent to run as"},
                "model":{"type":"string"},
                "workdir":{"type":"string"},
                "isolation":{"type":"string","enum":["none","worktree"]}
            },"required":["name","role"]}},
            "max_turns":{"type":"integer","description":"per member, default 25"}
        },"required":["goal","members"]})
    }

    async fn call(&self, args: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        if ctx.subagent.is_none() { bail!("team is not available here (a sub-agent cannot form its own team)"); }
        let goal = arg_str(&args, "goal")?.to_string();
        let members = args.get("members").and_then(|v| v.as_array()).cloned().unwrap_or_default();
        if members.len() < 2 { bail!("a team needs at least two members (use spawn_agent for one task)"); }
        if members.len() > 5 { bail!("at most five members"); }
        let max_turns = args.get("max_turns").and_then(|v| v.as_u64()).unwrap_or(25);

        let roster: Vec<String> = members.iter().map(|m| format!("{} ({})", m["name"].as_str().unwrap_or("?"), m["role"].as_str().unwrap_or(""))).collect();
        let board = { ctx.todos.lock().map(|t| t.iter().map(|i| i.line(&t)).collect::<Vec<_>>().join("\n")).unwrap_or_default() };
        let shared = format!(
"You are one member of a team of {} working on the same goal at the same time.\n\nGOAL\n{goal}\n\nTEAM\n{}\n\n\
SHARED TASK BOARD (the `todo` tool — every member sees the same list)\n{}\n\n\
How the team works:\n\
- Claim before you work: `todo start {{id, owner: \"<your name>\"}}`; never touch an item another member owns.\n\
- Mark items done the moment they are, and add items you discover with `todo add`.\n\
- If your work depends on another member's, say so with `agents send` instead of waiting silently or duplicating it.\n\
- Stay inside your role: overlapping edits to the same file are the one thing that ruins a parallel team.\n\
- Finish with a report of what you changed (paths), what you verified, and what you left for others.",
            members.len(), roster.join("\n"), if board.trim().is_empty() { "(empty — create the items you own with todo add)" } else { &board });

        // every member is an ordinary sub-agent; spawn_agent does the real work
        let spawner = super::subagent::SpawnAgent;
        let mut jobs = Vec::new();
        for m in &members {
            let name = m["name"].as_str().unwrap_or("member").to_string();
            let role = m["role"].as_str().unwrap_or("").to_string();
            let mut a = json!({
                "task": format!("{shared}\n\nYOU ARE: {name}\nYOUR ROLE: {role}\n\nStart now."),
                "max_turns": max_turns,
            });
            for k in ["subagent_type", "model", "workdir", "isolation"] { if let Some(v) = m.get(k) { a[k] = v.clone(); } }
            jobs.push(async { (name, spawner.call(a, ctx).await) });
        }
        let results = futures_util::future::join_all(jobs).await;

        let mut out = format!("team of {} finished on: {}\n", members.len(), crate::llm::truncate_for_log(&goal, 120));
        for (name, r) in results {
            match r {
                Ok(o) => out.push_str(&format!("\n── {name} ──\n{}\n", o.text)),
                Err(e) => out.push_str(&format!("\n── {name} ── failed: {e:#}\n")),
            }
        }
        let left = { ctx.todos.lock().map(|t| t.iter().filter(|i| i.status != "done").count()).unwrap_or(0) };
        out.push_str(&format!("\n{left} task board item(s) still open — check them before reporting the goal as met.\n"));
        Ok(out.into())
    }
}
