//! The shared `OperationKey` extraction contract.
//!
//! Extraction is per transport; enforcement is transport-agnostic. See
//! [`extraction`] for the one shared definition of what a valid key is and
//! what happens when one is missing — every transport adapter consumes this
//! module rather than re-implementing either rule.

/// [`extraction::OperationKeyCarrier`], [`extraction::resolve_operation_key`]
/// and [`extraction::OperationKeyRejection`] — the single validation and
/// missing-key policy entry point.
pub mod extraction;

pub use extraction::{
    resolve_operation_key, OperationKeyCarrier, OperationKeyRejection, RawOperationKey,
};
