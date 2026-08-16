use super::{arg_str, Tool, ToolCtx};
use crate::sandbox;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct Bash;

#[async_trait]
impl Tool for Bash {
    fn name(&self) -> &'static str { "bash" }
    fn description(&self) -> &'static str {
        "Run a shell command (/bin/sh -c) in the working directory. Use it for git (log, diff, branch, checkout, revert, commit), builds, tests, grep/find, curl, package managers. Non-interactive only; stdin is closed. Long-running commands are killed at the timeout."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "cmd": {"type": "string", "description": "The command line to run"},
                "timeout_secs": {"type": "integer", "description": "Optional override of the default timeout"}
            },
            "required": ["cmd"]
        })
    }
    async fn call(&self, args: Value, ctx: &ToolCtx) -> Result<String> {
        let cmd = arg_str(&args, "cmd")?;
        let timeout = args.get("timeout_secs").and_then(|v| v.as_u64())
            .map(std::time::Duration::from_secs)
            .map(|t| t.min(ctx.timeout * 5))
            .unwrap_or(ctx.timeout);
        let out = sandbox::run_shell(cmd, &ctx.workdir, timeout, ctx.max_output).await?;
        let mut s = String::new();
        if !out.stdout.is_empty() { s.push_str(&out.stdout); if !s.ends_with('\n') { s.push('\n'); } }
        if !out.stderr.is_empty() { s.push_str("[stderr]\n"); s.push_str(&out.stderr); if !s.ends_with('\n') { s.push('\n'); } }
        s.push_str(&format!("[exit {} in {:.1}s]", out.code.map(|c| c.to_string()).unwrap_or_else(|| "signal/timeout".into()), out.elapsed.as_secs_f64()));
        Ok(s)
    }
}
