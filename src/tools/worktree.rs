//! worktree: isolated git worktrees for risky or parallel work. enter creates <repo>/.harness-worktrees/<name>
//! on a new branch (from HEAD); exit removes it (optionally keeping the branch). Sub-agents can be
//! pointed at the returned path with spawn_agent {workdir}.

use super::{arg_str, Tool, ToolCtx, ToolOutput};
use anyhow::{bail, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct Worktree;

#[async_trait]
impl Tool for Worktree {
    fn name(&self) -> &'static str { "worktree" }
    fn description(&self) -> &'static str { "Git worktrees for isolated experiments or parallel sub-agents. enter {name?, branch?} creates .harness-worktrees/<name> on a new branch from HEAD and returns its path (use it as workdir for spawn_agent or cd in bash); list; exit {name, keep_branch?} removes the worktree (branch kept by default so work can be merged/reviewed)." }
    fn parameters(&self) -> Value { json!({"type":"object","properties":{"action":{"type":"string","enum":["enter","list","exit"]},"name":{"type":"string"},"branch":{"type":"string","description":"branch to create (default harness/<name>)"},"keep_branch":{"type":"boolean","description":"default true"}},"required":["action"]}) }
    async fn call(&self, args: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let action = arg_str(&args, "action")?;
        let sh = |cmd: String| async move { crate::sandbox::run_shell(&cmd, &ctx.workdir, std::time::Duration::from_secs(120), 8000).await };
        let root = sh("git rev-parse --show-toplevel 2>/dev/null".into()).await?;
        if !root.success() { bail!("not inside a git repository"); }
        let root = root.stdout.trim().to_string();
        match action {
            "list" => { let o = sh("git worktree list".into()).await?; Ok(o.stdout.trim().to_string().into()) }
            "enter" => {
                let name = args.get("name").and_then(|v| v.as_str()).map(|s| s.to_string()).unwrap_or_else(|| format!("wt-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() % 100000).unwrap_or(0)));
                let name = name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect::<String>();
                let branch = args.get("branch").and_then(|v| v.as_str()).map(String::from).unwrap_or(format!("harness/{name}"));
                let path = format!("{root}/.harness-worktrees/{name}");
                let o = sh(format!("mkdir -p '{root}/.harness-worktrees' && (grep -qx '.harness-worktrees/' '{root}/.git/info/exclude' 2>/dev/null || echo '.harness-worktrees/' >> '{root}/.git/info/exclude'); git worktree add -b '{branch}' '{path}' HEAD 2>&1 || git worktree add '{path}' '{branch}' 2>&1")).await?;
                if !o.success() { bail!("worktree add failed: {}{}", o.stdout, o.stderr); }
                Ok(format!("worktree ready: {path} (branch {branch}, from HEAD). Work there via spawn_agent {{workdir: \"{path}\"}} or `cd {path} && …`; when done: worktree exit {{name: \"{name}\"}} then merge the branch if wanted.").into())
            }
            "exit" => {
                let name = arg_str(&args, "name")?;
                let keep = args.get("keep_branch").and_then(|v| v.as_bool()).unwrap_or(true);
                let path = format!("{root}/.harness-worktrees/{name}");
                let br = sh(format!("git -C '{path}' rev-parse --abbrev-ref HEAD 2>/dev/null")).await.map(|o| o.stdout.trim().to_string()).unwrap_or_default();
                let o = sh(format!("git worktree remove --force '{path}' 2>&1 && git worktree prune")).await?;
                if !o.success() { bail!("worktree remove failed: {}{}", o.stdout, o.stderr); }
                let mut msg = format!("removed worktree {path}");
                if !keep && !br.is_empty() { let d = sh(format!("git branch -D '{br}' 2>&1")).await?; msg.push_str(&format!("; branch {br} {}", if d.success() { "deleted" } else { "NOT deleted" })); } else if !br.is_empty() { msg.push_str(&format!("; branch {br} kept (merge or delete it yourself)")); }
                Ok(msg.into())
            }
            _ => bail!("unknown action"),
        }
    }
}
