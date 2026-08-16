//! report_findings: TODO (stub — being implemented).

use super::{Tool, ToolCtx, ToolOutput};
use anyhow::{bail, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct ReportFindings;

#[async_trait]
impl Tool for ReportFindings {
    fn name(&self) -> &'static str { "report_findings" }
    fn description(&self) -> &'static str { "not implemented yet" }
    fn parameters(&self) -> Value { json!({"type":"object","properties":{}}) }
    async fn call(&self, _args: Value, _ctx: &ToolCtx) -> Result<ToolOutput> { bail!("report_findings: not implemented yet") }
}
