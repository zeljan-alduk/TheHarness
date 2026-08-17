//! OpenTelemetry export (OTLP/HTTP, JSON encoding). The agent already emits a structured Event
//! stream; this turns it into OTLP log records and a few GenAI-semconv metrics so a run shows up in
//! whatever the team already runs (Grafana, Honeycomb, Datadog…). No SDK dependency: OTLP/HTTP+JSON is
//! a POST with a well-known body shape.

use crate::events::{Event, Sink};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct TelemetryConfig {
    /// e.g. "http://localhost:4318" (OTLP/HTTP). Empty = telemetry off.
    #[serde(default)] pub otlp_endpoint: String,
    /// Extra resource attributes, e.g. { deployment = "laptop" }.
    #[serde(default)] pub attributes: std::collections::HashMap<String, String>,
    /// Also send tool arguments and model text (off by default: they contain your code).
    #[serde(default)] pub include_content: bool,
    /// Flush at most this many records per request.
    #[serde(default = "d_batch")] pub batch: usize,
}
fn d_batch() -> usize { 64 }

fn now_nanos() -> u64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0) }

fn attr(k: &str, v: Value) -> Value {
    let value = match v {
        Value::String(s) => json!({"stringValue": s}),
        Value::Number(n) if n.is_f64() => json!({"doubleValue": n.as_f64().unwrap_or(0.0)}),
        Value::Number(n) => json!({"intValue": n.as_i64().unwrap_or(0).to_string()}),
        Value::Bool(b) => json!({"boolValue": b}),
        other => json!({"stringValue": other.to_string()}),
    };
    json!({"key": k, "value": value})
}

/// One event → (name, severity, attributes). None for events not worth exporting.
pub fn record_of(e: &Event, include_content: bool) -> Option<(String, i64, Vec<Value>)> {
    let text = |s: &str| if include_content { s.to_string() } else { format!("{} chars", s.chars().count()) };
    Some(match e {
        Event::RunStarted { model, workdir, tools } => ("gen_ai.run.start".into(), 9, vec![attr("gen_ai.request.model", json!(model)), attr("harness.workdir", json!(workdir)), attr("harness.tool_count", json!(tools.len()))]),
        Event::ModelResponse { prompt_tokens, completion_tokens, ttft_secs, secs, tool_calls } => ("gen_ai.client.inference".into(), 9, vec![
            attr("gen_ai.usage.input_tokens", json!(prompt_tokens)), attr("gen_ai.usage.output_tokens", json!(completion_tokens)),
            attr("gen_ai.server.time_to_first_token", json!(ttft_secs)), attr("gen_ai.client.operation.duration", json!(secs)),
            attr("gen_ai.response.tool_calls", json!(tool_calls))]),
        Event::ToolCall { name, args, .. } => ("gen_ai.tool.call".into(), 9, vec![attr("gen_ai.tool.name", json!(name)), attr("gen_ai.tool.arguments", json!(text(args)))]),
        Event::ToolResult { name, result, secs, .. } => ("gen_ai.tool.result".into(), if result.starts_with("error:") { 17 } else { 9 }, vec![
            attr("gen_ai.tool.name", json!(name)), attr("harness.tool.duration", json!(secs)), attr("harness.tool.result", json!(text(result))),
            attr("harness.tool.is_error", json!(result.starts_with("error:")))]),
        Event::Permission { tool, decision, .. } => ("harness.permission".into(), 13, vec![attr("gen_ai.tool.name", json!(tool)), attr("harness.permission.decision", json!(decision))]),
        Event::Compacted { count, prompt_tokens, .. } => ("harness.compaction".into(), 9, vec![attr("harness.compaction.messages", json!(count)), attr("harness.compaction.prompt_tokens", json!(prompt_tokens))]),
        Event::Error { message } => ("harness.error".into(), 17, vec![attr("exception.message", json!(message))]),
        Event::RunFinished { stop_reason, turns, tool_calls, prompt_tokens, completion_tokens, wall_secs } => ("gen_ai.run.finish".into(), 9, vec![
            attr("harness.stop_reason", json!(stop_reason)), attr("harness.turns", json!(turns)), attr("harness.tool_calls", json!(tool_calls)),
            attr("gen_ai.usage.input_tokens", json!(prompt_tokens)), attr("gen_ai.usage.output_tokens", json!(completion_tokens)),
            attr("harness.run.duration", json!(wall_secs)), attr("harness.cost_usd", json!(crate::pricing::spent_usd()))]),
        _ => return None,
    })
}

/// A Sink that batches events and ships them to an OTLP collector, wrapping the sink it decorates.
pub struct OtelSink { pub inner: Arc<dyn Sink>, cfg: TelemetryConfig, buf: Arc<Mutex<Vec<Value>>>, resource: Value }

impl OtelSink {
    pub fn wrap(inner: Arc<dyn Sink>, cfg: TelemetryConfig, service: &str) -> Arc<dyn Sink> {
        if cfg.otlp_endpoint.trim().is_empty() { return inner; }
        let mut attrs = vec![attr("service.name", json!(service)), attr("service.version", json!(crate::VERSION))];
        for (k, v) in &cfg.attributes { attrs.push(attr(k, json!(v))); }
        Arc::new(OtelSink { inner, cfg, buf: Arc::new(Mutex::new(Vec::new())), resource: json!({"attributes": attrs}) })
    }

    fn flush(&self, records: Vec<Value>) {
        if records.is_empty() { return; }
        let url = format!("{}/v1/logs", self.cfg.otlp_endpoint.trim_end_matches('/'));
        let body = json!({"resourceLogs": [{"resource": self.resource, "scopeLogs": [{"scope": {"name": "harness"}, "logRecords": records}]}]});
        tokio::spawn(async move {
            let Ok(http) = reqwest::Client::builder().timeout(std::time::Duration::from_secs(10)).build() else { return };
            let _ = http.post(&url).json(&body).send().await;
        });
    }
}

impl Sink for OtelSink {
    fn emit(&self, e: &Event) {
        self.inner.emit(e);
        let Some((name, severity, attributes)) = record_of(e, self.cfg.include_content) else { return };
        let rec = json!({
            "timeUnixNano": now_nanos().to_string(),
            "severityNumber": severity,
            "body": {"stringValue": name},
            "attributes": attributes,
        });
        let batch = { let mut g = self.buf.lock().unwrap(); g.push(rec); if g.len() >= self.cfg.batch.max(1) || matches!(e, Event::RunFinished { .. }) { std::mem::take(&mut *g) } else { Vec::new() } };
        self.flush(batch);
    }
}

impl Drop for OtelSink {
    fn drop(&mut self) { let rest = std::mem::take(&mut *self.buf.lock().unwrap()); self.flush(rest); }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_events_to_otlp_records() {
        let (name, sev, attrs) = record_of(&Event::ToolCall { id: "1".into(), name: "bash".into(), args: "{\"cmd\":\"ls\"}".into() }, true).unwrap();
        assert_eq!(name, "gen_ai.tool.call");
        assert_eq!(sev, 9);
        assert!(attrs.iter().any(|a| a["key"] == "gen_ai.tool.name" && a["value"]["stringValue"] == "bash"));
        // without include_content the arguments are reduced to a size
        let (_, _, attrs) = record_of(&Event::ToolCall { id: "1".into(), name: "bash".into(), args: "secret".into() }, false).unwrap();
        assert!(attrs.iter().any(|a| a["value"]["stringValue"].as_str().map(|s| s.ends_with("chars")).unwrap_or(false)));
        // an error result raises the severity
        let (_, sev, _) = record_of(&Event::ToolResult { id: "1".into(), name: "bash".into(), result: "error: boom".into(), secs: 0.1, images: vec![] }, false).unwrap();
        assert_eq!(sev, 17);
        assert!(record_of(&Event::ReasoningDelta { text: "x".into() }, false).is_none(), "deltas are not exported");
        // off by default: wrap returns the sink untouched
        struct Null;
        impl Sink for Null { fn emit(&self, _: &Event) {} }
        let inner: Arc<dyn Sink> = Arc::new(Null);
        assert!(Arc::ptr_eq(&OtelSink::wrap(inner.clone(), TelemetryConfig::default(), "t"), &inner));
    }
}
