//! # ego-persistence-api
//!
//! Destination crate for the domain-owned persistence port vocabulary
//! (`EventStore`, `Repository`, `Snapshot`, the read-side projection SPIs,
//! and the operation-reservation ports), relocated out of `ego-domain` so a
//! port has exactly one owning module tree (CORE-PERSIST-A). Populated
//! slice by slice, per design.md AD-6 — S1 (read-side SPIs), S2 (operation
//! identity/receipt/reservation, `id_type!`), and S3 (`persistence`,
//! `event`) have all landed.
//!
//! This crate has no normal/build dependency on another workspace crate.
//! `ego-domain` depends on it and, as each slice lands, re-exports the
//! relocated items at their original path so no consumer outside these two
//! crates observes the move.

/// Read-side projection SPIs — offset tracking, dedup, event fetch, and
/// state storage (S1, design.md AD-6).
pub mod read_side;

/// Operation-scoped identity for end-to-end idempotent command processing —
/// `OperationKey`, `OperationFingerprint`, `OperationReceipt`, and the
/// reservation port (S2, design.md AD-6).
pub mod operation;

/// The `id_type!` identity-type generator and `TenantId`/`TenantIdError`
/// (S2, design.md AD-3).
pub mod context;

/// Persistence SPI — `EventStore`, `EventStoreUnitOfWork`, `Repository`,
/// `Snapshot`, `PersistenceError`, `resolve_tenant` (S3, design.md AD-6).
pub mod persistence;

/// Domain event trait for event-sourced state (S3, design.md AD-2).
pub mod event;
