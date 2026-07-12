//! `User` `PersistentEntity` (design.md AD-6).
//!
//! Satisfies reference-service spec "Registering a user".

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ego_domain::DomainEvent;
use persistent_entity::command_context::CommandContext;
use persistent_entity::error::EntityError;
use persistent_entity::persistent_entity::PersistentEntity;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Commands accepted by [`UserEntity`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum UserCommand {
    /// Register a new user identity within a tenant.
    Register {
        user_id: String,
        email: String,
        tenant_id: String,
    },
}

/// Emitted when a `Register` command succeeds.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UserRegistered {
    pub user_id: String,
    pub email: String,
    pub tenant_id: String,
    pub occurred_at: DateTime<Utc>,
    payload: Value,
}

impl UserRegistered {
    fn new(user_id: String, email: String, tenant_id: String, occurred_at: DateTime<Utc>) -> Self {
        let payload = serde_json::json!({
            "user_id": user_id,
            "email": email,
            "tenant_id": tenant_id,
        });
        Self {
            user_id,
            email,
            tenant_id,
            occurred_at,
            payload,
        }
    }
}

impl DomainEvent for UserRegistered {
    fn aggregate_id(&self) -> &str {
        &self.user_id
    }

    fn event_type(&self) -> &str {
        "UserRegistered"
    }

    fn payload(&self) -> &Value {
        &self.payload
    }

    fn occurred_at(&self) -> &DateTime<Utc> {
        &self.occurred_at
    }
}

/// State of a `User` aggregate.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum UserState {
    Unregistered,
    Registered { email: String, tenant_id: String },
}

/// The `User` aggregate (design.md AD-6).
#[derive(Debug, Clone)]
pub struct UserEntity;

impl UserEntity {
    pub fn new() -> Self {
        Self
    }
}

impl Default for UserEntity {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PersistentEntity for UserEntity {
    type Command = UserCommand;
    type Event = UserRegistered;
    type State = UserState;

    fn initial_state(&self) -> Self::State {
        UserState::Unregistered
    }

    async fn handle_command(
        &self,
        command: &Self::Command,
        _state: &Self::State,
        _context: &CommandContext,
    ) -> Result<Vec<Self::Event>, EntityError> {
        match command {
            UserCommand::Register {
                user_id,
                email,
                tenant_id,
            } => {
                // Real validation, not a test-only hook — also gives
                // register_user_partial_failure.rs (CORE-018 AD-5) a
                // deterministic trigger for a User-write failure after a
                // TenantOrganization-write success.
                if email.trim().is_empty() {
                    return Err(EntityError::Internal("email must not be empty".to_string()));
                }
                Ok(vec![UserRegistered::new(
                    user_id.clone(),
                    email.clone(),
                    tenant_id.clone(),
                    Utc::now(),
                )])
            }
        }
    }

    async fn apply_event(
        &self,
        _state: &Self::State,
        event: &Self::Event,
    ) -> Result<Self::State, EntityError> {
        Ok(UserState::Registered {
            email: event.email.clone(),
            tenant_id: event.tenant_id.clone(),
        })
    }

    async fn apply_events(
        &self,
        state: &Self::State,
        events: &[Self::Event],
    ) -> Result<Self::State, EntityError> {
        let mut new_state = state.clone();
        for event in events {
            new_state = self.apply_event(&new_state, event).await?;
        }
        Ok(new_state)
    }
}
