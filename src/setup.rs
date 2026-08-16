//! External tool management: one folder (`~/.config/harness/bin`) that gathers every CLI the agent
//! relies on, so nothing has to be searched for. `harness setup` audits, symlinks what exists, and
//! installs what is missing (Homebrew on macOS). The sandbox puts this folder first on PATH.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub struct ExtTool { pub name: &'static str, pub bins: &'static [&'static str], pub purpose: &'static str, pub brew: Option<&'static str>, pub cask: bool, pub required: bool, pub any_of: bool }

pub const TOOLS: &[ExtTool] = &[
    ExtTool { name: "git",      bins: &["git"],               purpose: "version control (agent undo/branches)",      brew: Some("git"),      cask: false, required: true, any_of: false },
    ExtTool { name: "python3",  bins: &["python3"],           purpose: "scripting, quick checks",                   brew: Some("python"),   cask: false, required: true, any_of: false },
    ExtTool { name: "ripgrep",  bins: &["rg"],                purpose: "fast code search",                          brew: Some("ripgrep"),  cask: false, required: false, any_of: false },
    ExtTool { name: "fd",       bins: &["fd"],                purpose: "fast file find",                            brew: Some("fd"),       cask: false, required: false, any_of: false },
    ExtTool { name: "jq",       bins: &["jq"],                purpose: "JSON processing",                           brew: Some("jq"),       cask: false, required: false, any_of: false },
    ExtTool { name: "ffmpeg",   bins: &["ffmpeg", "ffprobe"], purpose: "video frames / audio / media conversion",   brew: Some("ffmpeg"),   cask: false, required: false, any_of: false },
    ExtTool { name: "poppler",  bins: &["pdftotext", "pdftoppm"], purpose: "read PDFs (read_pdf), render PDF pages", brew: Some("poppler"),  cask: false, required: false, any_of: false },
    ExtTool { name: "7-zip",    bins: &["7z", "7zz"],         purpose: "archives incl. .7z/.rar (extract_archive)", brew: Some("sevenzip"), cask: false, required: false, any_of: true },
    ExtTool { name: "unzip",    bins: &["unzip", "zip"],      purpose: "zip archives",                              brew: None,             cask: false, required: true, any_of: false },
    ExtTool { name: "tar",      bins: &["tar", "gzip"],       purpose: "tar/gzip archives",                         brew: None,             cask: false, required: true, any_of: false },
    ExtTool { name: "curl",     bins: &["curl"],              purpose: "HTTP from the shell",                       brew: Some("curl"),     cask: false, required: true, any_of: false },
    ExtTool { name: "uv",       bins: &["uv"],                purpose: "Python envs and tools without global installs", brew: Some("uv"),   cask: false, required: false, any_of: false },
    ExtTool { name: "node",     bins: &["node", "npm"],       purpose: "JavaScript tooling",                        brew: Some("node"),     cask: false, required: false, any_of: false },
    ExtTool { name: "gh",       bins: &["gh"],                purpose: "GitHub CLI",                                brew: Some("gh"),       cask: false, required: false, any_of: false },
    ExtTool { name: "imagemagick", bins: &["magick"],         purpose: "image conversion/resizing",                 brew: Some("imagemagick"), cask: false, required: false, any_of: false },
    ExtTool { name: "kitty",    bins: &["kitty"],             purpose: "terminal with inline image previews",       brew: Some("kitty"),    cask: true,  required: false, any_of: false },
];

#[derive(Debug, Clone)]
pub struct Status { pub name: &'static str, pub found: Vec<(String, PathBuf)>, pub missing: Vec<&'static str>, pub purpose: &'static str, pub required: bool, pub install: Option<String> }
impl Status { pub fn ok(&self) -> bool { self.missing.is_empty() } }

pub fn bin_dir() -> PathBuf {
    std::env::var_os("HARNESS_BIN_DIR").map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config/harness/bin"))
}

fn which(bin: &str) -> Option<PathBuf> {
    let extra = ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin", "/usr/sbin", "/sbin", "/Applications/kitty.app/Contents/MacOS"];
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    dirs.extend(extra.iter().map(PathBuf::from));
    if let Ok(h) = std::env::var("HOME") { dirs.push(PathBuf::from(&h).join(".cargo/bin")); dirs.push(PathBuf::from(&h).join(".local/bin")); }
    dirs.into_iter().map(|d| d.join(bin)).find(|p| p.is_file())
}

pub fn check() -> Vec<Status> {
    TOOLS.iter().map(|t| {
        let mut found = Vec::new(); let mut missing = Vec::new();
        for b in t.bins { match which(b) { Some(p) => found.push((b.to_string(), p)), None => missing.push(*b) } }
        if t.any_of && !found.is_empty() { missing.clear(); }
        let install = t.brew.map(|f| if t.cask { format!("brew install --cask {f}") } else { format!("brew install {f}") });
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
            n += 1;
        }
    }
    Ok((n, dir))
}

/// Install missing tools with Homebrew (macOS). Streams brew's output to the terminal.
pub fn install_missing(statuses: &[Status]) -> Result<Vec<String>> {
    let brew = which("brew").context("Homebrew not found — install it from https://brew.sh then rerun `harness setup --install`")?;
    let mut done = Vec::new();
    for s in statuses.iter().filter(|s| !s.ok()) {
        let Some(cmd) = &s.install else { eprintln!("  {}: no installer known (system tool?)", s.name); continue };
        let args: Vec<&str> = cmd.split_whitespace().skip(1).collect();
        eprintln!("→ {cmd}");
        let st = std::process::Command::new(&brew).args(&args).status()?;
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

pub fn path_with_bin_dir(_cwd: &Path) -> String {
    let dir = bin_dir();
    let cur = std::env::var("PATH").unwrap_or_default();
    if cur.split(':').any(|p| Path::new(p) == dir) { cur } else { format!("{}:{cur}", dir.display()) }
}
