//! agents: TODO (stub — being implemented).

use super::{Tool, ToolCtx, ToolOutput};
use anyhow::{bail, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct Agents;

#[async_trait]
impl Tool for Agents {
    fn name(&self) -> &'static str { "agents" }
    fn description(&self) -> &'static str { "not implemented yet" }
    fn parameters(&self) -> Value { json!({"type":"object","properties":{}}) }
    async fn call(&self, _args: Value, _ctx: &ToolCtx) -> Result<ToolOutput> { bail!("agents: not implemented yet") }
}
