//! What a run costs. A small table of published prices (USD per million tokens) keyed by model-name
//! glob, plus whatever the user adds in `[llm.pricing]`; local models are free, so most sessions show
//! nothing. Spending is tracked per process, which is what `/cost` shows and what `--max-budget-usd`
//! stops a headless run on.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub struct Price { pub input: f64, pub output: f64 }

/// Published list prices (USD per 1M tokens), most specific glob first.
pub const TABLE: &[(&str, Price)] = &[
    ("*opus-5*", Price { input: 5.0, output: 25.0 }),
    ("*opus*", Price { input: 15.0, output: 75.0 }),
    ("*sonnet-5*", Price { input: 3.0, output: 15.0 }),
    ("*sonnet*", Price { input: 3.0, output: 15.0 }),
    ("*fable*", Price { input: 3.0, output: 15.0 }),
    ("*haiku*", Price { input: 0.8, output: 4.0 }),
    ("gpt-5*", Price { input: 1.25, output: 10.0 }),
    ("gpt-4.1*", Price { input: 2.0, output: 8.0 }),
    ("gpt-4o*", Price { input: 2.5, output: 10.0 }),
    ("o3*", Price { input: 2.0, output: 8.0 }),
    ("o4*", Price { input: 1.1, output: 4.4 }),
    ("gemini-2.5-pro*", Price { input: 1.25, output: 10.0 }),
    ("gemini*flash*", Price { input: 0.3, output: 2.5 }),
    ("deepseek*", Price { input: 0.28, output: 0.42 }),
    ("grok*", Price { input: 3.0, output: 15.0 }),
    ("*llama*", Price { input: 0.2, output: 0.6 }),
];

static OVERRIDES: Mutex<Option<Vec<(String, Price)>>> = Mutex::new(None);
/// `[llm.pricing]` entries: `"my-model*" = { input = 1.0, output = 3.0 }`.
pub fn configure(overrides: Vec<(String, Price)>) { *OVERRIDES.lock().unwrap() = Some(overrides); }

/// Price for a model name, or None when we do not know it (local models: free).
pub fn price_of(model: &str) -> Option<Price> {
    let m = model.trim().to_lowercase();
    if let Some(ov) = OVERRIDES.lock().unwrap().as_ref() {
        if let Some((_, p)) = ov.iter().find(|(g, _)| crate::permissions::glob_match(&g.to_lowercase(), &m)) { return Some(*p); }
    }
    // a local server is identified by the absence of a known vendor prefix, so unknown = free
    TABLE.iter().find(|(g, _)| crate::permissions::glob_match(g, &m)).map(|(_, p)| *p)
}

pub fn cost_of(model: &str, prompt_tokens: u64, completion_tokens: u64) -> Option<f64> {
    let p = price_of(model)?;
    Some((prompt_tokens as f64 / 1e6) * p.input + (completion_tokens as f64 / 1e6) * p.output)
}

// spending is kept in micro-dollars so it can live in an atomic
static SPENT_MICRO: AtomicU64 = AtomicU64::new(0);
static BUDGET_MICRO: AtomicU64 = AtomicU64::new(0);

/// Add one model call to the running total; returns its cost (0.0 for unpriced/local models).
pub fn record(model: &str, prompt_tokens: u64, completion_tokens: u64) -> f64 {
    let Some(c) = cost_of(model, prompt_tokens, completion_tokens) else { return 0.0 };
    SPENT_MICRO.fetch_add((c * 1e6).round() as u64, Ordering::Relaxed);
    c
}
pub fn spent_usd() -> f64 { SPENT_MICRO.load(Ordering::Relaxed) as f64 / 1e6 }
pub fn reset() { SPENT_MICRO.store(0, Ordering::Relaxed); }
/// `--max-budget-usd`: 0/None removes the cap.
pub fn set_budget(usd: Option<f64>) { BUDGET_MICRO.store(usd.filter(|u| *u > 0.0).map(|u| (u * 1e6) as u64).unwrap_or(0), Ordering::Relaxed); }
pub fn budget_usd() -> Option<f64> { let b = BUDGET_MICRO.load(Ordering::Relaxed); (b > 0).then(|| b as f64 / 1e6) }
pub fn over_budget() -> bool { let b = BUDGET_MICRO.load(Ordering::Relaxed); b > 0 && SPENT_MICRO.load(Ordering::Relaxed) >= b }

/// "$0.0123" / "$1.23" — or None for a free/unknown model.
pub fn fmt_usd(usd: f64) -> String {
    if usd >= 1.0 { format!("${usd:.2}") } else if usd >= 0.01 { format!("${usd:.3}") } else { format!("${usd:.4}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prices_and_budget() {
        assert!(price_of("claude-opus-5").is_some());
        assert!(price_of("qwen3.8-27b-mlx").is_none(), "local models are free");
        let c = cost_of("claude-sonnet-5", 1_000_000, 100_000).unwrap();
        assert!((c - (3.0 + 1.5)).abs() < 1e-9, "{c}");
        configure(vec![("qwen*".into(), Price { input: 0.1, output: 0.2 })]);
        assert!(price_of("qwen3.8-27b-mlx").is_some(), "overrides win");
        configure(vec![]);

        reset();
        set_budget(Some(0.01));
        assert!(!over_budget());
        record("claude-opus-5", 1_000_000, 0); // $5
        assert!(over_budget());
        assert!(spent_usd() > 4.9);
        set_budget(None);
        assert!(!over_budget(), "no budget, no limit");
        reset();
        assert_eq!(fmt_usd(1.5), "$1.50");
        assert_eq!(fmt_usd(0.0004), "$0.0004");
    }
}
