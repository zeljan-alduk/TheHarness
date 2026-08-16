//! list_sessions / send_message: talk to other live harness sessions (other terminals, the desktop app,
//! sub-sessions on the same config dir). Messages arrive in the target's inbox as wakeups.

use super::{arg_str, Tool, ToolCtx, ToolOutput};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct ListSessions;
pub struct SendMessage;

#[async_trait]
impl Tool for ListSessions {
    fn read_only(&self) -> bool { true }
    fn name(&self) -> &'static str { "list_sessions" }
    fn description(&self) -> &'static str { "List other live harness sessions (id, title, workdir, backend, busy) that you can message with send_message." }
    fn parameters(&self) -> Value { json!({"type":"object","properties":{}}) }
    async fn call(&self, _args: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let me = ctx.session_id.clone().unwrap_or_default();
        let l = crate::mailbox::live();
        if l.is_empty() { return Ok("no live sessions".into()); }
        Ok(l.iter().map(|s| format!("{}{}  {:<40} {}  [{}]{}", if s.id == me { "● " } else { "  " }, s.id, crate::llm::truncate_for_log(&s.title, 40), s.workdir, s.backend, if s.busy { " busy" } else { "" })).collect::<Vec<_>>().join("\n").into())
    }
}

#[async_trait]
impl Tool for SendMessage {
    fn name(&self) -> &'static str { "send_message" }
    fn description(&self) -> &'static str { "Send a message to another live harness session (by id, id prefix, title fragment, or 'all'). It is delivered to that session's agent as an inbox event (it wakes up if idle). Use for coordination between parallel sessions." }
    fn parameters(&self) -> Value { json!({"type":"object","properties":{"to":{"type":"string"},"text":{"type":"string"}},"required":["to","text"]}) }
    async fn call(&self, args: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let to = arg_str(&args, "to")?; let text = arg_str(&args, "text")?;
        let from = ctx.session_id.clone().unwrap_or_else(|| "cli".into());
        let n = crate::mailbox::send(to, &from, text)?;
        Ok(format!("delivered to {n} session(s)").into())
    }
}
