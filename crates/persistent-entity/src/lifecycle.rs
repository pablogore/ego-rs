//! Lifecycle management for persistent entities.
//!
//! This module handles the state transitions of entities through their lifecycle.

use crate::error::EntityError;
use std::fmt::Debug;

/// The possible states of an entity lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityState {
    /// The entity is being recovered.
    Recovering,
    /// The entity is active and processing commands.
    Active,
    /// The entity is passivating.
    Passivating,
    /// The entity is passivated.
    Passivated,
    /// The entity has failed.
    Failed,
}

/// A lifecycle state machine for entities.
#[derive(Debug, Clone)]
pub struct LifecycleStateMachine {
    /// The current state of the entity.
    pub current_state: EntityState,
}

impl LifecycleStateMachine {
    /// Create a new lifecycle state machine.
    pub fn new() -> Self {
        Self {
            current_state: EntityState::Recovering,
        }
    }

    /// Transition to a new state.
    pub fn transition_to(&mut self, state: EntityState) -> Result<(), EntityError> {
        // Basic state transition validation
        match (self.current_state, state) {
            // From Recovering, can go to Active or Failed
            (EntityState::Recovering, EntityState::Active) => {
                self.current_state = state;
                Ok(())
            }
            // From Active, can go to Passivating or Failed
            (EntityState::Active, EntityState::Passivating) => {
                self.current_state = state;
                Ok(())
            }
            // From Passivating, can go to Passivated or Failed
            (EntityState::Passivating, EntityState::Passivated) => {
                self.current_state = state;
                Ok(())
            }
            // From Passivated, can go to Active or Failed
            (EntityState::Passivated, EntityState::Active) => {
                self.current_state = state;
                Ok(())
            }
            // Any state can transition to Failed
            (_, EntityState::Failed) => {
                self.current_state = state;
                Ok(())
            }
            // No other transitions are allowed
            _ => Err(EntityError::Internal(
                format!(
                    "Invalid state transition from {:?} to {:?}",
                    self.current_state, state
                ),
            )),
        }
    }

    /// Check if the entity is active.
    pub fn is_active(&self) -> bool {
        matches!(self.current_state, EntityState::Active)
    }

    /// Check if the entity is passivated.
    pub fn is_passivated(&self) -> bool {
        matches!(self.current_state, EntityState::Passivated)
    }

    /// Check if the entity is in a recoverable state.
    pub fn is_recoverable(&self) -> bool {
        matches!(
            self.current_state,
            EntityState::Recovering | EntityState::Passivated
        )
    }
}