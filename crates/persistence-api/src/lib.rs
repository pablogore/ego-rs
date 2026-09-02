//! # ego-persistence-api
//!
//! Destination crate for the domain-owned persistence port vocabulary
//! (`EventStore`, `Repository`, `Snapshot`, the read-side projection SPIs,
//! and the operation-reservation ports), relocated out of `ego-domain` so a
//! port has exactly one owning module tree (CORE-PERSIST-A). Populated
//! slice by slice — this is the crate skeleton; no port has moved yet (S1
//! lands the read-side SPIs first, per design.md AD-6).
//!
//! This crate depends on **no workspace crate**. `ego-domain` depends on it
//! and, as each slice lands, re-exports the relocated items at their
//! original path so no consumer outside these two crates observes the move.
