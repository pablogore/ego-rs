//! Persistent Entity Runtime and SDK
//!
//! This crate provides the core runtime and SDK for persistent entities in the EGO system.
//!
//! # Modules
//!
//! - [`entity_ref`]: The main API for interacting with persistent entities
//! - [`persistent_entity`]: Trait defining the interface for persistent entities
//! - [`command_context`]: Context information available during command processing
//! - [`runtime`]: The main runtime manager for entity lifecycle
//! - [`actor`]: The entity actor implementation
//! - [`lifecycle`]: State management for entity lifecycle
//! - [`mailbox`]: Bounded FIFO mailbox for command queuing
//! - [`recovery`]: State recovery and snapshotting logic
//! - [`passivation`]: Passivation policy and registry
//! - [`snapshot`]: Snapshot strategy definitions
//! - [`error`]: Error types for the persistent entity system
//! - [`testing`]: Test helpers and utilities

pub mod entity_ref;
pub mod persistent_entity;
pub mod command_context;
pub mod command_envelope;
pub mod runtime;
pub mod actor;
pub mod lifecycle;
pub mod mailbox;
pub mod recovery;
pub mod passivation;
pub mod snapshot;
pub mod error;
pub mod testing;
pub mod test_entity;
pub mod registry;
pub mod scheduler;
pub mod persistence;
pub mod publisher;

// Re-export test types for easier access in tests
pub use testing::{TestCommand, TestEvent, TestState};