//! Dependency injection primitives for the Service SDK.
//!
//! This module provides the core DI types used to declare service dependencies
//! in a strongly-typed, macro-friendly way.

use std::any::TypeId;
use std::marker::PhantomData;
use std::ops::Deref;
use std::sync::Arc;

// TODO: replace with entity_sdk::EntityRef when CORE-006 is available
/// A reference to an entity. Placeholder until entity_sdk::EntityRef is available.
pub struct EntityRef<T>(PhantomData<T>);

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
    /// An entity dependency, keyed by type.
    Entity(TypeId),
    /// A projection dependency, keyed by type.
    Projection(TypeId),
    /// An adapter dependency, keyed by type.
    Adapter(TypeId),
    /// A configuration value dependency, keyed by type.
    Config(TypeId),
}

/// Trait that a service struct implements (via macro) to declare its dependencies.
pub trait Injectable: Send + Sync {
    /// Returns the list of dependency keys this type requires.
    fn dependencies() -> Vec<DepKey>
    where
        Self: Sized;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// REQ-011 / TASK-008 — DI primitives are discriminated by DepKey variant.
    #[test]
    fn di_primitives_are_recognizable() {
        let entity_key = DepKey::Entity(TypeId::of::<()>());
        let projection_key = DepKey::Projection(TypeId::of::<()>());
        let adapter_key = DepKey::Adapter(TypeId::of::<()>());
        let config_key = DepKey::Config(TypeId::of::<()>());

        // Same inner TypeId, different variant — must not be equal.
        assert_ne!(entity_key, projection_key);
        assert_ne!(entity_key, adapter_key);
        assert_ne!(entity_key, config_key);
        assert_ne!(projection_key, adapter_key);
        assert_ne!(projection_key, config_key);
        assert_ne!(adapter_key, config_key);
    }
}
