//! `memory` tool: lets the model read and edit its persistent memory files.

use super::{arg_str, Tool, ToolCtx, ToolOutput};
use anyhow::{bail, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct MemoryTool;

#[async_trait]
impl Tool for MemoryTool {
    fn name(&self) -> &'static str { "memory" }
    fn description(&self) -> &'static str {
        "Read or edit your persistent memory files: MEMORY (settings/preferences/ideas), WORKFLOWS (reusable recipes), BRAIN (what you learned about the user, projects, how-tos, lessons). Actions: show {file}; append {file, section, text} adds a bullet under a '## section'; remove {file, text} deletes bullets containing text; rewrite {file, content} replaces the whole file (consolidation). Record only durable, non-obvious facts; never secrets."
    }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{
            "action":{"type":"string","enum":["show","append","remove","rewrite"]},
            "file":{"type":"string","enum":["MEMORY","WORKFLOWS","BRAIN"]},
            "section":{"type":"string","description":"for append: heading such as User, Projects, How-to, Lessons, Settings, Preferences, Ideas, or a workflow name"},
            "text":{"type":"string","description":"for append/remove"},
            "content":{"type":"string","description":"for rewrite: full new markdown"}
        },"required":["action","file"]})
    }
    async fn call(&self, args: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let Some(store) = &ctx.memory else { bail!("memory is disabled in this context") };
        let action = arg_str(&args, "action")?;
        let file = arg_str(&args, "file")?;
        match action {
            "show" => Ok(store.read(file)?.into()),
            "append" => {
                let section = args.get("section").and_then(|v| v.as_str()).unwrap_or("Notes");
                let text = arg_str(&args, "text")?;
                if text.chars().count() > 400 { bail!("keep memory entries short (≤400 chars); put long material in WORKFLOWS as steps or in the project itself"); }
                Ok(if store.append(file, section, text)? { format!("added to {} › {section}: {text}", crate::memory::canonical_name(file)?) } else { "already present".into() }.into())
            }
            "remove" => { let n = store.remove(file, arg_str(&args, "text")?)?; Ok(format!("removed {n} bullet(s)").into()) }
            "rewrite" => {
                let content = arg_str(&args, "content")?;
                if !content.trim_start().starts_with('#') { bail!("rewrite content must be markdown starting with a '# ' title"); }
                store.write(file, content)?; Ok(format!("rewrote {}", crate::memory::canonical_name(file)?).into())
            }
            _ => bail!("unknown action {action}"),
        }
    }
}
