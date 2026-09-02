//! # ego-persistence-api
//!
//! Destination crate for the domain-owned persistence port vocabulary
//! (`EventStore`, `Repository`, `Snapshot`, the read-side projection SPIs,
//! and the operation-reservation ports), relocated out of `ego-domain` so a
//! port has exactly one owning module tree (CORE-PERSIST-A). Populated
//! slice by slice, per design.md AD-6 — S1 (read-side SPIs) has landed;
//! `operation`/`id_type!` (S2) and `persistence`/`event` (S3) are still to
//! move.
//!
//! This crate has no normal/build dependency on another workspace crate.
//! `ego-domain` depends on it and, as each slice lands, re-exports the
//! relocated items at their original path so no consumer outside these two
//! crates observes the move.

/// Read-side projection SPIs — offset tracking, dedup, event fetch, and
/// state storage (S1, design.md AD-6).
pub mod read_side;
