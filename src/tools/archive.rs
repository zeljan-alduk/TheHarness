//! Document & archive tools: read_pdf (pdftotext) and extract_archive (unzip/tar/gunzip/7z).
//! Both shell out via crate::sandbox::run_shell and resolve every path through ctx.resolve()
//! so nothing can escape the workdir.

use super::{arg_str, Tool, ToolCtx, ToolOutput};
use crate::sandbox::{self, shq};
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct ReadPdf;
pub struct ExtractArchive;

/// Is `cmd` on the PATH? Probed once per process (run_shell scrubs secret env vars but keeps PATH).
pub async fn has_cmd(ctx: &ToolCtx, cmd: &str) -> bool {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, bool>>> = std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(&v) = cache.lock().unwrap().get(cmd) { return v; }
    let v = sandbox::run_shell(&format!("command -v {} >/dev/null 2>&1", shq(cmd)), &ctx.workdir, ctx.timeout, 64)
        .await
        .map(|o| o.success())
        .unwrap_or(false);
    if v { cache.lock().unwrap().insert(cmd.to_string(), true); } // only cache hits: a missing tool may get installed mid-session
    v
}

/// Which CLI extracts this file? Returns the command prefix (no input/output args).
fn archive_cmd(ext: &str) -> Result<&'static str> {
    match ext.to_ascii_lowercase().as_str() {
        "zip" => Ok("unzip"),
        "tar" => Ok("tar -xf"),
        "gz" | "tgz" => Ok("tar -xzf"),
        "7z" => Ok("7z x -y"),
        "rar" => Ok("7z x -y"),
        other => bail!("unsupported archive type '.{other}' (supported: .zip, .tar, .tar.gz/.tgz, .gz, .7z, .rar)"),
    }
}

/// Directory named after the archive with all archive extensions stripped: data.zip -> data, a.tar.gz -> a.
fn default_dest(file_name: &str) -> String {
    let mut s = file_name.to_string();
    for ext in [".tar.gz", ".tgz", ".zip", ".tar", ".gz", ".7z", ".rar"] {
        if let Some(stripped) = s.strip_suffix(ext) {
            s = stripped.to_string();
        }
    }
    if s.is_empty() { "extracted".into() } else { s }
}

#[async_trait]
impl Tool for ReadPdf {
    fn read_only(&self) -> bool { true }
    fn name(&self) -> &'static str { "read_pdf" }
    fn description(&self) -> &'static str {
        "Extract text from a PDF file in the working directory (via pdftotext -layout). Use max_pages to read only the first N pages of a long document."
    }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{
            "path":{"type":"string"},
            "max_pages":{"type":"integer","description":"only extract the first N pages (default: all)"}
        },"required":["path"]})
    }
    async fn call(&self, args: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let path = ctx.resolve(arg_str(&args, "path")?)?;
        if !path.is_file() { bail!("not a file: {}", path.display()); }
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !ext.eq_ignore_ascii_case("pdf") { bail!("not a PDF (expected .pdf): {}", path.display()); }
        if !has_cmd(ctx, "pdftotext").await {
            bail!("pdftotext is not installed (poppler-utils); cannot read PDFs. Try `bash` with another extractor if available.");
        }
        let max_pages = args.get("max_pages").and_then(|v| v.as_u64()).filter(|&n| n > 0);
        let range = max_pages.map(|n| format!(" -f 1 -l {n}")).unwrap_or_default();
        let out = sandbox::run_shell(
            &format!("pdftotext -layout {} {} -", range, shq(&path.display().to_string())),
            &ctx.workdir, ctx.timeout, ctx.max_output * 4,
        ).await.with_context(|| format!("running pdftotext on {}", path.display()))?;
        if out.timed_out { bail!("pdftotext timed out after {}s on {}", ctx.timeout.as_secs(), path.display()); }
        if !out.success() { bail!("pdftotext failed on {}: {}", path.display(), out.stderr.trim()); }
        let text = out.stdout;
        if text.trim().is_empty() {
            return Ok(format!("(no extractable text in {} — it may be a scanned/image PDF; try OCR via bash)", path.display()).into());
        }
        let pages = max_pages.map(|n| format!(", first {n} page(s)")).unwrap_or_default();
        let out = format!("{}{pages}, {} chars\n{text}", path.display(), text.trim_end().chars().count());
        Ok((sandbox::truncate_middle(&out, ctx.max_output)).into())
    }
}

#[async_trait]
impl Tool for ExtractArchive {
    fn name(&self) -> &'static str { "extract_archive" }
    fn description(&self) -> &'static str {
        "Extract a .zip, .tar, .tar.gz/.tgz, .gz, .7z (or .rar if 7z is installed) archive into dest (default: a directory next to the archive named after it). Returns the extracted top-level entries."
    }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{
            "path":{"type":"string"},
            "dest":{"type":"string","description":"target directory (default: <archive basename> next to the archive)"}
        },"required":["path"]})
    }
    async fn call(&self, args: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let path = ctx.resolve(arg_str(&args, "path")?)?;
        if !path.is_file() { bail!("not a file: {}", path.display()); }
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let cmd_prefix = archive_cmd(ext)?;

        // .rar needs 7z; fail early with a clear explanation if it is missing.
        let tool = cmd_prefix.split_whitespace().next().unwrap_or("");
        if !has_cmd(ctx, tool).await {
            bail!("{tool} is not installed; cannot extract .{ext}. Install it (e.g. `brew install p7zip` for 7z) or use bash with another extractor.");
        }

        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("archive");
        let dest = match args.get("dest").and_then(|v| v.as_str()).filter(|s| !s.trim().is_empty()) {
            Some(d) => ctx.resolve(d)?,
            None => path.parent().unwrap_or(&ctx.workdir).join(default_dest(file_name)), // next to the archive
        };

        // .gz is a single-file format: gunzip -k decompresses in place next to the archive.
        // (A .tar.gz/.tgz is a tarball, not a single file — it goes through the tar path below.)
        let is_tarball = ext.eq_ignore_ascii_case("tgz") || file_name.to_lowercase().ends_with(".tar.gz");
        if ext.eq_ignore_ascii_case("gz") && !is_tarball {
            let out_name = path.with_extension(""); // foo.gz -> foo
            if out_name.exists() { bail!("refusing to overwrite existing {}", out_name.display()); }
            let o = sandbox::run_shell(&format!("gunzip -k {}", shq(&path.display().to_string())), &ctx.workdir, ctx.timeout, 4096)
                .await.with_context(|| format!("gunzip {}", path.display()))?;
            if !o.success() { bail!("gunzip failed: {}", o.stderr.trim()); }
            return Ok(format!("decompressed {} -> {}\nentries:\n{}", path.display(), out_name.display(),
                out_name.file_name().and_then(|n| n.to_str()).unwrap_or("?")).into());
        }

        tokio::fs::create_dir_all(&dest).await.with_context(|| format!("creating {}", dest.display()))?;
        let (p, d) = (shq(&path.display().to_string()), shq(&dest.display().to_string()));
        let cmd = match ext.to_ascii_lowercase().as_str() {
            "zip" => format!("unzip -o {p} -d {d}"),
            "tar" => format!("tar -xf {p} -C {d}"),
            "gz" | "tgz" => format!("tar -xzf {p} -C {d}"),
            "7z" | "rar" => format!("7z x -y {p} -o{d}"),
            _ => unreachable!("checked by archive_cmd"),
        };
        let o = sandbox::run_shell(&cmd, &ctx.workdir, ctx.timeout, 8192)
            .await.with_context(|| format!("extracting {}", path.display()))?;
        if o.timed_out { bail!("extraction timed out after {}s", ctx.timeout.as_secs()); }
        if !o.success() { bail!("extraction failed for {}: {}", path.display(), o.stderr.trim()); }

        let l = sandbox::run_shell(
            &format!("find {d} -mindepth 1 -maxdepth 1 | LC_ALL=C sort"),
            &ctx.workdir, ctx.timeout, 8192,
        ).await.unwrap_or_else(|_| sandbox::ProcOutput { stdout: String::new(), stderr: "listing failed".into(), code: None, timed_out: false, elapsed: std::time::Duration::ZERO });
        let mut entries: Vec<String> = l.stdout.lines().map(|s| {
            let s = s.trim();
            if dest.join(s).is_dir() { format!("{s}/") } else { s.to_string() }
        }).filter(|s| !s.trim().is_empty()).collect();
        entries.sort();
        let mut s = format!("extracted {} -> {}\nentries ({}):\n", path.display(), dest.display(), entries.len());
        for e in entries.iter().take(200) { s.push_str(e); s.push('\n'); }
        if entries.len() > 200 { s.push_str(&format!("…[{} more]", entries.len() - 200)); }
        if !o.stderr.trim().is_empty() { s.push_str(&format!("\n[extractor notes]\n{}", o.stderr.trim())); }
        Ok((sandbox::truncate_middle(&s, ctx.max_output)).into())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    fn ctx() -> (ToolCtx, PathBuf) {
        let d = std::env::temp_dir().join(format!("harness-archive-test-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        (ToolCtx { timeout: std::time::Duration::from_secs(30), ..ToolCtx::basic(d.clone()) }, d)
    }

    #[test]
    fn default_dest_strips_extensions() {
        assert_eq!(default_dest("data.zip"), "data");
        assert_eq!(default_dest("a.tar.gz"), "a");
        assert_eq!(default_dest("x.tgz"), "x");
        assert_eq!(default_dest("y.tar"), "y");
        assert_eq!(default_dest("z.7z"), "z");
    }

    #[test]
    fn archive_cmd_dispatch() {
        assert_eq!(archive_cmd("zip").unwrap(), "unzip");
        assert_eq!(archive_cmd("tar").unwrap(), "tar -xf");
        assert_eq!(archive_cmd("gz").unwrap(), "tar -xzf");
        assert_eq!(archive_cmd("tgz").unwrap(), "tar -xzf");
        assert_eq!(archive_cmd("7z").unwrap(), "7z x -y");
        assert_eq!(archive_cmd("rar").unwrap(), "7z x -y");
        assert!(archive_cmd("txt").is_err());
    }

    #[tokio::test]
    async fn resolve_rejects_escape() {
        let (c, _) = ctx();
        assert!(c.resolve("../../etc/passwd").is_err());
    }

    #[tokio::test]
    async fn zip_roundtrip() {
        let (c, d) = ctx();
        std::fs::create_dir_all(d.join("src/pkg")).unwrap();
        std::fs::write(d.join("hello.txt"), "hi there\n").unwrap();
        std::fs::write(d.join("src/pkg/mod.rs"), "fn main() {}\n").unwrap();
        let o = sandbox::run_shell("zip -qr data.zip hello.txt src", &d, std::time::Duration::from_secs(30), 4096).await.unwrap();
        assert!(o.success(), "zip failed: {}", o.stderr);

        let out = ExtractArchive.call(json!({"path": "data.zip"}), &c).await.unwrap();
        assert!(out.text.contains("hello.txt"), "{}", out.text);
        assert!(out.text.contains("src/"), "{}", out.text);
        assert_eq!(std::fs::read_to_string(d.join("data/hello.txt")).unwrap(), "hi there\n");
        assert_eq!(std::fs::read_to_string(d.join("data/src/pkg/mod.rs")).unwrap(), "fn main() {}\n");
    }

    #[tokio::test]
    async fn tar_gz_roundtrip() {
        let (c, d) = ctx();
        std::fs::write(d.join("a.txt"), "alpha\n").unwrap();
        let o = sandbox::run_shell("tar -czf bundle.tar.gz a.txt", &d, std::time::Duration::from_secs(30), 4096).await.unwrap();
        assert!(o.success(), "tar failed: {}", o.stderr);

        let out = ExtractArchive.call(json!({"path": "bundle.tar.gz", "dest": "out"}), &c).await.unwrap();
        assert!(out.text.contains("a.txt"), "{}", out.text);
        assert_eq!(std::fs::read_to_string(d.join("out/a.txt")).unwrap(), "alpha\n");
    }

    #[tokio::test]
    async fn unsupported_type_errors() {
        let (c, d) = ctx();
        std::fs::write(d.join("notes.txt"), "x").unwrap();
        let out = ExtractArchive.call(json!({"path": "notes.txt"}), &c).await.unwrap_err().to_string();
        assert!(out.contains("unsupported"), "{}", out);
    }

    #[tokio::test]
    async fn pdf_roundtrip() {
        if !std::process::Command::new("command").arg("-v").arg("pdftotext").output().map(|o| o.status.success()).unwrap_or(false) {
            eprintln!("skipping pdf test: pdftotext not installed");
            return;
        }
        let (c, d) = ctx();
        // Minimal one-page PDF with the text "Hello from the PDF".
        let pdf = b"%PDF-1.4\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj\n3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 612 792]/Contents 4 0 R/Resources<</Font<</F1 5 0 R>>>>>>endobj\n4 0 obj<</Length 62>>stream\nBT /F1 24 Tf 72 700 Td (Hello from the PDF) Tj ET\nendstream\nendobj\n5 0 obj<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>endobj\ntrailer<</Root 1 0 R>>\n%%EOF\n";
        std::fs::write(d.join("doc.pdf"), pdf).unwrap();

        let out = ReadPdf.call(json!({"path": "doc.pdf"}), &c).await.unwrap();
        assert!(out.text.contains("Hello from the PDF"), "{}", out.text);

        let bad = ReadPdf.call(json!({"path": "../../etc/passwd"}), &c).await.unwrap_err().to_string();
        assert!(bad.contains("escapes workdir"), "{}", bad);
    }
}
