//! # runtime-slice
//!
//! Minimal deterministic runtime slice for ego-rs. Provides core types for
//! governed, deterministic execution.
//!
//! ## Types
//!
//! | Type | Purpose |
//! |------|---------|
//! | [`RuntimeSliceId`] | Unique identifier for a runtime slice |
//! | [`DeterministicInput`] | A governed input (key-value pair) |
//! | [`ExecutionContext`] | The context in which a unit of work executes |
//! | [`ExecutionOutcome`] | The observable result of execution |
//! | [`RuntimeSliceError`] | Fail-closed error variants |
//!
//! ## Determinism
//!
//! All types enforce the **Determinism Axiom**: given identical inputs,
//! execution produces identical observable semantics. Fail-closed behavior
//! is enforced — ambiguous states produce rejection, not continuation.
//!
//! ## Architecture note
//!
//! This crate is **not yet a workspace member**. CORE-001 will integrate
//! it into the workspace and implement runtime behavior.

pub mod types;

pub use types::{
    DeterministicInput, ExecutionContext, ExecutionOutcome, RuntimeSliceError, RuntimeSliceId,
};