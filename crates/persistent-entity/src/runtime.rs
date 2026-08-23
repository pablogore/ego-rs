//! Runtime orchestration for the persistent entity system.
//!
//! Provides the top-level [`EntityRuntime`] that wires together scheduling,
//! persistence, publishing, and snapshot management.

use std::marker::PhantomData;
use std::sync::Arc;

use ego_domain::event::DomainEvent;

use crate::effect_acceptor::EffectAcceptor;
use crate::entity_ref::EntityRef;
use crate::entity_ref_tokio::TokioEntityRef;
use crate::persistence::PersistenceFacade;
use crate::persistent_entity::PersistentEntity;
use crate::publisher::EventPublisher;
use crate::registry::EntityRegistry;
use crate::scheduler::{EntityTriple, Scheduler};
use crate::scheduler_event::SchedulerEventSender;
use crate::snapshot::SnapshotStrategy;
use ego_domain::Observability;

/// Configuration for the entity runtime.
///
/// Controls mailbox capacity, concurrency budget, passivation timeout,
/// and tenant isolation settings.
///
/// **`passivation_timeout_secs` is a whole-seconds JSON/kit-config-facing
/// value, not necessarily what the runtime actually uses.** When a
/// sub-second [`crate::builder::EntityRuntimeBuilder::passivation_timeout`]
/// is configured directly (not via JSON), this field is populated by
/// rounding *up* to the nearest whole second purely for this struct's own
/// informational/serializable representation — it is never truncated down
/// to `0`, which would misleadingly read as "passivates instantly." For the
/// exact `Duration` actors are actually spawned with, use
/// [`EntityRuntime::passivation_timeout`], not this field or
/// [`Self::passivation_timeout`].
#[derive(serde::Deserialize)]
pub struct RuntimeConfig {
    /// Maximum number of commands queued per mailbox.
    pub mailbox_capacity: usize,
    /// Maximum number of concurrently active actors.
    pub concurrency_budget: usize,
    /// Seconds of inactivity before entity passivation — rounded up from any
    /// sub-second value (see this struct's doc comment); not the ground truth.
    pub passivation_timeout_secs: u64,
    /// When true, all entities share the default tenant scope.
    pub single_tenant_mode: bool,
    /// Tenant identifier used when single_tenant_mode is false.
    pub tenant_id: String,
}

impl RuntimeConfig {
    /// Returns `passivation_timeout_secs` as a [`std::time::Duration`] —
    /// this struct's own whole-seconds approximation (see its doc comment),
    /// not necessarily the exact value actors are spawned with. Prefer
    /// [`EntityRuntime::passivation_timeout`] for that.
    pub fn passivation_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.passivation_timeout_secs)
    }
}

impl ego_domain::Validate for RuntimeConfig {
    fn validate(&self) -> Result<(), ego_domain::ConfigError> {
        if self.mailbox_capacity == 0 {
            return Err(ego_domain::ConfigError::non_zero("mailbox_capacity"));
        }
        if self.concurrency_budget == 0 {
            return Err(ego_domain::ConfigError::non_zero("concurrency_budget"));
        }
        if !self.single_tenant_mode && self.tenant_id.trim().is_empty() {
            return Err(ego_domain::ConfigError::Invalid {
                field: "tenant_id".to_string(),
                reason: "must be non-empty when single_tenant_mode is false".to_string(),
            });
        }
        Ok(())
    }
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        RuntimeConfig {
            mailbox_capacity: 1000,
            concurrency_budget: 10000,
            passivation_timeout_secs: 300,
            single_tenant_mode: true,
            tenant_id: String::new(),
        }
    }
}

/// Top-level runtime that orchestrates entity lifecycle, scheduling, and persistence.
///
/// Owns shared references to registry, persistence, scheduler, publisher,
/// and snapshot strategy. Creates entity references for command dispatch.
pub struct EntityRuntime<E> {
    /// Shared entity registry.
    pub registry: Arc<EntityRegistry>,
    /// Shared scheduler for activation suggestions.
    pub scheduler: Arc<Scheduler>,
    /// Shared persistence facade.
    pub persistence: Arc<PersistenceFacade<E>>,
    /// Shared event publisher.
    pub publisher: Arc<dyn EventPublisher<E>>,
    /// Runtime configuration.
    pub config: RuntimeConfig,
    /// Snapshot strategy.
    pub snapshot_strategy: Arc<dyn SnapshotStrategy>,
    /// Scheduler event sender for lifecycle events.
    pub event_sender: SchedulerEventSender,
    /// Post-commit external-effect acceptance port (CORE-019 PR4 F-03 fix).
    /// `None` by default — a host that never calls
    /// [`crate::builder::EntityRuntimeBuilder::with_effect_acceptor`] keeps
    /// every spawned actor's `effect_acceptor` at `None`, preserving today's
    /// fail-closed-if-effects-described behavior unchanged.
    pub effect_acceptor: Option<Arc<dyn EffectAcceptor>>,
    /// Observability sink threaded to every actor spawned via [`Self::entity_ref`].
    /// `None` by default — an actor without one behaves exactly as it did before
    /// the receipt gate was instrumented.
    pub observability: Option<Arc<dyn Observability>>,
    /// Full-precision passivation timeout, as configured directly via
    /// [`crate::builder::EntityRuntimeBuilder::passivation_timeout`] — kept
    /// separate from `config.passivation_timeout_secs` (whole seconds only,
    /// intentional for that JSON/kit-config-facing schema) because rounding a
    /// sub-second `Duration` through `.as_secs()` silently truncates it to
    /// zero, making every spawned actor passivate almost immediately instead
    /// of after the configured idle period. This field is what
    /// [`Self::entity_ref`] actually uses.
    passivation_timeout: std::time::Duration,
    _event: PhantomData<E>,
}

impl<E> EntityRuntime<E>
where
    E: DomainEvent + Clone + serde::de::DeserializeOwned + serde::Serialize + Send + Sync + 'static,
{
    /// Creates a new [`EntityRuntime`] with the given components.
    ///
    /// Preserved for source compatibility (review F1) — any existing caller
    /// of this 8-argument signature keeps compiling. It reconstructs
    /// `passivation_timeout` from `config.passivation_timeout_secs` the same
    /// (lossy, sub-second-truncating) way this crate always did before this
    /// fix; only [`crate::builder::EntityRuntimeBuilder`] — the sole
    /// in-repo caller — was updated to call
    /// [`Self::new_with_passivation_timeout`] instead, which is what
    /// actually carries the fix. A caller who wants the fix directly should
    /// migrate to that constructor.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        registry: Arc<EntityRegistry>,
        scheduler: Arc<Scheduler>,
        persistence: Arc<PersistenceFacade<E>>,
        publisher: Arc<dyn EventPublisher<E>>,
        config: RuntimeConfig,
        snapshot_strategy: Arc<dyn SnapshotStrategy>,
        event_sender: SchedulerEventSender,
        effect_acceptor: Option<Arc<dyn EffectAcceptor>>,
    ) -> Self {
        let passivation_timeout = config.passivation_timeout();
        Self::new_with_passivation_timeout(
            registry,
            scheduler,
            persistence,
            publisher,
            config,
            snapshot_strategy,
            event_sender,
            effect_acceptor,
            passivation_timeout,
        )
    }

    /// Creates a new [`EntityRuntime`], accepting the full-precision idle
    /// `Duration` actually used when spawning actors directly (review F1) —
    /// see [`EntityRuntime`]'s `passivation_timeout` field doc comment for
    /// why this is threaded separately from `config.passivation_timeout_secs`.
    ///
    /// `pub(crate)`, not `pub` (review PR #186 finding 1): nothing enforces
    /// that a caller's `config.passivation_timeout_secs` and its separate
    /// `passivation_timeout: Duration` argument actually agree — an external
    /// caller could construct a runtime whose reported config timeout and
    /// actual actor behavior silently contradict each other.
    /// [`crate::builder::EntityRuntimeBuilder`] (the sole caller, same
    /// crate) is what keeps the two in sync via `ceil_secs`; it is the only
    /// sanctioned way to reach this constructor.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new_with_passivation_timeout(
        registry: Arc<EntityRegistry>,
        scheduler: Arc<Scheduler>,
        persistence: Arc<PersistenceFacade<E>>,
        publisher: Arc<dyn EventPublisher<E>>,
        config: RuntimeConfig,
        snapshot_strategy: Arc<dyn SnapshotStrategy>,
        event_sender: SchedulerEventSender,
        effect_acceptor: Option<Arc<dyn EffectAcceptor>>,
        passivation_timeout: std::time::Duration,
    ) -> Self {
        EntityRuntime {
            registry,
            scheduler,
            persistence,
            publisher,
            config,
            snapshot_strategy,
            event_sender,
            effect_acceptor,
            observability: None,
            passivation_timeout,
            _event: PhantomData,
        }
    }

    /// Wires a post-commit external-effect acceptor (CORE-019 Phase 12) so
    /// every actor spawned via [`Self::entity_ref`] from now on actually
    /// delivers effects a handler's `external_effects` describes, instead of
    /// silently dropping them. Purely additive — a runtime that never calls
    /// this keeps behaving exactly as before this method existed.
    pub fn with_effect_acceptor(mut self, acceptor: Arc<dyn EffectAcceptor>) -> Self {
        self.effect_acceptor = Some(acceptor);
        self
    }

    /// Wires an observability sink so every actor spawned from now on reports
    /// what its receipt gate decided. Purely additive: a runtime that never
    /// calls this keeps behaving exactly as before this method existed.
    ///
    /// Consumes `self`, so it must be called **before** the runtime is wrapped in
    /// the `Arc` a host registers — there is no way to add a sink afterwards. See
    /// [`crate::builder::EntityRuntimeBuilder::with_observability`] for the wiring
    /// order a host actually follows.
    pub fn with_observability(mut self, observability: Arc<dyn Observability>) -> Self {
        self.observability = Some(observability);
        self
    }

    /// Returns a [`TokioEntityRef`] for sending commands to the identified entity.
    ///
    /// Spawns a real [`EntityActor`] via `tokio::spawn` and returns a ref
    /// backed by its mailbox.  This method MUST be called from within a Tokio
    /// runtime context (e.g., inside an `async fn` or `#[tokio::test]`).
    ///
    /// Returns `Err(EntityError::Internal(..))` if a live registry entry
    /// exists for this triple but its erased mailbox does not downcast to
    /// `BoundedMailbox<ActorEnvelope<C>>` (ADR-002) — a programming error
    /// (mismatched `entity_type`/command type), never treated as "no live
    /// entry" and never a fallback spawn.
    pub fn entity_ref<C, S>(
        &self,
        entity_type: &'static str,
        entity_id: impl Into<String>,
        entity_handler: Arc<dyn PersistentEntity<Command = C, Event = E, State = S>>,
    ) -> Result<impl EntityRef<Command = C>, crate::error::EntityError>
    where
        C: Send + Sync + serde::Serialize + 'static,
        S: serde::Serialize + Clone + serde::de::DeserializeOwned + Send + Sync + 'static,
    {
        let tenant_id = if self.config.single_tenant_mode {
            "default".to_string()
        } else {
            self.config.tenant_id.clone()
        };

        let triple = EntityTriple::new(tenant_id, entity_type, entity_id);

        TokioEntityRef::new(
            triple,
            self.registry.clone(),
            self.persistence.clone(),
            self.publisher.clone(),
            self.snapshot_strategy.clone(),
            entity_handler,
            self.event_sender.clone(),
            self.config.mailbox_capacity,
            self.passivation_timeout,
            self.effect_acceptor.clone(),
            self.observability.clone(),
        )
    }

    /// Returns the number of currently active entities.
    pub fn active_count(&self) -> usize {
        self.registry.active_count()
    }

    /// Returns the number of passivated entities.
    pub fn passivated_count(&self) -> usize {
        self.registry.passivated_count()
    }

    /// Returns the exact, full-precision idle duration actors spawned by
    /// [`Self::entity_ref`] are actually configured with — the ground truth,
    /// unlike `self.config.passivation_timeout()` (whole seconds only,
    /// rounded up, purely informational; see [`RuntimeConfig`]'s doc
    /// comment). Introspection code (monitoring, logging) should read this,
    /// not `config`.
    pub fn passivation_timeout(&self) -> std::time::Duration {
        self.passivation_timeout
    }
}

#[cfg(test)]
mod runtime_config_validate_tests {
    use super::*;
    use ego_domain::{ConfigError, Validate};

    #[test]
    fn default_config_is_valid() {
        assert!(RuntimeConfig::default().validate().is_ok());
    }

    #[test]
    fn zero_mailbox_capacity_is_invalid() {
        let config = RuntimeConfig {
            mailbox_capacity: 0,
            ..RuntimeConfig::default()
        };
        assert_eq!(
            config.validate(),
            Err(ConfigError::Invalid {
                field: "mailbox_capacity".to_string(),
                reason: "must be non-zero".to_string(),
            })
        );
    }

    #[test]
    fn zero_concurrency_budget_is_invalid() {
        let config = RuntimeConfig {
            concurrency_budget: 0,
            ..RuntimeConfig::default()
        };
        assert_eq!(
            config.validate(),
            Err(ConfigError::Invalid {
                field: "concurrency_budget".to_string(),
                reason: "must be non-zero".to_string(),
            })
        );
    }

    #[test]
    fn multi_tenant_mode_requires_non_empty_tenant_id() {
        let config = RuntimeConfig {
            single_tenant_mode: false,
            tenant_id: String::new(),
            ..RuntimeConfig::default()
        };
        assert_eq!(
            config.validate(),
            Err(ConfigError::Invalid {
                field: "tenant_id".to_string(),
                reason: "must be non-empty when single_tenant_mode is false".to_string(),
            })
        );
    }

    #[test]
    fn multi_tenant_mode_with_tenant_id_is_valid() {
        let config = RuntimeConfig {
            single_tenant_mode: false,
            tenant_id: "tenant-a".to_string(),
            ..RuntimeConfig::default()
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn multi_tenant_mode_with_whitespace_only_tenant_id_is_invalid() {
        let config = RuntimeConfig {
            single_tenant_mode: false,
            tenant_id: "   ".to_string(),
            ..RuntimeConfig::default()
        };
        assert!(config.validate().is_err());
    }
}
