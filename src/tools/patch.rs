//! apply_patch: apply a unified diff to files in the workdir (git apply, with `patch` fallback).

use super::{arg_str, Tool, ToolCtx, ToolOutput};
use anyhow::{bail, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct ApplyPatch;

#[async_trait]
impl Tool for ApplyPatch {
    fn name(&self) -> &'static str { "apply_patch" }
    fn description(&self) -> &'static str { "Apply a unified diff (as produced by `diff -u` / `git diff`) to files under the working directory. Paths in the diff are relative to the workdir (a/ b/ prefixes are fine). Creates and deletes files as the diff specifies. Prefer edit_file for small single-spot changes; use this for multi-hunk or multi-file changes." }
    fn parameters(&self) -> Value { json!({"type":"object","properties":{"patch":{"type":"string","description":"the unified diff text"}},"required":["patch"]}) }
    async fn call(&self, args: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let patch = arg_str(&args, "patch")?;
        if !patch.contains("@@") && !patch.contains("+++") { bail!("this does not look like a unified diff (no @@ hunks / +++ headers)"); }
        // path safety: every target path must resolve inside the workdir
        for l in patch.lines() {
            if let Some(p) = l.strip_prefix("+++ ").or_else(|| l.strip_prefix("--- ")) {
                let p = p.split('\t').next().unwrap_or(p).trim();
                if p == "/dev/null" { continue; }
                let p = p.strip_prefix("a/").or_else(|| p.strip_prefix("b/")).unwrap_or(p);
                ctx.resolve(p)?;
            }
        }
        let mut text = patch.to_string(); if !text.ends_with('\n') { text.push('\n'); }
        // temp diff lives outside the workdir (never pollutes the tree / git status)
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let tmp = std::env::temp_dir().join(format!("harness-patch-{}-{}.diff", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
        tokio::fs::write(&tmp, &text).await?;
        let strip = if patch.lines().any(|l| l.starts_with("+++ b/") || l.starts_with("--- a/")) { 1 } else { 0 };
        // git apply is atomic; the `patch` fallback is not, so dry-run first and only apply when it is
        // clean (no partial application, no .rej/.orig files left behind).
        let t = crate::sandbox::shq(&tmp.display().to_string());
        let cmd = format!("git apply --whitespace=nowarn --recount -p{strip} {t} 2>&1 || {{ echo '--- git apply failed, trying patch ---'; patch --dry-run -p{strip} -N -s -r - < {t} 2>&1 && patch -p{strip} -N -s -r - --no-backup-if-mismatch < {t} 2>&1; }}");
        let out = crate::sandbox::run_shell(&cmd, &ctx.workdir, ctx.timeout, ctx.max_output).await;
        let _ = tokio::fs::remove_file(&tmp).await;
        let out = out?;
        let files: Vec<String> = patch.lines().filter_map(|l| l.strip_prefix("+++ ")).map(|p| p.split('\t').next().unwrap_or(p).trim().trim_start_matches("b/").to_string()).filter(|p| p != "/dev/null").collect();
        if out.success() { Ok(format!("patch applied to {} file(s): {}\n{}", files.len(), files.join(", "), out.stdout.trim()).into()) }
        else { bail!("patch failed:\n{}\n{}\nCheck that context lines match the current file contents (read_file first) or use edit_file.", out.stdout.trim(), out.stderr.trim()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static N: AtomicU64 = AtomicU64::new(0);
    fn ctx() -> (ToolCtx, PathBuf) {
        let d = std::env::temp_dir().join(format!("harness-patch-test-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        (ToolCtx { timeout: std::time::Duration::from_secs(30), ..ToolCtx::basic(d.clone()) }, d)
    }

    #[tokio::test]
    async fn applies_unified_diff() {
        let (c, d) = ctx();
        std::fs::write(d.join("app.txt"), "alpha\nbeta\ngamma\n").unwrap();
        let patch = "--- a/app.txt\n+++ b/app.txt\n@@ -1,3 +1,3 @@\n alpha\n-beta\n+BETA\n gamma\n";
        let out = ApplyPatch.call(json!({"patch": patch}), &c).await.unwrap();
        assert!(out.text.contains("app.txt"), "{}", out.text);
        let after = std::fs::read_to_string(d.join("app.txt")).unwrap();
        assert_eq!(after, "alpha\nBETA\ngamma\n", "file should be patched");
    }

    #[tokio::test]
    async fn mismatched_context_fails() {
        let (c, d) = ctx();
        std::fs::write(d.join("app.txt"), "alpha\nbeta\ngamma\n").unwrap();
        let patch = "--- a/app.txt\n+++ b/app.txt\n@@ -1,3 +1,3 @@\n alpha\n-DELTA\n+BETA\n gamma\n";
        let err = ApplyPatch.call(json!({"patch": patch}), &c).await.unwrap_err().to_string();
        assert!(err.contains("patch failed"), "{}", err);
        let after = std::fs::read_to_string(d.join("app.txt")).unwrap();
        assert_eq!(after, "alpha\nbeta\ngamma\n", "file must be unchanged");
    }

    #[tokio::test]
    async fn rejects_non_diff() {
        let (c, _) = ctx();
        let err = ApplyPatch.call(json!({"patch": "not a diff at all"}), &c).await.unwrap_err().to_string();
        assert!(err.contains("unified diff"), "{}", err);
    }

    #[tokio::test]
    async fn rejects_path_escape() {
        let (c, _) = ctx();
        let patch = "--- /etc/passwd\n+++ /etc/passwd\n@@ -1 +1 @@\n-x\n+y\n";
        let err = ApplyPatch.call(json!({"patch": patch}), &c).await.unwrap_err().to_string();
        assert!(err.contains("escapes workdir"), "{}", err);
    }
}
