pub mod archive;
pub mod ask_user;
pub mod bash;
pub mod diagnostics;
pub mod download;
pub mod fs;
pub mod image;
pub mod lsp;
pub mod memory;
pub mod notebook;
pub mod patch;
pub mod pdf;
pub mod process;
pub mod search;
pub mod skill;
pub mod subagent;
pub mod todo;
pub mod web;
pub mod worktree;

use crate::llm::ToolDef;
use anyhow::{bail, Result};
use async_trait::async_trait;
use serde_json::Value;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

#[derive(Clone)]
pub struct ToolCtx {
    pub workdir: PathBuf,
    pub timeout: Duration,
    pub max_output: usize,
    pub net: crate::config::NetConfig,
    /// Persistent memory store (None = memory tool disabled, e.g. in evals).
    pub memory: Option<crate::memory::MemoryStore>,
    /// Environment for spawning sub-agents (None = spawn_agent unavailable / nested).
    pub subagent: Option<std::sync::Arc<crate::agent::SubAgentEnv>>,
    pub redact_secrets: bool,
    pub hooks: crate::hooks::HooksConfig,
    /// Shared task list (todo tool) — the UI renders it.
    pub todos: std::sync::Arc<std::sync::Mutex<Vec<todo::TodoItem>>>,
    /// Language servers ([lsp] config; empty = built-in defaults).
    pub lsp_servers: std::collections::HashMap<String, crate::lsp::LspServerConfig>,
    /// Additional directories file tools may access (/add-dir).
    pub extra_roots: Vec<PathBuf>,
    /// Who answers questions / approvals for the model (None = headless: ask_user gets no answer).
    pub approver: Option<std::sync::Arc<dyn crate::permissions::Approver>>,
    /// Asynchronous events for the model (monitor lines, scheduled prompts); drained before each model call.
    pub inbox: std::sync::Arc<crate::inbox::Inbox>,
    /// Cooperative cancellation flag (sub-agents): checked before each model call.
    pub cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    /// Worktree switch (`worktree enter/exit`); None = fixed working directory (enter unavailable).
    pub cwd: Option<crate::worktree::CwdCell>,
}

impl ToolCtx {
    /// A context with defaults (no memory/sub-agents/hooks) — tests, `harness tool`, sub-processes.
    pub fn basic(workdir: PathBuf) -> Self {
        Self { workdir, timeout: Duration::from_secs(120), max_output: 16000, net: crate::config::NetConfig::default(), memory: None, subagent: None, redact_secrets: true, hooks: Default::default(), todos: Default::default(), lsp_servers: Default::default(), extra_roots: vec![], approver: None, inbox: Default::default(), cancel: None, cwd: None }
    }
    /// The context tools actually run in: if the session entered a worktree, workdir is the worktree and
    /// the original tree stays reachable as an extra root.
    pub fn effective(&self) -> ToolCtx {
        let Some(c) = self.cwd.as_ref().and_then(|c| c.lock().unwrap().clone()) else { return self.clone() };
        let mut e = self.clone();
        if c.current.is_dir() { e.workdir = c.current; if !e.extra_roots.contains(&c.original) { e.extra_roots.push(c.original); } }
        e
    }
    /// Resolve a model-supplied path against workdir and refuse escapes.
    /// Symlinks are resolved on the deepest existing ancestor.
    pub fn resolve(&self, p: &str) -> Result<PathBuf> {
        let raw = Path::new(p);
        let joined = if raw.is_absolute() { raw.to_path_buf() } else { self.workdir.join(raw) };
        // lexical normalisation
        let mut norm = PathBuf::new();
        for c in joined.components() {
            match c {
                Component::ParentDir => { norm.pop(); }
                Component::CurDir => {}
                other => norm.push(other.as_os_str()),
            }
        }
        // physical check on the deepest existing ancestor
        let root = self.workdir.canonicalize()?;
        let mut probe = norm.clone();
        while !probe.exists() { if !probe.pop() { break; } }
        let real = probe.canonicalize().unwrap_or(probe);
        if !real.starts_with(&root) && !self.extra_roots.iter().any(|r| real.starts_with(r)) {
            bail!("path escapes workdir: {} (workdir is {}; allow more with /add-dir)", p, root.display());
        }
        Ok(norm)
    }
}

/// What a tool hands back: text for the tool message, plus optional images that the
/// agent loop attaches as a follow-up user message (OpenAI tool results are text-only).
#[derive(Debug, Default)]
pub struct ToolOutput {
    pub text: String,
    /// (mime, base64)
    pub images: Vec<(String, String)>,
}
impl From<String> for ToolOutput { fn from(text: String) -> Self { Self { text, images: vec![] } } }
impl From<&str> for ToolOutput { fn from(text: &str) -> Self { Self { text: text.to_string(), images: vec![] } } }

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn parameters(&self) -> Value;
    /// True if the tool never mutates state; read-only calls in one turn are executed in parallel.
    fn read_only(&self) -> bool { false }
    /// True if several calls in one turn may run concurrently (default: read-only tools only).
    fn parallel_safe(&self) -> bool { self.read_only() }
    async fn call(&self, args: Value, ctx: &ToolCtx) -> Result<ToolOutput>;
}

#[derive(Clone)]
pub struct Registry {
    tools: Vec<std::sync::Arc<dyn Tool>>,
}

impl Registry {
    pub fn defaults(net_enabled: bool) -> Self {
        use std::sync::Arc;
        let mut tools: Vec<Arc<dyn Tool>> = vec![
            Arc::new(bash::Bash),
            Arc::new(fs::ReadFile),
            Arc::new(fs::WriteFile),
            Arc::new(fs::EditFile),
            Arc::new(patch::ApplyPatch),
            Arc::new(fs::ListDir),
            Arc::new(search::Grep),
            Arc::new(search::Glob),
            Arc::new(diagnostics::Diagnostics),
            Arc::new(lsp::Lsp),
            Arc::new(notebook::NotebookEdit),
            Arc::new(image::ViewImage),
            Arc::new(memory::MemoryTool),
            Arc::new(skill::LoadSkill),
            Arc::new(archive::ReadPdf),
            Arc::new(pdf::PdfEdit),
            Arc::new(archive::ExtractArchive),
            Arc::new(subagent::SpawnAgent),
            Arc::new(process::Process),
            Arc::new(todo::Todo),
            Arc::new(ask_user::AskUser),
            Arc::new(worktree::Worktree),
        ];
        if net_enabled {
            tools.push(Arc::new(web::WebFetch));
            tools.push(Arc::new(web::WebSearch));
            tools.push(Arc::new(download::DownloadFile));
        }
        Self { tools }
    }
    /// A copy without the named tool (used for sub-agents).
    pub fn without(&self, name: &str) -> Registry { Registry { tools: self.tools.iter().filter(|t| t.name() != name).cloned().collect() } }
    pub fn is_parallel_safe(&self, name: &str) -> bool { self.tools.iter().find(|t| t.name() == name).map(|t| t.parallel_safe()).unwrap_or(false) }

    /// Add tools at runtime (MCP servers, plugins).
    pub fn extend(&mut self, extra: Vec<std::sync::Arc<dyn Tool>>) { self.tools.extend(extra); }
    pub fn len(&self) -> usize { self.tools.len() }
    pub fn is_empty(&self) -> bool { self.tools.is_empty() }

    pub fn defs(&self) -> Vec<ToolDef> {
        self.tools.iter().map(|t| ToolDef::new(t.name(), t.description(), t.parameters())).collect()
    }

    pub fn names(&self) -> Vec<&'static str> { self.tools.iter().map(|t| t.name()).collect() }
    pub fn is_read_only(&self, name: &str) -> bool { self.tools.iter().find(|t| t.name() == name).map(|t| t.read_only()).unwrap_or(false) }
    pub fn get(&self, name: &str) -> Option<&dyn Tool> { self.tools.iter().find(|t| t.name() == name).map(|b| b.as_ref()) }

    /// Errors are returned as text so the model can recover.
    pub async fn call(&self, name: &str, args_json: &str, ctx: &ToolCtx) -> ToolOutput {
        let Some(tool) = self.tools.iter().find(|t| t.name() == name) else {
            return format!("error: unknown tool '{name}'. Available: {:?}", self.names()).into();
        };
        let args: Value = if args_json.trim().is_empty() {
            Value::Object(Default::default())
        } else {
            match serde_json::from_str(args_json) {
                Ok(v) => v,
                Err(e) => return format!("error: tool arguments are not valid JSON ({e}): {args_json}").into(),
            }
        };
        let mut out = match tool.call(args, ctx).await {
            Ok(s) => s,
            Err(e) => format!("error: {e:#}").into(),
        };
        if ctx.redact_secrets { out.text = crate::security::redact(&out.text); }
        // todo hygiene reminder (like Claude Code's system reminders): appended to a tool result, at most every 6 calls
        if name != "todo" && name != "load_skill" {
            if let Some(r) = todo_reminder(ctx) { out.text.push_str(&format!("\n\n[harness reminder] {r}")); }
        }
        // hooks: post_tool (fire and forget)
        if !ctx.hooks.post_tool.is_empty() { crate::hooks::run_post_tool(&ctx.hooks, name, &out.text, &ctx.workdir).await; }
        out
    }
}

/// Everything a session needs: built-in tools + MCP servers (global, project, plugins). Servers stay
/// alive as long as the returned handles are kept.
pub struct Toolset { pub registry: Registry, pub notes: Vec<String>, pub servers: Vec<std::sync::Arc<tokio::sync::Mutex<crate::mcp::McpServer>>>, pub prompt_extra: String }

pub async fn build_toolset(net_enabled: bool, workdir: &Path, with_mcp: bool) -> Toolset {
    let mut registry = Registry::defaults(net_enabled);
    let mut notes = Vec::new();
    let mut servers = Vec::new();
    let mut prompt_extra = String::new();
    let plugins = crate::plugins::Plugins::open().ok();
    if let Some(p) = &plugins { prompt_extra.push_str(&p.prompt_block()); }
    if with_mcp {
        let extra = plugins.as_ref().map(|p| p.mcp_files()).unwrap_or_default();
        let (tools, errs, srv) = crate::mcp::start_all(workdir, &extra).await;
        if !tools.is_empty() { notes.push(format!("MCP: {} tool(s) from {} server(s)", tools.len(), srv.len())); }
        for e in errs { notes.push(format!("MCP: {e}")); }
        registry.extend(tools);
        servers = srv;
    }
    Toolset { registry, notes, servers, prompt_extra }
}

static TODO_CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// A gentle reminder when the todo list exists but isn't being maintained.
fn todo_reminder(ctx: &ToolCtx) -> Option<String> {
    let todos = ctx.todos.lock().ok()?;
    if todos.is_empty() { return None; }
    let n = TODO_CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if n % 6 != 5 { return None; }
    let open: Vec<&todo::TodoItem> = todos.iter().filter(|t| t.status != "done").collect();
    if open.is_empty() { return Some("all todo items are marked done — if the work continues, add the next steps with todo add/set.".into()); }
    let in_prog = todos.iter().filter(|t| t.status == "in_progress").count();
    if in_prog == 0 { return Some(format!("your todo list has {} open item(s) but none is in_progress — mark the one you are working on with todo start {{id}} (and todo done / todo next as you finish).", open.len())); }
    if in_prog > 1 { return Some("more than one todo is in_progress — keep exactly one; use todo done for finished ones.".into()); }
    Some("keep the todo list current: todo done {id} for finished items, todo next to move on.".into())
}

pub fn arg_str<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    args.get(key).and_then(|v| v.as_str()).ok_or_else(|| anyhow::anyhow!("missing required string argument '{key}'"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ctx() -> ToolCtx {
        let d = std::env::temp_dir().join(format!("harness-test-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        ToolCtx { timeout: Duration::from_secs(5), max_output: 1000, ..ToolCtx::basic(d) }
    }
    #[test]
    fn resolve_rejects_escape() {
        let c = ctx();
        assert!(c.resolve("../../etc/passwd").is_err());
        assert!(c.resolve("/etc/passwd").is_err());
        assert!(c.resolve("sub/../ok.txt").is_ok());
        assert!(c.resolve("new/dir/file.txt").is_ok());
    }
}
