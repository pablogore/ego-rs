//! Runtime builder, runtime state, and typed proxy resolution.
//!
//! # Architecture
//!
//! `RuntimeBuilder` collects service registrations and dependency declarations,
//! validates the dependency graph with Kahn topological sort, and produces a
//! `Runtime` that can resolve typed service proxies.
//!
//! `RuntimeInner` is shared state held by all generated proxies via `Weak<RuntimeInner>`.
//! It owns the `ServiceRegistry` and the `InterceptorChain`.

use std::any::TypeId;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use crate::context::ServiceContext;
use crate::contract::version::VersionConstraint;
use crate::di::{DepKey, Injectable};
use crate::interceptor::InterceptorChain;
use crate::registry::ServiceRegistry;

use super::resolvable::Resolvable;

// ---------------------------------------------------------------------------
// Shared runtime state
// ---------------------------------------------------------------------------

/// Shared state held by all generated proxies via `Weak<RuntimeInner>`.
#[derive(Debug)]
pub struct RuntimeInner {
    /// The type-keyed service registry holding raw implementations.
    pub registry: ServiceRegistry,
    /// The interceptor chain applied to every resolved proxy.
    pub interceptor_chain: Arc<InterceptorChain>,
}

impl RuntimeInner {
    /// Creates a new `RuntimeInner` with the given registry and interceptor chain.
    pub fn new(registry: ServiceRegistry, interceptor_chain: Arc<InterceptorChain>) -> Self {
        Self {
            registry,
            interceptor_chain,
        }
    }

    /// Enforces tenant isolation — currently a stub.
    pub fn enforce_tenant(&self, _ctx: &ServiceContext) {}
}

impl Default for RuntimeInner {
    fn default() -> Self {
        Self {
            registry: ServiceRegistry::new(),
            interceptor_chain: Arc::new(InterceptorChain::new()),
        }
    }
}

// ---------------------------------------------------------------------------
// Dependency declarations
// ---------------------------------------------------------------------------

/// A dependency declaration used during builder configuration.
#[derive(Debug, Clone)]
pub struct Dependency {
    pub type_id: String,
    pub name: Option<String>,
    pub version: Option<String>,
}

// ---------------------------------------------------------------------------
// Runtime errors
// ---------------------------------------------------------------------------

/// Errors that can occur during build or resolution.
#[derive(Debug, Clone)]
pub enum RuntimeError {
    /// The requested service was not found in the registry.
    ServiceNotFound,
    /// A dependency was not found during graph validation.
    DependencyNotFound,
    /// A dependency referenced by a service was not registered (graph validation).
    MissingDependency,
    /// A cycle was detected in the dependency graph.
    DependencyCycle,
    /// The registry returned an error during registration or resolution.
    RegistryError(String),
}

// ---------------------------------------------------------------------------
// Runtime — the resolved, usable runtime
// ---------------------------------------------------------------------------

/// A runtime capable of resolving typed service proxies.
///
/// Created by [`RuntimeBuilder::build`]. Use [`Runtime::resolve`] to obtain
/// generated proxies for registered services.
#[derive(Clone)]
pub struct Runtime {
    inner: Arc<RuntimeInner>,
}

impl std::fmt::Debug for Runtime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Runtime").finish_non_exhaustive()
    }
}

impl Runtime {
    /// Resolves a typed service proxy for the given tag type.
    ///
/// # Type Parameters
///
/// * `T` — A tag type (e.g. `OrderServiceTag`) that implements `Resolvable`.
///   The tag connects the registry key to its proxy type.
    ///
    /// # Returns
    ///
    /// A generated proxy (e.g. `OrderServiceRef`) that wraps the registered
    /// implementation, the interceptor chain, and a weak runtime reference.
    ///
    /// # Errors
    ///
    /// Returns `RuntimeError::ServiceNotFound` if no matching implementation
    /// is registered under the tag's `TypeId`.
    ///
    /// Returns `RuntimeError::DependencyNotFound` if the registered implementation
    /// cannot be downcast to the expected trait.
    ///
    /// # Flow
    ///
    /// 1. Looks up the raw `Arc<dyn Any>` from the registry by `TypeId::of::<T>()`
    /// 2. Calls `T::create_proxy(...)` to downcast and construct the typed proxy
    /// 3. Returns the proxy
    pub fn resolve<T: Resolvable + 'static>(&self) -> Result<T::Proxy, RuntimeError> {
        // Use latest-compatible version resolution — pick the highest registered version.
        let constraint = VersionConstraint::LatestCompatible(
            semver::VersionReq::parse("*")
                .map_err(|e| RuntimeError::RegistryError(format!("invalid version req: {e}")))?,
        );

        let raw = self
            .inner
            .registry
            .resolve_raw::<T>(&constraint)
            .map_err(|e| match e {
                crate::registry::RegistryError::ServiceNotFound => RuntimeError::ServiceNotFound,
                crate::registry::RegistryError::DependencyNotFound => {
                    RuntimeError::DependencyNotFound
                }
                crate::registry::RegistryError::DependencyCycle => RuntimeError::DependencyCycle,
                crate::registry::RegistryError::DuplicateService { name, version } => {
                    RuntimeError::RegistryError(format!("duplicate {name} v{version}"))
                }
            })?;

        T::create_proxy(
            raw,
            self.inner.interceptor_chain.clone(),
            Arc::downgrade(&self.inner),
        )
    }

    /// Access the inner runtime (for interceptor registration, etc.).
    pub fn inner(&self) -> &Arc<RuntimeInner> {
        &self.inner
    }
}

// ---------------------------------------------------------------------------
// Internal graph node
// ---------------------------------------------------------------------------

/// A registered service node holding its dependency list.
#[derive(Debug, Clone)]
struct ServiceNode {
    type_id: TypeId,
    #[allow(dead_code)]
    type_name: String,
    deps: Vec<DepKey>,
}

// ---------------------------------------------------------------------------
// RuntimeBuilder
// ---------------------------------------------------------------------------

/// Builder for constructing and validating a service [`Runtime`].
///
/// Collects service registrations, projections, and entities, then validates
/// the dependency graph and produces a `Runtime` capable of resolving typed
/// service proxies.
///
/// # Example
///
/// ```ignore
/// let runtime = RuntimeBuilder::new()
///     .with_service::<ServiceA>()
///     .with_service::<ServiceB>()
///     .with_projection::<MyProjection>()
///     .build()
///     .await
///     .unwrap();
/// ```
pub struct RuntimeBuilder {
    /// Type-name strings for registered services (backward compat).
    pub services: Vec<String>,
    /// Dependency declarations from `with_entity`, `with_projection`, `with_service_bundle`.
    pub dependencies: Vec<Dependency>,

    // Internal tracking for graph validation and proxy resolution.
    service_nodes: Vec<ServiceNode>,
    projection_type_ids: Vec<TypeId>,
    entity_type_ids: Vec<TypeId>,

    /// Optional pre-configured service registry. Defaults to empty.
    registry: Option<ServiceRegistry>,

    /// Optional interceptor chain. Defaults to empty.
    interceptor_chain: Option<Arc<InterceptorChain>>,
}

impl Default for RuntimeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeBuilder {
    /// Creates a new, empty `RuntimeBuilder`.
    pub fn new() -> Self {
        Self {
            services: Vec::new(),
            dependencies: Vec::new(),
            service_nodes: Vec::new(),
            projection_type_ids: Vec::new(),
            entity_type_ids: Vec::new(),
            registry: None,
            interceptor_chain: None,
        }
    }

    /// Attaches a pre-configured `ServiceRegistry`.
    ///
    /// The registry holds service implementations registered via
    /// `ServiceRegistry::register`. Implementations are stored as
    /// `Arc<ResolvableContainer<dyn Trait>>` wrapped in `Arc<dyn Any>`.
    pub fn with_registry(mut self, registry: ServiceRegistry) -> Self {
        self.registry = Some(registry);
        self
    }

    /// Attaches an `InterceptorChain` that will be injected into every
    /// resolved proxy.
    pub fn with_interceptor_chain(mut self, chain: Arc<InterceptorChain>) -> Self {
        self.interceptor_chain = Some(chain);
        self
    }

    /// Registers a service whose `Injectable::dependencies()` will be used
    /// for dependency graph validation.
    ///
    /// The type `T` must implement `Injectable` (generated by `#[service]`
    /// on structs, or implemented manually for testing).
    pub fn with_service<T: Injectable + 'static>(mut self) -> Self {
        let type_id = TypeId::of::<T>();
        let deps = T::dependencies();
        let type_name = std::any::type_name::<T>().to_string();

        self.service_nodes.push(ServiceNode {
            type_id,
            type_name: type_name.clone(),
            deps,
        });
        self.services.push(type_name);
        self
    }

    /// Registers a dependency on a projection type.
    pub fn with_projection<P: 'static>(mut self) -> Self {
        self.projection_type_ids.push(TypeId::of::<P>());
        self.dependencies.push(Dependency {
            type_id: std::any::type_name::<P>().to_string(),
            name: None,
            version: None,
        });
        self
    }

    /// Registers a dependency on an entity type.
    pub fn with_entity<E: 'static>(mut self) -> Self {
        self.entity_type_ids.push(TypeId::of::<E>());
        self.dependencies.push(Dependency {
            type_id: std::any::type_name::<E>().to_string(),
            name: None,
            version: None,
        });
        self
    }

    /// Registers a named service bundle as an available dependency.
    pub fn with_service_bundle(mut self, bundle: &str) -> Self {
        self.dependencies.push(Dependency {
            type_id: bundle.to_string(),
            name: None,
            version: None,
        });
        self
    }

    // ------------------------------------------------------------------
    // Graph validation (Kahn topological sort)
    // ------------------------------------------------------------------

    /// Validates the dependency graph and produces a `Runtime`.
    ///
    /// # Validation
    ///
    /// 1. **Missing dependencies** — every dep `TypeId` referenced by a
    ///    service's `Injectable::dependencies()` must be explicitly registered
    ///    via `with_service`, `with_projection`, or `with_entity`.
    ///
    /// 2. **Cycle detection** — the directed graph is sorted with Kahn's
    ///    algorithm. If any node cannot be ordered, a `DependencyCycle` error
    ///    is returned.
    ///
    /// # Returns
    ///
    /// * `Ok(Runtime)` — graph is valid, runtime is ready for resolution.
    /// * `Err(RuntimeError::DependencyCycle)` — a cycle was detected.
/// * `Err(RuntimeError::MissingDependency)` — a referenced dependency
///   was not registered.
    pub async fn build(self) -> Result<Runtime, RuntimeError> {
        self.validate_deps()?;

        let registry = self.registry.unwrap_or_default();
        let chain = self
            .interceptor_chain
            .unwrap_or_else(|| Arc::new(InterceptorChain::new()));

        let inner = Arc::new(RuntimeInner {
            registry,
            interceptor_chain: chain,
        });

        Ok(Runtime { inner })
    }

    /// Runs Kahn topological sort on the declared dependency graph.
    ///
    /// Returns `Ok(())` if the graph is a valid DAG.
    /// Returns `Err(DependencyCycle)` if a cycle is detected.
    /// Returns `Err(MissingDependency)` if a dependency references an
    /// unregistered type.
    fn validate_deps(&self) -> Result<(), RuntimeError> {
        // ---- Step 1: collect all explicitly registered TypeIds ----
        let mut available: HashSet<TypeId> = HashSet::new();

        for node in &self.service_nodes {
            available.insert(node.type_id);
        }
        for tid in &self.projection_type_ids {
            available.insert(*tid);
        }
        for tid in &self.entity_type_ids {
            available.insert(*tid);
        }

        // ---- Step 2: collect edges and check for missing deps ----
        // Edge (from, to) means "from depends on to".
        let mut edges: Vec<(TypeId, TypeId)> = Vec::new();
        let mut referenced: HashSet<TypeId> = HashSet::new();

        for node in &self.service_nodes {
            for dep in &node.deps {
                let dep_type_id = dep_type_to_id(dep);
                referenced.insert(dep_type_id);
                edges.push((node.type_id, dep_type_id));
            }
        }

        // Missing dependency: referenced by a service but not registered.
        for dep_id in &referenced {
            if !available.contains(dep_id) {
                return Err(RuntimeError::MissingDependency);
            }
        }

        // ---- Step 3: build the full node set for cycle detection ----
        let mut all_nodes: HashSet<TypeId> = available.clone();
        all_nodes.extend(&referenced);

        // ---- Step 4: Kahn topological sort ----
        // deps_left[node] = number of dependencies this node is waiting on.
        let mut deps_left: HashMap<TypeId, usize> = HashMap::new();
        let mut dependents: HashMap<TypeId, Vec<TypeId>> = HashMap::new();

        for node in &all_nodes {
            deps_left.entry(*node).or_insert(0);
            dependents.entry(*node).or_default();
        }

        for (from, to) in &edges {
            // `from` depends on `to` → `from` is waiting on `to`.
            *deps_left.entry(*from).or_insert(0) += 1;
            dependents.entry(*to).or_default().push(*from);
        }

        // Seed queue with nodes that have zero pending dependencies.
        let mut queue: VecDeque<TypeId> = deps_left
            .iter()
            .filter(|(_, &count)| count == 0)
            .map(|(node, _)| *node)
            .collect();

        let mut processed = 0;
        while let Some(node) = queue.pop_front() {
            processed += 1;
            // When a node is resolved, decrement deps_left for everything
            // that depends on it.
            if let Some(deps_of_node) = dependents.get(&node) {
                for dependent in deps_of_node {
                    if let Some(count) = deps_left.get_mut(dependent) {
                        *count = count.saturating_sub(1);
                        if *count == 0 {
                            queue.push_back(*dependent);
                        }
                    }
                }
            }
        }

        if processed != all_nodes.len() {
            return Err(RuntimeError::DependencyCycle);
        }

        Ok(())
    }
}

/// Extracts the inner `TypeId` from any `DepKey` variant.
fn dep_type_to_id(dep: &DepKey) -> TypeId {
    match dep {
        DepKey::Entity(t) | DepKey::Projection(t) | DepKey::Adapter(t) | DepKey::Config(t) => *t,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::di::{DepKey, Injectable};

    // ------------------------------------------------------------------
    // Test types
    // ------------------------------------------------------------------

    struct ServiceC;
    impl Injectable for ServiceC {
        fn dependencies() -> Vec<DepKey> {
            vec![]
        }
        fn build(_rt: &RuntimeInner) -> Result<Self, RuntimeError> {
            Ok(ServiceC)
        }
    }

    struct ServiceB;
    impl Injectable for ServiceB {
        fn dependencies() -> Vec<DepKey> {
            vec![DepKey::Projection(TypeId::of::<ServiceC>())]
        }
        fn build(_rt: &RuntimeInner) -> Result<Self, RuntimeError> {
            Ok(ServiceB)
        }
    }

    struct ServiceA;
    impl Injectable for ServiceA {
        fn dependencies() -> Vec<DepKey> {
            vec![DepKey::Projection(TypeId::of::<ServiceB>())]
        }
        fn build(_rt: &RuntimeInner) -> Result<Self, RuntimeError> {
            Ok(ServiceA)
        }
    }

    /// A service that depends on itself (creates a cycle).
    struct SelfCycleService;
    impl Injectable for SelfCycleService {
        fn dependencies() -> Vec<DepKey> {
            vec![DepKey::Entity(TypeId::of::<SelfCycleService>())]
        }
        fn build(_rt: &RuntimeInner) -> Result<Self, RuntimeError> {
            Ok(SelfCycleService)
        }
    }

    /// A service with an unregistered dependency.
    struct MissingDepService;
    impl Injectable for MissingDepService {
        fn dependencies() -> Vec<DepKey> {
            vec![DepKey::Projection(TypeId::of::<ServiceC>())]
        }
        fn build(_rt: &RuntimeInner) -> Result<Self, RuntimeError> {
            Ok(MissingDepService)
        }
    }

    // ------------------------------------------------------------------
    // RuntimeBuilder creation
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_runtime_builder_creation() {
        let builder = RuntimeBuilder::new();
        assert_eq!(builder.services.len(), 0);
        assert_eq!(builder.dependencies.len(), 0);
    }

    #[tokio::test]
    async fn test_with_service() {
        let builder = RuntimeBuilder::new().with_service::<ServiceA>();
        assert_eq!(builder.services.len(), 1);
        assert!(builder.services[0].contains("ServiceA"));
    }

    #[tokio::test]
    async fn test_with_entity() {
        struct TestEntity;
        let builder = RuntimeBuilder::new().with_entity::<TestEntity>();
        assert_eq!(builder.dependencies.len(), 1);
    }

    #[tokio::test]
    async fn test_with_projection() {
        struct TestProjection;
        let builder = RuntimeBuilder::new().with_projection::<TestProjection>();
        assert_eq!(builder.dependencies.len(), 1);
    }

    #[tokio::test]
    async fn test_with_service_bundle() {
        let builder = RuntimeBuilder::new().with_service_bundle("test-bundle");
        assert_eq!(builder.dependencies.len(), 1);
    }

    // ------------------------------------------------------------------
    // TASK-013: Dependency graph validation
    // ------------------------------------------------------------------

    /// Valid graph: A -> B -> C  (all registered, no cycles).
    #[tokio::test]
    async fn runtime_build_succeeds_for_valid_graph() {
        let result = RuntimeBuilder::new()
            .with_service::<ServiceA>() // A depends on B
            .with_service::<ServiceB>() // B depends on C
            .with_service::<ServiceC>() // no deps
            .build()
            .await;

        assert!(
            result.is_ok(),
            "valid graph (A -> B -> C) must build: {:?}",
            result.err()
        );
    }

    /// Cycle: A -> B -> A  → must fail with DependencyCycle.
    #[tokio::test]
    async fn runtime_build_fails_on_cycle() {
        // Self-cycle: SelfCycleService depends on itself.
        let result = RuntimeBuilder::new()
            .with_service::<SelfCycleService>()
            .build()
            .await;

        assert!(
            matches!(result, Err(RuntimeError::DependencyCycle)),
            "self-cycle must yield DependencyCycle: {:?}",
            result
        );
    }

    /// Two-node cycle: A -> B, B -> A.
    #[tokio::test]
    async fn runtime_build_fails_on_two_node_cycle() {
        struct CycleA;
        impl Injectable for CycleA {
            fn dependencies() -> Vec<DepKey> {
                vec![DepKey::Projection(TypeId::of::<CycleB>())]
            }
            fn build(_rt: &RuntimeInner) -> Result<Self, RuntimeError> {
                Ok(CycleA)
            }
        }

        struct CycleB;
        impl Injectable for CycleB {
            fn dependencies() -> Vec<DepKey> {
                vec![DepKey::Projection(TypeId::of::<CycleA>())]
            }
            fn build(_rt: &RuntimeInner) -> Result<Self, RuntimeError> {
                Ok(CycleB)
            }
        }

        let result = RuntimeBuilder::new()
            .with_service::<CycleA>()
            .with_service::<CycleB>()
            .build()
            .await;

        assert!(
            matches!(result, Err(RuntimeError::DependencyCycle)),
            "two-node cycle must yield DependencyCycle"
        );
    }

    /// Missing dependency: A depends on C, but C is not registered.
    #[tokio::test]
    async fn runtime_build_fails_on_missing_dependency() {
        let result = RuntimeBuilder::new()
            .with_service::<MissingDepService>() // depends on C, but C not registered
            .build()
            .await;

        assert!(
            matches!(result, Err(RuntimeError::MissingDependency)),
            "missing dependency must yield MissingDependency: {:?}",
            result
        );
    }

    /// Missing dependency where dep IS registered as a projection.
    #[tokio::test]
    async fn runtime_build_succeeds_when_dep_is_projection() {
        let result = RuntimeBuilder::new()
            .with_service::<MissingDepService>() // needs Projection<ServiceC>
            .with_projection::<ServiceC>() // ServiceC is available as projection
            .build()
            .await;

            assert!(
            result.is_ok(),
            "dep satisfied by projection must build: {:?}",
            result.err()
        );
    }
}
