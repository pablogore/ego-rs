//! External effect delivery subsystem (CORE-019).
//!
//! Beside the dormant [`crate::interpreter::EffectInterpreter`], this module
//! owns reliable delivery of `ExternalEffectDescription` values: dedup,
//! retry/backoff, and an `effect_type`-keyed executor registry. See
//! `openspec/changes/core-019-reliable-external-effects/design.md`.

pub mod executor;
pub mod policy;
pub(crate) mod queue;
pub mod registry;
pub(crate) mod runner;
pub mod store;

pub use executor::{AttemptOutcome, EffectContext, ExternalEffectExecutor};
pub use policy::{DeliveryConfig, RetryPolicy, RunnerMode};
pub use registry::{DuplicateEffectType, ExecutorRegistry};
pub use store::{
    AcceptedEffect, DedupOutcome, DedupScope, EffectDedupStore, EffectId, EffectState,
    EffectStateStore, EffectStoreError, InMemoryEffectStore, StoredEffect, TerminalReason,
    Timestamp,
};
