//! Type-keyed service registry.
//!
//! Stores live service implementations keyed by `(TypeId, ContractVersion)`.
//! Version resolution supports exact matches and semver ranges.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::Arc;

use crate::contract::version::ContractVersion;

/// A raw implementation arc as stored in the registry.
type RawImpl = Arc<dyn Any + Send + Sync>;

/// The per-type version list stored in the registry.
type VersionList = Vec<(ContractVersion, RawImpl)>;

/// Error variants for registry operations.
#[derive(Debug, Clone)]
pub enum RegistryError {
    /// A service with the same tag and version was already registered.
    DuplicateService { name: String, version: String },
    /// No service matching the tag and version constraint was found.
    ServiceNotFound,
    /// A service dependency was not found (used by RuntimeBuilder).
    DependencyNotFound,
    /// A dependency cycle was detected (used by RuntimeBuilder).
    DependencyCycle,
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegistryError::DuplicateService { name, version } => {
                write!(f, "Duplicate service: {} v{}", name, version)
            }
            RegistryError::ServiceNotFound => write!(f, "Service not found"),
            RegistryError::DependencyNotFound => write!(f, "Dependency not found"),
            RegistryError::DependencyCycle => write!(f, "Dependency cycle detected"),
        }
    }
}

/// A type-keyed registry that holds live service implementations.
///
/// Keys are `TypeId` of a generated zero-sized tag type (e.g. `OrderServiceTag`).
/// Multiple versions of the same service can be registered simultaneously.
pub struct ServiceRegistry {
    // TypeId of the tag → list of (version, raw Arc<dyn Any + Send + Sync>)
    entries: HashMap<TypeId, VersionList>,
}

impl Default for ServiceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ServiceRegistry {
    /// Creates a new, empty `ServiceRegistry`.
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Checks if the registry contains no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Registers a service implementation under the given `Tag` type and version.
    ///
    /// # Errors
    /// Returns `Err(RegistryError::DuplicateService)` if an entry for the same
    /// `(TypeId<Tag>, version)` pair already exists.
    pub fn register<Tag: 'static>(
        &mut self,
        version: ContractVersion,
        impl_arc: RawImpl,
    ) -> Result<(), RegistryError> {
        let type_id = TypeId::of::<Tag>();
        let entries = self.entries.entry(type_id).or_default();

        // Duplicate detection: same (TypeId, ContractVersion) is rejected.
        if entries.iter().any(|(v, _)| v == &version) {
            return Err(RegistryError::DuplicateService {
                name: std::any::type_name::<Tag>().to_string(),
                version: version.to_string(),
            });
        }

        entries.push((version, impl_arc));
        Ok(())
    }

    /// Resolves a raw `Arc<dyn Any + Send + Sync>` matching the given `Tag` and constraint.
    ///
    /// For `VersionConstraint::Exact`, returns the entry at that exact version.
    /// For `VersionConstraint::LatestCompatible`, returns the highest satisfying version.
    ///
    /// # Errors
    /// Returns `Err(RegistryError::ServiceNotFound)` when no matching entry exists.
    pub fn resolve_raw<Tag: 'static>(
        &self,
        constraint: &crate::contract::version::VersionConstraint,
    ) -> Result<RawImpl, RegistryError> {
        let type_id = TypeId::of::<Tag>();
        let entries = self
            .entries
            .get(&type_id)
            .ok_or(RegistryError::ServiceNotFound)?;

        entries
            .iter()
            .filter(|(v, _)| constraint.matches(v))
            .max_by_key(|(v, _)| v.clone())
            .map(|(_, arc)| arc.clone())
            .ok_or(RegistryError::ServiceNotFound)
    }

    /// Merges another `ServiceRegistry` into this one.
    ///
    /// Re-runs `register` for every entry in `other`, so duplicate detection applies.
    ///
    /// # Errors
    /// Returns the first `RegistryError::DuplicateService` encountered, if any.
    pub fn merge(&mut self, other: ServiceRegistry) -> Result<(), RegistryError> {
        for (type_id, versions) in other.entries {
            for (version, arc) in versions {
                let entries = self.entries.entry(type_id).or_default();
                if entries.iter().any(|(v, _)| v == &version) {
                    return Err(RegistryError::DuplicateService {
                        name: format!("TypeId({:?})", type_id),
                        version: version.to_string(),
                    });
                }
                entries.push((version, arc));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::version::{ContractVersion, VersionConstraint};

    // Tag types used in tests — zero-sized types acting as registry keys.
    struct OrderServiceTag;
    struct PaymentServiceTag;

    fn make_arc() -> Arc<dyn Any + Send + Sync> {
        Arc::new(42u32)
    }

    /// REQ-001 / TASK-007 — register stores the implementation and resolve retrieves it.
    #[test]
    fn register_stores_live_implementation() {
        let mut registry = ServiceRegistry::new();
        let v1 = ContractVersion::new(1, 0, 0);
        let arc = make_arc();

        registry
            .register::<OrderServiceTag>(v1.clone(), arc.clone())
            .expect("register must succeed");

        let constraint = VersionConstraint::Exact(v1);
        let resolved = registry
            .resolve_raw::<OrderServiceTag>(&constraint)
            .expect("resolve must succeed");

        // The resolved Arc points to the same underlying value.
        assert!(Arc::ptr_eq(&arc, &resolved,));
    }

    /// REQ-002 / TASK-007 — duplicate (TypeId, version) is rejected.
    #[test]
    fn register_rejects_duplicate() {
        let mut registry = ServiceRegistry::new();
        let v1 = ContractVersion::new(1, 0, 0);

        registry
            .register::<OrderServiceTag>(v1.clone(), make_arc())
            .expect("first register must succeed");

        let result = registry.register::<OrderServiceTag>(v1.clone(), make_arc());
        assert!(
            matches!(result, Err(RegistryError::DuplicateService { .. })),
            "second register at same version must return DuplicateService"
        );

        // Original is still resolvable.
        let constraint = VersionConstraint::Exact(v1);
        assert!(registry.resolve_raw::<OrderServiceTag>(&constraint).is_ok());
    }

    /// REQ-003 / TASK-007 — exact version resolution returns the correct Arc.
    #[test]
    fn resolve_exact_version() {
        let mut registry = ServiceRegistry::new();
        let v1 = ContractVersion::new(1, 0, 0);
        let v2 = ContractVersion::new(2, 0, 0);

        let arc1: Arc<dyn Any + Send + Sync> = Arc::new(1u32);
        let arc2: Arc<dyn Any + Send + Sync> = Arc::new(2u32);

        registry
            .register::<OrderServiceTag>(v1.clone(), arc1.clone())
            .unwrap();
        registry
            .register::<OrderServiceTag>(v2.clone(), arc2.clone())
            .unwrap();

        let resolved_v1 = registry
            .resolve_raw::<OrderServiceTag>(&VersionConstraint::Exact(v1))
            .unwrap();
        let resolved_v2 = registry
            .resolve_raw::<OrderServiceTag>(&VersionConstraint::Exact(v2))
            .unwrap();

        assert!(Arc::ptr_eq(&resolved_v1, &arc1));
        assert!(Arc::ptr_eq(&resolved_v2, &arc2));
    }

    /// REQ-004 / TASK-007 — semver range resolution picks the highest satisfying version.
    #[test]
    fn resolve_semver_range() {
        let mut registry = ServiceRegistry::new();

        let v1_2 = ContractVersion::new(1, 2, 0);
        let v2_0 = ContractVersion::new(2, 0, 0);

        let arc_v1_2: Arc<dyn Any + Send + Sync> = Arc::new(12u32);
        let arc_v2_0: Arc<dyn Any + Send + Sync> = Arc::new(20u32);

        registry
            .register::<OrderServiceTag>(v1_2.clone(), arc_v1_2.clone())
            .unwrap();
        registry
            .register::<OrderServiceTag>(v2_0.clone(), arc_v2_0.clone())
            .unwrap();

        // ^1 should pick 1.2.0 (not 2.0.0).
        let range_1 = VersionConstraint::range("^1").unwrap();
        let resolved = registry.resolve_raw::<OrderServiceTag>(&range_1).unwrap();
        assert!(Arc::ptr_eq(&resolved, &arc_v1_2));

        // ^2 should pick 2.0.0.
        let range_2 = VersionConstraint::range("^2").unwrap();
        let resolved2 = registry.resolve_raw::<OrderServiceTag>(&range_2).unwrap();
        assert!(Arc::ptr_eq(&resolved2, &arc_v2_0));
    }

    /// REQ-005 / TASK-007 — resolving with no match returns ServiceNotFound.
    #[test]
    fn resolve_returns_not_found() {
        let registry = ServiceRegistry::new();

        // Empty registry.
        let result = registry.resolve_raw::<OrderServiceTag>(&VersionConstraint::Exact(
            ContractVersion::new(1, 0, 0),
        ));
        assert!(
            matches!(result, Err(RegistryError::ServiceNotFound)),
            "empty registry must return ServiceNotFound"
        );

        // Wrong type.
        let mut registry2 = ServiceRegistry::new();
        registry2
            .register::<OrderServiceTag>(ContractVersion::new(1, 0, 0), make_arc())
            .unwrap();

        let result2 = registry2.resolve_raw::<PaymentServiceTag>(&VersionConstraint::Exact(
            ContractVersion::new(1, 0, 0),
        ));
        assert!(matches!(result2, Err(RegistryError::ServiceNotFound)));

        // Unsatisfied semver range.
        let result3 =
            registry2.resolve_raw::<OrderServiceTag>(&VersionConstraint::range("^3").unwrap());
        assert!(matches!(result3, Err(RegistryError::ServiceNotFound)));
    }
}
