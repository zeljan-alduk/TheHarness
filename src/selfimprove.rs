//! Smart self-improvement loop (`harness improve`, `/improve` in the TUI).
//!
//! propose → gate 1 (auto for frontier models, otherwise the user confirms) → implement each item on a
//! `proposal/*` branch in its own worktree → arbiter (build · tests · eval vs cached main baseline) →
//! merge → build + install the new binary atomically → gate 2 (the front-end offers a grace period to
//! cancel the automatic restart; the restart resumes the session with the previously chosen model/effort).
//!
//! UI-agnostic: progress is reported through `Stage` callbacks, questions go through the `Approver`.

use crate::config::Config;
use crate::events::Sink;
use crate::permissions::{Answer, Approver, Question, QuestionOption};
use crate::runner::{prepare, RunSetup};
use crate::security::shell_quote;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct Proposal {
    pub title: String,
    #[serde(default)] pub rationale: String,
    /// Concrete steps for the implementing agent.
    #[serde(default)] pub plan: String,
    #[serde(default)] pub files: Vec<String>,
    /// Optional new eval task (name) that would demonstrate the improvement.
    #[serde(default)] pub eval_task: Option<String>,
}

/// Progress reports for front-ends.
#[derive(Debug, Clone)]
pub enum Stage {
    Log(String),
    /// The plan; `auto` = approved without asking (smart backend).
    Plan { items: Vec<Proposal>, auto: bool },
    Approved(Vec<Proposal>),
    /// One item finished (merged or not).
    Item { title: String, branch: String, merged: bool, note: String },
    /// New binary installed at `exe`; the front-end should offer `grace_secs` to cancel the restart.
    Installed { summary: String, exe: PathBuf, grace_secs: u64 },
    Done { summary: String },
    Failed(String),
}

pub struct Job {
    pub cfg: Config,
    /// Optional focus ("make the bash tool …"); empty = let the agent pick from README/GAPS/TODO/BRAIN.
    pub hint: String,
    pub sink: Arc<dyn Sink>,
    pub approver: Arc<dyn Approver>,
    pub report: Arc<dyn Fn(Stage) + Send + Sync>,
    pub cancel: Arc<AtomicBool>,
    /// Headless: gate 1 answered "yes" without an interactive approver.
    pub assume_yes: bool,
    /// Skip building/installing (dry: propose + implement + judge only).
    pub no_install: bool,
}

pub struct Outcome { pub items: Vec<(Proposal, String, bool)>, pub installed: Option<PathBuf> }

/// Frontier backends decide for themselves; smaller local models need a human at gate 1.
pub fn is_smart(cfg: &Config) -> bool {
    match cfg.selfimprove.auto.as_str() {
        "always" => return true,
        "never" => return false,
        _ => {}
    }
    if matches!(cfg.llm.provider.as_deref(), Some("claude-code") | Some("anthropic")) { return true; }
    let m = cfg.llm.model.to_lowercase();
    cfg.selfimprove.smart_models.iter().any(|g| crate::permissions::glob_match(&g.to_lowercase(), &m))
}

/// The harness's own checkout: `[self] repo`, `$HARNESS_REPO`, cwd, or the directory the binary was built from.
pub fn locate_repo(cfg: &Config) -> Result<PathBuf> {
    let mut cands: Vec<PathBuf> = Vec::new();
    if let Some(r) = &cfg.selfimprove.repo { cands.push(PathBuf::from(r)); }
    if let Some(r) = std::env::var_os("HARNESS_REPO") { cands.push(PathBuf::from(r)); }
    if let Ok(c) = std::env::current_dir() { cands.push(c); }
    let exe = std::env::var_os("HARNESS_ORIG_EXE").map(PathBuf::from).or_else(|| std::env::current_exe().ok());
    if let Some(exe) = exe { if let Some(r) = exe.ancestors().nth(3) { cands.push(r.to_path_buf()); } }
    cands.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    for c in cands { if c.join("Cargo.toml").is_file() && c.join("harness.toml").is_file() && c.join("src/agent.rs").is_file() { return Ok(c.canonicalize()?); } }
    bail!("could not locate the harness source checkout — set [self] repo in harness.toml or HARNESS_REPO")
}

/// The binary a restart will exec (the installed one, not a temp copy).
pub fn installed_exe() -> Result<PathBuf> {
    std::env::var_os("HARNESS_ORIG_EXE").map(PathBuf::from).or_else(|| std::env::current_exe().ok()).context("cannot locate the harness executable")
}

pub fn slug(s: &str) -> String {
    let s: String = s.to_lowercase().chars().map(|c| if c.is_alphanumeric() { c } else { '-' }).collect();
    let s = s.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-");
    if s.len() > 40 { s[..40].trim_end_matches('-').to_string() } else if s.is_empty() { "improve".into() } else { s }
}

/// Pull the JSON array of proposals out of a model answer (bare, fenced, or embedded in prose).
pub fn parse_plan(text: &str, max_items: usize) -> Vec<Proposal> {
    let try_parse = |s: &str| -> Option<Vec<Proposal>> {
        if let Ok(v) = serde_json::from_str::<Vec<Proposal>>(s) { return Some(v); }
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(s) { if let Some(arr) = v.get("proposals").or_else(|| v.get("items")).or_else(|| v.get("plan")) { return serde_json::from_value(arr.clone()).ok(); } }
        None
    };
    let mut cands: Vec<String> = Vec::new();
    cands.push(text.trim().to_string());
    // fenced blocks
    let mut rest = text;
    while let Some(i) = rest.find("```") {
        let after = &rest[i + 3..];
        let body_start = after.find('\n').map(|n| n + 1).unwrap_or(0);
        if let Some(j) = after[body_start..].find("```") { cands.push(after[body_start..body_start + j].trim().to_string()); rest = &after[body_start + j + 3..]; } else { break; }
    }
    // outermost [ ... ] / { ... }
    if let (Some(a), Some(b)) = (text.find('['), text.rfind(']')) { if a < b { cands.push(text[a..=b].to_string()); } }
    if let (Some(a), Some(b)) = (text.find('{'), text.rfind('}')) { if a < b { cands.push(text[a..=b].to_string()); } }
    for c in cands {
        if let Some(mut v) = try_parse(&c) {
            v.retain(|p| !p.title.trim().is_empty());
            v.truncate(max_items.max(1));
            if !v.is_empty() { return v; }
        }
    }
    Vec::new()
}

fn sh(cmd: &str, cwd: &Path, secs: u64) -> Result<(bool, String)> {
    let o = std::process::Command::new("/bin/sh").arg("-c").arg(format!("timeout_() {{ perl -e 'alarm shift; exec @ARGV' \"$@\"; }}; timeout_ {secs} /bin/sh -c {}", shell_quote(cmd))).current_dir(cwd).output()?;
    Ok((o.status.success(), format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))))
}
async fn sh_async(cmd: String, cwd: PathBuf, secs: u64) -> Result<(bool, String)> { tokio::task::spawn_blocking(move || sh(&cmd, &cwd, secs)).await? }

fn tail(s: &str, n: usize) -> String { let v: Vec<&str> = s.lines().collect(); v[v.len().saturating_sub(n)..].join("\n") }

/// Replace `dest` with `src` atomically (write next to it, then rename) so a running process keeps its mapped inode.
pub fn install_binary(src: &Path, dest: &Path) -> Result<()> {
    if src.canonicalize().ok() == dest.canonicalize().ok() { return Ok(()); }
    let tmp = dest.with_extension(format!("new-{}", std::process::id()));
    std::fs::copy(src, &tmp).with_context(|| format!("copy {} → {}", src.display(), tmp.display()))?;
    #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)); }
    if let Err(e) = std::fs::rename(&tmp, dest) { let _ = std::fs::remove_file(&tmp); return Err(e).with_context(|| format!("replace {}", dest.display())); }
    Ok(())
}

const PROPOSE_RULES: &str = "SELF-IMPROVEMENT · PROPOSAL STAGE. The working directory is the source of the harness you are running in (a Rust project). \
You are READ-ONLY here: inspect README.md, docs/GAPS.md, TODO.md, the memory files (BRAIN.md lessons), src/ and evals/tasks; do not edit anything. \
Propose the most valuable, self-contained improvements that a coding agent can implement in one session (each: ≤ ~300 changed lines, no changes to \
src/main.rs, src/llm.rs, src/sandbox.rs, src/agent.rs, src/eval.rs unless unavoidable; prefer tools under src/tools/, the system prompt, config, TUI polish, new eval tasks). \
Every item must be verifiable: cargo build + cargo test must pass, ideally with a new unit test or eval task. \
FINAL ANSWER FORMAT — reply with ONLY a JSON array (no prose before or after): \
[{\"title\": \"short imperative title\", \"rationale\": \"why it matters (1-3 sentences)\", \"plan\": \"concrete steps for the implementer\", \"files\": [\"src/...\"], \"eval_task\": null}]";

fn implement_rules(branch: &str, p: &Proposal, run_eval: bool) -> String {
    format!(
"SELF-IMPROVEMENT MODE. You are editing your own harness (a Rust project) in a git worktree on branch `{branch}`. The working directory IS the repo root.
Approved improvement: {title}
Rationale: {rationale}
Plan: {plan}
Files likely involved: {files}
Ground rules:
- Read README.md and the relevant src/ files before changing anything.
- Do NOT edit src/main.rs, src/llm.rs, src/sandbox.rs, src/agent.rs or src/eval.rs unless the task explicitly requires it; prefer adding tools under src/tools/, adjusting the system prompt, config and evals/tasks.
- Register new tools in src/tools/mod.rs (Registry::defaults) and add unit tests.
- After edits: `cargo build --release` must succeed and `cargo test` must pass (use timeout_secs 900 for cargo commands; the first build is slow).
{eval}- Commit your work on this branch (git add -A && git commit) with a message that states the change{evalmsg}. Never merge into main; the arbiter does that. Do not touch other branches or worktrees.
- Finish with a short report: what changed, how it was verified.",
        title = p.title, rationale = p.rationale, plan = p.plan, files = if p.files.is_empty() { "(unknown)".to_string() } else { p.files.join(", ") },
        eval = if run_eval { "- Then run `./target/release/harness eval` (timeout_secs 1800) and report the score.\n" } else { "- Do not run the full eval suite; the arbiter does that after you commit.\n" },
        evalmsg = if run_eval { " and the eval result" } else { "" })
}

/// Run the whole loop. Stages are reported as they happen; the returned Outcome summarizes.
pub async fn run(job: Job) -> Result<Outcome> {
    let report = job.report.clone();
    let log = |s: String| report(Stage::Log(s));
    let cfg = &job.cfg;
    let sc = cfg.selfimprove.clone();
    let repo = locate_repo(cfg)?;
    log(format!("harness source: {}", repo.display()));
    // preflight: git repo, clean tree, main checked out
    let (ok, out) = sh("git rev-parse --is-inside-work-tree && git status --porcelain", &repo, 20)?;
    if !ok { bail!("{} is not a git repository", repo.display()); }
    if out.lines().count() > 1 { bail!("working tree at {} is dirty; commit or stash first:\n{}", repo.display(), tail(&out, 8)); }
    let (_, cur) = sh("git rev-parse --abbrev-ref HEAD", &repo, 20)?; let cur = cur.trim().to_string();
    if cur != "main" { bail!("{} has `{cur}` checked out; the loop merges into main — check out main first", repo.display()); }
    let smart = is_smart(cfg);
    log(format!("backend {} · gate 1: {}", cfg.llm.model, if smart { "automatic (frontier model)" } else { "user confirms" }));

    // ── stage 1: propose ────────────────────────────────────────────────────────
    let propose_prompt = if job.hint.trim().is_empty() {
        format!("Analyse this harness and propose up to {} improvements, ranked by value. Look at docs/GAPS.md, TODO.md, the README roadmap and the BRAIN lessons for known gaps. Answer with the JSON array only.", sc.max_items)
    } else {
        format!("Analyse this harness and propose up to {} improvements focused on: {}\nAnswer with the JSON array only.", sc.max_items, job.hint.trim())
    };
    let mut setup = RunSetup::new(cfg.clone(), repo.clone(), job.sink.clone(), job.approver.clone());
    setup.perm_mode = Some(crate::permissions::Mode::Plan);
    setup.prompt_extra = Some(PROPOSE_RULES.to_string());
    let prepared = prepare(setup).await?;
    log("proposing…".into());
    let (text, _) = prepared.run_once(&propose_prompt, &repo).await.context("proposal run failed")?;
    if job.cancel.load(Ordering::Relaxed) { bail!("cancelled"); }
    let items = parse_plan(&text, sc.max_items);
    if items.is_empty() { bail!("the model did not return a usable plan:\n{}", crate::llm::truncate_for_log(&text, 600)); }
    report(Stage::Plan { items: items.clone(), auto: smart });

    // ── gate 1 ──────────────────────────────────────────────────────────────────
    let approved: Vec<Proposal> = if smart {
        items.clone()
    } else if job.approver.interactive() {
        let mut opts = vec![QuestionOption { label: "Approve all".into(), description: format!("implement all {} items, one branch each", items.len()) }];
        for (i, p) in items.iter().enumerate() { opts.push(QuestionOption { label: format!("Only #{}", i + 1), description: p.title.clone() }); }
        opts.push(QuestionOption { label: "Cancel".into(), description: "do nothing".into() });
        let q = Question { question: format!("Self-improvement plan ({} items, listed above) — implement?  (free text: numbers to pick, e.g. \"1 3\")", items.len()), options: opts, allow_free_text: true, timeout_secs: None };
        match job.approver.question(q).await {
            Some(Answer { declined: true, .. }) | None => Vec::new(),
            Some(Answer { choice: Some(0), .. }) => items.clone(),
            Some(Answer { choice: Some(c), .. }) if c >= 1 && c <= items.len() => vec![items[c - 1].clone()],
            Some(Answer { text: Some(t), .. }) => { let picks: Vec<usize> = t.split(|c: char| !c.is_ascii_digit()).filter_map(|s| s.parse::<usize>().ok()).collect(); if picks.is_empty() && matches!(t.trim().to_lowercase().as_str(), "y" | "yes" | "all" | "ok") { items.clone() } else { items.iter().enumerate().filter(|(i, _)| picks.contains(&(i + 1))).map(|(_, p)| p.clone()).collect() } }
            _ => Vec::new(),
        }
    } else if job.assume_yes { items.clone() } else { log("no user available to confirm and --yes not given → plan only".into()); Vec::new() };
    if approved.is_empty() { report(Stage::Done { summary: "no improvements approved".into() }); return Ok(Outcome { items: vec![], installed: None }); }
    report(Stage::Approved(approved.clone()));

    // ── stage 2: implement + judge each item ────────────────────────────────────
    let mut results: Vec<(Proposal, String, bool)> = Vec::new();
    let mut merged_any = false;
    for p in &approved {
        if job.cancel.load(Ordering::Relaxed) { bail!("cancelled"); }
        let branch = format!("proposal/{}", slug(&p.title));
        let wt = std::env::temp_dir().join("harness-proposals").join(branch.replace('/', "__"));
        let _ = sh(&format!("git worktree remove --force {} 2>/dev/null; git branch -D {} 2>/dev/null", shell_quote(&wt.display().to_string()), shell_quote(&branch)), &repo, 60);
        std::fs::create_dir_all(wt.parent().unwrap())?;
        let (ok, out) = sh(&format!("git worktree add -q -b {} {} main", shell_quote(&branch), shell_quote(&wt.display().to_string())), &repo, 60)?;
        if !ok { results.push((p.clone(), format!("worktree failed: {}", tail(&out, 3)), false)); continue; }
        log(format!("▶ {} → {branch} ({})", p.title, wt.display()));
        let mut setup = RunSetup::new(cfg.clone(), wt.clone(), job.sink.clone(), job.approver.clone());
        setup.prompt_extra = Some(implement_rules(&branch, p, sc.skip_arbiter));
        // permission mode: what the front-end runs with (bypass/auto); the worktree is disposable
        let prepared = match prepare(setup).await { Ok(x) => x, Err(e) => { results.push((p.clone(), format!("prepare failed: {e:#}"), false)); continue; } };
        let task = format!("Implement this improvement: {}\n\n{}", p.title, p.plan);
        match prepared.run_once(&task, &wt).await {
            Ok((t, st)) => log(format!("agent finished ({} tool calls, {:.0}s): {}", st.tool_calls, st.wall_secs, crate::llm::truncate_for_log(t.lines().next().unwrap_or(""), 160))),
            Err(e) => { results.push((p.clone(), format!("agent failed: {e:#}"), false)); report(Stage::Item { title: p.title.clone(), branch: branch.clone(), merged: false, note: format!("agent failed: {e:#}") }); continue; }
        }
        // anything committed?
        let (_, ahead) = sh(&format!("git rev-list --count main..{}", shell_quote(&branch)), &repo, 30)?;
        if ahead.trim().parse::<u64>().unwrap_or(0) == 0 {
            // maybe left uncommitted work → commit it for the arbiter
            let (_, dirty) = sh("git status --porcelain", &wt, 30)?;
            if dirty.trim().is_empty() { let note = "no changes committed".to_string(); results.push((p.clone(), note.clone(), false)); report(Stage::Item { title: p.title.clone(), branch, merged: false, note }); continue; }
            let _ = sh(&format!("git add -A && git commit -q -m {}", shell_quote(&format!("improve: {}", p.title))), &wt, 60);
        }
        // judge
        let (merged, note) = if sc.skip_arbiter {
            log("build + test (arbiter skipped)…".into());
            let (b, bo) = sh_async("cargo build --release 2>&1 | tail -n 3".into(), wt.clone(), 1800).await?;
            let (t, to) = if b { sh_async("cargo test 2>&1 | tail -n 5".into(), wt.clone(), 1800).await? } else { (false, String::new()) };
            if b && t {
                let (m, mo) = sh(&format!("git merge --no-ff -m {} {}", shell_quote(&format!("improve: merge {branch}")), shell_quote(&branch)), &repo, 120)?;
                if !m { let _ = sh("git merge --abort", &repo, 30); }
                (m, if m { "build+tests ok · merged".into() } else { format!("merge failed: {}", tail(&mo, 3)) })
            } else { (false, format!("{}: {}", if !b { "build failed" } else { "tests failed" }, tail(if !b { &bo } else { &to }, 3))) }
        } else {
            log(format!("arbiter: judging {branch} ({} eval run(s) per side)…", sc.arbiter_runs));
            let (repo2, branch2, runs, rep) = (repo.clone(), branch.clone(), sc.arbiter_runs.max(1), report.clone());
            let v = tokio::task::spawn_blocking(move || { let mut lg = |s: &str| rep(Stage::Log(format!("  {s}"))); crate::arbiter::judge(&repo2, &branch2, runs, None, true, &mut lg) }).await?;
            match v {
                Ok(v) => (v.green, if v.green { "arbiter GREEN · merged".into() } else { format!("arbiter RED: {}", v.reasons.join("; ")) }),
                Err(e) => (false, format!("arbiter error: {e:#}")),
            }
        };
        if merged { merged_any = true; }
        results.push((p.clone(), note.clone(), merged));
        report(Stage::Item { title: p.title.clone(), branch, merged, note });
    }

    // ── stage 3: build + install ────────────────────────────────────────────────
    let mut installed = None;
    if merged_any && !job.no_install {
        if job.cancel.load(Ordering::Relaxed) { bail!("cancelled"); }
        log("building main (release, separate target dir)…".into());
        let tdir = repo.join("target").join("selfimprove");
        let (ok, out) = sh_async(format!("CARGO_TARGET_DIR={} cargo build --release 2>&1 | tail -n 3", shell_quote(&tdir.display().to_string())), repo.clone(), 2400).await?;
        if !ok { bail!("release build of main failed after merge:\n{}", tail(&out, 5)); }
        let exe = installed_exe()?;
        let src = tdir.join("release").join(if cfg!(windows) { "harness.exe" } else { "harness" });
        install_binary(&src, &exe)?;
        let (_, ver) = sh(&format!("{} --version", shell_quote(&exe.display().to_string())), &repo, 20)?;
        let summary = results.iter().filter(|r| r.2).map(|r| r.0.title.clone()).collect::<Vec<_>>().join(" · ");
        log(format!("installed {} ({})", exe.display(), ver.trim()));
        installed = Some(exe.clone());
        report(Stage::Installed { summary, exe, grace_secs: sc.restart_grace_secs });
    }
    let n_ok = results.iter().filter(|r| r.2).count();
    let summary = format!("{n_ok}/{} improvement(s) merged{}", results.len(), if installed.is_some() { " · new binary installed" } else if merged_any { " · not installed" } else { "" });
    report(Stage::Done { summary });
    Ok(Outcome { items: results, installed })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cfg() -> Config { toml::from_str("[llm]\nbase_url='http://x'\nmodel='m'\n[agent]\n").unwrap() }

    #[test]
    fn parse_plan_variants() {
        let bare = r#"[{"title":"A","rationale":"r","plan":"p","files":["src/x.rs"]},{"title":"B"}]"#;
        assert_eq!(parse_plan(bare, 5).len(), 2);
        let fenced = format!("Here is the plan:\n```json\n{bare}\n```\nthanks");
        let v = parse_plan(&fenced, 1); assert_eq!(v.len(), 1); assert_eq!(v[0].title, "A");
        let obj = r#"{"proposals":[{"title":"C","rationale":"x"}]}"#;
        assert_eq!(parse_plan(obj, 3)[0].title, "C");
        assert!(parse_plan("no json here", 3).is_empty());
        assert!(parse_plan(r#"[{"title":""}]"#, 3).is_empty());
    }

    #[test]
    fn smart_policy() {
        let mut c = cfg();
        c.llm.model = "qwen3.8-27b-mlx".into(); assert!(!is_smart(&c));
        c.llm.model = "claude-fable-5".into(); assert!(is_smart(&c));
        c.llm.model = "qwen".into(); c.llm.provider = Some("claude-code".into()); assert!(is_smart(&c));
        c.llm.provider = None; c.selfimprove.auto = "always".into(); assert!(is_smart(&c));
        c.llm.model = "claude-fable-5".into(); c.selfimprove.auto = "never".into(); assert!(!is_smart(&c));
    }

    #[test]
    fn slug_and_install() {
        assert_eq!(slug("Add a --json flag to `harness models`!"), "add-a-json-flag-to-harness-models");
        assert_eq!(slug("!!!"), "improve");
        let d = std::env::temp_dir().join(format!("harness-si-test-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let (a, b) = (d.join("a"), d.join("b"));
        std::fs::write(&a, "new").unwrap(); std::fs::write(&b, "old").unwrap();
        install_binary(&a, &b).unwrap();
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "new");
        assert!(std::fs::read_dir(&d).unwrap().count() == 2, "no temp file left behind");
        let _ = std::fs::remove_dir_all(&d);
    }
}
