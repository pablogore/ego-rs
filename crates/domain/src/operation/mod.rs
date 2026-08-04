//! Operation-scoped identity for end-to-end idempotent command processing.
//!
//! `OperationKey` and `OperationFingerprint` identify one complete,
//! client-supplied business operation — distinct from
//! [`crate::idempotency::IdempotencyKey`], which identifies a single
//! external effect dispatch attempt. See [`key`] for the types and their
//! validation.
//!
//! [`reservation`] defines the port through which one such operation is
//! reserved under a fenced lease before dispatch. This module hosts the
//! trait and its supporting types only; concrete implementations (in-memory
//! test double, durable Postgres-backed store) live outside `ego-domain`,
//! per the hexagonal boundary this crate enforces.

/// `OperationKey` / `OperationFingerprint` — validated operation identity.
pub mod key;

/// `OperationReservationStore` port and its supporting types — lease,
/// fencing, and reservation outcomes.
pub mod reservation;

pub use key::{OperationFingerprint, OperationKey, OperationKeyError};
pub use reservation::{
    FencingToken, Lease, OperationId, OperationReservationStore, OwnerFence, OwnerId,
    ReservationError, ReservationOutcome, ReserveRequest, StoredResponse,
};
