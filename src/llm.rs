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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Provider { OpenAi, Anthropic, ClaudeCode }

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    base_url: String,
    model: String,
    api_key: Option<String>,
    temperature: f32,
    max_tokens: u32,
    aux_model: Option<String>,
    provider: Provider,
    thinking_budget: Option<u32>,
    /// Tool-calling mode for servers/models without native function calling (see `shim`).
    shim: std::sync::Arc<std::sync::atomic::AtomicU8>,
}

/// Tool shim state: 0 = auto (native until the server complains), 1 = shim on, 2 = native only.
const SHIM_AUTO: u8 = 0;
const SHIM_ON: u8 = 1;
const SHIM_OFF: u8 = 2;

impl Client {
    pub fn new(cfg: &crate::config::LlmConfig) -> Result<Self> {
        // No total timeout: a long reasoning turn can legitimately take many minutes on a local model.
        // Instead: bounded connect time and a read (stall) timeout between streamed chunks.
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(30))
            .read_timeout(std::time::Duration::from_secs(300))
            .build()?;
        let provider = match cfg.provider.as_deref() { Some("anthropic") => Provider::Anthropic, Some("claude-code") | Some("claude_code") | Some("claude") => Provider::ClaudeCode, Some(_) => Provider::OpenAi, None => if cfg.base_url.contains("anthropic.com") { Provider::Anthropic } else { Provider::OpenAi } };
        let api_key = cfg.api_key.clone().or_else(|| match provider { Provider::Anthropic => std::env::var("ANTHROPIC_API_KEY").ok(), Provider::OpenAi => std::env::var("OPENAI_API_KEY").ok(), Provider::ClaudeCode => None });
        Ok(Self {
            http,
            base_url: cfg.base_url.trim_end_matches('/').to_string(),
            model: cfg.model.clone(),
            api_key,
            temperature: cfg.temperature,
            max_tokens: cfg.max_tokens,
            aux_model: cfg.aux_model.clone().filter(|m| !m.trim().is_empty()),
            provider,
            thinking_budget: cfg.thinking_budget,
            shim: std::sync::Arc::new(std::sync::atomic::AtomicU8::new(match cfg.tool_shim.as_deref() { Some("on") | Some("always") | Some("true") => SHIM_ON, Some("off") | Some("never") | Some("false") => SHIM_OFF, _ => SHIM_AUTO })),
        })
    }
    pub fn provider(&self) -> Provider { self.provider }
    /// True when a separate (usually smaller/faster) aux model is configured.
    pub fn has_aux(&self) -> bool { self.aux_model.is_some() }
    /// Client for auxiliary calls (reflection, compaction): the configured aux model, or this one.
    pub fn aux(&self) -> Client { match &self.aux_model { Some(m) => self.with_model(m), None => self.clone() } }

    pub fn model(&self) -> &str { &self.model }
    /// Same server/settings, different model (auxiliary calls).
    pub fn with_model(&self, model: &str) -> Client { let mut c = self.clone(); c.model = model.to_string(); c }

    /// True when tool calls must be emitted as text (`<tool_call>{…}</tool_call>`) instead of the
    /// server's function-calling API.
    pub fn shim_active(&self) -> bool { self.shim.load(std::sync::atomic::Ordering::Relaxed) == SHIM_ON }
    fn shim_allowed(&self) -> bool { self.shim.load(std::sync::atomic::Ordering::Relaxed) != SHIM_OFF }
    /// Turn the shim on after a server rejected a request with tools (auto mode). Returns false if pinned off.
    fn enable_shim(&self) -> bool {
        if !self.shim_allowed() { return false; }
        self.shim.store(SHIM_ON, std::sync::atomic::Ordering::Relaxed);
        true
    }

    /// Turn `<tool_call>` text into real tool calls. Runs in shim mode, and also when a model that
    /// *was* offered native tools writes them as text anyway (common with small local models) — in
    /// that case the shim is latched on for the rest of the session.
    fn absorb_text_calls(&self, msg: &mut Message, had_tools: bool) {
        if msg.tool_calls.as_ref().map(|c| !c.is_empty()).unwrap_or(false) || !had_tools { return; }
        let text = msg.text();
        if !text.contains("<tool_call") && !text.contains("<function_call") && !text.contains("<tool_use") && !text.trim_start().starts_with('{') && !text.trim_start().starts_with("```") { return; }
        let (rest, calls) = parse_shim_calls(&text);
        if calls.is_empty() { return; }
        if !self.shim_active() && self.shim_allowed() { self.shim.store(SHIM_ON, std::sync::atomic::Ordering::Relaxed); }
        msg.content = if rest.trim().is_empty() { None } else { Some(Content::Text(rest)) };
        msg.tool_calls = Some(calls);
    }

    pub async fn list_models(&self) -> Result<Vec<String>> {
        if self.provider == Provider::ClaudeCode { return Ok(vec!["sonnet".into(), "opus".into(), "haiku".into(), "claude-sonnet-5".into(), "claude-opus-5".into()]); }
        if self.provider == Provider::Anthropic {
            let mut req = self.http.get(format!("{}/v1/models?limit=100", self.base_url.trim_end_matches("/v1"))).header("anthropic-version", "2023-06-01");
            if let Some(k) = &self.api_key { req = req.header("x-api-key", k); }
            let v: Value = req.send().await?.error_for_status()?.json().await?;
            return Ok(v["data"].as_array().map(|a| a.iter().filter_map(|m| m["id"].as_str().map(String::from)).collect()).unwrap_or_default());
        }
        let mut req = self.http.get(format!("{}/models", self.base_url));
        if let Some(k) = &self.api_key { req = req.bearer_auth(k); }
        let v: Value = req.send().await?.error_for_status()?.json().await?;
        Ok(v["data"].as_array().map(|a| a.iter().filter_map(|m| m["id"].as_str().map(String::from)).collect()).unwrap_or_default())
    }

    pub async fn chat(&self, messages: &[Message], tools: &[ToolDef]) -> Result<(Message, Usage)> {
        if self.provider == Provider::ClaudeCode { bail!("provider claude-code: model calls go through the claude CLI session, not the HTTP client"); }
        if self.provider == Provider::Anthropic { return self.chat_stream(messages, tools, |_| {}).await; }
        let shim = self.shim_active();
        let shimmed;
        let msgs: &[Message] = if shim { shimmed = shim_messages(messages, tools); &shimmed } else { messages };
        let mut body = json!({
            "model": self.model,
            "messages": msgs,
            "temperature": self.temperature,
            "max_tokens": self.max_tokens,
        });
        if !tools.is_empty() && !shim {
            body["tools"] = json!(tools);
            body["tool_choice"] = json!("auto");
        }
        let mut req = self.http.post(format!("{}/chat/completions", self.base_url)).json(&body);
        if let Some(k) = &self.api_key { req = req.bearer_auth(k); }
        let resp = req.send().await.context("LLM request failed (is the server running?)")?;
        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            // the server has no function-calling API: switch to the text protocol and retry once
            if !shim && !tools.is_empty() && error_is_about_tools(&text) && self.enable_shim() {
                return Box::pin(self.chat(messages, tools)).await;
            }
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
        self.absorb_text_calls(&mut msg, !tools.is_empty());
        Ok((msg, r.usage.unwrap_or_default()))
    }
}

impl Client {
    /// Same as `chat` but streams (SSE). `on_delta` is called for each reasoning/content increment;
    /// tool-call fragments are assembled internally and returned in the final message.
    pub async fn chat_stream(&self, messages: &[Message], tools: &[ToolDef], mut on_delta: impl FnMut(Delta)) -> Result<(Message, Usage)> {
        if self.provider == Provider::ClaudeCode { bail!("provider claude-code: model calls go through the claude CLI session, not the HTTP client"); }
        if self.provider == Provider::Anthropic { return self.anthropic_stream(messages, tools, &mut on_delta).await; }
        let shim = self.shim_active();
        let shimmed;
        let msgs: &[Message] = if shim { shimmed = shim_messages(messages, tools); &shimmed } else { messages };
        let mut body = json!({
            "model": self.model,
            "messages": msgs,
            "temperature": self.temperature,
            "max_tokens": self.max_tokens,
            "stream": true,
            "stream_options": {"include_usage": true},
        });
        if !tools.is_empty() && !shim { body["tools"] = json!(tools); body["tool_choice"] = json!("auto"); }
        let mut req = self.http.post(format!("{}/chat/completions", self.base_url)).json(&body);
        if let Some(k) = &self.api_key { req = req.bearer_auth(k); }
        let resp = req.send().await.context("LLM request failed (is the server running?)")?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            if !shim && !tools.is_empty() && error_is_about_tools(&text) && self.enable_shim() {
                return Box::pin(self.chat_stream(messages, tools, on_delta)).await;
            }
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
        let mut msg = Message {
            role: "assistant".into(),
            content: if content.is_empty() && !calls.is_empty() { None } else { Some(Content::Text(content)) },
            tool_calls: if calls.is_empty() { None } else { Some(calls) },
            reasoning_content: if reasoning.is_empty() { None } else { Some(reasoning) },
            ..Default::default()
        };
        self.absorb_text_calls(&mut msg, !tools.is_empty());
        Ok((msg, usage))
    }
}

/// Detect the loaded context length of `model` from the server. Tries LM Studio (`/api/v0/models`),
/// llama.cpp server (`/props`), Ollama (`/api/show`). Returns (tokens, source).
pub async fn detect_context_length(base_url: &str, model: &str) -> Option<(u64, &'static str)> {
    if base_url.contains("anthropic.com") || model.starts_with("claude-") || matches!(model, "sonnet" | "opus" | "haiku") { return Some((200_000, "Anthropic (known)")); }
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

impl Client {
    /// Anthropic Messages API (streaming). Converts our OpenAI-shaped transcript to Anthropic blocks and back.
    async fn anthropic_stream(&self, messages: &[Message], tools: &[ToolDef], on_delta: &mut impl FnMut(Delta)) -> Result<(Message, Usage)> {
        let (system, msgs) = to_anthropic_messages(messages);
        let a_tools: Vec<Value> = tools.iter().map(|t| json!({"name": t.function.name, "description": t.function.description, "input_schema": t.function.parameters})).collect();
        let mut body = json!({"model": self.model, "max_tokens": self.max_tokens, "messages": msgs, "stream": true, "temperature": self.temperature});
        if let Some(b) = self.thinking_budget { if b > 0 { body["thinking"] = json!({"type": "enabled", "budget_tokens": b.max(1024)}); body.as_object_mut().unwrap().remove("temperature"); if self.max_tokens <= b { body["max_tokens"] = json!(b + 4096); } } }
        if !system.is_empty() { body["system"] = json!(system); }
        if !a_tools.is_empty() { body["tools"] = json!(a_tools); }
        let url = format!("{}/v1/messages", self.base_url.trim_end_matches("/v1"));
        let mut req = self.http.post(&url).header("anthropic-version", "2023-06-01").header("content-type", "application/json").json(&body);
        if let Some(k) = &self.api_key { req = req.header("x-api-key", k); } else { bail!("Anthropic provider needs an API key (llm.api_key or ANTHROPIC_API_KEY)"); }
        let resp = req.send().await.context("Anthropic request failed")?;
        let status = resp.status();
        if !status.is_success() { let t = resp.text().await.unwrap_or_default(); bail!("Anthropic returned {status}: {}", truncate_for_log(&t, 600)); }
        let mut stream = resp.bytes_stream();
        let mut buf = String::new();
        let mut content = String::new(); let mut reasoning = String::new();
        let mut calls: Vec<ToolCall> = Vec::new(); let mut cur_json: Vec<String> = Vec::new(); // per tool_use block index → partial json
        let mut block_kind: std::collections::HashMap<u64, String> = std::collections::HashMap::new();
        let mut usage = Usage::default(); let mut stop: Option<String> = None;
        'outer: while let Some(chunk) = stream.next().await {
            buf.push_str(&String::from_utf8_lossy(&chunk.context("stream error")?));
            while let Some(pos) = buf.find('\n') {
                let line = buf[..pos].trim().to_string(); buf.drain(..=pos);
                let Some(data) = line.strip_prefix("data:") else { continue };
                let Ok(v) = serde_json::from_str::<Value>(data.trim()) else { continue };
                match v["type"].as_str().unwrap_or("") {
                    "message_start" => { if let Some(u) = v["message"]["usage"].as_object() { usage.prompt_tokens = u.get("input_tokens").and_then(|x| x.as_u64()).unwrap_or(0) + u.get("cache_read_input_tokens").and_then(|x| x.as_u64()).unwrap_or(0) + u.get("cache_creation_input_tokens").and_then(|x| x.as_u64()).unwrap_or(0); } }
                    "content_block_start" => {
                        let idx = v["index"].as_u64().unwrap_or(0); let cb = &v["content_block"];
                        let kind = cb["type"].as_str().unwrap_or("").to_string();
                        if kind == "tool_use" { calls.push(ToolCall { id: cb["id"].as_str().unwrap_or("").to_string(), kind: "function".into(), function: FunctionCall { name: cb["name"].as_str().unwrap_or("").to_string(), arguments: String::new() } }); cur_json.push(String::new()); }
                        block_kind.insert(idx, kind);
                    }
                    "content_block_delta" => {
                        let d = &v["delta"];
                        match d["type"].as_str().unwrap_or("") {
                            "text_delta" => { let t = d["text"].as_str().unwrap_or(""); content.push_str(t); on_delta(Delta::Content(t.to_string())); }
                            "thinking_delta" => { let t = d["thinking"].as_str().unwrap_or(""); reasoning.push_str(t); on_delta(Delta::Reasoning(t.to_string())); }
                            "input_json_delta" => { if let Some(last) = cur_json.last_mut() { last.push_str(d["partial_json"].as_str().unwrap_or("")); } }
                            _ => {}
                        }
                    }
                    "content_block_stop" => {}
                    "message_delta" => { if let Some(sr) = v["delta"]["stop_reason"].as_str() { stop = Some(sr.to_string()); } if let Some(o) = v["usage"]["output_tokens"].as_u64() { usage.completion_tokens = o; } }
                    "message_stop" => break 'outer,
                    "error" => bail!("Anthropic stream error: {}", v["error"]),
                    _ => {}
                }
            }
        }
        for (c, j) in calls.iter_mut().zip(cur_json) { c.function.arguments = if j.trim().is_empty() { "{}".into() } else { j }; }
        usage.total_tokens = usage.prompt_tokens + usage.completion_tokens;
        if stop.as_deref() == Some("max_tokens") { content.push_str("\n[output truncated by max_tokens]"); }
        Ok((Message { role: "assistant".into(), content: if content.is_empty() && !calls.is_empty() { None } else { Some(Content::Text(content)) }, tool_calls: if calls.is_empty() { None } else { Some(calls) }, reasoning_content: if reasoning.is_empty() { None } else { Some(reasoning) }, ..Default::default() }, usage))
    }
}

/// OpenAI-shaped transcript → (system, anthropic messages). Consecutive tool results are merged into one user turn.
fn to_anthropic_messages(messages: &[Message]) -> (String, Vec<Value>) {
    let mut system = String::new();
    let mut out: Vec<Value> = Vec::new();
    for m in messages {
        match m.role.as_str() {
            "system" => { if !system.is_empty() { system.push_str("\n\n"); } system.push_str(&m.text()); }
            "user" => {
                let blocks: Vec<Value> = match &m.content {
                    Some(Content::Parts(parts)) => parts.iter().filter_map(|p| match p["type"].as_str() {
                        Some("text") => Some(json!({"type": "text", "text": p["text"]})),
                        Some("image_url") => { let url = p["image_url"]["url"].as_str().unwrap_or(""); if let Some(rest) = url.strip_prefix("data:") { let (mime, b64) = rest.split_once(";base64,").unwrap_or(("image/png", "")); Some(json!({"type": "image", "source": {"type": "base64", "media_type": mime, "data": b64}})) } else { Some(json!({"type": "image", "source": {"type": "url", "url": url}})) } }
                        _ => None }).collect(),
                    _ => vec![json!({"type": "text", "text": m.text()})],
                };
                out.push(json!({"role": "user", "content": blocks}));
            }
            "assistant" => {
                let mut blocks: Vec<Value> = Vec::new();
                let t = m.text(); if !t.trim().is_empty() { blocks.push(json!({"type": "text", "text": t})); }
                if let Some(calls) = &m.tool_calls { for c in calls { let input: Value = serde_json::from_str(&c.function.arguments).unwrap_or(json!({})); blocks.push(json!({"type": "tool_use", "id": c.id, "name": c.function.name, "input": input})); } }
                if blocks.is_empty() { blocks.push(json!({"type": "text", "text": "(empty)"})); }
                out.push(json!({"role": "assistant", "content": blocks}));
            }
            "tool" => {
                let block = json!({"type": "tool_result", "tool_use_id": m.tool_call_id.clone().unwrap_or_default(), "content": m.text()});
                if let Some(last) = out.last_mut() { if last["role"] == "user" && last["content"].as_array().map(|a| a.iter().all(|b| b["type"] == "tool_result")).unwrap_or(false) { last["content"].as_array_mut().unwrap().push(block); continue; } }
                out.push(json!({"role": "user", "content": [block]}));
            }
            _ => {}
        }
    }
    (system, out)
}

// ───────────────────────── tool shim (models without native function calling) ─────────────────────────

/// The protocol description + tool catalogue appended to the system prompt when the shim is active.
/// Deliberately in the Qwen/Hermes `<tool_call>` shape: most local instruct models have seen it.
pub fn shim_prompt(tools: &[ToolDef]) -> String {
    let mut s = String::from("\n\n# Tool calling\nThis server has no function-calling API, so you call tools by WRITING them in your reply.\nTo call a tool, emit one or more blocks, each on its own lines and nothing else in them:\n<tool_call>\n{\"name\": \"<tool name>\", \"arguments\": {<json arguments>}}\n</tool_call>\nRules: valid JSON inside the block; no comments; one JSON object per block; several blocks = several calls in one turn. Stop after emitting the blocks — the results come back as <tool_response> messages. When you are done and need no tool, reply with plain text and no blocks.\n\nAvailable tools:\n");
    for t in tools {
        let desc = truncate_for_log(&t.function.description, 400);
        let params = serde_json::to_string(&t.function.parameters).unwrap_or_else(|_| "{}".into());
        s.push_str(&format!("\n## {}\n{}\nparameters (JSON Schema): {}\n", t.function.name, desc, truncate_for_log(&params, 1200)));
    }
    s
}

/// Rewrite a conversation for a server that knows nothing about tools: the tool catalogue goes into
/// the system message, past tool calls become `<tool_call>` text and tool results become user
/// `<tool_response>` messages.
pub fn shim_messages(messages: &[Message], tools: &[ToolDef]) -> Vec<Message> {
    let mut out: Vec<Message> = Vec::with_capacity(messages.len());
    let mut injected = false;
    for m in messages {
        match m.role.as_str() {
            "system" if !injected => {
                injected = true;
                out.push(Message::system(format!("{}{}", m.text(), if tools.is_empty() { String::new() } else { shim_prompt(tools) })));
            }
            "assistant" => {
                let mut text = m.text();
                if let Some(calls) = &m.tool_calls {
                    for c in calls {
                        let args: Value = serde_json::from_str(&c.function.arguments).unwrap_or(Value::Object(Default::default()));
                        text.push_str(&format!("\n<tool_call>\n{}\n</tool_call>", json!({"name": c.function.name, "arguments": args})));
                    }
                }
                out.push(Message { role: "assistant".into(), content: Some(Content::Text(text)), ..Default::default() });
            }
            "tool" => {
                let name = m.name.clone().unwrap_or_default();
                out.push(Message::user(format!("<tool_response name=\"{name}\">\n{}\n</tool_response>", m.text())));
            }
            _ => out.push(m.clone()),
        }
    }
    if !injected && !tools.is_empty() { out.insert(0, Message::system(shim_prompt(tools).trim_start().to_string())); }
    out
}

/// Pull `<tool_call>` blocks out of a model reply. Returns (remaining text, calls).
/// Accepts `<tool_call>`, `<function_call>` and ```json fences, plus a bare top-level
/// `{"name": ..., "arguments": ...}` reply — everything local models are observed to produce.
pub fn parse_shim_calls(text: &str) -> (String, Vec<ToolCall>) {
    let mut calls: Vec<ToolCall> = Vec::new();
    let mut rest = String::new();
    let open_tags = ["<tool_call>", "<function_call>", "<tool_use>"];
    let mut tail = text.to_string();
    loop {
        let hit = open_tags.iter().filter_map(|t| tail.find(t).map(|p| (p, *t))).min_by_key(|(p, _)| *p);
        let Some((pos, tag)) = hit else { rest.push_str(&tail); break };
        rest.push_str(&tail[..pos]);
        let after = tail[pos + tag.len()..].to_string();
        let close = tag.replace('<', "</");
        let (body, next) = match after.find(&close) {
            Some(e) => (after[..e].to_string(), after[e + close.len()..].to_string()),
            None => (after.clone(), String::new()),
        };
        if let Some(c) = call_from_json(&body) { calls.push(c); }
        tail = next;
        if tail.is_empty() { break; }
    }
    if calls.is_empty() {
        // fenced or bare JSON object with name/arguments
        if let Some(c) = call_from_json(text) { return (String::new(), vec![c]); }
        return (text.to_string(), calls);
    }
    for (i, c) in calls.iter_mut().enumerate() { if c.id.is_empty() { c.id = format!("shim_{i}"); } }
    (rest.trim().to_string(), calls)
}

fn call_from_json(body: &str) -> Option<ToolCall> {
    let t = body.trim();
    let t = t.strip_prefix("```json").or_else(|| t.strip_prefix("```")).map(|x| x.trim_end_matches("```").trim()).unwrap_or(t).trim();
    let v: Value = serde_json::from_str(t).ok()?;
    let name = v.get("name").or_else(|| v.get("tool")).or_else(|| v.get("function"))?.as_str()?.to_string();
    if name.is_empty() { return None; }
    let args = v.get("arguments").or_else(|| v.get("parameters")).or_else(|| v.get("args")).cloned().unwrap_or(Value::Object(Default::default()));
    let arguments = match args { Value::String(s) => s, other => other.to_string() };
    Some(ToolCall { id: String::new(), kind: "function".into(), function: FunctionCall { name, arguments } })
}

/// True when a server error means "this model/endpoint does not do tools".
fn error_is_about_tools(body: &str) -> bool {
    let b = body.to_lowercase();
    (b.contains("tool") || b.contains("function")) && (b.contains("not support") || b.contains("unsupported") || b.contains("unknown field") || b.contains("unrecognized") || b.contains("does not") || b.contains("no function"))
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn anthropic_conversion() {
        let msgs = vec![
            Message::system("sys"),
            Message::user("hi"),
            Message { role: "assistant".into(), content: None, tool_calls: Some(vec![ToolCall { id: "t1".into(), kind: "function".into(), function: FunctionCall { name: "bash".into(), arguments: "{\"cmd\":\"ls\"}".into() } }]), ..Default::default() },
            Message::tool("t1", "bash", "out"),
            Message::tool("t2", "bash", "out2"),
        ];
        let (system, out) = to_anthropic_messages(&msgs);
        assert_eq!(system, "sys");
        assert_eq!(out.len(), 3);
        assert_eq!(out[1]["content"][0]["type"], "tool_use");
        assert_eq!(out[1]["content"][0]["input"]["cmd"], "ls");
        assert_eq!(out[2]["content"].as_array().unwrap().len(), 2, "consecutive tool results merge into one user turn");
        assert_eq!(out[2]["content"][1]["tool_use_id"], "t2");
    }
    #[test]
    fn shim_roundtrip() {
        let tools = vec![ToolDef::new("read_file", "Read a file", json!({"type":"object","properties":{"path":{"type":"string"}}}))];
        let msgs = vec![
            Message::system("You are an agent."),
            Message::user("read a.txt"),
            Message { role: "assistant".into(), content: None, tool_calls: Some(vec![ToolCall { id: "c1".into(), kind: "function".into(), function: FunctionCall { name: "read_file".into(), arguments: "{\"path\":\"a.txt\"}".into() } }]), ..Default::default() },
            Message::tool("c1", "read_file", "hello"),
        ];
        let out = shim_messages(&msgs, &tools);
        assert!(out[0].text().contains("<tool_call>"), "protocol goes into the system prompt");
        assert!(out[0].text().contains("read_file"), "tool catalogue too");
        assert!(out[2].text().contains("\"name\":\"read_file\""), "past calls become text: {}", out[2].text());
        assert!(out[3].role == "user" && out[3].text().contains("<tool_response"), "tool results become user messages");
        assert!(out.iter().all(|m| m.tool_calls.is_none() && m.role != "tool"));

        let (rest, calls) = parse_shim_calls("Sure.\n<tool_call>\n{\"name\": \"read_file\", \"arguments\": {\"path\": \"a.txt\"}}\n</tool_call>\n<tool_call>{\"name\":\"grep\",\"arguments\":{\"pattern\":\"x\"}}</tool_call>");
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].function.name, "read_file");
        assert_eq!(calls[0].function.arguments, "{\"path\":\"a.txt\"}");
        assert_eq!(calls[1].function.name, "grep");
        assert_eq!(rest, "Sure.");

        let (_, bare) = parse_shim_calls("```json\n{\"name\": \"glob\", \"parameters\": {\"pattern\": \"*.rs\"}}\n```");
        assert_eq!(bare.len(), 1); assert_eq!(bare[0].function.name, "glob");
        let (text, none) = parse_shim_calls("just a normal answer");
        assert!(none.is_empty()); assert_eq!(text, "just a normal answer");
        assert!(error_is_about_tools("{\"error\":\"this model does not support tools\"}"));
        assert!(!error_is_about_tools("context length exceeded"));
    }
    #[test]
    fn think_split() { let (t, r) = split_think("<think>abc</think>hello"); assert_eq!(t.as_deref(), Some("abc")); assert_eq!(r, "hello"); }
}
