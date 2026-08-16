use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub llm: LlmConfig,
    pub agent: AgentConfig,
    #[serde(default)]
    pub eval: EvalConfig,
    #[serde(default)]
    pub net: NetConfig,
    #[serde(default)]
    pub memory: crate::memory::MemoryConfig,
    #[serde(default)]
    pub permissions: crate::permissions::PermissionsConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LlmConfig {
    pub base_url: String,
    pub model: String,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default = "d_temp")]
    pub temperature: f32,
    #[serde(default = "d_max_tokens")]
    pub max_tokens: u32,
    /// Prompt-token threshold for auto-compaction. If unset, derived at start: compact_at_fraction × detected context.
    #[serde(default)]
    pub context_budget_tokens: Option<u64>,
    #[serde(default = "d_compact_frac")]
    pub compact_at_fraction: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentConfig {
    #[serde(default = "d_max_turns")]
    pub max_turns: usize,
    #[serde(default = "d_tool_timeout")]
    pub tool_timeout_secs: u64,
    #[serde(default = "d_max_out")]
    pub max_tool_output_chars: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EvalConfig {
    #[serde(default = "d_tasks_dir")]
    pub tasks_dir: String,
    #[serde(default = "d_runs_dir")]
    pub runs_dir: String,
    #[serde(default = "d_task_timeout")]
    pub task_timeout_secs: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NetConfig {
    /// Expose web_fetch / web_search tools to the model.
    #[serde(default = "d_true")]
    pub enabled: bool,
    #[serde(default = "d_fetch_timeout")]
    pub timeout_secs: u64,
    /// Max bytes downloaded per fetch.
    #[serde(default = "d_fetch_bytes")]
    pub max_fetch_bytes: usize,
    #[serde(default = "d_ua")]
    pub user_agent: String,
    /// Parallel segments for download_file on servers that support HTTP ranges.
    #[serde(default = "d_dl_segments")]
    pub download_segments: usize,
    /// Wall-clock cap for a single download_file call (it resumes on the next call).
    #[serde(default = "d_dl_timeout")]
    pub download_timeout_secs: u64,
}

fn d_temp() -> f32 { 0.2 }
fn d_max_tokens() -> u32 { 16384 }
fn d_compact_frac() -> f64 { 0.75 }
fn d_max_turns() -> usize { 40 }
fn d_tool_timeout() -> u64 { 120 }
fn d_max_out() -> usize { 16000 }
fn d_tasks_dir() -> String { "evals/tasks".into() }
fn d_runs_dir() -> String { std::env::temp_dir().join("harness-eval-runs").display().to_string() }
fn d_task_timeout() -> u64 { 900 }
fn d_true() -> bool { true }
fn d_fetch_timeout() -> u64 { 30 }
fn d_fetch_bytes() -> usize { 2_000_000 }
fn d_dl_segments() -> usize { 4 }
fn d_dl_timeout() -> u64 { 3600 }
fn d_ua() -> String { "Mozilla/5.0 (compatible; harness/0.1; +https://github.com/) ".into() }

impl Default for EvalConfig {
    fn default() -> Self {
        Self { tasks_dir: d_tasks_dir(), runs_dir: d_runs_dir(), task_timeout_secs: d_task_timeout() }
    }
}
impl Default for NetConfig {
    fn default() -> Self {
        Self { enabled: true, timeout_secs: d_fetch_timeout(), max_fetch_bytes: d_fetch_bytes(), user_agent: d_ua(), download_segments: d_dl_segments(), download_timeout_secs: d_dl_timeout() }
    }
}

impl LlmConfig {
    /// The compaction threshold given a detected context length (if any).
    pub fn effective_budget(&self, detected_ctx: Option<u64>) -> u64 {
        if let Some(b) = self.context_budget_tokens { return b; }
        match detected_ctx { Some(n) => ((n as f64) * self.compact_at_fraction) as u64, None => 60_000 }
    }
}

impl Config {
    /// Lookup order: --config / $HARNESS_CONFIG, ./harness.toml, ~/.config/harness/harness.toml,
    /// next to the executable, and the repo root when running via cargo.
    pub fn load(explicit: Option<&Path>) -> Result<Config> {
        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Some(p) = explicit { candidates.push(p.to_path_buf()); }
        if let Some(p) = std::env::var_os("HARNESS_CONFIG") { candidates.push(PathBuf::from(p)); }
        candidates.push(PathBuf::from("harness.toml"));
        if let Some(home) = std::env::var_os("HOME") { candidates.push(PathBuf::from(home).join(".config/harness/harness.toml")); }
        let exe = std::env::var_os("HARNESS_ORIG_EXE").map(PathBuf::from).or_else(|| std::env::current_exe().ok());
        if let Some(exe) = exe {
            if let Some(dir) = exe.parent() { candidates.push(dir.join("harness.toml")); }
            // cargo run: target/{debug,release}/harness -> project root
            if let Some(root) = exe.ancestors().nth(3) { candidates.push(root.join("harness.toml")); }
        }
        let path = candidates.iter().find(|p| p.is_file())
            .with_context(|| format!("no harness.toml found (looked in {:?})", candidates))?;
        let text = std::fs::read_to_string(path)?;
        let mut cfg: Config = toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        cfg.apply_env();
        Ok(cfg)
    }

    fn apply_env(&mut self) {
        if let Ok(v) = std::env::var("HARNESS_BASE_URL") { self.llm.base_url = v; }
        if let Ok(v) = std::env::var("HARNESS_MODEL") { self.llm.model = v; }
        if let Ok(v) = std::env::var("HARNESS_API_KEY") { self.llm.api_key = Some(v); }
        if let Ok(v) = std::env::var("HARNESS_MAX_TURNS") { if let Ok(n) = v.parse() { self.agent.max_turns = n; } }
        if let Ok(v) = std::env::var("HARNESS_NET") { self.net.enabled = matches!(v.as_str(), "1" | "true" | "yes"); }
    }
}
