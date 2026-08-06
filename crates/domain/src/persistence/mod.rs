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
//! | `event_store` | `EventStore<E>` — append and load domain events; `EventStoreUnitOfWork<E>` — a span in which appends share one fate |
//! | `repository` | `Repository<A>` — save, load, and delete aggregates |
//! | `snapshot` | `Snapshot` — save and load aggregate state snapshots |
//! | `stored_event` | `StoredEvent<E>` — event wrapper with optional correlation_id |

/// Persistence error types.
pub mod error;

/// Event store contract — append and load domain events.
pub mod event_store;

/// Aggregate repository contract — save, load, and delete.
pub mod repository;

/// Aggregate snapshot contract — save and load state snapshots.
pub mod snapshot;

/// Event wrapper with optional correlation_id.
pub mod stored_event;

pub use error::PersistenceError;
pub use event_store::{EventStore, EventStoreUnitOfWork};
pub use repository::Repository;
pub use snapshot::Snapshot;
pub use stored_event::StoredEvent;
