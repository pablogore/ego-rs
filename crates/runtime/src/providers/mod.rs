//! External data provider subsystem (CORE-019A). Read-side counterpart to
//! [`crate::effects`].
//!
//! # Subsystem summary
//!
//! Apps implement [`ExternalDataProvider`] (one provider = one kind of
//! external data) and register instances via
//! [`ExternalDataProviderRegistry`]/`ego-service-sdk`'s
//! `RuntimeBuilder::register_data_provider` — one owner per `provider_id`,
//! fail-closed on a duplicate registration (AD-002/AD-005). A handler never
//! holds a concrete provider or the registry directly; it holds
//! `Arc<dyn persistent_entity::data_provider_access::DataProviderAccess>`,
//! whose sole runtime implementation, [`RuntimeDataProviderAccess`], performs
//! the registry lookup and is the single observability chokepoint — every
//! fetch attempt emits exactly one `tracing` signal carrying `provider_id`, a
//! hashed key, latency, `cache_hit`, and an explicit `ProviderOutcome`
//! (AD-008; `payload` and provider error message text are never logged).
//! See `openspec/changes/core-019a-external-data-providers/design.md`.

pub mod access;
pub mod provider;
pub mod registry;

pub use access::{ProviderOutcome, RuntimeDataProviderAccess};
pub use provider::ExternalDataProvider;
pub use registry::{DuplicateProviderId, ExternalDataProviderRegistry};
