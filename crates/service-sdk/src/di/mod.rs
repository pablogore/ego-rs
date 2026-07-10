//! Dependency injection primitives for the Service SDK.
//!
//! This module provides the core DI types used to declare service dependencies
//! in a strongly-typed, macro-friendly way.

// NOTE: EntityRef<T> is owned by entity-sdk (CORE-006) and must NOT be defined here.
// When CORE-006 is available, add: use entity_sdk::EntityRef; and re-export it.
// See INV-008 in openspec/changes/service-sdk/spec.md.

use std::any::TypeId;
use std::ops::Deref;
use std::sync::Arc;

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
}
