//! External effect delivery subsystem (CORE-019).
//!
//! Beside the dormant [`crate::interpreter::EffectInterpreter`], this module
//! owns reliable delivery of `ExternalEffectDescription` values: dedup,
//! retry/backoff, and an `effect_type`-keyed executor registry. See
//! `openspec/changes/core-019-reliable-external-effects/design.md`.

pub mod executor;
pub mod registry;
pub mod store;

pub use executor::{AttemptOutcome, EffectContext, ExternalEffectExecutor};
pub use registry::{DuplicateEffectType, ExecutorRegistry};
pub use store::{
    DedupOutcome, DedupScope, EffectDedupStore, EffectId, EffectState, EffectStateStore,
    EffectStoreError, InMemoryEffectStore, TerminalReason,
};
