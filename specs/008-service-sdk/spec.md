# Feature Specification: Service SDK

**Feature**: `008-service-sdk`

**Created**: 2026-06-08

**Status**: Draft

**Input**: User description: "CORE-008 — Service SDK: Define the Service SDK model for ego.rs to declare application services, register their implementations, and connect them to the runtime without imposing any transport protocol."

## Clarifications

### Session 2026-06-08

- Q: Contract Descriptor Ownership — Should contract metadata be defined as service-only descriptors or as explicit contract descriptors (services, operations, requests, responses, fields)? → A: Option B — Explicit Contract Descriptors. Services, operations, requests, and responses all expose descriptors (ServiceDescriptor, OperationDescriptor, ContractDescriptor, FieldDescriptor). This becomes the canonical source of truth for all transports and tooling (OpenAPI, GraphQL, Protobuf, SDK generation).
- Q: Service Metadata Generation Model — How should service metadata be produced: manual descriptors, runtime introspection, or generated from attributes? → A: Option C — Generated Metadata (Code First). Developers declare services with attributes (e.g., `#[service]`, `#[operation]`). The framework automatically generates ServiceDescriptor and OperationDescriptor. Developers never write descriptors manually. Transport adapters consume generated metadata (e.g., `AxumTransport::from_registry(&registry)`). This follows a "Code First, Metadata Generated, Transport Agnostic" design principle.
- Q: Service Construction and Dependency Injection Model — How should services be instantiated with their dependencies: manual factory registration, runtime dependency resolution, or external DI container? → A: Option B — Runtime Dependency Resolution. Services declare dependencies through annotated fields (e.g., `EntityRef<T>`, `Arc<ReadStore>`). The framework generates dependency resolution metadata. The runtime constructs services during startup through the builder pattern (e.g., `EgoRuntime::builder().with_entity::<T>().with_service::<S>().build()`). Developers never manually wire large dependency graphs. This follows a "Services declare dependencies, Runtime resolves dependencies" design principle.
- Q: Service Lifecycle Hooks — Should application services expose init() and shutdown() hooks? → A: Option A — No lifecycle hooks. Application services MUST NOT expose init() or shutdown() hooks. Services are fully initialized once dependency resolution and field injection complete. Lifecycle management belongs to runtime-managed components (ProjectionRunner, Scheduler, TransportAdapter, EventStreamConsumer, ConnectionPool), which MAY implement lifecycle contracts. Application services remain stateless, constructor-ready, transport-agnostic, and easy to test. The runtime stops accepting new requests, waits for in-flight operations to complete, then releases references.
- Q: Service Invocation Observability — Should the service SDK include built-in observability hooks? → A: Option B — Optional invocation interceptors at the registry/runtime boundary. Services remain pure business logic. Interceptors are pluggable and optional, supporting tracing spans, invocation metrics, error counters, structured logs, correlation IDs, and tenant context propagation. Interceptors MUST NOT require every service to implement middleware hooks. Transport adapters add protocol-specific telemetry; runtime-level interceptors make internal service-to-service calls observable.
- Q: Service Invocation Concurrency Model — How should concurrent invocations of the same service be handled? → A: Option A — No service-level concurrency control. Application services MUST NOT introduce their own execution model or concurrency guarantees. Services are invoked concurrently by callers. The SDK does not provide service mailboxes, actors, or serialization. Entity execution remains single-threaded per entity (CORE-006). Services SHOULD be stateless; mutable shared state is the implementor's responsibility. Design principle: Services orchestrate. Entities protect consistency. The runtime enforces concurrency where state exists.
- Q: Service Context Propagation — How should invocation-scoped metadata (tenant ID, correlation ID, trace ID, deadlines, request metadata) be exposed to services? → A: Option C — Runtime-Provided Invocation Context. The runtime automatically propagates a ServiceContext for every invocation. Services access the current context through a runtime API (e.g., `ServiceContext::current().tenant_id()`). Context belongs to the invocation, not the service instance. Service contracts remain clean with no parameter pollution. Compatible with all transports. Design principle: Context travels with the invocation, not the service singleton.
- Q: Service Version Resolution — How should service resolution behave when multiple versions of the same service contract exist? → A: Option C — Contract Type + Name + Version. Services may coexist under contract type, logical name, and version (e.g., OrderService v1 and OrderService v2). Supports migrations, blue/green evolution, and parallel contract evolution. Resolution accepts contract type plus optional version constraint; latest version is returned when no version is specified.
- Q: Contract Versioning Strategy — How should contract versions be defined and validated: developer-managed strings, semantic versioning with framework enforcement, or sequential integers? → A: Option A — Developer-managed semantic version strings (MAJOR.MINOR.PATCH). The framework stores version metadata and exposes it through ContractDescriptor and ServiceDescriptor but does NOT perform compatibility analysis or enforce versioning rules. Compatibility validation (breaking vs. non-breaking change detection) belongs to future tooling, CI validation, SDK generators, and contract governance tooling. Design principle: CORE-008 provides contract metadata. Contract governance belongs to future tooling layers.
- Q: Service Error Model — Should services use service-defined errors, a shared framework error enum, or a hierarchical error model? → A: Option C — Hierarchical error model. The framework MUST NOT provide a mandatory shared error enum. Services define their own domain-specific error types. Domain errors SHOULD implement a common framework error trait exposing metadata (error code, error category). Interceptors and transport adapters operate against the common trait. The framework MAY define common error categories (Validation, NotFound, Conflict, Authorization, BusinessRule, Infrastructure) without forcing services into a shared taxonomy. Design principle: Error taxonomy belongs to the domain. Error metadata belongs to the framework.
- Q: Service Invocation API — How do callers (transport adapters, internal services) invoke a service operation: registry-mediated, direct reference, or command bus? → A: Option A — Registry-mediated invocation via ServiceRef<T> handles. Callers resolve a ServiceRef<T> from the registry and call operations directly on the handle. The handle wraps the underlying implementation, applying context propagation, interceptors, tracing, and metrics transparently. Both internal service-to-service calls and external transport invocations use the same ServiceRef<T> abstraction. Design principle: EntityRef<T> defines the entity boundary. ServiceRef<T> defines the service boundary.
- Q: Multi-Tenant Service Isolation — Should the runtime enforce tenant isolation on entity references, repositories, read-side handlers, and service dependencies, or leave tenant awareness to each service? → A: Option B — Runtime-Enforced Isolation. TenantContext from ServiceContext is propagated automatically. EntityRef<T>, repositories, read-side handlers, and service dependencies automatically operate inside the current tenant boundary. Cross-tenant access requires explicit opt-in. This provides strong isolation, safer defaults, and consistent behavior without relying on each service to remember tenant scoping.
- Q: Deadline and Cancellation Propagation — How should deadline expiration and cancellation propagate through service invocations? → A: Option A — Runtime Propagation Only. ServiceContext carries deadline, timeout, and cancellation token. The runtime automatically propagates these across ServiceRef calls, service-to-service calls, EntityRef calls, and interceptor chains. The runtime MAY terminate execution when a deadline expires or cancellation is received. Services are not required to check deadlines manually. Design principle: Cancellation and deadlines belong to the invocation, not the service implementation.
- Q: Developer Experience Validation — How should implementation success be validated beyond functional requirements? → A: Option B — DX Acceptance Example. Implementation MUST include one or more acceptance examples demonstrating the intended developer experience (declare contract via `#[service]`, implement struct with field dependencies, build runtime via builder pattern). These examples become validation artifacts and regression targets for API ergonomics, protecting the code-first design principle and preventing framework complexity creep.
- Q: EntityRef Context Propagation Model — How should EntityRef<T> obtain invocation context (tenant, trace, deadline, cancellation) when a service sends commands to an entity? → A: Option B — Runtime Scope Propagation. EntityRef<T> remains API-compatible with CORE-006 — no API changes, no wrappers, no new required parameters. Service implementations MUST NOT manually pass tenant_id, trace_id, correlation_id, deadline, timeout, or cancellation information to EntityRef operations. The runtime binds ServiceContext to EntityRef execution boundaries transparently via the invocation scope (TaskLocal). EntityRef<T> executes inside the current runtime scope. EntityRef<T> MUST NOT depend directly on service-sdk types. EntityRef<T> MUST NOT become transport-aware. Tenant isolation is runtime-enforced; cross-tenant access requires explicit opt-in. Design principle: Context belongs to the invocation scope, not to service implementations and not to entity method signatures.
- Q: #[service] Trait vs Struct Semantics — Should the same #[service] attribute be used on both traits (contracts) and structs (implementations), or should separate attributes exist? → A: Option B — Single #[service] attribute on both traits and structs. Macro behavior is determined by target type. Trait target generates: ServiceDescriptor, OperationDescriptor, ContractDescriptor metadata. Struct target generates: DependencyMetadata, DependencyGraph metadata, Injectable implementation metadata, Runtime wiring metadata. Developers MUST NOT manually define descriptors or dependency metadata. All generated metadata is transport-agnostic. Design principle: Developers focus on business logic. Metadata is generated automatically.
- Q: ServiceContext Access API — Should ServiceContext::current() panic outside an invocation scope, return Option, or return Result? → A: Option B — Safe Context Access. ServiceContext::current() returns Option<&ServiceContext>. Returns Some(context) inside a valid invocation scope; returns None outside. MUST NOT panic. MUST NOT terminate runtime execution. Missing invocation context is a valid runtime state — tests, CLI tools, migrations, and background jobs MAY execute without a ServiceContext. Services MAY safely inspect context existence. Design principle: Missing invocation context is a recoverable condition, not a fatal runtime error.

## User Scenarios & Testing *(mandatory)*

### User Story 1 — Declare an Application Service Contract (Priority: P1)

A developer defines the logical boundary of a business capability — the operations the application exposes — as a named contract. The contract declares what operations exist and what inputs/outputs they have, without any knowledge of how requests arrive or how the service is invoked.

**Why this priority**: Service contracts are the foundation. Without them, there is no standard way to define what capabilities an application provides.

**Independent Test**: Can be tested by defining a contract for an order management service with create-order and get-order operations, and verifying the contract type can be referenced by other components.

**Acceptance Scenarios**:

1. **Given** a developer wants to expose order management capabilities, **When** they declare a service contract with operations `create_order` and `get_order`, **Then** the contract is recognized as a valid service definition without any transport annotations.
2. **Given** a service contract declares an operation that returns a result, **When** the contract is compiled, **Then** the operation accepts domain types (commands, queries) and returns domain types (identifiers, views, errors) without any serialization layer.
3. **Given** two separate service contracts (e.g., OrderService and CustomerService), **When** a component references both, **Then** each contract is independently identifiable by its type and name.

---

### User Story 2 — Resolve Services From a Central Registry (Priority: P1)

A developer registers service implementations in a central registry and later resolves them by their contract type. The registry validates that all declared dependencies are satisfied and rejects duplicate or conflicting registrations.

**Why this priority**: Registry is the mechanism through which the runtime discovers and connects services. Without it, services cannot be composed into an application.

**Independent Test**: Can be tested by registering two service implementations, resolving each by their contract type, and verifying the correct implementation is returned.

**Acceptance Scenarios**:

1. **Given** an order service implementation, **When** it is registered under the OrderService contract type, **Then** a subsequent lookup by OrderService contract returns the registered implementation.
2. **Given** two implementations registered for the same contract type, **When** the second is registered, **Then** the registry rejects the duplicate with a clear error indicating the conflict.
3. **Given** a service implementation that depends on another service not yet registered, **When** validation runs, **Then** the registry reports the missing dependency by contract type.
4. **Given** multiple service modules (e.g., orders module, payments module), **When** each module registers its own services in a shared registry, **Then** all services are available for cross-module resolution.

---

### User Story 3 — Wire Services With Runtime Dependencies (Priority: P2)

A developer wires a service implementation with the runtime's core capabilities: persistent entities (to send commands to event-sourced entities), read-side projections (to query materialized views), repositories, configuration, and external adapters. Dependencies are declared as annotated fields on the service struct; the runtime resolves them automatically at construction time through a builder pattern.

**Why this priority**: Services are the integration point between domain logic and runtime infrastructure. Without this wiring capability, services would be isolated from the entity and read-side layers.

**Independent Test**: Can be tested by constructing a service implementation that receives a persistent entity reference and a read-side handler, sending a command through the entity reference, and verifying the entity processes it.

**Acceptance Scenarios**:

1. **Given** a service implementation for order creation, **When** it receives a persistent entity reference for orders, **Then** it can send commands to order entities by their identifier and receive results.
2. **Given** a service implementation for order queries, **When** it receives a read-side handler reference, **Then** it can query the read-side for order views without accessing the entity event stream directly.
3. **Given** a service that depends on an external adapter (e.g., a notification client), **When** the adapter is injected at construction time, **Then** the service can delegate external communication to the adapter through its contract.
4. **Given** a service that requires configuration values, **When** configuration is injected at construction time, **Then** the service uses those values without hardcoding environment-specific settings.

---

### User Story 4 — Compose the Runtime From Wired Modules (Priority: P2)

A developer assembles the complete runtime by providing entities, read-side projections, and wired services. The runtime starts with all components connected and ready. The runtime validates that all required services are present before starting.

**Why this priority**: Runtime composition is the final step that makes the application operational. Without it, all the individually defined components remain disconnected.

**Independent Test**: Can be tested by building a runtime with a minimal set of entities and services, starting it, and verifying the runtime enters an operational state without errors.

**Acceptance Scenarios**:

1. **Given** a set of entity definitions, read-side projections, and wired services, **When** the runtime is built and started, **Then** all components are connected and the runtime reaches an operational state.
2. **Given** a runtime configuration that references a service not yet registered, **When** the runtime attempts to start, **Then** startup fails with a clear error identifying the missing service.
3. **Given** a runtime started with services, entities, and adapters, **When** an external transport adapter invokes a service, **Then** the service executes its logic using the wired runtime dependencies.

---

### User Story 5 — Adapt Services to External Transports (Priority: P3)

A developer or external transport author creates a transport adapter that invokes service operations. The adapter translates incoming requests (HTTP, gRPC, GraphQL, CLI commands, message queue messages) into service operation calls and translates results back to transport responses. The service itself has no knowledge of the transport.

**Why this priority**: Transport adaptation validates the claim that services are transport-agnostic. It is the external-facing proof that the SDK achieves clean separation. However, the transports themselves are outside CORE-008 scope, making this a validation scenario rather than a deliverable.

**Independent Test**: Can be tested by creating a lightweight test adapter that invokes a service operation and verifies the service executes correctly, then replacing it with a different adapter and verifying the same service works unchanged.

**Acceptance Scenarios**:

1. **Given** a service implementation with no transport knowledge, **When** a test adapter invokes the `create_order` operation, **Then** the service executes and returns the result to the adapter without any awareness of how it was invoked.
2. **Given** a service used by two different adapters, **When** the service implementation is modified with new business logic, **Then** both adapters benefit from the change without modification.
3. **Given** a transport adapter that fails to convert an incoming request, **When** the error occurs before the service is called, **Then** the service is never invoked and the error is handled entirely within the adapter layer.

---

### Edge Cases

- What happens when a service operation is invoked and the underlying entity is not yet created? The service returns a domain error (e.g., "EntityNotFound") without exposing transport-level status codes.
- How does the registry handle services registered after the runtime has started? Late registration is rejected — all services must be registered before runtime startup to ensure dependency validation is complete.
- What happens when a service depends on a resource that becomes unavailable during runtime (e.g., an external adapter connection drops)? The service surfaces errors through its operation return type. Recovery is the responsibility of the adapter implementation.
- Can a service call another service directly? Yes, if the dependency is declared and the dependent service is injected at construction time. Circular service dependencies are rejected at wiring time.
- What happens when multiple modules register their own service registries? Modules produce service bundles that are merged into a single application-wide registry at startup. Conflicts (duplicate contracts) are rejected.
- How does the system ensure a service implementation cannot be cast to a transport-specific type? The registry enforces contract-type-based resolution — services are resolved only by their declared contract, never by implementation type.
- What happens to ServiceContext when a service calls another service internally? The runtime automatically propagates the context across service-to-service calls. The callee sees the same tenant ID, correlation ID, and trace ID as the caller.
- What happens when a transport adapter fails to populate ServiceContext from incoming metadata? A default empty context is created. Operations that require specific context values (e.g., tenant ID for multi-tenant entities) fail with a domain error.
- How does versioned service resolution handle a dependency on `OrderService v1` when only `v2` is registered? Resolution fails with a clear error — exact version constraints are strict. If no version is specified, the latest is returned.
- Can a service depend on another service at a specific version? Yes. Dependency declarations include optional version constraints. The registry enforces availability of the requested version at validation time.
- What happens when a service attempts to access an entity or resource outside its current tenant scope? The operation is rejected by the runtime with a clear tenant violation error. Cross-tenant access requires explicit opt-in configuration.
- How does tenant scoping work for service-to-service calls? The callee inherits the caller's tenant scope from the ServiceContext. A service calling another service operates within the same tenant boundary unless explicitly overridden.
- What happens when a deadline expires during a service invocation? The runtime terminates the invocation and returns a timeout error. The deadline is propagated to all downstream calls (service-to-service, EntityRef), which also observe the expired deadline.
- What happens when a cancellation request arrives mid-invocation? The runtime propagates the cancellation signal to the executing operation. If the operation does not complete before cancellation takes effect, the invocation terminates with a cancellation error.
- How does EntityRef<T> obtain the current invocation context? EntityRef<T> reads ServiceContext from the runtime invocation scope (TaskLocal) transparently. No explicit context parameter is needed on entity operations. No tenant-aware wrappers are created. EntityRef<T> remains API-compatible with CORE-006 and does not depend on service-sdk types.

## Requirements *(mandatory)*

### Functional Requirements

#### Service Contracts

- **FR-001**: The system MUST allow developers to declare service contracts as independently identifiable boundaries that group related business operations.
- **FR-002**: Service contracts MUST accept domain types (commands, queries, identifiers, views, domain errors) as operation inputs and outputs without requiring serialization annotations or transport-specific metadata.
- **FR-003**: Service contracts MUST NOT require or import any transport library (HTTP, gRPC, GraphQL, WebSocket) to be declared.
- **FR-004**: Each service contract MUST be resolvable by its logical type, an optional logical name, and an optional version constraint. When no version is specified, the latest registered version is returned.

#### Service Implementation

- **FR-005**: The system MUST allow developers to implement service contracts as concrete types with field-declared dependencies. The `#[service]` attribute on structs generates DependencyMetadata, DependencyGraph, and Injectable wiring metadata automatically. Behavior depends on target type (trait → contract metadata; struct → dependency metadata).
- **FR-006**: Service implementations MUST be able to declare persistent entity references as dependencies (via fields), enabling operations to send commands to entities by their logical ID.
- **FR-007**: Service implementations MUST be able to declare read-side projection handlers as dependencies (via fields) to query materialized views.
- **FR-008**: Service implementations MUST be able to declare external adapters as dependencies (via fields) for notifications, external API calls, and messaging, through their declared contracts.
- **FR-009**: Service implementations MUST be able to declare configuration values as dependencies (via fields) resolved at construction time.

#### Service Registry

- **FR-010**: The system MUST provide a registry where service implementations can be registered by their contract type.
- **FR-011**: The registry MUST reject duplicate registrations for the same contract type, name, and version combination.
- **FR-012**: The registry MUST support concurrent coexistence of multiple versions of the same service contract under different version identifiers (e.g., OrderService v1, OrderService v2).
- **FR-013**: The registry MUST provide a resolution operation that returns a reference to the service identified by its contract type, optional name, and optional version constraint. Unspecified version resolves to latest.
- **FR-014**: The registry MUST validate that all dependencies declared by registered services (including version constraints on internal services) are satisfied before the runtime starts.
- **FR-015**: The registry MUST support merging multiple service bundles (including versioned services), each produced by an independent application module, into a single application-wide registry. Version conflicts across bundles are rejected.
- **FR-015a**: The registry MUST provide a resolution operation that returns a generated type-safe proxy (e.g., `OrderServiceRef`) for a given service contract type, enabling transparent invocation with context propagation and interceptor execution.
- **FR-015b**: Generated proxy types (e.g., `OrderServiceRef`) MUST be the exclusive invocation boundary for services — both internal service-to-service calls and external transport invocations use the same proxy abstraction. Each proxy implements the service trait directly, preserving type-safe method calls.
- **FR-015c**: Generated proxies MUST transparently apply context propagation, interceptor execution, tracing integration, and metrics integration on every operation call without the service implementation being aware of these concerns.
- **FR-015d**: Service invocation MUST use generated type-safe proxies that preserve trait method ergonomics (e.g., `orders.create_order(cmd).await?`). The framework MUST NOT require string-based invocation (e.g., `invoke("operation_name")`). Method visibility and type checking are compile-time enforced.
- **FR-015e**: The `#[service]` proc-macro on traits MUST generate a concrete named proxy type (`{TraitName}Ref`) that implements the service trait. Each method implementation on the proxy transparently enters the invocation scope, runs the interceptor chain, delegates to the underlying implementation, and returns the result.

#### Dependency Injection

- **FR-016**: The system MUST provide runtime-managed dependency resolution: dependencies are declared as annotated fields on service structs and resolved automatically by the runtime during construction.
- **FR-017**: Dependency injection MUST be declaration-based — developers annotate fields with their dependency types; the runtime resolves and injects them at construction time. Developers SHALL NOT manually wire dependency graphs.
- **FR-018**: The system MUST reject construction-time wiring that creates circular service dependencies, reporting the cycle clearly.

#### Runtime Wiring

- **FR-019**: The runtime MUST accept entity definitions, read-side projections, and service declarations through a builder pattern (e.g., `.with_entity::<T>()`, `.with_service::<S>()`) and construct the full service graph during startup.
- **FR-020**: The runtime MUST validate that all required components are present (entities, projections, services) before transitioning to an operational state.
- **FR-021**: The runtime MUST make registered services available for invocation by external components (transport adapters) once started.
- **FR-021a**: Application services MUST NOT expose initialization or shutdown hooks — services are fully initialized upon dependency resolution and field injection.
- **FR-021b**: Lifecycle management (startup, shutdown, health checks) belongs to runtime-managed components (ProjectionRunner, Scheduler, TransportAdapter, EventStreamConsumer, ConnectionPool), which MAY implement lifecycle contracts.
- **FR-021c**: During shutdown, the runtime MUST stop accepting new invocations, wait for in-flight operations to complete, then release service references and runtime-managed resources.

#### Invocation Interceptors

- **FR-021d**: The service SDK MUST support optional invocation interceptors at the registry/runtime boundary that can observe and instrument service invocations without modifying service implementations.
- **FR-021e**: Interceptors MUST be pluggable and optional — no service is required to implement middleware hooks or opt into instrumentation.
- **FR-021f**: Interceptors MUST support at minimum: pre-invocation (before operation call), post-invocation (after successful result), and error-invocation (after error) interception points.
- **FR-021g**: Interceptors MUST be usable for: tracing spans, invocation metrics, error counters, structured logging, correlation ID propagation, and tenant context propagation.

#### Concurrency Model

- **FR-021h**: The service SDK MUST NOT provide service-level concurrency control — no service mailboxes, actors, serialization, or operation-level queuing.
- **FR-021i**: Services MUST be invocable concurrently by multiple callers. Thread safety for mutable shared state is the implementor's responsibility.
- **FR-021j**: Services SHOULD be designed as stateless orchestrators that delegate to entities (single-threaded via CORE-006), read stores, repositories, and external clients.
- **FR-021k**: The service SDK MUST NOT duplicate or override the entity layer's concurrency guarantees — entity execution remains the responsibility of the entity runtime (CORE-006).

#### Invocation Context

- **FR-021l**: The runtime MUST automatically propagate an invocation context (ServiceContext) for every service invocation, carrying metadata scoped to the invocation lifecycle.
- **FR-021m**: ServiceContext MUST expose invocation-scoped metadata including at minimum: tenant ID, correlation ID, trace ID, deadline, timeout, cancellation token, and request metadata. Service operations MUST NOT need to accept context as a parameter.
- **FR-021n**: Services MUST access the current invocation context through a runtime-provided API (`ServiceContext::current()` returning `Option<&ServiceContext>`), never through service instance fields or constructor injection. Missing context returns None; the API never panics. Context belongs to the invocation, not the service instance.
- **FR-021n1**: ServiceContext::current() MUST NOT panic and MUST NOT terminate runtime execution. Missing invocation context is a valid runtime state. Tests, CLI tools, migrations, and background jobs MAY execute without a ServiceContext. Services MAY safely inspect context existence via `Option` pattern matching.
- **FR-021o**: ServiceContext MUST be propagated by the runtime across service-to-service calls, entity command invocations, and interceptor chains without developer intervention.
- **FR-021p**: Transport adapters MUST be able to populate ServiceContext from incoming transport metadata (headers, tokens) before service invocation.

#### Multi-Tenant Isolation

- **FR-021q**: The runtime MUST automatically enforce tenant isolation on all runtime-managed components — EntityRef<T>, repositories, and read-side handlers operate within the current tenant boundary from the invocation scope. No tenant-aware wrappers are required; EntityRef<T> receives tenant context transparently from the runtime scope.
- **FR-021q1**: EntityRef<T> MUST remain API-compatible with CORE-006 — no API changes, no new required parameters, and no dependency on service-sdk types. EntityRef<T> MUST NOT become transport-aware.
- **FR-021q2**: Service implementations MUST NOT manually pass tenant_id, correlation_id, trace_id, deadline, timeout, or cancellation information to EntityRef operations. The runtime is solely responsible for binding ServiceContext to EntityRef execution boundaries.
- **FR-021r**: Cross-tenant access (accessing an entity or resource outside the current tenant scope) MUST require explicit opt-in. By default, operations are scoped to the invocation's tenant.
- **FR-021s**: Services MUST NOT be required to manually pass tenant identifiers to entity references — EntityRef<T> obtains tenant context from the runtime invocation scope automatically.
- **FR-021t**: A service invocation without a tenant ID in its ServiceContext MUST operate in a default context or fail with a clear error for tenant-required resources, as defined by the resource configuration.

#### Deadline & Cancellation

- **FR-021u**: ServiceContext MUST carry deadline, timeout, and cancellation token metadata as part of the invocation scope.
- **FR-021v**: The runtime MUST automatically propagate deadline and cancellation information across ServiceRef calls, service-to-service calls, EntityRef calls, and interceptor chains without developer intervention.
- **FR-021w**: The runtime MAY terminate execution when a deadline expires or a cancellation request is received, producing a framework-level timeout or cancellation error.
- **FR-021x**: Services MUST NOT be required to manually check deadlines or cancellation tokens during operation execution — the runtime handles enforcement transparently.

#### Contract Descriptors

- **FR-022a**: The system MUST provide explicit contract descriptors that describe service contracts, individual operations, request types, response types, and fields without imposing transport semantics.
- **FR-022b**: Service descriptors MUST expose metadata about each operation: operation name, input type, output type, and operation-level metadata (e.g., idempotency, read-only vs. mutating).
- **FR-022c**: Contract descriptors (request/response types) MUST expose field-level metadata: field name, field type, optional/required designation, and field-level documentation.
- **FR-022d**: Contract descriptors MUST be the canonical source of truth for transport adapters — transport mappings (REST routes, gRPC methods, GraphQL queries) are derived from descriptors, not duplicated.
- **FR-022e**: Contract descriptors MUST support explicit versioning metadata using semantic versioning strings (MAJOR.MINOR.PATCH) so that contract evolution can be tracked across versions.
- **FR-022e1**: The Service SDK MUST NOT perform compatibility analysis or enforce versioning rules — breaking vs. non-breaking change detection belongs to future tooling, CI validation, and contract governance layers.
- **FR-022e2**: Version metadata MUST be stored in ContractDescriptor and ServiceDescriptor and exposed for consumption by transport adapters, SDK generators, and governance tooling.
- **FR-022f**: Contract descriptors MUST NOT depend on any transport protocol library — descriptors describe the logical contract, not how it is exposed.
- **FR-022g**: Service descriptors and operation descriptors MUST be generated automatically from `#[service]` attribute on traits. Dependency metadata and wiring descriptors MUST be generated automatically from `#[service]` attribute on structs. Developers SHALL NOT write descriptor or dependency implementations manually.
- **FR-022g1**: The `#[service]` proc-macro behavior MUST be determined solely by target type: trait targets produce contract metadata (ServiceDescriptor, OperationDescriptor, ContractDescriptor); struct targets produce dependency and wiring metadata (DependencyMetadata, DependencyGraph, Injectable). Developers SHALL NOT manually define any generated metadata.
- **FR-022h**: Transport adapters MUST be able to consume generated descriptor metadata to derive transport-specific mappings (routes, methods, queries, schemas) without the service layer knowing about the transport.

#### Error Model

- **FR-025a**: Services MUST define their own domain-specific error types for business rule violations, entity-not-found, validation failures, and conflict states.
- **FR-025b**: Domain error types SHOULD implement a common framework error trait that exposes metadata: error code (string identifier) and error category.
- **FR-025c**: The framework MAY define common error categories (Validation, NotFound, Conflict, Authorization, BusinessRule, Infrastructure) as a shared taxonomy without forcing services into a shared error enum.
- **FR-025d**: Interceptors and transport adapters MUST operate against the common error trait interface, not against concrete domain error types, enabling generic error instrumentation and transport-level mapping.
- **FR-025e**: The framework MUST NOT provide a mandatory shared error enum that all services are required to use.

#### Transport Agnosticism

- **FR-023**: No part of the service contract, service implementation, or service registry MUST depend on any transport protocol library (HTTP, gRPC, GraphQL, WebSocket, or any concrete HTTP framework).
- **FR-024**: The service SDK MUST NOT include, require, or assume any REST endpoint definitions, HTTP routers, gRPC service descriptors, GraphQL schemas, or OpenAPI generation.
- **FR-025**: Service operation errors MUST be domain errors, not transport-level status codes.

#### Testing

- **FR-026**: All service contracts and implementations MUST be testable using mock dependencies — no real persistent entities, external adapters, or external services required.
- **FR-027**: The service registry MUST be testable in isolation, allowing unit tests to register mock services and verify resolution behavior.

### Key Entities

- **Service Contract**: A named collection of business operations that defines an application boundary. Identified by its logical type, optional logical name, and version. Contains operations with domain inputs and domain outputs. Multiple versions may coexist.
- **Service Implementation**: A concrete type that fulfills a service contract, with dependencies declared as annotated fields. Dependencies are resolved automatically by the runtime at construction time — developers do not manually wire dependency graphs.
- **Service Registry**: A container that maps contract types to their implementations, validates dependency completeness, rejects duplicates, and supports multi-module composition.
- **Service Bundle**: A partial registry produced by an application module, containing the services registered by that module. Bundles are merged to form the application-wide registry.
- **Runtime Context**: The execution environment that provides entity references, read-side handler access, configuration, and adapter resolution during service wiring.
- **Service Descriptor**: Metadata describing a service contract: its logical name, operations list, and contract-level metadata. The canonical source of truth for transport adapters.
- **Operation Descriptor**: Metadata describing a single operation within a service: operation name, input type, output type, and operational characteristics (idempotent, read-only, mutating).
- **Contract Descriptor**: Metadata describing a request or response type: logical name, field list, and type-level documentation.
- **Field Descriptor**: Metadata describing a single field within a contract: field name, field type, optional/required designation, and field-level documentation.
- **Contract Version**: A version identifier attached to a contract descriptor, enabling explicit contract evolution tracking without breaking transport adapters.
- **Runtime-Managed Component**: Infrastructure components (ProjectionRunner, Scheduler, TransportAdapter, EventStreamConsumer, ConnectionPool) that MAY implement lifecycle contracts. Distinct from application services, which have no lifecycle hooks.
- **Invocation Interceptor**: A pluggable observer attached at the registry/runtime boundary that can instrument service invocations (pre, post, error) for tracing, metrics, logging, and context propagation without modifying service implementations.
- **ServiceContext**: Invocation-scoped metadata automatically propagated by the runtime for every service invocation. Carries tenant ID, correlation ID, trace ID, deadline, timeout, cancellation token, and request metadata. The runtime enforces tenant isolation and deadline/cancellation propagation based on this context. Accessed via a runtime API, not injected into service instances. Context belongs to the invocation, not the service.
- **Domain Error**: A service-defined error type implementing a common framework error trait with error code and error category metadata. Exposed to interceptors and transport adapters through the trait interface, not by concrete type.
- **ServiceRef`<T>`**: A generated concrete proxy type (e.g., `OrderServiceRef`) produced by the `#[service]` proc-macro for each service trait. Holds an `Arc<dyn ServiceTrait>` typed reference — no type erasure, no runtime downcasting. Implements the service trait directly via trait method calls. Each method transparently enters the invocation scope, runs the interceptor chain, and delegates to the inner implementation. Resolved from the service registry via `registry.resolve::<OrderService>()`. Used by both internal services and external transport adapters.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A developer can declare a new service contract and have it recognized by the registry in under 5 minutes of coding effort (excluding business logic).
- **SC-002**: A developer can register and resolve 10 distinct services with cross-dependencies in under 1 second during startup.
- **SC-003**: A service implementation operates identically regardless of which transport adapter invokes it — verified by swapping the adapter without modifying the service.
- **SC-004**: Missing or circular dependencies are detected and reported before the runtime starts, with 100% detection rate (no false negatives).
- **SC-005**: Duplicate service contract registration is rejected with 100% reliability, preventing runtime ambiguity.
- **SC-006**: All service contracts, implementations, and registry operations can be tested without requiring any external transport infrastructure (no HTTP server, gRPC server, or message broker).
- **SC-007**: The service SDK crate has zero direct or transitive dependencies on HTTP, gRPC, GraphQL, WebSocket, or any concrete transport framework libraries.
- **SC-008**: A DX acceptance example demonstrates the full developer journey (contract → implementation → wiring → invocation) in 25 lines of code or fewer, excluding business logic and import statements.

## Scope Boundaries *(mandatory)*

### In Scope

- Service contract declaration as application boundaries
- Explicit contract descriptors (services, operations, requests, responses, fields) as canonical metadata
- Contract descriptor generation via attribute-based declarations (code-first, metadata-generated)
- Service implementation with field-declared dependencies resolved automatically by the runtime
- Service registry with type-based, named, and versioned registration, resolution, and validation
- ServiceRef<T> handles as the invocation boundary for all service calls
- Multi-module service bundles that merge into an application-wide registry
- Runtime wiring that connects entities, read-side, and services
- In-memory service resolution for use by transport adapters
- Runtime-managed ServiceContext propagation (tenant, correlation, trace, deadlines, cancellation)
- Runtime-managed deadline and cancellation propagation across the invocation graph
- Runtime-enforced multi-tenant isolation on entities, repositories, and read-side handlers
- Optional invocation interceptors (tracing, metrics, logging, context propagation)
- Hierarchical error model with common trait and service-defined error types
- Mock-based testing support for all service components

### Out of Scope

CORE-008 explicitly does NOT include:

- REST API server, HTTP router, or HTTP endpoint definitions
- gRPC server or gRPC service descriptor generation
- GraphQL schema definition or GraphQL server
- WebSocket server or real-time connection management
- OpenAPI/Swagger specification generation
- HTTP authentication or authorization logic
- HTTP request/response serialization or content negotiation
- Web middleware (CORS, logging, rate limiting, compression)
- Transport-level error mapping (HTTP status codes, gRPC status codes)
- Controller annotations, route decorators, or Spring MVC-style metadata
- Service discovery or service mesh integration
- A mandatory "Service Gateway" or unified entry point

These capabilities belong to transport adapter crates (e.g., `ego-transport-axum`, `ego-transport-tonic`) that layer on top of the Service SDK.

### Future Extensions (Not in CORE-008)

- `ego-transport-axum`: REST adapter using the Axum framework
- `ego-transport-tonic`: gRPC adapter using the Tonic framework
- `ego-transport-graphql`: GraphQL adapter using async-graphql
- `ego-transport-actix-web`: REST adapter using actix-web
- `ego-transport-cli`: CLI-based service invocation

## Assumptions

1. The runtime provides entity references (persistent entity handles) and read-side handlers through a well-defined context that is available during service wiring. This context is established by prior core features (CORE-006, CORE-005).
2. Transport adapters are responsible for translating between external protocols and service operations. The service SDK provides no conversion utilities — adapters call service operations directly with domain types.
3. Service resolution is synchronous and in-process. Services are not remote — remote service invocation is a transport concern handled by external adapters.
4. Module composition follows a deterministic startup order: all modules register their services, the registry validates completeness, and then the runtime starts.
5. Service contracts use an async execution model consistent with the runtime's concurrency architecture (actor-per-entity, single-threaded entity execution). Service dependency resolution and construction happen during the runtime build phase, before traffic acceptance.
6. Error types used by service operations are domain-level errors (business rule violations, entity-not-found, validation failures), not infrastructure errors (connection refused, timeout). Infrastructure errors are handled by the runtime or adapters.
7. The codebase follows the constitution's immutability-by-default principle — service operation inputs and outputs are immutable value types.
8. Contract descriptors are generated at compile time via attribute-based declarations. Developers declare contracts with code annotations; the framework produces descriptor metadata. Manual descriptor implementation is not required or supported.
9. Services are designed as stateless orchestrators — they delegate to entities, read stores, and external clients but SHOULD NOT maintain mutable shared state. Services are invoked concurrently; thread safety for any shared state is the implementor's responsibility.
10. ServiceContext is propagated automatically by the runtime for every invocation. Transport adapters populate context from incoming metadata. The runtime carries context across service-to-service calls, entity commands (via EntityRef reading the invocation scope), and interceptor chains transparently. EntityRef<T> requires no API changes or wrappers.
11. Domain errors implement a common framework trait with error code and category metadata. Interceptors and adapters consume error metadata generically. Concrete error types remain service-defined. Contract governance and compatibility enforcement are deferred to future tooling layers.
12. A DX acceptance example (crates/service-sdk/examples/order_service.rs) demonstrates the full developer journey — contract declaration, implementation with field dependencies, runtime builder wiring, and invocation — and serves as a regression target for API ergonomics.

## Dependencies

- **CORE-006** (Persistent Entity Runtime): Provides entity references that services use to send commands to entities.
- **CORE-005** (Read Side Projections): Provides read-side handlers that services use to query materialized views.
- **CORE-001** (Persistence SPI): Provides the repository and event store abstractions that entities depend on (transitive).
- **CORE-003** (Effect API): Provides the effect model for handling external effects within service operations (transitive).
- **Constitution §7** (External Boundary): Service operations that produce external effects MUST describe them as intents during execution, not dispatch them directly.
- **Constitution §8** (Testing): All service components MUST be tested with mock dependencies. Coverage must meet the 85% threshold.
