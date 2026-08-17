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
}

impl Server {
    pub async fn stop(mut self) { let _ = self.child.kill().await; }
    pub fn pid(&self) -> Option<u32> { self.child.id() }
}

/// Serve a downloaded model over an OpenAI-compatible API on 127.0.0.1.
///
/// `mlx_lm.server` is the default: it knows the qwen3_5 architecture and is the more reliable of the two.
/// `mlx_vlm.server` adds the vision tower (the model is image-text-to-text) but refuses some builds, so it
/// is opt-in via `[local_model] server = "mlx-vlm"`.
pub async fn serve(model_dir: &Path, port: u16, kind: &str) -> Result<Server> {
    let py = mlx_python().context("no MLX runtime — re-run the installer, or: uv venv ~/.config/harness/runtime/mlx && uv pip install --python ~/.config/harness/runtime/mlx/bin/python mlx-lm")?;
    let module = if kind == "mlx-vlm" || kind == "mlx_vlm" { "mlx_vlm.server" } else { "mlx_lm.server" };
    let log_dir = crate::setup::config_dir().join("logs");
    std::fs::create_dir_all(&log_dir).ok();
    let log = std::fs::OpenOptions::new().create(true).append(true).open(log_dir.join("mlx-server.log"))?;
    let child = tokio::process::Command::new(&py)
        .args(["-m", module, "--host", "127.0.0.1", "--port", &port.to_string(), "--model"])
        .arg(model_dir)
        .stdout(log.try_clone()?)
        .stderr(log)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(false)
        .spawn()
        .with_context(|| format!("starting {module}"))?;

    // Loading 16–30GB of weights takes a while; poll until it answers or dies.
    let base_url = format!("http://127.0.0.1:{port}/v1");
    let http = reqwest::Client::builder().timeout(Duration::from_secs(3)).build()?;
    let mut server = Server { base_url: base_url.clone(), model: model_dir.display().to_string(), port, child };
    for _ in 0..600 {
        if let Ok(Some(status)) = server.child.try_wait() {
            bail!("{module} exited ({status}) — see {}", log_dir.join("mlx-server.log").display());
        }
        if let Ok(r) = http.get(format!("{base_url}/models")).send().await {
            if r.status().is_success() {
                if let Ok(v) = r.json::<serde_json::Value>().await {
                    if let Some(id) = v.get("data").and_then(|d| d.as_array()).and_then(|a| a.first()).and_then(|m| m.get("id")).and_then(|i| i.as_str()) {
                        server.model = id.to_string();
                    }
                }
                return Ok(server);
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let _ = server.child.kill().await;
    bail!("{module} did not answer on {base_url} within 5 minutes — see {}", log_dir.join("mlx-server.log").display())
}

/// Is something already serving an OpenAI-compatible API here?
pub async fn reachable(base_url: &str) -> bool {
    let Ok(http) = reqwest::Client::builder().timeout(Duration::from_secs(2)).build() else { return false };
    let url = format!("{}/models", base_url.trim_end_matches('/'));
    match http.get(&url).send().await {
        Ok(r) if r.status().is_success() => r.json::<serde_json::Value>().await
            .map(|v| v.get("data").and_then(|d| d.as_array()).map(|a| !a.is_empty()).unwrap_or(false))
            .unwrap_or(false),
        _ => false,
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
}
