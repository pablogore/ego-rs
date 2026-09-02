//! Operation-scoped identity for end-to-end idempotent command processing
//! (CORE-PERSIST-A S2). Relocated verbatim from
//! `ego_domain::operation::{key,receipt,reservation}` (D-6); `ego-domain`
//! re-exports each module at its original name and path.
//!
//! `ego_domain::operation::identity` (`OperationIdentity`) is not part of
//! this relocation and stays in `ego-domain`.

pub mod key;
pub mod receipt;
pub mod reservation;
