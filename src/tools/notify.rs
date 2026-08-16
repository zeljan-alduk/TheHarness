//! notify: send a desktop notification (macOS osascript, Linux notify-send, Windows toast).

use super::{arg_str, Tool, ToolCtx, ToolOutput};
use anyhow::{bail, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::Duration;

pub struct Notify;

const TIMEOUT: Duration = Duration::from_secs(10);

#[async_trait]
impl Tool for Notify {
    fn name(&self) -> &'static str { "notify" }
    fn description(&self) -> &'static str { "Send a desktop notification to the user (e.g. when a long task finishes or needs input). Best-effort; no-op when HARNESS_NO_NOTIFY is set." }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{
            "message":{"type":"string","description":"notification body"},
            "title":{"type":"string","description":"default 'harness'"},
            "subtitle":{"type":"string"},
            "sound":{"type":"boolean","description":"play the default sound (default false)"}
        },"required":["message"]})
    }
    fn parallel_safe(&self) -> bool { true }
    async fn call(&self, args: Value, _ctx: &ToolCtx) -> Result<ToolOutput> {
        let message = arg_str(&args, "message")?;
        let title = args.get("title").and_then(|v| v.as_str()).filter(|s| !s.trim().is_empty()).unwrap_or("harness");
        let subtitle = args.get("subtitle").and_then(|v| v.as_str()).filter(|s| !s.trim().is_empty());
        let sound = args.get("sound").and_then(|v| v.as_bool()).unwrap_or(false);
        if std::env::var_os("HARNESS_NO_NOTIFY").is_some() {
            return Ok(format!("notification suppressed (HARNESS_NO_NOTIFY set): {title}: {message}").into());
        }
        send(title, message, subtitle, sound).await?;
        Ok(format!("notification sent: {title}: {message}").into())
    }
}

/// Escape a string for use inside a double-quoted AppleScript literal.
/// Backslashes and quotes are escaped; newlines/CR/tabs are replaced by spaces
/// (osascript -e handles them poorly and notifications are single-line anyway).
pub fn applescript_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' | '\r' | '\t' => out.push(' '),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

fn applescript_command(title: &str, message: &str, subtitle: Option<&str>, sound: bool) -> String {
    let mut s = format!("display notification \"{}\" with title \"{}\"", applescript_escape(message), applescript_escape(title));
    if let Some(sub) = subtitle { s.push_str(&format!(" subtitle \"{}\"", applescript_escape(sub))); }
    if sound { s.push_str(" sound name \"default\""); }
    s
}

/// Escape for a single-quoted PowerShell literal.
fn ps_escape(s: &str) -> String { s.replace('\'', "''").replace(['\n', '\r'], " ") }

/// Escape for XML text content.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace(['\n', '\r'], " ")
}

async fn run(program: &str, args: &[String]) -> Result<()> {
    let child = tokio::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn();
    let child = match child {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => bail!("{program} not found"),
        Err(e) => bail!("failed to start {program}: {e}"),
    };
    match tokio::time::timeout(TIMEOUT, child.wait_with_output()).await {
        Ok(Ok(out)) if out.status.success() => Ok(()),
        Ok(Ok(out)) => bail!("{program} failed ({}): {}", out.status, String::from_utf8_lossy(&out.stderr).trim()),
        Ok(Err(e)) => bail!("{program} error: {e}"),
        Err(_) => bail!("{program} timed out after {}s", TIMEOUT.as_secs()),
    }
}

async fn send(title: &str, message: &str, subtitle: Option<&str>, sound: bool) -> Result<()> {
    if cfg!(target_os = "macos") {
        let script = applescript_command(title, message, subtitle, sound);
        run("osascript", &["-e".to_string(), script]).await
    } else if cfg!(target_os = "linux") {
        let body = match subtitle { Some(s) => format!("{s}\n{message}"), None => message.to_string() };
        let mut args = vec!["--app-name=harness".to_string()];
        if sound { args.push("--hint=string:sound-name:message-new-instant".to_string()); }
        args.push("--".to_string());
        args.push(title.to_string());
        args.push(body);
        run("notify-send", &args).await.map_err(|e| anyhow::anyhow!("notifications unavailable on this platform ({e})"))
    } else if cfg!(target_os = "windows") {
        let full = match subtitle { Some(s) => format!("{s} — {message}"), None => message.to_string() };
        let audio = if sound { "" } else { "<audio silent='true'/>" };
        let xml = format!(
            "<toast><visual><binding template='ToastGeneric'><text>{}</text><text>{}</text></binding></visual>{audio}</toast>",
            xml_escape(title), xml_escape(&full)
        );
        let script = format!(
            "if (Get-Command New-BurntToastNotification -ErrorAction SilentlyContinue) {{ New-BurntToastNotification -Text '{t}','{m}' {snd}; exit 0 }}; \
             [Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] | Out-Null; \
             [Windows.Data.Xml.Dom.XmlDocument, Windows.Data.Xml.Dom.XmlDocument, ContentType = WindowsRuntime] | Out-Null; \
             $x = New-Object Windows.Data.Xml.Dom.XmlDocument; $x.LoadXml('{xml}'); \
             $toast = New-Object Windows.UI.Notifications.ToastNotification $x; \
             [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('harness').Show($toast)",
            t = ps_escape(title), m = ps_escape(&full), snd = if sound { "" } else { "-Silent" }, xml = ps_escape(&xml)
        );
        let args = vec!["-NoProfile".to_string(), "-NonInteractive".to_string(), "-Command".to_string(), script];
        match run("powershell", &args).await {
            Ok(()) => Ok(()),
            Err(_) => run("pwsh", &args).await.map_err(|e| anyhow::anyhow!("notifications unavailable on this platform ({e})")),
        }
    } else {
        bail!("notifications unavailable on this platform")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_applescript() {
        let s = applescript_escape("say \"hi\" C:\\path\nnext\r\nline\ttab");
        assert_eq!(s, "say \\\"hi\\\" C:\\\\path next  line tab");
        let cmd = applescript_command("t\"", "m\\", Some("s"), true);
        assert_eq!(cmd, "display notification \"m\\\\\" with title \"t\\\"\" subtitle \"s\" sound name \"default\"");
        let cmd = applescript_command("harness", "done", None, false);
        assert_eq!(cmd, "display notification \"done\" with title \"harness\"");
    }

    #[tokio::test]
    async fn no_notify_env_skips_sending() {
        std::env::set_var("HARNESS_NO_NOTIFY", "1");
        let ctx = ToolCtx::basic(std::env::temp_dir());
        let out = Notify.call(json!({"message":"hello","title":"T"}), &ctx).await.unwrap();
        assert!(out.text.contains("suppressed"), "{}", out.text);
        assert!(out.text.contains("T: hello"));
        assert!(Notify.call(json!({}), &ctx).await.is_err());
        std::env::remove_var("HARNESS_NO_NOTIFY");
    }
}
