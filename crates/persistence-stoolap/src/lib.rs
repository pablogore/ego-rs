//! Stoolap-backed implementations of `ego_persistence_api::persistence::Repository<A>`
//! and `ego_persistence_api::persistence::Snapshot`.
//!
//! See `openspec/changes/stoolap-s1-repository-adapter/design.md` for `Repository<A>`'s
//! schema, tenant-sentinel encoding, and optimistic-concurrency algorithm, and
//! `openspec/changes/stoolap-s2-bridge-production-durable-profile/design.md` for `Snapshot`'s.

pub mod persistence;

#[cfg(feature = "event-sourcing")]
pub mod event_sourcing;

#[cfg(feature = "operation-reservation")]
pub mod operation;

pub use persistence::repository::StoolapRepository;
pub use persistence::snapshot::StoolapSnapshotStore;

#[cfg(feature = "event-sourcing")]
pub use event_sourcing::event_store::StoolapEventStore;

#[cfg(feature = "operation-reservation")]
pub use operation::reservation::StoolapOperationReservationStore;
