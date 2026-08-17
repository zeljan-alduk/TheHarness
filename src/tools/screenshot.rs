//! screenshot: hand the model a picture of the screen, a window, or a web page. macOS uses
//! `screencapture`, Linux tries gnome-screenshot / spectacle / import / grim, Windows uses PowerShell,
//! and a URL is rendered with headless Chrome when one is installed.

use super::{Tool, ToolCtx, ToolOutput};
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::Duration;

pub struct Screenshot;

/// The command that grabs the screen on this platform, if any.
pub fn capture_command(out: &str, window: bool) -> Option<String> {
    let q = crate::security::shell_quote(out);
    if cfg!(target_os = "macos") {
        return Some(if window { format!("screencapture -o -x -w {q}") } else { format!("screencapture -x {q}") });
    }
    if cfg!(target_os = "linux") {
        for (bin, cmd) in [
            ("gnome-screenshot", format!("gnome-screenshot {} -f {q}", if window { "-w" } else { "" })),
            ("spectacle", format!("spectacle -b -n {} -o {q}", if window { "-a" } else { "-f" })),
            ("grim", format!("grim {q}")),
            ("import", format!("import -window {} {q}", if window { "$(xdotool getactivewindow)" } else { "root" })),
            ("scrot", format!("scrot {} {q}", if window { "-u" } else { "" })),
        ] { if crate::setup::which(bin).is_some() { return Some(cmd); } }
        return None;
    }
    if cfg!(windows) {
        return Some(format!("powershell -NoProfile -Command \"Add-Type -AssemblyName System.Windows.Forms,System.Drawing; $b=[System.Windows.Forms.Screen]::PrimaryScreen.Bounds; $bmp=New-Object System.Drawing.Bitmap $b.Width,$b.Height; $g=[System.Drawing.Graphics]::FromImage($bmp); $g.CopyFromScreen($b.Location,[System.Drawing.Point]::Empty,$b.Size); $bmp.Save('{out}')\""));
    }
    None
}

/// A headless-Chrome command that renders `url` to a PNG, if a Chrome-like browser is installed.
pub fn page_command(url: &str, out: &str, width: u32, height: u32, full_page: bool) -> Option<String> {
    let candidates = ["google-chrome", "chromium", "chromium-browser", "brave-browser", "msedge"];
    let mac = ["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "/Applications/Chromium.app/Contents/MacOS/Chromium", "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser", "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge"];
    let bin = candidates.iter().find_map(|b| crate::setup::which(b).map(|p| p.display().to_string()))
        .or_else(|| mac.iter().find(|p| std::path::Path::new(p).exists()).map(|p| p.to_string()))?;
    Some(format!(
        "{} --headless=new --disable-gpu --hide-scrollbars --screenshot={} --window-size={width},{height} {} {}",
        crate::security::shell_quote(&bin), crate::security::shell_quote(out),
        if full_page { "--screenshot-full-page" } else { "" }, crate::security::shell_quote(url)))
}

#[async_trait]
impl Tool for Screenshot {
    fn read_only(&self) -> bool { false }
    fn name(&self) -> &'static str { "screenshot" }
    fn description(&self) -> &'static str { "Take a screenshot and look at it: the whole screen, the active window ({window:true}), or a web page ({url}) rendered with headless Chrome. The image comes back attached, so you can describe or debug what is on screen — UI work, a failing app, a chart." }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{
            "url":{"type":"string","description":"render this page instead of the screen"},
            "window":{"type":"boolean","description":"capture the active window only (screen capture)"},
            "path":{"type":"string","description":"where to save the PNG (default: a timestamped file in the pastes dir)"},
            "width":{"type":"integer","description":"url: viewport width (default 1280)"},
            "height":{"type":"integer","description":"url: viewport height (default 900)"},
            "full_page":{"type":"boolean","description":"url: capture the whole scrollable page"}
        }})
    }
    async fn call(&self, args: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let ctx = ctx.effective();
        let out: PathBuf = match args.get("path").and_then(|v| v.as_str()) {
            Some(p) => ctx.resolve(p)?,
            None => {
                let dir = ctx.memory.as_ref().map(|m| m.pastes_dir()).unwrap_or_else(std::env::temp_dir);
                std::fs::create_dir_all(&dir).ok();
                let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
                dir.join(format!("screenshot-{n}.png"))
            }
        };
        let outs = out.display().to_string();
        let cmd = match args.get("url").and_then(|v| v.as_str()) {
            Some(url) => {
                let (w, h) = (args.get("width").and_then(|v| v.as_u64()).unwrap_or(1280) as u32, args.get("height").and_then(|v| v.as_u64()).unwrap_or(900) as u32);
                page_command(url, &outs, w, h, args.get("full_page").and_then(|v| v.as_bool()).unwrap_or(false))
                    .context("no Chrome-like browser found for rendering a page (install Chrome/Chromium, or use the chrome-devtools MCP server)")?
            }
            None => capture_command(&outs, args.get("window").and_then(|v| v.as_bool()).unwrap_or(false))
                .context("no screen-capture tool on this platform (macOS: built in; Linux: install gnome-screenshot, spectacle, grim, import or scrot)")?,
        };
        let o = crate::sandbox::run_shell(&cmd, &ctx.workdir, Duration::from_secs(60), 4000).await?;
        if !out.is_file() { bail!("screenshot failed: {}{}", o.stdout.trim(), o.stderr.trim()); }
        // hand it back as an image the model can actually see (the file may live outside the workdir)
        let img = super::image::attach(&out).await?;
        Ok(ToolOutput { text: format!("screenshot saved to {outs}\n{}", img.text), images: img.images })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_platform_commands() {
        let c = capture_command("/tmp/a b.png", false);
        if cfg!(target_os = "macos") {
            let c = c.unwrap();
            assert!(c.starts_with("screencapture") && c.contains("'/tmp/a b.png'"), "{c}");
            assert!(capture_command("/tmp/x.png", true).unwrap().contains(" -w "));
        }
        // page rendering only when a browser exists; the command must quote the URL
        if let Some(p) = page_command("https://example.com/?a=b&c=d", "/tmp/p.png", 800, 600, true) {
            assert!(p.contains("--headless=new") && p.contains("--screenshot-full-page"), "{p}");
            assert!(p.contains("'https://example.com/?a=b&c=d'"), "{p}");
            assert!(p.contains("--window-size=800,600"), "{p}");
        }
    }
}
