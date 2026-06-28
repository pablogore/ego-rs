//! Thin typed accessor over `serde_json::Value` for ego-rs services.
//!
//! `ego-config-sdk` is NOT a config loading framework. It wraps the
//! `serde_json::Value` output from `kit_config::ConfigLoader` and provides
//! typed `get`/`require` access. Loading, merging, and file/env sources
//! are kit-config's responsibility.

mod convert;
mod config;
mod error;
mod value;

pub use config::Configuration;
pub use error::ConfigurationError;
pub use value::ConfigValue;
