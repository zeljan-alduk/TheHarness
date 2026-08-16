//! Plugins: git repositories under ~/.config/harness/plugins/<name>. A plugin can provide
//!  - skills:   SKILL.md at the root, or skills/<name>/SKILL.md (frontmatter name/description)
//!  - commands: commands/<name>.md — becomes /name; $ARGUMENTS is replaced by what follows the command
//!  - MCP servers: mcp.json / .mcp.json ({"mcpServers": {...}}), or DSH `*.cordis.yml` entries that use
//!    `@deepseek-ai/dsh-mcp-client` (converted to the same shape)
//!  - manifest: harness-plugin.toml or plugin.json (name, description, version) — optional
//! TypeScript-only DSH plugins (Cordis `apply(ctx)` modules) cannot run inside this harness; their
//! skills / commands / MCP parts are still loaded and the rest is reported honestly.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const TOPICS: [&str; 2] = ["harness-plugin", "dsh-plugin"];

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct State { #[serde(default)] pub disabled: Vec<String> }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogEntry { pub full_name: String, pub description: String, pub stars: u64, pub language: String, pub topics: Vec<String>, pub url: String }
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Catalog { pub fetched_at: u64, pub entries: Vec<CatalogEntry> }

#[derive(Debug, Clone)]
pub struct Skill { pub name: String, pub description: String, pub path: PathBuf, pub plugin: String }
#[derive(Debug, Clone)]
pub struct Command { pub name: String, pub description: String, pub template: String, pub plugin: String }

#[derive(Debug, Clone)]
pub struct Plugin {
    pub name: String, pub path: PathBuf, pub description: String, pub version: String, pub enabled: bool,
    pub skills: Vec<Skill>, pub commands: Vec<Command>, pub mcp_files: Vec<PathBuf>, pub mcp_servers: Vec<String>, pub ts_only: bool, pub origin: Option<String>,
}

pub struct Plugins { pub dir: PathBuf, pub state: State }

impl Plugins {
    pub fn open() -> Result<Self> {
        let dir = std::env::var_os("HARNESS_PLUGINS_DIR").map(PathBuf::from)
            .unwrap_or_else(|| crate::setup::config_dir().join("plugins"));
        std::fs::create_dir_all(&dir)?;
        let state = std::fs::read_to_string(dir.join("state.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        Ok(Self { dir, state })
    }
    fn save_state(&self) -> Result<()> { Ok(std::fs::write(self.dir.join("state.json"), serde_json::to_string_pretty(&self.state)?)?) }

    /// All installed plugins (enabled or not), with what they provide.
    pub fn installed(&self) -> Vec<Plugin> {
        let mut out = Vec::new();
        let Ok(rd) = std::fs::read_dir(&self.dir) else { return out };
        for e in rd.flatten() {
            let p = e.path();
            if !p.is_dir() || p.file_name().map(|n| n.to_string_lossy().starts_with('.')).unwrap_or(true) { continue; }
            out.push(self.inspect(&p));
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    pub fn inspect(&self, path: &Path) -> Plugin {
        let dirname = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let (mut name, mut description, mut version) = (dirname.clone(), String::new(), String::new());
        // manifests
        if let Ok(t) = std::fs::read_to_string(path.join("harness-plugin.toml")) {
            if let Ok(v) = t.parse::<toml::Value>() { name = v.get("name").and_then(|x| x.as_str()).unwrap_or(&name).to_string(); description = v.get("description").and_then(|x| x.as_str()).unwrap_or("").to_string(); version = v.get("version").and_then(|x| x.as_str()).unwrap_or("").to_string(); }
        } else if let Ok(t) = std::fs::read_to_string(path.join("plugin.json")).or_else(|_| std::fs::read_to_string(path.join(".claude-plugin/plugin.json"))) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&t) { name = v["name"].as_str().unwrap_or(&name).to_string(); description = v["description"].as_str().unwrap_or("").to_string(); version = v["version"].as_str().unwrap_or("").to_string(); }
        } else if let Ok(t) = std::fs::read_to_string(path.join("package.json")) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&t) { description = v["description"].as_str().unwrap_or("").to_string(); version = v["version"].as_str().unwrap_or("").to_string(); }
        }
        if description.is_empty() { description = first_readme_line(path); }
        // skills
        let mut skills = Vec::new();
        let mut cands: Vec<PathBuf> = Vec::new();
        find_files(path, "SKILL.md", 3, &mut cands);
        cands.sort(); cands.dedup();
        for cand in cands {
            if let Ok(t) = std::fs::read_to_string(&cand) {
                let (fm_name, fm_desc) = frontmatter(&t);
                let sname = fm_name.unwrap_or_else(|| cand.parent().and_then(|d| d.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or(dirname.clone()));
                skills.push(Skill { name: sname, description: fm_desc.unwrap_or_else(|| first_line(&t)), path: cand.clone(), plugin: dirname.clone() });
            }
        }
        // commands
        let mut commands = Vec::new();
        for dir in [path.join("commands"), path.join(".claude/commands")] {
            if let Ok(rd) = std::fs::read_dir(&dir) {
                for e in rd.flatten() {
                    let p = e.path();
                    if p.extension().map(|x| x == "md").unwrap_or(false) {
                        if let Ok(t) = std::fs::read_to_string(&p) {
                            let (_, d) = frontmatter(&t);
                            let cname = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                            commands.push(Command { name: cname, description: d.unwrap_or_else(|| first_line(&t)), template: strip_frontmatter(&t), plugin: dirname.clone() });
                        }
                    }
                }
            }
        }
        // mcp
        let mut mcp_files = Vec::new(); let mut mcp_servers = Vec::new();
        for f in ["mcp.json", ".mcp.json", ".harness/mcp.json"] { let p = path.join(f); if p.is_file() { if let Ok(t) = std::fs::read_to_string(&p) { if let Ok(m) = serde_json::from_str::<crate::mcp::McpFile>(&t) { mcp_servers.extend(m.mcp_servers.keys().cloned()); } } mcp_files.push(p); } }
        // DSH cordis.yml → generated mcp json
        let mut dsh_entries = serde_json::Map::new();
        for y in std::fs::read_dir(path).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.file_name().map(|n| n.to_string_lossy().ends_with(".cordis.yml") || n.to_string_lossy() == "cordis.yml").unwrap_or(false)) {
            if let Ok(t) = std::fs::read_to_string(&y) {
                if let Ok(doc) = serde_yaml::from_str::<serde_yaml::Value>(&t.replace("!!js ", "")) { collect_dsh_mcp(&doc, &mut dsh_entries); }
            }
        }
        if !dsh_entries.is_empty() {
            let gen = path.join(".harness-mcp.generated.json");
            let _ = std::fs::write(&gen, serde_json::to_string_pretty(&serde_json::json!({"mcpServers": dsh_entries})).unwrap_or_default());
            mcp_servers.extend(dsh_entries.keys().cloned());
            mcp_files.push(gen);
        }
        // TS-only?
        let has_ts = path.join("package.json").is_file() && (path.join("src").is_dir() || path.join("index.ts").is_file());
        let ts_only = has_ts && skills.is_empty() && commands.is_empty() && mcp_servers.is_empty();
        let origin = std::process::Command::new("git").args(["-C", &path.display().to_string(), "remote", "get-url", "origin"]).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
        Plugin { enabled: !self.state.disabled.contains(&dirname), name, path: path.to_path_buf(), description, version, skills, commands, mcp_files, mcp_servers, ts_only, origin }
    }

    pub fn enabled(&self) -> Vec<Plugin> { self.installed().into_iter().filter(|p| p.enabled).collect() }

    pub fn set_enabled(&mut self, dirname: &str, on: bool) -> Result<()> {
        if !self.dir.join(dirname).is_dir() { bail!("no installed plugin named '{dirname}'"); }
        self.state.disabled.retain(|d| d != dirname);
        if !on { self.state.disabled.push(dirname.to_string()); }
        self.save_state()
    }

    /// Install from `owner/repo`, a git URL, or a local path. Returns the plugin dir name.
    pub async fn install(&self, spec: &str) -> Result<String> {
        let (url, name) = if spec.starts_with("http://") || spec.starts_with("https://") || spec.starts_with("git@") {
            (spec.to_string(), spec.trim_end_matches('/').trim_end_matches(".git").rsplit('/').next().unwrap_or("plugin").to_string())
        } else if Path::new(spec).is_dir() {
            let p = Path::new(spec).canonicalize()?; let name = p.file_name().unwrap().to_string_lossy().to_string();
            let dst = self.dir.join(&name); if dst.exists() { bail!("'{name}' already installed"); }
            #[cfg(unix)] { std::os::unix::fs::symlink(&p, &dst)?; }
            #[cfg(windows)] { copy_dir(&p, &dst)?; }
            return Ok(name);
        } else if spec.matches('/').count() == 1 {
            (format!("https://github.com/{spec}.git"), spec.split('/').nth(1).unwrap().to_string())
        } else { bail!("install spec must be owner/repo, a git URL, or a local directory") };
        let dst = self.dir.join(&name);
        if dst.exists() { bail!("'{name}' already installed (use /plugin update {name})"); }
        let o = tokio::process::Command::new("git").args(["clone", "-q", "--depth", "1", &url]).arg(&dst).output().await?;
        if !o.status.success() { bail!("git clone failed: {}", String::from_utf8_lossy(&o.stderr).trim()); }
        Ok(name)
    }
    pub async fn update(&self, name: &str) -> Result<String> {
        let d = self.dir.join(name); if !d.is_dir() { bail!("no installed plugin named '{name}'"); }
        let o = tokio::process::Command::new("git").args(["-C", &d.display().to_string(), "pull", "-q", "--ff-only"]).output().await?;
        if !o.status.success() { bail!("git pull failed: {}", String::from_utf8_lossy(&o.stderr).trim()); }
        Ok(format!("updated {name}"))
    }
    /// Update every installed git plugin (`git pull --ff-only`). Returns per-plugin results.
    pub async fn update_all(&self) -> Vec<(String, Result<String>)> {
        let mut out = Vec::new();
        for p in self.installed() { let name = p.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(); if p.path.join(".git").exists() { out.push((name.clone(), self.update(&name).await)); } }
        out
    }
    /// Plugins whose last fetch is older than `days` (by .git/FETCH_HEAD or HEAD mtime).
    pub fn stale(&self, days: u64) -> Vec<String> {
        let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(days * 86400);
        self.installed().into_iter().filter(|p| p.path.join(".git").exists()).filter(|p| { let f = p.path.join(".git/FETCH_HEAD"); let h = p.path.join(".git/HEAD"); let m = std::fs::metadata(&f).or_else(|_| std::fs::metadata(&h)).and_then(|m| m.modified()).ok(); m.map(|t| t < cutoff).unwrap_or(false) }).map(|p| p.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()).collect()
    }
    pub fn remove(&mut self, name: &str) -> Result<()> {
        let d = self.dir.join(name); if !d.exists() { bail!("no installed plugin named '{name}'"); }
        if d.is_symlink() { std::fs::remove_file(&d)?; } else { std::fs::remove_dir_all(&d)?; }
        self.state.disabled.retain(|x| x != name); self.save_state()
    }

    /// Catalog of downloadable plugins from the GitHub topics (cached ~6h).
    pub async fn catalog(&self, refresh: bool) -> Result<Catalog> {
        let cache = self.dir.join("catalog.json");
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        if !refresh { if let Ok(t) = std::fs::read_to_string(&cache) { if let Ok(c) = serde_json::from_str::<Catalog>(&t) { if now.saturating_sub(c.fetched_at) < 6 * 3600 && !c.entries.is_empty() { return Ok(c); } } } }
        let http = reqwest::Client::builder().timeout(std::time::Duration::from_secs(20)).user_agent("harness-plugins").build()?;
        let mut entries: Vec<CatalogEntry> = Vec::new();
        for topic in TOPICS {
            let url = format!("https://api.github.com/search/repositories?q=topic:{topic}&sort=stars&order=desc&per_page=50");
            let mut req = http.get(&url).header("Accept", "application/vnd.github+json");
            if let Ok(tok) = std::env::var("GITHUB_TOKEN") { req = req.bearer_auth(tok); }
            let r = req.send().await.with_context(|| format!("GitHub search for topic {topic}"))?;
            if !r.status().is_success() { let st = r.status(); let body = r.text().await.unwrap_or_default(); bail!("GitHub API {st}: {}", crate::llm::truncate_for_log(&body, 200)); }
            let v: serde_json::Value = r.json().await?;
            for it in v["items"].as_array().cloned().unwrap_or_default() {
                let full = it["full_name"].as_str().unwrap_or("").to_string();
                if entries.iter().any(|e| e.full_name == full) { continue; }
                entries.push(CatalogEntry { full_name: full, description: it["description"].as_str().unwrap_or("").to_string(), stars: it["stargazers_count"].as_u64().unwrap_or(0), language: it["language"].as_str().unwrap_or("").to_string(), topics: it["topics"].as_array().map(|a| a.iter().filter_map(|t| t.as_str().map(String::from)).collect()).unwrap_or_default(), url: it["html_url"].as_str().unwrap_or("").to_string() });
            }
        }
        entries.sort_by(|a, b| b.stars.cmp(&a.stars));
        let c = Catalog { fetched_at: now, entries };
        let _ = std::fs::write(&cache, serde_json::to_string_pretty(&c)?);
        Ok(c)
    }

    /// Text for the system prompt: skills and commands the model can use.
    pub fn prompt_block(&self) -> String {
        let en = self.enabled();
        let skills: Vec<&Skill> = en.iter().flat_map(|p| p.skills.iter()).collect();
        if skills.is_empty() { return String::new(); }
        let mut s = String::from("\n\n# Skills (from plugins)\nWhen a task matches a skill, call load_skill {name} first and follow its instructions.\n");
        for sk in skills { s.push_str(&format!("- {} — {} [{}]\n", sk.name, crate::llm::truncate_for_log(&sk.description, 140), sk.plugin)); }
        s
    }
    pub fn find_skill(&self, name: &str) -> Option<Skill> {
        let n = name.trim().to_lowercase();
        self.enabled().into_iter().flat_map(|p| p.skills).find(|s| s.name.to_lowercase() == n || s.name.to_lowercase().replace(' ', "-") == n)
    }
    pub fn commands(&self) -> Vec<Command> { self.enabled().into_iter().flat_map(|p| p.commands).collect() }
    pub fn mcp_files(&self) -> Vec<PathBuf> { self.enabled().into_iter().flat_map(|p| p.mcp_files).collect() }
}

fn collect_dsh_mcp(v: &serde_yaml::Value, out: &mut serde_json::Map<String, serde_json::Value>) {
    match v {
        serde_yaml::Value::Sequence(items) => for it in items { collect_dsh_mcp(it, out); },
        serde_yaml::Value::Mapping(m) => {
            let name = m.get("name").and_then(|x| x.as_str()).unwrap_or("");
            if name.contains("dsh-mcp-client") || name.contains("mcp-client") {
                if let Some(cfg) = m.get("config").and_then(|c| c.as_mapping()) {
                    let sname = cfg.get("serverName").and_then(|x| x.as_str()).or_else(|| m.get("id").and_then(|x| x.as_str())).unwrap_or("mcp").to_string();
                    let cmd = cfg.get("command").and_then(|x| x.as_str()).unwrap_or("").to_string();
                    if !cmd.is_empty() && cfg.get("transport").and_then(|x| x.as_str()).map(|t| t == "stdio").unwrap_or(true) {
                        let args: Vec<String> = cfg.get("args").and_then(|a| a.as_sequence()).map(|s| s.iter().filter_map(|x| x.as_str().map(String::from).or_else(|| x.as_i64().map(|n| n.to_string()))).collect()).unwrap_or_default();
                        let env: serde_json::Map<String, serde_json::Value> = cfg.get("env").and_then(|e| e.as_mapping()).map(|mm| mm.iter().filter_map(|(k, v)| Some((k.as_str()?.to_string(), serde_json::Value::String(v.as_str().unwrap_or("").to_string())))).collect()).unwrap_or_default();
                        out.insert(sname, serde_json::json!({"command": cmd, "args": args, "env": env}));
                    }
                }
            }
            for (_, val) in m { collect_dsh_mcp(val, out); }
        }
        _ => {}
    }
}

#[allow(dead_code)]
fn copy_dir(src: &Path, dst: &Path) -> Result<()> { std::fs::create_dir_all(dst)?; for e in std::fs::read_dir(src)? { let e = e?; let to = dst.join(e.file_name()); if e.file_type()?.is_dir() { copy_dir(&e.path(), &to)?; } else { std::fs::copy(e.path(), &to)?; } } Ok(()) }

fn find_files(d: &Path, name: &str, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(d) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let fname = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if p.is_dir() { if depth > 0 && !matches!(fname.as_str(), "node_modules" | ".git" | "target" | "dist" | "build") { find_files(&p, name, depth - 1, out); } }
        else if fname.eq_ignore_ascii_case(name) { out.push(p); }
    }
}
#[allow(dead_code)]
fn glob_dir(d: &Path) -> Vec<PathBuf> { std::fs::read_dir(d).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect() }
fn first_line(t: &str) -> String {
    t.lines().map(|l| l.trim().trim_start_matches('#').trim())
        .find(|l| !l.is_empty() && !l.starts_with("---") && !l.starts_with('<') && !l.starts_with("![") && !l.starts_with("[!["))
        .unwrap_or("").chars().take(200).collect()
}
fn first_readme_line(p: &Path) -> String { ["README.md", "readme.md", "README"].iter().find_map(|n| std::fs::read_to_string(p.join(n)).ok()).map(|t| first_line(&strip_frontmatter(&t))).unwrap_or_default() }
/// (name, description) from YAML frontmatter.
fn frontmatter(t: &str) -> (Option<String>, Option<String>) {
    let t = t.trim_start();
    if !t.starts_with("---") { return (None, None); }
    let rest = &t[3..]; let Some(end) = rest.find("\n---") else { return (None, None) };
    let fm = &rest[..end];
    let get = |k: &str| fm.lines().find_map(|l| l.strip_prefix(&format!("{k}:")).map(|v| v.trim().trim_matches('"').trim_matches('\'').to_string())).filter(|v| !v.is_empty());
    (get("name"), get("description"))
}
fn strip_frontmatter(t: &str) -> String {
    let tt = t.trim_start();
    if tt.starts_with("---") { if let Some(end) = tt[3..].find("\n---") { return tt[3 + end + 4..].trim_start().to_string(); } }
    t.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_frontmatter_and_dsh_yaml() {
        let (n, d) = frontmatter("---\nname: my-skill\ndescription: Does things\n---\n# Body");
        assert_eq!(n.as_deref(), Some("my-skill")); assert_eq!(d.as_deref(), Some("Does things"));
        let y: serde_yaml::Value = serde_yaml::from_str("- insert:\n    - id: memory-engram\n      name: '@deepseek-ai/dsh-mcp-client'\n      config:\n        serverName: engram\n        transport: stdio\n        command: engram\n        args: [mcp]\n").unwrap();
        let mut out = serde_json::Map::new(); collect_dsh_mcp(&y, &mut out);
        assert_eq!(out["engram"]["command"], "engram"); assert_eq!(out["engram"]["args"][0], "mcp");
    }
}
