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

use std::any::{Any, TypeId};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use crate::context::ServiceContext;
use crate::contract::version::VersionConstraint;
use crate::di::{AdapterRef, ConfigValue, DepKey, Injectable, ProjectionRef};
use crate::error::ServiceError;
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
    /// Registered projection instances for dependency injection.
    projection_instances: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
    /// Registered adapter instances for dependency injection.
    adapter_instances: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
    /// Registered config instances for dependency injection.
    config_instances: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
    /// The tenant this runtime is bound to. `None` means system-level (no tenant).
    pub tenant_id: Option<String>,
    /// When `true`, all cross-tenant calls are allowed regardless of context.
    pub allow_cross_tenant: bool,
}

impl RuntimeInner {
    /// Creates a new `RuntimeInner`.
    pub fn new(registry: ServiceRegistry, interceptor_chain: Arc<InterceptorChain>) -> Self {
        Self {
            registry,
            interceptor_chain,
            projection_instances: HashMap::new(),
            adapter_instances: HashMap::new(),
            config_instances: HashMap::new(),
            tenant_id: None,
            allow_cross_tenant: false,
        }
    }

    /// Resolves a registered `ProjectionRef<T>` by type.
    ///
    /// Returns `DependencyNotFound` if no instance was registered for `T`.
    pub fn resolve_projection<T: 'static + Send + Sync>(
        &self,
    ) -> Result<ProjectionRef<T>, RuntimeError> {
        self.projection_instances
            .get(&TypeId::of::<T>())
            .and_then(|arc| arc.clone().downcast::<T>().ok())
            .map(ProjectionRef::new)
            .ok_or(RuntimeError::DependencyNotFound)
    }

    /// Resolves a registered `AdapterRef<A>` by type.
    ///
    /// Returns `DependencyNotFound` if no instance was registered for `A`.
    pub fn resolve_adapter<A: 'static + Send + Sync>(&self) -> Result<AdapterRef<A>, RuntimeError> {
        self.adapter_instances
            .get(&TypeId::of::<A>())
            .and_then(|arc| arc.clone().downcast::<A>().ok())
            .map(AdapterRef::new)
            .ok_or(RuntimeError::DependencyNotFound)
    }

    /// Resolves a registered `ConfigValue<C>` by type.
    ///
    /// Returns `DependencyNotFound` if no instance was registered for `C`.
    pub fn resolve_config<C: 'static + Send + Sync>(&self) -> Result<ConfigValue<C>, RuntimeError> {
        self.config_instances
            .get(&TypeId::of::<C>())
            .and_then(|arc| arc.clone().downcast::<C>().ok())
            .map(ConfigValue::new)
            .ok_or(RuntimeError::DependencyNotFound)
    }

    /// Enforces tenant isolation.
    ///
    /// Returns `Ok(())` when:
    /// - The runtime allows cross-tenant access (`allow_cross_tenant`), OR
    /// - The context's `allow_cross_tenant` is set, OR
    /// - Neither the runtime nor context has a tenant_id, OR
    /// - Both tenant IDs match.
    ///
    /// Returns `Err(CrossTenantDenied)` when enforcement is active and tenants differ.
    pub fn enforce_tenant(&self, ctx: &ServiceContext) -> Result<(), RuntimeError> {
        if self.allow_cross_tenant || ctx.allow_cross_tenant {
            return Ok(());
        }
        match (&self.tenant_id, &ctx.tenant_id) {
            (Some(rt_tenant), Some(ctx_tenant)) if rt_tenant != ctx_tenant => {
                Err(RuntimeError::CrossTenantDenied {
                    expected: rt_tenant.clone(),
                    actual: ctx_tenant.clone(),
                })
            }
            _ => Ok(()),
        }
    }
}

impl Default for RuntimeInner {
    fn default() -> Self {
        Self {
            registry: ServiceRegistry::new(),
            interceptor_chain: Arc::new(InterceptorChain::new()),
            projection_instances: HashMap::new(),
            adapter_instances: HashMap::new(),
            config_instances: HashMap::new(),
            tenant_id: None,
            allow_cross_tenant: false,
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
    /// A cross-tenant call was denied.
    CrossTenantDenied {
        /// The tenant the runtime expects.
        expected: String,
        /// The tenant the caller provided.
        actual: String,
    },
}

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RuntimeError::ServiceNotFound => write!(f, "service not found"),
            RuntimeError::DependencyNotFound => write!(f, "dependency not found"),
            RuntimeError::MissingDependency => write!(f, "missing dependency"),
            RuntimeError::DependencyCycle => write!(f, "dependency cycle detected"),
            RuntimeError::RegistryError(msg) => write!(f, "registry error: {msg}"),
            RuntimeError::CrossTenantDenied { expected, actual } => {
                write!(
                    f,
                    "cross-tenant call denied: expected tenant '{expected}', got '{actual}'"
                )
            }
        }
    }
}

impl std::error::Error for RuntimeError {}

/// Converts a `RuntimeError` into a `ServiceError` for use in generated proxy code.
impl From<RuntimeError> for ServiceError {
    fn from(e: RuntimeError) -> Self {
        match e {
            RuntimeError::CrossTenantDenied { expected, actual } => ServiceError::authorization(
                format!("cross-tenant call denied: expected tenant '{expected}', got '{actual}'"),
            ),
            other => ServiceError::internal(format!("runtime error: {other}")),
        }
    }
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

    /// Registered instances for dependency resolution during `build()`.
    projection_instances: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
    /// Registered instances for dependency resolution during `build()`.
    adapter_instances: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
    /// Registered instances for dependency resolution during `build()`.
    config_instances: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,

    /// Optional pre-configured service registry. Defaults to empty.
    registry: Option<ServiceRegistry>,

    /// Optional interceptor chain. Defaults to empty.
    interceptor_chain: Option<Arc<InterceptorChain>>,

    /// Optional tenant ID for this runtime.
    tenant_id: Option<String>,
    /// Whether cross-tenant access is allowed at the runtime level.
    allow_cross_tenant: bool,
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
            projection_instances: HashMap::new(),
            adapter_instances: HashMap::new(),
            config_instances: HashMap::new(),
            registry: None,
            interceptor_chain: None,
            tenant_id: None,
            allow_cross_tenant: false,
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

    /// Sets the tenant ID for this runtime. All services registered through
    /// this runtime are considered to belong to this tenant.
    pub fn with_tenant_id(mut self, tenant_id: impl Into<String>) -> Self {
        self.tenant_id = Some(tenant_id.into());
        self
    }

    /// Allows cross-tenant access at the runtime level. When set, `enforce_tenant`
    /// will permit all calls regardless of the caller's tenant context.
    pub fn allow_cross_tenant(mut self) -> Self {
        self.allow_cross_tenant = true;
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

    /// Registers a projection type AND stores the given instance for resolution.
    ///
    /// The type is also added to `projection_type_ids` so graph validation
    /// recognises `Projection<T>` as an available dependency.
    pub fn with_projection_value<P: 'static + Send + Sync>(mut self, instance: P) -> Self {
        let tid = TypeId::of::<P>();
        self.projection_type_ids.push(tid);
        self.projection_instances.insert(tid, Arc::new(instance));
        self.dependencies.push(Dependency {
            type_id: std::any::type_name::<P>().to_string(),
            name: None,
            version: None,
        });
        self
    }

    /// Registers an adapter type AND stores the given instance for resolution.
    pub fn with_adapter_value<A: 'static + Send + Sync>(mut self, instance: A) -> Self {
        let tid = TypeId::of::<A>();
        self.adapter_instances.insert(tid, Arc::new(instance));
        self.dependencies.push(Dependency {
            type_id: std::any::type_name::<A>().to_string(),
            name: None,
            version: None,
        });
        self
    }

    /// Registers a config value AND stores the given instance for resolution.
    pub fn with_config_value<C: 'static + Send + Sync>(mut self, instance: C) -> Self {
        let tid = TypeId::of::<C>();
        self.config_instances.insert(tid, Arc::new(instance));
        self.dependencies.push(Dependency {
            type_id: std::any::type_name::<C>().to_string(),
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

    /// Returns the topological sort order of the dependency graph.
    ///
    /// This is useful for tests that need to verify deterministic ordering.
    /// See `RuntimeBuilder::build` for validation details.
    pub fn sorted_order(&self) -> Result<Vec<TypeId>, RuntimeError> {
        self.validate_deps()
    }

    /// Validates the dependency graph and produces a `Runtime`.
    ///
    /// # Validation
    ///
    /// 1. **Missing dependencies** — every dep referenced by a service's
    ///    `Injectable::dependencies()` must be explicitly registered via
    ///    `with_projection`, `with_entity`, `with_adapter`, or `with_config`,
    ///    matching BOTH the `DepKey` variant AND the inner `TypeId`.
    ///
    ///    Services registered via `with_service` do NOT satisfy dependency
    ///    requirements — they are consumers, not providers.
    ///
    /// 2. **Cycle detection** — the directed graph is sorted with Kahn's
    ///    algorithm. If any node cannot be ordered, a `DependencyCycle`
    ///    error is returned.
    ///
    /// # Returns
    ///
    /// * `Ok(Runtime)` — graph is valid, runtime is ready for resolution.
    /// * `Err(RuntimeError::DependencyCycle)` — a cycle was detected.
    /// * `Err(RuntimeError::MissingDependency)` — a referenced dep was not
    ///   registered under the correct category.
    pub async fn build(self) -> Result<Runtime, RuntimeError> {
        let _order = self.validate_deps()?;

        let registry = self.registry.unwrap_or_default();
        let chain = self
            .interceptor_chain
            .unwrap_or_else(|| Arc::new(InterceptorChain::new()));

        let inner = Arc::new(RuntimeInner {
            registry,
            interceptor_chain: chain,
            projection_instances: self.projection_instances,
            adapter_instances: self.adapter_instances,
            config_instances: self.config_instances,
            tenant_id: self.tenant_id,
            allow_cross_tenant: self.allow_cross_tenant,
        });

        Ok(Runtime { inner })
    }

    /// Runs Kahn topological sort on the declared dependency graph.
    ///
    /// Deps are matched by `(DepKey variant, TypeId)` — a plain `TypeId`
    /// match is NOT enough.  For example, `Projection<T>` is NOT satisfied
    /// by `Entity<T>`.
    ///
    /// Returns `Ok(sorted_order)` if the graph is a valid DAG, where
    /// `sorted_order` is the topological order of all nodes in the graph.
    /// Returns `Err(DependencyCycle)` if a cycle is detected.
    /// Returns `Err(MissingDependency)` if a dep's (variant, TypeId) pair
    /// was not registered.
    fn validate_deps(&self) -> Result<Vec<TypeId>, RuntimeError> {
        // ---- Step 1: collect available deps by (variant, TypeId) ----
        // Services are tracked for cycle detection but do NOT satisfy
        // dependency requirements — they consume deps, not provide them.
        let mut registered_by_category: HashSet<DepKey> = HashSet::new();
        let mut all_type_ids: HashSet<TypeId> = HashSet::new();

        for node in &self.service_nodes {
            all_type_ids.insert(node.type_id);
        }
        for tid in &self.projection_type_ids {
            all_type_ids.insert(*tid);
            registered_by_category.insert(DepKey::Projection(*tid));
        }
        for tid in &self.entity_type_ids {
            all_type_ids.insert(*tid);
            registered_by_category.insert(DepKey::Entity(*tid));
        }

        // ---- Step 2: collect edges and check for missing deps ----
        let mut edges: Vec<(TypeId, TypeId)> = Vec::new();
        // Referenced TypeIds for cycle-detection node set.
        let mut referenced_type_ids: HashSet<TypeId> = HashSet::new();

        for node in &self.service_nodes {
            for dep in &node.deps {
                let tid = dep_key_type_id(dep);
                referenced_type_ids.insert(tid);
                edges.push((node.type_id, tid));

                // --- category-aware missing-dep check ---
                if !registered_by_category.contains(dep) {
                    return Err(RuntimeError::MissingDependency);
                }
            }
        }

        // ---- Step 3: build the full node set for cycle detection ----
        let mut all_nodes: HashSet<TypeId> = all_type_ids.clone();
        all_nodes.extend(&referenced_type_ids);

        // ---- Step 4: Kahn topological sort ----
        let mut deps_left: HashMap<TypeId, usize> = HashMap::new();
        let mut dependents: HashMap<TypeId, Vec<TypeId>> = HashMap::new();

        for node in &all_nodes {
            deps_left.entry(*node).or_insert(0);
            dependents.entry(*node).or_default();
        }

        for (from, to) in &edges {
            *deps_left.entry(*from).or_insert(0) += 1;
            dependents.entry(*to).or_default().push(*from);
        }

        let mut queue: VecDeque<TypeId> = deps_left
            .iter()
            .filter(|(_, &count)| count == 0)
            .map(|(node, _)| *node)
            .collect();

        let mut order: Vec<TypeId> = Vec::with_capacity(all_nodes.len());
        while let Some(node) = queue.pop_front() {
            order.push(node);
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

        if order.len() != all_nodes.len() {
            return Err(RuntimeError::DependencyCycle);
        }

        Ok(order)
    }
}

/// Returns the inner `TypeId` from any `DepKey` variant.
fn dep_key_type_id(dep: &DepKey) -> TypeId {
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

    /// A leaf projection type — no deps.
    struct ProjectionC;
    impl Injectable for ProjectionC {
        fn dependencies() -> Vec<DepKey> {
            vec![]
        }
        fn build(_rt: &RuntimeInner) -> Result<Self, RuntimeError> {
            Ok(ProjectionC)
        }
    }

    /// A projection type that depends on another projection.
    struct ProjectionB;
    impl Injectable for ProjectionB {
        fn dependencies() -> Vec<DepKey> {
            vec![DepKey::Projection(TypeId::of::<ProjectionC>())]
        }
        fn build(_rt: &RuntimeInner) -> Result<Self, RuntimeError> {
            Ok(ProjectionB)
        }
    }

    // ----------------------------------------------------------
    // Cycle-detection test types
    //   Services participate in cycle detection;
    //   dependencies must also be satisfied by category.
    // ----------------------------------------------------------

    struct ServiceA;
    impl Injectable for ServiceA {
        fn dependencies() -> Vec<DepKey> {
            vec![DepKey::Projection(TypeId::of::<ProjectionB>())]
        }
        fn build(_rt: &RuntimeInner) -> Result<Self, RuntimeError> {
            Ok(ServiceA)
        }
    }

    struct ServiceB;
    impl Injectable for ServiceB {
        fn dependencies() -> Vec<DepKey> {
            vec![DepKey::Projection(TypeId::of::<ProjectionC>())]
        }
        fn build(_rt: &RuntimeInner) -> Result<Self, RuntimeError> {
            Ok(ServiceB)
        }
    }

    struct ServiceOnlyC;
    impl Injectable for ServiceOnlyC {
        fn dependencies() -> Vec<DepKey> {
            vec![]
        }
        fn build(_rt: &RuntimeInner) -> Result<Self, RuntimeError> {
            Ok(ServiceOnlyC)
        }
    }

    /// A service that depends on itself (creates a self-cycle).
    struct SelfCycleService;
    impl Injectable for SelfCycleService {
        fn dependencies() -> Vec<DepKey> {
            vec![DepKey::Entity(TypeId::of::<SelfCycleService>())]
        }
        fn build(_rt: &RuntimeInner) -> Result<Self, RuntimeError> {
            Ok(SelfCycleService)
        }
    }

    // ------------------------------------------------------------------
    // Category mismatch test types
    // ------------------------------------------------------------------

    /// A struct used for category-mismatch testing.
    struct SomeType;

    /// A service that depends on Projection<SomeType>.
    struct ServiceProjectionDep;
    impl Injectable for ServiceProjectionDep {
        fn dependencies() -> Vec<DepKey> {
            vec![DepKey::Projection(TypeId::of::<SomeType>())]
        }
        fn build(_rt: &RuntimeInner) -> Result<Self, RuntimeError> {
            Ok(ServiceProjectionDep)
        }
    }

    /// A service that depends on Adapter<SomeType>.
    struct ServiceAdapterDep;
    impl Injectable for ServiceAdapterDep {
        fn dependencies() -> Vec<DepKey> {
            vec![DepKey::Adapter(TypeId::of::<SomeType>())]
        }
        fn build(_rt: &RuntimeInner) -> Result<Self, RuntimeError> {
            Ok(ServiceAdapterDep)
        }
    }

    /// A service that depends on Config<SomeType>.
    struct ServiceConfigDep;
    impl Injectable for ServiceConfigDep {
        fn dependencies() -> Vec<DepKey> {
            vec![DepKey::Config(TypeId::of::<SomeType>())]
        }
        fn build(_rt: &RuntimeInner) -> Result<Self, RuntimeError> {
            Ok(ServiceConfigDep)
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
        let builder = RuntimeBuilder::new().with_service::<ServiceOnlyC>();
        assert_eq!(builder.services.len(), 1);
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
    // TASK-013: Dependency graph validation (category-aware)
    // ------------------------------------------------------------------

    /// Valid graph: services depend on projections that are properly registered.
    #[tokio::test]
    async fn runtime_build_succeeds_for_valid_graph() {
        let result = RuntimeBuilder::new()
            .with_service::<ServiceA>() // A depends on Projection<B>
            .with_service::<ServiceB>() // B depends on Projection<C>
            .with_service::<ServiceOnlyC>() // no deps
            .with_projection::<ProjectionB>() // satisfies A's dep
            .with_projection::<ProjectionC>() // satisfies B's dep
            .build()
            .await;

        assert!(result.is_ok(), "valid graph must build: {:?}", result.err());
    }

    /// Self-cycle must fail.
    /// Dep is satisfied via Entity registration so cycle detection triggers.
    #[tokio::test]
    async fn runtime_build_fails_on_cycle() {
        let result = RuntimeBuilder::new()
            .with_service::<SelfCycleService>() // depends on Entity<SelfCycleService>
            .with_entity::<SelfCycleService>() // satisfies the dep → cycle detection fires
            .build()
            .await;

        assert!(
            matches!(result, Err(RuntimeError::DependencyCycle)),
            "self-cycle must yield DependencyCycle"
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

        // Both deps need projection registrations, but the cycle will
        // be detected before missing-dep because the graph is irreducible.
        let result = RuntimeBuilder::new()
            .with_service::<CycleA>()
            .with_service::<CycleB>()
            .with_projection::<CycleB>()
            .with_projection::<CycleA>()
            .build()
            .await;

        assert!(
            matches!(result, Err(RuntimeError::DependencyCycle)),
            "two-node cycle must yield DependencyCycle"
        );
    }

    /// Missing dependency: Projection<C> is not registered at all.
    #[tokio::test]
    async fn runtime_build_fails_on_missing_dependency() {
        struct NeedsProjection;
        impl Injectable for NeedsProjection {
            fn dependencies() -> Vec<DepKey> {
                vec![DepKey::Projection(TypeId::of::<ProjectionC>())]
            }
            fn build(_rt: &RuntimeInner) -> Result<Self, RuntimeError> {
                Ok(NeedsProjection)
            }
        }

        let result = RuntimeBuilder::new()
            .with_service::<NeedsProjection>()
            .build()
            .await;

        assert!(
            matches!(result, Err(RuntimeError::MissingDependency)),
            "unregistered dependency must yield MissingDependency"
        );
    }

    /// Dep satisfied by correctly-categorized registration.
    #[tokio::test]
    async fn runtime_build_succeeds_when_dep_is_projection() {
        struct NeedsProjection;
        impl Injectable for NeedsProjection {
            fn dependencies() -> Vec<DepKey> {
                vec![DepKey::Projection(TypeId::of::<ProjectionC>())]
            }
            fn build(_rt: &RuntimeInner) -> Result<Self, RuntimeError> {
                Ok(NeedsProjection)
            }
        }

        let result = RuntimeBuilder::new()
            .with_service::<NeedsProjection>()
            .with_projection::<ProjectionC>()
            .build()
            .await;

        assert!(
            result.is_ok(),
            "dep satisfied by matching category must build: {:?}",
            result.err()
        );
    }

    // ------------------------------------------------------------------
    // BLOCKER 2: DepKey category mismatch — same TypeId, wrong variant
    // ------------------------------------------------------------------

    /// Projection<T> MUST NOT be satisfied by Entity<T>.
    #[tokio::test]
    async fn runtime_build_fails_when_projection_satisfied_by_entity() {
        let result = RuntimeBuilder::new()
            .with_service::<ServiceProjectionDep>() // needs Projection<SomeType>
            .with_entity::<SomeType>() // Entity<SomeType> != Projection<SomeType>
            .build()
            .await;

        assert!(
            matches!(result, Err(RuntimeError::MissingDependency)),
            "Projection<T> must NOT be satisfied by Entity<T>"
        );
    }

    /// Projection<T> satisfied by Projection<T> — MUST work.
    #[tokio::test]
    async fn runtime_build_succeeds_projection_matched_by_projection() {
        let result = RuntimeBuilder::new()
            .with_service::<ServiceProjectionDep>() // needs Projection<SomeType>
            .with_projection::<SomeType>() // Projection<SomeType> ✓
            .build()
            .await;

        assert!(
            result.is_ok(),
            "Projection<T> must be satisfied by Projection<T>"
        );
    }

    /// Adapter<T> MUST NOT be satisfied by Projection<T>.
    #[tokio::test]
    async fn runtime_build_fails_when_adapter_satisfied_by_projection() {
        let result = RuntimeBuilder::new()
            .with_service::<ServiceAdapterDep>() // needs Adapter<SomeType>
            .with_projection::<SomeType>() // Projection<SomeType> != Adapter<SomeType>
            .build()
            .await;

        assert!(
            matches!(result, Err(RuntimeError::MissingDependency)),
            "Adapter<T> must NOT be satisfied by Projection<T>"
        );
    }

    /// Config<T> MUST NOT be satisfied by Entity<T>.
    #[tokio::test]
    async fn runtime_build_fails_when_config_satisfied_by_entity() {
        let result = RuntimeBuilder::new()
            .with_service::<ServiceConfigDep>() // needs Config<SomeType>
            .with_entity::<SomeType>() // Entity<SomeType> != Config<SomeType>
            .build()
            .await;

        assert!(
            matches!(result, Err(RuntimeError::MissingDependency)),
            "Config<T> must NOT be satisfied by Entity<T>"
        );
    }

    /// Entity<T> satisfied by Entity<T> — MUST work.
    #[tokio::test]
    async fn runtime_build_succeeds_entity_matched_by_entity() {
        struct EntityDepService;
        impl Injectable for EntityDepService {
            fn dependencies() -> Vec<DepKey> {
                vec![DepKey::Entity(TypeId::of::<SomeType>())]
            }
            fn build(_rt: &RuntimeInner) -> Result<Self, RuntimeError> {
                Ok(EntityDepService)
            }
        }

        let result = RuntimeBuilder::new()
            .with_service::<EntityDepService>() // needs Entity<SomeType>
            .with_entity::<SomeType>() // Entity<SomeType> ✓
            .build()
            .await;

        assert!(
            result.is_ok(),
            "Entity<T> must be satisfied by Entity<T>: {:?}",
            result.err()
        );
    }

    // ------------------------------------------------------------------
    // Tenant enforcement
    // ------------------------------------------------------------------

    #[test]
    fn enforce_tenant_same_tenant_allowed() {
        let rt = RuntimeInner {
            tenant_id: None,
            allow_cross_tenant: false,
            ..RuntimeInner::default()
        };
        let ctx = ServiceContext::new().with_tenant_id("tenant-a");
        assert!(rt.enforce_tenant(&ctx).is_ok());
    }

    #[test]
    fn enforce_tenant_tenants_match() {
        let rt = RuntimeInner {
            tenant_id: Some("tenant-a".to_string()),
            allow_cross_tenant: false,
            ..RuntimeInner::default()
        };
        let ctx = ServiceContext::new().with_tenant_id("tenant-a");
        assert!(rt.enforce_tenant(&ctx).is_ok());
    }

    #[test]
    fn enforce_tenant_cross_tenant_rejected() {
        let rt = RuntimeInner {
            tenant_id: Some("tenant-a".to_string()),
            allow_cross_tenant: false,
            ..RuntimeInner::default()
        };
        let ctx = ServiceContext::new().with_tenant_id("tenant-b");
        match rt.enforce_tenant(&ctx) {
            Err(RuntimeError::CrossTenantDenied { expected, actual }) => {
                assert_eq!(expected, "tenant-a");
                assert_eq!(actual, "tenant-b");
            }
            other => panic!("expected CrossTenantDenied, got: {other:?}"),
        }
    }

    #[test]
    fn enforce_tenant_context_allows_cross_tenant() {
        let rt = RuntimeInner {
            tenant_id: Some("tenant-a".to_string()),
            allow_cross_tenant: false,
            ..RuntimeInner::default()
        };
        let ctx = ServiceContext::new()
            .with_tenant_id("tenant-b")
            .allow_cross_tenant();
        assert!(rt.enforce_tenant(&ctx).is_ok());
    }

    #[test]
    fn enforce_tenant_runtime_allows_cross_tenant() {
        let rt = RuntimeInner {
            tenant_id: Some("tenant-a".to_string()),
            allow_cross_tenant: true,
            ..RuntimeInner::default()
        };
        let ctx = ServiceContext::new().with_tenant_id("tenant-b");
        assert!(rt.enforce_tenant(&ctx).is_ok());
    }

    #[test]
    fn enforce_tenant_no_runtime_tenant_skips() {
        let rt = RuntimeInner {
            tenant_id: None,
            allow_cross_tenant: false,
            ..RuntimeInner::default()
        };
        let ctx = ServiceContext::new().with_tenant_id("tenant-b");
        assert!(rt.enforce_tenant(&ctx).is_ok());
    }

    #[test]
    fn enforce_tenant_no_context_tenant_skips() {
        let rt = RuntimeInner {
            tenant_id: Some("tenant-a".to_string()),
            allow_cross_tenant: false,
            ..RuntimeInner::default()
        };
        let ctx = ServiceContext::new();
        assert!(rt.enforce_tenant(&ctx).is_ok());
    }
}
