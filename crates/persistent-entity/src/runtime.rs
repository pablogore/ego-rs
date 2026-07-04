//! Runtime orchestration for the persistent entity system.
//!
//! Provides the top-level [`EntityRuntime`] that wires together scheduling,
//! persistence, publishing, and snapshot management.

use std::marker::PhantomData;
use std::sync::Arc;

use ego_domain::event::DomainEvent;

use crate::entity_ref::EntityRef;
use crate::entity_ref_tokio::TokioEntityRef;
use crate::persistence::PersistenceFacade;
use crate::persistent_entity::PersistentEntity;
use crate::publisher::EventPublisher;
use crate::registry::EntityRegistry;
use crate::scheduler::{EntityTriple, Scheduler};
use crate::scheduler_event::SchedulerEventSender;
use crate::snapshot::SnapshotStrategy;

/// Configuration for the entity runtime.
///
/// Controls mailbox capacity, concurrency budget, passivation timeout,
/// and tenant isolation settings.
#[derive(serde::Deserialize)]
pub struct RuntimeConfig {
    /// Maximum number of commands queued per mailbox.
    pub mailbox_capacity: usize,
    /// Maximum number of concurrently active actors.
    pub concurrency_budget: usize,
    /// Seconds of inactivity before entity passivation.
    pub passivation_timeout_secs: u64,
    /// When true, all entities share the default tenant scope.
    pub single_tenant_mode: bool,
    /// Tenant identifier used when single_tenant_mode is false.
    pub tenant_id: String,
}

impl RuntimeConfig {
    /// Returns the passivation timeout as a [`std::time::Duration`].
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
    _event: PhantomData<E>,
}

impl<E> EntityRuntime<E>
where
    E: DomainEvent + Clone + serde::de::DeserializeOwned + serde::Serialize + Send + Sync + 'static,
{
    /// Creates a new [`EntityRuntime`] with the given components.
    pub fn new(
        registry: Arc<EntityRegistry>,
        scheduler: Arc<Scheduler>,
        persistence: Arc<PersistenceFacade<E>>,
        publisher: Arc<dyn EventPublisher<E>>,
        config: RuntimeConfig,
        snapshot_strategy: Arc<dyn SnapshotStrategy>,
        event_sender: SchedulerEventSender,
    ) -> Self {
        EntityRuntime {
            registry,
            scheduler,
            persistence,
            publisher,
            config,
            snapshot_strategy,
            event_sender,
            _event: PhantomData,
        }
    }

    /// Returns a [`TokioEntityRef`] for sending commands to the identified entity.
    ///
    /// Spawns a real [`EntityActor`] via `tokio::spawn` and returns a ref
    /// backed by its mailbox.  This method MUST be called from within a Tokio
    /// runtime context (e.g., inside an `async fn` or `#[tokio::test]`).
    pub fn entity_ref<C, S>(
        &self,
        entity_type: &'static str,
        entity_id: impl Into<String>,
        entity_handler: Arc<dyn PersistentEntity<Command = C, Event = E, State = S>>,
    ) -> impl EntityRef<Command = C>
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
            self.config.passivation_timeout(),
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
