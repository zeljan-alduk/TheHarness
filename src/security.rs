//! Secret redaction for tool outputs (so keys in .env files, logs, or configs don't get echoed into
//! the transcript / model context / session logs). Conservative patterns for well-known token formats.

/// POSIX single-quote shell quoting: the result is safe to splice into an `sh -c` string as one word.
pub fn shell_quote(s: &str) -> String { format!("'{}'", s.replace('\'', "'\\''")) }

pub fn redact(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        out.push_str(&redact_line(line));
    }
    out
}

fn redact_line(line: &str) -> String {
    let mut s = line.to_string();
    // key=value style secrets in env-like lines
    let lower = s.to_ascii_lowercase();
    if let Some(eq) = s.find('=') {
        let key = lower[..eq].trim().trim_start_matches("export ");
        if key.len() < 64 && !key.contains(' ') && ["secret", "token", "password", "passwd", "api_key", "apikey", "private_key", "access_key", "client_secret"].iter().any(|k| key.ends_with(k) || key.contains(k)) {
            let val = s[eq + 1..].trim();
            if val.len() >= 6 && !val.starts_with('$') && !val.starts_with('<') { return format!("{}=[REDACTED]\n", &line[..eq]).trim_end_matches('\n').to_string() + if line.ends_with('\n') { "\n" } else { "" }; }
        }
    }
    // well-known token shapes
    for (prefix, min_len) in [("sk-", 20usize), ("sk-ant-", 20), ("ghp_", 30), ("gho_", 30), ("github_pat_", 30), ("xoxb-", 20), ("xoxp-", 20), ("AKIA", 16), ("AIza", 30), ("ya29.", 30), ("glpat-", 20), ("npm_", 30), ("hf_", 30)] {
        let mut idx = 0;
        while let Some(i) = s[idx..].find(prefix) {
            let start = idx + i;
            let tail: String = s[start + prefix.len()..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-' || *c == '.').collect();
            if tail.len() >= min_len && (start == 0 || !s.as_bytes()[start - 1].is_ascii_alphanumeric()) {
                let end = start + prefix.len() + tail.len();
                s.replace_range(start..end, &format!("{prefix}[REDACTED]"));
                idx = start + prefix.len() + 10;
            } else { idx = start + prefix.len(); }
            if idx >= s.len() { break; }
        }
    }
    if s.contains("-----BEGIN") && s.contains("PRIVATE KEY") { return "-----BEGIN PRIVATE KEY----- [REDACTED]\n".to_string(); }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn redacts() {
        assert_eq!(redact("OPENAI_API_KEY=sk-abcdefghijklmnopqrstuvwxyz1234\n"), "OPENAI_API_KEY=[REDACTED]\n");
        assert!(redact("token ghp_abcdefghijklmnopqrstuvwxyz0123456789ABCD here").contains("ghp_[REDACTED]"));
        assert_eq!(redact("PORT=8080\n"), "PORT=8080\n");
        assert_eq!(redact("let x = sk_test;"), "let x = sk_test;");
    }
    #[test]
    fn quotes() { assert_eq!(shell_quote("a b"), "'a b'"); assert_eq!(shell_quote("it's"), "'it'\\''s'"); assert_eq!(shell_quote(""), "''"); }
}
