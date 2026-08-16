//! run_workflow: list / run `harness workflow` TOML scripts from inside a session.

use super::{Tool, ToolCtx, ToolOutput};
use anyhow::{bail, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct RunWorkflow;

/// "name — description — path" per line (what `list` returns).
pub fn format_list(items: &[(String, String, std::path::PathBuf)]) -> String {
    if items.is_empty() { return "(no workflows; add TOML files under ~/.config/harness/workflows or <workdir>/.harness/workflows)".into(); }
    items.iter().map(|(n, d, p)| if d.is_empty() { format!("{n} — {}", p.display()) } else { format!("{n} — {d} — {}", p.display()) }).collect::<Vec<_>>().join("\n")
}

#[async_trait]
impl Tool for RunWorkflow {
    fn name(&self) -> &'static str { "run_workflow" }
    fn description(&self) -> &'static str { "Run a saved `harness workflow` (TOML script of shell/agent steps). Actions: list (available workflows) · run {name, args?} (returns the final step's output)." }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{
            "action":{"type":"string","enum":["list","run"]},
            "name":{"type":"string","description":"run: workflow name (from list) or path to a .toml"},
            "args":{"type":"string","description":"run: text available as {args} inside the workflow"}
        },"required":["action"]})
    }
    async fn call(&self, args: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("list");
        match action {
            "list" => Ok(format_list(&crate::workflow::list(&ctx.workdir)).into()),
            "run" => {
                let Some(env) = &ctx.subagent else { bail!("run_workflow is not available here (nested)") };
                let name = match args.get("name").and_then(|v| v.as_str()) { Some(n) if !n.trim().is_empty() => n.trim().to_string(), _ => bail!("run: 'name' is required") };
                let wargs = args.get("args").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let wf = crate::workflow::find(&name, &ctx.workdir)?;
                // Steps run through the env's registry/policy; the inner ctx forbids nesting (spawn/run_workflow).
                let inner_ctx = ToolCtx { subagent: None, ..ctx.clone() };
                let sink: std::sync::Arc<dyn crate::events::Sink> = std::sync::Arc::new(crate::agent::PrefixSink { inner: env.sink.clone(), prefix: format!("⟳{} ", wf.name), info: None });
                let registry = env.registry.without("spawn_agent");
                let base_system = crate::agent::system_prompt_with_memory(&ctx.workdir.display().to_string(), &registry.names(), None, ctx.memory.as_ref());
                let wenv = crate::workflow::WorkflowEnv { env: env.clone(), ctx: inner_ctx, sink, base_system };
                let out = crate::workflow::run(&wf, &wargs, &wenv).await?;
                Ok(format!("[workflow {} finished]\n{}", wf.name, out).into())
            }
            other => bail!("unknown action '{other}' (list|run)"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_formatting() {
        let items = vec![("a".to_string(), "does a".to_string(), std::path::PathBuf::from("/x/a.toml")), ("b".to_string(), String::new(), std::path::PathBuf::from("/x/b.toml"))];
        assert_eq!(format_list(&items), "a — does a — /x/a.toml\nb — /x/b.toml");
        assert!(format_list(&[]).contains("no workflows"));
    }

    #[test]
    fn examples_are_listed() {
        let items = crate::workflow::list(&std::env::temp_dir());
        assert!(items.iter().any(|(n, _, _)| n == "review"), "example 'review' workflow should be listed");
        assert!(format_list(&items).contains("review — "));
    }

    #[tokio::test]
    async fn run_requires_subagent_env() {
        let ctx = ToolCtx::basic(std::env::temp_dir());
        let e = RunWorkflow.call(json!({"action":"run","name":"review"}), &ctx).await.unwrap_err();
        assert!(e.to_string().contains("nested"), "{e}");
    }
}
