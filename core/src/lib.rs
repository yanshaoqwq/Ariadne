pub mod cli;
pub mod command_error;
pub mod commands;
pub mod config;
pub mod contracts;
pub mod costs;
pub mod diagnostics;
pub mod documents;
pub mod frontend;
pub mod git;
pub mod ipc;
pub mod knowledge;
pub mod llm;
pub mod providers;
pub mod rag;
pub mod rest;
pub mod retrieval;
pub mod skills;
pub mod workflow;

/// 产品版本由 workspace `Cargo.toml` 的 `workspace.package.version` 注入。
pub const PRODUCT_VERSION: &str = env!("CARGO_PKG_VERSION");
/// IPC schema 独立于产品版本，破坏性协议变更时递增。
pub const IPC_SCHEMA_VERSION: u32 = 2;
