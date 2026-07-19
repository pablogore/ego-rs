//! Dependency injection primitives for the Service SDK.
//!
//! This module provides the core DI types used to declare service dependencies
//! in a strongly-typed, macro-friendly way.

// NOTE (CORE-028 Stage 2C): `EntityRuntimeRef<E>` below is the
// composition-time DI handle for an entity aggregate type, distinct from
// `persistent_entity::entity_ref::EntityRef<E>` — the per-dispatch handle to
// ONE specific entity instance, owned by `persistent-entity` and unchanged
// by this module. `EntityRuntimeRef<E>` wraps `Arc<EntityRuntime<E::Event>>`
// and exposes `entity_ref(...)` as a thin passthrough to obtain that
// per-dispatch handle.

use std::any::TypeId;
use std::ops::Deref;
use std::sync::Arc;

use ego_domain::event::DomainEvent;
use persistent_entity::entity_ref::EntityRef;
use persistent_entity::error::EntityError;
use persistent_entity::persistent_entity::PersistentEntity;
use persistent_entity::runtime::EntityRuntime;

/// Registering a second projection instance for a type that already has one
/// registered (CORE-028 Stage 2 design.md AD-1/AD-2). Strictly fail-closed —
/// there is no override; the first registration is left untouched.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("projection already registered for type `{type_name}`")]
pub struct DuplicateProjection {
    /// The concrete projection type name that was already registered.
    pub type_name: &'static str,
}

/// Registering a second entity runtime for an aggregate type that already
/// has one registered (CORE-028 Stage 2C design.md AD-1/AD-4). Strictly
/// fail-closed, mirroring `DuplicateProjection` — there is no override; the
/// first registration is left untouched.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("entity runtime already registered for type `{type_name}`")]
pub struct DuplicateEntity {
    /// The concrete aggregate type name that was already registered.
    pub type_name: &'static str,
}

/// A composition-time handle capable of dispatching to any entity instance
/// of aggregate type `E` (CORE-028 Stage 2C design.md AD-3). Wraps a
/// host-constructed `Arc<EntityRuntime<E::Event>>`; deliberately distinct
/// from `persistent_entity::EntityRef<E>`, the per-dispatch handle to ONE
/// specific entity instance obtained from [`Self::entity_ref`] below.
pub struct EntityRuntimeRef<E: PersistentEntity> {
    inner: Arc<EntityRuntime<E::Event>>,
}

impl<E: PersistentEntity> EntityRuntimeRef<E>
where
    E::Event: DomainEvent + Clone + serde::de::DeserializeOwned + serde::Serialize + Send + Sync + 'static,
{
    /// Creates a new `EntityRuntimeRef` wrapping the given `Arc<EntityRuntime<E::Event>>`.
    pub fn new(inner: Arc<EntityRuntime<E::Event>>) -> Self {
        Self { inner }
    }

    /// Opens a per-request handle to one entity instance — a thin passthrough
    /// to `EntityRuntime::entity_ref`, with `Event` pinned to `E::Event`.
    ///
    /// **Panics** if called outside an active Tokio runtime context (e.g.
    /// inside an `async fn` or `#[tokio::test]`) — this passthrough spawns a
    /// real actor via `tokio::spawn` on every call, exactly like the wrapped
    /// `EntityRuntime::entity_ref` it delegates to. A service holding an
    /// `EntityRuntimeRef<E>` field resolved through DI must only call this
    /// from within its own async request-handling path; it is not safe to
    /// call from a synchronous composition-time context (e.g. while still
    /// inside `AppBuilder::build`).
    pub fn entity_ref<C, S>(
        &self,
        entity_type: &'static str,
        entity_id: impl Into<String>,
        handler: Arc<dyn PersistentEntity<Command = C, Event = E::Event, State = S>>,
    ) -> Result<impl EntityRef<Command = C>, EntityError>
    where
        C: Send + Sync + serde::Serialize + 'static,
        S: serde::Serialize + Clone + serde::de::DeserializeOwned + Send + Sync + 'static,
    {
        self.inner.entity_ref(entity_type, entity_id, handler)
    }

    /// Test-only same-instance check (review fix, reliability): `EntityRuntime`
    /// has no meaningful value-equality, so proving two resolutions landed on
    /// the identical registered runtime — not just two independently-`Ok`
    /// resolutions — goes through `Arc::ptr_eq` on the wrapped runtime,
    /// mirroring how other resolved-instance equivalence tests in this crate
    /// assert sameness. Not part of the public API surface.
    #[cfg(test)]
    pub(crate) fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

/// A reference to a read-model projection.
pub struct ProjectionRef<P> {
    inner: Arc<P>,
}

impl<P> ProjectionRef<P> {
    /// Creates a new `ProjectionRef` wrapping the given `Arc<P>`.
    pub fn new(inner: Arc<P>) -> Self {
        Self { inner }
    }
}

impl<P> Deref for ProjectionRef<P> {
    type Target = P;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

/// A reference to an adapter (port implementation).
pub struct AdapterRef<A> {
    inner: Arc<A>,
}

impl<A> AdapterRef<A> {
    /// Creates a new `AdapterRef` wrapping the given `Arc<A>`.
    pub fn new(inner: Arc<A>) -> Self {
        Self { inner }
    }
}

impl<A> Deref for AdapterRef<A> {
    type Target = A;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

/// A configuration value.
pub struct ConfigValue<T> {
    value: Arc<T>,
}

impl<T> ConfigValue<T> {
    /// Creates a new `ConfigValue` wrapping the given `Arc<T>`.
    pub fn new(value: Arc<T>) -> Self {
        Self { value }
    }
}

impl<T> Deref for ConfigValue<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

/// A discriminated key identifying the kind and type of a dependency.
#[derive(Debug, PartialEq, Eq)]
pub enum DepKey {
    /// An entity dependency, keyed by type, with its type name for diagnostics.
    Entity(TypeId, &'static str),
    /// A projection dependency, keyed by type, with its type name for diagnostics.
    Projection(TypeId, &'static str),
    /// An adapter dependency, keyed by type, with its type name for diagnostics.
    Adapter(TypeId, &'static str),
    /// A configuration value dependency, keyed by type, with its type name for diagnostics.
    Config(TypeId, &'static str),
}

/// Trait that a service struct implements (via macro) to declare its dependencies.
pub trait Injectable: Send + Sync {
    /// Returns the list of dependency keys this type requires.
    fn dependencies() -> Vec<DepKey>
    where
        Self: Sized;

    /// Presence-only dependency check. Constructs nothing. The default is
    /// fully generic over `dependencies()` — zero per-service codegen.
    ///
    /// Used by `RuntimeBuilder::try_build()` (fail-fast bootstrap, AD-3 /
    /// OQ-2) to detect a missing adapter/config/projection before
    /// constructing anything, as opposed to `build()` which only discovers a
    /// missing dependency by trying to resolve it.
    fn validate(rt: &crate::runtime::RuntimeInner) -> Result<(), crate::runtime::RuntimeError>
    where
        Self: Sized,
    {
        for dep in Self::dependencies() {
            rt.check_dependency(&dep)?;
        }
        Ok(())
    }

    /// Constructs an instance by resolving dependencies from the runtime.
    /// Returns `RuntimeError::DependencyNotFound` while resolvers are not yet wired.
    fn build(rt: &crate::runtime::RuntimeInner) -> Result<Self, crate::runtime::RuntimeError>
    where
        Self: Sized;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// REQ-011 / TASK-008 — DI primitives are discriminated by DepKey variant.
    #[test]
    fn di_primitives_are_recognizable() {
        let entity_key = DepKey::Entity(TypeId::of::<()>(), "()");
        let projection_key = DepKey::Projection(TypeId::of::<()>(), "()");
        let adapter_key = DepKey::Adapter(TypeId::of::<()>(), "()");
        let config_key = DepKey::Config(TypeId::of::<()>(), "()");

        // Same inner TypeId, different variant — must not be equal.
        assert_ne!(entity_key, projection_key);
        assert_ne!(entity_key, adapter_key);
        assert_ne!(entity_key, config_key);
        assert_ne!(projection_key, adapter_key);
        assert_ne!(projection_key, config_key);
        assert_ne!(adapter_key, config_key);
    }

    // CORE-028 Stage 2 (task 1.1): `DuplicateProjection` carries the concrete
    // type name, mirroring `CompositionError::DuplicateAdapter`'s shape
    // (`duplicate_adapter_carries_type_name`).
    #[test]
    fn duplicate_projection_carries_type_name() {
        let err = DuplicateProjection { type_name: "MyProjection" };
        assert_eq!(err.type_name, "MyProjection");
        assert!(err.to_string().contains("MyProjection"));
    }

    // CORE-028 Stage 2C (task 1.1): `DuplicateEntity` carries the concrete
    // aggregate type name, mirroring `duplicate_projection_carries_type_name`.
    #[test]
    fn duplicate_entity_carries_type_name() {
        let err = DuplicateEntity { type_name: "MyEntity" };
        assert_eq!(err.type_name, "MyEntity");
        assert!(err.to_string().contains("MyEntity"));
    }
}
