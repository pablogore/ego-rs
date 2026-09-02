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

/// `OperationIdentity` — the two halves above, carried as one indivisible value.
pub mod identity;

// CORE-PERSIST-A S2 (AD-4): `key`, `receipt`, and `reservation` relocated to
// `ego-persistence-api`, re-exported here at module granularity so every old
// `ego_domain::operation::*` path keeps resolving to the identical item.
pub use ego_persistence_api::operation::{key, receipt, reservation};

pub use identity::OperationIdentity;
pub use key::{OperationFingerprint, OperationKey, OperationKeyError, OperationKeyHash};
pub use receipt::{AggregateOutcome, AggregateOutcomeError, OperationReceipt};
pub use reservation::{
    FencingToken, Lease, OldestCompleted, OperationId, OperationReservationStore, OwnerFence,
    OwnerId, ReservationError, ReservationOutcome, ReserveRequest, StoredServiceResponse,
};
