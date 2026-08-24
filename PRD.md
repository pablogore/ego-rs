# ego-rs — Product Requirements Document

**Status**: Living document  
**Last updated**: 2026-08-24  
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
domain ← persistence
domain ← transport
domain ← runtime
domain ← service-sdk (sdk layer)
domain ← security-sdk (cross-cutting)
```

### Crates

17 crates + `examples/reference-app` (root `Cargo.toml` workspace members):

| Crate | Layer | Responsibility |
|-------|-------|---------------|
| `ego-domain` | Domain | Core contracts: Actor, Command, Event, Query, persistence SPI, CQRS read-side traits, auth types |
| `ego-application` | Application | Command/Query handlers, ports, use case orchestration |
| `ego-persistence` | Infrastructure | PostgreSQL event-store/read-side adapters |
| `ego-infrastructure` | Infrastructure | In-memory persistence adapters, migrations |
| `ego-transport` | Transport | HTTP protocol handlers |
| `ego-runtime` | Foundation | Actor system, mailbox, supervision, scheduling, effect ports |
| `ego-runtime-tokio` | Infrastructure | Tokio-backed runtime implementation |
| `ego-effect-store` | Infrastructure | Durable external-effect providers: `PostgresEffectStore`, `StoolapEffectStore` |
| `ego-event-adapter` | Infrastructure | CloudEvent ↔ domain event translation |
| `persistent-entity` | Foundation | Persistent actor-per-entity lifecycle: recover, execute, passivate |
| `ego-scheduler` | Foundation | Tag-based event scheduling, backpressure, dedup |
| `ego-service-sdk` | SDK | Service contracts, registry, DI, interceptors, context propagation |
| `ego-service-sdk-macros` | Tooling | `#[service]`, `#[operation]`, `#[authorize]`, `#[tenant_scoped]` proc-macros |
| `ego-security-sdk` | Cross-cutting | AuthN/AuthZ, Principal, Credential, RBAC, SecurityContext, Claims |
| `security-jwt` | Infrastructure | JWT authentication (HS256/RS256/ES256 via one algorithm-parameterized provider) |
| `security-apikey` | Infrastructure | API-key authentication provider |
| `ego-testkit` | Tooling | Shared test doubles/fixtures for building services against the SDK |

### Layering Enforcement

Dependency direction is declared in `layers.toml` and locally enforced by `cargo run -p xtask -- verify-layers` — this repository has no CI at all; see [`ARCHITECTURE.md` → Layer enforcement](ARCHITECTURE.md#layer-enforcement) for the full model.

---

## 6. Core Capabilities

### 6.1 Domain Modeling

- `Actor` trait: stateless message handler with typed `Message` associated type
- `DomainEvent`: append-only event contract
- `Command` / `Query`: CQRS command and query types
- Persistence SPI: `EventStore`, `Repository`, `Snapshot`, `PersistenceError`
- CQRS read-side traits (`read_side/`): projection handlers, read-model stores, offset/dedup ports — the read half of the CQRS split
- Pure value types: domain contracts carry no runtime types and no serialization frameworks; `EventStore`'s I/O-shaped methods are `async fn` via `async-trait` as a trait *signature* only — `tokio` itself is not a production dependency of `ego-domain`

### 6.2 Authentication & Authorization (security-sdk + security-jwt + security-apikey)

- `AuthenticationProvider` trait: synchronous, object-safe, injectable
- `AuthorizationProvider` trait: async, RBAC-capable
- `SecurityContext`: carries `Principal` + `Claims`, explicit propagation via `ServiceContext`
- JWT: one algorithm-parameterized authenticator (`JwtAlgorithm::{Hs256,Rs256,Es256}`) over a `KeyResolver` abstraction (`LocalKeyResolver`, `JwksKeyResolver`)
- API-key: `ApiKeyAuthenticationProvider` with constant-time key-hash verification and a pluggable `ApiKeyResolver`
- No ambient security state — `SecurityContext` travels explicitly through `ServiceContext`

### 6.3 Service SDK

Two service-registration mechanisms coexist, selected by what `#[service]` is applied to:

- On a **trait** — generates `ServiceContract` + `ServiceDescriptor` (declarative, discovered/wired through `ServiceRegistry`)
- On a **struct** — generates `Injectable` (constructor-based DI; the primary path for `App::builder()`/`AppBuilder`)
- `ServiceContext`: carries tenant, correlation, causation, security — propagated explicitly
- `Interceptor` / `InterceptorChain`: cross-cutting concerns without framework coupling
- `#[service]` / `#[operation]` / `#[authorize]` / `#[tenant_scoped]` proc-macros — the authorization macros are compile-time enforced (used outside a `#[service]` trait, they fail to compile)
- Idempotency enforcement: fail-closed `IdempotencyEnforcementMode` (default `MandatoryKey`), configured via `RuntimeBuilder`/`AppBuilder`
- Composition: `App::builder()` → `AppBuilder` → `RuntimeBuilder` is the normal application composition path — see [`ARCHITECTURE.md` → Application Composition](ARCHITECTURE.md#application-composition); `RuntimeBuilder` remains the lower-level, directly supported primitive

### 6.4 Runtime & Scheduling

- `Runtime` trait: backend-neutral actor spawn and messaging
- `ExecutionState`: supervised actor lifecycle (Active → Draining → Terminated/Failed)
- `BatchExecutor`: deterministic read-side/projection batch execution with backpressure
- `TagSchedulerImpl`: tag-based projection scheduling
- `EffectInterpreter`: async trait for interpreting domain effects
- Persistent Entity Runtime (`persistent-entity`): actor-per-entity execution with a 5-state lifecycle (Recovering → Active → Passivating → Passivated/Failed), single-flight activation, deterministic recovery — see [`ARCHITECTURE.md` → Persistent Entity Runtime](ARCHITECTURE.md#persistent-entity-runtime-core-006)
- Fail-closed execution: panics terminate the unit of work, not the runtime

### 6.5 Persistence

- In-memory adapters for testing (no external dependencies)
- PostgreSQL adapter via `sqlx`
- Atomic commit: offset + dedup persisted in one transaction
- Append-only event store enforced by type system

### 6.6 External Effect Delivery (PROD-002)

- `EffectStateStore` / `EffectDedupStore` ports defined in `ego-runtime`
- Concrete durable providers in `ego-effect-store`: `PostgresEffectStore`, `StoolapEffectStore` (feature-gated, no default backend)
- Delivered through the `App`/`Runtime` effect-acceptor lifecycle — see [`ARCHITECTURE.md` → Application Composition](ARCHITECTURE.md#application-composition)

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

**[`ROADMAP.md`](ROADMAP.md) is the source of truth for current priority, exact sequencing, and shipped/planned status.** This section only describes the product's high-level direction and does not track individual work items — see ROADMAP.md for that.

ego.rs's evolution moves through six capability phases, in order:

1. **Foundation** — actor model, event sourcing, CQRS read-side projections, persistent entities, authentication/authorization, tenant isolation, service composition, test infrastructure
2. **Application Composition** — the `App`/`AppBuilder` developer-facing composition surface, with `RuntimeBuilder` as the supported lower-level primitive
3. **Production Foundation** — mandatory CI gates, observability, security hardening, health/readiness/startup for single-node deployment
4. **Reliable Distributed Integration** — a distributed-messaging SPI with broker adapters, transactional outbox, and optional CDC
5. **Durable Workflows** — saga/process-manager orchestration built on stable persistence and messaging
6. **Multi-Node Runtime** — distributed deployment guarantees, pursued only after single-node production readiness is proven

Phases 1 and 2 have shipped. Durable external effect delivery — a Production Foundation-adjacent capability — shipped ahead of sequence. Everything from Production Foundation onward is future work.

---

## 10. Success Criteria

A version of ego-rs is ready for broader adoption when:

1. `cargo test --workspace` passes with ≥ 85% coverage
2. A non-trivial service (3+ operations, auth, persistence) can be built using only ego-rs primitives
3. `cargo run -p xtask -- verify-layers` catches all known violation patterns
4. All public APIs carry `rustdoc` documentation
5. A new contributor can understand the architecture from `docs/` alone without reading source code

---

## 11. References

- [`ARCHITECTURE.md`](ARCHITECTURE.md) — engineering & runtime architecture, crate boundaries, design preferences
- [`docs/constitution-mapping.md`](docs/constitution-mapping.md) — constitutional rules and enforcement mechanisms
- [`openspec/specs/`](openspec/specs/) — canonical specifications per domain
