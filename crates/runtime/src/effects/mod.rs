//! External effect delivery subsystem (CORE-019).
//!
//! Beside the dormant [`crate::interpreter::EffectInterpreter`], this module
//! owns reliable delivery of `ExternalEffectDescription` values: dedup,
//! retry/backoff, and an `effect_type`-keyed executor registry. See
//! `openspec/changes/core-019-reliable-external-effects/design.md`.
//!
//! # Subsystem summary
//!
//! A `PersistentEntity` handler describes zero or more effects
//! post-commit (`external_effects`); the actor hands them to a
//! [`crate::effects::acceptor::RuntimeEffectAcceptor`] (the runtime-side
//! implementation of `persistent_entity`'s `EffectAcceptor` port), which
//! mints a [`crate::effects::store::EffectId`], records the effect via
//! [`crate::effects::store::EffectStateStore`], and enqueues it for the
//! internal `runner` module's delivery runner. The runner dedups via
//! [`crate::effects::store::EffectDedupStore`] (scoped to `(tenant,
//! effect_type, key)`), dispatches through the caller-registered
//! [`crate::effects::executor::ExternalEffectExecutor`], and retries
//! transient failures under [`crate::effects::policy::RetryPolicy`]'s
//! bounded backoff (AD-5). [`crate::effects::store::InMemoryEffectStore`] is
//! the non-durable convenience implementation shipped in this crate: it
//! loses every `Pending`/`InFlight` effect on process crash (see its own
//! doc comment). Durable `EffectStateStore`/`EffectDedupStore` providers
//! (PostgreSQL, Stoolap) ship in the sibling `ego-effect-store` crate
//! (PROD-002), implementing the same two ports from outside this crate.
//!
//! Delivery order (design.md §7): [`crate::effects::policy::DeliveryConfig::default`]
//! runs the `Deferred` runner mode (a spawned background loop);
//! [`crate::effects::policy::RunnerMode::Inline`] traverses the exact same
//! pipeline synchronously on the caller's task — never a bypass.
//! `ego-service-sdk`'s `RuntimeBuilder::register_effect_executor` is the
//! host-facing entry point; `persistent_entity`'s
//! `EntityRuntimeBuilder::with_effect_acceptor` is the seam that threads a
//! built acceptor into every actor a host spawns (CORE-019 Phase 12 closes
//! this: `examples/reference-app/tests/effects_e2e.rs` proves the full
//! describe → deliver → retry → dedup path through a real spawned actor).
//!
//! # Deferred: migrating `EventPublisher.publish`
//!
//! The actor's existing fire-and-forget `publisher.publish(&events)` call
//! (`persistent_entity::actor`, right before this subsystem's own
//! `external_effects` seam) is a second, independently unreliable post-commit
//! channel. Routing it through this subsystem instead was considered and
//! explicitly deferred to the roadmap — see
//! `openspec/changes/core-019-reliable-external-effects/proposal.md` §22,
//! Open Question 3 ("Deferred — see Design §6.3: EventPublisher migration out
//! of scope for this slice").

pub mod acceptor;
pub mod executor;
// PROD-002 Phase 4 (AD-14): `log_cleanup_deleted` is called from durable
// provider crates (`ego-effect-store`), so the module can no longer stay
// crate-private — the other signals here remain wired only from within
// ego-runtime (Phase 5, Postgres) and stay `pub(crate)` at the fn level.
pub mod observability;
pub mod policy;
pub(crate) mod queue;
pub mod registry;
pub(crate) mod runner;
pub mod store;

pub use acceptor::RuntimeEffectAcceptor;
pub use executor::{AttemptOutcome, EffectContext, ExternalEffectExecutor};
pub use policy::{DeliveryConfig, RetryPolicies, RetryPolicy, RunnerMode};
pub use registry::{DuplicateEffectType, ExecutorRegistry};
pub use store::{
    AcceptedEffect, DedupOutcome, DedupScope, EffectDedupStore, EffectFingerprint, EffectId,
    EffectState, EffectStateStore, EffectStoreCapabilities, EffectStoreError, InMemoryEffectStore,
    RetentionMaintenance, StoredEffect, TerminalReason, Timestamp,
};
