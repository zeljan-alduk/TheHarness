//! The local model, from nothing to serving: pick a Qwen3.8-27B MLX build, fetch it from Hugging Face
//! in resumable segments, and serve it with the MLX runtime the installer put in
//! `~/.config/harness/runtime/mlx`.
//!
//! Everything lives under the harness's own directory — `models/<build>` for weights, `runtime/mlx` for
//! the server — so the harness owns its model the way it owns its config, and `rm -rf ~/.config/harness`
//! is a complete uninstall. Nothing here touches another tool's library (LM Studio, Ollama, the HF cache):
//! a model the harness serves is one the harness downloaded.
//!
//! MLX is Apple-Silicon-only, which is why this is a macOS/arm64 project.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// The Qwen3.8-27B MLX builds the first-run picker offers. Sizes are the repos' actual totals (the HF
/// API is asked again before downloading, so a stale number here only affects the menu text).
pub const BUILDS: &[Build] = &[
    Build { bits: 4, repo: "lmstudio-community/Qwen3.8-27B-MLX-4bit", bytes: 16_080_000_000, note: "fastest, smallest — the default" },
    Build { bits: 6, repo: "lmstudio-community/Qwen3.8-27B-MLX-6bit", bytes: 22_800_000_000, note: "closer to full precision" },
    Build { bits: 8, repo: "lmstudio-community/Qwen3.8-27B-MLX-8bit", bytes: 29_530_000_000, note: "best quality, needs the most RAM" },
];

#[derive(Debug, Clone, Copy)]
pub struct Build {
    pub bits: u8,
    pub repo: &'static str,
    pub bytes: u64,
    pub note: &'static str,
}

impl Build {
    /// "Qwen3.8-27B-MLX-4bit"
    pub fn name(&self) -> &'static str { self.repo.rsplit('/').next().unwrap_or(self.repo) }
    pub fn dir(&self) -> PathBuf { models_dir().join(self.name()) }
    /// Roughly how much RAM the weights want resident; the picker warns when the machine has less.
    pub fn ram_gb(&self) -> u64 { self.bytes / 1_000_000_000 + 4 }
}

pub fn by_bits(bits: u8) -> Option<&'static Build> { BUILDS.iter().find(|b| b.bits == bits) }
pub fn by_name(name: &str) -> Option<&'static Build> { BUILDS.iter().find(|b| b.name() == name || b.repo == name) }

pub fn models_dir() -> PathBuf { crate::setup::config_dir().join("models") }
pub fn runtime_dir() -> PathBuf { crate::setup::config_dir().join("runtime") }

/// The python of the private MLX venv (`~/.config/harness/runtime/mlx`), if the installer ran.
pub fn mlx_python() -> Option<PathBuf> {
    let p = runtime_dir().join("mlx/bin/python");
    p.is_file().then_some(p)
}

/// Weights are usable when every file the repo lists is present at its full size and no `.harness-dl.json`
/// checkpoint is left over. A partially downloaded model is *not* usable — but it is resumable.
pub fn state_of(build: &Build) -> ModelState {
    let dir = build.dir();
    if !dir.is_dir() { return ModelState::Missing; }
    let partial = std::fs::read_dir(&dir).map(|rd| rd.flatten().any(|e| e.file_name().to_string_lossy().ends_with(".harness-dl.json"))).unwrap_or(false);
    let weights: u64 = std::fs::read_dir(&dir).map(|rd| rd.flatten().filter_map(|e| e.metadata().ok()).map(|m| m.len()).sum()).unwrap_or(0);
    let index = dir.join("config.json").is_file();
    if partial || !index { return ModelState::Partial { bytes: weights }; }
    // Allow slack: the menu's byte total is approximate, the repo may add small files.
    if weights + weights / 20 < build.bytes { return ModelState::Partial { bytes: weights }; }
    ModelState::Ready { bytes: weights }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelState { Missing, Partial { bytes: u64 }, Ready { bytes: u64 } }

/// A file in a Hugging Face repo, with the size the API reports.
#[derive(Debug, Clone)]
pub struct RepoFile { pub name: String, pub bytes: u64 }

/// List a repo's files, largest first, skipping what a runtime never reads.
pub async fn list_repo(http: &reqwest::Client, repo: &str) -> Result<Vec<RepoFile>> {
    let url = format!("https://huggingface.co/api/models/{repo}?blobs=true");
    let v: serde_json::Value = http.get(&url).send().await.with_context(|| format!("GET {url}"))?
        .error_for_status().with_context(|| format!("Hugging Face rejected {repo}"))?
        .json().await.context("Hugging Face returned something other than JSON")?;
    let mut files: Vec<RepoFile> = v.get("siblings").and_then(|s| s.as_array()).map(|a| a.iter().filter_map(|f| {
        let name = f.get("rfilename")?.as_str()?.to_string();
        if name == ".gitattributes" || name.ends_with(".md") { return None; }
        Some(RepoFile { name, bytes: f.get("size").and_then(|s| s.as_u64()).unwrap_or(0) })
    }).collect()).unwrap_or_default();
    if files.is_empty() { bail!("{repo} lists no files"); }
    files.sort_by_key(|f| std::cmp::Reverse(f.bytes));
    Ok(files)
}

/// Where a download has got to. `done`/`total` count every file in the repo, so the UI can show one bar.
#[derive(Debug, Clone, Copy, Default)]
pub struct Progress {
    pub done: u64,
    pub total: u64,
    pub bytes_per_sec: f64,
    pub eta_secs: u64,
    pub files_done: usize,
    pub files_total: usize,
}

impl Progress {
    pub fn percent(&self) -> f64 { if self.total == 0 { 0.0 } else { (self.done as f64 / self.total as f64 * 100.0).min(100.0) } }
    /// "4.21 GB / 16.08 GB · 78.4 MB/s · 2m 31s left"
    pub fn line(&self) -> String {
        use crate::tools::download::human;
        let eta = if self.bytes_per_sec < 1.0 || self.done >= self.total { String::new() } else {
            let (m, s) = (self.eta_secs / 60, self.eta_secs % 60);
            if m >= 60 { format!(" · {}h {}m left", m / 60, m % 60) } else if m > 0 { format!(" · {m}m {s}s left") } else { format!(" · {s}s left") }
        };
        format!("{} / {} · {}/s{}", human(self.done), human(self.total), human(self.bytes_per_sec as u64), eta)
    }
}

/// Fetch every file of `build` into its directory, resuming whatever is already there, reporting
/// aggregate progress. Safe to call again after an interruption — that is the whole point.
pub async fn fetch(build: &Build, segments: usize, on_progress: Arc<dyn Fn(Progress) + Send + Sync>) -> Result<PathBuf> {
    let dir = build.dir();
    tokio::fs::create_dir_all(&dir).await?;
    let http = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .user_agent(concat!("harness/", env!("HARNESS_VERSION")))
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()?;
    let files = list_repo(&http, build.repo).await?;
    let total: u64 = files.iter().map(|f| f.bytes).sum();
    let files_total = files.len();

    // Bytes already on disk from an earlier attempt count as done, so a resumed download does not
    // restart the bar at zero.
    let carried = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let started = std::time::Instant::now();
    let base_done: u64 = files.iter().filter_map(|f| std::fs::metadata(dir.join(&f.name)).ok().map(|m| {
        // a file with a checkpoint next to it is partial: count what is written, not its final size
        if dir.join(format!("{}.harness-dl.json", f.name)).is_file() { m.len().min(f.bytes) } else { f.bytes.min(m.len()) }
    })).sum();
    let session_start = base_done;

    for (i, f) in files.iter().enumerate() {
        let dest = dir.join(&f.name);
        let done_before = carried.load(std::sync::atomic::Ordering::Relaxed);
        let cb = {
            let on_progress = on_progress.clone();
            Arc::new(move |file_done: u64, _file_total: u64| {
                let done = base_done.max(done_before + file_done);
                let secs = started.elapsed().as_secs_f64().max(0.25);
                let speed = (done.saturating_sub(session_start)) as f64 / secs;
                let eta = if speed > 1.0 { ((total.saturating_sub(done)) as f64 / speed) as u64 } else { 0 };
                on_progress(Progress { done, total, bytes_per_sec: speed, eta_secs: eta, files_done: i, files_total });
            }) as Arc<dyn Fn(u64, u64) + Send + Sync>
        };
        let got = crate::tools::download::fetch_resumable(
            &http,
            &format!("https://huggingface.co/{}/resolve/main/{}", build.repo, f.name),
            &dest,
            segments,
            Duration::from_secs(6 * 3600),
            Some(cb),
        ).await.with_context(|| format!("downloading {}", f.name))?;
        carried.fetch_add(got, std::sync::atomic::Ordering::Relaxed);
    }
    on_progress(Progress { done: total, total, bytes_per_sec: 0.0, eta_secs: 0, files_done: files_total, files_total });
    Ok(dir)
}

/// A running MLX server. Dropping the handle leaves the process alone; call `stop` to end it.
pub struct Server {
    pub base_url: String,
    pub model: String,
    pub port: u16,
    child: tokio::process::Child,
    /// "mlx_vlm.server" (vision) or "mlx_lm.server" (text-only)
    pub module: &'static str,
}

impl Server {
    pub async fn stop(mut self) { let _ = self.child.kill().await; }
    pub fn pid(&self) -> Option<u32> { self.child.id() }
}

/// Does the runtime have mlx-vlm (the server with the vision tower)?
pub fn has_mlx_vlm() -> bool {
    let lib = runtime_dir().join("mlx/lib");
    std::fs::read_dir(&lib).map(|rd| rd.flatten().any(|e| e.path().join("site-packages/mlx_vlm/server").is_dir())).unwrap_or(false)
}

/// Make sure the runtime can serve vision: a harness that updated itself past 1.0.109 may sit on a venv
/// the older installer made with mlx-lm only. Installs mlx-vlm into the private venv with uv (what the
/// installer uses) — a no-op when it is already there. Returns what happened, for the transcript.
pub async fn ensure_mlx_vlm() -> Result<Option<String>> {
    if has_mlx_vlm() { return Ok(None); }
    let py = mlx_python().context("no MLX runtime — re-run the installer")?;
    let uv = crate::setup::which("uv").or_else(|| { let p = crate::setup::home_dir().join(".local/bin/uv"); p.is_file().then_some(p) })
        .context("mlx-vlm is missing from the MLX runtime and uv is not installed — re-run the installer (curl -fsSL https://zeljan-alduk.github.io/TheHarness/install.sh | sh)")?;
    let out = tokio::process::Command::new(&uv).args(["pip", "install", "--python"]).arg(&py).args(["--quiet", "--upgrade", "mlx-lm", "mlx-vlm>=0.6.13"]).output().await.context("running uv pip install")?;
    if !out.status.success() { bail!("installing mlx-vlm failed: {}", String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or_default()); }
    if !has_mlx_vlm() { bail!("uv reported success but mlx_vlm is still not importable in {}", py.display()); }
    Ok(Some("installed mlx-vlm into the MLX runtime (the server with the vision tower, so images and video frames work)".into()))
}

/// Which server module a `[local_model] server` setting means, and whether a fallback is allowed.
/// "auto" (default): mlx_vlm.server when the runtime has it, mlx_lm.server otherwise — and mlx_lm as the
/// fallback if mlx_vlm refuses the build. "mlx-vlm" / "mlx-lm" pin one.
pub fn server_plan(kind: &str) -> Vec<&'static str> {
    match kind {
        "mlx-vlm" | "mlx_vlm" | "vision" => vec!["mlx_vlm.server"],
        "mlx-lm" | "mlx_lm" | "text" => vec!["mlx_lm.server"],
        _ => if has_mlx_vlm() { vec!["mlx_vlm.server", "mlx_lm.server"] } else { vec!["mlx_lm.server"] },
    }
}

/// Serve a downloaded model over an OpenAI-compatible API on 127.0.0.1.
///
/// Both servers come from the same MLX runtime and load the same weights. `mlx_lm.server` is text-only —
/// it answers image parts with 404 "Only 'text' content type is supported" — while `mlx_vlm.server`
/// also loads the vision tower (Qwen3.8 is image-text-to-text), speaks the same chat/tools API and
/// streams reasoning, so it is what "auto" starts first; mlx_lm stays as the fallback in case a build
/// or an mlx-vlm release refuses to load.
pub async fn serve(model_dir: &Path, port: u16, kind: &str) -> Result<Server> {
    let mut last: Option<anyhow::Error> = None;
    for module in server_plan(kind) {
        match serve_with(model_dir, port, module).await {
            Ok(s) => return Ok(s),
            Err(e) => { last = Some(e); }
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("no MLX server module to start")))
}

async fn serve_with(model_dir: &Path, port: u16, module: &'static str) -> Result<Server> {
    let py = mlx_python().context("no MLX runtime — re-run the installer, or: uv venv ~/.config/harness/runtime/mlx && uv pip install --python ~/.config/harness/runtime/mlx/bin/python mlx-lm mlx-vlm")?;
    let log_dir = crate::setup::config_dir().join("logs");
    std::fs::create_dir_all(&log_dir).ok();
    let log = std::fs::OpenOptions::new().create(true).append(true).open(log_dir.join("mlx-server.log"))?;
    let mut cmd = tokio::process::Command::new(&py);
    cmd.args(["-m", module, "--host", "127.0.0.1", "--port", &port.to_string(), "--model"]).arg(model_dir);
    // mlx_vlm's prefix cache is off by default, and without it every agent turn re-prefills the whole
    // conversation (~85s for 10k tokens on a 27B 4-bit): APC_ENABLED=1 makes a repeated or extended
    // prompt cost only its new tail (measured 16s → 0.6s). 2048 blocks × 16 = 32k tokens of KV cache.
    if module == "mlx_vlm.server" { cmd.env("APC_ENABLED", "1"); }
    let child = cmd
        .stdout(log.try_clone()?)
        .stderr(log)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(false)
        .spawn()
        .with_context(|| format!("starting {module}"))?;

    // Loading 16–30GB of weights takes a while; poll until it answers or dies.
    let base_url = format!("http://127.0.0.1:{port}/v1");
    let http = reqwest::Client::builder().timeout(Duration::from_secs(3)).build()?;
    let mut server = Server { base_url: base_url.clone(), model: model_dir.display().to_string(), port, child, module };
    for _ in 0..600 {
        if let Ok(Some(status)) = server.child.try_wait() {
            bail!("{module} exited ({status}) — see {}", log_dir.join("mlx-server.log").display());
        }
        if let Ok(r) = http.get(format!("{base_url}/models")).send().await {
            if r.status().is_success() {
                if let Ok(v) = r.json::<serde_json::Value>().await {
                    // mlx_vlm lists whatever is in the HF cache too, and wants requests to name the model
                    // exactly as it lists it — so take the entry that is our directory, never just the first.
                    if let Some(id) = pick_model_id(&v, model_dir) { server.model = id; }
                }
                return Ok(server);
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let _ = server.child.kill().await;
    bail!("{module} did not answer on {base_url} within 5 minutes — see {}", log_dir.join("mlx-server.log").display())
}

/// The id under which a `/v1/models` listing offers `model_dir` (exact path, or the directory name), else the first id.
pub fn pick_model_id(models: &serde_json::Value, model_dir: &Path) -> Option<String> {
    let ids: Vec<&str> = models.get("data").and_then(|d| d.as_array()).map(|a| a.iter().filter_map(|m| m.get("id").and_then(|i| i.as_str())).collect()).unwrap_or_default();
    let want = model_dir.display().to_string();
    let name = model_dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    ids.iter().find(|i| i.trim_end_matches('/') == want.trim_end_matches('/'))
        .or_else(|| ids.iter().find(|i| !name.is_empty() && (i.trim_end_matches('/').ends_with(&format!("/{name}")) || **i == name)))
        .or_else(|| ids.first())
        .map(|s| s.to_string())
}

/// Which MLX server answers on this base_url, if it is one of ours — decided from the process bound to
/// that port (both servers answer `/health`, so routes cannot tell them apart). None = something else
/// (LM Studio, llama-server, …) or nothing.
pub async fn server_kind(base_url: &str) -> Option<&'static str> {
    let port = port_of(base_url)?;
    for module in ["mlx_vlm.server", "mlx_lm.server"] { if !pids_of(module, port).is_empty() { return Some(module); } }
    None
}

fn port_of(base_url: &str) -> Option<u16> { base_url.trim_end_matches('/').trim_end_matches("/v1").rsplit(':').next()?.parse().ok() }

fn pids_of(module: &str, port: u16) -> Vec<u32> {
    let pat = format!("{} .*--port {port}( |$)", module.replace('.', "\\."));
    let Ok(o) = std::process::Command::new("pgrep").args(["-f", &pat]).output() else { return vec![] };
    String::from_utf8_lossy(&o.stdout).split_whitespace().filter_map(|p| p.parse().ok()).collect()
}

/// PIDs of *our* MLX server processes bound to `port` (matched on the command line, so LM Studio or a
/// llama-server on the same port are never touched).
pub fn pids_on_port(port: u16) -> Vec<u32> {
    let mut v = pids_of("mlx_vlm.server", port); v.extend(pids_of("mlx_lm.server", port)); v
}

/// Stop the MLX server we (or a previous harness) started on `port`; true if one was running.
pub async fn stop_on_port(port: u16) -> bool {
    let pids = pids_on_port(port);
    if pids.is_empty() { return false; }
    for p in &pids { let _ = std::process::Command::new("kill").arg(p.to_string()).status(); }
    for _ in 0..40 { if pids_on_port(port).is_empty() { break; } tokio::time::sleep(Duration::from_millis(250)).await; }
    for p in pids_on_port(port) { let _ = std::process::Command::new("kill").args(["-9", &p.to_string()]).status(); }
    true
}

/// What the configured local endpoint can actually do for us.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    /// Nothing listening.
    Down,
    /// It answers, but no model can serve a turn — LM Studio lists every model it has *downloaded*,
    /// loaded or not, which is exactly how "9 models available" turned into "No models loaded" on the
    /// first real request. `listed` is how many it advertised.
    Idle { listed: usize },
    /// A model is loaded and will answer.
    Ready { model: String },
}

impl Endpoint {
    pub fn ready(&self) -> bool { matches!(self, Endpoint::Ready { .. }) }
}

/// Ask the local endpoint whether it can serve a turn — not merely whether it is listening.
///
/// `/v1/models` cannot answer that on its own, so where a server exposes load state we use it: LM Studio's
/// `/api/v0/models` carries a `state` field per model. llama.cpp, `mlx_lm.server` and Ollama only serve
/// what they have loaded, so for them a non-empty list is the answer.
pub async fn probe(base_url: &str) -> Endpoint {
    let Ok(http) = reqwest::Client::builder().timeout(Duration::from_secs(3)).build() else { return Endpoint::Down };
    let base = base_url.trim_end_matches('/');
    let listed: Vec<String> = match http.get(format!("{base}/models")).send().await {
        Ok(r) if r.status().is_success() => match r.json::<serde_json::Value>().await {
            Ok(v) => v.get("data").and_then(|d| d.as_array()).map(|a| a.iter().filter_map(|m| m.get("id").and_then(|i| i.as_str()).map(String::from)).collect()).unwrap_or_default(),
            Err(_) => return Endpoint::Down,
        },
        _ => return Endpoint::Down,
    };
    // LM Studio: same host, native API, tells us which of those are actually resident.
    let native = format!("{}/api/v0/models", base.trim_end_matches("/v1"));
    if let Ok(r) = http.get(&native).send().await {
        if r.status().is_success() {
            if let Ok(v) = r.json::<serde_json::Value>().await {
                if let Some(a) = v.get("data").and_then(|d| d.as_array()) {
                    if a.iter().any(|m| m.get("state").is_some()) {
                        return match a.iter().find(|m| m.get("state").and_then(|s| s.as_str()) == Some("loaded")).and_then(|m| m.get("id")).and_then(|i| i.as_str()) {
                            Some(id) => Endpoint::Ready { model: id.to_string() },
                            None => Endpoint::Idle { listed: a.len() },
                        };
                    }
                }
            }
        }
    }
    // Prefer an entry that is one of our own model directories: mlx_vlm.server lists the HF cache too,
    // and the first id there can be some tiny test model that would answer nothing useful.
    let ours = models_dir().display().to_string();
    match listed.iter().find(|m| m.starts_with(&ours)).cloned().or_else(|| listed.into_iter().next()) {
        Some(model) => Endpoint::Ready { model },
        None => Endpoint::Idle { listed: 0 },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_are_the_three_quants() {
        assert_eq!(BUILDS.len(), 3);
        assert_eq!(by_bits(4).unwrap().name(), "Qwen3.8-27B-MLX-4bit");
        assert_eq!(by_bits(8).unwrap().bits, 8);
        assert!(by_bits(5).is_none());
        // sizes must be ordered, or the picker's advice is nonsense
        assert!(by_bits(4).unwrap().bytes < by_bits(6).unwrap().bytes);
        assert!(by_bits(6).unwrap().bytes < by_bits(8).unwrap().bytes);
        assert_eq!(by_name("Qwen3.8-27B-MLX-6bit").unwrap().bits, 6);
        assert_eq!(by_name("lmstudio-community/Qwen3.8-27B-MLX-6bit").unwrap().bits, 6);
    }

    #[test]
    fn progress_reads_like_a_status_line() {
        let p = Progress { done: 4_210_000_000, total: 16_080_000_000, bytes_per_sec: 78_400_000.0, eta_secs: 151, files_done: 1, files_total: 15 };
        let line = p.line();
        assert!(line.contains("3.92 GB"), "{line}");        // human() is binary-prefix
        assert!(line.contains("/s"), "{line}");
        assert!(line.contains("2m 31s left"), "{line}");
        assert!((p.percent() - 26.2).abs() < 0.5, "{}", p.percent());
        // a finished download shows no ETA
        let done = Progress { done: 10, total: 10, bytes_per_sec: 5.0, eta_secs: 9, ..Default::default() };
        assert!(!done.line().contains("left"), "{}", done.line());
        assert_eq!(Progress::default().percent(), 0.0);
    }

    #[test]
    fn eta_switches_units() {
        let hours = Progress { done: 1, total: 2, bytes_per_sec: 10.0, eta_secs: 3 * 3600 + 25 * 60, ..Default::default() };
        assert!(hours.line().contains("3h 25m left"), "{}", hours.line());
        let secs = Progress { done: 1, total: 2, bytes_per_sec: 10.0, eta_secs: 42, ..Default::default() };
        assert!(secs.line().contains("42s left"), "{}", secs.line());
    }

    #[test]
    fn model_id_prefers_our_directory_over_hf_cache_entries() {
        let v: serde_json::Value = serde_json::json!({"data": [
            {"id": "hf-internal-testing/tiny-random-Idefics3ForConditionalGeneration"},
            {"id": "HuggingFaceTB/SmolLM2-135M"},
            {"id": "/Users/x/.config/harness/models/Qwen3.8-27B-MLX-4bit"}]});
        assert_eq!(pick_model_id(&v, Path::new("/Users/x/.config/harness/models/Qwen3.8-27B-MLX-4bit")).as_deref(), Some("/Users/x/.config/harness/models/Qwen3.8-27B-MLX-4bit"));
        // mlx_lm.server may list just the directory name; fall back to a name match, then to the first id
        let v: serde_json::Value = serde_json::json!({"data": [{"id": "Qwen3.8-27B-MLX-4bit"}]});
        assert_eq!(pick_model_id(&v, Path::new("/elsewhere/Qwen3.8-27B-MLX-4bit")).as_deref(), Some("Qwen3.8-27B-MLX-4bit"));
        let v: serde_json::Value = serde_json::json!({"data": [{"id": "something-else"}]});
        assert_eq!(pick_model_id(&v, Path::new("/elsewhere/Qwen3.8-27B-MLX-4bit")).as_deref(), Some("something-else"));
    }
    #[test]
    fn server_plan_pins_or_falls_back() {
        assert_eq!(server_plan("mlx-lm"), vec!["mlx_lm.server"]);
        assert_eq!(server_plan("vision"), vec!["mlx_vlm.server"]);
        assert!(server_plan("auto").ends_with(&["mlx_lm.server"]));
    }

    /// Real install into a throwaway venv: HARNESS_CONFIG_DIR=<dir with runtime/mlx (uv venv)> cargo test ensure_mlx_vlm -- --ignored
    #[tokio::test] #[ignore]
    async fn ensure_mlx_vlm_installs_into_a_bare_runtime() {
        assert!(std::env::var_os("HARNESS_CONFIG_DIR").is_some(), "point HARNESS_CONFIG_DIR at a scratch dir with runtime/mlx");
        assert!(!has_mlx_vlm(), "the scratch runtime must start without mlx_vlm");
        let note = ensure_mlx_vlm().await.unwrap();
        assert!(note.is_some(), "expected an install note");
        assert!(has_mlx_vlm());
        assert_eq!(ensure_mlx_vlm().await.unwrap(), None, "second call is a no-op");
        assert_eq!(server_plan("auto")[0], "mlx_vlm.server");
    }
}
