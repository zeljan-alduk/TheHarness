//! Self-update from GitHub Releases — the same asset `docs/install.sh` downloads.
//!
//! The rule: **the binary is only ever replaced between sessions, never under a running one.** So the
//! update happens when `harness` starts (before the TUI opens): ask the GitHub API for the latest
//! release — at most once per `[update] interval_hours` — and when its tag is newer than the running
//! build, download the tarball, verify the published sha256, unpack it, run `--version` on the new
//! binary, swap it into place atomically (the old one stays next to it as `harness.prev` for
//! `harness update --rollback`) and re-exec into it with the same arguments. Anything that fails just
//! means starting the version you have. `harness update` does the same on demand; `/update` in the TUI
//! only checks and tells you — quitting and starting again is what applies it.
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

pub const REPO: &str = "zeljan-alduk/TheHarness";
/// The release asset for this build's target (release.yml packages exactly this name).
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
pub const ASSET: &str = "harness-aarch64-apple-darwin.tar.gz";
#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
pub const ASSET: &str = "";

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UpdateConfig {
    /// What `harness` does on start when a newer release exists: "auto" (default) installs it and re-execs
    /// into it before the TUI opens; "notify" only says so; "off" never asks GitHub.
    #[serde(default = "d_mode")] pub mode: String,
    /// Hours between checks (one anonymous GitHub API call each); the verdict is cached in
    /// ~/.config/harness/update-check.json in between. `harness update` always asks.
    #[serde(default = "d_interval")] pub interval_hours: u64,
}
fn d_mode() -> String { "auto".into() }
fn d_interval() -> u64 { 1 }
impl Default for UpdateConfig { fn default() -> Self { Self { mode: d_mode(), interval_hours: d_interval() } } }

/// MAJOR.MINOR.BUILD, comparable. `1.0.108-dev` parses as (1,0,108) with `dev = true`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version { pub major: u64, pub minor: u64, pub build: u64 }

impl Version {
    pub fn parse(s: &str) -> Option<Version> {
        let s = s.trim().trim_start_matches('v');
        let core = s.split(['-', '+', ' ', '(']).next()?;
        let mut it = core.split('.').map(|p| p.parse::<u64>().ok());
        let (major, minor) = (it.next()??, it.next()??);
        let build = it.next().flatten().unwrap_or(0);
        Some(Version { major, minor, build })
    }
    pub fn current() -> Version { Version::parse(crate::VERSION).unwrap_or(Version { major: 0, minor: 0, build: 0 }) }
}
impl std::fmt::Display for Version { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "{}.{}.{:03}", self.major, self.minor, self.build) } }

/// True for `cargo build` / `cargo run` binaries: a -dev version, or an exe living under a `target/` dir.
/// Those are the developer's own builds — the release channel is not what they want to be nagged about.
pub fn is_dev_build() -> bool {
    if crate::VERSION.contains("-dev") { return true; }
    installed_exe().map(|e| e.components().any(|c| c.as_os_str() == "target")).unwrap_or(false)
}

/// The binary an update replaces: the installed one (not a temp copy `harness self` runs from), symlinks resolved.
pub fn installed_exe() -> Result<PathBuf> {
    let p = crate::selfimprove::installed_exe()?;
    Ok(p.canonicalize().unwrap_or(p))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Release {
    pub tag: String,
    pub version: Version,
    pub url: String,
    pub sha_url: Option<String>,
    pub bytes: u64,
    pub notes: String,
    pub published: String,
    pub html_url: String,
}
impl Release { pub fn is_newer(&self) -> bool { self.version > Version::current() } }

impl Serialize for Version { fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> { s.serialize_str(&self.to_string()) } }
impl<'de> Deserialize<'de> for Version {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(d)?; Version::parse(&s).ok_or_else(|| serde::de::Error::custom(format!("bad version {s:?}")))
    }
}

fn http() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(600))
        .user_agent(concat!("harness/", env!("HARNESS_VERSION"), " (+https://github.com/zeljan-alduk/TheHarness)"))
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()?)
}

/// The latest published release, from the GitHub API (anonymous: 60 calls/hour, we make one).
pub async fn latest() -> Result<Release> {
    let api = std::env::var("HARNESS_UPDATE_API").unwrap_or_else(|_| format!("https://api.github.com/repos/{REPO}/releases/latest"));
    let http = http()?;
    let resp = http.get(&api).header("Accept", "application/vnd.github+json").timeout(Duration::from_secs(20)).send().await.context("reaching api.github.com")?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if status.as_u16() == 404 { bail!("no release published yet"); }
    if status.as_u16() == 403 && body.contains("rate limit") { bail!("GitHub API rate limit hit — try again in a while"); }
    if !status.is_success() { bail!("GitHub API answered {status}: {}", body.chars().take(200).collect::<String>()); }
    let v: serde_json::Value = serde_json::from_str(&body).context("parsing the release JSON")?;
    let tag = v["tag_name"].as_str().unwrap_or_default().to_string();
    let version = Version::parse(&tag).or_else(|| Version::parse(v["name"].as_str().unwrap_or_default())).with_context(|| format!("release tag {tag:?} is not a version"))?;
    if ASSET.is_empty() { bail!("no prebuilt binary is published for this platform (only macOS on Apple Silicon)"); }
    let assets = v["assets"].as_array().cloned().unwrap_or_default();
    let find = |name: &str| assets.iter().find(|a| a["name"].as_str() == Some(name)).cloned();
    let a = find(ASSET).with_context(|| format!("release {tag} has no {ASSET} — the release workflow may still be running"))?;
    Ok(Release {
        tag,
        version,
        url: a["browser_download_url"].as_str().unwrap_or_default().to_string(),
        sha_url: find(&format!("{ASSET}.sha256")).and_then(|s| s["browser_download_url"].as_str().map(str::to_string)),
        bytes: a["size"].as_u64().unwrap_or(0),
        notes: v["body"].as_str().unwrap_or_default().to_string(),
        published: v["published_at"].as_str().unwrap_or_default().to_string(),
        html_url: v["html_url"].as_str().unwrap_or_default().to_string(),
    })
}

// ───────────────────────── throttled start-up check ─────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct CheckCache { checked_at: u64, #[serde(default)] latest: Option<Release>, #[serde(default)] error: String }

fn cache_path() -> PathBuf { crate::setup::config_dir().join("update-check.json") }
fn now() -> u64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) }

/// A newer release, if there is one — asking GitHub at most once per `interval_hours` and remembering the
/// answer in between (so the "update available" line keeps showing until you actually update). The API
/// call gets `budget`: a start must not hang behind a captive portal, and a failure (offline included) is
/// cached like any other answer, so the next hour of starts stays quiet and instant.
pub async fn check_throttled(interval_hours: u64, budget: Duration) -> Option<Release> {
    let path = cache_path();
    let cached: CheckCache = std::fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    let fresh = cached.checked_at > 0 && now().saturating_sub(cached.checked_at) < interval_hours.max(1) * 3600;
    let latest = if fresh { cached.latest.clone() } else {
        let r = match tokio::time::timeout(budget, latest()).await { Ok(r) => r, Err(_) => Err(anyhow::anyhow!("GitHub did not answer within {}s", budget.as_secs())) };
        let c = CheckCache { checked_at: now(), latest: r.as_ref().ok().cloned(), error: r.as_ref().err().map(|e| format!("{e:#}")).unwrap_or_default() };
        if let Ok(s) = serde_json::to_string_pretty(&c) { let _ = std::fs::create_dir_all(path.parent().unwrap()); let _ = std::fs::write(&path, s); }
        r.ok()
    };
    latest.filter(|r| r.is_newer())
}

/// Forget the cached verdict (after an install, so the next start-up re-checks instead of repeating stale news).
pub fn forget_check() { let _ = std::fs::remove_file(cache_path()); }

// ───────────────────────── download + install ─────────────────────────

/// What the caller sees while an update runs (TUI banner lines, CLI progress).
#[derive(Debug, Clone)]
pub enum Stage {
    Checking,
    /// Already on the latest (or newer, e.g. a locally built harness).
    UpToDate { current: Version, latest: Version },
    Available(Release),
    Downloading { done: u64, total: u64 },
    Verifying,
    /// The new binary is in place; restart to run it.
    Installed { release: Release, exe: PathBuf, previous: Option<PathBuf> },
    Failed(String),
}

/// Download `rel`, verify, and replace `dest` with it. Returns the path of the kept previous binary.
pub async fn install(rel: &Release, dest: &Path, report: Arc<dyn Fn(Stage) + Send + Sync>) -> Result<Option<PathBuf>> {
    use futures_util::StreamExt;
    use sha2::Digest;
    let http = http()?;
    let tmp = std::env::temp_dir().join(format!("harness-update-{}-{}", rel.tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    tokio::fs::create_dir_all(&tmp).await?;
    let _guard = RmDir(tmp.clone());
    // 1. tarball, streamed to disk with progress
    let tar_path = tmp.join(ASSET);
    let resp = http.get(&rel.url).send().await.with_context(|| format!("GET {}", rel.url))?;
    if !resp.status().is_success() { bail!("download failed: HTTP {} for {}", resp.status(), rel.url); }
    let total = resp.content_length().unwrap_or(rel.bytes);
    let mut file = tokio::fs::File::create(&tar_path).await?;
    let mut hasher = sha2::Sha256::new();
    let mut done: u64 = 0;
    let mut stream = resp.bytes_stream();
    let mut last = std::time::Instant::now();
    report(Stage::Downloading { done: 0, total });
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("reading the download")?;
        tokio::io::AsyncWriteExt::write_all(&mut file, &chunk).await?;
        hasher.update(&chunk);
        done += chunk.len() as u64;
        if last.elapsed() > Duration::from_millis(100) { report(Stage::Downloading { done, total }); last = std::time::Instant::now(); }
    }
    tokio::io::AsyncWriteExt::flush(&mut file).await?;
    drop(file);
    report(Stage::Downloading { done, total: total.max(done) });
    // 2. checksum, against the .sha256 the release workflow publishes next to the tarball
    report(Stage::Verifying);
    let got = format!("{:x}", hasher.finalize());
    match &rel.sha_url {
        Some(u) => {
            let text = http.get(u).send().await.and_then(|r| r.error_for_status()).context("fetching the .sha256")?.text().await?;
            let want = text.split_whitespace().next().unwrap_or_default().to_lowercase();
            if want.len() != 64 { bail!("the published .sha256 is malformed: {text:?}"); }
            if want != got { bail!("checksum mismatch: release says {want}, download is {got} — not installing"); }
        }
        None => bail!("release {} has no .sha256 next to the tarball — not installing an unverifiable binary", rel.tag),
    }
    // 3. unpack (tar is always there on macOS; the installer uses the same command)
    let st = tokio::process::Command::new("tar").arg("-xzf").arg(&tar_path).arg("-C").arg(&tmp).status().await.context("running tar")?;
    if !st.success() { bail!("tar failed to unpack {}", tar_path.display()); }
    let new_bin = tmp.join("harness");
    if !new_bin.is_file() { bail!("the tarball did not contain a `harness` binary"); }
    #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; std::fs::set_permissions(&new_bin, std::fs::Permissions::from_mode(0o755))?; }
    // 4. does it run at all? (a truncated or wrong-arch binary should never reach $PATH)
    let out = tokio::process::Command::new(&new_bin).arg("--version").env("HARNESS_NO_KITTY", "1").output().await.context("running the new binary")?;
    let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() || !v.contains(&rel.version.to_string()) {
        bail!("the downloaded harness does not report {} (`--version` said {:?}, exit {}) — not installing", rel.version, v, out.status);
    }
    // 5. keep the current binary as .prev, then rename the new one over dest (atomic; a running harness keeps its inode)
    let dest = dest.canonicalize().unwrap_or(dest.to_path_buf());
    let prev = dest.with_extension("prev");
    let mut kept = None;
    if dest.is_file() {
        let _ = std::fs::remove_file(&prev);
        match std::fs::rename(&dest, &prev) {
            Ok(()) => kept = Some(prev.clone()),
            Err(e) => bail!("cannot replace {}: {e} (is it writable? re-run with the right permissions, or re-run the installer)", dest.display()),
        }
    }
    if let Err(e) = crate::selfimprove::install_binary(&new_bin, &dest) {
        if let Some(p) = &kept { let _ = std::fs::rename(p, &dest); }   // put the old one back
        return Err(e);
    }
    forget_check();
    Ok(kept)
}

/// The whole thing: check, and install when newer. `force` installs even when not newer (same version re-download).
pub async fn run(dest: &Path, force: bool, report: Arc<dyn Fn(Stage) + Send + Sync>) -> Result<Stage> {
    report(Stage::Checking);
    let rel = latest().await?;
    let cur = Version::current();
    if !rel.is_newer() && !force { let s = Stage::UpToDate { current: cur, latest: rel.version }; report(s.clone()); return Ok(s); }
    report(Stage::Available(rel.clone()));
    let previous = install(&rel, dest, report.clone()).await?;
    let s = Stage::Installed { release: rel, exe: dest.to_path_buf(), previous };
    report(s.clone());
    Ok(s)
}

/// Swap `harness.prev` back in (undo the last update).
pub fn rollback(dest: &Path) -> Result<PathBuf> {
    let dest = dest.canonicalize().unwrap_or(dest.to_path_buf());
    let prev = dest.with_extension("prev");
    if !prev.is_file() { bail!("nothing to roll back to: {} does not exist", prev.display()); }
    let bad = dest.with_extension("rolled-back");
    let _ = std::fs::remove_file(&bad);
    if dest.is_file() { std::fs::rename(&dest, &bad).with_context(|| format!("moving {} aside", dest.display()))?; }
    if let Err(e) = std::fs::rename(&prev, &dest) { let _ = std::fs::rename(&bad, &dest); return Err(e).with_context(|| format!("restoring {}", prev.display())); }
    let _ = std::fs::remove_file(&bad);
    forget_check();
    Ok(dest)
}

/// What the start-up pass concluded (main.rs re-execs on `Installed`).
#[derive(Debug, Clone)]
pub enum Startup {
    /// Not looked: mode off, dev build, HARNESS_NO_UPDATE, CI, or a re-exec after an update (loop guard).
    Skipped(&'static str),
    UpToDate,
    /// mode = "notify", or the install failed (the message says which); the TUI shows it.
    Available { release: Release, note: String },
    Installed { release: Release, exe: PathBuf },
}

/// Reasons not to touch the network at start.
pub fn skip_reason() -> Option<&'static str> {
    if std::env::var_os("HARNESS_NO_UPDATE").is_some() { return Some("HARNESS_NO_UPDATE is set"); }
    if std::env::var_os("HARNESS_UPDATED_FROM").is_some() { return Some("just updated"); }
    if std::env::var_os("HARNESS_SELF_EXEC").is_some() { return Some("running from a temp copy (self mode)"); }
    if std::env::var_os("CI").is_some() { return Some("CI"); }
    if is_dev_build() { return Some("development build"); }
    None
}

/// The start-of-`harness` pass: throttled check, then (mode = auto) install. Never fails the start —
/// every error becomes an `Available { note }` or a plain `UpToDate` so the version you have still runs.
pub async fn startup(cfg: &UpdateConfig, report: Arc<dyn Fn(Stage) + Send + Sync>) -> Startup {
    if cfg.mode == "off" { return Startup::Skipped("[update] mode = off"); }
    if let Some(r) = skip_reason() { return Startup::Skipped(r); }
    let Some(rel) = check_throttled(cfg.interval_hours, Duration::from_secs(8)).await else { return Startup::UpToDate };
    if cfg.mode != "auto" { return Startup::Available { note: "run `harness update` to install it".into(), release: rel }; }
    report(Stage::Available(rel.clone()));
    let dest = match installed_exe() { Ok(d) => d, Err(e) => return Startup::Available { note: format!("cannot locate the installed binary: {e:#}"), release: rel } };
    // A start should not hang on a slow link: cap the download; `harness update` has no cap.
    match tokio::time::timeout(Duration::from_secs(180), install(&rel, &dest, report.clone())).await {
        Ok(Ok(_prev)) => Startup::Installed { release: rel, exe: dest },
        Ok(Err(e)) => Startup::Available { note: format!("update failed: {e:#} — run `harness update` to retry"), release: rel },
        Err(_) => Startup::Available { note: "the download took more than 3 minutes — skipped; run `harness update` to retry".into(), release: rel },
    }
}

struct RmDir(PathBuf);
impl Drop for RmDir { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }

pub fn fmt_bytes(b: u64) -> String {
    if b >= 1 << 30 { format!("{:.1}GB", b as f64 / (1u64 << 30) as f64) } else if b >= 1 << 20 { format!("{:.1}MB", b as f64 / (1u64 << 20) as f64) } else if b >= 1 << 10 { format!("{}KB", b >> 10) } else { format!("{b}B") }
}

/// The first line of the release notes that says something (release.yml puts the commit subjects first),
/// for one-line summaries: no code fences, no install/sha lines, no leading list dash.
pub fn headline(notes: &str) -> String {
    let boring = |l: &str| l.is_empty() || l.starts_with("```") || l.starts_with("curl ") || l.starts_with("sha256:") || l.starts_with("Install or update") || l.ends_with("install with:");
    notes.lines().map(str::trim).find(|l| !boring(l)).unwrap_or_default().trim_start_matches("- ").chars().take(120).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn versions_parse_and_order() {
        assert_eq!(Version::parse("v1.0.108"), Some(Version { major: 1, minor: 0, build: 108 }));
        assert_eq!(Version::parse("1.0.108-dev"), Some(Version { major: 1, minor: 0, build: 108 }));
        assert_eq!(Version::parse("1.0.108 (abc123)"), Some(Version { major: 1, minor: 0, build: 108 }));
        assert_eq!(Version::parse("1.0"), Some(Version { major: 1, minor: 0, build: 0 }));
        assert_eq!(Version::parse("nightly"), None);
        assert!(Version::parse("1.0.109").unwrap() > Version::parse("1.0.108").unwrap());
        assert!(Version::parse("1.1.0").unwrap() > Version::parse("1.0.999").unwrap());
        assert!(Version::parse("2.0.0").unwrap() > Version::parse("1.9.9").unwrap());
        assert_eq!(Version::parse("1.0.7").unwrap().to_string(), "1.0.007");
    }
    #[test]
    fn current_version_parses() { assert!(Version::current().major >= 1); }
    #[test]
    fn rollback_swaps_prev_back() {
        let d = std::env::temp_dir().join(format!("harness-rb-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d); std::fs::create_dir_all(&d).unwrap();
        let exe = d.join("harness"); std::fs::write(&exe, "new").unwrap(); std::fs::write(d.join("harness.prev"), "old").unwrap();
        rollback(&exe).unwrap();
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "old");
        assert!(!d.join("harness.prev").exists());
        assert!(rollback(&exe).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }
    #[test]
    fn notes_headline_skips_boilerplate() {
        assert_eq!(headline("harness 1.0.108 — install with:\n```sh\ncurl x\n```"), "");
        assert_eq!(headline("- Fix the thing\n- Another\n\nInstall or update: `curl …`\nsha256: `abc`"), "Fix the thing");
        assert_eq!(headline(""), "");
    }
}
