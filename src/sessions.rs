//! Session persistence: `~/.config/harness/sessions/<id>/{meta.json,transcript.jsonl}`.
//! Saved after every turn so a crash or quit loses nothing; `harness --resume` / `/resume` reload.

use crate::llm::Message;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Meta {
    pub id: String,
    pub title: String,
    pub workdir: String,
    pub model: String,
    /// Backend at save time ("claude-code" | "anthropic" | "" = local/OpenAI-compatible) and Claude effort — restored on resume/restart.
    #[serde(default)] pub provider: Option<String>,
    #[serde(default)] pub effort: Option<String>,
    pub created: u64,
    pub updated: u64,
    #[serde(default)] pub turns: usize,
    #[serde(default)] pub prompt_tokens: u64,
    #[serde(default)] pub completion_tokens: u64,
}

pub struct SessionStore { pub dir: PathBuf }

fn now() -> u64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) }

impl SessionStore {
    pub fn open() -> Result<Self> {
        let dir = std::env::var_os("HARNESS_SESSIONS_DIR").map(PathBuf::from)
            .unwrap_or_else(|| crate::setup::config_dir().join("sessions"));
        std::fs::create_dir_all(&dir)?;
        Ok(Self { dir })
    }
    /// New id: YYYYMMDD-HHMMSS-<pid> (sortable, unique enough).
    pub fn new_id() -> String {
        let secs = now(); let day = crate::memory::today_iso().replace('-', ""); let t = secs % 86400;
        format!("{day}-{:02}{:02}{:02}-{}", t / 3600, (t / 60) % 60, t % 60, std::process::id() % 10000)
    }
    pub fn save(&self, meta: &mut Meta, msgs: &[Message]) -> Result<()> {
        let d = self.dir.join(&meta.id);
        std::fs::create_dir_all(&d)?;
        if meta.created == 0 { meta.created = now(); }
        meta.updated = now();
        if meta.title.is_empty() { meta.title = msgs.iter().find(|m| m.role == "user").map(|m| crate::llm::truncate_for_log(m.text().lines().next().unwrap_or(""), 80)).unwrap_or_else(|| "(untitled)".into()); }
        meta.turns = msgs.iter().filter(|m| m.role == "user").count();
        let mut out = String::new();
        for m in msgs { out.push_str(&serde_json::to_string(m)?); out.push('\n'); }
        std::fs::write(d.join("transcript.jsonl"), out)?;
        std::fs::write(d.join("meta.json"), serde_json::to_string_pretty(meta)?)?;
        Ok(())
    }
    pub fn load(&self, id: &str) -> Result<(Meta, Vec<Message>)> {
        let d = self.dir.join(id);
        let meta: Meta = serde_json::from_str(&std::fs::read_to_string(d.join("meta.json")).with_context(|| format!("no session {id}"))?)?;
        let mut msgs = Vec::new();
        for line in std::fs::read_to_string(d.join("transcript.jsonl"))?.lines() { if line.trim().is_empty() { continue; } msgs.push(serde_json::from_str::<Message>(line)?); }
        Ok((meta, msgs))
    }
    /// Sessions, newest first. `workdir` filters to that directory.
    pub fn list(&self, workdir: Option<&str>) -> Vec<Meta> {
        let mut v: Vec<Meta> = std::fs::read_dir(&self.dir).into_iter().flatten().flatten()
            .filter_map(|e| std::fs::read_to_string(e.path().join("meta.json")).ok()).filter_map(|t| serde_json::from_str::<Meta>(&t).ok())
            .filter(|m| workdir.map(|w| m.workdir == w).unwrap_or(true)).collect();
        v.sort_by(|a, b| b.updated.cmp(&a.updated));
        v
    }
    pub fn latest_for(&self, workdir: &str) -> Option<Meta> { self.list(Some(workdir)).into_iter().next() }
    pub fn delete(&self, id: &str) -> Result<()> { Ok(std::fs::remove_dir_all(self.dir.join(id))?) }
}

pub fn fmt_age(updated: u64) -> String {
    let d = now().saturating_sub(updated);
    if d < 60 { format!("{d}s ago") } else if d < 3600 { format!("{}m ago", d / 60) } else if d < 86400 { format!("{}h ago", d / 3600) } else { format!("{}d ago", d / 86400) }
}
