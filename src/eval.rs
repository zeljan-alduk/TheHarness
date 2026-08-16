//! The fitness function. Each task = a directory with task.toml (+ optional fixture/ and check script).
//! Runs the agent in a fresh git-initialised workdir, then runs the check; exit 0 == pass.

use crate::agent::{Agent, RunStats};
use crate::config::Config;
use crate::llm::Client;
use crate::sandbox;
use crate::tools::{Registry, ToolCtx};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Deserialize)]
pub struct TaskSpec {
    pub name: String,
    pub prompt: String,
    /// Shell command run in the workdir after the agent finishes; exit 0 = pass.
    pub check: String,
    #[serde(default)]
    pub setup: Option<String>,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    #[serde(default)]
    pub max_turns: Option<usize>,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct TaskResult {
    pub name: String,
    pub passed: bool,
    pub check_output: String,
    pub agent_summary: String,
    pub stats: RunStats,
    pub error: Option<String>,
    pub workdir: String,
}

pub fn load_tasks(dir: &Path, filter: Option<&str>) -> Result<Vec<(PathBuf, TaskSpec)>> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).with_context(|| format!("reading tasks dir {}", dir.display()))? {
        let p = e?.path();
        let spec = p.join("task.toml");
        if !spec.is_file() { continue; }
        let t: TaskSpec = toml::from_str(&std::fs::read_to_string(&spec)?).with_context(|| format!("parsing {}", spec.display()))?;
        if let Some(f) = filter { if !t.name.contains(f) && !t.tags.iter().any(|x| x == f) { continue; } }
        out.push((p, t));
    }
    out.sort_by(|a, b| a.1.name.cmp(&b.1.name));
    Ok(out)
}

fn copy_dir(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for e in std::fs::read_dir(src)? {
        let e = e?;
        let to = dst.join(e.file_name());
        if e.file_type()?.is_dir() { copy_dir(&e.path(), &to)?; } else { std::fs::copy(e.path(), &to)?; }
    }
    Ok(())
}

pub async fn run_task(cfg: &Config, client: &Client, task_dir: &Path, spec: &TaskSpec, verbose: bool) -> TaskResult {
    let workdir = PathBuf::from(&cfg.eval.runs_dir).join(&spec.name);
    let _ = std::fs::remove_dir_all(&workdir);
    let mut result = TaskResult {
        name: spec.name.clone(), passed: false, check_output: String::new(), agent_summary: String::new(),
        stats: RunStats::default(), error: None, workdir: workdir.display().to_string(),
    };
    let prep = async {
        std::fs::create_dir_all(&workdir)?;
        let fixture = task_dir.join("fixture");
        if fixture.is_dir() { copy_dir(&fixture, &workdir)?; }
        // check script lives outside the workdir so the agent cannot see/modify it
        let big = Duration::from_secs(cfg.eval.task_timeout_secs);
        let init = "git init -q && git add -A && git -c user.name=harness -c user.email=harness@local commit -q -m 'initial fixture' --allow-empty";
        let o = sandbox::run_shell(init, &workdir, big, 4000).await?;
        if !o.success() { anyhow::bail!("git init failed: {}", o.stderr); }
        if let Some(s) = &spec.setup {
            let o = sandbox::run_shell(s, &workdir, big, 4000).await?;
            if !o.success() { anyhow::bail!("setup failed: {}\n{}", o.stdout, o.stderr); }
        }
        Ok::<_, anyhow::Error>(())
    }.await;
    if let Err(e) = prep { result.error = Some(format!("prep: {e:#}")); return result; }

    let mut ctx = ToolCtx {
        workdir: workdir.canonicalize().unwrap_or(workdir.clone()),
        timeout: Duration::from_secs(cfg.agent.tool_timeout_secs),
        max_output: cfg.agent.max_tool_output_chars,
        net: cfg.net.clone(),
        // evals get a throwaway memory store so the fitness function never depends on (or pollutes) the user's memory
        memory: crate::memory::MemoryStore::scratch(&workdir.join(".harness-memory"), &cfg.memory).ok(),
        subagent: None,
        redact_secrets: cfg.security.redact_secrets, hooks: cfg.hooks.clone(), todos: Default::default(), lsp_servers: cfg.lsp.servers.clone(), extra_roots: vec![], approver: None, inbox: Default::default(), cancel: None, cwd: None, session_id: None,
    };
    let registry = Registry::defaults(cfg.net.enabled);
    let sink: std::sync::Arc<dyn crate::events::Sink> = std::sync::Arc::new(crate::events::StderrSink { verbose });
    let budget = cfg.llm.effective_budget(crate::llm::detect_context_length(&cfg.llm.base_url, &cfg.llm.model).await.map(|d| d.0));
    let policy = std::sync::Arc::new(crate::permissions::Policy::new(crate::permissions::PermissionsConfig { mode: crate::permissions::Mode::Bypass, ..Default::default() }, &ctx.workdir));
    let approver: std::sync::Arc<dyn crate::permissions::Approver> = std::sync::Arc::new(crate::permissions::AutoApprover { yes: true });
    ctx.approver = Some(approver.clone());
    ctx.subagent = Some(std::sync::Arc::new(crate::agent::SubAgentEnv::new(client.clone(), registry.clone(), policy.clone(), approver.clone(), sink.clone(), budget, true)));
    let agent = Agent { client, registry: &registry, ctx: &ctx, max_turns: spec.max_turns.unwrap_or(cfg.agent.max_turns), context_budget: budget, sink: sink.as_ref(), stream: true, policy: &policy, approver: approver.as_ref() };
    let system = crate::agent::system_prompt(&ctx.workdir.display().to_string(), &registry.names(), None);
    let timeout = Duration::from_secs(spec.timeout_secs.unwrap_or(cfg.eval.task_timeout_secs));

    match tokio::time::timeout(timeout, agent.run(&system, &spec.prompt)).await {
        Err(_) => { result.error = Some(format!("task timeout after {}s", timeout.as_secs())); result.stats.stop_reason = "timeout".into(); }
        Ok(Err(e)) => { result.error = Some(format!("agent error: {e:#}")); }
        Ok(Ok((summary, stats))) => { result.agent_summary = summary; result.stats = stats; }
    }

    // Always run the check (partial work may still pass), from the task dir so relative scripts resolve.
    let q = crate::security::shell_quote;
    let check_cmd = format!("TASK_DIR={} WORKDIR={} sh -c {}", q(&task_dir.canonicalize().unwrap_or(task_dir.into()).display().to_string()), q(&ctx.workdir.display().to_string()), q(&spec.check));
    match sandbox::run_shell(&check_cmd, &ctx.workdir, Duration::from_secs(300), 8000).await {
        Ok(o) => { result.passed = o.success(); result.check_output = format!("{}{}", o.stdout, if o.stderr.is_empty() { String::new() } else { format!("\n[stderr]\n{}", o.stderr) }); }
        Err(e) => { result.check_output = format!("check failed to run: {e:#}"); }
    }
    result
}

#[derive(Debug, Serialize)]
pub struct EvalReport {
    pub model: String,
    pub passed: usize,
    pub total: usize,
    pub score: f64,
    pub total_prompt_tokens: u64,
    pub total_completion_tokens: u64,
    pub total_wall_secs: f64,
    pub results: Vec<TaskResult>,
}

pub async fn run_all(cfg: &Config, client: &Client, filter: Option<&str>, verbose: bool) -> Result<EvalReport> {
    let tasks = load_tasks(Path::new(&cfg.eval.tasks_dir), filter)?;
    if tasks.is_empty() { anyhow::bail!("no tasks found in {}", cfg.eval.tasks_dir); }
    let mut results = Vec::new();
    for (i, (dir, spec)) in tasks.iter().enumerate() {
        eprintln!("\n━━━ [{}/{}] {} ━━━", i + 1, tasks.len(), spec.name);
        let r = run_task(cfg, client, dir, spec, verbose).await;
        eprintln!("{} {}  turns={} tools={} tokens={}+{} wall={:.0}s{}",
            if r.passed { "✅ PASS" } else { "❌ FAIL" }, r.name, r.stats.turns, r.stats.tool_calls,
            r.stats.prompt_tokens, r.stats.completion_tokens, r.stats.wall_secs,
            r.error.as_ref().map(|e| format!("  error: {e}")).unwrap_or_default());
        if !r.passed && !r.check_output.trim().is_empty() { eprintln!("   check: {}", crate::llm::truncate_for_log(r.check_output.trim(), 400)); }
        results.push(r);
    }
    let passed = results.iter().filter(|r| r.passed).count();
    let total = results.len();
    Ok(EvalReport {
        model: client.model().to_string(),
        passed, total,
        score: if total == 0 { 0.0 } else { passed as f64 / total as f64 },
        total_prompt_tokens: results.iter().map(|r| r.stats.prompt_tokens).sum(),
        total_completion_tokens: results.iter().map(|r| r.stats.completion_tokens).sum(),
        total_wall_secs: results.iter().map(|r| r.stats.wall_secs).sum(),
        results,
    })
}
