//! Read side projection engine.
//!
//! Provides traits and types for defining, running, and persisting
//! tag-based read-side projections over event streams.

pub mod config;
pub mod error;
pub mod handler;
pub mod processor;
pub mod progress;
pub mod runner;
pub mod scheduler;
pub mod session;
pub mod tagger;

// CORE-PERSIST-A S1 (AD-4): relocated to `ego-persistence-api`, re-exported
// here at module granularity so every old `ego_domain::read_side::*` path
// keeps resolving to the identical item. `projection_state_store` is
// renamed `projection_state` at its new home; the alias below keeps this
// crate's path unchanged.
pub use ego_persistence_api::read_side::{
    claim, dedup, event_stream, event_tag, offset, projection_state as projection_state_store,
    state, store,
};
