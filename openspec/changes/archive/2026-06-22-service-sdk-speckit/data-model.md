# Data Model: Service SDK

**Feature**: 008-service-sdk  
**Date**: 2026-06-08

## Core Entities

### ServiceContract

Represents a service boundary defined by a trait with `#[service]` attribute.

| Field | Type | Description |
|-------|------|-------------|
| `type_id` | `TypeId` | Compile-time unique identifier for the contract trait |
| `name` | `String` | Logical name derived from trait name (e.g., "OrderService") |
| `version` | `Version` | Semantic version string (MAJOR.MINOR.PATCH) |
| `operations` | `Vec<OperationDescriptor>` | Operations declared on this service |

**Identity**: Uniquely identified by `(type_id, name, version)` tuple.

**Invariants**:
- Two services with same `(type_id, name, version)` cannot coexist in a registry (FR-011)
- If no name is specified, `type_id` alone provides identity (FR-012)

---

### OperationDescriptor

Describes a single operation within a service contract.

| Field | Type | Description |
|-------|------|-------------|
| `name` | `String` | Operation name derived from method name (e.g., "create_order") |
| `input_type` | `TypeId` | TypeId of the input/command type |
| `output_type` | `TypeId` | TypeId of the output/result type |
| `category` | `OperationCategory` | Idempotent, ReadOnly, Mutating |
| `documentation` | `Option<String>` | Doc comment from trait method |

**Enum: OperationCategory**
- `Idempotent` — Safe to retry, same result
- `ReadOnly` — No side effects
- `Mutating` — Produces state changes

---

### ContractDescriptor

Describes a request or response type.

| Field | Type | Description |
|-------|------|-------------|
| `type_id` | `TypeId` | TypeId of the described type |
| `name` | `String` | Logical name (e.g., "CreateOrderRequest") |
| `fields` | `Vec<FieldDescriptor>` | Fields within this contract |
| `documentation` | `Option<String>` | Type-level documentation |

---

### FieldDescriptor

Describes a single field within a contract type.

| Field | Type | Description |
|-------|------|-------------|
| `name` | `String` | Field name |
| `field_type` | `String` | Human-readable type name |
| `required` | `bool` | Whether the field is required |
| `documentation` | `Option<String>` | Field-level documentation |

---

### ContractVersion

Version identifier for contracts and services.

| Field | Type | Description |
|-------|------|-------------|
| `major` | `u32` | Major version (breaking changes) |
| `minor` | `u32` | Minor version (non-breaking additions) |
| `patch` | `u32` | Patch version (fixes, docs) |

**Format**: `MAJOR.MINOR.PATCH` string representation.

**Invariants**:
- Versions are developer-managed, not framework-enforced (FR-022e1)
- Comparison follows semver ordering: `1.2.0 > 1.1.0 > 1.0.0`

---

### ServiceRegistry

Central registry mapping contracts to implementations.

| Field | Type | Description |
|-------|------|-------------|
| `entries` | `HashMap<RegistryKey, RegistryEntry>` | Registered services |
| `interceptors` | `Vec<Box<dyn Interceptor>>` | Global interceptors applied to all invocations |
| `validated` | `bool` | Whether dependency validation has passed |

**Methods**:
- `register<T: ServiceContract>(service: Arc<T>, name: Option<&str>, version: Version)` — Register implementation
- `resolve<T: ServiceContract>(name: Option<&str>, version: Option<VersionConstraint>) -> Result<ServiceRef<T>>` — Resolve handle
- `validate() -> Result<(), Vec<MissingDependency>>` — Validate all dependencies satisfied
- `merge(bundle: ServiceBundle)` — Merge another bundle

---

### RegistryEntry

Internal registry storage for a single registered service.

| Field | Type | Description |
|-------|------|-------------|
| `key` | `RegistryKey` | (TypeId, name, version) compound key |
| `implementation` | `Arc<dyn Any + Send + Sync>` | Type-erased service implementation |
| `descriptor` | `ServiceDescriptor` | Generated metadata |
| `dependencies` | `Vec<TypeId>` | TypeIds this service depends on |

---

### ServiceBundle

Partial registry from an application module.

| Field | Type | Description |
|-------|------|-------------|
| `entries` | `Vec<RegistryEntry>` | Services in this bundle |
| `module_name` | `String` | Module identifier for error reporting |

**Invariants**:
- Multiple bundles can be merged into one registry (FR-015)
- Version conflicts across bundles are rejected

---

### ServiceRef<T>

Invocation handle resolved from the registry. The service boundary equivalent of `EntityRef<T>`.

| Field | Type | Description |
|-------|------|-------------|
| `service` | `Arc<dyn Any + Send + Sync>` | Downcasted service implementation |
| `descriptor` | `ServiceDescriptor` | Contract metadata |
| `chain` | `InterceptorChain` | Interceptor chain applied on invocation |

**Behavior**:
- On operation call: enters ServiceContext scope → runs interceptor chain → delegates to implementation
- Applies tenant isolation, deadline propagation transparently
- Used by both transport adapters and internal service-to-service calls (FR-015b)

---

### ServiceContext

Invocation-scoped metadata propagated by the runtime.

| Field | Type | Description |
|-------|------|-------------|
| `tenant_id` | `Option<String>` | Current tenant identifier |
| `correlation_id` | `Uuid` | Correlation ID for tracing |
| `trace_id` | `Uuid` | Distributed trace ID |
| `deadline` | `Option<Instant>` | Invocation deadline |
| `timeout` | `Option<Duration>` | Invocation timeout |
| `cancellation` | `CancellationToken` | Cancellation signal |
| `metadata` | `HashMap<String, String>` | Arbitrary key-value metadata |

**Propagation**: Set via `ServiceContext::scope(ctx, async { ... }).await`. Accessed via `ServiceContext::current()`.

---

### Interceptor

Pluggable observer for service invocations.

**Trait methods**:
- `on_request(ctx: &ServiceContext, op: &OperationDescriptor)` — Before operation execution
- `on_response(ctx: &ServiceContext, op: &OperationDescriptor)` — After successful result
- `on_error(ctx: &ServiceContext, op: &OperationDescriptor, error: &dyn DomainError)` — After error

**Chain**: Ordered list of interceptors. Each hook is called sequentially. Interceptor failure is logged, never propagated.

---

### DomainError (Trait)

Common trait implemented by service-specific error types.

**Trait methods**:
- `code(&self) -> &str` — Error code string (e.g., "ORDER_NOT_FOUND")
- `category(&self) -> ErrorCategory` — Error classification
- `message(&self) -> &str` — Human-readable message

**ErrorCategory enum**:
- `Validation` — Input validation failure
- `NotFound` — Resource not found
- `Conflict` — Resource conflict (e.g., duplicate)
- `Authorization` — Permission denied
- `BusinessRule` — Business rule violation
- `Infrastructure` — Runtime/infrastructure error

---

### RuntimeManagedComponent

Infrastructure components with lifecycle, distinct from application services.

| Field | Type | Description |
|-------|------|-------------|
| `name` | `String` | Component identifier |
| `state` | `ComponentState` | Current lifecycle state |

**ComponentState enum**: `Created`, `Initializing`, `Running`, `Stopping`, `Stopped`, `Failed`

**Note**: Application services do NOT implement this lifecycle. Only runtime-managed components (ProjectionRunner, Scheduler, TransportAdapter) do.

---

## Entity Relationships

```
ServiceContract 1 ──── * OperationDescriptor
ServiceContract 1 ──── 1 Version
ServiceContract 1 ──── * ContractDescriptor (input/output types)
ContractDescriptor 1 ──── * FieldDescriptor

ServiceRegistry 1 ──── * RegistryEntry
RegistryEntry 1 ──── 1 ServiceDescriptor
RegistryEntry 1 ──── 1 Arc<service implementation>
ServiceRegistry 1 ──── * Interceptor
ServiceRegistry 1 ──── * ServiceRef<T> (resolved handles)

ServiceRef<T> 1 ──── 1 InterceptorChain
InterceptorChain 1 ──── * Interceptor

ServiceInvocation 1 ──── 1 ServiceContext
ServiceContext 1 ──── optional deadline/timeout/cancellation

DomainError ────► ErrorCategory
```

## State Transitions

### Registry Lifecycle

```
[Empty] ──register──► [Populated]
[Populated] ──merge(bundle)──► [Populated]
[Populated] ──validate──► [Validated] or [Invalid(error)]
[Invalid] ──fix + validate──► [Validated]
[Validated] ──build runtime──► [Operational]
```

### Service Invocation Lifecycle

```
[Invocation Created] ──set context──► [Context Set]
[Context Set] ──interceptor::on_request──► [Processing]
[Processing] ──service operation──► [Completed] or [Failed]
[Completed/Failed] ──interceptor hooks──► [Finalized]
[Finalized] ──if deadline expired──► [Cancelled]
```

### Runtime-Managed Component Lifecycle

```
[Created] ──init──► [Initializing]
[Initializing] ──init complete──► [Running]
[Running] ──shutdown──► [Stopping]
[Stopping] ──drained──► [Stopped]
[Any] ──error──► [Failed]
```
