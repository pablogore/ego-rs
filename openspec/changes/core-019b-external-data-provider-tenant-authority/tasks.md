# Tasks: CORE-019B — External Data Provider Tenant Authority

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | ~460-620 (two error variants + `Debug` arms; two `ProviderOutcome` classifications + `is_retryable`/`from_tenant_error`; `reconcile_tenant` + `fetch_scoped` observable chokepoint; new `TenantScopedDataProviderAccess` per-dispatch wrapper; actor→handler dispatch wiring; unit + integration + observability + cross-tenant + end-to-end tests) |
| 400-line budget risk | Medium (was Low; raised after per-dispatch wrapper, real-dispatch wiring, observability outcomes, and the end-to-end test were added) |
| Chained PRs recommended | Yes |
| Chain strategy | feature-branch-chain (PR1 → PR2); only the tracker merged to develop |
| Delivery strategy | auto-forecast (no explicit ask-on-risk/auto-chain/single-pr/exception-ok label given) |

Decision needed before apply: Yes (ADR-1 inject-vs-validate-vs-both — resolved: BOTH, five-row matrix; ADR-3 tenant delivery — resolved: immutable per-dispatch wrapper)
Chained PRs recommended: Yes
400-line budget risk: Medium

### Suggested Work Units

| Unit | Goal | Likely PR | Focused test command | Rollback boundary |
|---|---|---|---|---|
| 1 | Error variants + `ProviderOutcome` classifications + `reconcile_tenant` + observable `fetch_scoped` (all fail-closed cases, one terminal signal, zero retries, zero provider calls); unit + integration on hand-built instances | PR1 | `cargo test -p ego-persistent-entity data_provider_access:: && cargo test -p ego-runtime providers::access::` | Drop `fetch_scoped`/`reconcile_tenant`; remove the two outcome classifications and error variants; restore pass-through `fetch` |
| 2 | `TenantScopedDataProviderAccess` per-dispatch wrapper + actor→handler dispatch wiring + end-to-end real-dispatch binding test + cross-tenant negative | PR2 | `cargo test -p ego-runtime providers::access:: && cargo test -p ego-persistent-entity` | Remove the wrapper type + the dispatch-construction diff; the shared `RuntimeDataProviderAccess` (PR1) is unaffected |

## Phase 1: Fail-Closed Error Surface

- [ ] TASK-001 RED: add a failing test in `crates/persistent-entity/src/data_provider_access.rs` (`mod tests`) asserting that `DataProviderError::TenantMismatch` AND `DataProviderError::TenantContextMissing` variants exist, are `PartialEq`, are distinct from each other, and that each one's `Debug` and `Display` renderings contain **no** raw tenant identifier (build each error and assert the formatted strings contain no sentinel tenant string). References design Error Model + Threat Matrix "Silent widening". AC: fails to compile because the variants do not yet exist.
- [ ] TASK-002 GREEN: add `#[error("...")] TenantMismatch` and `#[error("...")] TenantContextMissing` to `DataProviderError`, extending the hand-written `Debug` impl with `Self::TenantMismatch => f.write_str("TenantMismatch")` and `Self::TenantContextMissing => f.write_str("TenantContextMissing")` arms (matching the file's redaction style, `data_provider_access.rs:159-175`). AC: TASK-001 green.
- [ ] TASK-003 COMPAT (characterization — NOT a RED→GREEN pair): add a characterization test asserting the tenant-agnostic constructor path is unchanged — `DataRequest::new("k", vec![]).tenant == None`, and `for_tenant`/`with_tenant` still set `Some(tenant)` (compatibility guard for ADR-2). Update only the `DataRequest.tenant` doc comment to describe it as a caller assertion validated by the runtime (no code behavior change). AC: test passes against the unchanged constructors; `cargo build -p ego-persistent-entity` succeeds.

## Phase 2: Non-Retryable Fetch Outcome Classifications

- [ ] TASK-004 RED: add a failing unit test in `crates/runtime/src/providers/access.rs` (`mod tests`) asserting `ProviderOutcome` has `TenantMismatch` and `TenantContextMissing` classifications, that `is_retryable()` returns `false` for BOTH (the non-retryable proof required by the review minor), and that `from_tenant_error(&DataProviderError::TenantMismatch)`/`(&DataProviderError::TenantContextMissing)` map to the matching outcomes. AC: fails because the classifications do not yet exist.
- [ ] TASK-005 GREEN: add the two `ProviderOutcome` classifications, extend `is_retryable()` to return `false` for both, and add a `from_tenant_error(&DataProviderError) -> ProviderOutcome` mapping. AC: TASK-004 green.

## Phase 3: Reconciliation — The Five-Row Matrix

- [ ] TASK-006 RED: add a failing unit test for `reconcile_tenant(authoritative, &mut request)`: authority `Some(A)` + request tenant `None` ⇒ request tenant becomes `Some(A)` (inject). AC: fails because `reconcile_tenant` does not exist yet.
- [ ] TASK-007 GREEN: implement `fn reconcile_tenant(authoritative: Option<&TenantId>, request: &mut DataRequest) -> Result<(), DataProviderError>` with the inject arm; remove any tenant field from `RuntimeDataProviderAccess` (the shared chokepoint holds no tenant — ADR-3). AC: TASK-006 green.
- [ ] TASK-008 RED: add a failing test — authority `Some(A)` + request `Some(A)` ⇒ `Ok`, tenant stays `Some(A)` (match); authority `None` + request `None` ⇒ `Ok`, tenant stays `None` (tenant-agnostic pass). AC: fails until the match and both-absent arms exist.
- [ ] TASK-009 GREEN: implement the match and tenant-agnostic arms. AC: TASK-008 green.
- [ ] TASK-010 RED: add a failing test — authority `Some(A)` + request `Some(B)`, `B != A` ⇒ `Err(DataProviderError::TenantMismatch)`; authority `None` + request `Some(C)` ⇒ `Err(DataProviderError::TenantContextMissing)` (distinct). AC: fails until both fail-closed arms exist.
- [ ] TASK-011 GREEN: implement the two fail-closed arms, completing the five-row matrix exactly as design ADR-1. AC: TASK-010 green.

## Phase 4: Observable Chokepoint — `fetch_scoped`

- [ ] TASK-012 RED: add a failing `#[tokio::test]` — `RuntimeDataProviderAccess::fetch_scoped(authoritative, provider_id, request)` runs reconciliation then the (unchanged #234) retry/timeout loop; with a tenant-recording provider double (shape of `TenantRecordingProvider`, `data_provider_access.rs:284-331`): authority `A` + request `None` ⇒ provider records exactly `A`; authority `A` + request `A` ⇒ provider records `A`. AC: fails because `fetch_scoped` does not yet exist.
- [ ] TASK-013 GREEN: implement `fetch_scoped` — call `reconcile_tenant(authoritative, &mut request)`, and on `Ok` continue into the existing registry lookup + retry/timeout loop with the authorized request. AC: TASK-012 green; the #234 loop body is unchanged.
- [ ] TASK-014 RED: add a failing `#[tokio::test]` (observable fail-closed) asserting, for BOTH a mismatch (authority `A`, request `Some(B)`) and a context-missing (authority `None`, request `Some(C)`) case: `fetch_scoped` emits EXACTLY ONE terminal `data_provider_fetch` outcome classified as the matching non-retryable tenant outcome with `attempts == 1`, schedules ZERO retries, and the tenant-recording provider records ZERO invocations. AC: fails until the observable fail-closed path is wired.
- [ ] TASK-015 GREEN: implement the fail-closed path in `fetch_scoped` — on a reconciliation `Err`, call the shared `log_fetch(provider_id, &request.key, elapsed, false, ProviderOutcome::from_tenant_error(&e), 1)` terminal emitter once, then return the error without entering the retry loop or calling any provider. AC: TASK-014 green; exactly one terminal signal, zero retries, zero provider calls.

## Phase 5: Per-Dispatch Wrapper (ADR-3 Freeze)

- [ ] TASK-016 RED: add a failing `#[tokio::test]` — `TenantScopedDataProviderAccess { inner: Arc<RuntimeDataProviderAccess>, authoritative_tenant }` implements `DataProviderAccess`; a wrapper capturing `A` over a shared `inner` delegates so the provider records `A` when the caller passes `None`; AND two wrappers capturing `tenant-a` and `tenant-b` respectively over the SAME shared `inner` `Arc`, driven concurrently with request tenant `None`, each cause their own tenant to be recorded (no cross-contamination — the concurrency guarantee ADR-3 freezes). AC: fails because the wrapper type does not exist.
- [ ] TASK-017 GREEN: implement `TenantScopedDataProviderAccess`, whose `fetch` delegates to `self.inner.fetch_scoped(self.authoritative_tenant.as_ref(), provider_id, request)`; confirm `RuntimeDataProviderAccess` holds no tenant field (a mutable/captured singleton field is rejected by ADR-3). AC: TASK-016 green.

## Phase 6: Real Dispatch Wiring + End-to-End Binding

- [ ] TASK-018 RED: add a failing `#[tokio::test]` (end-to-end, ADR-3 real-wiring proof) — drive a real actor→handler dispatch established for `tenant-a`; the handler issues a `fetch` with `request.tenant == None`; assert the provider receives `tenant-a`. This must go through real dispatch, not a hand-built wrapper. AC: fails until dispatch constructs and hands a per-dispatch wrapper.
- [ ] TASK-019 GREEN: wire the actor→handler dispatch (`crates/persistent-entity/src/actor.rs` / runtime builder) to construct, per dispatch, a `TenantScopedDataProviderAccess` capturing `entity_id.tenant_id` / the resolved `canonical_tenant` and hand THAT to the handler as `Arc<dyn DataProviderAccess>`. AC: TASK-018 green; the handler cannot read or overwrite the captured tenant.

## Phase 7: Cross-Tenant Negative + Compatibility

- [ ] TASK-020 RED: add a failing `#[tokio::test]` (CROSS-TENANT negative) — established authority `tenant-a`; a caller forges `tenant-b` via `DataRequest::with_tenant(TenantId::new("tenant-b"))`; assert the fetch fails closed with `TenantMismatch`, the recording provider records ZERO invocations (never receives `tenant-b`), and no response scoped to `tenant-b` is returned. This is the dedicated cross-tenant read-forgery test (design Threat Matrix "Cross-tenant read"). AC: fails until enforcement is complete.
- [ ] TASK-021 GREEN: confirm the cross-tenant negative passes under the completed `reconcile_tenant`/`fetch_scoped`/wrapper wiring (no new code beyond Phases 3-6 expected; if it fails, close the remaining gap so a forged different tenant can never reach the provider). AC: TASK-020 green.
- [ ] TASK-022 COMPAT (characterization): add a `#[tokio::test]` — captured authority `None` (tenant-agnostic wrapper), `DataRequest::new` (tenant `None`) ⇒ provider is invoked with tenant `None`, exactly as before; assert unchanged pass-through and that no fail-closed outcome is emitted. AC: passes against the completed no-op path.

## Phase 8: Cross-Cutting Guarantees & Verification

- [ ] TASK-023: confirm #234 behavior is untouched — run the existing provider timeout/retry/observability tests (`cargo test -p ego-runtime providers::access::`) unmodified. AC: all pre-existing #234 tests pass; the retry/timeout loop is entered only on the authorized (success) path.
- [ ] TASK-024: grep-verify no raw tenant id is added as a metric/log label at the chokepoint and that `TenantMismatch`/`TenantContextMissing` carry no tenant field (bounded-cardinality + redaction rules). AC: grep clean; neither error variant has a tenant field.
- [ ] TASK-025: run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo build --workspace`. AC: exit 0, no regressions.
