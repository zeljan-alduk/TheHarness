//! Inbox: asynchronous events for the model (monitor output lines, scheduled prompts, sub-agent messages).
//! Tools push items; the agent loop drains them before each model call and hands them over as a user
//! message; a frontend waits on the inbox while idle and starts a new turn when something arrives.

use std::sync::Mutex;
use std::time::Instant;

#[derive(Debug, Clone)]
pub struct Item { pub source: String, pub text: String, pub at: Instant }

#[derive(Default)]
pub struct Inbox { items: Mutex<Vec<Item>>, notify: tokio::sync::Notify }

impl Inbox {
    pub fn new() -> Self { Self::default() }
    pub fn push(&self, source: impl Into<String>, text: impl Into<String>) {
        self.items.lock().unwrap().push(Item { source: source.into(), text: text.into(), at: Instant::now() });
        self.notify.notify_one();
    }
    pub fn is_empty(&self) -> bool { self.items.lock().unwrap().is_empty() }
    pub fn len(&self) -> usize { self.items.lock().unwrap().len() }
    pub fn drain(&self) -> Vec<Item> { std::mem::take(&mut *self.items.lock().unwrap()) }
    /// Wait until an item is pushed (a permit is stored if a push happened while nobody waited).
    pub async fn wait(&self) { self.notify.notified().await }
    /// Drain and render as one message for the model ("" if empty). Consecutive items from one source are grouped.
    pub fn take_message(&self) -> Option<String> {
        let items = self.drain();
        if items.is_empty() { return None; }
        let mut out = String::from("[harness inbox] events that arrived while you were working — react if relevant, otherwise continue:\n");
        let mut last = String::new();
        for it in items {
            if it.source != last { out.push_str(&format!("\n## {}\n", it.source)); last = it.source.clone(); }
            out.push_str(&it.text);
            if !it.text.ends_with('\n') { out.push('\n'); }
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn groups_by_source() {
        let i = Inbox::new();
        assert!(i.take_message().is_none());
        i.push("monitor #1", "a");
        i.push("monitor #1", "b\n");
        i.push("schedule", "c");
        let m = i.take_message().unwrap();
        assert!(m.contains("## monitor #1\na\nb\n\n## schedule\nc\n"), "{m}");
        assert!(i.is_empty());
    }
    #[tokio::test]
    async fn wait_gets_permit() {
        let i = std::sync::Arc::new(Inbox::new());
        i.push("x", "y");
        tokio::time::timeout(std::time::Duration::from_millis(100), i.wait()).await.unwrap();
    }
}
