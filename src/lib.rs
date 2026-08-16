//! TheHarness core library: agent loop, tools, sandbox, LLM client, evals.
//! The CLI (`src/main.rs`) and the Tauri UI (`ui/src-tauri`) are thin frontends over this.

pub mod agent;
pub mod config;
pub mod eval;
pub mod events;
pub mod llm;
pub mod mcp;
pub mod memory;
pub mod plugins;
pub mod sandbox;
pub mod setup;
pub mod tools;
