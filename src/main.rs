mod tui;

use harness::{agent, config, eval, events, llm, sandbox, tools};

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Parser)]
#[command(name = "harness", version = env!("HARNESS_VERSION"), about = "Local-first agentic coding harness (Qwen/LM Studio/llama.cpp/Ollama)")]
struct Cli {
    /// Path to harness.toml (default: ./harness.toml or next to the binary)
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    /// Print model reasoning and full tool results
    #[arg(short, long, global = true)]
    verbose: bool,
    /// Emit machine-readable JSONL events on stdout instead of human logs (for UIs)
    #[arg(long, global = true)]
    json: bool,
    /// Non-interactive approval: answer "yes" to permission prompts (otherwise they are denied)
    #[arg(short = 'y', long, global = true)]
    yes: bool,
    /// Permission mode override: bypass | auto | ask | plan
    #[arg(long, global = true)]
    permissions: Option<String>,
    /// Resume a saved session in the TUI (id, number from /sessions, or "last")
    #[arg(short = 'r', long)]
    resume: Option<String>,
    /// Continue the most recent session for this directory
    #[arg(short = 'c', long)]
    r#continue: bool,
    /// No subcommand → interactive terminal UI
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Interactive terminal UI (default when no subcommand is given)
    Chat,
    /// Run the agent on a task in a working directory
    Run {
        /// Working directory (default: current dir)
        #[arg(short = 'C', long)]
        dir: Option<PathBuf>,
        #[arg(long)]
        max_turns: Option<usize>,
        /// Disable web tools for this run
        #[arg(long)]
        no_net: bool,
        /// The task, in natural language. Use '-' to read from stdin.
        task: String,
    },
    /// Run the eval suite (the fitness function) and print a JSON report
    Eval {
        /// Only tasks whose name or tag contains this
        #[arg(short, long)]
        filter: Option<String>,
        /// Write JSON report here (default: <runs_dir>/report.json)
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
    /// Self-improvement: run the agent on the harness's own repo, on a new git branch
    #[command(name = "self")]
    SelfImprove {
        /// Branch name (default: proposal/<slug of task>)
        #[arg(short, long)]
        branch: Option<String>,
        /// What to improve
        task: String,
    },
    /// Call a single tool directly (no model) — for debugging tools: harness tool bash '{"cmd":"ls"}'
    Tool {
        #[arg(short = 'C', long)]
        dir: Option<PathBuf>,
        name: String,
        /// JSON arguments (default: {})
        args: Option<String>,
    },
    /// Audit external tools; gather them in ~/.config/harness/bin; --install adds missing ones via Homebrew
    Setup {
        #[arg(long)]
        install: bool,
        /// Add recommended MCP servers (chrome-devtools enabled; playwright, filesystem disabled) to ~/.config/harness/mcp.json
        #[arg(long)]
        mcp_defaults: bool,
    },
    /// Manage plugins: list | install <spec> | enable|disable|remove|update <name>
    Plugin {
        #[command(subcommand)]
        action: PluginCmd,
    },
    /// Show configured MCP servers, start them, and list the tools they expose
    Mcp {
        #[arg(short = 'C', long)]
        dir: Option<PathBuf>,
    },
    /// Judge a proposal branch against main with the eval suite (build, test, N eval runs, regression gate); --merge merges on green
    Arbiter {
        branch: String,
        #[arg(long, default_value_t = 1)]
        runs: usize,
        /// Reuse an existing eval report as the main baseline (skips baseline runs)
        #[arg(long)]
        baseline: Option<PathBuf>,
        #[arg(short, long)]
        filter: Option<String>,
        #[arg(long)]
        merge: bool,
    },
    /// Serve the web UI (same UI as the desktop app) on localhost
    Serve {
        /// address to bind, e.g. 127.0.0.1:7878 (use 0.0.0.0:7878 to reach it from other devices — no auth!)
        #[arg(long, default_value = "127.0.0.1:7878")]
        bind: String,
    },
    /// Workflows: list | run <name|path> [args]
    Workflow {
        #[command(subcommand)]
        action: WorkflowCmd,
    },
    /// List saved sessions
    Sessions,
    /// (internal) stdio↔socket proxy used to expose harness tools to Claude Code
    #[command(name = "mcp-proxy", hide = true)]
    McpProxy { addr: String },
    /// List models on the configured server
    Models,
    /// Print the effective configuration
    Config,
}

#[derive(Subcommand)]
enum WorkflowCmd {
    List,
    Run {
        #[arg(short = 'C', long)]
        dir: Option<PathBuf>,
        name: String,
        /// arguments available as {args} in the workflow
        args: Vec<String>,
    },
}

#[derive(Subcommand)]
enum PluginCmd {
    /// Show installed plugins and the downloadable catalog (● enabled ◐ disabled ○ downloadable)
    List { #[arg(long)] refresh: bool },
    Install { spec: String },
    Enable { name: String },
    Disable { name: String },
    Remove { name: String },
    Update { name: String },
    /// Update all installed git plugins
    UpdateAll,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut cfg = config::Config::load(cli.config.as_deref())?;
    if let Some(m) = &cli.permissions { cfg.permissions.mode = harness::permissions::Mode::parse(m).context("--permissions must be bypass|auto|ask|plan")?; }
    sandbox::configure_seatbelt(cfg.sandbox.mode == "seatbelt" || cfg.sandbox.mode == "bwrap", cfg.sandbox.deny_network, cfg.sandbox.allow_write.clone());
    let client = llm::Client::new(&cfg.llm)?;

    match cli.cmd.unwrap_or(Cmd::Chat) {
        Cmd::Chat => {
            let resume = cli.resume.clone().or(if cli.r#continue { Some("last".into()) } else { None });
            tui::run(cfg, resume).await?;
        }
        Cmd::Arbiter { branch, runs, filter, merge, baseline } => {
            let repo = repo_root()?;
            let mut log = |s: &str| eprintln!("{s}");
            if let Some(b) = &baseline { harness::arbiter::seed_baseline(&repo, filter.as_deref(), b)?; }
            let v = harness::arbiter::judge(&repo, &branch, runs.max(1), filter.as_deref(), merge, &mut log)?;
            println!("{}", serde_json::to_string(&serde_json::json!({"branch": v.branch, "green": v.green, "tests_ok": v.tests_ok, "reasons": v.reasons}))?);
            if !v.green { std::process::exit(1); }
        }
        Cmd::Serve { bind } => { harness::serve::serve(cfg, &bind).await?; }
        Cmd::Workflow { action } => match action {
            WorkflowCmd::List => { for (n, d, p) in harness::workflow::list(&std::env::current_dir()?) { println!("{:<16} {:<70} {}", n, llm::truncate_for_log(&d, 70), p.display()); } }
            WorkflowCmd::Run { dir, name, args } => {
                let workdir = dir.unwrap_or(std::env::current_dir()?).canonicalize()?;
                let wf = harness::workflow::find(&name, &workdir)?;
                let sink: std::sync::Arc<dyn events::Sink> = if cli.json { std::sync::Arc::new(events::JsonlSink) } else { std::sync::Arc::new(events::StderrSink { verbose: cli.verbose }) };
                let ts = tools::build_toolset(cfg.net.enabled, &workdir, true).await;
                let budget = cfg.llm.effective_budget(llm::detect_context_length(&cfg.llm.base_url, &cfg.llm.model).await.map(|d| d.0));
                let mut pcfg = cfg.permissions.clone(); pcfg.allow.extend(harness::permissions::persisted_rules());
                let policy = std::sync::Arc::new(harness::permissions::Policy::new(pcfg, &workdir));
                let approver: std::sync::Arc<dyn harness::permissions::Approver> = std::sync::Arc::new(harness::permissions::AutoApprover { yes: cli.yes });
                let env = std::sync::Arc::new(agent::SubAgentEnv::new(client.clone(), ts.registry.clone(), policy.clone(), approver, sink.clone(), budget, true));
                let store = if cfg.memory.enabled { harness::memory::MemoryStore::open(&cfg.memory).ok() } else { None };
                let ctx = tools::ToolCtx { memory: store.clone(), subagent: Some(env.clone()), redact_secrets: cfg.security.redact_secrets, hooks: cfg.hooks.clone(), lsp_servers: cfg.lsp.servers.clone(), timeout: Duration::from_secs(cfg.agent.tool_timeout_secs), max_output: cfg.agent.max_tool_output_chars, net: cfg.net.clone(), ..tools::ToolCtx::basic(workdir.clone()) };
                let base_system = agent::system_prompt_with_memory(&workdir.display().to_string(), &ts.registry.names(), Some(&ts.prompt_extra), store.as_ref());
                let wenv = harness::workflow::WorkflowEnv { env, ctx, sink, base_system };
                let out = harness::workflow::run(&wf, &args.join(" "), &wenv).await?;
                println!("\n{out}");
            }
        },
        Cmd::Sessions => {
            let store = harness::sessions::SessionStore::open()?;
            for (i, m) in store.list(None).iter().take(40).enumerate() { println!("{:>2}. {}  {:<50} {:<30} {} turns · {}", i + 1, m.id, llm::truncate_for_log(&m.title, 50), m.workdir, m.turns, harness::sessions::fmt_age(m.updated)); }
        }
        Cmd::Tool { dir, name, args } => {
            let workdir = dir.unwrap_or(std::env::current_dir()?).canonicalize().context("workdir does not exist")?;
            let store = if cfg.memory.enabled { harness::memory::MemoryStore::open(&cfg.memory).ok() } else { None };
            let ctx = tools::ToolCtx { workdir: workdir.clone(), timeout: Duration::from_secs(cfg.agent.tool_timeout_secs), max_output: cfg.agent.max_tool_output_chars, net: cfg.net.clone(), memory: store, subagent: None, redact_secrets: cfg.security.redact_secrets, hooks: cfg.hooks.clone(), todos: Default::default(), lsp_servers: cfg.lsp.servers.clone(), extra_roots: vec![], approver: None, inbox: Default::default(), cancel: None, cwd: None, session_id: None };
            let ts = tools::build_toolset(cfg.net.enabled, &workdir, name.starts_with("mcp__")).await;
            let out = ts.registry.call(&name, args.as_deref().unwrap_or("{}"), &ctx).await;
            println!("{}", out.text);
            if !out.images.is_empty() { eprintln!("({} image(s) attached)", out.images.len()); }
            if out.text.starts_with("error:") { std::process::exit(1); }
        }
        Cmd::Setup { install, mcp_defaults } => {
            if mcp_defaults { let added = harness::setup::write_default_mcp()?; println!("mcp.json: added {}", if added.is_empty() { "nothing (all present)".into() } else { added.join(", ") }); }
            let mut st = harness::setup::check();
            harness::setup::print_report(&st);
            if install {
                let done = harness::setup::install_missing(&st)?;
                if !done.is_empty() { eprintln!("installed: {}", done.join(", ")); }
                st = harness::setup::check();
            }
            let (n, dir) = harness::setup::link_all(&st)?;
            let missing: Vec<&str> = st.iter().filter(|s| !s.ok()).map(|s| s.name).collect();
            println!("\n{n} binaries linked into {}", dir.display());
            if missing.is_empty() { println!("all tools available ✓"); } else { println!("missing: {} → run `harness setup --install`", missing.join(", ")); }
        }
        Cmd::Plugin { action } => {
            let mut p = harness::plugins::Plugins::open()?;
            match action {
                PluginCmd::List { refresh } => {
                    let installed = p.installed();
                    println!("Installed ({}):", installed.len());
                    for pl in &installed { println!("  {} {:<28} {}sk {}cmd {}mcp{}  {}", if pl.enabled { "●" } else { "◐" }, pl.path.file_name().unwrap().to_string_lossy(), pl.skills.len(), pl.commands.len(), pl.mcp_servers.len(), if pl.ts_only { " ts-only" } else { "" }, pl.description); }
                    match p.catalog(refresh).await {
                        Ok(c) => { println!("\nDownloadable ({}), install with: harness plugin install <owner/repo>", c.entries.len()); for e in c.entries.iter().take(60) { let inst = installed.iter().any(|x| x.origin.as_deref().map(|o| o.contains(&e.full_name)).unwrap_or(false)); println!("  {} {:<40} ★{:<6} {:<10} {}", if inst { "●" } else { "○" }, e.full_name, e.stars, e.language, e.description); } }
                        Err(e) => eprintln!("catalog unavailable: {e:#}"),
                    }
                }
                PluginCmd::Install { spec } => { let n = p.install(&spec).await?; let info = p.inspect(&p.dir.join(&n)); println!("installed {n}: {} skills, {} commands, {} mcp servers{}", info.skills.len(), info.commands.len(), info.mcp_servers.len(), if info.ts_only { " (TypeScript-only DSH plugin: code not runnable here)" } else { "" }); }
                PluginCmd::Enable { name } => { p.set_enabled(&name, true)?; println!("enabled {name}"); }
                PluginCmd::Disable { name } => { p.set_enabled(&name, false)?; println!("disabled {name}"); }
                PluginCmd::Remove { name } => { p.remove(&name)?; println!("removed {name}"); }
                PluginCmd::Update { name } => { println!("{}", p.update(&name).await?); }
                PluginCmd::UpdateAll => { for (n, r) in p.update_all().await { println!("{n}: {}", match r { Ok(m) => m, Err(e) => format!("failed: {e:#}") }); } }
            }
        }
        Cmd::Mcp { dir } => {
            let workdir = dir.unwrap_or(std::env::current_dir()?).canonicalize()?;
            let extra = harness::plugins::Plugins::open().map(|p| p.mcp_files()).unwrap_or_default();
            let servers = harness::mcp::discover(&workdir, &extra);
            println!("configured servers: {}", servers.len());
            for (n, c, f) in &servers { println!("  {:<18} {} {}   ← {}", n, if c.command.is_empty() { c.url.clone().unwrap_or_default() } else { c.command.clone() }, c.args.join(" "), f.display()); }
            let ts = tools::build_toolset(cfg.net.enabled, &workdir, true).await;
            for n in &ts.notes { println!("· {n}"); }
            for d in ts.registry.defs().into_iter().filter(|d| d.function.name.starts_with("mcp__")) { println!("  {:<40} {}", d.function.name, llm::truncate_for_log(&d.function.description, 90)); }
        }
        Cmd::McpProxy { addr } => { harness::mcp_bridge::proxy(&addr).await?; }
        Cmd::Models => {
            for m in client.list_models().await? { println!("{m}"); }
        }
        Cmd::Config => {
            println!("{cfg:#?}");
        }
        Cmd::Run { dir, max_turns, no_net, task } => {
            let task = if task == "-" { let mut s = String::new(); std::io::Read::read_to_string(&mut std::io::stdin(), &mut s)?; s } else { task };
            if let Some(n) = max_turns { cfg.agent.max_turns = n; }
            if no_net { cfg.net.enabled = false; }
            let workdir = dir.unwrap_or(std::env::current_dir()?).canonicalize().context("workdir does not exist")?;
            let (text, _stats) = run_agent(&cfg, &client, &workdir, &task, None, cli.verbose, cli.json, cli.yes).await?;
            if !cli.json { println!("\n{text}"); }
        }
        Cmd::Eval { filter, out } => {
            let report = eval::run_all(&cfg, &client, filter.as_deref(), cli.verbose).await?;
            let out = out.unwrap_or(PathBuf::from(&cfg.eval.runs_dir).join("report.json"));
            if let Some(p) = out.parent() { std::fs::create_dir_all(p)?; }
            std::fs::write(&out, serde_json::to_string_pretty(&report)?)?;
            eprintln!("\n══ {}/{} passed ({:.0}%)  model={}  tokens={}+{}  wall={:.0}s  → {}",
                report.passed, report.total, report.score * 100.0, report.model,
                report.total_prompt_tokens, report.total_completion_tokens, report.total_wall_secs, out.display());
            println!("{}", serde_json::to_string(&serde_json::json!({"passed": report.passed, "total": report.total, "score": report.score}))?);
            if report.passed < report.total { std::process::exit(1); }
        }
        Cmd::SelfImprove { branch, task } => {
            reexec_from_temp_copy()?;
            let task = if task == "-" { let mut s = String::new(); std::io::Read::read_to_string(&mut std::io::stdin(), &mut s)?; s.trim().to_string() } else { task };
            let repo = repo_root()?;
            let branch = branch.unwrap_or_else(|| format!("proposal/{}", slug(&task)));
            let o = sandbox::run_shell("git rev-parse --is-inside-work-tree && git status --porcelain", &repo, Duration::from_secs(10), 4000).await?;
            if !o.success() { bail!("{} is not a git repository; run `git init` first", repo.display()); }
            if o.stdout.lines().count() > 1 { bail!("working tree is dirty; commit or stash before `harness self`:\n{}", o.stdout); }
            // Work in a separate git worktree so this checkout (and any human editing it) is untouched.
            let wt = std::env::temp_dir().join("harness-proposals").join(branch.replace('/', "__"));
            std::fs::create_dir_all(wt.parent().unwrap())?;
            let cmd = format!("git worktree add -q -b {} {}", harness::security::shell_quote(&branch), harness::security::shell_quote(&wt.display().to_string()));
            let o = sandbox::run_shell(&cmd, &repo, Duration::from_secs(30), 4000).await?;
            if !o.success() { bail!("could not create worktree for {branch}: {}{}", o.stdout, o.stderr); }
            eprintln!("branch {branch} → worktree {}", wt.display());
            let extra = format!(
"SELF-IMPROVEMENT MODE. You are editing your own harness (a Rust project) in a git worktree on branch `{branch}`. The working directory IS the repo root.
Ground rules:
- Read README.md and the relevant src/ files (src/tools/mod.rs, an existing tool like src/tools/web.rs) before changing anything.
- Do NOT edit src/main.rs, src/llm.rs, src/sandbox.rs, src/agent.rs or src/eval.rs unless the task explicitly requires it; prefer adding tools under src/tools/, adjusting the system prompt, and adding evals/tasks.
- Register new tools in src/tools/mod.rs (Registry::defaults) and add unit tests.
- After edits: `cargo build --release` must succeed and `cargo test` must pass (use timeout_secs 600 for cargo commands; the first build is slow).
- Then run `./target/release/harness eval` (timeout_secs 1800) and report the score.
- Commit your work on this branch with a message that states the change and the eval result. Never merge into main; a human (or the arbiter) does that. Do not touch other branches or worktrees.");
            let (text, _stats) = run_agent(&cfg, &client, &wt, &task, Some(&extra), cli.verbose, cli.json, true).await?;
            if !cli.json { println!("\n{text}"); }
            eprintln!("Review with: git log --oneline main..{branch} && git diff main..{branch}\nWorktree: {} (remove with: git worktree remove --force '{}')", wt.display(), wt.display());
        }
    }
    Ok(())
}

async fn run_agent(cfg: &config::Config, client: &llm::Client, workdir: &std::path::Path, task: &str, extra: Option<&str>, verbose: bool, json: bool, yes: bool) -> Result<(String, agent::RunStats)> {
    let sink: std::sync::Arc<dyn events::Sink> = if json { std::sync::Arc::new(events::JsonlSink) } else { std::sync::Arc::new(events::StderrSink { verbose }) };
    let ctx = tools::ToolCtx {
        workdir: workdir.to_path_buf(),
        timeout: Duration::from_secs(cfg.agent.tool_timeout_secs),
        max_output: cfg.agent.max_tool_output_chars,
        net: cfg.net.clone(),
        memory: None,
        subagent: None,
        redact_secrets: cfg.security.redact_secrets, hooks: cfg.hooks.clone(), todos: Default::default(), lsp_servers: cfg.lsp.servers.clone(), extra_roots: vec![], approver: None, inbox: Default::default(), cancel: None, cwd: None, session_id: None,
    };
    let store = if cfg.memory.enabled { harness::memory::MemoryStore::open(&cfg.memory).ok() } else { None };
    if let Some(m) = &store { let _ = m.touch_project(workdir); }
    let ctx = tools::ToolCtx { memory: store.clone(), ..ctx };
    let toolset = tools::build_toolset(cfg.net.enabled, workdir, true).await;
    for n in &toolset.notes { eprintln!("· {n}"); }
    let registry = &toolset.registry;
    let extra_all = format!("{}{}", extra.unwrap_or(""), toolset.prompt_extra);
    let system = agent::system_prompt_with_memory(&workdir.display().to_string(), &registry.names(), Some(&extra_all), store.as_ref());
    let detected = llm::detect_context_length(&cfg.llm.base_url, &cfg.llm.model).await;
    let budget = cfg.llm.effective_budget(detected.map(|d| d.0));
    if let Some((n, src)) = detected { eprintln!("· context {} tokens ({src}) · auto-compact at {}", n, budget); } else { eprintln!("· context length unknown · auto-compact at {budget}"); }
    let mut pcfg = cfg.permissions.clone();
    pcfg.allow.extend(harness::permissions::persisted_rules());
    let policy = std::sync::Arc::new(harness::permissions::Policy::new(pcfg, workdir));
    let approver: std::sync::Arc<dyn harness::permissions::Approver> = std::sync::Arc::new(harness::permissions::AutoApprover { yes });
    if !yes && policy.mode() != harness::permissions::Mode::Bypass { eprintln!("· permissions: {} (non-interactive: prompts are denied; pass -y to approve, or --permissions bypass)", policy.mode().label()); }
    let ctx = tools::ToolCtx { subagent: Some(std::sync::Arc::new(agent::SubAgentEnv::new(client.clone(), registry.clone(), policy.clone(), approver.clone(), sink.clone(), budget, true))), approver: Some(approver.clone()), cwd: Some(harness::worktree::new_cell()), ..ctx };
    let mut msgs = Vec::new();
    let out = if client.provider() == llm::Provider::ClaudeCode {
        // Claude Code backend: our tools bridged over MCP; the claude CLI drives the loop
        let host = std::sync::Arc::new(harness::mcp_bridge::BridgeHost { registry: registry.clone(), ctx: ctx.clone(), policy: policy.clone(), approver: approver.clone(), sink: sink.clone() });
        let session = harness::claude_code::ClaudeCodeSession::start(workdir, Some(client.model()), &system, host, None).await?;
        msgs.push(llm::Message::system(system.clone())); msgs.push(llm::Message::user(task));
        let r = session.run_turn(task, &[], sink.as_ref()).await;
        if let Ok((t, _)) = &r { msgs.push(llm::Message { role: "assistant".into(), content: Some(llm::Content::Text(t.clone())), ..Default::default() }); }
        session.stop().await;
        r
    } else {
        let a = agent::Agent { client, registry, ctx: &ctx, max_turns: cfg.agent.max_turns, context_budget: budget, sink: sink.as_ref(), stream: true, policy: &policy, approver: approver.as_ref() };
        a.run_turn(&mut msgs, &system, task).await
    };
    // always persist the transcript (also on error) — it is the run log
    if let Ok(store_s) = harness::sessions::SessionStore::open() {
        let mut meta = harness::sessions::Meta { id: harness::sessions::SessionStore::new_id(), workdir: workdir.display().to_string(), model: client.model().to_string(), ..Default::default() };
        if let Ok((_, st)) = &out { meta.prompt_tokens = st.prompt_tokens; meta.completion_tokens = st.completion_tokens; }
        let _ = store_s.save(&mut meta, &msgs);
        if !json { eprintln!("· session saved: {} (harness --resume {})", meta.id, meta.id); }
    }
    let out = out?;
    if let Some(m) = &store { agent::reflect_after_run(client, m, &msgs, &out.1, sink.as_ref()).await; }
    Ok(out)
}

/// `self` mode rebuilds the harness while it runs. Never run from the binary being edited:
/// copy ourselves to a temp path and exec that copy (once).
fn reexec_from_temp_copy() -> Result<()> {
    if std::env::var_os("HARNESS_SELF_EXEC").is_some() { return Ok(()); }
    let exe = std::env::current_exe()?;
    let tmp = std::env::temp_dir().join(format!("harness-self-{}", std::process::id()));
    std::fs::copy(&exe, &tmp).context("copying harness binary to temp")?;
    let mut cmd = std::process::Command::new(&tmp);
    cmd.args(std::env::args_os().skip(1)).env("HARNESS_SELF_EXEC", "1").env("HARNESS_ORIG_EXE", &exe);
    #[cfg(unix)] { let err = cmd.exec(); bail!("failed to re-exec {}: {err}", tmp.display()) }
    #[cfg(not(unix))] { let st = cmd.status().with_context(|| format!("failed to run {}", tmp.display()))?; std::process::exit(st.code().unwrap_or(1)); }
}

fn repo_root() -> Result<PathBuf> {
    // The harness's own repo: the directory containing Cargo.toml, found from cwd or the exe.
    let mut cands = vec![std::env::current_dir()?];
    let exe = std::env::var_os("HARNESS_ORIG_EXE").map(PathBuf::from).or_else(|| std::env::current_exe().ok());
    if let Some(exe) = exe { if let Some(r) = exe.ancestors().nth(3) { cands.push(r.to_path_buf()); } }
    for c in cands { if c.join("Cargo.toml").is_file() && c.join("harness.toml").is_file() { return Ok(c.canonicalize()?); } }
    bail!("could not locate the harness repo (Cargo.toml + harness.toml); run from the repo root")
}

fn slug(s: &str) -> String {
    let s: String = s.to_lowercase().chars().map(|c| if c.is_alphanumeric() { c } else { '-' }).collect();
    let mut out = String::new();
    for part in s.split('-').filter(|p| !p.is_empty()) { if out.len() + part.len() > 40 { break; } if !out.is_empty() { out.push('-'); } out.push_str(part); }
    if out.is_empty() { "task".into() } else { out }
}
