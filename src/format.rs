//! Auto-formatting and post-edit diagnostics: after the agent writes a file, run the project's
//! formatter for that language (as editors do on save) and surface fresh errors from an already
//! running language server. Both are advisory — the note is appended to the tool result so the model
//! sees what happened and can react.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FormatConfig {
    /// Run a formatter after write_file / edit_file / apply_patch / notebook_edit.
    #[serde(default = "d_true")] pub enabled: bool,
    /// Report new diagnostics from a language server that is already running for this project.
    #[serde(default = "d_true")] pub diagnostics_after_edit: bool,
    /// extension → command; `{file}` is replaced with the path (default table below is used otherwise).
    #[serde(default)] pub commands: HashMap<String, String>,
    #[serde(default = "d_timeout")] pub timeout_secs: u64,
}
fn d_true() -> bool { true }
fn d_timeout() -> u64 { 15 }
impl Default for FormatConfig { fn default() -> Self { Self { enabled: true, diagnostics_after_edit: true, commands: HashMap::new(), timeout_secs: d_timeout() } } }

/// extension → formatter command lines, tried in order; the first whose binary exists wins.
/// `{file}` = the edited file. Kept deliberately close to what OpenCode/Kilo ship.
pub const DEFAULTS: &[(&str, &[&str])] = &[
    ("rs", &["rustfmt --edition 2021 {file}"]),
    ("py", &["ruff format {file}", "black -q {file}", "autopep8 -i {file}"]),
    ("pyi", &["ruff format {file}", "black -q {file}"]),
    ("go", &["gofmt -w {file}", "goimports -w {file}"]),
    ("ts", &["biome format --write {file}", "prettier --write {file}"]),
    ("tsx", &["biome format --write {file}", "prettier --write {file}"]),
    ("js", &["biome format --write {file}", "prettier --write {file}"]),
    ("jsx", &["biome format --write {file}", "prettier --write {file}"]),
    ("mjs", &["biome format --write {file}", "prettier --write {file}"]),
    ("cjs", &["biome format --write {file}", "prettier --write {file}"]),
    ("json", &["biome format --write {file}", "prettier --write {file}"]),
    ("jsonc", &["biome format --write {file}", "prettier --write {file}"]),
    ("css", &["prettier --write {file}"]),
    ("scss", &["prettier --write {file}"]),
    ("html", &["prettier --write {file}"]),
    ("md", &["prettier --write {file}"]),
    ("yaml", &["prettier --write {file}"]),
    ("yml", &["prettier --write {file}"]),
    ("toml", &["taplo fmt {file}"]),
    ("sh", &["shfmt -w {file}"]),
    ("bash", &["shfmt -w {file}"]),
    ("c", &["clang-format -i {file}"]),
    ("h", &["clang-format -i {file}"]),
    ("cpp", &["clang-format -i {file}"]),
    ("hpp", &["clang-format -i {file}"]),
    ("java", &["clang-format -i {file}"]),
    ("rb", &["rubocop -a --format quiet {file}"]),
    ("swift", &["swift-format -i {file}"]),
    ("zig", &["zig fmt {file}"]),
    ("lua", &["stylua {file}"]),
    ("nix", &["nixpkgs-fmt {file}"]),
    ("sql", &["sqlfluff fix -f {file}"]),
    ("php", &["php-cs-fixer fix {file}"]),
    ("kt", &["ktlint -F {file}"]),
    ("dart", &["dart format {file}"]),
    ("ex", &["mix format {file}"]),
    ("exs", &["mix format {file}"]),
];

/// The formatter command for a file, if one is configured and its binary is installed.
pub fn command_for(path: &Path, cfg: &FormatConfig, workdir: &Path) -> Option<String> {
    let ext = path.extension()?.to_str()?.to_lowercase();
    let quoted = crate::security::shell_quote(&path.display().to_string());
    if let Some(custom) = cfg.commands.get(&ext) {
        if custom.trim().is_empty() { return None; }
        return Some(custom.replace("{file}", &quoted));
    }
    let cands = DEFAULTS.iter().find(|(e, _)| *e == ext)?.1;
    for c in cands {
        let bin = c.split_whitespace().next()?;
        if let Some(full) = crate::setup::which_in(bin, workdir) {
            // use the resolved path: project-local formatters (node_modules/.bin) are not on PATH
            let rest = c.splitn(2, ' ').nth(1).unwrap_or("");
            let line = format!("{} {rest}", crate::security::shell_quote(&full.display().to_string()));
            return Some(line.trim().replace("{file}", &quoted));
        }
    }
    None
}

/// Format `path` if possible and check it. Returns a one-line note for the tool result (or None).
pub async fn after_edit(path: &Path, ctx: &crate::tools::ToolCtx) -> Option<String> {
    let cfg = &ctx.format;
    if !path.is_file() { return None; }
    let mut notes: Vec<String> = Vec::new();
    if cfg.enabled {
        if let Some(cmd) = command_for(path, cfg, &ctx.workdir) {
            let before = std::fs::read(path).ok();
            let o = crate::sandbox::run_shell(&cmd, &ctx.workdir, Duration::from_secs(cfg.timeout_secs), 4000).await.ok()?;
            let bin = cmd.split_whitespace().next().unwrap_or("formatter").trim_matches('\'');
            let bin = std::path::Path::new(bin).file_name().and_then(|b| b.to_str()).unwrap_or(bin);
            if !o.success() {
                let msg = format!("{}{}", o.stderr.trim(), o.stdout.trim());
                if !msg.trim().is_empty() { notes.push(format!("{bin} failed: {}", crate::llm::truncate_for_log(msg.trim(), 300))); }
            } else if std::fs::read(path).ok() != before {
                notes.push(format!("formatted with {bin}"));
            }
        }
    }
    if cfg.diagnostics_after_edit {
        if let Some(d) = diagnostics_note(path, ctx).await { notes.push(d); }
    }
    (!notes.is_empty()).then(|| format!("\n[{}]", notes.join(" · ")))
}

/// Errors/warnings for the file from a language server that is *already* running for this project
/// (never starts one — that would add seconds to the first edit).
async fn diagnostics_note(path: &Path, ctx: &crate::tools::ToolCtx) -> Option<String> {
    let servers = if ctx.lsp_servers.is_empty() { crate::lsp::default_servers() } else { ctx.lsp_servers.clone() };
    let (name, _cfg) = crate::lsp::server_for(path, &servers)?;
    let root = ctx.workdir.canonicalize().unwrap_or_else(|_| ctx.workdir.clone());
    let server = crate::lsp::LspServer::running(&name, &root)?;
    let uri = server.sync_doc(path).await.ok()?;
    let diags = server.wait_diagnostics(&uri, Duration::from_secs(6), Duration::from_millis(600)).await;
    let errs: Vec<String> = diags.iter().filter(|d| d["severity"].as_u64().unwrap_or(1) <= 2).take(8).map(|d| crate::lsp::fmt_diag(&uri, d, &root)).collect();
    if errs.is_empty() { return None; }
    Some(format!("{} diagnostic(s) from {name}:\n{}", errs.len(), errs.join("\n")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_configured_and_default_commands() {
        let wd = std::env::temp_dir();
        let mut cfg = FormatConfig::default();
        cfg.commands.insert("rs".into(), "myfmt --check {file}".into());
        let c = command_for(Path::new("/tmp/a.rs"), &cfg, &wd).unwrap();
        assert!(c.starts_with("myfmt --check ") && c.contains("a.rs"), "{c}");
        // an empty override disables formatting for that extension
        cfg.commands.insert("py".into(), String::new());
        assert!(command_for(Path::new("/tmp/a.py"), &cfg, &wd).is_none());
        // unknown extension → nothing
        assert!(command_for(Path::new("/tmp/a.unknownext"), &FormatConfig::default(), &wd).is_none());
        // defaults only fire when the binary exists
        let d = command_for(Path::new("/tmp/a.rs"), &FormatConfig::default(), &wd);
        assert_eq!(d.is_some(), crate::setup::which_in("rustfmt", &wd).is_some());
    }
}
