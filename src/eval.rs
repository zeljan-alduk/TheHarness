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

/// What happened to one task during an import.
#[derive(Debug, Clone, Serialize)]
pub struct Imported { pub name: String, pub ok: bool, pub note: String }

/// Import Terminal-Bench / Harbor style tasks into our `evals/tasks` format.
///
/// A Harbor task is a directory with `task.yaml` (the instruction), `tests/` (what decides pass/fail),
/// usually an `environment/` with a Dockerfile, and a reference `solution/`. We keep the instruction as
/// the prompt, turn the tests into our `check` command, and copy everything that is not scaffolding
/// into `fixture/`. Tasks whose environment is a container are skipped unless `include_docker` is set,
/// because their checks cannot pass on a bare workdir — being honest about that beats a fake red suite.
pub fn import_harbor(src: &Path, dest: &Path, include_docker: bool, limit: usize) -> Result<Vec<Imported>> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if src.join("task.yaml").is_file() || src.join("task.toml").is_file() { roots.push(src.to_path_buf()); }
    else {
        for e in std::fs::read_dir(src).with_context(|| format!("reading {}", src.display()))? {
            let p = e?.path();
            if p.is_dir() && (p.join("task.yaml").is_file() || p.join("task.toml").is_file()) { roots.push(p); }
        }
    }
    roots.sort();
    let mut out = Vec::new();
    for root in roots.into_iter().take(limit.max(1)) {
        let name = root.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "task".into());
        match import_one(&root, dest, &name, include_docker) {
            Ok(note) => out.push(Imported { name, ok: true, note }),
            Err(e) => out.push(Imported { name, ok: false, note: format!("{e:#}") }),
        }
    }
    Ok(out)
}

fn yaml_str(v: &serde_yaml::Value, keys: &[&str]) -> Option<String> {
    for k in keys {
        if let Some(s) = v.get(*k).and_then(|x| x.as_str()) { if !s.trim().is_empty() { return Some(s.trim().to_string()); } }
    }
    None
}

fn import_one(root: &Path, dest: &Path, name: &str, include_docker: bool) -> Result<String> {
    let spec_path = if root.join("task.yaml").is_file() { root.join("task.yaml") } else { root.join("task.toml") };
    let text = std::fs::read_to_string(&spec_path)?;
    let doc: serde_yaml::Value = serde_yaml::from_str(&text).with_context(|| format!("parsing {}", spec_path.display()))?;
    let prompt = yaml_str(&doc, &["instruction", "prompt", "description", "task"]).context("no instruction/prompt in the task file")?;
    let dockerish = ["environment/Dockerfile", "Dockerfile", "docker-compose.yaml", "environment/docker-compose.yaml"].iter().any(|f| root.join(f).exists());
    if dockerish && !include_docker { anyhow::bail!("needs a container environment (pass --include-docker to import it anyway)"); }

    // what decides pass/fail
    let check = if root.join("run-tests.sh").is_file() { "bash run-tests.sh".to_string() }
        else if root.join("tests/run-tests.sh").is_file() { "bash tests/run-tests.sh".to_string() }
        else if root.join("tests").is_dir() {
            let py = std::fs::read_dir(root.join("tests")).map(|rd| rd.flatten().any(|e| e.path().extension().map(|x| x == "py").unwrap_or(false))).unwrap_or(false);
            if py { "python3 -m pytest -q tests".to_string() } else { "bash -c 'for t in tests/*.sh; do bash \"$t\" || exit 1; done'".to_string() }
        } else { anyhow::bail!("no tests/ or run-tests.sh — nothing would decide pass/fail") };

    let timeout = doc.get("max_agent_timeout_sec").or_else(|| doc.get("timeout_sec")).and_then(|v| v.as_u64());
    let mut tags: Vec<String> = doc.get("tags").and_then(|v| v.as_sequence()).map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect()).unwrap_or_default();
    tags.push("imported".into());
    if dockerish { tags.push("needs-docker".into()); }
    if let Some(d) = yaml_str(&doc, &["difficulty"]) { tags.push(d); }

    // copy everything that is not scaffolding into fixture/ (the tests come along: the check runs them)
    let out_dir = dest.join(name);
    std::fs::create_dir_all(&out_dir)?;
    let fixture = out_dir.join("fixture");
    let _ = std::fs::remove_dir_all(&fixture);
    std::fs::create_dir_all(&fixture)?;
    let mut copied = 0usize;
    for e in std::fs::read_dir(root)? {
        let p = e?.path();
        let base = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if matches!(base.as_str(), "task.yaml" | "task.toml" | "solution" | "solution.sh" | "environment" | ".git") { continue; }
        let to = fixture.join(&base);
        if p.is_dir() { copy_dir(&p, &to)?; } else { std::fs::copy(&p, &to)?; }
        copied += 1;
    }
    // some layouts keep the working files under environment/ next to the Dockerfile
    if include_docker && root.join("environment").is_dir() {
        for e in std::fs::read_dir(root.join("environment"))? {
            let p = e?.path();
            let base = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            if base == "Dockerfile" || base.starts_with("docker-compose") { continue; }
            let to = fixture.join(&base);
            if p.is_dir() { copy_dir(&p, &to)?; } else { std::fs::copy(&p, &to)?; }
            copied += 1;
        }
    }

    let spec = format!(
"name = {name}\nprompt = {prompt}\ncheck = {check}\ntags = {tags}\n{timeout}# imported from {src} by `harness eval-import` — review the check before trusting the score\n",
        name = toml_str(name), prompt = toml_str(&prompt), check = toml_str(&check),
        tags = toml::Value::Array(tags.into_iter().map(toml::Value::String).collect()),
        timeout = timeout.map(|t| format!("timeout_secs = {t}\n")).unwrap_or_default(),
        src = root.display());
    std::fs::write(out_dir.join("task.toml"), spec)?;
    Ok(format!("{copied} fixture entr{} · check: {check}", if copied == 1 { "y" } else { "ies" }))
}

fn toml_str(s: &str) -> String { toml::Value::String(s.to_string()).to_string() }

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
        redact_secrets: cfg.security.redact_secrets, injection_scan: cfg.security.injection_scan, hooks: cfg.hooks.clone(), todos: Default::default(), lsp_servers: cfg.lsp.servers.clone(), format: cfg.format.clone(), extra_roots: vec![], approver: None, inbox: Default::default(), cancel: None, cwd: None, session_id: None,
    };
    let registry = Registry::defaults(cfg.net.enabled);
    let sink: std::sync::Arc<dyn crate::events::Sink> = std::sync::Arc::new(crate::events::StderrSink { verbose });
    let budget = cfg.llm.effective_budget(crate::llm::detect_context_length(&cfg.llm.base_url, &cfg.llm.model).await.map(|d| d.0));
    let policy = std::sync::Arc::new(crate::permissions::Policy::new(crate::permissions::PermissionsConfig { mode: crate::permissions::Mode::Bypass, ..Default::default() }, &ctx.workdir));
    let approver: std::sync::Arc<dyn crate::permissions::Approver> = std::sync::Arc::new(crate::permissions::AutoApprover { yes: true });
    ctx.approver = Some(approver.clone());
    ctx.subagent = Some(std::sync::Arc::new(crate::agent::SubAgentEnv::new(client.clone(), registry.clone(), policy.clone(), approver.clone(), sink.clone(), budget, true)));
    let agent = Agent { client, registry: &registry, ctx: &ctx, max_turns: spec.max_turns.unwrap_or(cfg.agent.max_turns), context_budget: budget, sink: sink.as_ref(), stream: true, policy: &policy, tool_history_keep: cfg.agent.tool_history_keep, tool_history_chars: cfg.agent.tool_history_max_chars, approver: approver.as_ref() };
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

#[cfg(test)]
mod import_tests {
    use super::*;

    fn write(p: &Path, body: &str) { std::fs::create_dir_all(p.parent().unwrap()).unwrap(); std::fs::write(p, body).unwrap(); }

    #[test]
    fn imports_harbor_tasks() {
        let d = std::env::temp_dir().join(format!("harness-tbimport-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let (src, dest) = (d.join("bench"), d.join("out"));
        // a plain task: instruction + shell tests + a workspace file
        write(&src.join("fix-parser/task.yaml"), "instruction: |\n  Fix the parser so the tests pass.\ntags: [parsing, hard]\nmax_agent_timeout_sec: 600\ndifficulty: hard\n");
        write(&src.join("fix-parser/run-tests.sh"), "#!/bin/sh\nexit 0\n");
        write(&src.join("fix-parser/src/parser.py"), "def parse(s): return None\n");
        write(&src.join("fix-parser/solution/solution.sh"), "echo cheat\n");
        // a containerised task: skipped by default
        write(&src.join("needs-image/task.yaml"), "instruction: Build the image and run it.\n");
        write(&src.join("needs-image/environment/Dockerfile"), "FROM alpine\n");
        write(&src.join("needs-image/tests/test_x.py"), "def test_x(): assert True\n");
        // a task with no tests at all: refused
        write(&src.join("no-tests/task.yaml"), "instruction: Do something.\n");

        let res = import_harbor(&src, &dest, false, 100).unwrap();
        let by = |n: &str| res.iter().find(|r| r.name == n).cloned().unwrap();
        assert!(by("fix-parser").ok, "{res:?}");
        assert!(!by("needs-image").ok && by("needs-image").note.contains("container"), "{res:?}");
        assert!(!by("no-tests").ok && by("no-tests").note.contains("tests"), "{res:?}");

        let spec: TaskSpec = toml::from_str(&std::fs::read_to_string(dest.join("fix-parser/task.toml")).unwrap()).unwrap();
        assert_eq!(spec.name, "fix-parser");
        assert!(spec.prompt.contains("Fix the parser"));
        assert_eq!(spec.check, "bash run-tests.sh");
        assert_eq!(spec.timeout_secs, Some(600));
        assert!(spec.tags.contains(&"imported".to_string()) && spec.tags.contains(&"hard".to_string()));
        assert!(dest.join("fix-parser/fixture/src/parser.py").is_file(), "workspace files are copied");
        assert!(!dest.join("fix-parser/fixture/solution").exists(), "the reference solution is never copied");
        // the imported task is loadable by the eval runner
        let tasks = load_tasks(&dest, None).unwrap();
        assert_eq!(tasks.len(), 1);

        // with --include-docker the containerised one comes in, tagged
        let res = import_harbor(&src, &dest, true, 100).unwrap();
        assert!(res.iter().find(|r| r.name == "needs-image").unwrap().ok);
        let spec: TaskSpec = toml::from_str(&std::fs::read_to_string(dest.join("needs-image/task.toml")).unwrap()).unwrap();
        assert!(spec.tags.contains(&"needs-docker".to_string()));
        assert!(spec.check.contains("pytest"));
        let _ = std::fs::remove_dir_all(&d);
    }
}
