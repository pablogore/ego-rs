//! Stoolap-backed implementations of the framework's read-side ports
//! (`ego_persistence_api::read_side`): `OffsetStore`, `DedupStore`, and
//! `ReadSideClaimStore`.
//!
//! # Concurrency scope
//!
//! Every store here is safe for concurrent use by multiple async tasks
//! within **one** ego-rs process holding the underlying Stoolap file.
//! Nothing in this module is tested, documented, or claimed to be safe when
//! the same file is shared by more than one OS process, let alone more than
//! one node — see `openspec/changes/stoolap-rs-01-durable-read-side-stores/design.md`
//! "Concurrency Scope".
//!
//! PR1 (this module, as first landed) ships only `offset`. `dedup` and
//! `claim` land in later PRs of the same change.

pub mod offset;
