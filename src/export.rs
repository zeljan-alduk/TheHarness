//! Session export: markdown for reading/diffing, and a self-contained HTML page for sharing
//! (`/share`). No upload — the file is yours; `gh gist create` is one command away if you want a link.

use crate::llm::Message;
use crate::sessions::Meta;

pub fn markdown(meta: &Meta, msgs: &[Message]) -> String {
    let mut md = format!("# {}\n\n_{} · {} · {} turns_\n\n", if meta.title.is_empty() { "Harness session".into() } else { meta.title.clone() }, meta.workdir, meta.model, meta.turns);
    for m in msgs.iter() {
        match m.role.as_str() {
            "user" => md.push_str(&format!("## User\n\n{}\n\n", m.text())),
            "assistant" => {
                let t = m.text();
                if !t.trim().is_empty() { md.push_str(&format!("## Assistant\n\n{}\n\n", t)); }
                if let Some(calls) = &m.tool_calls {
                    for c in calls { md.push_str(&format!("**tool** `{}` `{}`\n\n", c.function.name, crate::llm::truncate_for_log(&c.function.arguments, 300))); }
                }
            }
            "tool" => md.push_str(&format!("```\n{}\n```\n\n", crate::llm::truncate_for_log(&m.text(), 1500))),
            _ => {}
        }
    }
    md
}

fn esc(s: &str) -> String { s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;") }

/// A single self-contained HTML file: no scripts, no external assets, readable in any browser and
/// fine to attach to an issue or drop in a gist.
pub fn html(meta: &Meta, msgs: &[Message]) -> String {
    let title = if meta.title.is_empty() { "Harness session".to_string() } else { meta.title.clone() };
    let mut body = String::new();
    for m in msgs.iter() {
        match m.role.as_str() {
            "user" => body.push_str(&format!("<section class=\"user\"><h2>User</h2><pre>{}</pre></section>\n", esc(&m.text()))),
            "assistant" => {
                let t = m.text();
                if !t.trim().is_empty() { body.push_str(&format!("<section class=\"assistant\"><h2>Assistant</h2><pre>{}</pre></section>\n", esc(&t))); }
                if let Some(calls) = &m.tool_calls {
                    for c in calls {
                        body.push_str(&format!("<section class=\"call\"><code>{}</code> <span class=\"args\">{}</span></section>\n", esc(&c.function.name), esc(&crate::llm::truncate_for_log(&c.function.arguments, 400))));
                    }
                }
            }
            "tool" => body.push_str(&format!("<section class=\"result\"><pre>{}</pre></section>\n", esc(&crate::llm::truncate_for_log(&m.text(), 4000)))),
            _ => {}
        }
    }
    format!(r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>{title}</title>
<style>
 :root {{ color-scheme: light dark; --fg:#1b1b1f; --bg:#fbfbfd; --muted:#6b6b76; --line:#e4e4ea; --user:#eef3ff; --tool:#f4f4f7; }}
 @media (prefers-color-scheme: dark) {{ :root {{ --fg:#e6e6ea; --bg:#131317; --muted:#9a9aa6; --line:#2a2a32; --user:#1a2333; --tool:#1c1c22; }} }}
 body {{ margin:0 auto; padding:2rem 1rem 6rem; max-width:52rem; background:var(--bg); color:var(--fg);
        font:15px/1.6 ui-sans-serif,-apple-system,Segoe UI,Roboto,sans-serif; }}
 header {{ border-bottom:1px solid var(--line); padding-bottom:1rem; margin-bottom:2rem; }}
 h1 {{ font-size:1.35rem; margin:0 0 .35rem; }}
 .meta {{ color:var(--muted); font-size:.85rem; }}
 h2 {{ font-size:.75rem; text-transform:uppercase; letter-spacing:.08em; color:var(--muted); margin:0 0 .4rem; }}
 section {{ margin:0 0 1.1rem; }}
 pre {{ margin:0; white-space:pre-wrap; word-wrap:break-word; font:13px/1.55 ui-monospace,SFMono-Regular,Menlo,monospace; }}
 .user pre {{ background:var(--user); padding:.8rem 1rem; border-radius:.6rem; }}
 .result pre {{ background:var(--tool); padding:.7rem .9rem; border-radius:.5rem; color:var(--muted); max-height:22rem; overflow:auto; }}
 .call {{ font:12px/1.5 ui-monospace,monospace; color:var(--muted); }}
 .call code {{ color:var(--fg); }}
 .args {{ opacity:.75; }}
</style></head><body>
<header><h1>{title}</h1><div class="meta">{workdir} · {model} · {turns} turns · exported by harness {version}</div></header>
{body}</body></html>
"#, title = esc(&title), workdir = esc(&meta.workdir), model = esc(&meta.model), turns = meta.turns, version = crate::VERSION, body = body)
}

/// Write an export next to the others and return its path.
pub fn write(meta: &Meta, msgs: &[Message], as_html: bool) -> std::io::Result<std::path::PathBuf> {
    let dir = crate::setup::config_dir().join("exports");
    std::fs::create_dir_all(&dir)?;
    let id = if meta.id.is_empty() { "unsaved".to_string() } else { meta.id.clone() };
    let path = dir.join(format!("session-{id}.{}", if as_html { "html" } else { "md" }));
    std::fs::write(&path, if as_html { html(meta, msgs) } else { markdown(meta, msgs) })?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn renders_both_formats() {
        let meta = Meta { id: "s1".into(), title: "Fix <the> bug".into(), workdir: "/proj".into(), model: "m".into(), turns: 1, ..Default::default() };
        let msgs = vec![Message::system("sys"), Message::user("do <it>"), Message::tool("t1", "bash", "output & more")];
        let md = markdown(&meta, &msgs);
        assert!(md.contains("## User") && md.contains("do <it>"));
        let h = html(&meta, &msgs);
        assert!(h.contains("Fix &lt;the&gt; bug"), "titles are escaped");
        assert!(h.contains("output &amp; more"), "tool output is escaped");
        assert!(!h.contains("<script"), "no scripts in a shared page");
        assert!(h.contains("prefers-color-scheme"));
    }
}
