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
    #[serde(default)]
    pub checkpoints: crate::checkpoints::CheckpointsConfig,
    #[serde(default)]
    pub format: crate::format::FormatConfig,
    #[serde(default)]
    pub telemetry: crate::telemetry::TelemetryConfig,
    /// Smart self-improvement loop (`harness improve`, `/improve`).
    #[serde(default, rename = "self")]
    pub selfimprove: SelfConfig,
    #[serde(default)]
    pub local_model: LocalModelConfig,
    /// Self-update from GitHub Releases on start (`harness update` on demand).
    #[serde(default)]
    pub update: crate::update::UpdateConfig,
}

/// The model the harness downloads and serves itself: Qwen3.8-27B on MLX, under `~/.config/harness`.
/// Written by the first-run flow; edit it to change quant, port or runtime.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LocalModelConfig {
    /// Which build is in use — "Qwen3.8-27B-MLX-4bit" (4, 6 or 8 bit). Empty = none chosen yet, which is
    /// what makes the first run offer the picker.
    #[serde(default)] pub build: String,
    /// Loopback port for the MLX server the harness starts.
    #[serde(default = "d_mlx_port")] pub port: u16,
    /// "auto" (default): mlx_vlm.server — same weights plus the vision tower, so the model sees images —
    /// falling back to mlx_lm.server if it refuses the build; "mlx-vlm" / "mlx-lm" pin one (mlx-lm is text-only).
    #[serde(default = "d_mlx_server")] pub server: String,
    /// Start the MLX server when the TUI starts and the weights are complete.
    #[serde(default = "d_true")] pub autostart: bool,
    /// Parallel range segments per file while downloading the weights.
    #[serde(default = "d_segments")] pub download_segments: usize,
    /// Offer the download/backend dialog when there is no local model yet.
    #[serde(default = "d_true")] pub first_run_prompt: bool,
    /// Speculative decoding (mlx_vlm.server only): a drafter model — HF id or local path — proposes tokens
    /// the 27B verifies in a batch, speeding up generation on high-acceptance text (code). Empty = off.
    /// Must match the target family (e.g. a Qwen3.5 DFlash drafter for the qwen3_5 build). `/localmodel draft`.
    #[serde(default)] pub draft_model: String,
    /// Drafter family: "auto" (from the drafter's model_type), or "dflash" | "eagle3" | "mtp".
    #[serde(default = "d_auto")] pub draft_kind: String,
    /// Override the drafter's block size (0 = the drafter's own default).
    #[serde(default)] pub draft_block_size: u32,
}
fn d_auto() -> String { "auto".into() }
fn d_mlx_port() -> u16 { 8890 }
fn d_mlx_server() -> String { "auto".into() }
fn d_segments() -> usize { 8 }
impl Default for LocalModelConfig {
    fn default() -> Self { Self { build: String::new(), port: d_mlx_port(), server: d_mlx_server(), autostart: true, download_segments: d_segments(), first_run_prompt: true, draft_model: String::new(), draft_kind: d_auto(), draft_block_size: 0 } }
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
    /// "none" (default), "seatbelt" (macOS sandbox-exec) / "bwrap" (Linux bubblewrap): shell commands
    /// may only write inside the workdir, $TMPDIR and ~/.config/harness; or "docker" / "podman", which
    /// runs every shell command inside `image` with the working directory mounted.
    #[serde(default)] pub mode: String,
    /// Container image for mode = docker|podman (default debian:stable-slim).
    #[serde(default)] pub image: String,
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
    /// How tool calls appear in the transcript: "summary" (one line per burst, click to expand), "hidden", "full"
    #[serde(default = "d_tool_view")] pub tool_view: String,
    /// Show the model's thinking inline by default
    #[serde(default)] pub show_thinking: bool,
    /// Dashboard panel: "auto" (by width) | "on" | "off"
    #[serde(default = "d_panel")] pub panel: String,
    /// Vim-style editing in the prompt
    #[serde(default)] pub vim: bool,
    /// Auto-fold the previous turn's outputs when a new turn starts
    #[serde(default = "d_true")] pub fold_previous: bool,
    /// Typing while a task runs steers it (delivered at the next tool boundary) instead of queueing.
    #[serde(default = "d_true")] pub steer: bool,
    /// Ring the terminal bell when a long task finishes (an OSC 9 notification is always sent).
    #[serde(default)] pub sound: bool,
    /// Shell command whose first line of stdout is shown on the right of the mode line. It receives a
    /// JSON snapshot (model, workdir, session, tokens, cost, permission mode) on stdin.
    #[serde(default)] pub statusline: String,
    /// Terminal font size (pt) applied at start when the terminal can be driven (kitty, iTerm2, Terminal.app); 0 = leave alone. ctrl+= / ctrl+- / ctrl+0.
    #[serde(default)] pub font_size: u32,
    /// Re-open the interactive TUI in kitty when started from another terminal (inline images, font
    /// control, the graphics protocol the UI is built for). HARNESS_NO_KITTY=1 skips it for one run.
    #[serde(default = "d_true")] pub prefer_kitty: bool,
}
fn d_theme() -> String { "dark".into() }
fn d_tool_view() -> String { "summary".into() }
fn d_panel() -> String { "auto".into() }
impl Default for UiConfig { fn default() -> Self { Self { notify: true, theme: d_theme(), event_log: true, tool_view: d_tool_view(), show_thinking: false, panel: d_panel(), vim: false, fold_previous: true, steer: true, sound: false, statusline: String::new(), font_size: 0, prefer_kitty: true } } }

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SecurityConfig {
    /// Redact well-known secret formats (API keys, tokens, private keys) in tool outputs.
    #[serde(default = "d_true")]
    pub redact_secrets: bool,
    /// Warn the model when fetched/MCP content tries to give it instructions (prompt injection).
    #[serde(default = "d_true")]
    pub injection_scan: bool,
}
impl Default for SecurityConfig { fn default() -> Self { Self { redact_secrets: true, injection_scan: true } } }

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
    /// Tool calling for models/servers without a function-calling API: "auto" (default — native first,
    /// switch to the text protocol when the server rejects tools), "on" (always shim), "off".
    #[serde(default)]
    pub tool_shim: Option<String>,
    /// Per-role models: aux · compaction · title · classifier · goal · vision · subagent.
    /// `role = "model-name"`, or a table `{ model, base_url, api_key, temperature }` for another server.
    #[serde(default)]
    pub roles: std::collections::HashMap<String, RoleConfig>,
    /// Models to fall back to, in order, when the configured one is unreachable or 5xx.
    #[serde(default)]
    pub fallback: Vec<String>,
    /// Anthropic prompt caching: mark the system prompt + tool catalogue as cacheable.
    #[serde(default = "d_true")]
    pub prompt_cache: bool,
    /// Extra prices for `/cost`: `"my-model*" = { input = 1.0, output = 3.0 }` (USD per 1M tokens).
    #[serde(default)]
    pub pricing: std::collections::HashMap<String, crate::pricing::Price>,
}

/// One entry of `[llm.roles]`: just a model name, or a full endpoint.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum RoleConfig {
    Model(String),
    Full { #[serde(default)] model: Option<String>, #[serde(default)] base_url: Option<String>, #[serde(default)] api_key: Option<String>, #[serde(default)] temperature: Option<f32> },
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
    /// How deep sub-agents may nest (1 = only the main agent delegates; default 2).
    #[serde(default = "d_depth")]
    pub max_subagent_depth: usize,
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
    /// Search backend for web_search: "auto" (default — whichever key is present), brave, tavily,
    /// exa, searxng, duckduckgo.
    #[serde(default)]
    pub search_provider: Option<String>,
    /// API key for the chosen search provider (else BRAVE_API_KEY / TAVILY_API_KEY / EXA_API_KEY).
    #[serde(default)]
    pub search_api_key: Option<String>,
    /// Base URL of a SearXNG instance (or $SEARXNG_URL).
    #[serde(default)]
    pub searxng_url: Option<String>,
    /// Network allow-list proxy for tools and shell commands ([net.proxy]).
    #[serde(default)]
    pub proxy: crate::proxy::ProxyConfig,
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
fn d_depth() -> usize { 2 }
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
        Self { enabled: true, timeout_secs: d_fetch_timeout(), max_fetch_bytes: d_fetch_bytes(), user_agent: d_ua(), search_provider: None, search_api_key: None, searxng_url: None, proxy: Default::default(), download_segments: d_dl_segments(), download_timeout_secs: d_dl_timeout() }
    }
}

impl LlmConfig {
    /// The compaction threshold given a detected context length (if any).
    pub fn effective_budget(&self, detected_ctx: Option<u64>) -> u64 {
        if let Some(b) = self.context_budget_tokens { return b; }
        match detected_ctx { Some(n) => ((n as f64) * self.compact_at_fraction) as u64, None => 60_000 }
    }
}

/// Where a setting came from, lowest precedence first. `--setting-sources` selects which are read.
pub const SOURCES: [&str; 5] = ["managed", "user", "project", "local", "cli"];

impl Config {
    /// `Config::load` plus the layered overlays: managed → user → project → local → CLI `--set`.
    /// `sources` is a comma-separated subset of `SOURCES` (None = all of them).
    pub fn load_layered(explicit: Option<&Path>, sources: Option<&str>, sets: &[String]) -> Result<Config> {
        let mut cfg = Config::load(explicit)?;
        let wanted: Vec<String> = match sources {
            Some(s) => s.split(',').map(|x| x.trim().to_lowercase()).filter(|x| !x.is_empty()).collect(),
            None => SOURCES.iter().map(|s| s.to_string()).collect(),
        };
        for w in &wanted { if !SOURCES.contains(&w.as_str()) { anyhow::bail!("unknown setting source '{w}' (use {})", SOURCES.join(",")); } }
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let trusted = crate::permissions::is_trusted(&cwd);
        for (source, path) in Self::overlay_files(&cwd) {
            if !wanted.iter().any(|w| w == source) { continue; }
            let project_scoped = source == "project" || source == "local";
            cfg.apply_overlay_file(&path, !(project_scoped && !trusted));
        }
        if wanted.iter().any(|w| w == "cli") {
            for kv in sets {
                let (k, v) = kv.split_once('=').with_context(|| format!("--set needs key=value (got {kv})"))?;
                cfg.set_setting(k.trim(), v.trim()).with_context(|| format!("--set {kv}"))?;
            }
        }
        Ok(cfg)
    }

    /// The overlay files in precedence order (later wins). Managed settings come from
    /// $HARNESS_MANAGED_CONFIG or /etc/harness/managed.toml and are applied first so a policy file
    /// cannot be silently dropped, then user, project and personal-local settings.
    pub fn overlay_files(cwd: &Path) -> Vec<(&'static str, PathBuf)> {
        let mut v: Vec<(&'static str, PathBuf)> = Vec::new();
        let managed = std::env::var_os("HARNESS_MANAGED_CONFIG").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/etc/harness/managed.toml"));
        v.push(("managed", managed));
        v.push(("user", Self::settings_overlay_path()));
        v.push(("project", cwd.join(".harness").join("settings.toml")));
        v.push(("local", cwd.join(".harness").join("settings.local.toml")));
        v
    }

    /// Apply one overlay file: flat dotted keys (`"ui.theme" = "light"`) or nested tables
    /// (`[ui] theme = "light"`). Untrusted project files may not raise privileges.
    pub fn apply_overlay_file(&mut self, path: &Path, trusted: bool) -> usize {
        let Ok(text) = std::fs::read_to_string(path) else { return 0 };
        let Ok(v) = text.parse::<toml::Value>() else { eprintln!("config: {} is not valid TOML — ignored", path.display()); return 0 };
        let mut pairs = Vec::new();
        flatten("", &v, &mut pairs);
        let mut n = 0;
        for (k, val) in pairs {
            if !trusted && k == "permissions.mode" && val == "bypass" {
                eprintln!("config: {} sets permissions.mode = bypass — ignored because this directory is not trusted (/trust to enable)", path.display());
                continue;
            }
            match self.set_setting(&k, &val) { Ok(()) => n += 1, Err(e) => eprintln!("config: {} — {e:#}", path.display()) }
        }
        n
    }

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
        cfg.apply_settings_overlay();
        cfg.apply_env();
        // hooks defined for Claude Code in this project are honoured too (trusted directories only)
        if crate::permissions::is_trusted(&cwd) {
            for f in [cwd.join(".claude").join("settings.json"), cwd.join(".claude").join("settings.local.json")] {
                let n = crate::hooks::import_claude_hooks(&mut cfg.hooks, &f);
                if n > 0 { eprintln!("config: imported {n} hook(s) from {}", f.display()); }
            }
        }
        Ok(cfg)
    }

    /// ~/.config/harness/settings.toml — values changed from the /settings panel; a flat "section.key = value" overlay.
    pub fn settings_overlay_path() -> PathBuf { crate::setup::config_dir().join("settings.toml") }
    pub fn apply_settings_overlay(&mut self) {
        let Ok(t) = std::fs::read_to_string(Self::settings_overlay_path()) else { return };
        let Ok(v) = t.parse::<toml::Value>() else { return };
        let Some(tbl) = v.as_table() else { return };
        for (k, val) in tbl { let sv = match val { toml::Value::String(x) => x.clone(), other => other.to_string() }; let _ = self.set_setting(k, &sv); }
    }
    /// Apply one setting by dotted key (used by the /settings panel and the overlay). Err for unknown keys/values.
    pub fn set_setting(&mut self, key: &str, val: &str) -> anyhow::Result<()> {
        let b = |v: &str| matches!(v, "true" | "on" | "yes" | "1");
        match key {
            "ui.theme" => self.ui.theme = val.into(),
            "ui.tool_view" => self.ui.tool_view = val.into(),
            "ui.notify" => self.ui.notify = b(val),
            "ui.event_log" => self.ui.event_log = b(val),
            "ui.show_thinking" => self.ui.show_thinking = b(val),
            "ui.panel" => self.ui.panel = val.into(),
            "ui.vim" => self.ui.vim = b(val),
            "ui.fold_previous" => self.ui.fold_previous = b(val),
            "ui.steer" => self.ui.steer = b(val),
            "ui.statusline" => self.ui.statusline = val.into(),
            "ui.sound" => self.ui.sound = b(val),
            "ui.font_size" => self.ui.font_size = val.parse().context("bad size")?,
            "ui.prefer_kitty" => self.ui.prefer_kitty = b(val),
            "local_model.build" => self.local_model.build = val.into(),
            "local_model.port" => self.local_model.port = val.parse().context("bad port")?,
            "local_model.server" => self.local_model.server = val.into(),
            "local_model.autostart" => self.local_model.autostart = b(val),
            "local_model.first_run_prompt" => self.local_model.first_run_prompt = b(val),
            "permissions.mode" => self.permissions.mode = crate::permissions::Mode::parse(val).context("bad mode")?,
            "llm.compact_at_fraction" => self.llm.compact_at_fraction = val.parse().context("bad fraction")?,
            "llm.effort" => self.llm.effort = if val.is_empty() || val == "default" { None } else { Some(val.into()) },
            "llm.provider" => self.llm.provider = if val.is_empty() || val == "local" { None } else { Some(val.into()) },
            "llm.model" => self.llm.model = val.into(),
            "llm.tool_shim" => self.llm.tool_shim = if val.is_empty() || val == "auto" { None } else { Some(val.into()) },
            "memory.auto_reflect" => self.memory.auto_reflect = b(val),
            "memory.enabled" => self.memory.enabled = b(val),
            "security.redact_secrets" => self.security.redact_secrets = b(val),
            "security.injection_scan" => self.security.injection_scan = b(val),
            "agent.max_task_secs" => self.agent.max_task_secs = val.parse().context("bad number")?,
            "agent.max_turns" => self.agent.max_turns = val.parse().context("bad number")?,
            "net.enabled" => self.net.enabled = b(val),
            "net.proxy.enabled" => self.net.proxy.enabled = b(val),
            "net.search_provider" => self.net.search_provider = (!val.is_empty()).then(|| val.to_string()),
            "net.searxng_url" => self.net.searxng_url = (!val.is_empty()).then(|| val.to_string()),
            "net.proxy.verbose" => self.net.proxy.verbose = b(val),
            "llm.base_url" => self.llm.base_url = val.into(),
            "llm.api_key" => self.llm.api_key = (!val.is_empty()).then(|| val.to_string()),
            "llm.aux_model" => self.llm.aux_model = (!val.is_empty()).then(|| val.to_string()),
            "llm.temperature" => self.llm.temperature = val.parse().context("bad temperature")?,
            "llm.max_tokens" => self.llm.max_tokens = val.parse().context("bad number")?,
            "llm.thinking_budget" => self.llm.thinking_budget = val.parse().ok(),
            "llm.prompt_cache" => self.llm.prompt_cache = b(val),
            "agent.tool_timeout_secs" => self.agent.tool_timeout_secs = val.parse().context("bad number")?,
            "agent.max_tool_output_chars" => self.agent.max_tool_output_chars = val.parse().context("bad number")?,
            "agent.max_subagent_depth" => self.agent.max_subagent_depth = val.parse().context("bad number")?,
            "eval.tasks_dir" => self.eval.tasks_dir = val.into(),
            "eval.runs_dir" => self.eval.runs_dir = val.into(),
            "eval.task_timeout_secs" => self.eval.task_timeout_secs = val.parse().context("bad number")?,
            "sandbox.deny_network" => self.sandbox.deny_network = b(val),
            "checkpoints.max_file_mb" => self.checkpoints.max_file_mb = val.parse().context("bad number")?,
            "memory.max_inject_chars" => self.memory.max_inject_chars = val.parse().context("bad number")?,
            "self.skip_arbiter" => self.selfimprove.skip_arbiter = b(val),
            "self.auto" => self.selfimprove.auto = val.into(),
            "sandbox.mode" => self.sandbox.mode = if val == "none" { String::new() } else { val.into() },
            "sandbox.image" => self.sandbox.image = val.into(),
            "checkpoints.enabled" => self.checkpoints.enabled = b(val),
            "format.enabled" => self.format.enabled = b(val),
            "format.diagnostics_after_edit" => self.format.diagnostics_after_edit = b(val),
            _ => anyhow::bail!("unknown setting {key}"),
        }
        Ok(())
    }
    /// Persist one setting to the overlay file.
    pub fn save_setting(key: &str, val: &str) -> anyhow::Result<()> {
        let p = Self::settings_overlay_path();
        let mut tbl = std::fs::read_to_string(&p).ok().and_then(|t| t.parse::<toml::Value>().ok()).and_then(|v| v.as_table().cloned()).unwrap_or_default();
        tbl.insert(key.to_string(), toml::Value::String(val.to_string()));
        std::fs::create_dir_all(p.parent().unwrap())?;
        let mut out = String::from("# Settings changed from /settings (override harness.toml). Delete a line to fall back to harness.toml.\n");
        for (k, v) in &tbl { out.push_str(&format!("\"{k}\" = {}\n", v)); }
        std::fs::write(&p, out)?;
        Ok(())
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

/// Nested TOML tables → flat dotted keys, so `[ui] theme = "x"` and `"ui.theme" = "x"` are the same.
fn flatten(prefix: &str, v: &toml::Value, out: &mut Vec<(String, String)>) {
    match v {
        toml::Value::Table(t) => {
            for (k, val) in t {
                let key = if prefix.is_empty() { k.clone() } else { format!("{prefix}.{k}") };
                flatten(&key, val, out);
            }
        }
        toml::Value::String(s) => out.push((prefix.to_string(), s.clone())),
        other => out.push((prefix.to_string(), other.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn layered_overlays() {
        let base = "[llm]\nbase_url='http://x'\nmodel='m'\n[agent]\n";
        let mut cfg: Config = toml::from_str(base).unwrap();
        let d = std::env::temp_dir().join(format!("harness-cfg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        // nested tables and flat dotted keys are equivalent
        let nested = d.join("nested.toml");
        std::fs::write(&nested, "[ui]\ntheme = \"light\"\ntool_view = \"full\"\n").unwrap();
        assert_eq!(cfg.apply_overlay_file(&nested, true), 2);
        assert_eq!(cfg.ui.theme, "light");
        assert_eq!(cfg.ui.tool_view, "full");
        let flat = d.join("flat.toml");
        std::fs::write(&flat, "\"ui.theme\" = \"dark\"\n\"agent.max_turns\" = 7\n").unwrap();
        assert_eq!(cfg.apply_overlay_file(&flat, true), 2);
        assert_eq!(cfg.ui.theme, "dark");
        assert_eq!(cfg.agent.max_turns, 7);
        // an untrusted project overlay may not turn permissions off
        let danger = d.join("danger.toml");
        std::fs::write(&danger, "\"permissions.mode\" = \"bypass\"\n\"ui.theme\" = \"light\"\n").unwrap();
        assert_eq!(cfg.apply_overlay_file(&danger, false), 1);
        assert_eq!(cfg.permissions.mode, crate::permissions::Mode::Auto);
        assert_eq!(cfg.ui.theme, "light");
        assert_eq!(cfg.apply_overlay_file(&danger, true), 2);
        assert_eq!(cfg.permissions.mode, crate::permissions::Mode::Bypass);
        // unknown files are simply absent
        assert_eq!(cfg.apply_overlay_file(&d.join("nope.toml"), true), 0);
        let names: Vec<&str> = Config::overlay_files(std::path::Path::new("/proj")).iter().map(|(s, _)| *s).collect();
        assert_eq!(names, vec!["managed", "user", "project", "local"]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn untrusted_sanitize() {
        let mut cfg: Config = toml::from_str("[llm]\nbase_url='http://x'\nmodel='m'\n[agent]\n[permissions]\nmode='bypass'\n[hooks]\npre_tool=['echo hi']\n").unwrap();
        assert_eq!(cfg.sanitize_untrusted(), vec!["hooks", "permissions.mode = bypass"]);
        assert!(cfg.hooks.pre_tool.is_empty()); assert_eq!(cfg.permissions.mode, crate::permissions::Mode::Auto);
        assert!(cfg.sanitize_untrusted().is_empty());
    }
}
