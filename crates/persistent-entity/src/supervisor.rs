//! Entity supervision and failure handling.
//!
//! Provides the [`Supervisor`] that handles actor and recovery failures
//! by logging errors and removing failed entities from the active registry.

use crate::error::EntityError;
use crate::registry::EntityRegistry;
use crate::scheduler::EntityTriple;
use std::sync::Arc;

/// Handles entity actor and recovery failures.
///
/// On failure, logs the error and removes the entity from the active registry
/// so it can be reactivated on the next command.
pub struct Supervisor {
    registry: Arc<EntityRegistry>,
}

impl Supervisor {
    /// Creates a new [`Supervisor`] backed by the given registry.
    pub fn new(registry: Arc<EntityRegistry>) -> Self {
        Supervisor { registry }
    }

    /// Called when an actor fails during command processing.
    ///
    /// Logs the error and removes the entity from the active registry.
    pub async fn on_actor_failure(
        &self,
        entity: &EntityTriple,
        error: &EntityError,
    ) {
        log::error!(
            "Actor failed for entity {}: {}",
            entity.aggregate_id(),
            error
        );
        self.registry.remove_active(&entity.aggregate_id()).await;
    }

    /// Called when entity recovery fails.
    ///
    /// Logs the error and removes the entity from the active registry.
    pub async fn on_recovery_failure(
        &self,
        entity: &EntityTriple,
        error: &EntityError,
    ) {
        log::error!(
            "Recovery failed for entity {}: {}",
            entity.aggregate_id(),
            error
        );
        self.registry.remove_active(&entity.aggregate_id()).await;
    }
}
