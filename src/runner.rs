//! Shared run bootstrap for every front-end (`harness serve`, the Tauri UI, `harness run`, the TUI):
//! Client · MemoryStore · toolset (built-ins + MCP + plugins) · Policy · SubAgentEnv · ToolCtx · system prompt.
//! Front-ends only supply a `Sink`, an `Approver` and the knobs they expose; everything else is derived from `Config`.

use crate::agent::{Agent, SubAgentEnv};
use crate::config::Config;
use crate::events::Sink;
use crate::llm::Client;
use crate::permissions::{Approver, Mode, Policy};
use crate::tools::{Registry, ToolCtx, Toolset};
use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// Everything a front-end decides; the rest comes from `cfg`. Build with `RunSetup::new(..)` then tweak fields.
pub struct RunSetup {
    pub cfg: Config,
    pub workdir: PathBuf,
    pub sink: Arc<dyn Sink>,
    pub approver: Arc<dyn Approver>,
    /// Extra text appended to the system prompt (after plugin/MCP notes), e.g. an "interactive session" note.
    pub prompt_extra: Option<String>,
    /// Override `cfg.permissions.mode` (TUI `/perm`).
    pub perm_mode: Option<Mode>,
    /// Pre-built toolset (TUI builds it once at startup); None = build now (built-ins + MCP + plugins).
    pub toolset: Option<Arc<Toolset>>,
    /// Known context window (None = probe the server).
    pub context_length: Option<u64>,
    /// Shared state a long-lived UI keeps across turns.
    pub todos: Arc<std::sync::Mutex<Vec<crate::tools::todo::TodoItem>>>,
    pub extra_roots: Vec<PathBuf>,
    pub inbox: Arc<crate::inbox::Inbox>,
    /// Worktree switch cell (None = a fresh cell, so `worktree enter` works within this run).
    pub cwd: Option<crate::worktree::CwdCell>,
    pub session_id: Option<String>,
}

impl RunSetup {
    pub fn new(cfg: Config, workdir: PathBuf, sink: Arc<dyn Sink>, approver: Arc<dyn Approver>) -> Self {
        Self { cfg, workdir, sink, approver, prompt_extra: None, perm_mode: None, toolset: None, context_length: None, todos: Default::default(), extra_roots: vec![], inbox: Default::default(), cwd: None, session_id: None }
    }
}

/// A fully wired run: borrow an `Agent` via `agent()` and drive it (`run`, `run_turn_message`, …).
pub struct Prepared {
    pub client: Client,
    /// External ACP agent (provider = "acp:…"), started on first use and reused across turns.
    pub acp: tokio::sync::Mutex<Option<Arc<crate::acp_client::AcpSession>>>,
    pub toolset: Arc<Toolset>,
    pub store: Option<crate::memory::MemoryStore>,
    pub policy: Arc<Policy>,
    pub approver: Arc<dyn Approver>,
    pub sink: Arc<dyn Sink>,
    pub env: Arc<SubAgentEnv>,
    pub ctx: ToolCtx,
    pub system: String,
    pub budget: u64,
    pub max_turns: usize,
}

impl Prepared {
    pub fn registry(&self) -> &Registry { &self.toolset.registry }
    /// True when another program runs the loop (Claude Code, or an external ACP agent) instead of us.
    pub fn external_backend(&self) -> bool {
        self.client.provider() == crate::llm::Provider::ClaudeCode || self.client.acp_command().is_some()
    }
    pub fn agent(&self) -> Agent<'_> {
        Agent { client: &self.client, registry: &self.toolset.registry, ctx: &self.ctx, max_turns: self.max_turns, context_budget: self.budget, sink: self.sink.as_ref(), stream: true, policy: &self.policy, approver: self.approver.as_ref() }
    }
}

/// Wire up a run (opens memory, starts MCP servers unless a toolset was supplied, probes context length).
pub async fn prepare(setup: RunSetup) -> Result<Prepared> {
    let RunSetup { cfg, workdir, sink, approver, prompt_extra, perm_mode, toolset, context_length, todos, extra_roots, inbox, cwd, session_id } = setup;
    let client = Client::new(&cfg.llm)?;
    let store = if cfg.memory.enabled { crate::memory::MemoryStore::open(&cfg.memory).ok() } else { None };
    if let Some(m) = &store { let _ = m.touch_project(&workdir); }
    let toolset = match toolset { Some(ts) => ts, None => Arc::new(crate::tools::build_toolset(cfg.net.enabled, &workdir, true).await) };
    let ctx_len = match context_length { Some(n) => Some(n), None => crate::llm::detect_context_length(&cfg.llm.base_url, &cfg.llm.model).await.map(|d| d.0) };
    let budget = cfg.llm.effective_budget(ctx_len);
    let mut pcfg = cfg.permissions.clone();
    if let Some(m) = perm_mode { pcfg.mode = m; }
    pcfg.allow.extend(crate::permissions::persisted_rules());
    let policy = Arc::new(Policy::new(pcfg, &workdir));
    // a configured vision model lets text-only backends still look at images
    crate::llm::set_vision_client(cfg.llm.roles.contains_key("vision").then(|| client.role("vision")));
    let mut env = SubAgentEnv::new(client.clone(), toolset.registry.clone(), policy.clone(), approver.clone(), sink.clone(), budget, true);
    env.cc_effort = cfg.llm.effort.clone();
    env.max_depth = cfg.agent.max_subagent_depth.max(1);
    let env = Arc::new(env);
    let ctx = ToolCtx { workdir: workdir.clone(), timeout: Duration::from_secs(cfg.agent.tool_timeout_secs), max_output: cfg.agent.max_tool_output_chars, net: cfg.net.clone(), memory: store.clone(), subagent: Some(env.clone()), redact_secrets: cfg.security.redact_secrets, hooks: cfg.hooks.clone(), todos, lsp_servers: cfg.lsp.servers.clone(), format: cfg.format.clone(), extra_roots, approver: Some(approver.clone()), inbox, cancel: None, cwd: Some(cwd.unwrap_or_else(crate::worktree::new_cell)), session_id };
    let extra = match prompt_extra { Some(e) => format!("{e}{}", toolset.prompt_extra), None => toolset.prompt_extra.clone() };
    let system = crate::agent::system_prompt_with_memory(&workdir.display().to_string(), &toolset.registry.names(), Some(&extra), store.as_ref());
    Ok(Prepared { client, acp: tokio::sync::Mutex::new(None), toolset, store, policy, approver, sink, env, ctx, system, budget, max_turns: cfg.agent.max_turns })
}

impl Prepared {
    /// Run one task to completion on whichever backend is configured (Claude Code drives its own loop
    /// with our tools bridged over MCP; everything else goes through `Agent`). Returns (final text, stats).
    pub async fn run_once(&self, prompt: &str, workdir: &std::path::Path) -> Result<(String, crate::agent::RunStats)> {
        // provider = "acp:<command>": another agent does the work; we are its ACP client
        if let Some(cmd) = self.client.acp_command() {
            let mut slot = self.acp.lock().await;
            if slot.is_none() {
                *slot = Some(crate::acp_client::AcpSession::start(&cmd, workdir, self.policy.clone(), self.approver.clone()).await?);
            }
            let session = slot.clone().unwrap();
            drop(slot);
            return session.run_turn(prompt, self.sink.clone()).await;
        }
        if self.client.provider() == crate::llm::Provider::ClaudeCode {
            let host = Arc::new(crate::mcp_bridge::BridgeHost { registry: self.toolset.registry.clone(), ctx: self.ctx.clone(), policy: self.policy.clone(), approver: self.approver.clone(), sink: self.sink.clone() });
            let session = crate::claude_code::ClaudeCodeSession::start_with(workdir, Some(self.client.model()), self.env.cc_effort.as_deref().or(Some("medium")), &self.system, host, None).await?;
            let r = session.run_turn(prompt, &[], self.sink.as_ref()).await;
            session.stop().await;
            r
        } else {
            self.agent().run(&self.system, prompt).await
        }
    }
}

/// One-shot: prepare and run `prompt` to completion, returning the final answer text.
pub async fn start_run(setup: RunSetup, prompt: String) -> Result<String> {
    let p = prepare(setup).await?;
    let wd = p.ctx.workdir.clone();
    p.run_once(&prompt, &wd).await.map(|(t, _)| t)
}
