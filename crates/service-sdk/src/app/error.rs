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
    #[error("adapter already registered for type `{type_name}`")]
    DuplicateAdapter {
        /// The concrete adapter type name that was already registered.
        type_name: &'static str,
    },
    /// A second effect store was registered through
    /// `AppBuilder::effect_store(...)` (CORE-028D1). Single-slot: rejected
    /// even for a different concrete type on the second call — `type_name`
    /// names the *rejected* registration, not the first-registered one.
    #[error("effect store already registered; second registration of `{type_name}` rejected")]
    DuplicateEffectStore {
        /// The concrete effect store type name that was rejected.
        type_name: &'static str,
    },
    /// A second effect retention store was registered through
    /// `AppBuilder::effect_retention_store(...)` (CORE-028D1). No
    /// `type_name`: the parameter is `Arc<dyn RetentionMaintenance>`, so no
    /// concrete type identity is available.
    #[error("effect retention store already registered")]
    DuplicateEffectRetentionStore,
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
    /// The config-to-logger pipeline failed during initialization.
    #[error("logger initialization failed: {0}")]
    Logger(RuntimeInfraError),
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
        assert_eq!(err.to_string(), "effect retention store already registered");
    }
}
