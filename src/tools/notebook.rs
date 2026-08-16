//! notebook_edit: cell-level edits for Jupyter notebooks (.ipynb) without corrupting the JSON.

use super::{arg_str, Tool, ToolCtx, ToolOutput};
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct NotebookEdit;

#[async_trait]
impl Tool for NotebookEdit {
    fn name(&self) -> &'static str { "notebook_edit" }
    fn description(&self) -> &'static str { "Read or edit a Jupyter notebook (.ipynb) at cell level: list (index, type, first line), replace {index, source}, insert {index, source, cell_type}, delete {index}. Preserves the rest of the notebook exactly." }
    fn parameters(&self) -> Value { json!({"type":"object","properties":{"path":{"type":"string"},"action":{"type":"string","enum":["list","get","replace","insert","delete"]},"index":{"type":"integer"},"source":{"type":"string"},"cell_type":{"type":"string","enum":["code","markdown"]}},"required":["path","action"]}) }
    async fn call(&self, args: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let path = ctx.resolve(arg_str(&args, "path")?)?;
        let action = arg_str(&args, "action")?;
        let text = tokio::fs::read_to_string(&path).await.with_context(|| format!("reading {}", path.display()))?;
        let mut nb: Value = serde_json::from_str(&text).context("not valid notebook JSON")?;
        let cells = nb.get_mut("cells").and_then(|c| c.as_array_mut()).context("notebook has no cells array")?;
        let idx = args.get("index").and_then(|v| v.as_u64()).map(|v| v as usize);
        let src_of = |c: &Value| -> String { match &c["source"] { Value::Array(a) => a.iter().filter_map(|s| s.as_str()).collect::<String>(), Value::String(s) => s.clone(), _ => String::new() } };
        let to_lines = |s: &str| -> Value { let mut v: Vec<Value> = Vec::new(); let mut rest = s; while let Some(i) = rest.find('\n') { v.push(json!(&rest[..=i])); rest = &rest[i + 1..]; } if !rest.is_empty() { v.push(json!(rest)); } Value::Array(v) };
        match action {
            "list" => { let mut out = Vec::new(); for (i, c) in cells.iter().enumerate() { let s = src_of(c); out.push(format!("[{i}] {:<8} {}", c["cell_type"].as_str().unwrap_or("?"), crate::llm::truncate_for_log(s.lines().next().unwrap_or(""), 90))); } Ok(out.join("\n").into()) }
            "get" => { let i = idx.context("index required")?; let c = cells.get(i).context("index out of range")?; Ok(src_of(c).into()) }
            "replace" | "insert" | "delete" => {
                let i = idx.context("index required")?;
                match action {
                    "replace" => { let c = cells.get_mut(i).context("index out of range")?; c["source"] = to_lines(arg_str(&args, "source")?); if c["cell_type"] == "code" { c["outputs"] = json!([]); c["execution_count"] = Value::Null; } }
                    "insert" => { if i > cells.len() { bail!("index out of range") } let ct = args.get("cell_type").and_then(|v| v.as_str()).unwrap_or("code"); let mut c = json!({"cell_type": ct, "metadata": {}, "source": to_lines(arg_str(&args, "source")?)}); if ct == "code" { c["outputs"] = json!([]); c["execution_count"] = Value::Null; } cells.insert(i, c); }
                    _ => { if i >= cells.len() { bail!("index out of range") } cells.remove(i); }
                }
                let n = cells.len();
                tokio::fs::write(&path, serde_json::to_string_pretty(&nb)? + "\n").await?;
                Ok(format!("{action} cell {i}; notebook now has {n} cells").into())
            }
            _ => bail!("unknown action"),
        }
    }
}
