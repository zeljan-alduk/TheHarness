//! diagnostics: run the project's fast static checks (compile/type/lint) and return errors compactly.
//! Detects Rust (cargo check), Python (py_compile / pyflakes / ruff if present), TypeScript (tsc),
//! JavaScript (node --check), Go (go vet). Use after edits, before claiming success.

use super::{Tool, ToolCtx, ToolOutput};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct Diagnostics;

#[async_trait]
impl Tool for Diagnostics {
    fn read_only(&self) -> bool { true }
    fn name(&self) -> &'static str { "diagnostics" }
    fn description(&self) -> &'static str { "Run the project's fast static checks (Rust: cargo check; Python: compile + pyflakes/ruff; TS: tsc --noEmit; JS: node --check; Go: go vet) and return errors/warnings compactly. Call after edits. Optionally restrict to a path." }
    fn parameters(&self) -> Value { json!({"type":"object","properties":{"path":{"type":"string","description":"file or dir (default: workdir)"}},"required":[]}) }
    async fn call(&self, args: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let target = ctx.resolve(args.get("path").and_then(|v| v.as_str()).unwrap_or("."))?;
        let wd = &ctx.workdir;
        let mut sections: Vec<String> = Vec::new();
        let run = |cmd: String| async move { crate::sandbox::run_shell(&cmd, wd, ctx.timeout, ctx.max_output).await };
        let has = |f: &str| wd.join(f).exists();
        let ext = target.extension().and_then(|e| e.to_str()).unwrap_or("").to_string();
        // Rust
        if has("Cargo.toml") && (target.is_dir() || ext == "rs") {
            let o = run("cargo check --message-format=short 2>&1 | grep -E '^(warning|error)|-->|^\\S+\\.rs:[0-9]+' | head -80".into()).await?;
            sections.push(format!("cargo check: {}", if o.stdout.trim().is_empty() { "ok (no warnings/errors)".to_string() } else { format!("\n{}", o.stdout.trim()) }));
        }
        // Python
        let py_target = if ext == "py" { target.display().to_string() } else { ".".into() };
        if ext == "py" || has("pyproject.toml") || has("setup.py") || has("requirements.txt") || (target.is_dir() && wd.join("__init__.py").exists()) {
            let o = run(format!("if command -v ruff >/dev/null; then ruff check --output-format concise {py_target} 2>&1 | head -60; elif python3 -c 'import pyflakes' 2>/dev/null; then python3 -m pyflakes {py_target} 2>&1 | head -60; else find {py_target} -name '*.py' -not -path '*/.venv/*' -not -path '*/node_modules/*' | head -200 | xargs -I{{}} python3 -m py_compile {{}} 2>&1 | head -40; fi")).await?;
            sections.push(format!("python: {}", if o.stdout.trim().is_empty() && o.stderr.trim().is_empty() { "ok".to_string() } else { format!("\n{}{}", o.stdout.trim(), o.stderr.trim()) }));
        }
        // TypeScript / JavaScript
        if has("tsconfig.json") && (target.is_dir() || ext == "ts" || ext == "tsx") {
            let o = run("npx --no-install tsc --noEmit --pretty false 2>&1 | head -60".into()).await?;
            sections.push(format!("tsc: {}", if o.success() && o.stdout.trim().is_empty() { "ok".to_string() } else { format!("\n{}", o.stdout.trim()) }));
        } else if ext == "js" || ext == "mjs" || ext == "cjs" {
            let o = run(format!("node --check '{}' 2>&1 | head -30", target.display())).await?;
            sections.push(format!("node --check: {}", if o.success() { "ok".to_string() } else { format!("\n{}{}", o.stdout.trim(), o.stderr.trim()) }));
        }
        // Go
        if has("go.mod") { let o = run("go vet ./... 2>&1 | head -60".into()).await?; sections.push(format!("go vet: {}", if o.success() && o.stdout.trim().is_empty() && o.stderr.trim().is_empty() { "ok".to_string() } else { format!("\n{}{}", o.stdout.trim(), o.stderr.trim()) })); }
        // Shell
        if ext == "sh" { let o = run(format!("sh -n '{}' 2>&1", target.display())).await?; sections.push(format!("sh -n: {}", if o.success() { "ok".to_string() } else { o.stderr.trim().to_string() })); }
        if sections.is_empty() { return Ok("no known project type detected (Rust/Python/TS/JS/Go/sh); run the project's own checker with bash".into()); }
        Ok(crate::sandbox::truncate_middle(&sections.join("\n\n"), ctx.max_output).into())
    }
}
