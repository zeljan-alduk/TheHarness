//! External tool management: one folder (`~/.config/harness/bin`) that gathers every CLI the agent
//! relies on, so nothing has to be searched for. `harness setup` audits, symlinks what exists, and
//! installs what is missing (Homebrew on macOS). The sandbox puts this folder first on PATH.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub struct ExtTool { pub name: &'static str, pub bins: &'static [&'static str], pub purpose: &'static str, pub brew: Option<&'static str>, pub cask: bool, pub required: bool, pub any_of: bool, pub other_install: Option<&'static str> }

pub const TOOLS: &[ExtTool] = &[
    ExtTool { name: "git",      bins: &["git"],               purpose: "version control (agent undo/branches)",      brew: Some("git"),      cask: false, required: true, any_of: false, other_install: None },
    ExtTool { name: "python3",  bins: &["python3"],           purpose: "scripting, quick checks",                   brew: Some("python"),   cask: false, required: true, any_of: false, other_install: None },
    ExtTool { name: "ripgrep",  bins: &["rg"],                purpose: "fast code search",                          brew: Some("ripgrep"),  cask: false, required: false, any_of: false, other_install: None },
    ExtTool { name: "fd",       bins: &["fd"],                purpose: "fast file find",                            brew: Some("fd"),       cask: false, required: false, any_of: false, other_install: None },
    ExtTool { name: "jq",       bins: &["jq"],                purpose: "JSON processing",                           brew: Some("jq"),       cask: false, required: false, any_of: false, other_install: None },
    ExtTool { name: "ffmpeg",   bins: &["ffmpeg", "ffprobe"], purpose: "video frames / audio / media conversion",   brew: Some("ffmpeg"),   cask: false, required: false, any_of: false, other_install: None },
    ExtTool { name: "poppler",  bins: &["pdftotext", "pdftoppm"], purpose: "read PDFs (read_pdf)", brew: Some("poppler"),  cask: false, required: false, any_of: false, other_install: None },
    ExtTool { name: "7-zip",    bins: &["7z", "7zz"],         purpose: "archives incl. .7z/.rar (extract_archive)", brew: Some("sevenzip"), cask: false, required: false, any_of: true, other_install: None },
    ExtTool { name: "unzip",    bins: &["unzip", "zip"],      purpose: "zip archives",                              brew: None,             cask: false, required: true, any_of: false, other_install: None },
    ExtTool { name: "tar",      bins: &["tar", "gzip"],       purpose: "tar/gzip archives",                         brew: None,             cask: false, required: true, any_of: false, other_install: None },
    ExtTool { name: "curl",     bins: &["curl"],              purpose: "HTTP from the shell",                       brew: Some("curl"),     cask: false, required: true, any_of: false, other_install: None },
    ExtTool { name: "uv",       bins: &["uv"],                purpose: "Python envs without global installs; PyMuPDF for pdf_edit", brew: Some("uv"),   cask: false, required: false, any_of: false, other_install: None },
    ExtTool { name: "node",     bins: &["node", "npm"],       purpose: "JavaScript tooling",                        brew: Some("node"),     cask: false, required: false, any_of: false, other_install: None },
    ExtTool { name: "gh",       bins: &["gh"],                purpose: "GitHub CLI",                                brew: Some("gh"),       cask: false, required: false, any_of: false, other_install: None },
    ExtTool { name: "imagemagick", bins: &["magick"],         purpose: "image conversion/resizing",                 brew: Some("imagemagick"), cask: false, required: false, any_of: false, other_install: None },
    ExtTool { name: "rust-analyzer", bins: &["rust-analyzer"], purpose: "Rust language server (lsp tool)",     brew: None,             cask: false, required: false, any_of: false, other_install: Some("rustup component add rust-analyzer") },
    ExtTool { name: "pyright",  bins: &["pyright-langserver"], purpose: "Python language server (lsp tool)",  brew: None,             cask: false, required: false, any_of: false, other_install: Some("npm install -g pyright") },
    ExtTool { name: "typescript-language-server", bins: &["typescript-language-server"], purpose: "TS/JS language server (lsp tool)", brew: None, cask: false, required: false, any_of: false, other_install: Some("npm install -g typescript-language-server typescript") },
    ExtTool { name: "gopls",    bins: &["gopls"],             purpose: "Go language server (lsp tool)",         brew: Some("gopls"),    cask: false, required: false, any_of: false, other_install: None },
    ExtTool { name: "macmon",   bins: &["macmon"],            purpose: "CPU/GPU temperature & power in the dashboard", brew: Some("macmon"), cask: false, required: false, any_of: false, other_install: None },
    ExtTool { name: "kitty",    bins: &["kitty"],             purpose: "terminal with inline image previews",       brew: Some("kitty"),    cask: true,  required: false, any_of: false, other_install: None },
];

#[derive(Debug, Clone)]
pub struct Status { pub name: &'static str, pub found: Vec<(String, PathBuf)>, pub missing: Vec<&'static str>, pub purpose: &'static str, pub required: bool, pub install: Option<String> }
impl Status { pub fn ok(&self) -> bool { self.missing.is_empty() } }

/// The user's home directory (HOME, or USERPROFILE on Windows).
pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}
/// ~/.config/harness (all harness state lives here on every platform); HARNESS_CONFIG_DIR overrides it.
pub fn config_dir() -> PathBuf {
    std::env::var_os("HARNESS_CONFIG_DIR").map(PathBuf::from).unwrap_or_else(|| home_dir().join(".config/harness"))
}

pub fn bin_dir() -> PathBuf {
    std::env::var_os("HARNESS_BIN_DIR").map(PathBuf::from).unwrap_or_else(|| config_dir().join("bin"))
}

pub fn which(bin: &str) -> Option<PathBuf> {
    let extra = ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin", "/usr/sbin", "/sbin", "/Applications/kitty.app/Contents/MacOS"];
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    dirs.extend(extra.iter().map(PathBuf::from));
    let h = home_dir(); dirs.push(h.join(".cargo/bin")); dirs.push(h.join(".local/bin"));
    let exts: &[&str] = if cfg!(windows) { &["", ".exe", ".cmd", ".bat"] } else { &[""] };
    dirs.into_iter().flat_map(|d| exts.iter().map(move |e| d.join(format!("{bin}{e}")))).find(|p| p.is_file())
}

/// `which`, but also honouring the harness bin dir and the project's own `node_modules/.bin`
/// (that is where prettier/biome/eslint live in a JS project).
pub fn which_in(bin: &str, workdir: &Path) -> Option<PathBuf> {
    let local = workdir.join("node_modules").join(".bin").join(bin);
    if local.is_file() { return Some(local); }
    let hb = bin_dir().join(bin);
    if hb.is_file() { return Some(hb); }
    which(bin)
}

pub fn check() -> Vec<Status> {
    TOOLS.iter().map(|t| {
        let mut found = Vec::new(); let mut missing = Vec::new();
        for b in t.bins { match which(b) { Some(p) => found.push((b.to_string(), p)), None => missing.push(*b) } }
        if t.any_of && !found.is_empty() { missing.clear(); }
        // rustup proxies exist even when the component is not installed: verify it runs
        if t.name == "rust-analyzer" && !found.is_empty() { let ok = std::process::Command::new("rust-analyzer").arg("--version").output().map(|o| o.status.success()).unwrap_or(false); if !ok { found.clear(); missing = vec!["rust-analyzer"]; } }
        let install = t.brew.map(|f| if t.cask { format!("brew install --cask {f}") } else { format!("brew install {f}") }).or_else(|| t.other_install.map(String::from));
        Status { name: t.name, found, missing, purpose: t.purpose, required: t.required, install }
    }).collect()
}

/// Populate the harness bin dir with symlinks to every found binary. Returns (linked, dir).
pub fn link_all(statuses: &[Status]) -> Result<(usize, PathBuf)> {
    let dir = bin_dir();
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let mut n = 0;
    for s in statuses {
        for (bin, path) in &s.found {
            let link = dir.join(bin);
            if link.read_link().map(|t| &t == path).unwrap_or(false) { n += 1; continue; }
            let _ = std::fs::remove_file(&link);
            #[cfg(unix)] { std::os::unix::fs::symlink(path, &link)?; }
            #[cfg(windows)] { let shim = dir.join(format!("{bin}.cmd")); std::fs::write(&shim, format!("@echo off\r\n\"{}\" %*\r\n", path.display()))?; }
            n += 1;
        }
    }
    Ok((n, dir))
}

/// Install missing tools with Homebrew (macOS). Streams brew's output to the terminal.
pub fn install_missing(statuses: &[Status]) -> Result<Vec<String>> {
    let mut done = Vec::new();
    for s in statuses.iter().filter(|s| !s.ok()) {
        let Some(cmd) = &s.install else { eprintln!("  {}: no installer known (system tool?)", s.name); continue };
        eprintln!("→ {cmd}");
        let (prog, flag) = crate::sandbox::shell_program();
        let st = std::process::Command::new(prog).arg(flag).arg(cmd).status()?;
        if st.success() { done.push(s.name.to_string()); } else { eprintln!("  failed: {cmd}"); }
    }
    Ok(done)
}

/// One line for the system prompt: what the agent can rely on.
pub fn summary_line() -> String {
    let st = check();
    let ok: Vec<&str> = st.iter().filter(|s| s.ok()).map(|s| s.name).collect();
    let miss: Vec<&str> = st.iter().filter(|s| !s.ok()).map(|s| s.name).collect();
    let mut line = format!("External CLIs available (also symlinked in {}): {}.", bin_dir().display(), ok.join(", "));
    if !miss.is_empty() { line.push_str(&format!(" Not installed: {} — tell the user to run `harness setup --install`.", miss.join(", "))); }
    line
}

pub fn print_report(statuses: &[Status]) {
    for s in statuses {
        let mark = if s.ok() { "✓" } else if s.required { "✗" } else { "·" };
        let where_ = s.found.first().map(|(_, p)| p.parent().map(|d| d.display().to_string()).unwrap_or_default()).unwrap_or_default();
        println!("{mark} {:<12} {:<52} {}", s.name, s.purpose, if s.ok() { where_ } else { format!("missing: {} → {}", s.missing.join(", "), s.install.clone().unwrap_or("(system)".into())) });
    }
}

pub fn path_with_bin_dir(_cwd: &Path) -> std::ffi::OsString {
    let dir = bin_dir();
    let mut paths: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    if !paths.contains(&dir) { paths.insert(0, dir); }
    std::env::join_paths(paths).unwrap_or_else(|_| std::env::var_os("PATH").unwrap_or_default())
}

/// Write recommended MCP servers into ~/.config/harness/mcp.json (merging; never overwrites existing names).
pub fn write_default_mcp() -> Result<Vec<String>> {
    let path = config_dir().join("mcp.json");
    let mut doc: serde_json::Value = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(serde_json::json!({"mcpServers": {}}));
    if !doc["mcpServers"].is_object() { doc["mcpServers"] = serde_json::json!({}); }
    let defaults = [
        ("chrome-devtools", serde_json::json!({"command": "npx", "args": ["-y", "chrome-devtools-mcp@latest"], "disabled": false})),
        ("playwright", serde_json::json!({"command": "npx", "args": ["-y", "@playwright/mcp@latest"], "disabled": true})),
        ("filesystem-home", serde_json::json!({"command": "npx", "args": ["-y", "@modelcontextprotocol/server-filesystem", "${HOME}"], "disabled": true})),
    ];
    let mut added = Vec::new();
    for (name, cfg) in defaults { if doc["mcpServers"].get(name).is_none() { doc["mcpServers"][name] = cfg; added.push(name.to_string()); } }
    std::fs::create_dir_all(path.parent().unwrap())?;
    std::fs::write(&path, serde_json::to_string_pretty(&doc)?)?;
    Ok(added)
}
