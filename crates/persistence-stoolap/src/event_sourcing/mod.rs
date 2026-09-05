//! Stoolap-backed `EventStore<E>`, gated behind the `event-sourcing` feature
//! (design.md AD-1) so `Repository<A>`/`Snapshot` consumers of this crate
//! gain no async runtime dependency.

pub mod event_store;
