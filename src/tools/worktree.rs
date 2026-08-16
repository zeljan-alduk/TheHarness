//! worktree: isolated git worktrees for a task (Claude Code EnterWorktree/ExitWorktree parity).
//! `enter` switches the session's working directory for all tools until `exit`; the original repo stays
//! readable/writable (added as an extra root).

use super::{Tool, ToolCtx, ToolOutput};
use anyhow::{bail, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct Worktree;

#[async_trait]
impl Tool for Worktree {
    fn name(&self) -> &'static str { "worktree" }
    fn description(&self) -> &'static str { "Isolated git worktrees under <repo>/.harness/worktrees/<name> (each on its own branch, default wt/<name>). Actions: create {name, branch?, base?} makes one and returns its path; enter {name} creates it if needed and switches the working directory of ALL tools (bash, file tools, sub-agents) to it until exit; exit returns to the original directory (the worktree and branch stay; add remove=true to delete it, which refuses if there are uncommitted changes unless force=true); list; remove {name, delete_branch?, force?}. Commit inside the worktree, then merge/cherry-pick from the main tree." }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{
            "action":{"type":"string","enum":["create","enter","exit","list","remove"]},
            "name":{"type":"string","description":"worktree name (letters, digits, - _ .)"},
            "branch":{"type":"string","description":"branch to check out (default wt/<name>; created from base if missing)"},
            "base":{"type":"string","description":"start point for a new branch (default HEAD)"},
            "delete_branch":{"type":"boolean","description":"remove: also delete the branch"},
            "remove":{"type":"boolean","description":"exit: also remove the worktree"},
            "force":{"type":"boolean","description":"remove/exit: discard uncommitted changes"}
        },"required":["action"]})
    }
    async fn call(&self, args: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("list");
        let name = args.get("name").and_then(|v| v.as_str()).map(|s| s.trim().to_string());
        let flag = |k: &str| args.get(k).and_then(|v| v.as_bool()).unwrap_or(false);
        let cwd = ctx.workdir.clone();
        match action {
            "create" | "enter" => {
                let Some(name) = name.filter(|n| !n.is_empty()) else { bail!("name required") };
                let branch = args.get("branch").and_then(|v| v.as_str()); let base = args.get("base").and_then(|v| v.as_str());
                let root = crate::worktree::main_root(&cwd)?;
                let existing = crate::worktree::path_of(&root, &name);
                let (path, created) = if action == "enter" && existing.is_dir() { (existing, false) } else { (crate::worktree::create(&cwd, &name, branch, base)?, true) };
                let st = crate::worktree::status(&path);
                if action == "create" { return Ok(format!("created worktree '{name}' at {} ({st}). Use worktree enter {{name}} to work inside it, or bash with cd.", path.display()).into()); }
                let Some(cell) = &ctx.cwd else { bail!("enter is not available in this context (fixed working directory); use bash with cd {} instead", path.display()) };
                let mut cur = cell.lock().unwrap();
                if cur.is_none() { *cur = Some(crate::worktree::Cwd { original: root.clone(), current: path.clone(), name: name.clone() }); }
                else if let Some(c) = cur.as_mut() { c.current = path.clone(); c.name = name.clone(); }
                Ok(format!("{} worktree '{name}' at {} ({st}). All tools now run there; the main tree {} remains accessible. Call worktree exit when done.", if created { "created and entered" } else { "entered" }, path.display(), root.display()).into())
            }
            "exit" => {
                let Some(cell) = &ctx.cwd else { bail!("not inside a worktree") };
                let prev = cell.lock().unwrap().take();
                let Some(prev) = prev else { bail!("not inside a worktree") };
                let mut msg = format!("left worktree '{}' — back in {}. Worktree state: {}.", prev.name, prev.original.display(), crate::worktree::status(&prev.current));
                if flag("remove") { match crate::worktree::remove(&prev.original, &prev.name, flag("delete_branch"), flag("force")) { Ok(m) => msg.push_str(&format!(" {m}.")), Err(e) => msg.push_str(&format!(" (not removed: {e})")) } }
                Ok(msg.into())
            }
            "list" => {
                let l = crate::worktree::list(&cwd)?;
                if l.is_empty() { return Ok("no harness worktrees".into()); }
                let inside = ctx.cwd.as_ref().and_then(|c| c.lock().unwrap().as_ref().map(|c| c.name.clone()));
                Ok(l.into_iter().map(|(n, p, b)| format!("{}{n}  branch {b}  {}  ({})", if inside.as_deref() == Some(&n) { "* " } else { "  " }, p.display(), crate::worktree::status(&p))).collect::<Vec<_>>().join("\n").into())
            }
            "remove" => {
                let Some(name) = name.filter(|n| !n.is_empty()) else { bail!("name required") };
                if let Some(c) = &ctx.cwd { if c.lock().unwrap().as_ref().map(|c| c.name == name).unwrap_or(false) { bail!("you are inside worktree '{name}'; call worktree exit first (exit {{remove:true}} does both)"); } }
                Ok(crate::worktree::remove(&cwd, &name, flag("delete_branch"), flag("force"))?.into())
            }
            _ => bail!("unknown action {action}"),
        }
    }
}
