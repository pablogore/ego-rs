//! Configuration framework SDK for ego-rs.
//!
//! Provides a composable, multi-source configuration system with
//! priority-based resolution, type coercion, and conflict detection.
//!
//! # Quick start
//!
//! Use `builder::ConfigurationBuilder` to compose providers and resolve
//! a `config::Configuration`.
#![deny(missing_docs)]

/// Configuration value types.
pub mod value;

/// Error types for the configuration framework.
pub mod error;

/// Provider trait and source snapshot.
pub mod provider;

/// Post-load configuration source snapshot.
pub mod source;

/// Priority-based resolver (crate-internal).
pub(crate) mod resolver;

/// Fluent builder for composing providers.
pub mod builder;

/// Resolved, immutable configuration handle.
pub mod config;

/// Built-in provider implementations.
pub mod providers;
