//! Git worktrees for isolated work: `<repo>/.harness/worktrees/<name>` (kept out of `git status` via
//! `.git/info/exclude`). Used by the `worktree` tool and `spawn_agent {isolation:"worktree"}`.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

pub const DIR: &str = ".harness/worktrees";

/// The session's current worktree (set by `worktree enter`, cleared by `exit`).
#[derive(Debug, Clone)]
pub struct Cwd { pub original: PathBuf, pub current: PathBuf, pub name: String }
/// Shared cell: `worktree enter/exit` writes it, the agent loop reads it before every tool batch.
pub type CwdCell = std::sync::Arc<std::sync::Mutex<Option<Cwd>>>;
pub fn new_cell() -> CwdCell { std::sync::Arc::new(std::sync::Mutex::new(None)) }

fn git(cwd: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git").args(args).current_dir(cwd).output().with_context(|| "running git")?;
    if !out.status.success() { bail!("git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim()); }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Root of the *main* working tree (works from inside a linked worktree too).
pub fn main_root(cwd: &Path) -> Result<PathBuf> {
    let common = git(cwd, &["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .or_else(|_| git(cwd, &["rev-parse", "--git-common-dir"]).map(|p| cwd.join(p).display().to_string()))?;
    let common = PathBuf::from(common);
    let root = common.parent().map(|p| p.to_path_buf()).unwrap_or(common);
    Ok(root.canonicalize().unwrap_or(root))
}

fn valid_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 64 || !name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')) || name.starts_with('.') {
        bail!("invalid worktree name '{name}' (use letters, digits, '-', '_', '.')");
    }
    Ok(())
}

/// Branch names / base refs go straight to git argv: keep them to a safe charset, no `..`, and never
/// option-like (a leading '-' would be parsed as a git flag).
fn valid_ref(kind: &str, r: &str) -> Result<()> {
    if r.is_empty() || r.len() > 200 || r.contains("..") || r.starts_with('-') || r.starts_with('/') || r.ends_with('/') || r.ends_with(".lock")
        || !r.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | '@' | '~' | '^')) {
        bail!("invalid {kind} '{r}' (use letters, digits, '-', '_', '.', '/'; no '..', no leading '-')");
    }
    Ok(())
}

pub fn path_of(root: &Path, name: &str) -> PathBuf { root.join(DIR).join(name) }

fn ensure_excluded(root: &Path) {
    let git_dir = match git(root, &["rev-parse", "--path-format=absolute", "--git-dir"]) { Ok(d) => PathBuf::from(d), Err(_) => root.join(".git") };
    let excl = git_dir.join("info").join("exclude");
    let cur = std::fs::read_to_string(&excl).unwrap_or_default();
    if cur.lines().any(|l| l.trim() == ".harness/" || l.trim() == "/.harness/") { return; }
    let _ = std::fs::create_dir_all(excl.parent().unwrap());
    let mut s = cur; if !s.is_empty() && !s.ends_with('\n') { s.push('\n'); }
    s.push_str("/.harness/\n");
    let _ = std::fs::write(&excl, s);
}

/// Create a worktree `name` on branch `branch` (default: `wt/<name>`; created from `base` (default HEAD)
/// if it does not exist). Returns the worktree path.
pub fn create(cwd: &Path, name: &str, branch: Option<&str>, base: Option<&str>) -> Result<PathBuf> {
    valid_name(name)?;
    let root = main_root(cwd)?;
    let path = path_of(&root, name);
    if path.exists() { bail!("worktree '{name}' already exists at {}", path.display()); }
    std::fs::create_dir_all(path.parent().unwrap())?;
    ensure_excluded(&root);
    let branch = branch.map(|s| s.to_string()).unwrap_or_else(|| format!("wt/{name}"));
    valid_ref("branch", &branch)?;
    if let Some(b) = base { valid_ref("base", b)?; }
    let exists = git(&root, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{branch}")]).is_ok();
    let p = path.display().to_string();
    if exists {
        git(&root, &["worktree", "add", &p, &branch])?;
    } else {
        let base = base.unwrap_or("HEAD");
        git(&root, &["worktree", "add", "-b", &branch, &p, base])?;
    }
    Ok(path)
}

/// (name, path, branch) of harness-managed worktrees.
pub fn list(cwd: &Path) -> Result<Vec<(String, PathBuf, String)>> {
    let root = main_root(cwd)?;
    let base = root.join(DIR);
    let out = git(&root, &["worktree", "list", "--porcelain"])?;
    let mut v = Vec::new();
    let (mut path, mut branch) = (None::<PathBuf>, String::new());
    let flush = |v: &mut Vec<_>, path: &mut Option<PathBuf>, branch: &mut String| {
        if let Some(p) = path.take() { if p.starts_with(&base) { if let Some(n) = p.file_name() { v.push((n.to_string_lossy().to_string(), p.clone(), std::mem::take(branch))); } } }
        branch.clear();
    };
    for line in out.lines() {
        if let Some(p) = line.strip_prefix("worktree ") { flush(&mut v, &mut path, &mut branch); path = Some(PathBuf::from(p)); }
        else if let Some(b) = line.strip_prefix("branch ") { branch = b.trim_start_matches("refs/heads/").to_string(); }
        else if line == "detached" { branch = "(detached)".into(); }
    }
    flush(&mut v, &mut path, &mut branch);
    Ok(v)
}

/// Remove a worktree (and optionally its branch). Refuses if it has uncommitted changes unless `force`.
pub fn remove(cwd: &Path, name: &str, delete_branch: bool, force: bool) -> Result<String> {
    valid_name(name)?;
    let root = main_root(cwd)?;
    let path = path_of(&root, name);
    let branch = list(cwd)?.into_iter().find(|(n, _, _)| n == name).map(|(_, _, b)| b);
    if !path.exists() && branch.is_none() { bail!("no worktree named '{name}'"); }
    if !force && path.exists() {
        let dirty = git(&path, &["status", "--porcelain"]).unwrap_or_default();
        if !dirty.is_empty() { bail!("worktree '{name}' has uncommitted changes ({} entries); commit them or pass force=true", dirty.lines().count()); }
    }
    let p = path.display().to_string();
    let mut args = vec!["worktree", "remove"]; if force { args.push("--force"); } args.push(&p);
    git(&root, &args)?;
    let mut msg = format!("removed worktree '{name}'");
    if delete_branch { if let Some(b) = branch.filter(|b| b != "(detached)") { let flag = if force { "-D" } else { "-d" }; match git(&root, &["branch", flag, &b]) { Ok(_) => msg.push_str(&format!(" and branch {b}")), Err(e) => msg.push_str(&format!(" (branch {b} kept: {e})")) } } }
    Ok(msg)
}

/// Short summary of a worktree vs the main branch: commits ahead and dirty files.
pub fn status(path: &Path) -> String {
    let branch = git(path, &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_default();
    let dirty = git(path, &["status", "--porcelain"]).map(|s| s.lines().count()).unwrap_or(0);
    let ahead = git(path, &["rev-list", "--count", "@{u}..HEAD"]).ok().or_else(|| git(path, &["rev-list", "--count", "HEAD", "--not", "--branches", "--exclude", &branch]).ok()).unwrap_or_default();
    format!("branch {branch}, {dirty} uncommitted, {ahead} commit(s) not on other branches", ahead = if ahead.is_empty() { "?".into() } else { ahead })
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Tmp(PathBuf);
    impl Tmp { fn path(&self) -> &Path { &self.0 } }
    impl Drop for Tmp { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
    fn repo() -> Tmp {
        let d = Tmp(std::env::temp_dir().join(format!("harness-wt-test-{}", std::process::id())));
        let _ = std::fs::remove_dir_all(&d.0); std::fs::create_dir_all(&d.0).unwrap();
        for a in [vec!["init", "-q", "-b", "main"], vec!["config", "user.email", "t@t"], vec!["config", "user.name", "t"]] { git(d.path(), &a).unwrap(); }
        std::fs::write(d.path().join("a.txt"), "a").unwrap();
        git(d.path(), &["add", "."]).unwrap(); git(d.path(), &["commit", "-qm", "init"]).unwrap();
        d
    }
    #[test]
    fn create_list_remove() {
        let d = repo();
        let p = create(d.path(), "feat", None, None).unwrap();
        assert!(p.join("a.txt").exists());
        assert_eq!(main_root(&p).unwrap(), d.path().canonicalize().unwrap());
        let l = list(d.path()).unwrap();
        assert_eq!(l.len(), 1); assert_eq!(l[0].0, "feat"); assert_eq!(l[0].2, "wt/feat");
        assert!(git(d.path(), &["status", "--porcelain"]).unwrap().is_empty(), "worktree dir must be excluded");
        std::fs::write(p.join("b.txt"), "b").unwrap();
        assert!(remove(d.path(), "feat", true, false).is_err());
        let m = remove(d.path(), "feat", true, true).unwrap();
        assert!(m.contains("branch wt/feat"), "{m}");
        assert!(list(d.path()).unwrap().is_empty());
        assert!(create(d.path(), "../x", None, None).is_err());
        assert!(create(d.path(), "ok", Some("--upload-pack=evil"), None).is_err());
        assert!(create(d.path(), "ok", Some("a b"), None).is_err());
        assert!(create(d.path(), "ok", Some("x/../y"), None).is_err());
        assert!(create(d.path(), "ok", None, Some("-x")).is_err());
        assert!(valid_ref("branch", "feat/x-1.2_y").is_ok());
    }
}
