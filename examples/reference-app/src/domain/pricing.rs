//! `PricingEntity` — CORE-019A Phase 6 dogfood: a trivial `PersistentEntity`
//! handler that fetches external data exclusively through the
//! `DataProviderAccess` facade (design.md AD-010/AD-012), never by
//! constructing an external client inline. The concrete dogfood provider
//! type lives in `crate::providers::pricing_lookup` instead of here —
//! `handle_command` below never names it — so `external_data_provider_lint.rs`
//! can audit every file under `domain/` (this one included) for that
//! separation instead of having to exclude the one file that most needs
//! auditing.
//!
//! Satisfies `external-data-providers` spec: "Reference-app handler never
//! constructs a client inline"; `persistent-entity` spec: "Handler fetches
//! external data during command handling".

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ego_domain::DomainEvent;
use persistent_entity::command_context::CommandContext;
use persistent_entity::data_provider_access::{DataProviderAccess, DataRequest};
use persistent_entity::error::EntityError;
use persistent_entity::persistent_entity::PersistentEntity;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Commands accepted by [`PricingEntity`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PricingCommand {
    /// Look up the price for `sku` through the registered provider.
    Lookup { sku: String },
}

/// Emitted once a `Lookup` command's fetch succeeds.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PriceLooked {
    pub sku: String,
    /// The provider's opaque response payload, passed through unexamined.
    pub price_payload: Vec<u8>,
    pub cache_hit: bool,
    pub occurred_at: DateTime<Utc>,
    payload: Value,
}

impl PriceLooked {
    fn new(sku: String, price_payload: Vec<u8>, cache_hit: bool, occurred_at: DateTime<Utc>) -> Self {
        let payload = serde_json::json!({ "sku": sku, "cache_hit": cache_hit });
        Self {
            sku,
            price_payload,
            cache_hit,
            occurred_at,
            payload,
        }
    }
}

impl DomainEvent for PriceLooked {
    fn aggregate_id(&self) -> &str {
        &self.sku
    }

    fn event_type(&self) -> &str {
        "PriceLooked"
    }

    fn payload(&self) -> &Value {
        &self.payload
    }

    fn occurred_at(&self) -> &DateTime<Utc> {
        &self.occurred_at
    }
}

/// State of a `Pricing` aggregate — trivial, one SKU lookup per aggregate id.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PricingState {
    NeverLookedUp,
    Looked { price_payload: Vec<u8> },
}

/// The `Pricing` "handler" (design.md AD-003): holds `Arc<dyn
/// DataProviderAccess>` and never a concrete provider or the runtime's
/// registry — the facade is the only path to external data (AD-012
/// genericity guardrail: this dogfood is the forcing function against an
/// over-abstract SPI, and the SPI needed no generic parameter to satisfy
/// it — `handle_command` below is the entire integration).
pub struct PricingEntity {
    access: Arc<dyn DataProviderAccess>,
}

impl PricingEntity {
    pub fn new(access: Arc<dyn DataProviderAccess>) -> Self {
        Self { access }
    }
}

impl std::fmt::Debug for PricingEntity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PricingEntity").finish_non_exhaustive()
    }
}

#[async_trait]
impl PersistentEntity for PricingEntity {
    type Command = PricingCommand;
    type Event = PriceLooked;
    type State = PricingState;

    fn initial_state(&self) -> Self::State {
        PricingState::NeverLookedUp
    }

    async fn handle_command(
        &self,
        command: &Self::Command,
        _state: &Self::State,
        _context: &CommandContext,
    ) -> Result<Vec<Self::Event>, EntityError> {
        match command {
            PricingCommand::Lookup { sku } => {
                // The ONLY path to external data: the facade. Never a
                // concrete provider reference, never a runtime-internal
                // type.
                let response = self
                    .access
                    .fetch(
                        "pricing",
                        DataRequest {
                            key: sku.clone(),
                            payload: Vec::new(),
                        },
                    )
                    .await
                    .map_err(|e| EntityError::Internal(e.to_string()))?;
                Ok(vec![PriceLooked::new(sku.clone(), response.payload, response.cache_hit, Utc::now())])
            }
        }
    }

    async fn apply_event(
        &self,
        _state: &Self::State,
        event: &Self::Event,
    ) -> Result<Self::State, EntityError> {
        Ok(PricingState::Looked {
            price_payload: event.price_payload.clone(),
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
