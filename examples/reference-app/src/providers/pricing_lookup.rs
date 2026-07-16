//! `PricingLookupProvider` — CORE-019A Phase 6 dogfood: a trivial,
//! deterministic, in-memory-only `ExternalDataProvider` (design.md
//! AD-010/AD-012), never a real HTTP/gRPC/DB call. Registered via
//! `RuntimeBuilder::register_data_provider` (see `tests/providers_e2e.rs`);
//! `domain::pricing::PricingEntity` reaches it only through the
//! `DataProviderAccess` facade, never this type directly — that separation
//! is what `external_data_provider_lint.rs` audits `domain/` for.

use async_trait::async_trait;
use ego_runtime::providers::ExternalDataProvider;
use persistent_entity::data_provider_access::{DataProviderError, DataRequest, DataResponse};

/// This reference app's own trivial dogfood provider (AD-010/AD-012):
/// deterministically derives a "price" from the requested SKU — never a
/// real HTTP/gRPC/DB call — so this crate has no external client dependency
/// to construct inline anywhere.
pub struct PricingLookupProvider;

#[async_trait]
impl ExternalDataProvider for PricingLookupProvider {
    async fn fetch(&self, request: DataRequest) -> Result<DataResponse, DataProviderError> {
        // Deterministic, non-hardcoded: the price is derived from the key
        // itself, proving the response is real data flowing through, not a
        // fixed constant (mirrors persistent-entity's `AlwaysEcho` double).
        let price_cents = 100u64 + (request.key.len() as u64) * 10;
        let payload = serde_json::json!({ "sku": request.key, "price_cents": price_cents })
            .to_string()
            .into_bytes();
        Ok(DataResponse {
            payload,
            cache_hit: false,
        })
    }
}
