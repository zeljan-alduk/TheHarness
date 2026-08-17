//! download_file: large-file HTTP(S) download with parallel range segments, a persisted
//! state file, automatic retry and resume. Designed so a call that dies mid-way (timeout,
//! network drop, harness restart) picks up exactly where it left off on the next call.

use super::{arg_str, Tool, ToolCtx, ToolOutput};
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
#[cfg(unix)] use std::os::unix::fs::FileExt;
#[cfg(windows)] use std::os::windows::fs::FileExt;

fn write_at(f: &std::fs::File, buf: &[u8], off: u64) -> std::io::Result<()> {
    #[cfg(unix)] { f.write_all_at(buf, off) }
    #[cfg(windows)] { let mut done = 0usize; while done < buf.len() { let n = f.seek_write(&buf[done..], off + done as u64)?; if n == 0 { return Err(std::io::Error::new(std::io::ErrorKind::WriteZero, "write zero")); } done += n; } Ok(()) }
}
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub struct DownloadFile;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Segment { start: u64, end: u64, done: u64 } // inclusive end; done = bytes written from start

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DlState { url: String, size: u64, segments: Vec<Segment>, ranges: bool }

impl DlState {
    fn remaining(&self) -> u64 { self.segments.iter().map(|s| (s.end + 1 - s.start).saturating_sub(s.done)).sum() }
    fn downloaded(&self) -> u64 { self.segments.iter().map(|s| s.done).sum() }
    fn complete(&self) -> bool { self.remaining() == 0 }
}

fn state_path(dest: &Path) -> PathBuf {
    let mut s = dest.as_os_str().to_owned(); s.push(".harness-dl.json"); PathBuf::from(s)
}

fn client(ctx: &ToolCtx) -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(ctx.net.timeout_secs))
        .user_agent(&ctx.net.user_agent)
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()?)
}

/// Probe size and range support with a 1-byte ranged GET (HEAD is unreliable on many CDNs).
async fn probe(http: &reqwest::Client, url: &str) -> Result<(Option<u64>, bool, String)> {
    let r = http.get(url).header(reqwest::header::RANGE, "bytes=0-0").send().await.with_context(|| format!("GET {url}"))?;
    let status = r.status();
    if !(status.is_success()) { bail!("HTTP {status} for {url}"); }
    let final_url = r.url().to_string();
    if status == reqwest::StatusCode::PARTIAL_CONTENT {
        // Content-Range: bytes 0-0/12345
        let total = r.headers().get(reqwest::header::CONTENT_RANGE).and_then(|v| v.to_str().ok())
            .and_then(|s| s.rsplit('/').next()).and_then(|t| t.parse::<u64>().ok());
        return Ok((total, true, final_url));
    }
    let len = r.headers().get(reqwest::header::CONTENT_LENGTH).and_then(|v| v.to_str().ok()).and_then(|s| s.parse::<u64>().ok());
    Ok((len, false, final_url))
}

fn plan(url: &str, size: u64, ranges: bool, nseg: usize) -> DlState {
    if !ranges || size == 0 || nseg <= 1 || size < 4 * 1024 * 1024 {
        return DlState { url: url.into(), size, segments: vec![Segment { start: 0, end: size.saturating_sub(1), done: 0 }], ranges };
    }
    let n = nseg.min(16) as u64;
    let chunk = size / n;
    let segments = (0..n).map(|i| {
        let start = i * chunk;
        let end = if i == n - 1 { size - 1 } else { (i + 1) * chunk - 1 };
        Segment { start, end, done: 0 }
    }).collect();
    DlState { url: url.into(), size, segments, ranges }
}

async fn run_segment(http: reqwest::Client, url: String, file: Arc<std::fs::File>, state: Arc<Mutex<DlState>>, idx: usize, ranged: bool, state_file: PathBuf) -> Result<()> {
    let mut attempt = 0u32;
    loop {
        let (start, end, done) = { let s = state.lock().unwrap(); let g = &s.segments[idx]; (g.start, g.end, g.done) };
        if end + 1 - start <= done { return Ok(()); }
        let from = start + done;
        let mut req = http.get(&url);
        if ranged { req = req.header(reqwest::header::RANGE, format!("bytes={from}-{end}")); }
        let res: Result<()> = async {
            let r = req.send().await?;
            if ranged && r.status() != reqwest::StatusCode::PARTIAL_CONTENT { bail!("server ignored Range (HTTP {})", r.status()); }
            if !r.status().is_success() { bail!("HTTP {}", r.status()); }
            let mut stream = r.bytes_stream();
            let mut off = from;
            let mut since_save = 0u64;
            let mut last_data = Instant::now();
            loop {
                let next = tokio::time::timeout(Duration::from_secs(60), stream.next()).await;
                let Ok(item) = next else { bail!("stalled for 60s") };
                let Some(item) = item else { break };
                let bytes = item?;
                let len = bytes.len() as u64;
                let f = file.clone();
                let o = off;
                tokio::task::spawn_blocking(move || write_at(&f, &bytes, o)).await??;
                off += len;
                since_save += len;
                { let mut s = state.lock().unwrap(); s.segments[idx].done = off - start; }
                if since_save > 4 * 1024 * 1024 || last_data.elapsed() > Duration::from_secs(2) {
                    save_state(&state_file, &state);
                    since_save = 0; last_data = Instant::now();
                }
            }
            Ok(())
        }.await;
        save_state(&state_file, &state);
        match res {
            Ok(()) => {
                let s = state.lock().unwrap();
                let g = &s.segments[idx];
                if ranged && g.done < g.end + 1 - g.start { drop(s); attempt += 1; if attempt > 8 { bail!("segment {idx} short after 8 attempts"); } continue; }
                return Ok(());
            }
            Err(e) => {
                attempt += 1;
                if attempt > 8 { bail!("segment {idx} failed after 8 attempts: {e:#}"); }
                tokio::time::sleep(Duration::from_millis(500 * 2u64.pow(attempt.min(6)))).await;
            }
        }
    }
}

fn save_state(path: &Path, state: &Arc<Mutex<DlState>>) {
    let s = state.lock().unwrap().clone();
    if let Ok(j) = serde_json::to_vec(&s) { let _ = std::fs::write(path, j); }
}

pub fn human(n: u64) -> String {
    if n < 1024 { format!("{n} B") } else if n < 1 << 20 { format!("{:.1} KB", n as f64 / 1024.0) } else if n < 1 << 30 { format!("{:.1} MB", n as f64 / 1048576.0) } else { format!("{:.2} GB", n as f64 / 1073741824.0) }
}

/// The same segmented, checkpointed, resuming download as the `download_file` tool, for callers inside
/// the harness (the first-run model fetch). `tick(done, total)` is called ~4×/s with the byte counts of
/// *this* file, so a caller can render progress, speed and an ETA.
///
/// The tool's `call` keeps its own copy of this orchestration because it layers extra behaviour on top
/// (sha256 verification, overwrite, "already complete" reporting, unknown-size streaming); both share
/// the planning, segment and state-file machinery below.
pub async fn fetch_resumable(
    http: &reqwest::Client,
    url: &str,
    dest: &Path,
    nseg: usize,
    timeout: Duration,
    tick: Option<Arc<dyn Fn(u64, u64) + Send + Sync>>,
) -> Result<u64> {
    if let Some(p) = dest.parent() { tokio::fs::create_dir_all(p).await?; }
    let sfile = state_path(dest);
    // A finished file has no state file next to it: nothing to do.
    let mut prior: Option<DlState> = None;
    if sfile.is_file() && dest.is_file() {
        if let Ok(s) = serde_json::from_slice::<DlState>(&std::fs::read(&sfile)?) {
            if s.url == url && !s.complete() { prior = Some(s); }
        }
    }
    let (probe_size, ranges, final_url) = probe(http, url).await?;
    let size = probe_size.unwrap_or(0);
    if prior.is_none() && dest.is_file() && !sfile.is_file() {
        let have = std::fs::metadata(dest)?.len();
        if size == 0 || have == size {
            if let Some(t) = &tick { t(have, have); }
            return Ok(have);
        }
    }
    let state = match prior {
        Some(s) => s,
        None => {
            let f = std::fs::File::create(dest)?;
            if size > 0 { f.set_len(size)?; }
            plan(url, size, ranges, nseg)
        }
    };
    let ranged = state.ranges && state.size > 0;
    let total = state.size;
    let file = Arc::new(std::fs::OpenOptions::new().write(true).open(dest)?);
    let state = Arc::new(Mutex::new(state));
    save_state(&sfile, &state);

    let n = state.lock().unwrap().segments.len();
    let mut tasks = tokio::task::JoinSet::new();
    for idx in 0..n {
        tasks.spawn(run_segment(http.clone(), final_url.clone(), file.clone(), state.clone(), idx, ranged, sfile.clone()));
    }
    // Report from the shared state rather than from the segments, so resumed bytes are included.
    let watcher = tick.map(|t| {
        let st = state.clone();
        tokio::spawn(async move {
            loop {
                { let s = st.lock().unwrap(); t(s.downloaded(), s.size); }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        })
    });
    let all = async { let mut errs = Vec::new(); while let Some(r) = tasks.join_next().await { match r { Ok(Ok(())) => {}, Ok(Err(e)) => errs.push(format!("{e:#}")), Err(e) => errs.push(e.to_string()) } } errs };
    let outcome = tokio::time::timeout(timeout, all).await;
    if let Some(w) = watcher { w.abort(); }
    let errs = match outcome {
        Ok(errs) => errs,
        Err(_) => { save_state(&sfile, &state); bail!("timed out after {}s ({} of {})", timeout.as_secs(), human(state.lock().unwrap().downloaded()), human(total)); }
    };
    let downloaded = state.lock().unwrap().downloaded();
    if !errs.is_empty() || (total > 0 && downloaded < total) {
        save_state(&sfile, &state);
        bail!("incomplete ({} of {}){}{}", human(downloaded), human(total), if errs.is_empty() { "" } else { ": " }, errs.join("; "));
    }
    let _ = std::fs::remove_file(&sfile);   // no state file = complete
    let got = std::fs::metadata(dest)?.len();
    Ok(got)
}

#[async_trait]
impl Tool for DownloadFile {
    fn name(&self) -> &'static str { "download_file" }
    fn description(&self) -> &'static str {
        "Download a file from an http(s) URL into the working directory. Large files are fetched with parallel range segments; progress is checkpointed so an interrupted or timed-out download resumes automatically when you call this again with the same url+path. Optionally verifies sha256. Use web_search / web_fetch to find the URL first."
    }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{
            "url":{"type":"string"},
            "path":{"type":"string","description":"destination file path (default: file name from the URL)"},
            "segments":{"type":"integer","description":"parallel segments (default from config, max 16)"},
            "sha256":{"type":"string","description":"expected hex digest; verified after download"},
            "timeout_secs":{"type":"integer","description":"give up (keeping partial state) after this long; default from config"},
            "overwrite":{"type":"boolean","description":"discard any existing file/partial state and start over"}
        },"required":["url"]})
    }
    async fn call(&self, args: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let url = arg_str(&args, "url")?.to_string();
        if !(url.starts_with("http://") || url.starts_with("https://")) { bail!("only http(s) URLs are allowed"); }
        let default_name = url.split('?').next().unwrap_or(&url).rsplit('/').next().filter(|s| !s.is_empty()).unwrap_or("download.bin").to_string();
        let dest = ctx.resolve(args.get("path").and_then(|v| v.as_str()).unwrap_or(&default_name))?;
        let nseg = args.get("segments").and_then(|v| v.as_u64()).map(|n| n as usize).unwrap_or(ctx.net.download_segments).clamp(1, 16);
        let timeout = Duration::from_secs(args.get("timeout_secs").and_then(|v| v.as_u64()).unwrap_or(ctx.net.download_timeout_secs));
        let overwrite = args.get("overwrite").and_then(|v| v.as_bool()).unwrap_or(false);
        let expect_sha = args.get("sha256").and_then(|v| v.as_str()).map(|s| s.trim().to_ascii_lowercase());
        if let Some(p) = dest.parent() { tokio::fs::create_dir_all(p).await?; }
        let sfile = state_path(&dest);
        let http = client(ctx)?;
        let t0 = Instant::now();

        if overwrite { let _ = std::fs::remove_file(&dest); let _ = std::fs::remove_file(&sfile); }

        // Resume?
        let mut resumed = false;
        let mut state: Option<DlState> = None;
        if sfile.is_file() && dest.is_file() {
            if let Ok(s) = serde_json::from_slice::<DlState>(&std::fs::read(&sfile)?) {
                if s.url == url && !s.complete() { resumed = true; state = Some(s); }
            }
        }
        if state.is_none() && dest.is_file() && !sfile.is_file() && !overwrite {
            let n = std::fs::metadata(&dest)?.len();
            let mut msg = format!("{} already exists ({}) and is complete (no partial state). Pass overwrite=true to re-download.", dest.display(), human(n));
            if let Some(exp) = &expect_sha { let got = sha256_file(&dest).await?; msg.push_str(&format!(" sha256 {}", if &got == exp { "matches" } else { "MISMATCH" })); }
            return Ok(msg.into());
        }
        let (size, ranges, final_url) = probe(&http, &url).await?;
        let state = match state {
            Some(s) => s,
            None => {
                let size = size.unwrap_or(0);
                let f = std::fs::File::create(&dest)?;
                if size > 0 { f.set_len(size)?; }
                plan(&url, size, ranges, nseg)
            }
        };
        let ranged = state.ranges;
        let unknown_size = state.size == 0;
        let file = Arc::new(std::fs::OpenOptions::new().write(true).open(&dest)?);
        let state = Arc::new(Mutex::new(state));
        save_state(&sfile, &state);

        let n = state.lock().unwrap().segments.len();
        let mut tasks = tokio::task::JoinSet::new();
        for idx in 0..n {
            tasks.spawn(run_segment(http.clone(), final_url.clone(), file.clone(), state.clone(), idx, ranged && !unknown_size, sfile.clone()));
        }
        let all = async { let mut errs = Vec::new(); while let Some(r) = tasks.join_next().await { match r { Ok(Ok(())) => {}, Ok(Err(e)) => errs.push(format!("{e:#}")), Err(e) => errs.push(e.to_string()) } } errs };
        let errs = match tokio::time::timeout(timeout, all).await {
            Ok(errs) => errs,
            Err(_) => {
                save_state(&sfile, &state);
                let s = state.lock().unwrap();
                bail!("timed out after {}s with {} of {} downloaded; partial state saved — call download_file again with the same url and path to resume", timeout.as_secs(), human(s.downloaded()), human(s.size));
            }
        };
        let (downloaded, size) = { let s = state.lock().unwrap(); (s.downloaded(), s.size) };
        if unknown_size {
            // streamed to end; size becomes what we got
            let got = std::fs::metadata(&dest)?.len();
            let _ = std::fs::remove_file(&sfile);
            if !errs.is_empty() { bail!("download failed: {}", errs.join("; ")); }
            return finish(&dest, got, resumed, 1, t0, expect_sha).await;
        }
        if !errs.is_empty() || downloaded < size {
            save_state(&sfile, &state);
            bail!("download incomplete ({} of {}): {}. Partial state saved — call download_file again with the same url and path to resume.", human(downloaded), human(size), errs.join("; "));
        }
        let _ = std::fs::remove_file(&sfile);
        finish(&dest, size, resumed, n, t0, expect_sha).await
    }
}

async fn sha256_file(p: &Path) -> Result<String> {
    let p = p.to_path_buf();
    tokio::task::spawn_blocking(move || -> Result<String> {
        let mut f = std::fs::File::open(&p)?;
        let mut h = Sha256::new();
        std::io::copy(&mut f, &mut h)?;
        Ok(format!("{:x}", h.finalize()))
    }).await?
}

async fn finish(dest: &Path, size: u64, resumed: bool, segs: usize, t0: Instant, expect_sha: Option<String>) -> Result<ToolOutput> {
    let secs = t0.elapsed().as_secs_f64().max(0.001);
    let mut msg = format!("downloaded {} → {} ({}, {} segment{}, {:.1}s, {}/s{})", human(size), dest.display(), size, segs, if segs == 1 { "" } else { "s" }, secs, human((size as f64 / secs) as u64), if resumed { ", resumed" } else { "" });
    if let Some(exp) = expect_sha {
        let got = sha256_file(dest).await?;
        if got == exp { msg.push_str("\nsha256 verified"); } else { bail!("{msg}\nsha256 MISMATCH: expected {exp}, got {got}. Re-run with overwrite=true."); }
    }
    Ok(msg.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn plans_segments() {
        let s = plan("u", 100 * 1024 * 1024, true, 4);
        assert_eq!(s.segments.len(), 4);
        assert_eq!(s.segments[0].start, 0);
        assert_eq!(s.segments[3].end, 100 * 1024 * 1024 - 1);
        let total: u64 = s.segments.iter().map(|g| g.end + 1 - g.start).sum();
        assert_eq!(total, 100 * 1024 * 1024);
        assert_eq!(plan("u", 1000, true, 4).segments.len(), 1); // small → single
        assert_eq!(plan("u", 1 << 30, false, 4).segments.len(), 1); // no ranges → single
    }
    #[test]
    fn state_roundtrip() {
        let mut s = plan("u", 8 * 1024 * 1024, true, 2);
        assert!(!s.complete());
        s.segments[0].done = s.segments[0].end + 1;
        s.segments[1].done = s.segments[1].end + 1 - s.segments[1].start;
        assert!(s.complete());
        let j = serde_json::to_string(&s).unwrap();
        let back: DlState = serde_json::from_str(&j).unwrap();
        assert_eq!(back.downloaded(), 8 * 1024 * 1024);
    }
}
