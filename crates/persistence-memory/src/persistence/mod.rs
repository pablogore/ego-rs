//! In-memory implementations of `ego-persistence-api`'s `persistence` ports —
//! `EventStore`, `Repository`, `Snapshot` — relocated verbatim from
//! `ego-infrastructure` (design.md AD-3, AD-4).

pub mod event_store;
pub mod repository;
pub mod snapshot;
