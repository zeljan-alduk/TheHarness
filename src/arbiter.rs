//! Arbiter: judge a `proposal/*` branch against `main` with the eval suite, and merge on green.
//! 1) build + test the branch in its worktree, 2) run the eval N times with the branch's own binary,
//! 3) run (or load cached) baseline eval for main, 4) verdict = tests pass ∧ mean score not lower ∧
//! no task that always passed on main now always fails, 5) optionally merge.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunSummary { pub passed: usize, pub total: usize, pub per_task: BTreeMap<String, bool>, pub wall_secs: f64, pub tokens: u64 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Verdict { pub branch: String, pub base_sha: String, pub tests_ok: bool, pub baseline: Vec<RunSummary>, pub proposal: Vec<RunSummary>, pub green: bool, pub reasons: Vec<String> }

fn sh(cmd: &str, cwd: &Path, secs: u64) -> Result<(bool, String)> {
    let o = std::process::Command::new("/bin/sh").arg("-c").arg(format!("timeout_() {{ perl -e 'alarm shift; exec @ARGV' \"$@\"; }}; timeout_ {secs} /bin/sh -c {}", shell_quote(cmd))).current_dir(cwd).output()?;
    Ok((o.status.success(), format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))))
}
fn shell_quote(s: &str) -> String { format!("'{}'", s.replace('\'', "'\\''")) }

pub fn parse_report(path: &Path) -> Result<RunSummary> {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    let mut per_task = BTreeMap::new();
    for r in v["results"].as_array().cloned().unwrap_or_default() { per_task.insert(r["name"].as_str().unwrap_or("").to_string(), r["passed"].as_bool().unwrap_or(false)); }
    Ok(RunSummary { passed: v["passed"].as_u64().unwrap_or(0) as usize, total: v["total"].as_u64().unwrap_or(0) as usize, per_task, wall_secs: v["total_wall_secs"].as_f64().unwrap_or(0.0), tokens: v["total_prompt_tokens"].as_u64().unwrap_or(0) + v["total_completion_tokens"].as_u64().unwrap_or(0) })
}

fn run_eval(bin: &Path, cwd: &Path, filter: Option<&str>, out: &Path) -> Result<RunSummary> {
    let f = filter.map(|f| format!(" -f {}", shell_quote(f))).unwrap_or_default();
    let cmd = format!("{}{} eval --out {} 2>&1 | tail -3", shell_quote(&bin.display().to_string()), f, shell_quote(&out.display().to_string()));
    let (_ok, log) = sh(&cmd, cwd, 3 * 3600)?; // eval exits 1 when not all pass; that's fine
    parse_report(out).with_context(|| format!("no eval report at {} — eval output: {}", out.display(), crate::llm::truncate_for_log(&log, 400)))
}

fn mean(runs: &[RunSummary]) -> f64 { if runs.is_empty() { 0.0 } else { runs.iter().map(|r| if r.total == 0 { 0.0 } else { r.passed as f64 / r.total as f64 }).sum::<f64>() / runs.len() as f64 } }
fn always(runs: &[RunSummary], task: &str, val: bool) -> bool { !runs.is_empty() && runs.iter().all(|r| r.per_task.get(task).copied() == Some(val)) }

pub fn cache_dir() -> PathBuf { PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config/harness/arbiter") }

pub fn judge(repo: &Path, branch: &str, runs: usize, filter: Option<&str>, merge: bool, log: &mut dyn FnMut(&str)) -> Result<Verdict> {
    let (ok, _) = sh(&format!("git rev-parse --verify {}", shell_quote(branch)), repo, 30)?;
    if !ok { bail!("branch {branch} does not exist"); }
    let (_, base_sha) = sh("git rev-parse --short main", repo, 30)?; let base_sha = base_sha.trim().to_string();
    // worktree
    let wt = std::env::temp_dir().join("harness-proposals").join(branch.replace('/', "__"));
    if !wt.join("Cargo.toml").exists() {
        std::fs::create_dir_all(wt.parent().unwrap())?;
        let (ok, out) = sh(&format!("git worktree add {} {}", shell_quote(&wt.display().to_string()), shell_quote(branch)), repo, 120)?;
        if !ok { bail!("worktree add failed: {out}"); }
    }
    log(&format!("worktree {}", wt.display()));
    // build + test
    log("building proposal (cargo build --release)…");
    let (ok, out) = sh("cargo build --release 2>&1 | tail -3", &wt, 1800)?; if !ok { bail!("proposal build failed:\n{out}"); }
    log("testing proposal (cargo test)…");
    let (tests_ok, tout) = sh("cargo test 2>&1 | tail -5", &wt, 1800)?;
    log(&format!("tests: {}", if tests_ok { "ok" } else { "FAILED" }));
    let mut reasons = Vec::new();
    if !tests_ok { reasons.push(format!("cargo test failed on the proposal: {}", crate::llm::truncate_for_log(tout.trim(), 300))); }
    // proposal evals
    let mut proposal = Vec::new();
    for i in 0..runs {
        log(&format!("proposal eval run {}/{}…", i + 1, runs));
        let out = wt.join("target").join(format!("arbiter-proposal-{i}.json"));
        let r = run_eval(&wt.join("target/release/harness"), &wt, filter, &out)?;
        log(&format!("  → {}/{} ({:.0}s)", r.passed, r.total, r.wall_secs));
        proposal.push(r);
    }
    // baseline (cached per main sha + filter + task count)
    std::fs::create_dir_all(cache_dir())?;
    let key = format!("baseline-{base_sha}-{}-{}.json", filter.unwrap_or("all").replace('/', "_"), proposal.first().map(|r| r.total).unwrap_or(0));
    let cache = cache_dir().join(&key);
    let mut baseline: Vec<RunSummary> = std::fs::read_to_string(&cache).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    if baseline.len() < runs {
        let (ok, _) = sh("cargo build --release 2>&1 | tail -1", repo, 1800)?; if !ok { bail!("baseline build failed"); }
        for i in baseline.len()..runs {
            log(&format!("baseline (main @ {base_sha}) eval run {}/{}…", i + 1, runs));
            let out = repo.join("target").join(format!("arbiter-baseline-{i}.json"));
            let r = run_eval(&repo.join("target/release/harness"), repo, filter, &out)?;
            log(&format!("  → {}/{} ({:.0}s)", r.passed, r.total, r.wall_secs));
            baseline.push(r);
        }
        std::fs::write(&cache, serde_json::to_string_pretty(&baseline)?)?;
    } else { log(&format!("baseline: cached ({} run(s) for main @ {base_sha})", baseline.len())); }
    // verdict
    let (mb, mp) = (mean(&baseline), mean(&proposal));
    if mp + 1e-9 < mb { reasons.push(format!("mean score dropped {:.1}% → {:.1}%", mb * 100.0, mp * 100.0)); }
    let tasks: std::collections::BTreeSet<String> = baseline.iter().chain(proposal.iter()).flat_map(|r| r.per_task.keys().cloned()).collect();
    for t in &tasks { if always(&baseline, t, true) && always(&proposal, t, false) { reasons.push(format!("regression: {t} always passed on main, always fails on the proposal")); } }
    let green = tests_ok && reasons.is_empty();
    // report table
    log(""); log(&format!("{:<24} {:>9} {:>9}", "task", "main", "proposal"));
    for t in &tasks {
        let pb = baseline.iter().filter(|r| r.per_task.get(t) == Some(&true)).count(); let pp = proposal.iter().filter(|r| r.per_task.get(t) == Some(&true)).count();
        log(&format!("{:<24} {:>5}/{:<3} {:>5}/{:<3}{}", t, pb, baseline.len(), pp, proposal.len(), if pb == baseline.len() && pp == 0 && !proposal.is_empty() { "  ◄ regression" } else if pb == 0 && pp == proposal.len() && !baseline.is_empty() { "  ▲ improvement" } else { "" }));
    }
    log(&format!("mean score  main {:.1}%  proposal {:.1}%  · tests {}", mb * 100.0, mp * 100.0, if tests_ok { "ok" } else { "FAILED" }));
    log(&format!("verdict: {}", if green { "GREEN ✓" } else { "RED ✗" }));
    for r in &reasons { log(&format!("  - {r}")); }
    if green && merge {
        let (ok, out) = sh(&format!("git merge --no-ff -m {} {}", shell_quote(&format!("arbiter: merge {branch} (eval {:.0}% → {:.0}%)", mb * 100.0, mp * 100.0)), shell_quote(branch)), repo, 120)?;
        log(if ok { "merged into main ✓" } else { "merge FAILED (conflicts?) — resolve manually" }); if !ok { log(&out); }
    }
    Ok(Verdict { branch: branch.into(), base_sha, tests_ok, baseline, proposal, green, reasons })
}
