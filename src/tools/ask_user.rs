//! ask_user: let the model ask the user a question (multiple choice and/or free text) when a decision
//! is genuinely the user's. Interactive front ends answer; headless runs get "no user available".

use super::{arg_str, Tool, ToolCtx, ToolOutput};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct AskUser;

#[async_trait]
impl Tool for AskUser {
    fn name(&self) -> &'static str { "ask_user" }
    fn description(&self) -> &'static str { "Ask the user a question when a decision is genuinely theirs (design choice, ambiguity that changes the work, destructive action). Provide 2–4 options with short descriptions when possible; the user may also type free text. Do not use it for things you can decide or verify yourself. Blocks until answered." }
    fn parameters(&self) -> Value { json!({"type":"object","properties":{"question":{"type":"string"},"options":{"type":"array","items":{"type":"object","properties":{"label":{"type":"string"},"description":{"type":"string"}},"required":["label"]}},"allow_free_text":{"type":"boolean","description":"default true"}},"required":["question"]}) }
    async fn call(&self, args: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let question = arg_str(&args, "question")?.to_string();
        let options: Vec<crate::permissions::QuestionOption> = args.get("options").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|o| o.get("label").and_then(|l| l.as_str()).map(|l| crate::permissions::QuestionOption { label: l.to_string(), description: o.get("description").and_then(|d| d.as_str()).unwrap_or("").to_string() })).collect()).unwrap_or_default();
        let allow_free_text = args.get("allow_free_text").and_then(|v| v.as_bool()).unwrap_or(true);
        let Some(approver) = &ctx.approver else { return Ok("no user is available to answer (headless run) — decide yourself using the most reasonable default and state the assumption in your final answer.".into()) };
        let q = crate::permissions::Question { question: question.clone(), options: options.clone(), allow_free_text, timeout_secs: None };
        match approver.question(q).await {
            None => Ok("no user is available to answer (non-interactive) — decide yourself using the most reasonable default and state the assumption in your final answer.".into()),
            Some(a) => {
                if a.declined { return Ok("the user declined to answer — proceed with the most reasonable default and say so.".into()); }
                if a.timed_out { return Ok("the user did not answer in time — proceed with the most reasonable default and say so.".into()); }
                let mut s = String::new();
                if let Some(i) = a.choice { if let Some(o) = options.get(i) { s.push_str(&format!("user chose: {}{}", o.label, if o.description.is_empty() { String::new() } else { format!(" — {}", o.description) })); } }
                if let Some(t) = &a.text { if !t.trim().is_empty() { if !s.is_empty() { s.push_str("\n"); } s.push_str(&format!("user says: {}", t.trim())); } }
                if s.is_empty() { s = "user answered without a choice or text".into(); }
                Ok(s.into())
            }
        }
    }
}
