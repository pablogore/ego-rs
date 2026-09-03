//! Composition errors (design.md AD-8).
//!
//! `CompositionError` wraps existing typed errors — never replaces them —
//! one variant per phase/component so errors are distinguishable by phase
//! (composition / initialization / execution / shutdown) and each names the
//! failing component. **Invariant (L1):** a variant wraps one of the
//! existing typed errors below (or a plain field), never another
//! `CompositionError` — exactly one layer of wrapping, always.

use ego_runtime::effects::DuplicateEffectType;
use ego_runtime::providers::DuplicateProviderId;

use crate::di::{DuplicateEntity, DuplicateProjection};
use crate::registry::RegistryError;
use crate::runtime::{RuntimeError, RuntimeInfraError};

/// Errors produced while composing, validating, starting, or shutting down
/// an [`crate::app::App`] (AD-8).
#[derive(Debug, thiserror::Error)]
pub enum CompositionError {
    /// A second adapter of the same concrete type was registered without
    /// using the explicit `.replace_adapter()` escape hatch (AD-4).
    #[error("adapter already registered for type `{type_name}` — use `.replace_adapter(...)` to override it")]
    DuplicateAdapter {
        /// The concrete adapter type name that was already registered.
        type_name: &'static str,
    },
    /// A second effect store was registered through
    /// `AppBuilder::effect_store(...)` (CORE-028D1). Single-slot: rejected
    /// even for a different concrete type on the second call — `type_name`
    /// names the *rejected* registration, not the first-registered one.
    /// Deliberately has no replace escape hatch (CORE-028D1) — the message
    /// must not invent one.
    #[error(
        "effect store already registered; second registration of `{type_name}` rejected — register exactly one effect store"
    )]
    DuplicateEffectStore {
        /// The concrete effect store type name that was rejected.
        type_name: &'static str,
    },
    /// A second effect retention store was registered through
    /// `AppBuilder::effect_retention_store(...)` (CORE-028D1). No
    /// `type_name`: the parameter is `Arc<dyn RetentionMaintenance>`, so no
    /// concrete type identity is available. Deliberately has no replace
    /// escape hatch (CORE-028D1) — the message must not invent one.
    #[error(
        "effect retention store already registered — register exactly one effect retention store"
    )]
    DuplicateEffectRetentionStore,
    /// A second durable progress pair was registered for the same
    /// `projection_id` through `AppBuilder::read_side_progress(...)`
    /// (PROD-014A). Rejected even when the second pair is durable and the
    /// first was not: silently replacing a projection's resume state is not
    /// a composition a reader can verify. Deliberately has no replace escape
    /// hatch — the message must not invent one.
    #[error(
        "read-side progress stores already registered for projection `{projection_id}`; \
         second registration rejected — register exactly one progress pair per projection"
    )]
    DuplicateReadSideProgress {
        /// The `projection_id` whose second registration was rejected.
        projection_id: String,
    },
    /// A second durable claim store was registered through
    /// `AppBuilder::read_side_claims(...)` (PROD-014C). One global slot —
    /// unlike `DuplicateReadSideProgress`, there is no `projection_id` to
    /// key by: `projection_id` is already part of the claim identity
    /// itself, so one store serves every projection (AD-9 criteria c).
    /// Deliberately has no replace escape hatch — the message must not
    /// invent one.
    #[error(
        "read-side claim store already registered via `AppBuilder::read_side_claims(...)`; \
         second registration rejected — register exactly one claim store"
    )]
    DuplicateReadSideClaimStore,
    /// A service registration was rejected by the underlying registry
    /// (e.g. a duplicate `(Tag, version)` pair).
    #[error("service registration failed: {0}")]
    Service(#[from] RegistryError),
    /// Composition-time validation failed — e.g. a registered service's
    /// declared dependency is missing. Preserves the missing type and the
    /// requesting service, exactly as `RuntimeBuilder::try_build` already
    /// reports them.
    #[error("composition validation failed: {0}")]
    Validation(#[from] RuntimeError),
    /// An effect executor registration was rejected (duplicate effect type).
    #[error("effect executor registration failed: {0}")]
    EffectExecutor(#[from] DuplicateEffectType),
    /// A data provider registration was rejected (duplicate provider id).
    #[error("data provider registration failed: {0}")]
    DataProvider(#[from] DuplicateProviderId),
    /// A projection registration was rejected (duplicate projection type,
    /// CORE-028 Stage 2 AD-4).
    #[error("projection registration failed: {0}")]
    Projection(#[from] DuplicateProjection),
    /// An entity runtime registration was rejected (duplicate aggregate
    /// type, CORE-028 Stage 2C AD-4).
    #[error("entity registration failed: {0}")]
    Entity(#[from] DuplicateEntity),
    /// Starting the application's background effects failed.
    #[error("application startup failed: {0}")]
    Startup(RuntimeInfraError),
    /// Shutting down the application (async hooks or sync teardown) failed.
    #[error("application shutdown failed: {0}")]
    Shutdown(RuntimeInfraError),
}

#[cfg(test)]
mod tests {
    use crate::runtime::RuntimeError;

    use super::CompositionError;

    // Task 1.1 (RED): `CompositionError::DuplicateAdapter` carries `type_name`.
    #[test]
    fn duplicate_adapter_carries_type_name() {
        let err = CompositionError::DuplicateAdapter {
            type_name: "MyAdapter",
        };
        match err {
            CompositionError::DuplicateAdapter { type_name } => {
                assert_eq!(type_name, "MyAdapter");
            }
            other => panic!("expected DuplicateAdapter, got {other:?}"),
        }
    }

    // Task 1.1 (RED): `CompositionError::Validation` wraps `RuntimeError`
    // preserving both the missing type and the requesting service (AD-8).
    #[test]
    fn validation_wraps_runtime_error_preserving_type_and_service() {
        let inner = RuntimeError::DependencyNotFound {
            kind: crate::runtime::DependencyKind::Adapter,
            type_name: "MyAdapter",
            service_name: Some("MyService"),
        };
        let err: CompositionError = inner.into();
        match err {
            CompositionError::Validation(RuntimeError::DependencyNotFound {
                type_name,
                service_name,
                ..
            }) => {
                assert_eq!(type_name, "MyAdapter");
                assert_eq!(service_name, Some("MyService"));
            }
            other => panic!("expected Validation(DependencyNotFound), got {other:?}"),
        }
    }

    // Triangulation: a second distinct scenario proving `#[from]` wiring is
    // real, not a hardcoded pass-through — a `ServiceNotFound` variant with
    // no fields still round-trips through the wrapper untouched.
    #[test]
    fn validation_wraps_service_not_found_variant_too() {
        let err: CompositionError = RuntimeError::ServiceNotFound {
            type_name: "MyTag",
            required_by: None,
        }
        .into();
        assert!(matches!(
            err,
            CompositionError::Validation(RuntimeError::ServiceNotFound { .. })
        ));
    }

    // Task 1.4: `CompositionError::Projection` round-trips a `DuplicateProjection`
    // via `.into()`, mirroring `validation_wraps_service_not_found_variant_too`.
    #[test]
    fn projection_wraps_duplicate_projection_variant() {
        use crate::di::DuplicateProjection;

        let inner = DuplicateProjection {
            type_name: "MyProjection",
        };
        let err: CompositionError = inner.into();
        match err {
            CompositionError::Projection(DuplicateProjection { type_name }) => {
                assert_eq!(type_name, "MyProjection");
            }
            other => panic!("expected Projection(DuplicateProjection), got {other:?}"),
        }
    }

    // Task 1.4 (CORE-028 Stage 2C): `CompositionError::Entity` round-trips a
    // `DuplicateEntity` via `.into()`, mirroring
    // `projection_wraps_duplicate_projection_variant`.
    #[test]
    fn entity_wraps_duplicate_entity_variant() {
        use crate::di::DuplicateEntity;

        let inner = DuplicateEntity {
            type_name: "MyEntity",
        };
        let err: CompositionError = inner.into();
        match err {
            CompositionError::Entity(DuplicateEntity { type_name }) => {
                assert_eq!(type_name, "MyEntity");
            }
            other => panic!("expected Entity(DuplicateEntity), got {other:?}"),
        }
    }

    // CORE-028D1 task 1.1 (RED): `CompositionError::DuplicateEffectStore`
    // carries `type_name`, mirroring `duplicate_adapter_carries_type_name`.
    #[test]
    fn duplicate_effect_store_carries_type_name() {
        let err = CompositionError::DuplicateEffectStore {
            type_name: "MyEffectStore",
        };
        match err {
            CompositionError::DuplicateEffectStore { type_name } => {
                assert_eq!(type_name, "MyEffectStore");
            }
            other => panic!("expected DuplicateEffectStore, got {other:?}"),
        }
    }

    // CORE-028D1 task 1.3 (RED): pins the fieldless
    // `DuplicateEffectRetentionStore` variant's `Display` message.
    #[test]
    fn duplicate_effect_retention_store_display_text() {
        let err = CompositionError::DuplicateEffectRetentionStore;
        assert_eq!(
            err.to_string(),
            "effect retention store already registered — register exactly one effect retention store"
        );
    }

    // Composition Error Diagnostics Cleanup: `DuplicateAdapter` has a real
    // escape hatch (`.replace_adapter()`), so its message must point to it.
    #[test]
    fn duplicate_adapter_message_points_to_replace_adapter() {
        let err = CompositionError::DuplicateAdapter {
            type_name: "MyAdapter",
        };
        assert_eq!(
            err.to_string(),
            "adapter already registered for type `MyAdapter` — use `.replace_adapter(...)` to override it"
        );
    }

    // Composition Error Diagnostics Cleanup: `DuplicateEffectStore` has no
    // replace escape hatch (CORE-028D1 deliberate decision) — the message
    // must not invent or imply one, only state the actual contract.
    #[test]
    fn duplicate_effect_store_message_states_the_contract_without_a_replace_api() {
        let err = CompositionError::DuplicateEffectStore {
            type_name: "MyEffectStore",
        };
        let text = err.to_string();
        assert_eq!(
            text,
            "effect store already registered; second registration of `MyEffectStore` rejected — register exactly one effect store"
        );
        assert!(
            !text.contains("replace"),
            "must not suggest a non-existent replace API: {text:?}"
        );
    }

    // Composition Error Diagnostics Cleanup: same non-goal as the effect
    // store above — no replace API exists for the retention store either.
    #[test]
    fn duplicate_effect_retention_store_message_has_no_replace_api() {
        let text = CompositionError::DuplicateEffectRetentionStore.to_string();
        assert!(
            !text.contains("replace"),
            "must not suggest a non-existent replace API: {text:?}"
        );
    }

    // PROD-014A task 3.1 (RED): `CompositionError::DuplicateReadSideProgress`
    // carries `projection_id`, mirroring `duplicate_adapter_carries_type_name`.
    #[test]
    fn duplicate_read_side_progress_carries_projection_id() {
        let err = CompositionError::DuplicateReadSideProgress {
            projection_id: "users-by-tenant".to_string(),
        };
        match err {
            CompositionError::DuplicateReadSideProgress { projection_id } => {
                assert_eq!(projection_id, "users-by-tenant");
            }
            other => panic!("expected DuplicateReadSideProgress, got {other:?}"),
        }
    }

    // PROD-014A task 3.1 (RED): the message names the projection and, like
    // `DuplicateEffectStore`, suggests no non-existent replace API.
    #[test]
    fn duplicate_read_side_progress_message_names_projection_without_a_replace_api() {
        let err = CompositionError::DuplicateReadSideProgress {
            projection_id: "users-by-tenant".to_string(),
        };
        let text = err.to_string();
        assert_eq!(
            text,
            "read-side progress stores already registered for projection `users-by-tenant`; \
             second registration rejected — register exactly one progress pair per projection"
        );
        assert!(
            !text.contains("replace"),
            "must not suggest a non-existent replace API: {text:?}"
        );
    }

    // PROD-014C task 7.3 (RED): `CompositionError::DuplicateReadSideClaimStore`
    // names the offending call and suggests no replace API, mirroring
    // `DuplicateReadSideProgress` (PROD-014A 3.1) — but fieldless: one global
    // slot, not a per-projection map (AD-9 criteria c), so there is no
    // `projection_id` to distinguish registrations by.
    #[test]
    fn duplicate_read_side_claim_store_message_names_call_without_a_replace_api() {
        let err = CompositionError::DuplicateReadSideClaimStore;
        let text = err.to_string();
        assert!(
            text.contains("read_side_claims"),
            "the message must name the offending call: {text:?}"
        );
        assert!(
            !text.contains("replace"),
            "must not suggest a non-existent replace API: {text:?}"
        );
    }
}
