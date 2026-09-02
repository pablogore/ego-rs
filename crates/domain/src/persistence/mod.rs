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
//! | `stored_event` | `StoredEvent<E>` — event wrapper carrying an optional operation key |

// CORE-PERSIST-A S3 (AD-4): relocated to `ego-persistence-api`, re-exported
// here at module granularity so every old `ego_domain::persistence::*` path
// keeps resolving to the identical item.
pub use ego_persistence_api::persistence::{
    error, event_store, repository, snapshot, stored_event, tenant,
};

pub use error::PersistenceError;
pub use event_store::{EventStore, EventStoreUnitOfWork};
pub use repository::Repository;
pub use snapshot::Snapshot;
pub use stored_event::StoredEvent;
pub use tenant::resolve_tenant;
