//! Stoolap-backed implementation of `ego_persistence_api::persistence::Repository<A>`.
//!
//! See `openspec/changes/stoolap-s1-repository-adapter/design.md` for the schema,
//! tenant-sentinel encoding, and optimistic-concurrency algorithm this crate implements.

pub mod persistence;

pub use persistence::repository::StoolapRepository;
