//! Operation-scoped identity for end-to-end idempotent command processing.
//!
//! `OperationKey` and `OperationFingerprint` identify one complete,
//! client-supplied business operation — distinct from
//! [`crate::idempotency::IdempotencyKey`], which identifies a single
//! external effect dispatch attempt. See [`key`] for the types and their
//! validation.

/// `OperationKey` / `OperationFingerprint` — validated operation identity.
pub mod key;

pub use key::{OperationFingerprint, OperationKey, OperationKeyError};
