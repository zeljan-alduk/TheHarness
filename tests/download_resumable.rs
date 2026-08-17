//! The segmented, resuming download, tested against a local server that speaks just enough HTTP.
//!
//! No network and no 16GB model: the point is the machinery the first-run model fetch relies on —
//! parallel range requests, a checkpoint that survives an interruption, and a resume that re-downloads
//! only what is missing.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

const SIZE: usize = 8 * 1024 * 1024; // big enough that the planner splits it

fn body() -> Vec<u8> { (0..SIZE).map(|i| (i % 251) as u8).collect() }

/// Serves `GET /file` with Range support. `served` counts bytes actually written, so a test can prove a
/// resume did not re-download the whole thing. `cut_after` closes the connection early to simulate a drop.
fn serve(served: Arc<AtomicU64>, cut_after: Option<usize>) -> (String, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().unwrap();
    let handle = std::thread::spawn(move || {
        let data = body();
        // one connection per range request; the client opens several
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { break };
            if handle_one(&mut s, &data, &served, cut_after).is_err() { /* client hung up */ }
        }
    });
    (format!("http://{addr}/file"), handle)
}

fn handle_one(s: &mut TcpStream, data: &[u8], served: &AtomicU64, cut_after: Option<usize>) -> std::io::Result<()> {
    let mut reader = BufReader::new(s.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    if line.is_empty() { return Ok(()); }
    let mut range: Option<(usize, usize)> = None;
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h)? == 0 { break; }
        if h == "\r\n" || h == "\n" { break; }
        if let Some(v) = h.to_ascii_lowercase().strip_prefix("range: bytes=").map(|v| v.trim().to_string()) {
            let (a, b) = v.split_once('-').unwrap_or((v.as_str(), ""));
            let start: usize = a.parse().unwrap_or(0);
            let end: usize = b.trim().parse().unwrap_or(data.len() - 1);
            range = Some((start, end.min(data.len() - 1)));
        }
    }
    let (start, end) = range.unwrap_or((0, data.len() - 1));
    let slice = &data[start..=end];
    let head = if range.is_some() {
        format!("HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n", slice.len(), start, end, data.len())
    } else {
        format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n", slice.len())
    };
    s.write_all(head.as_bytes())?;
    let stop = cut_after.map(|n| n.min(slice.len())).unwrap_or(slice.len());
    // Count before writing: the client may hang up as soon as it has what it needs (a 1-byte probe does),
    // and then write_all fails with EPIPE and never gets to increment — which made this flaky on CI.
    served.fetch_add(stop as u64, Ordering::Relaxed);
    let _ = s.write_all(&slice[..stop]);
    let _ = s.flush();
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn downloads_in_segments_and_reports_progress() {
    let served = Arc::new(AtomicU64::new(0));
    let (url, _srv) = serve(served.clone(), None);
    let dir = std::env::temp_dir().join(format!("harness-dl-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let dest = dir.join("blob.bin");
    let _ = std::fs::remove_file(&dest);

    let seen = Arc::new(std::sync::Mutex::new(Vec::<(u64, u64)>::new()));
    let s2 = seen.clone();
    let http = reqwest::Client::new();
    let got = harness::tools::download::fetch_resumable(
        &http, &url, &dest, 4, Duration::from_secs(60),
        Some(Arc::new(move |done, total| s2.lock().unwrap().push((done, total)))),
    ).await.expect("download");

    assert_eq!(got, SIZE as u64, "whole file");
    assert_eq!(std::fs::read(&dest).unwrap(), body(), "bytes are in the right order");
    let ticks = seen.lock().unwrap().clone();
    assert!(!ticks.is_empty(), "progress was reported");
    assert!(ticks.iter().all(|(_, total)| *total == SIZE as u64), "total is the file size");
    // the checkpoint is cleaned up on success — its absence is what marks a file complete
    assert!(!dir.join("blob.bin.harness-dl.json").exists(), "state file removed");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn resumes_without_refetching_what_it_already_has() {
    let dir = std::env::temp_dir().join(format!("harness-dl-resume-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let dest = dir.join("blob.bin");
    let _ = std::fs::remove_file(&dest);

    // 1) a server that truncates every response: the download must fail, leaving a checkpoint
    let cut = Arc::new(AtomicU64::new(0));
    let (url1, _s1) = serve(cut.clone(), Some(64 * 1024));
    let http = reqwest::Client::new();
    let first = harness::tools::download::fetch_resumable(&http, &url1, &dest, 4, Duration::from_secs(30), None).await;
    assert!(first.is_err(), "a truncated download reports failure: {first:?}");
    let state = dir.join("blob.bin.harness-dl.json");
    assert!(state.exists(), "partial progress is checkpointed for the next attempt");
    let partial: u64 = cut.load(Ordering::Relaxed);
    assert!(partial > 0 && partial < SIZE as u64, "some but not all arrived: {partial}");

    // 2) an honest server, same url path → resume. It must not re-send the bytes we already wrote.
    let served2 = Arc::new(AtomicU64::new(0));
    let (url2, _s2) = serve(served2.clone(), None);
    // the state file remembers the *old* url; a different one is treated as a fresh download, so use the
    // same trick the harness does and keep the url stable by rewriting the checkpoint
    let mut st: serde_json::Value = serde_json::from_slice(&std::fs::read(&state).unwrap()).unwrap();
    st["url"] = serde_json::Value::String(url2.clone());
    std::fs::write(&state, serde_json::to_vec(&st).unwrap()).unwrap();

    let got = harness::tools::download::fetch_resumable(&http, &url2, &dest, 4, Duration::from_secs(60), None).await.expect("resume");
    assert_eq!(got, SIZE as u64);
    assert_eq!(std::fs::read(&dest).unwrap(), body(), "resumed file is byte-correct");
    let refetched = served2.load(Ordering::Relaxed);
    assert!(refetched < SIZE as u64, "resume re-sent {refetched} of {SIZE} — it should skip what was already written");
    assert!(!state.exists(), "checkpoint cleared once complete");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_complete_file_is_left_alone() {
    let served = Arc::new(AtomicU64::new(0));
    let (url, _srv) = serve(served.clone(), None);
    let dir = std::env::temp_dir().join(format!("harness-dl-done-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let dest = dir.join("blob.bin");
    std::fs::write(&dest, body()).unwrap();

    let http = reqwest::Client::new();
    let before = std::fs::metadata(&dest).unwrap().modified().unwrap();
    let got = harness::tools::download::fetch_resumable(&http, &url, &dest, 4, Duration::from_secs(30), None).await.expect("no-op");
    assert_eq!(got, SIZE as u64);
    // The claim is "it did not re-download", not an exact byte count: only the ranged probe should have
    // been served, which is a byte or so, never the 8MB body.
    let served = served.load(Ordering::Relaxed);
    assert!(served <= 1, "served {served} bytes for a file that was already complete");
    assert_eq!(std::fs::read(&dest).unwrap(), body(), "the existing file was left intact");
    assert_eq!(std::fs::metadata(&dest).unwrap().modified().unwrap(), before, "the file was not rewritten");
    let _ = std::fs::remove_dir_all(&dir);
}
