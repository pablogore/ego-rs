//! In-memory persistence backends.
//!
//! Reference implementations of the domain SPI traits using in-memory
//! storage. All backends tenant-isolate data and enforce optimistic
//! concurrency.

pub use ego_persistence_memory::persistence::event_store::InMemoryEventStore;
pub use ego_persistence_memory::persistence::repository::InMemoryRepository;
pub use ego_persistence_memory::persistence::snapshot::InMemorySnapshotStore;
pub use ego_persistence_memory::read_side::store::{paginate, InMemoryReadSideStore};
