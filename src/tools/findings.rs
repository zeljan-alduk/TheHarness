//! report_findings: structured code-review findings (Claude Code `ReportFindings` parity).
//!
//! The model calls it once at the end of a review/audit instead of writing a wall of text. Entries are
//! validated, saved as JSON under `<workdir>/.harness/findings/<timestamp>.json` (and `latest.json`,
//! which the UI may read), and echoed back as a compact list sorted by severity.

use super::{Tool, ToolCtx, ToolOutput};
use anyhow::{bail, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

const SEVERITIES: [&str; 5] = ["critical", "high", "medium", "low", "info"];
fn sev_rank(s: &str) -> usize { SEVERITIES.iter().position(|x| *x == s).unwrap_or(2) }

pub struct ReportFindings;

fn opt_int(v: &Value, key: &str, idx: usize) -> Result<Option<u64>> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => n.as_u64().map(Some).ok_or_else(|| anyhow::anyhow!("finding #{idx}: '{key}' must be a non-negative integer")),
        Some(_) => bail!("finding #{idx}: '{key}' must be an integer"),
    }
}
fn opt_str(v: &Value, key: &str, idx: usize) -> Result<Option<String>> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => bail!("finding #{idx}: '{key}' must be a string"),
    }
}

#[async_trait]
impl Tool for ReportFindings {
    fn name(&self) -> &'static str { "report_findings" }
    fn description(&self) -> &'static str { "Report structured code-review findings. Use it at the end of a review / audit / bug hunt instead of a wall of prose: one entry per issue with file, line, severity (critical|high|medium|low|info), a short title, a summary, and optionally the failure scenario and a suggested fix. Findings are validated, saved to .harness/findings/<timestamp>.json (and latest.json) and echoed back sorted by severity." }
    fn parameters(&self) -> Value { json!({"type":"object","properties":{
        "findings":{"type":"array","description":"one entry per issue","items":{"type":"object","properties":{
            "file":{"type":"string","description":"path relative to the workdir"},
            "line":{"type":"integer"},"end_line":{"type":"integer"},
            "severity":{"type":"string","enum":SEVERITIES,"description":"default medium"},
            "title":{"type":"string","description":"short one-line title"},
            "summary":{"type":"string","description":"what is wrong and why it matters"},
            "failure_scenario":{"type":"string","description":"how it breaks in practice"},
            "suggestion":{"type":"string","description":"how to fix it"}},
            "required":["file","title","summary"]}},
        "summary":{"type":"string","description":"overall verdict / one-paragraph summary of the review"}},
        "required":["findings"]}) }
    fn read_only(&self) -> bool { false }
    fn parallel_safe(&self) -> bool { false }

    async fn call(&self, args: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let Some(list) = args.get("findings").and_then(|v| v.as_array()) else { bail!("'findings' must be an array of {{file, line?, end_line?, severity?, title, summary, failure_scenario?, suggestion?}}") };
        let overall = match args.get("summary") { None | Some(Value::Null) => None, Some(Value::String(s)) => Some(s.clone()), Some(_) => bail!("'summary' must be a string") };
        let mut items: Vec<Value> = Vec::with_capacity(list.len());
        let mut missing_files: Vec<String> = vec![];
        for (i, f) in list.iter().enumerate() {
            let idx = i + 1;
            if !f.is_object() { bail!("finding #{idx}: must be an object"); }
            let file = match f.get("file") { Some(Value::String(s)) if !s.trim().is_empty() => s.trim().to_string(), _ => bail!("finding #{idx}: 'file' must be a non-empty string") };
            let title = match f.get("title") { Some(Value::String(s)) if !s.trim().is_empty() => s.trim().to_string(), _ => bail!("finding #{idx}: 'title' must be a non-empty string") };
            let summary = match f.get("summary") { Some(Value::String(s)) => s.trim().to_string(), None | Some(Value::Null) => String::new(), _ => bail!("finding #{idx}: 'summary' must be a string") };
            let severity = match f.get("severity") {
                None | Some(Value::Null) => "medium".to_string(),
                Some(Value::String(s)) => { let s = s.trim().to_lowercase(); if !SEVERITIES.contains(&s.as_str()) { bail!("finding #{idx}: invalid severity '{s}' (expected one of {})", SEVERITIES.join("|")); } s }
                Some(_) => bail!("finding #{idx}: 'severity' must be a string"),
            };
            let line = opt_int(f, "line", idx)?;
            let end_line = opt_int(f, "end_line", idx)?;
            let failure_scenario = opt_str(f, "failure_scenario", idx)?;
            let suggestion = opt_str(f, "suggestion", idx)?;
            // Referenced files need not exist, but note it (path resolution is lenient: only inside-workdir paths are checked).
            let exists = ctx.resolve(&file).map(|p| p.exists()).unwrap_or(false) || ctx.extra_roots.iter().any(|r| r.join(&file).exists());
            if !exists && !missing_files.contains(&file) { missing_files.push(file.clone()); }
            let mut o = serde_json::Map::new();
            o.insert("file".into(), json!(file));
            if let Some(l) = line { o.insert("line".into(), json!(l)); }
            if let Some(l) = end_line { o.insert("end_line".into(), json!(l)); }
            o.insert("severity".into(), json!(severity));
            o.insert("title".into(), json!(title));
            o.insert("summary".into(), json!(summary));
            if let Some(s) = failure_scenario { o.insert("failure_scenario".into(), json!(s)); }
            if let Some(s) = suggestion { o.insert("suggestion".into(), json!(s)); }
            if !exists { o.insert("file_exists".into(), json!(false)); }
            items.push(Value::Object(o));
        }
        // stable sort by severity, then file, then line
        items.sort_by(|a, b| {
            let ra = sev_rank(a["severity"].as_str().unwrap_or("medium"));
            let rb = sev_rank(b["severity"].as_str().unwrap_or("medium"));
            ra.cmp(&rb)
                .then_with(|| a["file"].as_str().cmp(&b["file"].as_str()))
                .then_with(|| a.get("line").and_then(|v| v.as_u64()).cmp(&b.get("line").and_then(|v| v.as_u64())))
        });
        let mut totals = serde_json::Map::new();
        for s in SEVERITIES { let n = items.iter().filter(|f| f["severity"] == s).count(); if n > 0 { totals.insert(s.into(), json!(n)); } }

        // save
        let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let dir = ctx.resolve(".harness/findings")?;
        std::fs::create_dir_all(&dir)?;
        let doc = json!({"timestamp": ts, "workdir": ctx.workdir.display().to_string(), "summary": overall, "totals": Value::Object(totals.clone()), "findings": items});
        let text = serde_json::to_string_pretty(&doc)?;
        let mut path = dir.join(format!("{ts}.json"));
        let mut n = 1;
        while path.exists() { path = dir.join(format!("{ts}-{n}.json")); n += 1; }
        std::fs::write(&path, &text)?;
        std::fs::write(dir.join("latest.json"), &text)?;

        // render
        let mut out = String::new();
        if items.is_empty() { out.push_str("no findings reported\n"); }
        for f in &items {
            let sev = f["severity"].as_str().unwrap_or("medium").to_uppercase();
            let mut loc = f["file"].as_str().unwrap_or("").to_string();
            if let Some(l) = f.get("line").and_then(|v| v.as_u64()) { loc.push_str(&format!(":{l}")); if let Some(e) = f.get("end_line").and_then(|v| v.as_u64()) { if e > l { loc.push_str(&format!("-{e}")); } } }
            out.push_str(&format!("[{sev}] {loc} — {}\n", f["title"].as_str().unwrap_or("")));
            let s = f["summary"].as_str().unwrap_or("");
            if !s.is_empty() { out.push_str(&format!("    {}\n", s.replace('\n', "\n    "))); }
        }
        if let Some(s) = &overall { if !s.trim().is_empty() { out.push_str(&format!("\nsummary: {}\n", s.trim())); } }
        let tot: Vec<String> = SEVERITIES.iter().filter_map(|s| totals.get(*s).map(|n| format!("{s}={n}"))).collect();
        out.push_str(&format!("\n{} finding(s){}\n", items.len(), if tot.is_empty() { String::new() } else { format!(" ({})", tot.join(", ")) }));
        if !missing_files.is_empty() { out.push_str(&format!("note: referenced file(s) not found in workdir: {}\n", missing_files.join(", "))); }
        let rel = path.strip_prefix(&ctx.workdir).map(|p| p.display().to_string()).unwrap_or_else(|_| path.display().to_string());
        out.push_str(&format!("saved: {rel} (also .harness/findings/latest.json)"));
        Ok(out.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ctx(tag: &str) -> ToolCtx {
        let d = std::env::temp_dir().join(format!("harness-findings-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        ToolCtx::basic(d)
    }

    #[tokio::test]
    async fn validates_writes_and_sorts() {
        let c = ctx("ok");
        std::fs::write(c.workdir.join("a.rs"), "fn main(){}\n").unwrap();
        let args = json!({"findings":[
            {"file":"a.rs","line":3,"severity":"low","title":"nit","summary":"minor"},
            {"file":"missing.rs","line":10,"end_line":12,"severity":"critical","title":"boom","summary":"crashes","suggestion":"fix"},
            {"file":"a.rs","title":"default sev","summary":"no severity given"}
        ],"summary":"overall ok"});
        let out = ReportFindings.call(args, &c).await.unwrap().text;
        let crit = out.find("[CRITICAL] missing.rs:10-12 — boom").expect("critical rendered");
        let med = out.find("[MEDIUM] a.rs — default sev").expect("medium rendered");
        let low = out.find("[LOW] a.rs:3 — nit").expect("low rendered");
        assert!(crit < med && med < low, "sorted by severity: {out}");
        assert!(out.contains("    crashes"));
        assert!(out.contains("critical=1, medium=1, low=1"));
        assert!(out.contains("not found in workdir: missing.rs"));
        assert!(out.contains("summary: overall ok"));
        let latest = c.workdir.join(".harness/findings/latest.json");
        let doc: Value = serde_json::from_str(&std::fs::read_to_string(&latest).unwrap()).unwrap();
        let fs = doc["findings"].as_array().unwrap();
        assert_eq!(fs.len(), 3);
        assert_eq!(fs[0]["severity"], "critical");
        assert_eq!(fs[0]["file_exists"], false);
        assert_eq!(fs[1]["severity"], "medium");
        assert!(fs[1].get("file_exists").is_none());
        assert_eq!(doc["totals"]["critical"], 1);
        assert_eq!(doc["summary"], "overall ok");
        // timestamped file also written
        let n = std::fs::read_dir(c.workdir.join(".harness/findings")).unwrap().count();
        assert_eq!(n, 2);
        let _ = std::fs::remove_dir_all(&c.workdir);
    }

    #[tokio::test]
    async fn rejects_invalid() {
        let c = ctx("bad");
        let e = ReportFindings.call(json!({}), &c).await.unwrap_err().to_string();
        assert!(e.contains("'findings' must be an array"), "{e}");
        let e = ReportFindings.call(json!({"findings":[{"file":"a.rs","title":"t","summary":"s","severity":"urgent"}]}), &c).await.unwrap_err().to_string();
        assert!(e.contains("invalid severity 'urgent'"), "{e}");
        let e = ReportFindings.call(json!({"findings":[{"title":"t","summary":"s"}]}), &c).await.unwrap_err().to_string();
        assert!(e.contains("'file' must be a non-empty string"), "{e}");
        let e = ReportFindings.call(json!({"findings":[{"file":"a.rs","title":"t","summary":"s","line":"12"}]}), &c).await.unwrap_err().to_string();
        assert!(e.contains("'line' must be an integer"), "{e}");
        assert!(!c.workdir.join(".harness/findings").exists(), "nothing written on error");
        let _ = std::fs::remove_dir_all(&c.workdir);
    }
}
