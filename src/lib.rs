//! TheHarness core library: agent loop, tools, sandbox, LLM client, evals.
//! The CLI (`src/main.rs`) and the Tauri UI (`ui/src-tauri`) are thin frontends over this.

pub mod acp;
pub mod acp_client;
pub mod agent;
pub mod agentdefs;
pub mod arbiter;
pub mod arena;
pub mod checkpoints;
pub mod claude_code;
pub mod config;
pub mod eval;
pub mod events;
pub mod export;
pub mod format;
pub mod headless;
pub mod hooks;
pub mod import;
pub mod inbox;
pub mod instructions;
pub mod llm;
pub mod lsp;
pub mod mailbox;
pub mod mcp;
pub mod mcp_bridge;
pub mod memory;
pub mod permissions;
pub mod plugins;
pub mod procs;
pub mod repomap;
pub mod runner;
pub mod sandbox;
pub mod selfimprove;
pub mod serve;
pub mod scheduler;
pub mod security;
pub mod sessions;
pub mod setup;
pub mod skills;
pub mod tools;
pub mod workflow;
pub mod worktree;

/// Harness version string: MAJOR.MINOR.BUILD (build number increments per release build), plus git sha.
pub fn version() -> String { format!("{} ({})", env!("HARNESS_VERSION"), env!("HARNESS_GIT")) }
pub const VERSION: &str = env!("HARNESS_VERSION");
