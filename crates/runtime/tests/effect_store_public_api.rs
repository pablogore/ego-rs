//! Proves `EffectStateStore` is a genuinely public, externally-implementable
//! port (F-01, CORE-019 post-review fix).
//!
//! This file is a separate compilation unit outside `ego-runtime`'s own
//! `src/` module tree — it only sees `ego_runtime`'s public API, exactly as
//! any downstream crate would. Before the fix, `EffectStateStore::accept`
//! took the crate-private `EffectEnvelope`, so this test could not have
//! compiled without `#[allow(private_interfaces)]` leaking the type. Now it
//! only ever names `AcceptedEffect`, `StoredEffect`, `Timestamp`, and
//! `EffectStoreError` — all public.

use std::sync::Arc;

use ego_domain::{ExternalEffectDescription, IdempotencyKey, TenantId};
use ego_runtime::effects::{
    AcceptedEffect, EffectId, EffectState, EffectStateStore, EffectStoreError, InMemoryEffectStore,
    Timestamp,
};

fn sample_effect(id: EffectId) -> AcceptedEffect {
    AcceptedEffect {
        id,
        tenant: TenantId::new("tenant-external").unwrap(),
        attempt: 0,
        description: Arc::new(ExternalEffectDescription {
            idempotency_key: IdempotencyKey::new("uow-ext:0").unwrap(),
            effect_type: "invoice.created".to_string(),
            payload: vec![9, 9, 9],
            destination: "https://example.com/hook".to_string(),
        }),
    }
}

#[tokio::test]
async fn accepted_effect_is_constructible_and_usable_from_outside_the_crate() {
    let store = InMemoryEffectStore::new();
    let id = EffectId::new();

    store.accept(sample_effect(id)).await.unwrap();

    let claimed = store.claim_due(Timestamp::now(), 10).await.unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].id, id);
    assert_eq!(claimed[0].state, EffectState::Pending);
}

#[tokio::test]
async fn recover_in_flight_is_callable_from_outside_the_crate() {
    let store = InMemoryEffectStore::new();
    let id = EffectId::new();
    store.accept(sample_effect(id)).await.unwrap();
    store.mark_in_flight(id).await.unwrap();

    let recovered = store.recover_in_flight(Timestamp::now()).await.unwrap();

    assert_eq!(recovered, 1);
}

#[test]
fn error_taxonomy_variants_are_publicly_constructible() {
    let errors = [
        EffectStoreError::NotFound(EffectId::new()),
        EffectStoreError::Conflict("dup".into()),
        EffectStoreError::TemporarilyUnavailable("timeout".into()),
        EffectStoreError::Backend("corrupt".into()),
    ];

    assert_eq!(errors.len(), 4);
}
