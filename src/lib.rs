//! Code Atlas: application services and infrastructure adapters.
pub mod analyzer;
pub mod config;
pub(crate) mod diagnostics_log;
pub mod domain;
pub mod glob;
pub mod grep_search;
pub mod http;
pub mod indexer;
pub mod mcp_text;
pub(crate) mod parallel;
pub mod plugins;
pub mod postgres;
pub mod progress;
pub mod project;
pub mod repository;
pub mod service;
pub mod setup;
