//! Trait defining the interface for persistent entities.
//!
//! This trait must be implemented by all persistent entities in the system.

use crate::command_context::CommandContext;
use crate::effect_acceptor::EffectAcceptanceError;
use crate::error::EntityError;
use async_trait::async_trait;
use ego_domain::operation::AggregateOutcome;
use ego_domain::ExternalEffectDescription;
use serde::{Deserialize, Serialize};
use std::fmt::Debug;

/// Result type for command processing.
#[derive(Debug, Clone)]
pub enum CommandResult<E, S> {
    /// Command produced events and a new state.
    Events {
        /// The new state after applying events.
        new_state: S,
        /// The events produced by the command.
        events: Vec<E>,
    },
    /// Command produced no events but returned a state.
    NoEvents {
        /// The state returned by the command.
        state: S,
    },
    /// The command was **not executed**: a receipt showed this exact operation
    /// already completed against this aggregate.
    ///
    /// # What this asserts, and what it deliberately does not
    ///
    /// It asserts exactly one thing: *this aggregate's transition was already
    /// confirmed; it must not run again and must not produce effects again.*
    ///
    /// It carries **no state and no events**, and that is a decision rather than
    /// an omission. A receipt records that a transition happened; it cannot
    /// reproduce the answer the aggregate once gave. Returning the aggregate's
    /// *current* state here would look helpful and break idempotency silently:
    /// if this operation left the aggregate at version 10 and later commands
    /// took it to 14, one operation key would produce two different logical
    /// results.
    ///
    /// # For callers
    ///
    /// Treat this as recovery of a partially executed operation. Do not persist,
    /// publish, or accept effects from it. If the workflow needs data to
    /// continue, read current state explicitly — and do not present that reading
    /// as the command's historical answer. If it cannot continue safely, fail
    /// explicitly rather than re-running the command.
    Replayed {
        /// Durable evidence of the confirmed transition: an event range, or
        /// `NoEvents` for a success that appended nothing.
        outcome: AggregateOutcome,
    },
    /// Command's events committed successfully, but at least one described
    /// external effect could not be durably-enough accepted (AD-9).
    ///
    /// **This is NOT a command failure.** The commit is final and was never
    /// rolled back. Collapsing this into `Err(EntityError)` would make it
    /// indistinguishable from a real command failure, and a caller that
    /// retries an indistinguishable `Err` would re-execute an
    /// already-committed command (a duplicate side effect). Callers MUST
    /// treat this variant as a successful commit with a post-commit warning,
    /// never as grounds for retrying the command itself.
    EffectsAcceptanceFailed {
        /// The new state after applying the committed events.
        new_state: S,
        /// The events that were committed.
        events: Vec<E>,
        /// Why post-commit effect acceptance ultimately failed.
        error: EffectAcceptanceError,
    },
}

/// Trait that all persistent entities must implement.
///
/// This trait defines the core behavior of persistent entities, including
/// command handling and event application.
#[async_trait]
pub trait PersistentEntity: Send + Sync + Debug {
    /// The type of commands this entity can handle.
    type Command: Serialize + Send + Sync + 'static;

    /// The type of events this entity produces.
    type Event: Serialize + Send + Sync + 'static;

    /// The type of state this entity maintains.
    type State: Serialize + for<'de> Deserialize<'de> + Send + Sync + 'static;

    /// Get the initial state for this entity.
    ///
    /// # Returns
    /// * `Self::State` - The initial state
    fn initial_state(&self) -> Self::State;

    /// Handle a command and produce events.
    ///
    /// # Arguments
    /// * `command` - The command to process
    /// * `state` - The current state of the entity
    /// * `context` - The command context
    ///
    /// # Returns
    /// * `Result<Vec<Self::Event>, EntityError>` - The events produced by the command
    async fn handle_command(
        &self,
        command: &Self::Command,
        state: &Self::State,
        context: &CommandContext,
    ) -> Result<Vec<Self::Event>, EntityError>;

    /// Apply a single event to update the entity state.
    ///
    /// # Arguments
    /// * `state` - The current state of the entity
    /// * `event` - The event to apply
    ///
    /// # Returns
    /// * `Result<Self::State, EntityError>` - The updated state
    async fn apply_event(
        &self,
        state: &Self::State,
        event: &Self::Event,
    ) -> Result<Self::State, EntityError>;

    /// Apply events to update the entity state.
    ///
    /// # Arguments
    /// * `state` - The current state of the entity
    /// * `events` - The events to apply
    ///
    /// # Returns
    /// * `Result<Self::State, EntityError>` - The updated state
    async fn apply_events(
        &self,
        state: &Self::State,
        events: &[Self::Event],
    ) -> Result<Self::State, EntityError>;

    /// Describes zero or more external effects to dispatch after this
    /// command's events have already committed (AD-1, AD-2).
    ///
    /// Receives the committed `new_state`/`events` — not the pre-command
    /// state — so effects are derived from what actually persisted. Defaults
    /// to no effects, so every pre-CORE-019 handler compiles unchanged and
    /// pays no cost unless it opts in.
    ///
    /// # Arguments
    /// * `command` - The command that was handled.
    /// * `new_state` - The state after applying the committed events.
    /// * `events` - The events that were just committed.
    /// * `context` - The command context.
    ///
    /// # Returns
    /// * `Vec<ExternalEffectDescription>` - Effects to accept post-commit; empty by default.
    async fn external_effects(
        &self,
        _command: &Self::Command,
        _new_state: &Self::State,
        _events: &[Self::Event],
        _context: &CommandContext,
    ) -> Vec<ExternalEffectDescription> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> CommandContext {
        CommandContext {
            tenant_id: Some("test-tenant".to_string()),
            entity_type: "unit-test".to_string(),
            entity_id: "unit-test-id".to_string(),
            expected_version: None,
            causation_id: None,
            metadata: std::collections::HashMap::new(),
            operation_key: None,
            fingerprint: None,
        }
    }

    /// AD-2 / spec "Backward Compatibility": a handler that never overrides
    /// `external_effects` must still compile and describe no effects. Every
    /// method here matches the pre-CORE-019 shape exactly — proving this
    /// handler is unmodified, not merely omitting the new method.
    #[derive(Debug)]
    struct UnmodifiedHandler;

    #[async_trait]
    impl PersistentEntity for UnmodifiedHandler {
        type Command = ();
        type Event = ();
        type State = ();

        fn initial_state(&self) {}

        async fn handle_command(
            &self,
            _command: &(),
            _state: &(),
            _context: &CommandContext,
        ) -> Result<Vec<()>, EntityError> {
            Ok(vec![])
        }

        async fn apply_event(&self, _state: &(), _event: &()) -> Result<(), EntityError> {
            Ok(())
        }

        async fn apply_events(&self, _state: &(), _events: &[()]) -> Result<(), EntityError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn unmodified_handler_compiles_and_describes_no_external_effects_by_default() {
        let handler = UnmodifiedHandler;

        let effects = handler.external_effects(&(), &(), &[], &ctx()).await;

        assert!(
            effects.is_empty(),
            "the default external_effects body must describe zero effects"
        );
    }
}
