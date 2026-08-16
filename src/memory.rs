//! Persistent memory: MEMORY.md (settings/preferences/ideas), WORKFLOWS.md (recipes) and BRAIN.md
//! (what the agent has learned about the user, projects and how to do things). All three are plain
//! markdown the user can edit; the agent edits them through the `memory` tool and a post-run
//! reflection step. Files live in `~/.config/harness/` by default.

use crate::llm::{Client, Message};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const FILES: [&str; 3] = ["MEMORY.md", "WORKFLOWS.md", "BRAIN.md"];

const MEMORY_TEMPLATE: &str = "# MEMORY — global settings, preferences, ideas\n\nEdited by you and by the agent (`memory` tool). Loaded into every session. Keep entries short and durable.\n\n## Settings\n\n## Preferences\n\n## Ideas\n";
const WORKFLOWS_TEMPLATE: &str = "# WORKFLOWS — reusable recipes\n\nReference a workflow by its heading from MEMORY.md/BRAIN.md or ask for it by name. Format: `## <name>` then numbered steps.\n";
const BRAIN_TEMPLATE: &str = "# BRAIN — what the agent has learned\n\nGrows over time from reflection after runs; consolidated when it gets long. Facts about the user, the projects, how to do things here, and lessons from mistakes.\n\n## User\n\n## Projects\n\n## How-to\n\n## Lessons\n";

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MemoryConfig {
    #[serde(default = "d_true")]
    pub enabled: bool,
    /// Directory holding MEMORY.md / WORKFLOWS.md / BRAIN.md (default ~/.config/harness)
    #[serde(default)]
    pub dir: Option<String>,
    /// After a run with at least this many tool calls, ask the model what to remember.
    #[serde(default = "d_true")]
    pub auto_reflect: bool,
    #[serde(default = "d_min_calls")]
    pub reflect_min_tool_calls: usize,
    /// Max characters of each file injected into the system prompt.
    #[serde(default = "d_inject")]
    pub max_inject_chars: usize,
    /// When a file exceeds this many characters, ask the model to consolidate it.
    #[serde(default = "d_consolidate")]
    pub consolidate_over_chars: usize,
}
fn d_true() -> bool { true }
fn d_min_calls() -> usize { 2 }
fn d_inject() -> usize { 6000 }
fn d_consolidate() -> usize { 14000 }
impl Default for MemoryConfig {
    fn default() -> Self { Self { enabled: true, dir: None, auto_reflect: true, reflect_min_tool_calls: d_min_calls(), max_inject_chars: d_inject(), consolidate_over_chars: d_consolidate() } }
}

#[derive(Debug, Clone)]
pub struct MemoryStore { pub dir: PathBuf, pub cfg: MemoryConfig }

impl MemoryStore {
    pub fn open(cfg: &MemoryConfig) -> Result<Self> {
        let dir = match std::env::var_os("HARNESS_MEMORY_DIR") {
            Some(d) => PathBuf::from(d),
            None => match &cfg.dir {
                Some(d) => PathBuf::from(shellexpand(d)),
                None => crate::setup::config_dir(),
            },
        };
        std::fs::create_dir_all(&dir)?;
        let s = Self { dir, cfg: cfg.clone() };
        s.ensure_templates()?;
        Ok(s)
    }
    /// An isolated store (evals, tests) that never touches the user's memory.
    pub fn scratch(dir: &Path, cfg: &MemoryConfig) -> Result<Self> {
        std::fs::create_dir_all(dir)?;
        let s = Self { dir: dir.to_path_buf(), cfg: cfg.clone() };
        s.ensure_templates()?;
        Ok(s)
    }
    fn ensure_templates(&self) -> Result<()> {
        for (f, t) in [("MEMORY.md", MEMORY_TEMPLATE), ("WORKFLOWS.md", WORKFLOWS_TEMPLATE), ("BRAIN.md", BRAIN_TEMPLATE)] {
            let p = self.dir.join(f);
            if !p.exists() { std::fs::write(&p, t)?; }
        }
        Ok(())
    }
    pub fn path(&self, file: &str) -> Result<PathBuf> {
        let f = canonical_name(file)?;
        Ok(self.dir.join(f))
    }
    pub fn read(&self, file: &str) -> Result<String> {
        Ok(std::fs::read_to_string(self.path(file)?).unwrap_or_default())
    }
    pub fn write(&self, file: &str, content: &str) -> Result<()> {
        let p = self.path(file)?;
        if p.exists() { let _ = std::fs::copy(&p, p.with_extension("md.bak")); }
        std::fs::write(&p, content)?;
        Ok(())
    }
    /// Append a bullet under `## section` (created at the end if missing). Returns false if the exact line already exists.
    pub fn append(&self, file: &str, section: &str, text: &str) -> Result<bool> {
        let mut doc = self.read(file)?;
        let line = format!("- {}", text.trim().trim_start_matches("- ").replace('\n', " "));
        if doc.lines().any(|l| l.trim() == line) { return Ok(false); }
        let heading = format!("## {}", section.trim().trim_start_matches('#').trim());
        let lines: Vec<&str> = doc.lines().collect();
        let mut out: Vec<String> = Vec::new();
        let mut inserted = false;
        let mut i = 0;
        while i < lines.len() {
            out.push(lines[i].to_string());
            if !inserted && lines[i].trim().eq_ignore_ascii_case(&heading) {
                // find end of this section: next "## " heading or EOF; insert before trailing blank lines
                let mut j = i + 1;
                while j < lines.len() && !lines[j].starts_with("## ") { j += 1; }
                let mut body: Vec<String> = lines[i + 1..j].iter().map(|s| s.to_string()).collect();
                while body.last().map(|l| l.trim().is_empty()).unwrap_or(false) { body.pop(); }
                if body.is_empty() { out.push(String::new()); }
                out.extend(body);
                out.push(line.clone());
                out.push(String::new());
                inserted = true;
                i = j;
                continue;
            }
            i += 1;
        }
        if !inserted {
            if !doc.ends_with('\n') && !doc.is_empty() { out.push(String::new()); }
            out.push(heading); out.push(String::new()); out.push(line); out.push(String::new());
        }
        doc = out.join("\n");
        if !doc.ends_with('\n') { doc.push('\n'); }
        std::fs::write(self.path(file)?, doc)?;
        Ok(true)
    }
    /// Remove lines containing `needle` (case-insensitive). Returns how many were removed.
    pub fn remove(&self, file: &str, needle: &str) -> Result<usize> {
        let doc = self.read(file)?;
        let n = needle.to_lowercase();
        let kept: Vec<&str> = doc.lines().filter(|l| !(l.trim_start().starts_with("- ") && l.to_lowercase().contains(&n))).collect();
        let removed = doc.lines().count() - kept.len();
        if removed > 0 { std::fs::write(self.path(file)?, kept.join("\n") + "\n")?; }
        Ok(removed)
    }

    /// Bump the per-project familiarity ledger in BRAIN.md ("## Projects").
    pub fn touch_project(&self, workdir: &Path) -> Result<()> {
        let key = workdir.display().to_string();
        let today = today_iso();
        let doc = self.read("BRAIN.md")?;
        let mut found = false;
        let out: Vec<String> = doc.lines().map(|l| {
            if l.trim_start().starts_with("- ") && l.contains(&key) && l.contains("sessions:") {
                found = true;
                // "- <path> · sessions: N · last: DATE · rest"
                let mut parts: Vec<String> = l.split(" · ").map(|s| s.to_string()).collect();
                for p in parts.iter_mut() {
                    if let Some(n) = p.trim().strip_prefix("sessions:") { let n: u64 = n.trim().parse().unwrap_or(0); *p = format!("sessions: {}", n + 1); }
                    else if p.trim().starts_with("last:") { *p = format!("last: {today}"); }
                }
                parts.join(" · ")
            } else { l.to_string() }
        }).collect();
        if found { std::fs::write(self.path("BRAIN.md")?, out.join("\n") + "\n")?; }
        else { self.append("BRAIN.md", "Projects", &format!("{key} · sessions: 1 · last: {today}"))?; }
        Ok(())
    }

    /// Where clipboard pastes (images, files) are stored: <dir>/pastes/YYYY-MM-DD/
    pub fn pastes_dir(&self) -> PathBuf { self.dir.join("pastes").join(today_iso()) }

    /// Save pasted bytes into the pastes dir with a timestamped name; returns the path.
    pub fn save_paste(&self, ext: &str, bytes: &[u8]) -> Result<PathBuf> {
        let dir = self.pastes_dir();
        std::fs::create_dir_all(&dir)?;
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) % 86400;
        let mut p = dir.join(format!("paste-{:02}{:02}{:02}.{ext}", secs / 3600, (secs / 60) % 60, secs % 60));
        let mut n = 1;
        while p.exists() { p = dir.join(format!("paste-{:02}{:02}{:02}-{n}.{ext}", secs / 3600, (secs / 60) % 60, secs % 60)); n += 1; }
        std::fs::write(&p, bytes)?;
        Ok(p)
    }

    /// The most recent pasted files (newest first), for the system prompt.
    pub fn recent_pastes(&self, n: usize) -> Vec<PathBuf> {
        let mut all: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
        if let Ok(days) = std::fs::read_dir(self.dir.join("pastes")) {
            for d in days.flatten() { if let Ok(files) = std::fs::read_dir(d.path()) { for f in files.flatten() { if let Ok(md) = f.metadata() { all.push((md.modified().unwrap_or(std::time::UNIX_EPOCH), f.path())); } } } }
        }
        all.sort_by(|a, b| b.0.cmp(&a.0));
        all.into_iter().take(n).map(|(_, p)| p).collect()
    }

    /// The block injected into the system prompt.
    pub fn prompt_block(&self, workdir: &Path) -> String {
        let cap = self.cfg.max_inject_chars;
        let mut s = String::from("\n\n# Persistent memory\nYou have three markdown memory files (below), loaded every session. Use the `memory` tool to record DURABLE, NON-OBVIOUS facts: user preferences and settings → MEMORY.md; reusable multi-step recipes → WORKFLOWS.md; what you learned about the user, this project, how things work here, and lessons from mistakes → BRAIN.md. Never store secrets, trivia, or anything derivable from the code. Prefer editing an existing bullet over adding a near-duplicate.\n");
        for f in FILES {
            let body = self.read(f).unwrap_or_default();
            let body = if body.chars().count() > cap { format!("{}\n…[truncated; use memory show to read all]", body.chars().take(cap).collect::<String>()) } else { body };
            s.push_str(&format!("\n--- {} ({}) ---\n{}\n", f, self.dir.join(f).display(), body.trim_end()));
        }
        // Pasted files: always tell the model where they live
        let pastes = self.dir.join("pastes");
        s.push_str(&format!("\n--- pasted files ---\nImages/files the user pastes into the prompt are saved under {} (one folder per day). Attached images are referenced in the user message as [image #N: <full path>]. You can read_file / view_image / bash them by that path.", pastes.display()));
        let recent = self.recent_pastes(5);
        if !recent.is_empty() { s.push_str("\nMost recent pastes: "); s.push_str(&recent.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")); }
        s.push('\n');
        // Project instructions file, if any (like CLAUDE.md)
        for name in ["HARNESS.md", ".harness/HARNESS.md"] {
            let p = workdir.join(name);
            if let Ok(t) = std::fs::read_to_string(&p) {
                let t = if t.chars().count() > cap { t.chars().take(cap).collect::<String>() + "\n…[truncated]" } else { t };
                s.push_str(&format!("\n--- project instructions ({}) ---\n{}\n", p.display(), t.trim_end()));
                break;
            }
        }
        s
    }

    /// Ask the model what is worth remembering from a finished run; apply the additions.
    /// Returns the (file, section, text) entries that were written.
    pub async fn reflect(&self, client: &Client, msgs: &[Message]) -> Result<Vec<(String, String, String)>> {
        let transcript = compact_transcript(msgs, 9000);
        let system = "You maintain an AI coding agent's long-term memory. Given a transcript of a finished session, extract ONLY durable, non-obvious knowledge worth keeping for future sessions: how the user likes things done, facts about this project (structure quirks, commands that work, gotchas), how-to knowledge, and lessons from mistakes. Skip anything trivial, one-off, secret, or already present in the memory files. Reply with JSON only, no prose:\n{\"brain\": [{\"section\": \"User|Projects|How-to|Lessons\", \"text\": \"...\"}], \"memory\": [{\"section\": \"Settings|Preferences|Ideas\", \"text\": \"...\"}], \"workflows\": [{\"name\": \"...\", \"steps\": [\"...\"]}]}\nEmpty arrays are the normal, correct answer for routine sessions. Max 3 items total; each text ≤ 140 chars.";
        let existing = format!("Current memory files:\n{}\n{}\n{}", self.read("MEMORY.md")?, self.read("BRAIN.md")?, self.read("WORKFLOWS.md")?);
        let req = vec![Message::system(system), Message::user(format!("{existing}\n\n=== Session transcript ===\n{transcript}\n\nJSON:"))];
        let (reply, _) = client.chat(&req, &[]).await?;
        let text = reply.text();
        let json = extract_json(&text).unwrap_or_else(|| "{}".into());
        #[derive(Deserialize, Default)] struct Item { section: String, text: String }
        #[derive(Deserialize, Default)] struct Wf { name: String, #[serde(default)] steps: Vec<String> }
        #[derive(Deserialize, Default)] struct Out { #[serde(default)] brain: Vec<Item>, #[serde(default)] memory: Vec<Item>, #[serde(default)] workflows: Vec<Wf> }
        let out: Out = serde_json::from_str(&json).unwrap_or_default();
        let mut written = Vec::new();
        for it in out.brain.into_iter().take(3) { if !it.text.trim().is_empty() && self.append("BRAIN.md", &it.section, &it.text)? { written.push(("BRAIN.md".into(), it.section, it.text)); } }
        for it in out.memory.into_iter().take(3) { if !it.text.trim().is_empty() && self.append("MEMORY.md", &it.section, &it.text)? { written.push(("MEMORY.md".into(), it.section, it.text)); } }
        for wf in out.workflows.into_iter().take(2) {
            if wf.name.trim().is_empty() || wf.steps.is_empty() { continue; }
            let mut doc = self.read("WORKFLOWS.md")?;
            let heading = format!("## {}", wf.name.trim());
            if doc.contains(&heading) { continue; }
            doc.push_str(&format!("\n{heading}\n"));
            for (i, st) in wf.steps.iter().enumerate() { doc.push_str(&format!("{}. {}\n", i + 1, st)); }
            std::fs::write(self.path("WORKFLOWS.md")?, doc)?;
            written.push(("WORKFLOWS.md".into(), wf.name, format!("{} steps", wf.steps.len())));
        }
        Ok(written)
    }

    /// If a file is over the size threshold, ask the model to consolidate it (merge, dedupe, drop stale).
    pub async fn maybe_consolidate(&self, client: &Client) -> Result<Vec<String>> {
        let mut done = Vec::new();
        for f in FILES {
            let doc = self.read(f)?;
            if doc.chars().count() <= self.cfg.consolidate_over_chars { continue; }
            let system = "You tidy an AI agent's markdown memory file. Rewrite it: keep the same top-level structure and headings, merge duplicates and near-duplicates, drop stale or contradictory entries (keep the newer), keep every distinct durable fact, keep the per-project ledger lines (they contain 'sessions:') verbatim. Output only the new markdown.";
            let req = vec![Message::system(system), Message::user(doc.clone())];
            let (reply, _) = client.chat(&req, &[]).await?;
            let new = reply.text();
            if new.trim().starts_with('#') && new.chars().count() > 200 && new.chars().count() < doc.chars().count() {
                self.write(f, new.trim_end())?;
                std::fs::write(self.path(f)?.with_extension("md"), format!("{}\n", new.trim_end()))?;
                done.push(f.to_string());
            }
        }
        Ok(done)
    }
}

pub fn canonical_name(file: &str) -> Result<&'static str> {
    let f = file.trim().trim_end_matches(".md").to_ascii_uppercase();
    match f.as_str() {
        "MEMORY" => Ok("MEMORY.md"),
        "WORKFLOWS" | "WORKFLOW" => Ok("WORKFLOWS.md"),
        "BRAIN" => Ok("BRAIN.md"),
        _ => bail!("unknown memory file '{file}' (use MEMORY, WORKFLOWS or BRAIN)"),
    }
}

fn shellexpand(p: &str) -> String {
    if let Some(rest) = p.strip_prefix('~') { format!("{}{}", crate::setup::home_dir().display().to_string(), rest) } else { p.to_string() }
}

/// Compact a transcript for reflection: user text, assistant text, tool names + short args/results.
fn compact_transcript(msgs: &[Message], max_chars: usize) -> String {
    let mut out = String::new();
    for m in msgs.iter().skip(1) { // skip system
        match m.role.as_str() {
            "user" => out.push_str(&format!("USER: {}\n", crate::llm::truncate_for_log(&m.text(), 600))),
            "assistant" => {
                let t = m.text(); if !t.trim().is_empty() { out.push_str(&format!("ASSISTANT: {}\n", crate::llm::truncate_for_log(&t, 500))); }
                if let Some(calls) = &m.tool_calls { for c in calls { out.push_str(&format!("  CALL {}({})\n", c.function.name, crate::llm::truncate_for_log(&c.function.arguments.replace('\n', " "), 160))); } }
            }
            "tool" => out.push_str(&format!("  RESULT {}: {}\n", m.name.clone().unwrap_or_default(), crate::llm::truncate_for_log(&m.text().replace('\n', " "), 200))),
            _ => {}
        }
    }
    // keep the tail if too long (recent context matters most)
    let n = out.chars().count();
    if n > max_chars { out = format!("…[{} chars elided]\n{}", n - max_chars, out.chars().skip(n - max_chars).collect::<String>()); }
    out
}

fn extract_json(s: &str) -> Option<String> {
    let a = s.find('{')?; let b = s.rfind('}')?;
    if b > a { Some(s[a..=b].to_string()) } else { None }
}

/// YYYY-MM-DD from the system clock (UTC), no chrono dependency.
pub fn today_iso() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let days = (secs / 86400) as i64;
    // civil-from-days (Howard Hinnant)
    let z = days + 719468; let era = if z >= 0 { z } else { z - 146096 } / 146097; let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; let y = yoe + era * 400; let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153; let d = doy - (153 * mp + 2) / 5 + 1; let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn store(name: &str) -> MemoryStore {
        let d = std::env::temp_dir().join(format!("harness-mem-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        MemoryStore::scratch(&d, &MemoryConfig::default()).unwrap()
    }
    #[test]
    fn append_and_dedupe_and_remove() {
        let s = store("append");
        assert!(s.append("brain", "Lessons", "always run cargo test").unwrap());
        assert!(!s.append("BRAIN.md", "Lessons", "always run cargo test").unwrap());
        assert!(s.append("brain", "New Section", "x").unwrap());
        let doc = s.read("brain").unwrap();
        assert!(doc.contains("## Lessons\n\n- always run cargo test"), "{doc}");
        assert!(doc.contains("## New Section\n\n- x"));
        assert_eq!(s.remove("brain", "cargo test").unwrap(), 1);
        assert!(!s.read("brain").unwrap().contains("cargo test"));
    }
    #[test]
    fn project_ledger() {
        let s = store("ledger");
        let p = Path::new("/tmp/proj-x");
        s.touch_project(p).unwrap(); s.touch_project(p).unwrap();
        let doc = s.read("brain").unwrap();
        assert!(doc.contains("/tmp/proj-x · sessions: 2 · last:"), "{doc}");
    }
    #[test]
    fn dates_and_names() {
        assert!(today_iso().starts_with("20"));
        assert_eq!(canonical_name("workflows.md").unwrap(), "WORKFLOWS.md");
        assert!(canonical_name("nope").is_err());
    }
}
