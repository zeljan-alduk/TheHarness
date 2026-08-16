//! todo: a small shared task list the model maintains (shown in the TUI dashboard).
//! Items form a light task graph: `blocked_by` (ids), `owner`, `details`.

use super::{Tool, ToolCtx, ToolOutput};
use anyhow::{bail, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoItem {
    pub id: u32,
    pub text: String,
    pub status: String, // pending | in_progress | done
    #[serde(default)] pub blocked_by: Vec<u32>,
    #[serde(default)] pub owner: String,
    #[serde(default)] pub details: String,
}

impl TodoItem {
    fn new(id: u32, text: String, status: String) -> Self { Self { id, text, status, blocked_by: vec![], owner: String::new(), details: String::new() } }
    /// Ids in `blocked_by` that are not yet done (missing ids count as unblocked).
    pub fn open_blockers(&self, all: &[TodoItem]) -> Vec<u32> {
        self.blocked_by.iter().copied().filter(|b| all.iter().any(|t| t.id == *b && t.status != "done")).collect()
    }
    /// One-line rendering used by the tool output and the `/todos` command.
    pub fn line(&self, all: &[TodoItem]) -> String {
        let mut s = format!("{} #{} {}", match self.status.as_str() { "done" => "☑", "in_progress" => "▶", _ => "☐" }, self.id, self.text);
        let blk = self.open_blockers(all);
        if !blk.is_empty() { s.push_str(&format!("  ⏳blocked by {}", blk.iter().map(|b| format!("#{b}")).collect::<Vec<_>>().join(","))); }
        if !self.owner.is_empty() { s.push_str(&format!("  @{}", self.owner)); }
        s
    }
}

fn parse_ids(v: Option<&Value>) -> Result<Option<Vec<u32>>> {
    let Some(v) = v else { return Ok(None) };
    let out = match v {
        Value::Null => return Ok(None),
        Value::Array(a) => a.iter().map(|x| x.as_u64().map(|n| n as u32).ok_or_else(|| anyhow::anyhow!("blocked_by must be a list of integer ids"))).collect::<Result<Vec<_>>>()?,
        Value::Number(n) => vec![n.as_u64().ok_or_else(|| anyhow::anyhow!("blocked_by must be a list of integer ids"))? as u32],
        Value::String(s) => s.split(|c: char| c == ',' || c.is_whitespace()).filter(|p| !p.is_empty()).map(|p| p.trim_start_matches('#').parse::<u32>().map_err(|_| anyhow::anyhow!("blocked_by must be a list of integer ids"))).collect::<Result<Vec<_>>>()?,
        _ => bail!("blocked_by must be a list of integer ids"),
    };
    Ok(Some(out))
}

fn check_blockers(id: u32, blockers: &[u32], all: &[TodoItem]) -> Result<()> {
    for b in blockers {
        if *b == id { bail!("todo #{id} cannot be blocked by itself"); }
        if !all.iter().any(|t| t.id == *b) { bail!("blocked_by references unknown todo #{b}"); }
    }
    Ok(())
}

fn opt_str(o: &Value, key: &str) -> Option<String> { o.get(key).and_then(|v| v.as_str()).map(|s| s.trim().to_string()) }

pub struct Todo;

#[async_trait]
impl Tool for Todo {
    fn name(&self) -> &'static str { "todo" }
    fn description(&self) -> &'static str { "Maintain your task list for multi-step work (the user watches it live). Actions: set {items:[...]} replaces the list (items may be strings or {text,status,blocked_by,owner,details}); add {text, blocked_by?, owner?, details?}; start {id|text} marks an item in_progress (and the previous in_progress one done; refuses if its blockers are not done unless force:true); done {id|text}; next = mark current in_progress done and start the first unblocked pending item; update {id|text, status?, blocked_by?, owner?, details?}; get {id|text} shows full details; list; clear. Dependencies: blocked_by = ids of items that must be done first. Keep exactly ONE item in_progress while you work on it and mark items done the moment they are finished." }
    fn parameters(&self) -> Value { json!({"type":"object","properties":{"action":{"type":"string","enum":["set","add","start","done","next","update","get","list","clear"]},"items":{"type":"array","items":{}},"text":{"type":"string"},"id":{"type":"integer"},"status":{"type":"string","enum":["pending","in_progress","done"]},"blocked_by":{"type":"array","items":{"type":"integer"},"description":"ids of items that must be done before this one"},"owner":{"type":"string"},"details":{"type":"string"},"force":{"type":"boolean","description":"start even if blockers are open"}},"required":["action"]}) }
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
            "set" => {
                let items = args.get("items").and_then(|v| v.as_array()).cloned().unwrap_or_default();
                let mut new: Vec<TodoItem> = Vec::new();
                for v in items.iter() {
                    let id = new.len() as u32 + 1;
                    match v {
                        Value::String(t) => new.push(TodoItem::new(id, t.clone(), "pending".into())),
                        Value::Object(o) => {
                            let Some(t) = o.get("text").and_then(|x| x.as_str()) else { continue };
                            let mut it = TodoItem::new(id, t.to_string(), o.get("status").and_then(|x| x.as_str()).unwrap_or("pending").to_string());
                            it.blocked_by = parse_ids(o.get("blocked_by"))?.unwrap_or_default();
                            it.owner = opt_str(v, "owner").unwrap_or_default();
                            it.details = opt_str(v, "details").unwrap_or_default();
                            new.push(it);
                        }
                        _ => {}
                    }
                }
                for it in &new { check_blockers(it.id, &it.blocked_by, &new)?; }
                *todos = new;
            }
            "add" => {
                let t = args.get("text").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
                if t.is_empty() { bail!("text required"); }
                let id = todos.iter().map(|t| t.id).max().unwrap_or(0) + 1;
                let mut it = TodoItem::new(id, t, "pending".into());
                it.blocked_by = parse_ids(args.get("blocked_by"))?.unwrap_or_default();
                check_blockers(id, &it.blocked_by, &todos)?;
                it.owner = opt_str(&args, "owner").unwrap_or_default();
                it.details = opt_str(&args, "details").unwrap_or_default();
                todos.push(it);
            }
            "start" => {
                let Some(id) = find(&todos) else { bail!("start needs id or text of an existing item") };
                let Some(item) = todos.iter().find(|t| t.id == id) else { bail!("no todo #{id}") };
                let force = args.get("force").and_then(|v| v.as_bool()).unwrap_or(false);
                let blk = item.open_blockers(&todos);
                if !blk.is_empty() && !force { bail!("todo #{id} is blocked by {} (not done yet) — finish those first, or pass force:true", blk.iter().map(|b| format!("#{b}")).collect::<Vec<_>>().join(",")); }
                for t in todos.iter_mut() { if t.status == "in_progress" { t.status = "done".into(); } }
                if let Some(t) = todos.iter_mut().find(|t| t.id == id) { t.status = "in_progress".into(); }
            }
            "done" => { let Some(id) = find(&todos) else { bail!("done needs id or text of an existing item") }; match todos.iter_mut().find(|t| t.id == id) { Some(t) => t.status = "done".into(), None => bail!("no todo #{id}") } }
            "next" => {
                for t in todos.iter_mut() { if t.status == "in_progress" { t.status = "done".into(); } }
                let snapshot = todos.clone();
                if let Some(t) = todos.iter_mut().find(|t| t.status == "pending" && t.open_blockers(&snapshot).is_empty()) { t.status = "in_progress".into(); }
            }
            "update" => {
                let Some(id) = find(&todos) else { bail!("update needs id or text") };
                if !todos.iter().any(|t| t.id == id) { bail!("no todo #{id}"); }
                let blocked = parse_ids(args.get("blocked_by"))?;
                if let Some(b) = &blocked { check_blockers(id, b, &todos)?; }
                let has_fields = blocked.is_some() || args.get("owner").is_some() || args.get("details").is_some();
                let st = args.get("status").and_then(|v| v.as_str()).map(|s| s.to_string()).or_else(|| if has_fields { None } else { Some("done".into()) });
                let t = todos.iter_mut().find(|t| t.id == id).unwrap();
                if let Some(s) = st { t.status = s; }
                if let Some(b) = blocked { t.blocked_by = b; }
                if let Some(o) = opt_str(&args, "owner") { t.owner = o; }
                if let Some(d) = opt_str(&args, "details") { t.details = d; }
            }
            "get" => {
                let Some(id) = find(&todos) else { bail!("get needs id or text") };
                let Some(t) = todos.iter().find(|t| t.id == id) else { bail!("no todo #{id}") };
                let mut out = vec![t.line(&todos), format!("status: {}", t.status)];
                if !t.blocked_by.is_empty() { out.push(format!("blocked_by: {}", t.blocked_by.iter().map(|b| format!("#{b}")).collect::<Vec<_>>().join(","))); }
                if !t.owner.is_empty() { out.push(format!("owner: {}", t.owner)); }
                if !t.details.is_empty() { out.push(format!("details: {}", t.details)); }
                let dependents: Vec<String> = todos.iter().filter(|x| x.blocked_by.contains(&t.id)).map(|x| format!("#{}", x.id)).collect();
                if !dependents.is_empty() { out.push(format!("blocks: {}", dependents.join(","))); }
                return Ok(out.join("\n").into());
            }
            "clear" => todos.clear(),
            _ => {}
        }
        if todos.is_empty() { return Ok("todo list is empty".into()); }
        Ok(todos.iter().map(|t| t.line(&todos)).collect::<Vec<_>>().join("\n").into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> ToolCtx { ToolCtx::basic(std::env::temp_dir()) }
    async fn call(ctx: &ToolCtx, v: Value) -> Result<String> { Todo.call(v, ctx).await.map(|o| o.text) }

    #[tokio::test]
    async fn add_with_blocked_by_and_list_marks() {
        let c = ctx();
        call(&c, json!({"action":"set","items":["a","b"]})).await.unwrap();
        let out = call(&c, json!({"action":"add","text":"c","blocked_by":[1,2],"owner":"bob","details":"needs a and b"})).await.unwrap();
        assert!(out.contains("☐ #3 c  ⏳blocked by #1,#2  @bob"), "{out}");
        assert!(out.contains("☐ #1 a\n"), "simple lines unchanged: {out}");
    }

    #[tokio::test]
    async fn start_blocked_errors_then_ok_after_blocker_done() {
        let c = ctx();
        call(&c, json!({"action":"set","items":["a",{"text":"b","blocked_by":[1]}]})).await.unwrap();
        let err = call(&c, json!({"action":"start","id":2})).await.unwrap_err().to_string();
        assert!(err.contains("blocked by #1"), "{err}");
        assert!(call(&c, json!({"action":"start","id":2,"force":true})).await.is_ok());
        call(&c, json!({"action":"update","id":2,"status":"pending"})).await.unwrap();
        call(&c, json!({"action":"done","id":1})).await.unwrap();
        let out = call(&c, json!({"action":"start","id":2})).await.unwrap();
        assert!(out.contains("▶ #2 b") && !out.contains("⏳"), "{out}");
    }

    #[tokio::test]
    async fn next_skips_blocked() {
        let c = ctx();
        call(&c, json!({"action":"set","items":["a",{"text":"b","blocked_by":[1]},"c"]})).await.unwrap();
        call(&c, json!({"action":"start","id":1})).await.unwrap();
        // finishing #1 unblocks #2 → next picks #2
        let out = call(&c, json!({"action":"next"})).await.unwrap();
        assert!(out.contains("☑ #1 a") && out.contains("▶ #2 b"), "{out}");
        // now block #3 on a still-open item and check next skips it
        call(&c, json!({"action":"add","text":"d"})).await.unwrap();
        call(&c, json!({"action":"update","id":3,"blocked_by":[4]})).await.unwrap();
        let out = call(&c, json!({"action":"next"})).await.unwrap();
        assert!(out.contains("☐ #3 c  ⏳blocked by #4") && out.contains("▶ #4 d"), "{out}");
    }

    #[tokio::test]
    async fn get_renders_details() {
        let c = ctx();
        call(&c, json!({"action":"set","items":["a",{"text":"b","blocked_by":[1],"owner":"ann","details":"the plan"}]})).await.unwrap();
        let out = call(&c, json!({"action":"get","id":2})).await.unwrap();
        assert!(out.contains("status: pending") && out.contains("blocked_by: #1") && out.contains("owner: ann") && out.contains("details: the plan"), "{out}");
        let out = call(&c, json!({"action":"get","text":"a"})).await.unwrap();
        assert!(out.contains("blocks: #2"), "{out}");
    }

    #[tokio::test]
    async fn invalid_blocked_by_errors() {
        let c = ctx();
        call(&c, json!({"action":"set","items":["a"]})).await.unwrap();
        let err = call(&c, json!({"action":"add","text":"b","blocked_by":[9]})).await.unwrap_err().to_string();
        assert!(err.contains("unknown todo #9"), "{err}");
        let err = call(&c, json!({"action":"update","id":1,"blocked_by":[1]})).await.unwrap_err().to_string();
        assert!(err.contains("itself"), "{err}");
        let err = call(&c, json!({"action":"set","items":[{"text":"x","blocked_by":[5]}]})).await.unwrap_err().to_string();
        assert!(err.contains("unknown todo #5"), "{err}");
        // old JSON without the new fields still loads
        let it: TodoItem = serde_json::from_str(r#"{"id":1,"text":"t","status":"pending"}"#).unwrap();
        assert!(it.blocked_by.is_empty() && it.owner.is_empty());
    }
}
