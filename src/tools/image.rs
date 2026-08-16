//! Vision input: hand an image file to the (VLM) model.

use super::{arg_str, Tool, ToolCtx, ToolOutput};
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use base64::Engine;
use serde_json::{json, Value};

pub struct ViewImage;

pub fn mime_for(path: &std::path::Path) -> Option<&'static str> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "bmp" => Some("image/bmp"),
        _ => None,
    }
}

/// Best-effort dimensions from PNG/JPEG headers, for the text summary.
fn dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() > 24 && &bytes[..8] == b"\x89PNG\r\n\x1a\n" {
        return Some((u32::from_be_bytes(bytes[16..20].try_into().ok()?), u32::from_be_bytes(bytes[20..24].try_into().ok()?)));
    }
    if bytes.len() > 4 && bytes[0] == 0xFF && bytes[1] == 0xD8 {
        let mut i = 2;
        while i + 9 < bytes.len() {
            if bytes[i] != 0xFF { return None; }
            let marker = bytes[i + 1];
            let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
            if (0xC0..=0xCF).contains(&marker) && marker != 0xC4 && marker != 0xC8 && marker != 0xCC {
                let h = u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]) as u32;
                let w = u16::from_be_bytes([bytes[i + 7], bytes[i + 8]]) as u32;
                return Some((w, h));
            }
            i += 2 + len;
        }
    }
    None
}

#[async_trait]
impl Tool for ViewImage {
    fn name(&self) -> &'static str { "view_image" }
    fn description(&self) -> &'static str {
        "Look at an image file (png/jpg/gif/webp) — screenshots, plots, generated images, UI renders. The image is shown to you in the next message so you can describe, verify or critique it."
    }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"]})
    }
    async fn call(&self, args: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let path = ctx.resolve(arg_str(&args, "path")?)?;
        let mime = mime_for(&path).context("unsupported image type (use png/jpg/gif/webp)")?;
        let bytes = tokio::fs::read(&path).await.with_context(|| format!("reading {}", path.display()))?;
        const MAX: usize = 12 * 1024 * 1024;
        if bytes.len() > MAX { bail!("image is {} bytes; max {MAX}. Downscale it first (e.g. `sips -Z 1600 in.png --out small.png`).", bytes.len()); }
        let dims = dimensions(&bytes).map(|(w, h)| format!("{w}x{h}, ")).unwrap_or_default();
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        Ok(ToolOutput {
            text: format!("attached {} ({dims}{} KB) — the image follows in the next message.", path.display(), bytes.len() / 1024),
            images: vec![(mime.to_string(), b64)],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn png_dims() {
        let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        b.extend_from_slice(&64u32.to_be_bytes()); b.extend_from_slice(&32u32.to_be_bytes()); b.extend_from_slice(&[8, 2, 0, 0, 0]);
        assert_eq!(dimensions(&b), Some((64, 32)));
        assert_eq!(mime_for(std::path::Path::new("a.JPG")), Some("image/jpeg"));
        assert_eq!(mime_for(std::path::Path::new("a.txt")), None);
    }
}
