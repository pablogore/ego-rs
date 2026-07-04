//! PostgreSQL persistence backend for ego-rs.
//!
//! Provides concrete implementations of domain persistence traits
//! (`EventStore`, `Repository`, `Snapshot`) backed by PostgreSQL.
//! Includes embedded migration support via `sqlx`.

pub mod config;
pub mod postgres;

pub use config::DatabaseConfig;
pub use postgres::{PostgreSQLEventStore, PostgreSQLRepository, PostgreSQLSnapshotStore};
