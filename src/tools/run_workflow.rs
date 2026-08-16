//! run_workflow: TODO (stub — being implemented).

use super::{Tool, ToolCtx, ToolOutput};
use anyhow::{bail, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct RunWorkflow;

#[async_trait]
impl Tool for RunWorkflow {
    fn name(&self) -> &'static str { "run_workflow" }
    fn description(&self) -> &'static str { "not implemented yet" }
    fn parameters(&self) -> Value { json!({"type":"object","properties":{}}) }
    async fn call(&self, _args: Value, _ctx: &ToolCtx) -> Result<ToolOutput> { bail!("run_workflow: not implemented yet") }
}
