//! TheHarness core library: agent loop, tools, sandbox, LLM client, evals.
//! The CLI (`src/main.rs`) and the Tauri UI (`ui/src-tauri`) are thin frontends over this.

pub mod agent;
pub mod arbiter;
pub mod config;
pub mod eval;
pub mod events;
pub mod hooks;
pub mod llm;
pub mod mcp;
pub mod memory;
pub mod permissions;
pub mod plugins;
pub mod procs;
pub mod sandbox;
pub mod security;
pub mod sessions;
pub mod setup;
pub mod tools;
