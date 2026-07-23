//! CORE-019A Phase 6 (6.1/6.2) E2E: `PricingEntity` fetches external data
//! through a registered provider — never an inline client — driven
//! end-to-end through `RuntimeBuilder::register_data_provider` ->
//! `Runtime::data_provider_access()` -> the handler's `DataProviderAccess`
//! facade. Mirrors `tests/effects_e2e.rs`'s real-runtime-construction shape.
//!
//! Also covers Phase 5.3: swapping the registered provider for a `testkit`
//! double at the registration boundary, with zero `PricingEntity` code
//! changes ("Test double swaps in without touching handler code").

use std::sync::Arc;

use ego_runtime::providers::ExternalDataProvider;
use ego_service_sdk::RuntimeBuilder;
use ego_testkit::RecordingDataProvider;
use persistent_entity::builder::EntityRuntimeBuilder;
use persistent_entity::command_context::CommandContext;
use persistent_entity::data_provider_access::DataResponse;
use persistent_entity::entity_ref::EntityRef;
use persistent_entity::error::EntityError;
use persistent_entity::persistent_entity::CommandResult;
use reference_app::domain::pricing::{PriceLooked, PricingCommand, PricingEntity, PricingState};
use reference_app::providers::pricing_lookup::PricingLookupProvider;

fn ctx() -> CommandContext {
    CommandContext::new("pricing".to_string())
}

async fn lookup(
    provider: Arc<dyn ExternalDataProvider>,
    sku: &str,
) -> Result<CommandResult<PriceLooked, PricingState>, EntityError> {
    let rt = RuntimeBuilder::new()
        .register_data_provider("pricing", provider)
        .unwrap()
        .build();
    let access = rt
        .data_provider_access()
        .expect("a provider was registered — the facade must be built");

    let entity_runtime = Arc::new(EntityRuntimeBuilder::<PriceLooked>::new().build());
    let entity_ref = entity_runtime
        .entity_ref::<PricingCommand, PricingState>(
            "pricing",
            sku,
            Arc::new(PricingEntity::new(access)),
        )
        .unwrap();

    entity_ref
        .send_command(
            PricingCommand::Lookup {
                sku: sku.to_string(),
            },
            ctx(),
        )
        .await
}

#[tokio::test]
async fn pricing_entity_fetches_through_the_registered_dogfood_provider_e2e() {
    let result = lookup(Arc::new(PricingLookupProvider), "sku-abc").await;

    let events = match result.unwrap() {
        CommandResult::Events { events, .. } => events,
        other => panic!("expected Events, got {other:?}"),
    };
    assert_eq!(events.len(), 1);
    let payload_str = String::from_utf8(events[0].price_payload.clone()).unwrap();
    assert!(
        payload_str.contains("sku-abc"),
        "the fetched payload must reflect the requested sku: {payload_str}"
    );
}

#[tokio::test]
async fn testkit_double_swaps_in_for_the_dogfood_provider_with_zero_handler_changes() {
    let canned = DataResponse {
        payload: b"canned-price".to_vec(),
        cache_hit: true,
    };
    let double = Arc::new(RecordingDataProvider::new(canned.clone()));

    let result = lookup(double.clone(), "sku-xyz").await;

    let events = match result.unwrap() {
        CommandResult::Events { events, .. } => events,
        other => panic!("expected Events, got {other:?}"),
    };
    assert_eq!(
        events[0].price_payload, canned.payload,
        "unchanged PricingEntity code must return exactly the double's canned payload"
    );
    assert_eq!(events[0].cache_hit, canned.cache_hit);
    assert_eq!(
        double.requests().len(),
        1,
        "the double must observe exactly one fetch"
    );
    assert_eq!(double.requests()[0].key, "sku-xyz");
}
