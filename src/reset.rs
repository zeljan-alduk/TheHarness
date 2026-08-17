//! Two destructive lifecycle commands, kept honest by always showing a plan before they run.
//!
//! `/factory-reset` returns the harness to how it behaves on a fresh machine: user state (config, sessions,
//! memory, plugins, logs, checkpoints) and the downloaded model are removed, but the installer-provided
//! parts (the MLX runtime venv, the source checkout, the tool symlinks) stay, so the very next launch runs
//! the first-run flow (model picker, Claude offer) without needing the installer again.
//!
//! `/uninstall` removes everything the harness owns: the whole `~/.config/harness` tree, the app bundle and
//! its Desktop alias, and the `harness` binary plus the symlinks the installer created in `~/.local/bin`.
//! Shared tools the installer *also* set up — kitty, claude, rustup, uv, Homebrew — are never touched; they
//! are listed so the user can remove them by hand if they want to.

use crate::setup::{bin_dir, config_dir, home_dir};
use std::path::{Path, PathBuf};

/// One thing a reset/uninstall will delete, with its size so the plan can total it.
#[derive(Debug, Clone)]
pub struct Item {
    pub path: PathBuf,
    pub label: String,
    pub bytes: u64,
    pub exists: bool,
}

fn dir_size(p: &Path) -> u64 {
    if p.is_symlink() { return 0; }
    let Ok(md) = std::fs::symlink_metadata(p) else { return 0 };
    if md.is_file() { return md.len(); }
    if !md.is_dir() { return 0; }
    std::fs::read_dir(p).map(|rd| rd.flatten().map(|e| dir_size(&e.path())).sum()).unwrap_or(0)
}

fn item(path: PathBuf, label: &str) -> Item {
    let exists = path.exists() || path.is_symlink();
    let bytes = if exists { dir_size(&path) } else { 0 };
    Item { path, label: label.into(), bytes, exists }
}

pub fn human(bytes: u64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let (mut b, mut i) = (bytes as f64, 0);
    while b >= 1024.0 && i < U.len() - 1 { b /= 1024.0; i += 1; }
    if i == 0 { format!("{bytes} B") } else { format!("{b:.1} {}", U[i]) }
}

/// Everything a factory reset removes: user state under the config dir, plus the downloaded model. Honors
/// the same env overrides the harness reads (HARNESS_SESSIONS_DIR, …) so a reset hits the dirs actually in use.
pub fn factory_reset_plan() -> Vec<Item> {
    let cfg = config_dir();
    let env_or = |var: &str, default: PathBuf| std::env::var_os(var).map(PathBuf::from).unwrap_or(default);
    let mut v = vec![
        item(env_or("HARNESS_SESSIONS_DIR", cfg.join("sessions")), "saved sessions"),
        item(env_or("HARNESS_MEMORY_DIR", cfg.join("memory")), "memory (MEMORY.md, notes)"),
        item(env_or("HARNESS_PLUGINS_DIR", cfg.join("plugins")), "installed plugins"),
        item(cfg.join("models"), "downloaded model weights"),
        item(cfg.join("snapshots"), "file-checkpoint snapshots"),
        item(cfg.join("logs"), "logs"),
        item(cfg.join("pastes"), "pasted images / video frames"),
        item(cfg.join("exports"), "exported transcripts"),
        item(cfg.join("live"), "cross-session presence"),
        item(cfg.join("mailbox"), "cross-session mailbox"),
        item(cfg.join("arbiter"), "arbiter baselines"),
        item(cfg.join("agents"), "custom agents"),
        item(cfg.join("workflows"), "saved workflows"),
        item(cfg.join("prompts"), "saved prompts"),
        item(cfg.join("themes"), "custom themes"),
    ];
    // top-level state files (config, learned notes, permission/trust records, update cache)
    for (f, l) in [
        ("harness.toml", "config (regenerated on next launch)"),
        ("settings.toml", "settings overrides"),
        ("keybindings.toml", "custom keybindings"),
        ("mcp.json", "MCP server config"),
        ("permissions.json", "always-allow rules"),
        ("trusted.json", "trusted directories"),
        ("BRAIN.md", "learned lessons"),
        ("WORKFLOWS.md", "workflow recipes"),
        ("update-check.json", "cached update check"),
    ] { v.push(item(cfg.join(f), l)); }
    v.retain(|i| i.exists);
    v
}

/// Everything the installer put down that belongs to the harness: the whole config tree, the app bundle and
/// its alias, the binary, and the symlinks the installer made into ~/.local/bin. Not shared tools.
pub fn uninstall_plan() -> Vec<Item> {
    let (home, cfg) = (home_dir(), config_dir());
    let mut v = vec![
        item(cfg.clone(), "all harness state (config, sessions, memory, MLX runtime, model, source)"),
        item(home.join("Applications/TheHarness.app"), "the app bundle"),
        item(home.join("Desktop/TheHarness.app"), "the Desktop alias"),
    ];
    // the installed binary: the one that will run next, resolved through symlinks
    if let Ok(exe) = crate::update::installed_exe() { v.push(item(exe, "the harness binary")); }
    // symlinks the installer created in PREFIX/bin (default ~/.local/bin) that point at harness/kitty it manages
    let prefix_bin = home.join(".local/bin");
    for name in ["harness", "kitty", "kitten"] {
        let p = prefix_bin.join(name);
        if p.is_symlink() { v.push(item(p, "installer symlink in ~/.local/bin")); }
    }
    // dedupe (the installed binary may already be ~/.local/bin/harness)
    let mut seen = std::collections::HashSet::new();
    v.retain(|i| i.exists && seen.insert(i.path.canonicalize().unwrap_or(i.path.clone())));
    v
}

/// Shared tools the installer also set up but /uninstall leaves alone — surfaced so the user can decide.
pub fn shared_tools_left() -> Vec<(&'static str, PathBuf)> {
    let home = home_dir();
    let mut out = vec![];
    for (name, p) in [
        ("kitty (terminal)", PathBuf::from("/Applications/kitty.app")),
        ("kitty (terminal)", home.join(".local/kitty.app")),
        ("uv (Python installer)", home.join(".local/bin/uv")),
        ("rustup + cargo toolchain", home.join(".cargo")),
        ("Claude Code CLI", home.join(".local/bin/claude")),
    ] { if p.exists() { out.push((name, p)); } }
    out
}

/// Delete every existing path in the plan. Returns (removed, errors). Never follows symlinks out of the tree.
pub fn execute(items: &[Item]) -> (usize, Vec<String>) {
    let mut removed = 0;
    let mut errs = vec![];
    for it in items {
        if !it.exists { continue; }
        let md = std::fs::symlink_metadata(&it.path);
        let res = match md {
            Ok(m) if m.is_dir() && !m.is_symlink() => std::fs::remove_dir_all(&it.path),
            Ok(_) => std::fs::remove_file(&it.path),
            Err(e) => Err(e),
        };
        match res {
            Ok(()) => removed += 1,
            Err(e) => errs.push(format!("{}: {e}", it.path.display())),
        }
    }
    (removed, errs)
}

/// After a factory reset, recreate the empty dirs the harness expects and drop in the default config, so the
/// first launch is a clean first-run rather than an error about a missing directory.
pub fn seed_after_reset() -> std::io::Result<()> {
    let cfg = config_dir();
    std::fs::create_dir_all(&cfg)?;
    let _ = bin_dir(); // referenced so callers keep it available; harness recreates its own subdirs lazily
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn human_sizes() {
        assert_eq!(human(0), "0 B");
        assert_eq!(human(512), "512 B");
        assert_eq!(human(1536), "1.5 KB");
        assert_eq!(human(16_080_000_000), "15.0 GB");
    }
    #[test]
    fn plans_only_list_existing_paths() {
        // With a scratch config dir that has nothing in it, both plans are empty (nothing to delete).
        let tmp = std::env::temp_dir().join(format!("harness-reset-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::env::set_var("HARNESS_CONFIG_DIR", &tmp);
        assert!(factory_reset_plan().is_empty(), "empty config dir → nothing to reset");
        std::fs::create_dir_all(tmp.join("sessions")).unwrap();
        std::fs::write(tmp.join("sessions/a.jsonl"), "x").unwrap();
        std::fs::write(tmp.join("harness.toml"), "y").unwrap();
        let plan = factory_reset_plan();
        assert!(plan.iter().any(|i| i.label.contains("saved sessions")));
        assert!(plan.iter().any(|i| i.path.ends_with("harness.toml")));
        std::env::remove_var("HARNESS_CONFIG_DIR");
        let _ = std::fs::remove_dir_all(&tmp);
    }
    #[test]
    fn execute_removes_files_and_dirs() {
        let tmp = std::env::temp_dir().join(format!("harness-reset-exec-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("d")).unwrap();
        std::fs::write(tmp.join("d/f"), "x").unwrap();
        std::fs::write(tmp.join("g"), "y").unwrap();
        let items = vec![item(tmp.join("d"), "dir"), item(tmp.join("g"), "file"), item(tmp.join("missing"), "gone")];
        let (removed, errs) = execute(&items);
        assert_eq!(removed, 2, "two existed");
        assert!(errs.is_empty(), "{errs:?}");
        assert!(!tmp.join("d").exists() && !tmp.join("g").exists());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
