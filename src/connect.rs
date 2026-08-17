//! Chat connectors: run the agent from a messaging app. Telegram first, because a bot token is one
//! message away and it makes the useful case work — you are away from the machine, the agent asks for
//! permission, and you answer from your phone.
//!
//! `harness connect telegram --token <bot token> [--allow <chat id>] [-C dir]`

use crate::config::Config;
use crate::permissions::{Approval, ApprovalRequest, Approver, Question, Answer};
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub struct Telegram {
    http: reqwest::Client,
    token: String,
    chat: Mutex<Option<i64>>,
    /// Chat ids allowed to drive the agent (empty = the first chat that talks to us).
    allow: Vec<i64>,
    /// Replies waiting to be consumed by a pending question.
    pending: Mutex<Vec<String>>,
}

impl Telegram {
    pub fn new(token: &str, allow: Vec<i64>) -> Result<Arc<Self>> {
        if token.trim().is_empty() { bail!("a bot token is required (talk to @BotFather, then pass --token or set TELEGRAM_BOT_TOKEN)"); }
        Ok(Arc::new(Self {
            http: reqwest::Client::builder().timeout(std::time::Duration::from_secs(70)).build()?,
            token: token.to_string(), chat: Mutex::new(None), allow, pending: Mutex::new(Vec::new()),
        }))
    }
    fn api(&self, method: &str) -> String { format!("https://api.telegram.org/bot{}/{method}", self.token) }

    pub async fn send(&self, text: &str) {
        let Some(chat) = *self.chat.lock().unwrap() else { return };
        for chunk in split_message(text) {
            let _ = self.http.post(self.api("sendMessage")).json(&json!({"chat_id": chat, "text": chunk, "disable_web_page_preview": true})).send().await;
        }
    }

    fn allowed(&self, chat: i64) -> bool { self.allow.is_empty() || self.allow.contains(&chat) }

    /// Poll for messages; returns the ones addressed to us since `offset`.
    async fn updates(&self, offset: i64) -> Result<(i64, Vec<String>)> {
        let v: Value = self.http.get(self.api("getUpdates")).query(&[("timeout", "50"), ("offset", &offset.to_string())]).send().await?.json().await?;
        let mut next = offset;
        let mut texts = Vec::new();
        for u in v["result"].as_array().cloned().unwrap_or_default() {
            next = u["update_id"].as_i64().unwrap_or(next) + 1;
            let Some(msg) = u.get("message").or_else(|| u.get("channel_post")) else { continue };
            let chat = msg["chat"]["id"].as_i64().unwrap_or(0);
            if !self.allowed(chat) { continue; }
            { let mut c = self.chat.lock().unwrap(); if c.is_none() { *c = Some(chat); } }
            if let Some(t) = msg["text"].as_str() { if !t.trim().is_empty() { texts.push(t.trim().to_string()); } }
        }
        Ok((next, texts))
    }

    /// Wait for the next message, treating it as an answer to a question we asked.
    async fn wait_reply(&self, timeout: std::time::Duration) -> Option<String> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if let Some(t) = self.pending.lock().unwrap().pop() { return Some(t); }
            if std::time::Instant::now() >= deadline { return None; }
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }
    }
}

/// Telegram caps a message at 4096 characters.
pub fn split_message(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for line in text.lines() {
        if cur.chars().count() + line.chars().count() > 3800 { out.push(std::mem::take(&mut cur)); }
        cur.push_str(line); cur.push('\n');
        while cur.chars().count() > 3800 { let head: String = cur.chars().take(3800).collect(); out.push(head); cur = cur.chars().skip(3800).collect(); }
    }
    if !cur.trim().is_empty() { out.push(cur); }
    if out.is_empty() { out.push("(no output)".into()); }
    out
}

/// Permission prompts and questions travel to the chat and back.
#[async_trait::async_trait]
impl Approver for Telegram {
    fn interactive(&self) -> bool { true }
    async fn ask(&self, req: ApprovalRequest) -> Approval {
        self.send(&format!("🔒 permission needed\n{}({})\nwhy: {}\n\nreply: y (once) · a (always: {}) · n (deny)", req.tool, crate::llm::truncate_for_log(&req.summary, 300), req.reason, req.suggested_rule)).await;
        match self.wait_reply(std::time::Duration::from_secs(300)).await {
            Some(r) => match r.trim().chars().next().unwrap_or('n') {
                'y' | 'Y' => Approval::Once,
                'a' | 'A' => Approval::Always,
                _ => { self.send("denied").await; Approval::Deny }
            },
            None => { self.send("no answer in 5 minutes — denied").await; Approval::Deny }
        }
    }
    async fn question(&self, q: Question) -> Option<Answer> {
        let opts: Vec<String> = q.options.iter().enumerate().map(|(i, o)| format!("{}. {}{}", i + 1, o.label, if o.description.is_empty() { String::new() } else { format!(" — {}", o.description) })).collect();
        self.send(&format!("❓ {}\n{}", q.question, if opts.is_empty() { "(reply with your answer)".to_string() } else { opts.join("\n") })).await;
        let reply = self.wait_reply(std::time::Duration::from_secs(q.timeout_secs.unwrap_or(300))).await?;
        let t = reply.trim().to_string();
        if let Ok(n) = t.parse::<usize>() { if n >= 1 && n <= q.options.len() { return Some(Answer { choice: Some(n - 1), ..Default::default() }); } }
        Some(Answer { text: Some(t), ..Default::default() })
    }
}

/// Run the Telegram connector until interrupted.
pub async fn telegram(cfg: Config, token: &str, allow: Vec<i64>, workdir: PathBuf) -> Result<()> {
    let bot = Telegram::new(token, allow)?;
    let me: Value = bot.http.get(bot.api("getMe")).send().await.context("reaching Telegram")?.json().await?;
    let name = me["result"]["username"].as_str().unwrap_or("bot").to_string();
    eprintln!("connected as @{name} — message the bot to start; ctrl+c stops. Working directory: {}", workdir.display());
    let mut offset = 0i64;
    let busy = Arc::new(std::sync::atomic::AtomicBool::new(false));
    loop {
        let (next, texts) = match bot.updates(offset).await { Ok(v) => v, Err(e) => { eprintln!("· telegram: {e:#}"); tokio::time::sleep(std::time::Duration::from_secs(5)).await; continue; } };
        offset = next;
        for text in texts {
            // while a task runs, messages are answers to whatever it asked
            if busy.load(std::sync::atomic::Ordering::Relaxed) { bot.pending.lock().unwrap().push(text); continue; }
            match text.as_str() {
                "/start" | "/help" => { bot.send("Send me a task and I will work on it in the repository this bot was started in. While I work, replies answer my questions (y/a/n for permissions). /status shows where I am.").await; continue; }
                "/status" => { bot.send(&format!("idle · {} · model {}", workdir.display(), cfg.llm.model)).await; continue; }
                _ => {}
            }
            bot.send("working…").await;
            busy.store(true, std::sync::atomic::Ordering::Relaxed);
            let (cfg2, wd, bot2, busy2) = (cfg.clone(), workdir.clone(), bot.clone(), busy.clone());
            tokio::spawn(async move {
                let sink: Arc<dyn crate::events::Sink> = Arc::new(crate::events::StderrSink { verbose: false });
                let approver: Arc<dyn Approver> = bot2.clone();
                let mut setup = crate::runner::RunSetup::new(cfg2, wd.clone(), sink, approver);
                setup.session_id = Some(format!("telegram-{}", crate::scheduler::now()));
                setup.prompt_extra = Some("You are being driven from a chat app: the user is not at the keyboard. Keep answers short and concrete, and ask (ask_user) rather than guessing when something is ambiguous.".into());
                let out = crate::runner::start_run(setup, text).await;
                bot2.send(&match out { Ok(t) => t, Err(e) => format!("✖ {e:#}") }).await;
                busy2.store(false, std::sync::atomic::Ordering::Relaxed);
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_long_messages() {
        let one = split_message("short");
        assert_eq!(one, vec!["short\n"]);
        let long = "x".repeat(9000);
        let parts = split_message(&long);
        assert!(parts.len() >= 3, "{}", parts.len());
        assert!(parts.iter().all(|p| p.chars().count() <= 3801));
        assert_eq!(split_message("")[0], "(no output)");
    }
}
