//! Read side projection engine.
//!
//! Provides traits and types for defining, running, and persisting
//! tag-based read-side projections over event streams.

pub mod config;
pub mod dedup;
pub mod error;
pub mod event_stream;
pub mod event_tag;
pub mod handler;
pub mod offset;
pub mod progress;
pub mod processor;
pub mod runner;
pub mod scheduler;
pub mod session;
pub mod state;
pub mod store;
pub mod tagger;
pub mod projection_state_store;
