//! Arena / best-of-n: run the same task on several models (or the same model several times) in
//! isolated git worktrees, then let a judge model pick the best result from the actual diffs.
//! Cheap to run locally, and the outcome is data: which model wins which kind of task, and a branch
//! per attempt that the arbiter can gate before anything reaches main.

use crate::config::Config;
use crate::events::{Event, Sink};
use anyhow::{bail, Context, Result};
use std::sync::Arc;

#[derive(Debug, Clone, serde::Serialize)]
pub struct Contender {
    pub model: String,
    pub label: String,
    pub branch: String,
    pub worktree: String,
    pub answer: String,
    pub diffstat: String,
    pub files_changed: usize,
    pub turns: usize,
    pub tool_calls: usize,
    pub wall_secs: f64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ArenaResult { pub task: String, pub contenders: Vec<Contender>, pub winner: Option<usize>, pub verdict: String }

fn slug(s: &str) -> String {
    let mut out: String = s.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    while out.contains("--") { out = out.replace("--", "-"); }
    out.trim_matches('-').chars().take(28).collect()
}

/// Run `task` once per entry in `models` (duplicates allowed: best-of-n on one model), each in its own
/// worktree, then judge. Every attempt keeps its branch so the user can inspect or merge it.
pub async fn run(cfg: &Config, workdir: &std::path::Path, task: &str, models: &[String], sink: Arc<dyn Sink>, yes: bool) -> Result<ArenaResult> {
    if models.len() < 2 { bail!("an arena needs at least two contenders (pass a model twice for best-of-2 on one model)"); }
    if models.len() > 6 { bail!("at most 6 contenders"); }
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() % 100000).unwrap_or(0);

    let mut jobs = Vec::new();
    for (i, model) in models.iter().enumerate() {
        let name = format!("arena-{stamp}-{i}-{}", slug(model));
        let wt = crate::worktree::create(workdir, &name, None, None).with_context(|| format!("creating a worktree for {model}"))?;
        let (mut cfg2, task2, sink2) = (cfg.clone(), task.to_string(), sink.clone());
        cfg2.llm.model = model.clone();
        let (model2, label) = (model.clone(), format!("{}#{i}", model));
        let branch = format!("wt/{name}");
        sink.emit(&Event::Assistant { text: format!("[arena] {label} → {} (branch {branch})", wt.display()) });
        jobs.push(tokio::spawn(async move {
            let approver: Arc<dyn crate::permissions::Approver> = Arc::new(crate::permissions::AutoApprover { yes });
            let prefix: Arc<dyn Sink> = Arc::new(crate::agent::PrefixSink { inner: sink2, prefix: format!("[{label}] "), info: None });
            let mut setup = crate::runner::RunSetup::new(cfg2, wt.clone(), prefix, approver);
            setup.prompt_extra = Some(format!("You are contender '{label}' in an arena: several agents solve the SAME task independently in separate git worktrees, and a judge compares the results. Do the task well and completely, commit nothing unless asked, and finish with a short summary of what you changed and how you verified it."));
            let started = std::time::Instant::now();
            let out = match crate::runner::prepare(setup).await {
                Ok(p) => p.run_once(&task2, &wt).await,
                Err(e) => Err(e),
            };
            let diffstat = crate::sandbox::run_shell("git add -A -N >/dev/null 2>&1; git diff --stat | tail -25", &wt, std::time::Duration::from_secs(30), 6000).await.map(|o| o.stdout.trim().to_string()).unwrap_or_default();
            let files_changed = crate::sandbox::run_shell("git diff --name-only | wc -l", &wt, std::time::Duration::from_secs(30), 2000).await.map(|o| o.stdout.trim().parse().unwrap_or(0)).unwrap_or(0);
            let (answer, turns, tool_calls, error) = match out {
                Ok((t, st)) => (t, st.turns, st.tool_calls, None),
                Err(e) => (String::new(), 0, 0, Some(format!("{e:#}"))),
            };
            Contender { model: model2, label, branch, worktree: wt.display().to_string(), answer, diffstat, files_changed, turns, tool_calls, wall_secs: started.elapsed().as_secs_f64(), error }
        }));
    }

    let mut contenders: Vec<Contender> = Vec::new();
    for j in jobs { match j.await { Ok(c) => contenders.push(c), Err(e) => sink.emit(&Event::Error { message: format!("arena contender panicked: {e}") }) } }

    let client = crate::llm::Client::new(&cfg.llm)?;
    let (winner, verdict) = judge(&client, task, &contenders).await;
    Ok(ArenaResult { task: task.to_string(), contenders, winner, verdict })
}

/// Compare the attempts and pick one. The judge sees each answer and diffstat, never the model names,
/// so it cannot vote for a brand.
pub async fn judge(client: &crate::llm::Client, task: &str, contenders: &[Contender]) -> (Option<usize>, String) {
    let ok: Vec<(usize, &Contender)> = contenders.iter().enumerate().filter(|(_, c)| c.error.is_none()).collect();
    if ok.is_empty() { return (None, "every contender failed".into()); }
    if ok.len() == 1 { return (Some(ok[0].0), "only one contender finished".into()); }
    let mut body = String::new();
    for (i, c) in &ok {
        body.push_str(&format!("\n## Attempt {i}\nfiles changed: {} · {} turns · {} tool calls · {:.0}s\ndiffstat:\n{}\n\nreport:\n{}\n", c.files_changed, c.turns, c.tool_calls, c.wall_secs, crate::llm::truncate_for_log(&c.diffstat, 1500), crate::llm::truncate_for_log(&c.answer, 3000)));
    }
    let system = "You judge attempts by different coding agents at the SAME task. Pick the one most likely to be correct, complete and maintainable: evidence of verification (tests run, output shown) beats confident prose; a focused diff beats a sprawling one; unfinished or unverified work loses. Reply with JSON only: {\"winner\": <attempt number>, \"reason\": \"<= 30 words\", \"ranking\": [<attempt numbers, best first>]}.";
    let user = format!("Task:\n{task}\n\nAttempts:\n{body}\nJSON:");
    let Ok((reply, _)) = client.role("judge").chat(&[crate::llm::Message::system(system), crate::llm::Message::user(user)], &[]).await else {
        return (Some(ok[0].0), "judge model unavailable — showing the first successful attempt".into());
    };
    let text = reply.text();
    let Some(json) = crate::memory::extract_json(&text) else { return (Some(ok[0].0), format!("judge returned no JSON: {}", crate::llm::truncate_for_log(&text, 200))) };
    let v: serde_json::Value = serde_json::from_str(&json).unwrap_or_default();
    let idx = v["winner"].as_u64().map(|n| n as usize).filter(|i| contenders.get(*i).map(|c| c.error.is_none()).unwrap_or(false));
    let reason = v["reason"].as_str().unwrap_or("").trim().to_string();
    match idx { Some(i) => (Some(i), reason), None => (Some(ok[0].0), format!("judge did not pick a valid attempt ({reason})")) }
}

/// Human-readable summary (CLI/TUI).
pub fn render(r: &ArenaResult) -> Vec<String> {
    let mut lines = vec![format!("Arena — {} contenders · task: {}", r.contenders.len(), crate::llm::truncate_for_log(&r.task, 90))];
    for (i, c) in r.contenders.iter().enumerate() {
        let mark = if r.winner == Some(i) { "★" } else { " " };
        let status = match &c.error { Some(e) => format!("failed: {}", crate::llm::truncate_for_log(e, 60)), None => format!("{} file(s), {} turns, {} tool calls, {:.0}s", c.files_changed, c.turns, c.tool_calls, c.wall_secs) };
        lines.push(format!(" {mark} {:<2} {:<28} {}", i, crate::llm::truncate_for_log(&c.model, 28), status));
        lines.push(format!("      branch {} · {}", c.branch, c.worktree));
    }
    lines.push(format!("verdict: {}", r.verdict));
    if let Some(w) = r.winner {
        let c = &r.contenders[w];
        lines.push(format!("merge the winner with:  git merge {}   (or inspect: git diff main..{})", c.branch, c.branch));
        lines.push("clean up the rest with: git worktree list / git worktree remove <path>".into());
    }
    lines
}

/// Parse `--models a,b,c`, `a b c`, or `model x3` (best-of-3 on one model). `.` means the configured model.
pub fn parse_models(spec: &str, default_model: &str) -> Vec<String> {
    let rep = regex::Regex::new(r"(?i)^(.*?)\s*[x×*]\s*(\d+)$").ok();
    let mut out = Vec::new();
    for part in spec.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
        // "<model> x3" / "<model>×3" — the same contender several times
        if let Some(c) = rep.as_ref().and_then(|r| r.captures(part)) {
            let name = c.get(1).map(|m| m.as_str().trim()).unwrap_or("");
            let n: usize = c.get(2).and_then(|m| m.as_str().parse().ok()).unwrap_or(1);
            let name = if name.is_empty() || name == "." { default_model } else { name };
            for _ in 0..n.clamp(1, 6) { out.push(name.to_string()); }
            continue;
        }
        for name in part.split_whitespace() {
            out.push(if name == "." { default_model.to_string() } else { name.to_string() });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_specs() {
        assert_eq!(parse_models("a,b,c", "d"), vec!["a", "b", "c"]);
        assert_eq!(parse_models("qwen3.8-27b-mlx x3", "d"), vec!["qwen3.8-27b-mlx"; 3]);
        assert_eq!(parse_models(".,other", "main-model"), vec!["main-model", "other"]);
        assert!(parse_models("", "d").is_empty());
    }

    #[test]
    fn rendering_marks_the_winner() {
        let c = |m: &str, err: Option<&str>| Contender { model: m.into(), label: m.into(), branch: format!("wt/{m}"), worktree: format!("/tmp/{m}"), answer: "done".into(), diffstat: "1 file".into(), files_changed: 1, turns: 2, tool_calls: 3, wall_secs: 4.0, error: err.map(String::from) };
        let r = ArenaResult { task: "t".into(), contenders: vec![c("a", None), c("b", Some("boom"))], winner: Some(0), verdict: "a verified its work".into() };
        let out = render(&r).join("\n");
        assert!(out.contains("★"), "{out}");
        assert!(out.contains("failed: boom"), "{out}");
        assert!(out.contains("git merge wt/a"), "{out}");
    }
}
