//! Persistent Entity Runtime and SDK
//!
//! This crate provides the core runtime and SDK for persistent entities in the EGO system.
// Pre-existing lints suppressed at crate level — not introduced by this change.
#![allow(clippy::new_without_default)]
#![allow(clippy::needless_return)]
#![allow(clippy::needless_borrows_for_generic_args)]
#![allow(clippy::manual_is_multiple_of)]
#![allow(clippy::unwrap_or_default)]
//!
//! # Modules
//!
//! - [`entity_ref`]: The main API for interacting with persistent entities
//! - [`entity_ref_tokio`]: Tokio-backed production [`EntityRef`] implementation
//! - [`persistent_entity`]: Trait defining the interface for persistent entities
//! - [`command_context`]: Context information available during command processing
//! - [`runtime`]: The main runtime manager for entity lifecycle
//! - [`actor`]: The entity actor implementation
//! - [`lifecycle`]: State management for entity lifecycle
//! - [`mailbox`]: Bounded FIFO mailbox for command queuing
//! - [`recovery`]: State recovery and snapshotting logic
//! - [`passivation`]: Passivation policy and registry
//! - [`passivation_signal`]: Runtime-agnostic passivation signal trait
//! - [`snapshot`]: Snapshot strategy definitions
//! - [`error`]: Error types for the persistent entity system
//! - [`testing`]: Test helpers and utilities

pub mod actor;
pub mod builder;
pub mod command_context;
pub mod command_envelope;
pub mod effect_acceptor;
pub mod entity_ref;
pub mod entity_ref_tokio;
pub mod error;
pub mod execution_backend;
pub mod execution_backend_tokio;
pub mod lifecycle;
pub mod mailbox;
pub mod passivation;
pub mod passivation_signal;
pub mod persistence;
pub mod persistent_entity;
pub mod publisher;
pub mod recovery;
pub mod registry;
pub mod runtime;
pub mod scheduler;
pub mod scheduler_event;
pub mod scheduler_policy;
pub mod snapshot;
pub mod test_entity;
pub mod testing;

// Re-export test types for easier access in tests
pub use testing::{TestCommand, TestEvent, TestState};
