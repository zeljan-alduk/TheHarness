//! notify: TODO (stub — being implemented).

use super::{Tool, ToolCtx, ToolOutput};
use anyhow::{bail, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct Notify;

#[async_trait]
impl Tool for Notify {
    fn name(&self) -> &'static str { "notify" }
    fn description(&self) -> &'static str { "not implemented yet" }
    fn parameters(&self) -> Value { json!({"type":"object","properties":{}}) }
    async fn call(&self, _args: Value, _ctx: &ToolCtx) -> Result<ToolOutput> { bail!("notify: not implemented yet") }
}
