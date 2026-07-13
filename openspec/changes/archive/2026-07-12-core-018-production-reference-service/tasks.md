# Tasks: CORE-018 — Production Reference Service

Reads `proposal.md` (#1210), `specs/reference-service/spec.md` + `specs/http-transport/spec.md`
(#1211), and `design.md` (#1212 — AD-1..AD-7). Strict TDD (`cargo test --workspace`): every
GREEN task is preceded by its RED test task.

**Ground truth reverified during breakdown** (corrections to design.md's own sketch):

- Design AD-3 assumes `state.security_providers()` is callable from `ego-transport`. It is
  not: `RuntimeInner::authorization_provider()` (`crates/service-sdk/src/runtime/runtime_builder.rs:331`)
  is the only such accessor, is `#[doc(hidden)] pub` **solely for macro-codegen visibility**,
  and its own doc says "Application code MUST NOT call this method directly." No authn
  equivalent exists at all. Resolution: `ego-transport::AppState` carries
  `Arc<dyn AuthenticationProvider>` directly (constructed by `reference-app`, alongside
  `Arc<Runtime>`) — never fished from `Runtime` internals. Zero changes to `service-sdk`,
  preserving the proposal's "Zero non-transport framework changes" success criterion.
- `security-sdk` already ships `RequestContext` + `BearerExtractor`
  (`crates/security-sdk/src/credential_extractor.rs`) — transport-neutral bearer-token
  parsing already exists. `ego-transport`'s extractor wraps `axum::http::HeaderMap` in a
  `RequestContext` impl and reuses `BearerExtractor` + `Hs256AuthenticationProvider::authenticate`
  (sync, `crates/security-sdk/src/authentication/mod.rs:20`) directly — no new bearer-parsing
  logic in transport, easier to keep AD-2 "mechanism only" than design implied.
- Spec's `reference-service` capability names the `TenantOrganization` event
  `UserAssociatedWithTenant` with state "current membership set". design.md AD-6 settled a
  different, simpler, decision-backed shape: `Command::Ensure{org_id,name}` →
  `Event::OrganizationEnsured` → `State::Absent | Present{name}` (idempotent ensure, no
  membership tracking — this idempotency is exactly what makes AD-5's "benign reusable
  orphan" claim true). Implementing per AD-6 (the concrete, rationale-backed artifact);
  spec's `UserAssociatedWithTenant`/"membership set" wording is stale prose to reconcile at
  `sdd-verify`, not a second event to build.
- `#[tenant_scoped]` resolves tenant from `SecurityContext.principal().tenant_id`
  (`crates/service-sdk/src/runtime/tenant.rs`, branch b) **or** a caller-supplied
  `ServiceContext::with_tenant_id(...)` hint compared against it (branch c). Spec's
  "Cross-tenant request denied" scenario is exercised by setting the hint (target tenant)
  to differ from the authenticated principal's own tenant — no new SDK surface needed
  (`crates/service-sdk/src/context/mod.rs:53-65,320`).
- CORE-012A's `RuntimeBuilder::with_observability(...)` already auto-records macro-guard
  denials (`#[authorize]`/`#[tenant_scoped]` failures) — Phase 6's two denial scenarios get
  observability "for free" at the runtime level. Only the **success** and **partial-failure**
  business outcomes (Phase 8) need an explicit `obs.trace(...)` call inside `RegisterUserImpl`,
  since those aren't guard-macro paths.
- `TestKit::ServiceTestFixture::builder().with_service::<Tag>(...).principal(...).authorization(...).build()`
  (`crates/testkit/src/fixtures.rs`) is the real guard-chain harness — reused as-is, no new
  TestKit surface.
- `DomainEvent` (`crates/domain/src/event.rs:47-61`) requires 4 methods:
  `aggregate_id() -> &str`, `event_type() -> &str`, `payload() -> &serde_json::Value`,
  `occurred_at() -> &DateTime<Utc>` — both new event types must carry a stored
  `serde_json::Value` payload field, not just derive `Serialize`.

---

## Phase 1 — `ego-transport` foundation: `AppState` + error mapper (AD-1, AD-2)

- [x] TASK-001 [RED]: Table test for `ServiceError`/`SecurityError` → `StatusCode` mapping (validation
  → 400, business/not-found → 404/409 as applicable, infra → 500; `SecurityError::MissingContext`/
  `AuthorizationDenied` → 401/403, `TenantMismatch` → 403), asserting no raw error `Debug` text
  leaks into the response body. File: `crates/transport/src/error.rs` (new, test module).
  Satisfies: http-transport spec "Success/Error Response Contract".
- [x] TASK-002 [GREEN]: Implement `crates/transport/src/error.rs` — `TransportError` enum +
  `IntoResponse` impl + `From<ServiceError>`/`From<SecurityError>` conversions per TASK-001.
- [x] TASK-003 [RED]: Test `AppState` is `Clone + Send + Sync` and a tag registered on its inner
  `Runtime` resolves through it. File: `crates/transport/src/state.rs` (new, test module).
- [x] TASK-004 [GREEN]: Implement `crates/transport/src/state.rs` —
  `pub struct AppState { pub runtime: Arc<Runtime>, pub authn: Arc<dyn AuthenticationProvider> }` + `Clone`.

## Phase 2 — Security extractor reusing `BearerExtractor` (AD-3)

- [x] TASK-005 [RED]: Unit test — an `AxumRequestContext` wrapping `http::HeaderMap` implements
  `RequestContext::header()` case-insensitively (mirrors existing `MockRequestContext` tests in
  `credential_extractor.rs`). File: `crates/transport/src/security.rs` (new, test module).
- [x] TASK-006 [GREEN]: Implement `AxumRequestContext<'a>(&'a HeaderMap)` + `RequestContext` impl.
- [x] TASK-007 [RED]: Integration test — missing `Authorization` header → 401 rejection before any
  handler runs; malformed `Bearer` → 401; a real Hs256-signed JWT → `Ok(SecurityContext)` with
  matching principal/tenant claims. File: `crates/transport/tests/security_extractor.rs` (new).
  Satisfies: http-transport spec "Missing or invalid credentials rejected pre-invocation",
  "Valid credentials produce a SecurityContext".
- [x] TASK-008 [GREEN]: Implement `AuthenticatedContext(pub SecurityContext)` +
  `impl FromRequestParts<AppState> for AuthenticatedContext` in `security.rs`, using
  `BearerExtractor::extract` + `state.authn.authenticate(...)`, mapping any failure to
  `TransportError::Unauthorized` (401) via `TASK-002`'s mapper.

## Phase 3 — `serve()` bootstrap + crate wiring (AD-7, AD-2)

- [x] TASK-009 [RED]: Integration test — `serve()` binds an ephemeral `TcpListener`
  (`127.0.0.1:0`), serves a trivial router, a real client request succeeds, then a shutdown
  signal makes `serve()` return `Ok(())` within a bounded `tokio::time::timeout`. File:
  `crates/transport/tests/server.rs` (new).
- [x] TASK-010 [GREEN]: Implement `crates/transport/src/server.rs` —
  `pub async fn serve(listener: TcpListener, router: Router<AppState>, shutdown: impl Future<Output = ()> + Send + 'static) -> std::io::Result<()>`
  wrapping `axum::serve(...).with_graceful_shutdown(shutdown)`.
  **Ground-truth correction found during apply**: `Router<AppState>` does not compile as a
  parameter type — `axum::serve` requires `M: Service<IncomingStream, ...>`, which `Router<S>`
  only implements for `S = ()` (state already applied via `.with_state(..)`). Implemented as
  `pub async fn serve(listener: TcpListener, router: Router, shutdown: impl Future<Output = ()> + Send + 'static) -> std::io::Result<()>`
  instead — the caller calls `.with_state(app_state)` before passing `router` in (matches
  design.md's AD-7 data flow: `main`'s `router.with_state(rt.clone())`). No behavioral change,
  the RED test (`tests/server.rs`) exercises the corrected signature directly.
- [x] TASK-011 [GREEN]: Update `crates/transport/src/lib.rs` — export `AppState`,
  `AuthenticatedContext`, `TransportError`, `serve`; remove the stale "Provides HTTP/gRPC
  handlers" doc claim (AD-2: no gRPC). Update `crates/transport/Cargo.toml` — add
  `ego-service-sdk`, `ego-security-sdk` path deps (needed for `Runtime`/`AuthenticationProvider`/
  `SecurityContext` types used in Phases 1-2). Also added `async-trait` (production dep — axum's
  `FromRequestParts` trait is `#[async_trait]`-based in 0.7.9, so the impl needs the same macro)
  and, as dev-dependencies only, `ego-service-sdk-macros` (TASK-003's resolve test needs a real
  `#[service]`-generated tag), `security-jwt` + `jsonwebtoken` (TASK-007's real Hs256 JWT).

## Phase 4 — `User` `PersistentEntity` (AD-6)

- [x] TASK-012 [RED]: Test `handle_command(Register{user_id,email,tenant_id})` on `Unregistered`
  produces exactly one `UserRegistered` event; `apply_event` transitions to
  `Registered{email,tenant_id}`. File: `examples/reference-app/tests/user_entity.rs` (new).
  Satisfies: reference-service spec "Registering a user".
- [x] TASK-013 [GREEN]: Implement `examples/reference-app/src/domain/user.rs` —
  `UserCommand::Register{user_id,email,tenant_id}`; `UserRegistered{user_id,email,tenant_id,
  occurred_at,payload}` implementing `DomainEvent` (`aggregate_id`=user_id,
  `event_type`="UserRegistered"); `UserState::Unregistered | Registered{email,tenant_id}`;
  `UserEntity: PersistentEntity`.

## Phase 5 — `TenantOrganization` `PersistentEntity`, idempotent ensure (AD-5, AD-6)

- [x] TASK-014 [RED]: Test `handle_command(Ensure{org_id,name})` on `Absent` produces
  `OrganizationEnsured` → `Present{name}`; calling `Ensure` again on `Present` returns
  `CommandResult::NoEvents` (idempotent — the property AD-5's "benign orphan" depends on).
  File: `examples/reference-app/tests/tenant_org_entity.rs` (new). Satisfies: reference-service
  spec "Ensuring a tenant org exists" (per this breakdown's ground-truth note on AD-6's
  event-shape resolution).
- [x] TASK-015 [GREEN]: Implement `examples/reference-app/src/domain/tenant_org.rs` —
  `TenantOrgCommand::Ensure{org_id,name}`; `OrganizationEnsured{org_id,name,occurred_at,payload}`
  `DomainEvent` impl; `TenantOrgState::Absent | Present{name}`;
  `TenantOrganizationEntity: PersistentEntity` with the idempotent `handle_command` branch.

## Phase 6 — `RegisterUser` guard chain + happy path (AD-4)

- [x] TASK-016 [RED]: TestKit test — `ServiceTestFixture::builder().with_service::<RegisterUserTag>(...)
  .authorization(deny_all).build()`; invoking `register` is denied and neither entity write occurs
  (assert no live registry entry for the target `user_id`/`org_id`). File:
  `examples/reference-app/tests/register_user_guard_chain.rs` (new). Satisfies: reference-service
  spec "Unauthorized principal denied".
- [x] TASK-017 [RED]: Same fixture, authorized principal with tenant `"tenant-a"`, `ServiceContext
  ::with_tenant_id("tenant-b")` on the call → `TenantMismatch` denial, no entity write. Satisfies:
  "Cross-tenant request denied".
- [x] TASK-018 [RED]: Same fixture, authorized + matching tenant → `Ok(RegisterOutput)`, org entity
  `Present`, user entity `Registered`. Satisfies: "Successful registration".
- [x] TASK-019 [GREEN]: Implement `examples/reference-app/src/service.rs` — `#[service(version =
  "1.0.0")] trait RegisterUser` with `#[operation] #[authorize(context = ctx, permission =
  "user:register")] #[tenant_scoped] async fn register(...)`; `RegisterUserImpl` holding both
  `Arc<EntityRuntime<_>>`s, org-first sequencing per AD-5 (`Ensure` then `Register`).
  **Ground-truth addition found during apply**: `TestKit::FixtureBuilder` had no
  `with_observability` pass-through (needed by Phase 8) — added one, thin pass-through mirroring
  `with_service`'s existing style (`crates/testkit/src/fixtures.rs`).

## Phase 7 — Non-atomic dual-write, partial-failure proof (AD-5)

- [x] TASK-020 [RED]: Partial-failure test — drive a `User` write failure for a specific trigger
  input after the `TenantOrganization` write has already succeeded; assert `RegisterUser`
  returns `Err`, the org entity is `Present` (not rolled back), and a subsequent `Ensure` on the
  same `org_id` returns `NoEvents` (proves the "benign reusable orphan" claim, not just
  "org still exists"). File: `examples/reference-app/tests/register_user_partial_failure.rs`
  (new). This is the critical RED test proving the documented limitation is real, not
  accidental — write it before touching TASK-019's error-propagation path. Satisfies:
  reference-service spec "TenantOrganization succeeds, User write fails".
  **Ground-truth addition**: `UserEntity::handle_command` needed a real (non-test-only)
  validation trigger to fail deterministically — added "email must not be empty" as a genuine
  validation rule (RED/GREEN in `tests/user_entity.rs` first), reused by this test as the
  User-write failure trigger.
- [x] TASK-021 [GREEN]: Verify/adjust TASK-019's error propagation so a `User`-write failure
  surfaces unmodified (no compensating delete of the org). If org-first ordering alone already
  satisfies TASK-020, this is a verification gate, not new logic. Confirmed: org-first sequencing
  alone satisfies it, no compensating-delete code exists.

## Phase 8 — Observability test-double assertions (spec Observability requirement)

- [x] TASK-022 [RED]: Extend Phase 6/7 fixtures with `.with_observability(Arc::new(RecordingObservability::new()))`;
  assert success (TASK-018) and partial-failure (TASK-020) each record ≥1 event via
  `RegisterUserImpl`'s explicit trace call; assert the two guard denials (TASK-016/017) are
  already recorded by CORE-012A's existing macro-guard wiring (no new code needed for those
  two). File: `examples/reference-app/tests/register_user_observability.rs` (new, or extend
  Phase 6/7 files directly).
- [x] TASK-023 [GREEN]: Thread an `Option<Arc<dyn Observability>>` through `RegisterUserImpl`'s
  constructor; call `obs.trace(...)` for the success and partial-failure business outcomes only
  (guard denials are already covered — see ground-truth note above).

## Phase 9 — HTTP route wiring + end-to-end acceptance (AD-1, AD-7)

- [x] TASK-024 [GREEN]: Update `examples/reference-app/src/lib.rs` — build two `EntityRuntime`s
  (`EntityRuntimeBuilder::<UserRegistered>::new().build()`, `::<OrganizationEnsured>::new().build()`),
  construct `RegisterUserImpl`, register via `RuntimeBuilder::with_service::<RegisterUserTag>(...)`;
  expose `build_runtime()`'s already-constructed `authn: Arc<dyn AuthenticationProvider>`
  (currently discarded after `.with_security(authn, authz)`) so the bin can build `AppState`.
  **Ground-truth correction found during apply**: `build_runtime`'s return type changed from
  `Result<Runtime, _>` to `Result<(Runtime, Arc<dyn AuthenticationProvider>), _>` — both existing
  callers (`main.rs`, `tests/pipeline.rs`) only ever checked `.is_ok()`/destructured with `_`, so
  this is a non-breaking, additive change. Also found and fixed a latent bug: the pre-existing
  25-byte HMAC signing-key literal fails `Hs256AuthenticationProvider`'s NIST SP 800-107 32-byte
  minimum — never exercised before this PR (no HTTP layer existed to invoke `authenticate`).
  Lengthened via a new `pub const DEV_SIGNING_KEY`.
- [x] TASK-025 [RED]: Router-level test (no real socket — `tower::ServiceExt::oneshot`) — `POST
  /register` with a valid Bearer JWT + valid JSON body → 201; with no `Authorization` header →
  401 and `RegisterUser` never invoked. File: `examples/reference-app/tests/http_route.rs` (new).
  Satisfies: http-transport spec "Request reaches the guarded operation", "Outcomes map to
  appropriate responses".
- [x] TASK-026 [GREEN]: Implement the `POST /register` handler (`examples/reference-app/src/routes.rs`
  or inline in the bin) — resolves `state.runtime.resolve::<RegisterUserTag>()`, builds
  `ServiceContext::new().with_security(Arc::new(sc)).with_tenant_id(&input.tenant_id)`, maps
  `Ok`/`Err` via `TransportError`.
- [x] TASK-027 [RED]: Full E2E test — real `axum::serve()` via `ego_transport::serve` on an
  ephemeral port, a real HTTP client: (a) no JWT → 401, operation never reached; (b) validly
  signed Hs256 JWT + valid payload → 201, both entities persisted. File:
  `examples/reference-app/tests/e2e_register.rs` (new). Satisfies proposal's explicit success
  criterion "A real HTTP request against a running axum server completes registration
  end-to-end."
- [x] TASK-028 [GREEN]: Implement `examples/reference-app/src/bin/server.rs` `main()` —
  `build_runtime()` → `Arc::new(rt)` → `Router::new().route("/register", post(register_handler))
  .with_state(AppState{...})` → `ego_transport::serve(listener, router, shutdown_signal)` → on
  return, `rt.shutdown()` (AD-7 teardown order: stop accepting, then shutdown Runtime — verified
  `Runtime::shutdown()` exists at `crates/service-sdk/src/runtime/builder.rs:278`).

## Phase 10 — Non-goal / scope-boundary verification (no code)

- [x] TASK-029 [Verify only]: Confirm no saga/compensation/outbox code exists anywhere in
  `crates/transport` or `examples/reference-app` (grep `saga|compensat|outbox`; zero matches
  outside doc comments describing the non-goal). Confirmed: only 2 matches, both in
  `service.rs` doc comments describing the non-goal.
- [x] TASK-030 [Verify only]: Confirm `crates/transport/Cargo.toml` has no `tonic`/gRPC dependency
  added; `GrpcServerConfig` (`crates/transport/src/config.rs`) is untouched — it predates this
  change and is out of scope, not a "no gRPC" violation. Confirmed: no tonic/grpc references in
  either Cargo.toml; `git status --porcelain crates/transport/src/config.rs` shows no change.
- [x] TASK-031 [Verify only]: Confirm no production `Observability` adapter was added — only
  `RecordingObservability` test doubles under `#[cfg(test)]`/`tests/`; `crates/infrastructure/src/observability.rs`
  unchanged. Confirmed: the only new `Observability` implementors are test doubles in
  `examples/reference-app/tests/register_user_observability.rs`; `RegisterUserImpl` only calls
  the trait, it does not implement or ship an adapter. Also applied the recommended spec.md fix
  (TenantOrganization table row + scenario rewording, per verify-report-pr2's exact diff).

## Traceability

| Spec requirement | Scenario | Task(s) |
|---|---|---|
| PersistentEntity Contracts | Registering a user | TASK-012, TASK-013 |
| PersistentEntity Contracts | Ensuring a tenant org exists | TASK-014, TASK-015 |
| Authorization and Tenant-Scoping | Unauthorized principal denied | TASK-016, TASK-019 |
| Authorization and Tenant-Scoping | Cross-tenant request denied | TASK-017, TASK-019 |
| RegisterUser Happy Path | Successful registration | TASK-018, TASK-019 |
| Non-Atomic Dual Write | TenantOrganization succeeds, User write fails | TASK-020, TASK-021 |
| RegisterUser Observability | Success and failure are observed | TASK-022, TASK-023 |
| HTTP Route Reaches RegisterUser | Request reaches the guarded operation | TASK-025, TASK-026 |
| Security Context Extraction | Missing/invalid credentials rejected | TASK-007, TASK-008 |
| Security Context Extraction | Valid credentials produce a SecurityContext | TASK-007, TASK-008 |
| Success/Error Response Contract | Outcomes map to appropriate responses | TASK-001, TASK-002, TASK-025 |
| (proposal) HTTP e2e success criterion | Real socket, full flow | TASK-027, TASK-028 |
| Non-goals (saga, gRPC, prod adapter) | — | TASK-029, TASK-030, TASK-031 |

---

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | ~1,100-1,400 across 2 crates, ~18 files (7 new in transport/tests, 11 new in reference-app/tests+domain+bin) |
| 400-line budget risk | High |
| Chained PRs recommended | Yes |
| Suggested split | PR 1 → PR 2 → PR 3 |
| Delivery strategy | ask-on-risk (default — not explicitly supplied to this task-breakdown; orchestrator should confirm) |
| Chain strategy | pending — user decision required |

Decision needed before apply: Yes
Chained PRs recommended: Yes
Chain strategy: pending
400-line budget risk: High

### Suggested Work Units

| Unit | Goal | Likely PR | Focused test command | Runtime harness | Rollback boundary |
|------|------|-----------|----------------------|-----------------|-------------------|
| 1 | Phases 1-3: `ego-transport` generic axum layer — `AppState`, JWT `SecurityContext` extractor (reusing `BearerExtractor`), error mapper, `serve()` bootstrap | PR 1 | `cargo test -p ego-transport` | `crates/transport/tests/server.rs` real ephemeral-socket test | Revert `crates/transport/src/{state,security,error,server}.rs` + `lib.rs`/`Cargo.toml` hunks — additive, no reference-app code depends on it yet if PR 1 lands alone |
| 2 | Phases 4-5: `User` + `TenantOrganization` `PersistentEntity` aggregates | PR 2 | `cargo test -p reference-app --test user_entity --test tenant_org_entity` | Direct entity unit tests (no live runtime process needed) | Revert `examples/reference-app/src/domain/{user,tenant_org}.rs` + their tests — independent of PR 1/3, no service wires to them yet |
| 3 | Phases 6-10: `RegisterUser` guard chain, non-atomic partial-failure proof, observability, HTTP route wiring, e2e acceptance, scope-boundary verification | PR 3 | `cargo test -p reference-app` (full crate) | `examples/reference-app/tests/e2e_register.rs` — real `axum::serve()` + real HTTP client + real Hs256 JWT | Revert `service.rs`, `lib.rs` wiring, `bin/server.rs`, and all `tests/register_user_*.rs`/`http_route.rs`/`e2e_register.rs` — depends on PR 1 (transport) and PR 2 (entities) both being merged first |

Rationale for High risk / 3-PR chain: two crates touched, one crate (`ego-transport`) going
from a 7-line stub to a real mechanism layer, two new `PersistentEntity` aggregates, one new
guarded service with non-trivial dual-write + partial-failure + observability test coverage,
and a real end-to-end HTTP acceptance test — each concern is independently reviewable and
each PR has a clean, testable finish line matching the user's requested 3-slice split.

---

Total: 31 tasks across 10 implementation phases (+ 1 traceability phase, no code). Sequential
within each phase (RED strictly gates GREEN); Phase 3 depends on 1-2; Phase 6 depends on 3-5;
Phase 7 depends on 6; Phase 8 depends on 6-7; Phase 9 depends on 1-3 and 6-8. Phase 10 is
independent (verification-only, no dependency, run any time after Phase 9).
