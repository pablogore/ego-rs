//! Persistence SPI — domain-owned contracts for event sourcing.
//!
//! Defines the `EventStore`, `Repository`, and `Snapshot` traits along
//! with `PersistenceError`. All persistence logic is abstracted behind
//! these interfaces — infrastructure provides concrete backends.
//!
//! ## Modules
//!
//! | Module | Purpose |
//! |--------|---------|
//! | `error` | `PersistenceError` — NotFound, Conflict, Internal, MissingTenant |
//! | `event_store` | `EventStore<E>` — append and load domain events |
//! | `repository` | `Repository<A>` — save, load, and delete aggregates |
//! | `snapshot` | `Snapshot` — save and load aggregate state snapshots |

/// Persistence error types.
pub mod error;

/// Event store contract — append and load domain events.
pub mod event_store;

/// Aggregate repository contract — save, load, and delete.
pub mod repository;

/// Aggregate snapshot contract — save and load state snapshots.
pub mod snapshot;

pub use error::PersistenceError;
pub use event_store::EventStore;
pub use repository::Repository;
pub use snapshot::Snapshot;
