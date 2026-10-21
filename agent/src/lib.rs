//! NanoHermes Agent — a lightweight, low-memory terminal coding agent.
//!
//! This crate can be used both as a binary (`agent` CLI) and as a library
//! (for embedding in other apps such as the Tauri desktop GUI).

pub mod agent;
pub mod board;
pub mod config;
pub mod hooks;
pub mod interrupt;
pub mod llm;
pub mod mcp;
pub mod memory;
pub mod prompt;
pub mod sandbox;
pub mod skills;
pub mod sys;
pub mod tools;
pub mod ui;

pub use agent::{user_content, Agent, UsageTotals, MAX_ATTACHED_IMAGES};
pub use config::{ApprovalPolicy, Config, HookEvent, OutputFormat, PermissionMode, SandboxMode};
