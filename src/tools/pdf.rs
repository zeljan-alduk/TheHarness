//! pdf_edit: edit text in existing PDFs in place (find / replace / add_text / render / info) without
//! disturbing the rest of the page. Runs an embedded PyMuPDF script (src/tools/pdf_edit.py) under
//! `python3` (if pymupdf is importable) or `uv run --with pymupdf`; args travel via a JSON temp file.

use super::{arg_str, Tool, ToolCtx, ToolOutput};
use crate::sandbox;
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;

pub struct PdfEdit;

const SCRIPT: &str = include_str!("pdf_edit.py");

/// The script on disk (temp dir, content-addressed so upgrades never reuse a stale copy).
fn script_path() -> Result<PathBuf> {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    SCRIPT.hash(&mut h);
    let p = std::env::temp_dir().join(format!("harness-pdf_edit-{:016x}.py", h.finish()));
    if !p.exists() { std::fs::write(&p, SCRIPT).with_context(|| format!("writing {}", p.display()))?; }
    Ok(p)
}

/// How to run Python with PyMuPDF: plain python3 if the module is installed, else via uv.
async fn python_cmd(ctx: &ToolCtx) -> Result<&'static str> {
    async fn probe(ctx: &ToolCtx, cmd: &str) -> bool { sandbox::run_shell(cmd, &ctx.workdir, ctx.timeout, 64).await.map(|o| o.success()).unwrap_or(false) }
    if probe(ctx, "python3 -c 'import pymupdf' >/dev/null 2>&1").await { return Ok("python3"); }
    if probe(ctx, "command -v uv >/dev/null 2>&1").await { return Ok("uv run --quiet --with pymupdf python"); }
    bail!("pdf_edit needs PyMuPDF: install `uv` (brew install uv; the tool then fetches pymupdf automatically) or `pip install pymupdf`");
}

fn sh_quote(s: &str) -> String { format!("'{}'", s.replace('\'', "'\\''")) }

#[async_trait]
impl Tool for PdfEdit {
    fn name(&self) -> &'static str { "pdf_edit" }
    fn description(&self) -> &'static str {
        "Edit an existing PDF in place without disturbing the rest of the page. Actions: \
         info (pages, fonts, metadata) · find {text} (occurrences with page/position/font) · \
         replace {old,new} (removes the old text and writes the new one at the same spot, matching font size/style/color; \
         new='' deletes; occurrence=N picks one match, else all; shrinks to fit unless there is room to the right) · \
         add_text {page,x,y,text} (baseline point in PDF points from top-left) · render {page} (returns a PNG of the page — use it to verify edits). \
         Edits overwrite the file (a .bak.pdf copy is kept unless backup=false) or go to `output`. Text is matched per line as extracted."
    }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{
            "action":{"type":"string","enum":["info","find","replace","add_text","render"]},
            "path":{"type":"string","description":"the PDF file"},
            "text":{"type":"string","description":"find: text to search for; add_text: text to insert (\\n = new line)"},
            "old":{"type":"string","description":"replace: existing text (must lie on one line)"},
            "new":{"type":"string","description":"replace: replacement text ('' removes the old text)"},
            "page":{"type":"integer","description":"1-based page (find/replace: restrict to this page; add_text/render: the page, default 1)"},
            "occurrence":{"type":"integer","description":"replace: only the N-th match (1-based); default: all matches"},
            "output":{"type":"string","description":"write the result here instead of overwriting the input (render: save the PNG here)"},
            "backup":{"type":"boolean","description":"keep <name>.bak.pdf when overwriting (default true)"},
            "font_size":{"type":"number","description":"override the font size (default: same as the replaced text / 11 for add_text)"},
            "color":{"type":"string","description":"text color '#rrggbb' (default: same as the replaced text / black)"},
            "bold":{"type":"boolean"},"italic":{"type":"boolean"},"mono":{"type":"boolean"},"serif":{"type":"boolean"},
            "fit":{"type":"boolean","description":"replace: shrink longer text to the original width (default true; false = keep size, may overlap)"},
            "align":{"type":"string","enum":["left","center","right"],"description":"replace: where to place shorter text within the old width (default left)"},
            "fill":{"type":"string","description":"replace: paint the old text area with this '#rrggbb' color (default: transparent, background untouched)"},
            "x":{"type":"number"},"y":{"type":"number"},
            "dpi":{"type":"integer","description":"render resolution (default 110)"},
            "clip":{"type":"array","items":{"type":"number"},"description":"render: [x0,y0,x1,y1] region in points"},
            "password":{"type":"string"}
        },"required":["action","path"]})
    }
    fn read_only(&self) -> bool { false }
    async fn call(&self, args: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let path = ctx.resolve(arg_str(&args, "path")?)?;
        if !path.is_file() { bail!("not a file: {}", path.display()); }
        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("find").to_string();
        let mut a = args.clone();
        a["path"] = json!(path.display().to_string());
        if let Some(o) = args.get("output").and_then(|v| v.as_str()).filter(|s| !s.trim().is_empty()) {
            a["output"] = json!(ctx.resolve(o)?.display().to_string());
        }
        let py = python_cmd(ctx).await?;
        let script = script_path()?;
        let args_file = std::env::temp_dir().join(format!("harness-pdf_edit-args-{}-{}.json", std::process::id(), rand_suffix()));
        std::fs::write(&args_file, serde_json::to_vec(&a)?)?;
        let cmd = format!("{py} {} {}", sh_quote(&script.display().to_string()), sh_quote(&args_file.display().to_string()));
        // long timeout for the first uv run (wheel download); output cap generous for the PNG
        let out = sandbox::run_shell(&cmd, &ctx.workdir, ctx.timeout.max(std::time::Duration::from_secs(300)), 12_000_000)
            .await.with_context(|| format!("running pdf_edit {action} on {}", path.display()));
        let _ = std::fs::remove_file(&args_file);
        let out = out?;
        if out.timed_out { bail!("pdf_edit timed out on {}", path.display()); }
        let stdout = out.stdout.trim();
        let v: Value = match serde_json::from_str(stdout.lines().last().unwrap_or("")) {
            Ok(v) => v,
            Err(_) => bail!("pdf_edit failed ({}): {}{}", out.code.map(|c| c.to_string()).unwrap_or("?".into()), out.stderr.trim(), if stdout.is_empty() { String::new() } else { format!("\n{stdout}") }),
        };
        if !v["ok"].as_bool().unwrap_or(false) { bail!("{}", v["error"].as_str().unwrap_or("pdf_edit failed")); }
        let mut res = ToolOutput::from(sandbox::truncate_middle(v["text"].as_str().unwrap_or(""), ctx.max_output));
        if let (Some(mime), Some(b64)) = (v["image"]["mime"].as_str(), v["image"]["b64"].as_str()) {
            res.images.push((mime.to_string(), b64.to_string()));
            res.text.push_str("\n(the rendered page image is attached)");
        }
        Ok(res)
    }
}

fn rand_suffix() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn script_embedded_and_written() {
        assert!(SCRIPT.contains("def act_replace"));
        let p = script_path().unwrap();
        assert!(p.exists());
    }
    #[test]
    fn quoting() { assert_eq!(sh_quote("a'b"), "'a'\\''b'"); }
}
