//! run_code: let the model write a short program that drives the harness's own tools ("code mode").
//! One `run_code` call replaces dozens of tool round-trips — filtering hundreds of files, aggregating
//! results, retrying, batching edits — and the loop stays in the program instead of the context window.
//! The script gets a `tool(name, **args)` helper that shells out to `harness tool`, so everything the
//! agent can do by hand it can also do in a loop, under the same permission policy.

use super::{arg_str, Tool, ToolCtx, ToolOutput};
use anyhow::{bail, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::Duration;

pub struct RunCode;

const PY_PREAMBLE: &str = r#"import json as _json, os as _os, subprocess as _sp
_HB = _os.environ.get("HARNESS_BIN", "harness")
_WD = _os.environ.get("HARNESS_WORKDIR", ".")
def tool(name, **args):
    """Call a harness tool by name; returns its text output (raises on failure)."""
    p = _sp.run([_HB, "tool", "-C", _WD, name, _json.dumps(args)], capture_output=True, text=True)
    out = (p.stdout or "") + (p.stderr or "" if p.returncode != 0 else "")
    if p.returncode != 0 and not p.stdout: raise RuntimeError(out.strip())
    return out
def read(path): return tool("read_file", path=path)
def write(path, content): return tool("write_file", path=path, content=content)
def sh(cmd): return tool("bash", cmd=cmd)
"#;

const JS_PREAMBLE: &str = r#"const {execFileSync} = require("child_process");
const HB = process.env.HARNESS_BIN || "harness";
const WD = process.env.HARNESS_WORKDIR || ".";
function tool(name, args = {}) {
  return execFileSync(HB, ["tool", "-C", WD, name, JSON.stringify(args)], {encoding: "utf8", maxBuffer: 64 * 1024 * 1024});
}
const read = (path) => tool("read_file", {path});
const write = (path, content) => tool("write_file", {path, content});
const sh = (cmd) => tool("bash", {cmd});
"#;

#[async_trait]
impl Tool for RunCode {
    fn name(&self) -> &'static str { "run_code" }
    fn description(&self) -> &'static str {
        "Run a short Python or JavaScript program that can call harness tools in a loop: `tool(name, **args)` (Python) / `tool(name, args)` (JS), plus read/write/sh shortcuts. Use it instead of many separate tool calls when you need to iterate, filter, aggregate or batch — e.g. inspect 200 files and report the 5 that match, or apply the same edit across a directory. Print what you want to see; stdout comes back to you."
    }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{
            "code":{"type":"string","description":"the program; print results to stdout"},
            "language":{"type":"string","enum":["python","javascript"],"description":"default python"},
            "timeout_secs":{"type":"integer","description":"default: the tool timeout"}
        },"required":["code"]})
    }
    async fn call(&self, args: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let ctx = ctx.effective();
        let code = arg_str(&args, "code")?;
        let lang = args.get("language").and_then(|v| v.as_str()).unwrap_or("python").to_lowercase();
        let timeout = Duration::from_secs(args.get("timeout_secs").and_then(|v| v.as_u64()).unwrap_or(ctx.timeout.as_secs()));
        let (ext, preamble, runner) = match lang.as_str() {
            "python" | "py" | "python3" => ("py", PY_PREAMBLE, "python3"),
            "javascript" | "js" | "node" => ("js", JS_PREAMBLE, "node"),
            other => bail!("unsupported language '{other}' (python | javascript)"),
        };
        if crate::setup::which_in(runner, &ctx.workdir).is_none() { bail!("{runner} is not installed — run_code needs it for language {lang}"); }
        let dir = ctx.workdir.join(".harness").join("tmp");
        std::fs::create_dir_all(&dir)?;
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let file = dir.join(format!("run_code-{stamp}.{ext}"));
        std::fs::write(&file, format!("{preamble}\n# ── model code ──\n{code}\n"))?;
        let exe = std::env::var_os("HARNESS_ORIG_EXE").map(std::path::PathBuf::from).or_else(|| std::env::current_exe().ok()).unwrap_or_else(|| "harness".into());
        let cmd = format!(
            "HARNESS_BIN={} HARNESS_WORKDIR={} {} {}",
            crate::security::shell_quote(&exe.display().to_string()),
            crate::security::shell_quote(&ctx.workdir.display().to_string()),
            runner,
            crate::security::shell_quote(&file.display().to_string()),
        );
        let out = crate::sandbox::run_shell(&cmd, &ctx.workdir, timeout, ctx.max_output).await?;
        let _ = std::fs::remove_file(&file);
        let mut text = String::new();
        if !out.stdout.trim().is_empty() { text.push_str(out.stdout.trim_end()); }
        if !out.stderr.trim().is_empty() { text.push_str(&format!("\n[stderr]\n{}", out.stderr.trim_end())); }
        if !out.success() { text.push_str(&format!("\n[exit code {}]", out.code.unwrap_or(-1))); }
        if text.trim().is_empty() { text = "(the program produced no output — print what you want to see)".into(); }
        Ok(text.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn runs_python_and_calls_tools() {
        let d = std::env::temp_dir().join(format!("harness-runcode-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("a.txt"), "hello\n").unwrap();
        let ctx = ToolCtx { timeout: Duration::from_secs(30), ..ToolCtx::basic(d.canonicalize().unwrap()) };
        let out = RunCode.call(json!({"code":"print('sum', sum(range(5)))"}), &ctx).await.unwrap().text;
        assert!(out.contains("sum 10"), "{out}");
        let out = RunCode.call(json!({"code":"import glob; print(len(glob.glob('*.txt')), 'txt files')"}), &ctx).await.unwrap().text;
        assert!(out.contains("1 txt files"), "{out}");
        let err = RunCode.call(json!({"code":"raise SystemExit(3)"}), &ctx).await.unwrap().text;
        assert!(err.contains("exit code 3"), "{err}");
        assert!(RunCode.call(json!({"code":"x","language":"cobol"}), &ctx).await.is_err());
        let _ = std::fs::remove_dir_all(&d);
    }
}
