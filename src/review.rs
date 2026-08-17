//! `harness review`: review a pull request or the working branch, report structured findings, and
//! optionally post them to the PR or fix them. The agent does the reading; this module supplies the
//! diff, the house rules and the plumbing back to GitHub.
//!
//! House rules come from `.harness/review-rules.md` (or REVIEW.md) so a team's review taste lives in
//! the repository instead of in a prompt someone pasted once.

use crate::config::Config;
use anyhow::{bail, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub struct Options {
    pub pr: Option<String>,
    pub base: Option<String>,
    pub comment: bool,
    pub fix: bool,
    pub max_turns: usize,
    pub yes: bool,
}

/// The project's review rules, if it has any.
pub fn house_rules(workdir: &Path) -> Option<(PathBuf, String)> {
    for name in [".harness/review-rules.md", "REVIEW.md", ".github/REVIEW.md", ".harness/REVIEW.md"] {
        let p = workdir.join(name);
        if let Ok(t) = std::fs::read_to_string(&p) { if !t.trim().is_empty() { return Some((p, t)); } }
    }
    None
}

/// The diff to review, and a human label for it.
pub async fn collect_diff(workdir: &Path, opts: &Options) -> Result<(String, String)> {
    let sh = |cmd: String| async move { crate::sandbox::run_shell(&cmd, workdir, Duration::from_secs(120), 400_000).await };
    if let Some(pr) = &opts.pr {
        let o = sh(format!("gh pr diff {} 2>&1", crate::security::shell_quote(pr))).await?;
        if !o.success() || o.stdout.trim().is_empty() { bail!("gh pr diff {pr} failed: {}{}", o.stdout.trim(), o.stderr.trim()); }
        return Ok((o.stdout, format!("pull request {pr}")));
    }
    let base = match &opts.base {
        Some(b) => b.clone(),
        None => {
            let o = sh("git symbolic-ref refs/remotes/origin/HEAD 2>/dev/null | sed 's@.*/@@' || echo main".into()).await?;
            let b = o.stdout.trim().to_string();
            if b.is_empty() { "main".into() } else { b }
        }
    };
    let o = sh(format!("git diff {}...HEAD 2>/dev/null", crate::security::shell_quote(&base))).await?;
    let diff = if o.success() && !o.stdout.trim().is_empty() { o.stdout } else { sh("git diff HEAD".into()).await?.stdout };
    if diff.trim().is_empty() { bail!("nothing to review (no diff against {base}, and the working tree is clean)"); }
    Ok((diff, format!("changes against {base}")))
}

/// The findings the agent wrote with `report_findings`, if any.
pub fn latest_findings(workdir: &Path) -> Vec<Value> {
    let p = workdir.join(".harness").join("findings").join("latest.json");
    let Ok(text) = std::fs::read_to_string(p) else { return vec![] };
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    v["findings"].as_array().cloned().unwrap_or_default()
}

/// One markdown block per finding, in the shape a reviewer would leave on a PR.
pub fn render_markdown(findings: &[Value], label: &str) -> String {
    if findings.is_empty() { return format!("### harness review — {label}\n\nNo findings.\n"); }
    let mut out = format!("### harness review — {label}\n\n{} finding(s):\n", findings.len());
    for f in findings {
        let (file, line) = (f["file"].as_str().unwrap_or(""), f["line"].as_u64());
        let where_ = match line { Some(l) => format!("`{file}:{l}`"), None => format!("`{file}`") };
        out.push_str(&format!("\n**{}** · {} — {}\n", f["severity"].as_str().unwrap_or("medium"), where_, f["title"].as_str().unwrap_or("")));
        if let Some(s) = f["summary"].as_str() { out.push_str(&format!("\n{s}\n")); }
        if let Some(s) = f["failure_scenario"].as_str() { out.push_str(&format!("\n_How it fails:_ {s}\n")); }
        if let Some(s) = f["suggested_fix"].as_str() { out.push_str(&format!("\n_Suggested fix:_ {s}\n")); }
    }
    out.push_str("\n_Posted by `harness review`._\n");
    out
}

/// Review, then optionally comment and/or fix. Returns (findings, markdown).
pub async fn run(cfg: &Config, workdir: &Path, opts: Options) -> Result<(Vec<Value>, String)> {
    let (diff, label) = collect_diff(workdir, &opts).await?;
    let rules = house_rules(workdir);
    let mut prompt = format!(
"Review the following {label} as a careful senior engineer on this codebase.\n\n\
Work from the diff, but read the surrounding files before judging: a hunk out of context lies.\n\
Report only what is actually wrong — correctness bugs, broken error handling, race conditions, security \
holes, resource leaks, API misuse, tests that assert nothing — plus genuinely valuable simplifications. \
Do not report style preferences, and do not invent problems to look thorough. For each finding give the \
file and line, what breaks, and the concrete input or state that triggers it.\n\n\
Finish by calling report_findings once with everything you found (an empty list is a valid, good answer).\n\n");
    if let Some((p, r)) = &rules { prompt.push_str(&format!("House rules from {} — apply them:\n{}\n\n", p.display(), crate::llm::truncate_for_log(r, 4000))); }
    prompt.push_str(&format!("```diff\n{}\n```\n", crate::sandbox::truncate_middle(&diff, 120_000)));

    let sink: std::sync::Arc<dyn crate::events::Sink> = std::sync::Arc::new(crate::events::StderrSink { verbose: false });
    let approver: std::sync::Arc<dyn crate::permissions::Approver> = std::sync::Arc::new(crate::permissions::AutoApprover { yes: opts.yes });
    let mut setup = crate::runner::RunSetup::new(cfg.clone(), workdir.to_path_buf(), sink.clone(), approver.clone());
    setup.cfg.agent.max_turns = opts.max_turns;
    if !opts.fix { setup.perm_mode = Some(crate::permissions::Mode::Plan); } // a review does not edit
    setup.session_id = Some(format!("review-{}", crate::scheduler::now()));
    let prepared = crate::runner::prepare(setup).await?;
    let _ = prepared.run_once(&prompt, workdir).await?;
    let findings = latest_findings(workdir);
    let markdown = render_markdown(&findings, &label);

    if opts.comment {
        if let Some(pr) = &opts.pr {
            let file = std::env::temp_dir().join(format!("harness-review-{}.md", std::process::id()));
            std::fs::write(&file, &markdown)?;
            let cmd = format!("gh pr comment {} --body-file {}", crate::security::shell_quote(pr), crate::security::shell_quote(&file.display().to_string()));
            let o = crate::sandbox::run_shell(&cmd, workdir, Duration::from_secs(60), 8000).await?;
            let _ = std::fs::remove_file(&file);
            if !o.success() { bail!("posting the comment failed: {}{}", o.stdout.trim(), o.stderr.trim()); }
            eprintln!("· posted the review to PR {pr}");
        } else { bail!("--comment needs --pr <number>"); }
    }

    if opts.fix && !findings.is_empty() {
        let list = findings.iter().map(|f| format!("- {} {}:{} — {}: {}",
            f["severity"].as_str().unwrap_or("medium"), f["file"].as_str().unwrap_or(""), f["line"].as_u64().unwrap_or(0),
            f["title"].as_str().unwrap_or(""), f["summary"].as_str().unwrap_or(""))).collect::<Vec<_>>().join("\n");
        let fix_prompt = format!(
"Fix these review findings in this repository. Change only what a finding calls for; if one is wrong or \
not worth fixing, say so instead of changing code. Run the tests afterwards and report what you changed.\n\n{list}");
        let mut setup = crate::runner::RunSetup::new(cfg.clone(), workdir.to_path_buf(), sink, approver);
        setup.cfg.agent.max_turns = opts.max_turns;
        setup.session_id = Some(format!("review-fix-{}", crate::scheduler::now()));
        let prepared = crate::runner::prepare(setup).await?;
        let out = prepared.run_once(&fix_prompt, workdir).await?;
        eprintln!("\n{}", out.0);
    }
    Ok((findings, markdown))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn renders_findings_like_a_reviewer() {
        let f = vec![json!({"file": "src/a.rs", "line": 12, "severity": "high", "title": "unchecked unwrap", "summary": "panics on empty input", "failure_scenario": "POST /x with an empty body", "suggested_fix": "use ok_or_else"})];
        let md = render_markdown(&f, "pull request 7");
        assert!(md.contains("harness review — pull request 7"));
        assert!(md.contains("`src/a.rs:12`") && md.contains("**high**"));
        assert!(md.contains("How it fails:") && md.contains("Suggested fix:"));
        assert!(render_markdown(&[], "x").contains("No findings"));
    }

    #[test]
    fn finds_house_rules() {
        let d = std::env::temp_dir().join(format!("harness-review-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join(".harness")).unwrap();
        assert!(house_rules(&d).is_none());
        std::fs::write(d.join(".harness/review-rules.md"), "never approve a TODO").unwrap();
        let (p, t) = house_rules(&d).unwrap();
        assert!(p.ends_with("review-rules.md") && t.contains("TODO"));
        let _ = std::fs::remove_dir_all(&d);
    }
}
