# ego-rs — Product Requirements Document

**Status**: Living document  
**Last updated**: 2026-06-25  
**Owner**: @pablogore

---

## 1. Vision

> Build a hexagonal, actor-oriented, deterministic backend framework for Rust that makes correctness the default — not the exception.

ego-rs is a **framework**, not a library. It prescribes architecture, enforces invariants at compile time where possible, and provides the scaffolding for building production-grade backend systems that are provably correct by design.

---

## 2. Problem

Rust backend development today has two extremes:

- **Roll your own** — pure `tokio` + `axum` + your own abstractions. Fast to start, impossible to scale with consistency. Every team reinvents domain modeling, error handling, and deployment patterns differently.
- **Import a framework** — existing Rust frameworks focus on HTTP ergonomics, not architectural correctness. None enforce hexagonal layering, domain isolation, or deterministic execution.

The result: teams build Rust backends that *compile* but don't *converge* — race conditions, ambient state, untestable domain logic, and architecture that erodes over time.

**ego-rs solves this** by codifying correct architecture at the framework level.

---

## 3. Target Users

**Primary**: Rust engineers building production backend systems who care about long-term maintainability and correctness over initial velocity.

**Secondary**: Teams migrating from JVM frameworks (Spring, Quarkus, Akka) who want Rust's performance guarantees without sacrificing architectural discipline.

**Not the target**: Prototypers who need something running in an hour, or teams that treat architecture as optional.

---

## 4. Core Principles

| Principle | What it means |
|-----------|--------------|
| **Deterministic-first** | Same inputs → same outputs, every time. No hidden clocks, no ambient state, no side effects in domain logic. |
| **Fail-closed** | Ambiguous states produce rejection, not silent continuation. Unknown inputs and undefined transitions terminate immediately. |
| **Hexagonal architecture** | Domain owns contracts. Infrastructure owns adapters. Transport owns protocols. No layer violates another. Enforced by `layers.toml`. |
| **CQRS + Event Sourcing** | Commands mutate. Events record. Queries read from projections. The event log is the source of truth. |
| **Minimal primitives** | One concept, one trait, one responsibility. No God types, no kitchen-sink abstractions. |
| **Runtime neutrality** | Domain contracts have no `async`, no `tokio`, no runtime types. Infrastructure owns async integration. |

---

## 5. Architecture

ego-rs is organized as a workspace of layered crates. Each crate owns a defined set of responsibilities and may only depend on crates to its left:

```
domain ← application ← infrastructure
domain ← transport
domain ← runtime
domain ← service-sdk (cross-cutting)
domain ← security-sdk (cross-cutting)
```

### Crates

| Crate | Layer | Responsibility |
|-------|-------|---------------|
| `ego-domain` | Domain | Core contracts: Actor, Command, Event, Query, persistence SPI, auth types |
| `ego-application` | Application | Command/Query handlers, ports, use case orchestration |
| `ego-infrastructure` | Infrastructure | Adapters: in-memory, PostgreSQL persistence, migrations |
| `ego-transport` | Transport | HTTP/gRPC protocol handlers |
| `ego-runtime` | Foundation | Actor system, mailbox, supervision, scheduling |
| `ego-runtime-tokio` | Infrastructure | Tokio-backed runtime implementation |
| `ego-event-adapter` | Infrastructure | CloudEvent ↔ domain event translation |
| `ego-persistent-entity` | Foundation | Persistent actor lifecycle: load, execute, snapshot, save |
| `ego-ego-scheduler` | Foundation | Tag-based event scheduling, backpressure, dedup |
| `ego-service-sdk` | Cross-cutting | Service contracts, registry, DI, interceptors, context propagation |
| `ego-service-sdk-macros` | Cross-cutting | `#[service]`, `#[operation]` proc-macro attributes |
| `ego-security-sdk` | Cross-cutting | AuthN/AuthZ, Principal, Credential, RBAC, SecurityContext, Claims |
| `ego-security-jwt` | Cross-cutting (impl) | HS256/RS256/ES256 JWT providers over KeyResolver abstraction |

### Layering Enforcement

Dependency direction is enforced at CI time via `layers.toml` and `scripts/verify-layers.sh`. A PR that introduces a layering violation fails CI.

---

## 6. Core Capabilities

### 6.1 Domain Modeling

- `Actor` trait: stateless message handler with typed `Message` associated type
- `DomainEvent`: append-only event contract
- `Command` / `Query`: CQRS command and query types
- Persistence SPI: `EventStore`, `Repository`, `SnapshotStore`, `PersistenceError`
- Pure value types: no runtime types, no async, no serialization frameworks in domain contracts

### 6.2 Authentication & Authorization (security-sdk + security-jwt)

- `AuthenticationProvider` trait: synchronous, object-safe, injectable
- `AuthorizationProvider` trait: async, RBAC-capable
- `SecurityContext`: carries `Principal` + `Claims`, explicit propagation via `ServiceContext`
- JWT providers: `Hs256AuthenticationProvider`, `Rs256AuthenticationProvider`, `Es256AuthenticationProvider`
- `KeyResolver` abstraction: cache-first, async, pluggable key backends
- No ambient security state — `SecurityContext` travels explicitly through `ServiceContext`

### 6.3 Service SDK

- `ServiceContract` trait + `ServiceDescriptor` for declarative service definition
- `ServiceRegistry` for service discovery and wiring
- `ServiceContext`: carries tenant, correlation, causation, security — propagated explicitly
- `Interceptor` / `InterceptorChain`: cross-cutting concerns without framework coupling
- `#[service]` / `#[operation]` proc-macros for ergonomic service declaration
- `RuntimeBuilder` extension for wiring services at startup

### 6.4 Runtime & Scheduling

- `Runtime` trait: backend-neutral actor spawn and messaging
- `ExecutionState`: supervised actor lifecycle (Active → Draining → Terminated/Failed)
- `BatchExecutor`: deterministic batch processing with backpressure
- `TagSchedulerImpl`: tag-based projection scheduling
- `EffectInterpreter`: async trait for interpreting domain effects
- Fail-closed execution: panics terminate the unit of work, not the runtime

### 6.5 Persistence

- In-memory adapters for testing (no external dependencies)
- PostgreSQL adapter via `sqlx`
- Atomic commit: offset + dedup persisted in one transaction
- Append-only event store enforced by type system

---

## 7. Design Constraints

- **No transport types in domain** — `http::HeaderValue`, `tonic::*`, `axum::*` must never appear in domain contracts
- **No runtime types in domain** — `tokio::*`, `async` must never appear in domain trait signatures
- **No ambient state** — `SecurityContext` and `ServiceContext` travel through explicit parameters, never thread-locals or globals
- **No shims** — when a public API is removed, it is removed. No deprecated aliases in pre-stable crates
- **Clock injection always** — time-sensitive logic receives `Arc<dyn Clock>`. `Utc::now()` is forbidden in domain and application layers
- **Docs required** — `#![deny(missing_docs)]` in all public crates. CI fails on undocumented public items

---

## 8. What ego-rs Is Not

- **Not a web framework** — ego-rs does not compete with `axum` or `actix-web`. Transport is a thin adapter layer, not the core product
- **Not an ORM** — persistence is event-sourced. ego-rs does not provide query builders or entity mapping
- **Not an async runtime** — `ego-runtime-tokio` wraps Tokio; it does not replace it
- **Not a domain contract registry** — the `contracts/` directory exists for illustrative examples only. ego-rs does not manage domain-level protobuf contracts

---

## 9. Roadmap

### Active

| Change | Status |
|--------|--------|
| CORE-013: JWT Providers + KeyResolver (Hs256/Rs256/Es256) | ✅ Archived |

### Planned

| Change | Description |
|--------|-------------|
| CORE-014 | `#[authorize]` macro — declarative authorization at the operation level |
| CORE-011B | JWKS remote key resolver (OIDC discovery, cache-backed, multi-issuer) |
| CORE-015 | Telemetry SDK — `ego-telemetry-sdk` cross-cutting observability primitive |

### Deferred

| Item | Reason |
|------|--------|
| `CompositeAuthenticationProvider` | Multi-algorithm dispatch deferred until real consumer exists |
| EdDSA algorithm support | Blocked on JWKS resolver for practical use |
| gRPC transport implementation | Transport layer exists, gRPC adapter not started |
| `ego-config-sdk` | Deferred until service SDK adoption grows |

---

## 10. Success Criteria

A version of ego-rs is ready for broader adoption when:

1. `cargo test --workspace` passes with ≥ 85% coverage
2. A non-trivial service (3+ operations, auth, persistence) can be built using only ego-rs primitives
3. The layering enforcement script catches all known violation patterns
4. All public APIs carry `rustdoc` documentation
5. A new contributor can understand the architecture from `docs/` alone without reading source code

---

## 11. References

- [`docs/architecture.md`](docs/architecture.md) — engineering architecture, crate boundaries, design preferences
- [`docs/constitution-mapping.md`](docs/constitution-mapping.md) — constitutional rules and enforcement mechanisms
- [`openspec/specs/`](openspec/specs/) — canonical specifications per domain
- [`ARCHITECTURE.md`](ARCHITECTURE.md) — runtime architecture overview
