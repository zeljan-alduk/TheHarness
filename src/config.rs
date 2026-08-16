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
    #[serde(default)]
    pub hooks: crate::hooks::HooksConfig,
    #[serde(default)]
    pub security: SecurityConfig,
    #[serde(default)]
    pub ui: UiConfig,
    #[serde(default)]
    pub sandbox: SandboxConfig,
    #[serde(default)]
    pub lsp: LspConfig,
    /// Smart self-improvement loop (`harness improve`, `/improve`).
    #[serde(default, rename = "self")]
    pub selfimprove: SelfConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SelfConfig {
    /// Who approves the plan: "smart" (auto when the backend is a frontier model, otherwise ask the user),
    /// "always" (never ask), "never" (always ask).
    #[serde(default = "d_self_auto")] pub auto: String,
    /// Model-name globs counted as "smart" (provider claude-code/anthropic always counts).
    #[serde(default = "d_smart_models")] pub smart_models: Vec<String>,
    /// Eval runs per side for the arbiter verdict (baseline for main is cached).
    #[serde(default = "d_one")] pub arbiter_runs: usize,
    /// Seconds the user gets to cancel the automatic restart after an improvement was installed.
    #[serde(default = "d_grace")] pub restart_grace_secs: u64,
    /// Max proposals per round.
    #[serde(default = "d_three")] pub max_items: usize,
    /// Harness source checkout to improve (default: located from cwd / the binary / HARNESS_REPO).
    #[serde(default)] pub repo: Option<String>,
    /// Skip the eval-based arbiter (only build + tests gate the merge). Faster, less safe.
    #[serde(default)] pub skip_arbiter: bool,
}
fn d_self_auto() -> String { "smart".into() }
fn d_smart_models() -> Vec<String> { ["claude*", "*opus*", "*sonnet*", "*fable*", "gpt-5*", "o3*", "o4*"].iter().map(|s| s.to_string()).collect() }
fn d_one() -> usize { 1 }
fn d_three() -> usize { 3 }
fn d_grace() -> u64 { 60 }
impl Default for SelfConfig { fn default() -> Self { Self { auto: d_self_auto(), smart_models: d_smart_models(), arbiter_runs: 1, restart_grace_secs: 60, max_items: 3, repo: None, skip_arbiter: false } } }

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct LspConfig {
    /// name → {command, args, exts}. Empty = built-in defaults (rust-analyzer, pyright, typescript-language-server, gopls).
    #[serde(default)] pub servers: std::collections::HashMap<String, crate::lsp::LspServerConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct SandboxConfig {
    /// "none" (default) or "seatbelt" (macOS sandbox-exec: shell commands may only write inside the
    /// workdir, $TMPDIR and ~/.config/harness; set deny_network to also block outbound network).
    #[serde(default)] pub mode: String,
    #[serde(default)] pub deny_network: bool,
    /// Extra writable paths for seatbelt mode.
    #[serde(default)] pub allow_write: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UiConfig {
    /// macOS notification when a task longer than 20s finishes.
    #[serde(default = "d_true")] pub notify: bool,
    /// "dark" | "light"
    #[serde(default = "d_theme")] pub theme: String,
    /// Append every event to ~/.config/harness/logs/<date>/tui-<pid>.jsonl
    #[serde(default = "d_true")] pub event_log: bool,
}
fn d_theme() -> String { "dark".into() }
impl Default for UiConfig { fn default() -> Self { Self { notify: true, theme: d_theme(), event_log: true } } }

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SecurityConfig {
    /// Redact well-known secret formats (API keys, tokens, private keys) in tool outputs.
    #[serde(default = "d_true")]
    pub redact_secrets: bool,
}
impl Default for SecurityConfig { fn default() -> Self { Self { redact_secrets: true } } }

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LlmConfig {
    pub base_url: String,
    pub model: String,
    /// "openai" (any OpenAI-compatible server; default) or "anthropic" (Messages API). Auto-detected from base_url.
    #[serde(default)]
    pub provider: Option<String>,
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
    /// Claude Code backend: reasoning effort passed as `claude --effort` (low | medium | high | max).
    #[serde(default)]
    pub effort: Option<String>,
    /// Anthropic API: enable extended thinking with this token budget (summarized thinking is streamed back).
    #[serde(default)]
    pub thinking_budget: Option<u32>,
    /// Optional smaller/faster model for auxiliary calls (memory reflection, compaction, consolidation).
    #[serde(default)]
    pub aux_model: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentConfig {
    #[serde(default = "d_max_turns")]
    pub max_turns: usize,
    #[serde(default = "d_tool_timeout")]
    pub tool_timeout_secs: u64,
    #[serde(default = "d_max_out")]
    pub max_tool_output_chars: usize,
    /// Optional wall-clock cap per task in the TUI (seconds); 0 = unlimited. The queue continues afterwards.
    #[serde(default)]
    pub max_task_secs: u64,
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
        let local_idx = candidates.len(); // ./harness.toml (project-local; trust-gated below)
        candidates.push(PathBuf::from("harness.toml"));
        candidates.push(crate::setup::config_dir().join("harness.toml"));
        let exe = std::env::var_os("HARNESS_ORIG_EXE").map(PathBuf::from).or_else(|| std::env::current_exe().ok());
        if let Some(exe) = exe {
            if let Some(dir) = exe.parent() { candidates.push(dir.join("harness.toml")); }
            // cargo run: target/{debug,release}/harness -> project root
            if let Some(root) = exe.ancestors().nth(3) { candidates.push(root.join("harness.toml")); }
        }
        let idx = candidates.iter().position(|p| p.is_file())
            .with_context(|| format!("no harness.toml found (looked in {:?})", candidates))?;
        let path = &candidates[idx];
        let text = std::fs::read_to_string(path)?;
        let mut cfg: Config = toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        // A project-local ./harness.toml in an untrusted directory may not run hooks or bypass permissions.
        let same = |a: &Path, b: &Path| a.canonicalize().ok().zip(b.canonicalize().ok()).map(|(x, y)| x == y).unwrap_or(false);
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let project_local = idx == local_idx && !candidates[local_idx + 1..].iter().any(|c| same(c, path));
        if project_local && !crate::permissions::is_trusted(&cwd) {
            let dropped = cfg.sanitize_untrusted();
            if !dropped.is_empty() { eprintln!("config: {} defines {} — ignored because this directory is not trusted (/trust to enable)", path.display(), dropped.join(" and ")); }
        }
        cfg.apply_env();
        Ok(cfg)
    }

    /// Drop hooks and downgrade bypass → auto (project-local configs in untrusted dirs). Returns what was dropped.
    pub fn sanitize_untrusted(&mut self) -> Vec<&'static str> {
        let mut dropped = Vec::new();
        let h = &self.hooks;
        if [&h.pre_tool, &h.post_tool, &h.on_stop, &h.on_prompt, &h.session_start, &h.session_end, &h.subagent_stop, &h.pre_compact, &h.notification].iter().any(|v| !v.is_empty()) { self.hooks = crate::hooks::HooksConfig::default(); dropped.push("hooks"); }
        if self.permissions.mode == crate::permissions::Mode::Bypass { self.permissions.mode = crate::permissions::Mode::Auto; dropped.push("permissions.mode = bypass"); }
        dropped
    }

    fn apply_env(&mut self) {
        if let Ok(v) = std::env::var("HARNESS_BASE_URL") { self.llm.base_url = v; }
        if let Ok(v) = std::env::var("HARNESS_MODEL") { self.llm.model = v; }
        if let Ok(v) = std::env::var("HARNESS_API_KEY") { self.llm.api_key = Some(v); }
        if let Ok(v) = std::env::var("HARNESS_MAX_TURNS") { if let Ok(n) = v.parse() { self.agent.max_turns = n; } }
        if let Ok(v) = std::env::var("HARNESS_NET") { self.net.enabled = matches!(v.as_str(), "1" | "true" | "yes"); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn untrusted_sanitize() {
        let mut cfg: Config = toml::from_str("[llm]\nbase_url='http://x'\nmodel='m'\n[agent]\n[permissions]\nmode='bypass'\n[hooks]\npre_tool=['echo hi']\n").unwrap();
        assert_eq!(cfg.sanitize_untrusted(), vec!["hooks", "permissions.mode = bypass"]);
        assert!(cfg.hooks.pre_tool.is_empty()); assert_eq!(cfg.permissions.mode, crate::permissions::Mode::Auto);
        assert!(cfg.sanitize_untrusted().is_empty());
    }
}
