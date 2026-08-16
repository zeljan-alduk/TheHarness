//! Deterministic multi-agent workflows: TOML scripts of steps run in order.
//!   type = "shell"  cmd, must_succeed
//!   type = "agent"  task (template), parallel = ["a","b"] or items_from = "steps.<id>.lines" (one agent per item, run
//!                   concurrently), read_only, max_turns, check (shell; if it fails the agent is re-run with the failure
//!                   output, up to max_attempts)
//! Templates: {args} {workdir} {item} {steps.<id>.output}
//! Files: ~/.config/harness/workflows/*.toml and <workdir>/.harness/workflows/*.toml

use crate::agent::{Agent, SubAgentEnv};
use crate::events::{Event, Sink};
use crate::tools::ToolCtx;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Workflow { pub name: String, #[serde(default)] pub description: String, pub steps: Vec<Step> }

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Step {
    pub id: String,
    #[serde(rename = "type")] pub kind: String,
    #[serde(default)] pub cmd: String,
    #[serde(default)] pub must_succeed: bool,
    #[serde(default)] pub task: String,
    #[serde(default)] pub parallel: Vec<String>,
    #[serde(default)] pub items_from: Option<String>,
    #[serde(default)] pub read_only: bool,
    #[serde(default)] pub max_turns: Option<usize>,
    #[serde(default)] pub check: Option<String>,
    #[serde(default = "d_attempts")] pub max_attempts: usize,
}
fn d_attempts() -> usize { 3 }

pub struct WorkflowEnv { pub env: Arc<SubAgentEnv>, pub ctx: ToolCtx, pub sink: Arc<dyn Sink>, pub base_system: String }

pub fn dirs(workdir: &Path) -> Vec<PathBuf> {
    let mut v = Vec::new();
    v.push(crate::setup::config_dir().join("workflows"));
    v.push(workdir.join(".harness/workflows"));
    v
}

pub fn ensure_examples() {
    let d = crate::setup::config_dir().join("workflows");
    let _ = std::fs::create_dir_all(&d);
    let review = r#"name = "review"
description = "Review the working-tree diff from three lenses in parallel, then merge into one prioritized list"

[[steps]]
id = "diff"
type = "shell"
cmd = "git diff HEAD --stat && git diff HEAD | head -n 4000"

[[steps]]
id = "lenses"
type = "agent"
parallel = ["correctness bugs", "security and unsafe operations", "simplification and readability"]
read_only = true
max_turns = 12
task = "You are reviewing a code change for {item}. Read the relevant files with read_file/grep if the diff lacks context. Return a concise bullet list of concrete findings with file:line and a one-line fix suggestion each; say 'no findings' if none.\n\nDIFF:\n{steps.diff.output}"

[[steps]]
id = "merge"
type = "agent"
read_only = true
max_turns = 4
task = "Merge these review notes into ONE prioritized list (P0 blocking, P1 should fix, P2 nit), de-duplicated, keeping file:line references. Notes:\n{steps.lenses.output}"
"#;
    let fix = r#"name = "fix-tests"
description = "Run the test command; if it fails, let an agent fix the code and re-run, up to 3 attempts"

[[steps]]
id = "fix"
type = "agent"
max_turns = 25
check = "{args}"
max_attempts = 3
task = "The command `{args}` must pass. Run it, read the failures, fix the code (not the tests unless they are wrong), and re-run until it passes. Report what you changed."
"#;
    for (n, t) in [("review.toml", review), ("fix-tests.toml", fix)] { let p = d.join(n); if !p.exists() { let _ = std::fs::write(p, t); } }
}

pub fn list(workdir: &Path) -> Vec<(String, String, PathBuf)> {
    ensure_examples();
    let mut out = Vec::new();
    for d in dirs(workdir) {
        if let Ok(rd) = std::fs::read_dir(&d) { for e in rd.flatten() { let p = e.path(); if p.extension().map(|x| x == "toml").unwrap_or(false) { if let Ok(w) = load(&p) { out.push((w.name.clone(), w.description.clone(), p)); } } } }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

pub fn load(p: &Path) -> Result<Workflow> { Ok(toml::from_str(&std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?)?) }

pub fn find(name_or_path: &str, workdir: &Path) -> Result<Workflow> {
    let p = Path::new(name_or_path);
    if p.is_file() { return load(p); }
    for (n, _, path) in list(workdir) { if n == name_or_path { return load(&path); } }
    bail!("no workflow named '{name_or_path}' (see: harness workflow list)")
}

fn render(t: &str, vars: &HashMap<String, String>) -> String {
    let mut s = t.to_string();
    for (k, v) in vars { s = s.replace(&format!("{{{k}}}"), v); }
    s
}

pub async fn run(wf: &Workflow, args: &str, wenv: &WorkflowEnv) -> Result<String> {
    let workdir = wenv.ctx.workdir.clone();
    let mut vars: HashMap<String, String> = HashMap::new();
    vars.insert("args".into(), args.to_string());
    vars.insert("workdir".into(), workdir.display().to_string());
    let mut last_output = String::new();
    for (i, step) in wf.steps.iter().enumerate() {
        let id = format!("wf:{}:{}", wf.name, step.id);
        match step.kind.as_str() {
            "shell" => {
                let cmd = render(&step.cmd, &vars);
                wenv.sink.emit(&Event::ToolCall { id: id.clone(), name: format!("workflow ▸ shell {}", step.id), args: cmd.clone() });
                let t0 = std::time::Instant::now();
                let o = crate::sandbox::run_shell(&cmd, &workdir, Duration::from_secs(1800), 60_000).await?;
                let out = format!("{}{}", o.stdout, if o.stderr.is_empty() { String::new() } else { format!("\n[stderr]\n{}", o.stderr) });
                wenv.sink.emit(&Event::ToolResult { id, name: format!("workflow ▸ shell {}", step.id), result: crate::sandbox::truncate_middle(&out, 4000), secs: t0.elapsed().as_secs_f64(), images: vec![] });
                if step.must_succeed && !o.success() { bail!("step '{}' failed (exit {:?}); workflow stopped", step.id, o.code); }
                vars.insert(format!("steps.{}.output", step.id), out.clone());
                vars.insert(format!("steps.{}.lines", step.id), out.clone());
                last_output = out;
            }
            "agent" => {
                let items: Vec<String> = if let Some(from) = &step.items_from { vars.get(from).map(|s| s.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect()).unwrap_or_default() } else if !step.parallel.is_empty() { step.parallel.clone() } else { vec![String::new()] };
                let futs = items.iter().enumerate().map(|(k, item)| {
                    let mut v = vars.clone(); v.insert("item".into(), item.clone());
                    let task = render(&step.task, &v);
                    let label = if item.is_empty() { step.id.clone() } else { format!("{} [{}]", step.id, crate::llm::truncate_for_log(item, 30)) };
                    let check = step.check.as_ref().map(|c| render(c, &v));
                    let sid = format!("{id}:{k}");
                    async move { run_agent_step(wenv, &sid, &label, &task, step, check.as_deref()).await.map(|o| if item.is_empty() { o } else { format!("### {item}\n{o}") }) }
                });
                let results = futures_util::future::join_all(futs).await;
                let mut outs = Vec::new();
                for r in results { outs.push(r?); }
                let out = outs.join("\n\n");
                vars.insert(format!("steps.{}.output", step.id), out.clone());
                vars.insert(format!("steps.{}.lines", step.id), out.clone());
                last_output = out;
            }
            other => bail!("step {} ({}): unknown type '{other}'", i + 1, step.id),
        }
    }
    Ok(last_output)
}

async fn run_agent_step(wenv: &WorkflowEnv, sid: &str, label: &str, task: &str, step: &Step, check: Option<&str>) -> Result<String> {
    let env = &wenv.env;
    let mut pcfg = env.policy.cfg.clone(); if step.read_only { pcfg.mode = crate::permissions::Mode::Plan; }
    let policy = crate::permissions::Policy::new(pcfg, &wenv.ctx.workdir);
    let registry = env.registry.without("spawn_agent");
    let sink = crate::agent::PrefixSink { inner: wenv.sink.clone(), prefix: format!("wf {} ", crate::llm::truncate_for_log(label, 24)), info: None };
    let mut task_now = task.to_string();
    let mut last = String::new();
    for attempt in 1..=step.max_attempts.max(1) {
        wenv.sink.emit(&Event::ToolCall { id: format!("{sid}:{attempt}"), name: format!("workflow ▸ agent {label}"), args: crate::llm::truncate_for_log(&task_now, 400) });
        let t0 = std::time::Instant::now();
        let agent = Agent { client: &env.client, registry: &registry, ctx: &wenv.ctx, max_turns: step.max_turns.unwrap_or(25), context_budget: env.context_budget, sink: &sink, stream: env.stream, policy: &policy, approver: env.approver.as_ref() };
        let system = format!("{}\n\nYou are running as one step of the workflow; do exactly the step's task and end with a concise report.", wenv.base_system);
        let (text, stats) = agent.run(&system, &task_now).await?;
        last = text.clone();
        let mut result = format!("[{} turns, {} tools, {:.0}s]\n{}", stats.turns, stats.tool_calls, stats.wall_secs, crate::llm::truncate_for_log(&text, 3000));
        let mut ok = true;
        if let Some(c) = check {
            let o = crate::sandbox::run_shell(c, &wenv.ctx.workdir, Duration::from_secs(1800), 20_000).await?;
            ok = o.success();
            result.push_str(&format!("\ncheck `{}` → {}", crate::llm::truncate_for_log(c, 80), if ok { "ok" } else { "FAILED" }));
            if !ok { task_now = format!("{task}\n\nPrevious attempt did not make the check pass. Check output:\n{}\n{}", crate::llm::truncate_for_log(&o.stdout, 3000), crate::llm::truncate_for_log(&o.stderr, 1500)); }
        }
        wenv.sink.emit(&Event::ToolResult { id: format!("{sid}:{attempt}"), name: format!("workflow ▸ agent {label}"), result, secs: t0.elapsed().as_secs_f64(), images: vec![] });
        if ok { return Ok(last); }
    }
    bail!("step '{}' did not pass its check after {} attempt(s); last report:\n{}", step.id, step.max_attempts, last)
}
