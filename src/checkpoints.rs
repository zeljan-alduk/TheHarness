//! File checkpoints: a shadow git repository that snapshots the working tree before every mutating
//! tool call, so `/undo`, `/redo` and the rewind menu can put the files back.
//!
//! The shadow repo lives in `~/.config/harness/snapshots/<session>/` and never touches the project's
//! own `.git` (it is a separate GIT_DIR with the project as GIT_WORK_TREE; `.git/` itself is excluded).
//! Untracked files above `max_file_mb` and anything the project's `.gitignore` hides are skipped, so a
//! snapshot of a normal repo costs a few tens of milliseconds and a few KB.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    /// Commit sha in the shadow repo.
    pub id: String,
    pub label: String,
    pub time: u64,
    /// Number of messages in the transcript when it was taken (for "rewind code + conversation").
    #[serde(default)] pub msgs: usize,
    /// How many files differed from the previous checkpoint.
    #[serde(default)] pub changed: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct Index { #[serde(default)] list: Vec<Checkpoint>, #[serde(default)] cursor: usize }

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CheckpointsConfig {
    /// Snapshot the working tree before mutating tool calls (`/undo`).
    #[serde(default = "d_true")] pub enabled: bool,
    /// Skip untracked files bigger than this (MB).
    #[serde(default = "d_two")] pub max_file_mb: u64,
}
fn d_true() -> bool { true }
fn d_two() -> u64 { 2 }
impl Default for CheckpointsConfig { fn default() -> Self { Self { enabled: true, max_file_mb: 2 } } }

static ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
static MAX_MB: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(2);
/// Called once at start-up from the config (like `sandbox::configure_seatbelt`).
pub fn configure(cfg: &CheckpointsConfig) {
    ENABLED.store(cfg.enabled, std::sync::atomic::Ordering::Relaxed);
    MAX_MB.store(cfg.max_file_mb.max(1), std::sync::atomic::Ordering::Relaxed);
}
pub fn enabled() -> bool { ENABLED.load(std::sync::atomic::Ordering::Relaxed) }

/// Tools that can change files on disk — a snapshot is taken before these run.
pub const MUTATING_TOOLS: [&str; 10] = ["bash", "write_file", "edit_file", "apply_patch", "notebook_edit", "pdf_edit", "extract_archive", "download_file", "run_workflow", "terminal"];

pub struct Checkpoints {
    pub git_dir: PathBuf,
    pub workdir: PathBuf,
    index: Mutex<Index>,
}

/// One shadow repo per (session, workdir), reused across tool calls.
pub fn for_session(session_id: &str, workdir: &Path) -> Option<Arc<Checkpoints>> {
    if !enabled() || session_id.is_empty() { return None; }
    static CACHE: OnceLock<Mutex<HashMap<String, Option<Arc<Checkpoints>>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let key = format!("{session_id}\u{1}{}", workdir.display());
    if let Some(hit) = cache.lock().ok().and_then(|g| g.get(&key).cloned()) { return hit; }
    let made = Checkpoints::open(session_id, workdir).ok().map(Arc::new);
    if let Ok(mut g) = cache.lock() { g.insert(key, made.clone()); }
    made
}

pub fn snapshots_dir() -> PathBuf { crate::setup::config_dir().join("snapshots") }

impl Checkpoints {
    pub fn open(session_id: &str, workdir: &Path) -> Result<Checkpoints> {
        Self::open_in(snapshots_dir().join(session_id), workdir)
    }

    /// Same, with an explicit shadow-repo location (tests, `harness checkpoint`).
    pub fn open_in(git_dir: PathBuf, workdir: &Path) -> Result<Checkpoints> {
        let workdir = workdir.canonicalize().unwrap_or_else(|_| workdir.to_path_buf());
        if !workdir.is_dir() { bail!("workdir does not exist"); }
        let fresh = !git_dir.join("HEAD").is_file();
        std::fs::create_dir_all(&git_dir)?;
        let cp = Checkpoints { git_dir, workdir, index: Mutex::new(Index::default()) };
        if fresh {
            cp.git(&["init", "-q"])?;
            cp.git(&["config", "gc.auto", "0"])?;
            cp.git(&["config", "core.fsmonitor", "false"])?;
            std::fs::create_dir_all(cp.git_dir.join("info"))?;
            std::fs::write(cp.git_dir.join("info").join("exclude"), "# harness shadow repo\n.git/\n")?;
        }
        *cp.index.lock().unwrap() = cp.read_index();
        Ok(cp)
    }

    fn index_path(&self) -> PathBuf { self.git_dir.join("checkpoints.json") }
    fn read_index(&self) -> Index { std::fs::read_to_string(self.index_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default() }
    fn write_index(&self, idx: &Index) { let _ = std::fs::write(self.index_path(), serde_json::to_string_pretty(idx).unwrap_or_default()); }

    fn git(&self, args: &[&str]) -> Result<String> {
        let out = Command::new("git")
            .env("GIT_DIR", &self.git_dir).env("GIT_WORK_TREE", &self.workdir)
            .env("GIT_AUTHOR_NAME", "harness").env("GIT_AUTHOR_EMAIL", "harness@local")
            .env("GIT_COMMITTER_NAME", "harness").env("GIT_COMMITTER_EMAIL", "harness@local")
            .env("GIT_CONFIG_NOSYSTEM", "1").env("HOME", self.git_dir.display().to_string())
            .current_dir(&self.workdir)
            .args(["-c", "core.hooksPath=/dev/null", "-c", "commit.gpgsign=false"]).args(args)
            .output().context("running git for the checkpoint store")?;
        if !out.status.success() { bail!("git {:?}: {}", args, String::from_utf8_lossy(&out.stderr).trim().to_string()); }
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }

    pub fn list(&self) -> Vec<Checkpoint> { self.index.lock().unwrap().list.clone() }
    pub fn cursor(&self) -> usize { self.index.lock().unwrap().cursor }

    /// Take a snapshot. Returns None when nothing changed since the last one.
    pub fn snapshot(&self, label: &str, msgs: usize) -> Result<Option<Checkpoint>> {
        let status = self.git(&["status", "--porcelain=v1", "-z", "--untracked-files=all", "--no-renames"])?;
        let limit = MAX_MB.load(std::sync::atomic::Ordering::Relaxed) * 1024 * 1024;
        let mut paths: Vec<String> = Vec::new();
        for entry in status.split('\0') {
            if entry.len() < 4 { continue; }
            let path = entry[3..].to_string();
            let full = self.workdir.join(&path);
            if let Ok(md) = std::fs::metadata(&full) { if md.is_file() && md.len() > limit { continue; } }
            paths.push(path);
            if paths.len() > 5000 { break; }
        }
        let first = self.index.lock().unwrap().list.is_empty();
        if paths.is_empty() && !first { return Ok(None); }
        if !paths.is_empty() {
            for chunk in paths.chunks(200) {
                let mut args: Vec<&str> = vec!["add", "--"];
                args.extend(chunk.iter().map(|s| s.as_str()));
                self.git(&args)?;
            }
        }
        let msg = format!("{label}");
        self.git(&["commit", "-q", "--allow-empty", "--no-verify", "-m", &msg])?;
        let id = self.git(&["rev-parse", "HEAD"])?.trim().to_string();
        let mut idx = self.index.lock().unwrap();
        // keep every snapshot reachable even after a reset --hard to an earlier one
        let _ = self.git(&["update-ref", &format!("refs/harness/cp-{}", idx.list.len()), &id]);
        let cp = Checkpoint { id, label: label.to_string(), time: now(), msgs, changed: paths.len() };
        idx.list.push(cp.clone());
        idx.cursor = idx.list.len() - 1;
        self.write_index(&idx);
        Ok(Some(cp))
    }

    /// Restore the working tree to a checkpoint (by index or sha prefix). Returns (checkpoint, files changed).
    pub fn restore(&self, which: &str) -> Result<(Checkpoint, usize)> {
        let pos = self.resolve(which)?;
        let target = self.index.lock().unwrap().list[pos].clone();
        // capture uncommitted work first so the restore itself is undoable (a clean tree already is a checkpoint)
        if !self.git(&["status", "--porcelain", "--untracked-files=all"])?.trim().is_empty() {
            let _ = self.snapshot(&format!("before restore to #{}", pos + 1), 0);
        }
        let changed = self.git(&["diff", "--name-only", "HEAD", &target.id]).map(|s| s.lines().count()).unwrap_or(0);
        self.git(&["reset", "-q", "--hard", &target.id])?;
        let mut idx = self.index.lock().unwrap();
        idx.cursor = pos;
        self.write_index(&idx);
        Ok((target, changed))
    }

    /// Step back one checkpoint (`/undo`).
    pub fn undo(&self, steps: usize) -> Result<(Checkpoint, usize)> {
        let (len, cursor) = { let i = self.index.lock().unwrap(); (i.list.len(), i.cursor) };
        if len == 0 { bail!("no checkpoints yet in this session"); }
        // uncommitted work: snapshot it so "undo" goes back to the last known-good state
        if !self.git(&["status", "--porcelain", "--untracked-files=all"])?.trim().is_empty() {
            let _ = self.snapshot("before undo", 0)?;
        }
        let cursor = self.index.lock().unwrap().cursor.max(cursor);
        let target = cursor.checked_sub(steps.max(1)).context("nothing left to undo (already at the first checkpoint)")?;
        self.restore(&(target + 1).to_string())
    }

    /// Step forward again (`/redo`).
    pub fn redo(&self, steps: usize) -> Result<(Checkpoint, usize)> {
        let (len, cursor) = { let i = self.index.lock().unwrap(); (i.list.len(), i.cursor) };
        let target = cursor + steps.max(1);
        if target >= len { bail!("nothing to redo"); }
        self.restore(&(target + 1).to_string())
    }

    /// `n` (1-based index into list) or a sha prefix.
    fn resolve(&self, which: &str) -> Result<usize> {
        let idx = self.index.lock().unwrap();
        let w = which.trim();
        if let Ok(n) = w.parse::<usize>() {
            if n >= 1 && n <= idx.list.len() { return Ok(n - 1); }
            bail!("no checkpoint #{n} (have {})", idx.list.len());
        }
        idx.list.iter().position(|c| c.id.starts_with(w)).with_context(|| format!("no checkpoint matching '{w}'"))
    }

    /// What a restore would change, as a diff --stat.
    pub fn diff(&self, which: &str) -> Result<String> {
        let pos = self.resolve(which)?;
        let id = self.index.lock().unwrap().list[pos].id.clone();
        self.git(&["diff", "--stat", &id, "HEAD"])
    }

    /// Delete the shadow repo (session ended / cleanup).
    pub fn discard(&self) { let _ = std::fs::remove_dir_all(&self.git_dir); }
}

/// Drop snapshot repos for sessions that no longer exist / are older than `days`.
pub fn prune(days: u64) -> usize {
    let cutoff = now().saturating_sub(days * 86400);
    let mut n = 0;
    let Ok(rd) = std::fs::read_dir(snapshots_dir()) else { return 0 };
    for e in rd.flatten() {
        let p = e.path();
        let old = std::fs::metadata(p.join("HEAD")).and_then(|m| m.modified()).ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() < cutoff).unwrap_or(false);
        if old && std::fs::remove_dir_all(&p).is_ok() { n += 1; }
    }
    n
}

fn now() -> u64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_undo_redo() {
        let base = std::env::temp_dir().join(format!("harness-cp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let wd = base.join("proj");
        std::fs::create_dir_all(&wd).unwrap();
        std::fs::write(wd.join("a.txt"), "one\n").unwrap();
        let cp = Checkpoints::open_in(base.join("shadow"), &wd).unwrap();
        cp.snapshot("start", 0).unwrap().expect("first snapshot");
        std::fs::write(wd.join("a.txt"), "two\n").unwrap();
        std::fs::write(wd.join("b.txt"), "new\n").unwrap();
        cp.snapshot("after edit", 2).unwrap().expect("second snapshot");
        assert_eq!(cp.list().len(), 2);
        assert!(cp.snapshot("no change", 2).unwrap().is_none(), "clean tree must not create a checkpoint");

        let (target, _) = cp.undo(1).unwrap();
        assert_eq!(target.label, "start");
        assert_eq!(std::fs::read_to_string(wd.join("a.txt")).unwrap(), "one\n");
        assert!(!wd.join("b.txt").exists(), "file created after the checkpoint is removed on undo");

        let (fwd, _) = cp.redo(1).unwrap();
        assert_eq!(fwd.label, "after edit");
        assert_eq!(std::fs::read_to_string(wd.join("a.txt")).unwrap(), "two\n");
        assert!(wd.join("b.txt").exists());

        // an edit made after undo/redo starts a new branch of history
        std::fs::write(wd.join("a.txt"), "three\n").unwrap();
        cp.snapshot("third", 4).unwrap().unwrap();
        assert!(cp.redo(1).is_err());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn skips_big_untracked_files() {
        let base = std::env::temp_dir().join(format!("harness-cp2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let wd = base.join("proj");
        std::fs::create_dir_all(&wd).unwrap();
        configure(&CheckpointsConfig { enabled: true, max_file_mb: 1 });
        std::fs::write(wd.join("big.bin"), vec![7u8; 2 * 1024 * 1024]).unwrap();
        std::fs::write(wd.join("small.txt"), "hi").unwrap();
        let cp = Checkpoints::open_in(base.join("shadow"), &wd).unwrap();
        let c = cp.snapshot("start", 0).unwrap().unwrap();
        assert_eq!(c.changed, 1, "only the small file is snapshotted");
        let _ = std::fs::remove_dir_all(&base);
    }
}
