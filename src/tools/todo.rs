//! todo: a small shared task list the model maintains (shown in the TUI dashboard).

use super::{Tool, ToolCtx, ToolOutput};
use anyhow::{bail, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoItem { pub id: u32, pub text: String, pub status: String } // pending | in_progress | done

pub struct Todo;

#[async_trait]
impl Tool for Todo {
    fn name(&self) -> &'static str { "todo" }
    fn description(&self) -> &'static str { "Maintain your task list for multi-step work (visible to the user). Actions: set {items:[\"...\"]} replaces the list; add {text}; update {id, status: pending|in_progress|done}; list. Mark exactly one item in_progress while working on it and mark items done as you finish." }
    fn parameters(&self) -> Value { json!({"type":"object","properties":{"action":{"type":"string","enum":["set","add","update","list","clear"]},"items":{"type":"array","items":{"type":"string"}},"text":{"type":"string"},"id":{"type":"integer"},"status":{"type":"string","enum":["pending","in_progress","done"]}},"required":["action"]}) }
    async fn call(&self, args: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("list");
        let mut todos = ctx.todos.lock().unwrap();
        match action {
            "set" => { let items = args.get("items").and_then(|v| v.as_array()).cloned().unwrap_or_default(); *todos = items.iter().enumerate().filter_map(|(i, v)| v.as_str().map(|t| TodoItem { id: i as u32 + 1, text: t.to_string(), status: "pending".into() })).collect(); }
            "add" => { let t = args.get("text").and_then(|v| v.as_str()).unwrap_or("").trim().to_string(); if t.is_empty() { bail!("text required"); } let id = todos.iter().map(|t| t.id).max().unwrap_or(0) + 1; todos.push(TodoItem { id, text: t, status: "pending".into() }); }
            "update" => { let id = args.get("id").and_then(|v| v.as_u64()).map(|v| v as u32); let st = args.get("status").and_then(|v| v.as_str()).unwrap_or("done"); let Some(id) = id else { bail!("id required") }; match todos.iter_mut().find(|t| t.id == id) { Some(t) => t.status = st.to_string(), None => bail!("no todo #{id}") } }
            "clear" => todos.clear(),
            _ => {}
        }
        if todos.is_empty() { return Ok("todo list is empty".into()); }
        Ok(todos.iter().map(|t| format!("{} #{} {}", match t.status.as_str() { "done" => "☑", "in_progress" => "▶", _ => "☐" }, t.id, t.text)).collect::<Vec<_>>().join("\n").into())
    }
}
