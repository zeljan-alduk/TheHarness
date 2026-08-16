mod agent;
mod config;
mod eval;
mod events;
mod llm;
mod sandbox;
mod tools;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Parser)]
#[command(name = "harness", version, about = "Local-first agentic coding harness (Qwen/LM Studio/llama.cpp/Ollama)")]
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
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
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
    /// List models on the configured server
    Models,
    /// Print the effective configuration
    Config,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut cfg = config::Config::load(cli.config.as_deref())?;
    let client = llm::Client::new(&cfg.llm)?;

    match cli.cmd {
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
            let (text, _stats) = run_agent(&cfg, &client, &workdir, &task, None, cli.verbose, cli.json).await?;
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
            let repo = repo_root()?;
            let branch = branch.unwrap_or_else(|| format!("proposal/{}", slug(&task)));
            let o = sandbox::run_shell("git rev-parse --is-inside-work-tree && git status --porcelain", &repo, Duration::from_secs(10), 4000).await?;
            if !o.success() { bail!("{} is not a git repository; run `git init` first", repo.display()); }
            if o.stdout.lines().count() > 1 { bail!("working tree is dirty; commit or stash before `harness self`:\n{}", o.stdout); }
            let o = sandbox::run_shell(&format!("git checkout -q -b '{branch}'"), &repo, Duration::from_secs(10), 4000).await?;
            if !o.success() { bail!("could not create branch {branch}: {}", o.stderr); }
            eprintln!("on branch {branch} in {}", repo.display());
            let extra = format!(
"SELF-IMPROVEMENT MODE. You are editing your own harness (a Rust project). You are on git branch `{branch}`.
Ground rules:
- Read README.md and the relevant src/ files before changing anything.
- Do NOT edit src/main.rs, src/llm.rs, src/sandbox.rs or the eval runner unless the task explicitly requires it; prefer changing tools, prompts, and evals/tasks.
- After edits: `cargo build --release` must succeed and `cargo test` must pass. Then run `./target/release/harness eval` and report the score before/after.
- Commit your work on this branch with a message that states the change and the eval delta. Never merge into main; a human (or the arbiter) does that.");
            let (text, _stats) = run_agent(&cfg, &client, &repo, &task, Some(&extra), cli.verbose, cli.json).await?;
            if !cli.json { println!("\n{text}"); }
            eprintln!("Review with: git log --oneline main..{branch} && git diff main..{branch}");
        }
    }
    Ok(())
}

async fn run_agent(cfg: &config::Config, client: &llm::Client, workdir: &std::path::Path, task: &str, extra: Option<&str>, verbose: bool, json: bool) -> Result<(String, agent::RunStats)> {
    let sink: Box<dyn events::Sink> = if json { Box::new(events::JsonlSink) } else { Box::new(events::StderrSink { verbose }) };
    let ctx = tools::ToolCtx {
        workdir: workdir.to_path_buf(),
        timeout: Duration::from_secs(cfg.agent.tool_timeout_secs),
        max_output: cfg.agent.max_tool_output_chars,
        net: cfg.net.clone(),
    };
    let registry = tools::Registry::defaults(cfg.net.enabled);
    let system = agent::system_prompt(&workdir.display().to_string(), &registry.names(), extra);
    let a = agent::Agent { client, registry: &registry, ctx: &ctx, max_turns: cfg.agent.max_turns, context_budget: cfg.llm.context_budget_tokens, sink: sink.as_ref() };
    a.run(&system, task).await
}

fn repo_root() -> Result<PathBuf> {
    // The harness's own repo: the directory containing Cargo.toml, found from cwd or the exe.
    let mut cands = vec![std::env::current_dir()?];
    if let Ok(exe) = std::env::current_exe() { if let Some(r) = exe.ancestors().nth(3) { cands.push(r.to_path_buf()); } }
    for c in cands { if c.join("Cargo.toml").is_file() && c.join("harness.toml").is_file() { return Ok(c.canonicalize()?); } }
    bail!("could not locate the harness repo (Cargo.toml + harness.toml); run from the repo root")
}

fn slug(s: &str) -> String {
    let s: String = s.to_lowercase().chars().map(|c| if c.is_alphanumeric() { c } else { '-' }).collect();
    let mut out = String::new();
    for part in s.split('-').filter(|p| !p.is_empty()) { if out.len() + part.len() > 40 { break; } if !out.is_empty() { out.push('-'); } out.push_str(part); }
    if out.is_empty() { "task".into() } else { out }
}
