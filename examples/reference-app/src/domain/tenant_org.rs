//! `TenantOrganization` `PersistentEntity` (design.md AD-5, AD-6).
//!
//! Idempotent "ensure org exists" shape: `Ensure` on an already-`Present`
//! org produces zero events (the runtime/actor layer maps that to
//! `CommandResult::NoEvents`) — this idempotency is exactly what makes
//! AD-5's "benign reusable orphan" dual-write claim true.
//!
//! Satisfies reference-service spec "Associating a user with a tenant org"
//! per this change's ground-truth resolution: the spec's
//! `UserAssociatedWithTenant`/"membership set" wording is stale prose,
//! reconciled at `sdd-verify` — AD-6 is the decision-backed shape actually
//! implemented here.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ego_domain::DomainEvent;
use persistent_entity::command_context::CommandContext;
use persistent_entity::error::EntityError;
use persistent_entity::persistent_entity::PersistentEntity;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Commands accepted by [`TenantOrganizationEntity`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TenantOrgCommand {
    /// Ensure a tenant organization exists (idempotent).
    Ensure { org_id: String, name: String },
}

/// Emitted the first time `Ensure` transitions `Absent` -> `Present`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OrganizationEnsured {
    pub org_id: String,
    pub name: String,
    pub occurred_at: DateTime<Utc>,
    payload: Value,
}

impl OrganizationEnsured {
    fn new(org_id: String, name: String, occurred_at: DateTime<Utc>) -> Self {
        let payload = serde_json::json!({
            "org_id": org_id,
            "name": name,
        });
        Self {
            org_id,
            name,
            occurred_at,
            payload,
        }
    }
}

impl DomainEvent for OrganizationEnsured {
    fn aggregate_id(&self) -> &str {
        &self.org_id
    }

    fn event_type(&self) -> &str {
        "OrganizationEnsured"
    }

    fn payload(&self) -> &Value {
        &self.payload
    }

    fn occurred_at(&self) -> &DateTime<Utc> {
        &self.occurred_at
    }
}

/// State of a `TenantOrganization` aggregate.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TenantOrgState {
    Absent,
    Present { name: String },
}

/// The `TenantOrganization` aggregate (design.md AD-6).
#[derive(Debug, Clone)]
pub struct TenantOrganizationEntity;

impl TenantOrganizationEntity {
    pub fn new() -> Self {
        Self
    }
}

impl Default for TenantOrganizationEntity {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PersistentEntity for TenantOrganizationEntity {
    type Command = TenantOrgCommand;
    type Event = OrganizationEnsured;
    type State = TenantOrgState;

    fn initial_state(&self) -> Self::State {
        TenantOrgState::Absent
    }

    async fn handle_command(
        &self,
        command: &Self::Command,
        state: &Self::State,
        _context: &CommandContext,
    ) -> Result<Vec<Self::Event>, EntityError> {
        match command {
            TenantOrgCommand::Ensure { org_id, name } => match state {
                // Idempotent: already ensured, nothing new to apply.
                TenantOrgState::Present { .. } => Ok(vec![]),
                TenantOrgState::Absent => Ok(vec![OrganizationEnsured::new(
                    org_id.clone(),
                    name.clone(),
                    Utc::now(),
                )]),
            },
        }
    }

    async fn apply_event(
        &self,
        _state: &Self::State,
        event: &Self::Event,
    ) -> Result<Self::State, EntityError> {
        Ok(TenantOrgState::Present {
            name: event.name.clone(),
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
