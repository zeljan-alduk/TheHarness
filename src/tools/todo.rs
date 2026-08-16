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
    fn description(&self) -> &'static str { "Maintain your task list for multi-step work (the user watches it live). Actions: set {items:[...]} replaces the list (items may be strings or {text,status}); add {text}; start {id|text} marks an item in_progress (and the previous in_progress one done); done {id|text}; next = mark current in_progress done and start the next pending; update {id, status}; list; clear. Keep exactly ONE item in_progress while you work on it and mark items done the moment they are finished." }
    fn parameters(&self) -> Value { json!({"type":"object","properties":{"action":{"type":"string","enum":["set","add","start","done","next","update","list","clear"]},"items":{"type":"array","items":{}},"text":{"type":"string"},"id":{"type":"integer"},"status":{"type":"string","enum":["pending","in_progress","done"]}},"required":["action"]}) }
    async fn call(&self, args: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("list");
        let mut todos = ctx.todos.lock().unwrap();
        let find = |todos: &Vec<TodoItem>| -> Option<u32> {
            if let Some(id) = args.get("id").and_then(|v| v.as_u64()) { return Some(id as u32); }
            let t = args.get("text").and_then(|v| v.as_str()).unwrap_or("").trim().to_lowercase();
            if t.is_empty() { return None; }
            todos.iter().find(|x| x.text.to_lowercase() == t).or_else(|| todos.iter().find(|x| x.text.to_lowercase().contains(&t))).map(|x| x.id)
        };
        match action {
            "set" => { let items = args.get("items").and_then(|v| v.as_array()).cloned().unwrap_or_default(); *todos = items.iter().enumerate().filter_map(|(i, v)| match v { Value::String(t) => Some(TodoItem { id: i as u32 + 1, text: t.clone(), status: "pending".into() }), Value::Object(o) => o.get("text").and_then(|x| x.as_str()).map(|t| TodoItem { id: i as u32 + 1, text: t.to_string(), status: o.get("status").and_then(|x| x.as_str()).unwrap_or("pending").to_string() }), _ => None }).collect(); }
            "add" => { let t = args.get("text").and_then(|v| v.as_str()).unwrap_or("").trim().to_string(); if t.is_empty() { bail!("text required"); } let id = todos.iter().map(|t| t.id).max().unwrap_or(0) + 1; todos.push(TodoItem { id, text: t, status: "pending".into() }); }
            "start" => { let Some(id) = find(&todos) else { bail!("start needs id or text of an existing item") }; for t in todos.iter_mut() { if t.status == "in_progress" { t.status = "done".into(); } } match todos.iter_mut().find(|t| t.id == id) { Some(t) => t.status = "in_progress".into(), None => bail!("no todo #{id}") } }
            "done" => { let Some(id) = find(&todos) else { bail!("done needs id or text of an existing item") }; match todos.iter_mut().find(|t| t.id == id) { Some(t) => t.status = "done".into(), None => bail!("no todo #{id}") } }
            "next" => { for t in todos.iter_mut() { if t.status == "in_progress" { t.status = "done".into(); } } if let Some(t) = todos.iter_mut().find(|t| t.status == "pending") { t.status = "in_progress".into(); } }
            "update" => { let Some(id) = find(&todos) else { bail!("update needs id or text") }; let st = args.get("status").and_then(|v| v.as_str()).unwrap_or("done"); match todos.iter_mut().find(|t| t.id == id) { Some(t) => t.status = st.to_string(), None => bail!("no todo #{id}") } }
            "clear" => todos.clear(),
            _ => {}
        }
        if todos.is_empty() { return Ok("todo list is empty".into()); }
        Ok(todos.iter().map(|t| format!("{} #{} {}", match t.status.as_str() { "done" => "☑", "in_progress" => "▶", _ => "☐" }, t.id, t.text)).collect::<Vec<_>>().join("\n").into())
    }
}
