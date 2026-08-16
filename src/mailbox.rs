//! Cross-session messaging: every interactive session registers a heartbeat in ~/.config/harness/live/<id>.json
//! and reads its mailbox ~/.config/harness/mailbox/<id>.jsonl. `send_message` appends to another session's
//! mailbox; the receiving TUI pushes it into the agent's inbox (a wakeup). Works across terminals/machines
//! that share the config dir.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Live { pub id: String, pub title: String, pub workdir: String, pub pid: u32, pub backend: String, pub updated: u64, #[serde(default)] pub busy: bool }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mail { pub from: String, pub text: String, pub at: u64 }

fn now() -> u64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) }
fn live_dir() -> PathBuf { crate::setup::config_dir().join("live") }
fn mail_dir() -> PathBuf { crate::setup::config_dir().join("mailbox") }
fn mail_file(id: &str) -> PathBuf { mail_dir().join(format!("{}.jsonl", id.replace('/', "_"))) }

pub fn heartbeat(l: &Live) { let _ = std::fs::create_dir_all(live_dir()); let mut l = l.clone(); l.updated = now(); let _ = std::fs::write(live_dir().join(format!("{}.json", l.id)), serde_json::to_string(&l).unwrap_or_default()); }
pub fn unregister(id: &str) { let _ = std::fs::remove_file(live_dir().join(format!("{id}.json"))); }
/// Live sessions (heartbeat within 30 s), newest first.
pub fn live() -> Vec<Live> {
    let mut v: Vec<Live> = std::fs::read_dir(live_dir()).into_iter().flatten().flatten().filter_map(|e| std::fs::read_to_string(e.path()).ok()).filter_map(|t| serde_json::from_str::<Live>(&t).ok()).filter(|l| now().saturating_sub(l.updated) < 30).collect();
    v.sort_by(|a, b| b.updated.cmp(&a.updated)); v
}
pub fn send(to: &str, from: &str, text: &str) -> Result<usize> {
    let targets: Vec<String> = if to == "all" || to == "*" { live().into_iter().map(|l| l.id).filter(|id| id != from).collect() } else { let l = live(); match l.iter().find(|x| x.id == to || x.id.starts_with(to) || x.title.to_lowercase().contains(&to.to_lowercase())) { Some(x) => vec![x.id.clone()], None => vec![to.to_string()] } };
    let _ = std::fs::create_dir_all(mail_dir());
    for t in &targets { let mut f = std::fs::OpenOptions::new().create(true).append(true).open(mail_file(t))?; use std::io::Write; writeln!(f, "{}", serde_json::to_string(&Mail { from: from.into(), text: text.into(), at: now() })?)?; }
    Ok(targets.len())
}
/// Take and clear pending mail for `id`.
pub fn take(id: &str) -> Vec<Mail> {
    let p = mail_file(id);
    let Ok(t) = std::fs::read_to_string(&p) else { return vec![] };
    let _ = std::fs::remove_file(&p);
    t.lines().filter_map(|l| serde_json::from_str::<Mail>(l).ok()).collect()
}
