//! PostgreSQL persistence backend for ego-rs.
//!
//! Provides concrete implementations of domain persistence traits
//! (`EventStore`, `Repository`, `Snapshot`) backed by PostgreSQL.
//! Includes embedded migration support via `sqlx`.

pub mod postgres;

pub use postgres::{PostgreSQLEventStore, PostgreSQLRepository, PostgreSQLSnapshotStore};
