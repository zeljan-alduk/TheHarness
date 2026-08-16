//! plan_mode: model-driven plan mode. `enter` switches the session policy to `Mode::Plan` (read-only) so
//! the model can explore safely; `exit {plan}` presents the plan to the user for approval and restores the
//! previous permission mode (or switches to "ask") — or stays in plan mode when the user asks for changes.
//! The shared `Policy` is reached through `ctx.subagent` (the session env); sub-agents get an error.

use super::{Tool, ToolCtx, ToolOutput};
use crate::permissions::{Answer, Mode, Question, QuestionOption};
use anyhow::{bail, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Mutex;

/// The mode that was active before the model entered plan mode (restored on approval).
static PREVIOUS_MODE: Mutex<Option<Mode>> = Mutex::new(None);

/// Longest plan text shown in the approval prompt (the full plan is still in the tool result).
const PROMPT_MAX: usize = 4000;

/// Approval options, in order (indices are matched by `outcome_for`).
const OPT_APPROVE: usize = 0;
const OPT_APPROVE_ASK: usize = 1;
const OPT_REVISE: usize = 2;

/// What an exit-plan answer means for the session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Leave plan mode; switch to this mode.
    Approve(Mode),
    /// Stay in plan mode; the user's feedback (may be empty).
    Revise(String),
}

/// Map the user's answer to an outcome. `previous` is the mode active before plan mode (None or Plan → Auto).
/// A declined/timed-out answer or free text without a choice counts as "revise".
pub fn outcome_for(answer: &Answer, previous: Option<Mode>) -> Outcome {
    let restored = match previous { None | Some(Mode::Plan) => Mode::Auto, Some(m) => m };
    let text = answer.text.as_deref().map(str::trim).unwrap_or("").to_string();
    if answer.declined { return Outcome::Revise(if text.is_empty() { "the user declined the plan".into() } else { text }); }
    if answer.timed_out { return Outcome::Revise(if text.is_empty() { "the user did not answer in time".into() } else { text }); }
    match answer.choice {
        Some(OPT_APPROVE) => Outcome::Approve(restored),
        Some(OPT_APPROVE_ASK) => Outcome::Approve(Mode::Ask),
        Some(OPT_REVISE) => Outcome::Revise(text),
        _ => Outcome::Revise(text),
    }
}

fn truncate_for_prompt(plan: &str) -> String {
    if plan.chars().count() <= PROMPT_MAX { return plan.to_string() }
    let cut: String = plan.chars().take(PROMPT_MAX).collect();
    format!("{cut}\n… [plan truncated for the prompt; {} chars total]", plan.chars().count())
}

pub struct PlanMode;

#[async_trait]
impl Tool for PlanMode {
    fn name(&self) -> &'static str { "plan_mode" }
    fn description(&self) -> &'static str { "Enter/exit plan mode. Use `enter` before non-trivial multi-file changes or when the user asked for a plan first: file-modifying tools are blocked while you explore; then call `exit` with the plan (markdown) — the user approves it (permissions restored) or asks for changes (you stay in plan mode)." }
    fn parameters(&self) -> Value { json!({"type":"object","properties":{"action":{"type":"string","enum":["enter","exit"]},"plan":{"type":"string","description":"exit: the plan to present for approval (markdown; required)"}},"required":["action"]}) }
    /// Not truly read-only (it changes the permission mode) but it must be callable while in plan mode —
    /// `Policy::check` only lets read-only tools through in `Mode::Plan` — and it touches no files.
    fn read_only(&self) -> bool { true }
    /// Never run concurrently with other tools: it flips the mode the other calls are checked against.
    fn parallel_safe(&self) -> bool { false }

    async fn call(&self, args: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("").trim().to_lowercase();
        let Some(env) = &ctx.subagent else { bail!("plan_mode is not available here") };
        let policy = &env.policy;
        match action.as_str() {
            "enter" => {
                let cur = policy.mode();
                if cur == Mode::Plan { return Ok("already in plan mode: file-modifying tools are blocked; explore, then call plan_mode exit {plan:\"...\"} with the plan (markdown) for approval".into()) }
                *PREVIOUS_MODE.lock().unwrap() = Some(cur);
                policy.set_mode(Mode::Plan);
                Ok("plan mode on: file-modifying tools are blocked; explore, then call plan_mode exit {plan:\"...\"} with the plan (markdown) for approval".into())
            }
            "exit" => {
                let plan = args.get("plan").and_then(|v| v.as_str()).map(str::trim).unwrap_or("");
                if plan.is_empty() { bail!("plan_mode exit: 'plan' (markdown) is required") }
                if policy.mode() != Mode::Plan { bail!("not in plan mode (current: {}); call plan_mode enter first", policy.mode().label()) }
                let previous = *PREVIOUS_MODE.lock().unwrap();
                let Some(approver) = &ctx.approver else {
                    let m = match outcome_for(&Answer { choice: Some(OPT_APPROVE), ..Default::default() }, previous) { Outcome::Approve(m) => m, Outcome::Revise(_) => Mode::Auto };
                    policy.set_mode(m); *PREVIOUS_MODE.lock().unwrap() = None;
                    return Ok(format!("no user available (headless) — plan auto-approved; permissions: {}. Proceed.\n\nPlan:\n{plan}", m.label()).into())
                };
                let q = Question {
                    question: format!("The model proposes this plan:\n\n{}\n\nApprove it?", truncate_for_prompt(plan)),
                    options: vec![
                        QuestionOption { label: "Approve — proceed".into(), description: "leave plan mode and restore the previous permissions".into() },
                        QuestionOption { label: "Approve, keep asking before changes".into(), description: "leave plan mode; every file-modifying tool call is confirmed first".into() },
                        QuestionOption { label: "Revise — stay in plan mode".into(), description: "type what should change".into() },
                    ],
                    allow_free_text: true, timeout_secs: None,
                };
                let outcome = match approver.question(q).await {
                    None => { let Outcome::Approve(m) = outcome_for(&Answer { choice: Some(OPT_APPROVE), ..Default::default() }, previous) else { unreachable!() }; policy.set_mode(m); *PREVIOUS_MODE.lock().unwrap() = None; return Ok(format!("no user available (non-interactive) — plan auto-approved; permissions: {}. Proceed.\n\nPlan:\n{plan}", m.label()).into()) }
                    Some(a) => outcome_for(&a, previous),
                };
                match outcome {
                    Outcome::Approve(m) => { policy.set_mode(m); *PREVIOUS_MODE.lock().unwrap() = None; Ok(format!("plan approved; permissions: {}. Proceed.\n\nPlan:\n{plan}", m.label()).into()) }
                    Outcome::Revise(t) => Ok(format!("user asked for changes: {}\n(still in plan mode — revise and call plan_mode exit again)\n\nPlan:\n{plan}", if t.is_empty() { "(no details given)" } else { t.as_str() }).into()),
                }
            }
            _ => bail!("plan_mode: action must be 'enter' or 'exit'"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permissions::{Approval, ApprovalRequest, Approver, PermissionsConfig, Policy};
    use std::sync::Arc;

    #[tokio::test]
    async fn unavailable_without_session_env() {
        let dir = std::env::temp_dir();
        let ctx = ToolCtx::basic(dir.clone());
        let e = PlanMode.call(json!({"action":"enter"}), &ctx).await.err().unwrap().to_string();
        assert!(e.contains("plan_mode is not available here"), "{e}");
    }

    #[test]
    fn answer_to_outcome() {
        let a = |c: Option<usize>, t: Option<&str>| Answer { choice: c, text: t.map(String::from), ..Default::default() };
        assert_eq!(outcome_for(&a(Some(0), None), Some(Mode::Bypass)), Outcome::Approve(Mode::Bypass));
        assert_eq!(outcome_for(&a(Some(0), None), None), Outcome::Approve(Mode::Auto));
        assert_eq!(outcome_for(&a(Some(0), None), Some(Mode::Plan)), Outcome::Approve(Mode::Auto));
        assert_eq!(outcome_for(&a(Some(1), None), Some(Mode::Bypass)), Outcome::Approve(Mode::Ask));
        assert_eq!(outcome_for(&a(Some(2), Some(" split it up ")), Some(Mode::Auto)), Outcome::Revise("split it up".into()));
        assert_eq!(outcome_for(&a(None, Some("no")), Some(Mode::Auto)), Outcome::Revise("no".into()));
        assert_eq!(outcome_for(&Answer { declined: true, ..Default::default() }, Some(Mode::Auto)), Outcome::Revise("the user declined the plan".into()));
    }

    /// Scripted approver: answers every question with a fixed choice/text.
    struct Scripted(Option<usize>, Option<&'static str>);
    #[async_trait]
    impl Approver for Scripted {
        async fn ask(&self, _r: ApprovalRequest) -> Approval { Approval::Deny }
        async fn question(&self, _q: Question) -> Option<Answer> { Some(Answer { choice: self.0, text: self.1.map(String::from), ..Default::default() }) }
    }

    fn session_ctx(dir: &std::path::Path, approver: Option<Arc<dyn Approver>>) -> (ToolCtx, Arc<Policy>) {
        let cfg: crate::config::LlmConfig = serde_json::from_value(json!({"base_url":"http://127.0.0.1:1/v1","model":"test"})).unwrap();
        let client = crate::llm::Client::new(&cfg).unwrap();
        let policy = Arc::new(Policy::new(PermissionsConfig::default(), dir));
        let appr: Arc<dyn Approver> = approver.clone().unwrap_or_else(|| Arc::new(crate::permissions::AutoApprover { yes: false }));
        let env = crate::agent::SubAgentEnv::new(client, crate::tools::Registry::defaults(false), policy.clone(), appr, Arc::new(crate::events::StderrSink { verbose: false }), 1000, false);
        let mut ctx = ToolCtx::basic(dir.to_path_buf());
        ctx.subagent = Some(Arc::new(env));
        ctx.approver = approver;
        (ctx, policy)
    }

    // The tests below share PREVIOUS_MODE (a process-wide static) — one serialised test.
    #[tokio::test]
    async fn enter_exit_flow() {
        let dir = std::env::temp_dir();
        // approve → previous mode restored
        let (ctx, policy) = session_ctx(&dir, Some(Arc::new(Scripted(Some(0), None))));
        policy.set_mode(Mode::Bypass);
        let e = PlanMode.call(json!({"action":"exit","plan":"x"}), &ctx).await.err().unwrap().to_string();
        assert!(e.contains("not in plan mode"), "{e}");
        let out = PlanMode.call(json!({"action":"enter"}), &ctx).await.unwrap().text;
        assert!(out.starts_with("plan mode on"), "{out}");
        assert_eq!(policy.mode(), Mode::Plan);
        let out = PlanMode.call(json!({"action":"enter"}), &ctx).await.unwrap().text;
        assert!(out.starts_with("already in plan mode"), "{out}");
        assert!(PlanMode.call(json!({"action":"exit"}), &ctx).await.is_err());
        let out = PlanMode.call(json!({"action":"exit","plan":"# Plan\n1. do it"}), &ctx).await.unwrap().text;
        assert!(out.contains("plan approved") && out.contains("bypass permissions on") && out.contains("1. do it"), "{out}");
        assert_eq!(policy.mode(), Mode::Bypass);

        // revise → stays in plan mode; then "approve, keep asking" → Ask
        let (ctx, policy) = session_ctx(&dir, Some(Arc::new(Scripted(Some(2), Some("smaller steps")))));
        PlanMode.call(json!({"action":"enter"}), &ctx).await.unwrap();
        let out = PlanMode.call(json!({"action":"exit","plan":"p"}), &ctx).await.unwrap().text;
        assert!(out.contains("user asked for changes: smaller steps"), "{out}");
        assert_eq!(policy.mode(), Mode::Plan);
        let mut ctx2 = ctx.clone();
        ctx2.approver = Some(Arc::new(Scripted(Some(1), None)));
        let out = PlanMode.call(json!({"action":"exit","plan":"p2"}), &ctx2).await.unwrap().text;
        assert!(out.contains("ask before changes"), "{out}");
        assert_eq!(policy.mode(), Mode::Ask);

        // headless (no approver) → auto-approve
        let (ctx, policy) = session_ctx(&dir, None);
        PlanMode.call(json!({"action":"enter"}), &ctx).await.unwrap();
        let out = PlanMode.call(json!({"action":"exit","plan":"p"}), &ctx).await.unwrap().text;
        assert!(out.contains("auto-approved"), "{out}");
        assert_eq!(policy.mode(), Mode::Auto);
    }
}
