//! In-memory persistence backends.
//!
//! Reference implementations of the domain SPI traits using in-memory
//! storage. All backends tenant-isolate data and enforce optimistic
//! concurrency.

mod event_store;
mod read_side_store;
mod repository;
mod snapshot;

pub use event_store::InMemoryEventStore;
pub use read_side_store::{paginate, InMemoryReadSideStore};
pub use repository::InMemoryRepository;
pub use snapshot::InMemorySnapshotStore;
