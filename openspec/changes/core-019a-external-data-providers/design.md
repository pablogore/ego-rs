# Design: CORE-019A — External Data Providers SPI

## 1. Technical Approach

Add a runtime-owned **read-side data-provider subsystem** that mirrors, one
concept at a time, the CORE-019 write-side split: an `ExternalDataProvider`
SPI (apps implement, one provider = one kind of external data) and an
`ExternalDataProviderRegistry` (keyed ownership, one owner per id, duplicate
fails at registration). Handlers never touch the registry or a concrete
provider directly; they hold an `Arc<dyn DataProviderAccess>` — a
`persistent-entity` port whose sole runtime impl (`RuntimeDataProviderAccess`)
performs the registry lookup and is the single observability chokepoint. This
reuses CORE-019's port-location split (AD-3/AD-4) verbatim: the handler-facing
trait lives beside the handler, its impl lives in the runtime, adding no
cross-layer edge. The SPI is deliberately monomorphic and transport-unaware:
opaque bytes in (`DataRequest`), opaque bytes out (`DataResponse`), object-safe
for `Arc<dyn …>`. No concrete adapter, no runtime-owned cache, no retry loop
ships here (§9 non-goals).

## 2. Architecture Decisions (AD-001 – AD-012)

| AD | Decision | Rejected | Rationale |
|----|----------|----------|-----------|
| **AD-001 Ownership** | SPI trait + registry owned by `crates/runtime`; handler-facing port owned by `crates/persistent-entity`; wiring in `crates/service-sdk`. | Domain owns the SPI; a new dedicated crate. | Discovery/ownership/lifecycle is a runtime responsibility (CORE-019 AD-4). Domain stays I/O-free. Existing dep shape (`runtime→domain`, `service-sdk→both`) already fits — no new crate earns its keep (CORE-019 §2). |
| **AD-002 Registry** | `ExternalDataProviderRegistry` = `HashMap<String, Arc<dyn ExternalDataProvider>>`; `register(id, provider) -> Result<(), DuplicateProviderId>`, fail-closed at registration, wired via `RuntimeBuilder::register_data_provider`. | Auto-discovery / reflection scanning. | Direct clone of `ExecutorRegistry` (§3 no-reflection, explicit-registration principles). One owner per id; first registration wins, duplicate is a loud error. |
| **AD-003 Resolution** | Hybrid: **B's chokepoint delivered by A's mechanism** — the handler holds an `Arc<dyn DataProviderAccess>` facade (registry-backed, single ownership + instrumentation point) and never holds a concrete provider. | Pure A (concrete provider in handler struct — no chokepoint, registry optional); Model C (inject into `CommandContext`). | Meets the selection criterion (minimize new edges, preserve ownership + observability): the facade port must exist anyway for the chokepoint, so this adds **zero** further edges. C additionally mutates `CommandContext` — a `Serialize`/`Deserialize` DTO with public struct-literal construction — breaking backward compat and coupling a serializable value to a runtime handle. |
| **AD-004 Provider Contract** | `async fn fetch(&self, request: DataRequest) -> Result<DataResponse, DataProviderError>`. `DataRequest { key: String, payload: Vec<u8> }`, `DataResponse { payload: Vec<u8>, cache_hit: bool }`, opaque. | `fetch<T>(…)` / typed results; `query`/`execute` verbs; no request object. | A generic method breaks object-safety, so `Arc<dyn>` registry (AD-002) would be impossible. Opaque bytes mirror CORE-019's opaque `payload`/`destination` — the runtime stays transport-unaware; the handler deserializes. `fetch` matches `KeyResolver::resolve`'s single-verb precedent. |
| **AD-005 Registry Key** | `provider_id: String`. | `TypeId`; trait-object identity; request type as key; dedicated id newtype. | Matches `effect_type: String` and the whole `*Provider` naming convention; directly usable as the observability correlation field (§7). Typed keys drag generics back in (fights object-safety, AD-012). |
| **AD-006 Lifecycle** | Singleton providers, registered at build, held for runtime lifetime. `fetch` may do real I/O — `handle_command`/`apply_event`/`apply_events` are already `async fn` (`persistent-entity/src/persistent_entity.rs`), so there is no synchronous call site requiring a cache-first/`block_on` bridge; caching is a per-provider optimization, never an SPI precondition. Cache (if a provider has one) lives **inside** the provider (never runtime-owned); the runtime never inspects cache contents, only the `cache_hit` flag the provider reports. Optional `async fn shutdown(&self) {}` (default no-op) driven by `register_async_teardown` for providers holding long-lived resources (HTTP pool, gRPC channel, Redis/S3 client). Zero-cost when unused: no registration → no registry, no facade injected. | Scoped/per-request providers; runtime-owned cache store; mandatory shutdown; mandating cache-first/`block_on` (rejected on review — see Correction below). | No real consumer justifies scoped lifetimes or a shared cache (non-goals §9, AD-012). Singleton + optional-teardown is the minimal shape that still accommodates long-lived-resource providers and reuses the existing teardown hook. |
| **AD-007 Error Model** | Adopt the **read-side classification** convention: `DataProviderError { Transient(String), Fatal(String), NotFound { key }, ProviderMissing { provider_id } }`. No retry/backoff policy in this slice. | Reuse CORE-019 `RetryPolicy`; reuse `ProjectionError` verbatim (incl. `PoisonEvent`). | A fetch is inline to command handling — there is no delivery loop, so `RetryPolicy` (write-side) has nothing to drive. `Transient`/`Fatal` mirror `ProjectionError`; `PoisonEvent` is projection-stream-specific and omitted; `NotFound` traces to `KeyResolverError::KeyNotFound`; `ProviderMissing` traces to CORE-019 `ExecutorMissing`. Every variant is grounded in an existing in-repo error — no third convention invented (§13). |
| **AD-008 Observability** | The `RuntimeDataProviderAccess` chokepoint emits one `tracing` span per fetch: `provider_id`, hashed `key`, latency, `cache_hit`, and an explicit `outcome: ProviderOutcome { Success, NotFound, Transient, Fatal, ProviderMissing }` field derived once at the chokepoint. Providers never emit independently. | Providers emit their own telemetry; per-provider metric registries; inferring outcome ad hoc from `DataProviderError` at each call site. | Single chokepoint = consistent signals across the running service (§7). Reuses CORE-012A/CORE-019 `tracing` pipeline. `payload` and sensitive values never logged by default (CORE-019 precedent). An explicit `ProviderOutcome` enum (mirrors `DataProviderError`'s variants) is queryable/alertable without parsing error strings. No `retries` signal — no retry loop exists this slice. |
| **AD-009 Crate Placement** | `crates/runtime/src/providers/{mod,provider,registry,access}.rs`; `crates/persistent-entity/src/data_provider_access.rs` (port + `DataRequest`/`DataResponse`/`DataProviderError` DTOs); `service-sdk/src/runtime/builder.rs` gains registration. | Port trait in `runtime`; new crate. | DTOs and port live in `persistent-entity` so the port signature never references a `runtime` type (would be a wrong-direction edge). Exact mirror of `effect_acceptor.rs` + `effects/` layout. |
| **AD-010 Testability** | `testkit` ships `RecordingDataProvider` / `StaticDataProvider` (canned bytes, records calls); `examples/reference-app` dogfoods one trivial provider through the facade. | Mock frameworks; no test double. | Object-safe async trait → doubles are trivial. Continues CORE-017/018/019 TestKit-first convention; the dogfood is the forcing function against an over-abstract SPI (§13). |
| **AD-011 KeyResolver Relationship** | Establish direction only: `ExternalDataProvider` (general) ← `KeyResolver` (specific). The SPI shape (async, `Send + Sync`, object-safe) is a deliberate superset of `KeyResolver`, so a future retrofit is a thin adapter (`impl ExternalDataProvider` wrapping a `KeyResolver`, registered as `"jwt.verification-key"`), not a rewrite. | Retrofit `KeyResolver` now. | Retrofit is out of scope (§8); forcing it couples this change to the security stack for no first-slice benefit. `key_resolver.rs` is unchanged, referenced as prior art. `KeyResolver`'s own cache-first/`block_on` constraint is specific to its sync call site and is *not* generalized onto `ExternalDataProvider` (AD-006 correction). |
| **AD-012 Genericity** | SPI is monomorphic: no generic parameters, no `fetch<T>`, no typed-provider registry. **Rule**: no new generic parameters or typed helper APIs may be introduced until at least one additional production provider demonstrates a concrete need — a second real consumer, not speculation, is the bar. | Generic/typed SPI now. | Object-safety (AD-002/AD-004) and the guardrail against the over-general-SPI risk (§13): with zero real consumers, genericity is pure speculation. Typed convenience is a handler-side/future-helper concern. |

### Corrections (post-review)

- **AD-006, PR1 review**: the original AD-006 mandated `fetch` be cache-first
  so a `futures_executor::block_on` sync bridge (copied from
  `KeyResolver`'s test) would stay correct. Review found this unjustified:
  `PersistentEntity::handle_command`/`apply_event`/`apply_events` are
  already `async fn` — there is no synchronous call site for this SPI that
  needs a `block_on` bridge. The mandate was dropped; `fetch` may do real
  I/O. Caching remains a valid per-provider optimization (and `cache_hit`
  remains a valid observability signal, AD-008), just no longer an SPI
  precondition. `futures-executor` was removed from `crates/runtime`'s
  dev-dependencies; the corresponding test now proves only object-safety
  and result propagation via `#[tokio::test]`.

## 3. Extension-Surface Classification

| Category | Types |
|----------|-------|
| **Public SPI** | `ExternalDataProvider`, `ExternalDataProviderRegistry`, `DuplicateProviderId` (runtime); `DataProviderAccess` port + `DataRequest`, `DataResponse`, `DataProviderError` DTOs (persistent-entity) |
| **Internal runtime** | `RuntimeDataProviderAccess` (the port impl / observability chokepoint) |
| **Private helper** | key-hashing + span helpers in `access.rs` |

## 4. Dependency Edges (layering preserved)

| Edge | Direction | Verdict |
|------|-----------|---------|
| `runtime` → `persistent-entity` (`RuntimeDataProviderAccess` impls the port; SPI uses the DTOs) | allowed (`domain → persistent-entity/runtime`… wait: `runtime→persistent-entity` is CORE-019's existing direction) | OK — identical to `RuntimeEffectAcceptor` impl of `EffectAcceptor` |
| `persistent-entity` → `runtime` | never introduced | OK — port + DTOs are self-contained in `persistent-entity` |
| `service-sdk` → `runtime` + `persistent-entity` | allowed (pre-existing) | OK |

No new cross-layer edge; the port + DTOs living in `persistent-entity` is what
keeps it that way.

## 5. Data Flow

    App: impl ExternalDataProvider ──register_data_provider──▶ RuntimeBuilder (service-sdk)
                                                                     │ build
                              ExternalDataProviderRegistry (runtime, one owner per provider_id)
                                                                     │ wrapped by
                              RuntimeDataProviderAccess (runtime) ─── tracing chokepoint
                                                                     │ Arc<dyn DataProviderAccess>
                                                                     ▼ handed to handler at wiring
    handle_command(cmd, state, ctx):
        access.fetch(provider_id, DataRequest { key, payload })
            ├─ registry lookup ── miss ─▶ DataProviderError::ProviderMissing (fail closed, signalled)
            └─ hit ─▶ Provider::fetch (may do real I/O) ─▶ DataResponse { payload, cache_hit }
        emit span: provider_id, hashed key, latency, outcome, cache_hit   (payload never logged)

## 6. Interfaces

```rust
// persistent-entity — Public SPI (handler-facing)
pub struct DataRequest  { pub key: String, pub payload: Vec<u8> }
pub struct DataResponse { pub payload: Vec<u8>, pub cache_hit: bool }
pub enum DataProviderError { Transient(String), Fatal(String),
                             NotFound { key: String }, ProviderMissing { provider_id: String } }

#[async_trait]
pub trait DataProviderAccess: Send + Sync {
    async fn fetch(&self, provider_id: &str, request: DataRequest)
        -> Result<DataResponse, DataProviderError>;
}

// runtime — Public SPI (apps implement); object-safe → Arc<dyn>
#[async_trait]
pub trait ExternalDataProvider: Send + Sync {
    async fn fetch(&self, request: DataRequest) -> Result<DataResponse, DataProviderError>;
    async fn shutdown(&self) {} // default no-op; override for long-lived resources
}
```

## 7. File Changes

| File | Action | Description |
|------|--------|-------------|
| `crates/runtime/src/providers/mod.rs` | Create | Subsystem root, re-exports |
| `crates/runtime/src/providers/provider.rs` | Create | `ExternalDataProvider` SPI |
| `crates/runtime/src/providers/registry.rs` | Create | `ExternalDataProviderRegistry`, `DuplicateProviderId` (fail-closed) |
| `crates/runtime/src/providers/access.rs` | Create | `RuntimeDataProviderAccess` chokepoint (lookup + tracing) |
| `crates/persistent-entity/src/data_provider_access.rs` | Create | `DataProviderAccess` port + `DataRequest`/`DataResponse`/`DataProviderError` |
| `crates/service-sdk/src/runtime/builder.rs` | Modify | `register_data_provider`, teardown wiring |
| `crates/testkit/...` | Modify | `RecordingDataProvider` / `StaticDataProvider` |
| `examples/reference-app/...` | Modify | One trivial dogfood provider through the facade |
| `openspec/specs/external-data-providers/` | Create | Canonical spec (spec phase) |
| `crates/security-jwt/src/key_resolver.rs` | Referenced | Prior art only; unchanged |

## 8. Testing Strategy

| Layer | What | Approach |
|-------|------|----------|
| Unit | Registry one-owner-per-id, duplicate fails; `DataProviderError` classification | `runtime` unit tests |
| Unit | Object-safety (`Arc<dyn ExternalDataProvider>`) and result propagation, via `#[tokio::test]` — no `block_on` bridge, since the real call site (`handle_command`) is already async | Async trait-object test, distinct-response triangulation |
| Integration | Fail-closed on unregistered `provider_id`; facade emits signals; payload never logged | `RuntimeDataProviderAccess` + `testkit` double |
| Integration | Two providers registered under distinct `provider_id`s never cross-resolve, even given structurally identical `DataRequest`s | Register two `testkit` doubles under different ids, fetch both, assert each returns its own provider's response |
| E2E | Reference-app handler fetches through a registered provider (never inline client) | Dogfood provider (§16 success criterion 1) |

## 9. Threat Matrix

N/A — no routing, shell, subprocess, VCS/PR automation, executable-file
classification, or process-integration boundary. This is an in-process SPI.

## 10. Migration / Rollout

No migration. All new modules behind opt-in registration (AD-006 zero-cost when
unused). Rollback = remove `register_data_provider` and delete the modules; no
existing handler, frozen domain type, or `KeyResolver` call site is touched.

## 11. Open Questions

- [ ] **Tenant scoping (spec-owned):** if fetches are tenant-scoped (CORE-008A),
      the established tenant MUST be stamped by the facade/runtime, never
      accepted from a handler-built `DataRequest` ("cannot mint"). Deferred
      until the first real consumer validates the shape (AD-012). The
      handler-held-facade model (AD-003) makes per-command tenant binding a
      real design point for that consumer.
- [ ] First concrete domain use case — none named in-repo; the trivial
      reference-app provider stands in until a real consumer exists.

**Implementation watch-items (not blocking, not a design change — revisit
only when the first real provider lands):**

- `provider_id: String` collision risk across crates (e.g. two unrelated
  crates both registering `"pricing"`). AD-005 stands as decided; if this
  bites in practice, a namespacing convention (`crate::provider`) is an
  additive change, not a redesign.
- `DataRequest`/`DataResponse` opaque-bytes ergonomics — every call pays a
  serialize/deserialize tax. This is the design's biggest accepted
  tradeoff (traded for object-safety, AD-004); watch whether it's
  materially uncomfortable once the reference-app dogfood is real code,
  not a hypothesis.
- Genericity pressure (AD-012) — if the first real provider strains the
  monomorphic contract, that is itself the "second real consumer" signal
  AD-012 requires before any generic parameter is considered.
- **PR1 review (G-02):** `DataProviderError::Transient(String)` and
  `Fatal(String)` carry provider-authored free text, which may contain
  sensitive detail. When Phase 3's chokepoint (`RuntimeDataProviderAccess`)
  emits its `tracing` span, do not log these strings directly — only the
  `ProviderOutcome` variant (AD-008), never the message payload.
