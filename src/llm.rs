//! Minimal OpenAI-compatible chat client (works with LM Studio, llama-server, Ollama, vLLM).

use anyhow::{bail, Context, Result};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Streaming increments handed to the caller as they arrive.
#[derive(Debug, Clone)]
pub enum Delta { Reasoning(String), Content(String) }

/// OpenAI content: either a plain string or an array of parts (text / image_url).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Content {
    Text(String),
    Parts(Vec<Value>),
}

impl Content {
    /// Concatenated text of the content (image parts contribute a placeholder).
    pub fn text(&self) -> String {
        match self {
            Content::Text(s) => s.clone(),
            Content::Parts(parts) => parts.iter().map(|p| match p["type"].as_str() {
                Some("text") => p["text"].as_str().unwrap_or("").to_string(),
                Some("image_url") => "[image]".to_string(),
                _ => String::new(),
            }).collect::<Vec<_>>().join("\n"),
        }
    }
    pub fn image_part(mime: &str, b64: &str) -> Value {
        json!({"type": "image_url", "image_url": {"url": format!("data:{mime};base64,{b64}")}})
    }
    pub fn text_part(s: &str) -> Value { json!({"type": "text", "text": s}) }
}
impl From<String> for Content { fn from(s: String) -> Self { Content::Text(s) } }
impl From<&str> for Content { fn from(s: &str) -> Self { Content::Text(s.to_string()) } }

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Message {
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<Content>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Reasoning channel (Qwen3 / DeepSeek style). Never sent back to the server.
    #[serde(default, skip_serializing, alias = "reasoning")]
    pub reasoning_content: Option<String>,
}

impl Message {
    pub fn system(s: impl Into<String>) -> Self { Self { role: "system".into(), content: Some(Content::Text(s.into())), ..Default::default() } }
    pub fn user(s: impl Into<String>) -> Self { Self { role: "user".into(), content: Some(Content::Text(s.into())), ..Default::default() } }
    pub fn user_parts(parts: Vec<Value>) -> Self { Self { role: "user".into(), content: Some(Content::Parts(parts)), ..Default::default() } }
    pub fn tool(id: impl Into<String>, name: impl Into<String>, s: impl Into<String>) -> Self {
        Self { role: "tool".into(), content: Some(Content::Text(s.into())), tool_call_id: Some(id.into()), name: Some(name.into()), ..Default::default() }
    }
    /// Text view of the content ("" if none).
    pub fn text(&self) -> String { self.content.as_ref().map(|c| c.text()).unwrap_or_default() }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    #[serde(default)]
    pub id: String,
    #[serde(rename = "type", default = "fn_type")]
    pub kind: String,
    pub function: FunctionCall,
}
fn fn_type() -> String { "function".into() }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    #[serde(default)]
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolDef {
    #[serde(rename = "type")]
    pub kind: String,
    pub function: FunctionDef,
}
#[derive(Debug, Clone, Serialize)]
pub struct FunctionDef {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}
impl ToolDef {
    pub fn new(name: &str, description: &str, parameters: Value) -> Self {
        Self { kind: "function".into(), function: FunctionDef { name: name.into(), description: description.into(), parameters } }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Usage {
    #[serde(default)]
    pub prompt_tokens: u64,
    #[serde(default)]
    pub completion_tokens: u64,
    #[serde(default)]
    #[allow(dead_code)]
    pub total_tokens: u64,
}

pub struct Client {
    http: reqwest::Client,
    base_url: String,
    model: String,
    api_key: Option<String>,
    temperature: f32,
    max_tokens: u32,
}

impl Client {
    pub fn new(cfg: &crate::config::LlmConfig) -> Result<Self> {
        // No total timeout: a long reasoning turn can legitimately take many minutes on a local model.
        // Instead: bounded connect time and a read (stall) timeout between streamed chunks.
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(30))
            .read_timeout(std::time::Duration::from_secs(300))
            .build()?;
        Ok(Self {
            http,
            base_url: cfg.base_url.trim_end_matches('/').to_string(),
            model: cfg.model.clone(),
            api_key: cfg.api_key.clone(),
            temperature: cfg.temperature,
            max_tokens: cfg.max_tokens,
        })
    }

    pub fn model(&self) -> &str { &self.model }

    pub async fn list_models(&self) -> Result<Vec<String>> {
        let mut req = self.http.get(format!("{}/models", self.base_url));
        if let Some(k) = &self.api_key { req = req.bearer_auth(k); }
        let v: Value = req.send().await?.error_for_status()?.json().await?;
        Ok(v["data"].as_array().map(|a| a.iter().filter_map(|m| m["id"].as_str().map(String::from)).collect()).unwrap_or_default())
    }

    pub async fn chat(&self, messages: &[Message], tools: &[ToolDef]) -> Result<(Message, Usage)> {
        let mut body = json!({
            "model": self.model,
            "messages": messages,
            "temperature": self.temperature,
            "max_tokens": self.max_tokens,
        });
        if !tools.is_empty() {
            body["tools"] = json!(tools);
            body["tool_choice"] = json!("auto");
        }
        let mut req = self.http.post(format!("{}/chat/completions", self.base_url)).json(&body);
        if let Some(k) = &self.api_key { req = req.bearer_auth(k); }
        let resp = req.send().await.context("LLM request failed (is the server running?)")?;
        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            bail!("LLM server returned {}: {}", status, truncate_for_log(&text, 800));
        }
        #[derive(Deserialize)]
        struct Choice { message: Message, #[serde(default)] finish_reason: Option<String> }
        #[derive(Deserialize)]
        struct Resp { choices: Vec<Choice>, #[serde(default)] usage: Option<Usage> }
        let r: Resp = serde_json::from_str(&text).with_context(|| format!("bad LLM JSON: {}", truncate_for_log(&text, 800)))?;
        let mut choice = r.choices.into_iter().next().context("LLM returned no choices")?;
        let mut msg = choice.message;
        // Some servers leave <think>…</think> inline instead of a reasoning field; split it out.
        if let Some(c) = msg.content.take() {
            let (think, rest) = split_think(&c.text());
            if let Some(t) = think { msg.reasoning_content.get_or_insert_with(String::new).push_str(&t); }
            msg.content = Some(Content::Text(rest));
        }
        if choice.finish_reason.as_deref() == Some("length") {
            let mut t = msg.text(); t.push_str("\n[output truncated by max_tokens]");
            msg.content = Some(Content::Text(t));
        }
        choice.finish_reason = None;
        Ok((msg, r.usage.unwrap_or_default()))
    }
}

impl Client {
    /// Same as `chat` but streams (SSE). `on_delta` is called for each reasoning/content increment;
    /// tool-call fragments are assembled internally and returned in the final message.
    pub async fn chat_stream(&self, messages: &[Message], tools: &[ToolDef], mut on_delta: impl FnMut(Delta)) -> Result<(Message, Usage)> {
        let mut body = json!({
            "model": self.model,
            "messages": messages,
            "temperature": self.temperature,
            "max_tokens": self.max_tokens,
            "stream": true,
            "stream_options": {"include_usage": true},
        });
        if !tools.is_empty() { body["tools"] = json!(tools); body["tool_choice"] = json!("auto"); }
        let mut req = self.http.post(format!("{}/chat/completions", self.base_url)).json(&body);
        if let Some(k) = &self.api_key { req = req.bearer_auth(k); }
        let resp = req.send().await.context("LLM request failed (is the server running?)")?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            bail!("LLM server returned {}: {}", status, truncate_for_log(&text, 800));
        }
        let mut stream = resp.bytes_stream();
        let mut buf = String::new();
        let mut content = String::new();
        let mut reasoning = String::new();
        let mut calls: Vec<ToolCall> = Vec::new();
        let mut usage = Usage::default();
        let mut finish: Option<String> = None;
        'outer: while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("stream error")?;
            buf.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(pos) = buf.find('\n') {
                let line = buf[..pos].trim().to_string();
                buf.drain(..=pos);
                let Some(data) = line.strip_prefix("data:") else { continue };
                let data = data.trim();
                if data == "[DONE]" { break 'outer; }
                let v: Value = match serde_json::from_str(data) { Ok(v) => v, Err(_) => continue };
                if let Some(u) = v.get("usage") { if !u.is_null() { if let Ok(u) = serde_json::from_value::<Usage>(u.clone()) { usage = u; } } }
                if let Some(err) = v.get("error") { bail!("LLM stream error: {err}"); }
                for ch in v["choices"].as_array().cloned().unwrap_or_default() {
                    let d = &ch["delta"];
                    if let Some(t) = d["reasoning_content"].as_str().or_else(|| d["reasoning"].as_str()) {
                        if !t.is_empty() { reasoning.push_str(t); on_delta(Delta::Reasoning(t.to_string())); }
                    }
                    if let Some(t) = d["content"].as_str() {
                        if !t.is_empty() { content.push_str(t); on_delta(Delta::Content(t.to_string())); }
                    }
                    if let Some(tcs) = d["tool_calls"].as_array() {
                        for tc in tcs {
                            let idx = tc["index"].as_u64().unwrap_or(calls.len() as u64) as usize;
                            while calls.len() <= idx { calls.push(ToolCall { id: String::new(), kind: "function".into(), function: FunctionCall { name: String::new(), arguments: String::new() } }); }
                            if let Some(id) = tc["id"].as_str() { if !id.is_empty() { calls[idx].id = id.to_string(); } }
                            if let Some(n) = tc["function"]["name"].as_str() { calls[idx].function.name.push_str(n); }
                            if let Some(a) = tc["function"]["arguments"].as_str() { calls[idx].function.arguments.push_str(a); }
                        }
                    }
                    if let Some(fr) = ch["finish_reason"].as_str() { finish = Some(fr.to_string()); }
                }
            }
        }
        // <think> tags inline (servers that don't split reasoning)
        let (think, rest) = split_think(&content);
        if let Some(t) = think { reasoning.push_str(&t); }
        let mut content = rest;
        if finish.as_deref() == Some("length") { content.push_str("\n[output truncated by max_tokens]"); }
        for (i, c) in calls.iter_mut().enumerate() { if c.id.is_empty() { c.id = format!("call_{i}"); } }
        let msg = Message {
            role: "assistant".into(),
            content: if content.is_empty() && !calls.is_empty() { None } else { Some(Content::Text(content)) },
            tool_calls: if calls.is_empty() { None } else { Some(calls) },
            reasoning_content: if reasoning.is_empty() { None } else { Some(reasoning) },
            ..Default::default()
        };
        Ok((msg, usage))
    }
}

/// Detect the loaded context length of `model` from the server. Tries LM Studio (`/api/v0/models`),
/// llama.cpp server (`/props`), Ollama (`/api/show`). Returns (tokens, source).
pub async fn detect_context_length(base_url: &str, model: &str) -> Option<(u64, &'static str)> {
    let root = base_url.trim_end_matches('/').trim_end_matches("/v1").to_string();
    let http = reqwest::Client::builder().timeout(std::time::Duration::from_secs(4)).build().ok()?;
    // LM Studio
    if let Ok(r) = http.get(format!("{root}/api/v0/models")).send().await {
        if let Ok(v) = r.json::<Value>().await {
            for m in v["data"].as_array().cloned().unwrap_or_default() {
                if m["id"].as_str() == Some(model) {
                    if let Some(n) = m["loaded_context_length"].as_u64() { return Some((n, "LM Studio (loaded)")); }
                    if let Some(n) = m["max_context_length"].as_u64() { return Some((n, "LM Studio (max)")); }
                }
            }
        }
    }
    // llama.cpp server
    if let Ok(r) = http.get(format!("{root}/props")).send().await {
        if let Ok(v) = r.json::<Value>().await { if let Some(n) = v["default_generation_settings"]["n_ctx"].as_u64().or_else(|| v["n_ctx"].as_u64()) { return Some((n, "llama.cpp")); } }
    }
    // Ollama
    if let Ok(r) = http.post(format!("{root}/api/show")).json(&json!({"model": model})).send().await {
        if let Ok(v) = r.json::<Value>().await {
            if let Some(obj) = v["model_info"].as_object() { for (k, val) in obj { if k.ends_with(".context_length") { if let Some(n) = val.as_u64() { return Some((n, "Ollama (model max)")); } } } }
        }
    }
    None
}

fn split_think(s: &str) -> (Option<String>, String) {
    if let (Some(a), Some(b)) = (s.find("<think>"), s.find("</think>")) {
        if a < b {
            let think = s[a + 7..b].trim().to_string();
            let rest = format!("{}{}", &s[..a], &s[b + 8..]).trim().to_string();
            return (Some(think), rest);
        }
    }
    (None, s.to_string())
}

pub fn truncate_for_log(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_string() } else { format!("{}…", s.chars().take(n).collect::<String>()) }
}
