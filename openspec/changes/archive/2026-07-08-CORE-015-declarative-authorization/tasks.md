# Tasks: CORE-015 Declarative Authorization & Service Security Integration

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | 500–610 |
| 400-line budget risk | High |
| Chained PRs recommended | Yes |
| Delivery strategy | chained-prs |
| Chain strategy | stacked-to-main |

Decision needed before apply: **resolved**
Chained PRs: **3 PRs / stacked-to-main**

### PR Boundaries (resolved)

| PR | Scope | Tasks | Base | Est. LOC |
|----|-------|-------|------|----------|
| **PR 1 — Foundation** | `RuntimeInner` accessor + `AuthorizeArgs` parser/validator + parser unit tests. No `#[service]` integration. | T-01–T-05 | `develop` | ~150–200 |
| **PR 2 — Codegen** | `#[service]` integration: detect/strip `#[authorize]`, inject guard, standalone rejection, compile-pass smoke test. | T-06–T-09 | PR 1 | ~180–220 |
| **PR 3 — Tests + cleanup** | All trybuild compile-fail fixtures + integration tests. | T-10–T-24 | PR 2 | ~150 |

---

## Phase 1 — Foundation: RuntimeInner Accessor

### T-01
**Title**: Add `authorization_provider()` accessor to `RuntimeInner`
**File**: `crates/service-sdk/src/runtime/runtime_builder.rs`
**Description**: Add `pub fn authorization_provider(&self) -> Option<Arc<dyn AuthorizationProvider>>` to the `RuntimeInner` impl block. Implementation: `self.security_providers.as_ref().map(|(_, authz)| Arc::clone(authz))`. No other fields or visibility changes.
**Acceptance criteria**:
- AC-10.1: Method signature matches `pub fn authorization_provider(&self) -> Option<Arc<dyn AuthorizationProvider>>`.
- AC-10.2: Returns `None` when `security_providers` is `None`.
- AC-10.3: Returns `Some(Arc<dyn AuthorizationProvider>)` (owned clone) when providers are set.
- AC-10.4: `authentication` provider remains inaccessible; only the authz `Arc` is exposed.
- `cargo test --workspace` passes.
**Dependencies**: none
**Test task**: no (test task is T-02)

---

### T-02 (RED)
**Title**: Unit tests for `authorization_provider()` accessor
**File**: `crates/service-sdk/src/runtime/runtime_builder.rs` — `#[cfg(test)] mod tests` block
**Description**: In the existing `tests` module, add two unit tests: one asserting `authorization_provider()` returns `None` on `RuntimeInner::default()`, and one asserting it returns `Some(Arc<...>)` when a `RuntimeInner` is constructed with `security_providers = Some((authn_stub, authz_stub))`.
**Acceptance criteria**:
- Test `authorization_provider_returns_none_when_no_providers` passes.
- Test `authorization_provider_returns_arc_when_providers_set` passes and the returned `Arc` pointer equals the one passed in.
- `cargo test --workspace` passes.
**Dependencies**: T-01
**Test task**: yes

---

## Phase 2 — Foundation: `AuthorizeArgs` Parser & Validator

### T-03 (RED)
**Title**: Define `AuthorizeArgs` struct and its unit test module in `service-sdk-macros`
**File**: `crates/service-sdk-macros/src/lib.rs` (or a new `authorize.rs` module imported from `lib.rs`)
**Description**: Introduce `struct AuthorizeArgs { context_ident: syn::Ident, resource: String, action: String, permission_span: proc_macro2::Span }`. Add a minimal skeleton `fn parse_authorize_args` that always returns `Err(syn::Error::new(Span::call_site(), "not implemented"))`. Add a `#[cfg(test)]` module `authorize_args_tests` with real test assertions for all cases in T-04 — tests will be RED because the skeleton always errors. No `todo!()`, no `#[ignore]`.
**Acceptance criteria**:
- Struct compiles.
- Test module exists with named tests containing real assertions.
- `cargo test --workspace` compiles; the new tests fail (RED) — that is the expected outcome for this task.
**Dependencies**: none
**Test task**: yes

---

### T-04 (GREEN)
**Title**: Implement `parse_authorize_args` function with full validation
**File**: `crates/service-sdk-macros/src/lib.rs` (or `authorize.rs`)
**Description**: Implement `fn parse_authorize_args(meta: &syn::meta::ParseNestedMeta) -> syn::Result<AuthorizeArgs>` using `syn::meta::parser`. Rules: accept exactly `context = <ident>` (non-ident → AD-4 non-ident error) and `permission = <str-lit>` (non-literal → AD-4 non-literal error); unknown key → E4; missing either key after full parse → E4b. After parsing the permission literal, split on `:`: zero colons → E1; more than one colon → E1b; empty resource → E2; empty action → E3. All errors must use `syn::Error::new_spanned` at the offending token.
**Acceptance criteria**:
- All test stubs from T-03 pass (valid input, E1, E1b, E2, E3, E4, E4b, AD-4 non-literal, AD-4 non-ident).
- Error messages match exactly the diagnostics contract in the spec.
- `cargo test --workspace` passes.
**Dependencies**: T-03
**Test task**: no (test task is T-03)

---

### T-05
**Title**: Implement `validate_context_ident_in_signature` helper
**File**: `crates/service-sdk-macros/src/lib.rs` (or `authorize.rs`)
**Description**: Add `fn validate_context_ident_in_signature(ident: &syn::Ident, sig: &syn::Signature) -> syn::Result<()>`. Iterates `sig.inputs` and checks that at least one `FnArg::Typed` has a `Pat::Ident` matching `ident`; if not, emits E6 error spanned at `ident`.
**Acceptance criteria**:
- Returns `Ok(())` when `ident` names a typed parameter in `sig`.
- Returns `Err(syn::Error)` with message `#[authorize] context parameter 'X' not found in method signature` when not found.
- Unit test covers both paths in T-03's test module.
- `cargo test --workspace` passes.
**Dependencies**: T-03
**Test task**: no (covered by T-03 test module)

---

## Phase 3 — Codegen: `#[service]` Extension

### T-06 (RED)
**Title**: Add trybuild compile-pass smoke test `authorize_ok.rs`
**File**: `crates/service-sdk-macros/tests/authorize_ok.rs` (new); `crates/service-sdk-macros/src/tests.rs` (register)
**Description**: Write `authorize_ok.rs` with a minimal `#[service]` trait containing one `#[operation] #[authorize(context = ctx, permission = "orders:read")]` method whose error type implements `From<SecurityError>`. Register in `tests.rs` as a trybuild compile-pass case. This test will fail (RED) until T-07 and T-08 are complete.
**Acceptance criteria**:
- File exists and registers in `tests.rs`.
- After T-08 merges, `cargo test --workspace` passes this case.
**Dependencies**: T-03, T-04, T-05
**Test task**: yes

---

### T-07
**Title**: Detect and strip `#[authorize]` in `expand_service_trait` loop
**File**: `crates/service-sdk-macros/src/lib.rs`
**Description**: In the `expand_service_trait` for-loop body (at the `has_operation` branch), add detection of `#[authorize(...)]` on the method: parse its arguments using `parse_authorize_args`, validate the `context` ident using `validate_context_ident_in_signature`, and strip `#[authorize]` from the `clean` output attrs (mirrors how `#[operation]` is stripped at line 154). Store the parsed `AuthorizeArgs` for use in T-08. Methods without `#[authorize]` continue unchanged.
**Acceptance criteria**:
- `#[authorize]` is absent from the emitted trait item (clean output).
- Parser errors (E1–E6, AD-4) propagate as `compile_error!` at the correct span.
- Methods without `#[authorize]` are unaffected.
- `cargo test --workspace` passes (existing tests unbroken).
**Dependencies**: T-04, T-05
**Test task**: no (tested by T-06 and T-09 fixtures)

---

### T-08
**Title**: Inject `__assert_from_security_error` bound and authorization guard into generated proxy body
**File**: `crates/service-sdk-macros/src/lib.rs`
**Description**: When a method has an `#[authorize]` annotation (parsed `AuthorizeArgs` present from T-07), prepend two code blocks to the `forwarding_methods` body before the existing `enforce_tenant` call: (1) `fn __assert_from_security_error<E: From<ego_security_sdk::SecurityError>>() {}` followed by `__assert_from_security_error::<ErrorType>();` where `ErrorType` is extracted from `Result<_, ErrorType>` in the return type; (2) the fail-closed guard block per the design's generated code shape (AD-9): check `ctx.security().is_some()`, upgrade runtime weak-ref or return `ProviderError`, get `authorization_provider()` or return `CapabilityNotEnabled`, call `authorize_in_context(...).await.map_err(...)?`. Execution order: authorize block at slot 1, `enforce_tenant` at slot 3.
**Acceptance criteria**:
- AC-4.3: authorize guard appears before `enforce_tenant` in generated body.
- AC-4.2: exactly one `authorize_in_context` call per annotated method.
- AC-5.1–5.5: all five security-state behaviors produce the correct outcomes.
- AC-6.1: method with error type lacking `From<SecurityError>` fails compilation.
- AC-8.2: guard always in slot 1 regardless of attribute lexical order.
- `cargo test --workspace` passes; `authorize_ok.rs` trybuild case now passes.
**Dependencies**: T-07
**Test task**: no (tested by T-06, compile-fail fixtures T-10 through T-17, and integration tests T-18–T-23)

---

### T-09
**Title**: Register standalone `#[authorize]` proc-macro that emits `compile_error!` (E5)
**File**: `crates/service-sdk-macros/src/lib.rs`
**Description**: Add `#[proc_macro_attribute] pub fn authorize(_args: TokenStream, input: TokenStream) -> TokenStream` that emits `compile_error!("#[authorize] can only be used on methods inside a #[service] trait")` followed by `#item`. The existing `#[service]` expansion strips `#[authorize]` before this standalone macro fires, so it only activates on genuinely standalone usage.
**Acceptance criteria**:
- AC-7.1: `#[authorize]` on a free function fails with E5.
- AC-7.2: `#[authorize]` on a method in a plain `impl` block fails with E5.
- AC-7.3: `#[authorize]` inside `#[service]` does NOT trigger E5.
- `cargo test --workspace` passes.
**Dependencies**: T-07 (strip must happen before standalone fires)
**Test task**: no (tested by T-15 fixture)

---

## Phase 4 — trybuild Compile-Fail Fixtures

Each fixture task is: write the `.rs` source file + paired `.stderr` snapshot; register the case in `tests.rs`. All fixtures live in `crates/service-sdk-macros/tests/`.

### T-10 (RED/GREEN)
**Title**: trybuild fixture `authorize_bad_format.rs` — E1 (permission missing `:`)
**Files**: `crates/service-sdk-macros/tests/authorize_bad_format.rs`, `crates/service-sdk-macros/tests/authorize_bad_format.stderr`, `crates/service-sdk-macros/src/tests.rs`
**Description**: Write a `#[service]` trait method with `#[authorize(context = ctx, permission = "ordersread")]`. Capture the expected stderr from `cargo test` (trybuild will generate it on first run). Register in `tests.rs`.
**Acceptance criteria**: AC-3.1 — `cargo test --workspace` passes the compile-fail case. `.stderr` snapshot matches E1 message exactly.
**Dependencies**: T-07, T-08
**Test task**: yes

---

### T-11 (RED/GREEN)
**Title**: trybuild fixture `authorize_empty_resource.rs` — E2 (empty resource `":read"`)
**Files**: `crates/service-sdk-macros/tests/authorize_empty_resource.rs`, `.stderr`, `tests.rs`
**Description**: Method with `permission = ":read"`. Register and capture stderr.
**Acceptance criteria**: AC-3.3 — compile-fail case passes with E2 message.
**Dependencies**: T-07, T-08
**Test task**: yes

---

### T-12 (RED/GREEN)
**Title**: trybuild fixture `authorize_empty_action.rs` — E3 (empty action `"orders:"`)
**Files**: `crates/service-sdk-macros/tests/authorize_empty_action.rs`, `.stderr`, `tests.rs`
**Description**: Method with `permission = "orders:"`. Register and capture stderr.
**Acceptance criteria**: AC-3.4 — compile-fail case passes with E3 message.
**Dependencies**: T-07, T-08
**Test task**: yes

---

### T-13 (RED/GREEN)
**Title**: trybuild fixture `authorize_missing_from.rs` — E_from (error type without `From<SecurityError>`)
**Files**: `crates/service-sdk-macros/tests/authorize_missing_from.rs`, `.stderr`, `tests.rs`
**Description**: Method returning `Result<_, MyError>` where `MyError` does not impl `From<SecurityError>`. Register and capture stderr.
**Acceptance criteria**: AC-6.1 — compile-fail case passes with E_from message.
**Dependencies**: T-08
**Test task**: yes

---

### T-14 (RED/GREEN)
**Title**: trybuild fixture `authorize_unknown_ctx.rs` — E6 (context ident not in signature)
**Files**: `crates/service-sdk-macros/tests/authorize_unknown_ctx.rs`, `.stderr`, `tests.rs`
**Description**: Method with `context = wrong` where `wrong` is not a parameter name. Register and capture stderr.
**Acceptance criteria**: AC-1.2 — compile-fail case passes with E6 message.
**Dependencies**: T-05, T-07
**Test task**: yes

---

### T-15 (RED/GREEN)
**Title**: trybuild fixture `authorize_outside_service.rs` — E5 (`#[authorize]` on standalone fn)
**Files**: `crates/service-sdk-macros/tests/authorize_outside_service.rs`, `.stderr`, `tests.rs`
**Description**: Free function (outside `#[service]`) annotated with `#[authorize(...)]`. Register and capture stderr.
**Acceptance criteria**: AC-7.1 — compile-fail case passes with E5 message.
**Dependencies**: T-09
**Test task**: yes

---

### T-16 (RED/GREEN)
**Title**: trybuild fixture `authorize_unknown_arg.rs` — E4 (unknown named key `perm`)
**Files**: `crates/service-sdk-macros/tests/authorize_unknown_arg.rs`, `.stderr`, `tests.rs`
**Description**: Method with `#[authorize(context = ctx, perm = "orders:read")]`. Register and capture stderr.
**Acceptance criteria**: AC-2.2 — compile-fail case passes with E4 message.
**Dependencies**: T-04, T-07
**Test task**: yes

---

### T-17 (RED/GREEN)
**Title**: trybuild fixture `authorize_non_literal.rs` — AD-4 non-literal (`permission = SOME_CONST`)
**Files**: `crates/service-sdk-macros/tests/authorize_non_literal.rs`, `.stderr`, `tests.rs`
**Description**: Method with `permission = SOME_CONST` (a path, not a string literal). Register and capture stderr.
**Acceptance criteria**: AC-3.5 — compile-fail case passes with AD-4 non-literal message.
**Dependencies**: T-04, T-07
**Test task**: yes

---

## Phase 5 — Integration Tests

All integration tests live in `crates/service-sdk/tests/` or an appropriate integration test file using a stub `AuthorizationProvider`. They use the established project test harness (no external resources).

### T-18 (RED)
**Title**: Integration test stubs — define stub `AuthorizationProvider` (Allow/Deny) and shared test fixture
**File**: `crates/service-sdk/tests/authorization_integration.rs` (new)
**Description**: Create the test file with: (a) `struct AllowProvider` and `struct DenyProvider` implementing `AuthorizationProvider`; (b) a helper that constructs a `RuntimeInner` with security providers set; (c) empty test function bodies for T-19 through T-23. These will fail (RED) until T-19–T-23 are completed.
**Acceptance criteria**: File compiles; stubs compile; `cargo test --workspace` does not error on this file.
**Dependencies**: T-01, T-08
**Test task**: yes

---

### T-19 (GREEN)
**Title**: Integration test — allow path: body executes when provider grants
**File**: `crates/service-sdk/tests/authorization_integration.rs`
**Description**: Construct a `#[service]`-generated proxy with `AllowProvider`, a `ServiceContext` with `SecurityContext`, call the `#[authorize]`-annotated method. Assert the method body executes (via a side-effect counter or return value) and no error is returned.
**Acceptance criteria**: AC-5.5, AC-4.1 (body runs on allow). Test passes.
**Dependencies**: T-18
**Test task**: yes

---

### T-20 (GREEN)
**Title**: Integration test — deny path: body does not execute when provider denies
**File**: `crates/service-sdk/tests/authorization_integration.rs`
**Description**: Use `DenyProvider`. Assert that the method returns `Err(AuthorizationDenied { .. })` and the body side-effect is absent (counter not incremented, or equivalent).
**Acceptance criteria**: AC-4.1, AC-5.4. Test passes.
**Dependencies**: T-18
**Test task**: yes

---

### T-21 (GREEN)
**Title**: Integration test — security disabled: body executes without authorization call
**File**: `crates/service-sdk/tests/authorization_integration.rs`
**Description**: Construct `RuntimeInner` with `security_providers = None` and a `ServiceContext` whose `security()` returns `None`. Call the annotated method. Assert body executes normally, no error.
**Acceptance criteria**: AC-5.1. Test passes.
**Dependencies**: T-18
**Test task**: yes

---

### T-22 (GREEN)
**Title**: Integration test — fail-closed: runtime dropped returns `ProviderError`
**File**: `crates/service-sdk/tests/authorization_integration.rs`
**Description**: Construct a proxy with a `Weak<RuntimeInner>` that has been dropped (all `Arc` clones dropped). Call the annotated method with a `ServiceContext` whose `security()` returns `Some`. Assert `Err(SecurityError::ProviderError(...))` is returned and body does not execute.
**Acceptance criteria**: AC-5.2. Test passes.
**Dependencies**: T-18
**Test task**: yes

---

### T-23 (GREEN)
**Title**: Integration test — fail-closed: provider returns `CapabilityNotEnabled`
**File**: `crates/service-sdk/tests/authorization_integration.rs`
**Description**: Define `struct CapabilityNotEnabledProvider` implementing `AuthorizationProvider` that always returns `Err(SecurityError::CapabilityNotEnabled)`. Use this provider in a proxy configured with a live runtime and a `ServiceContext` whose `security()` returns `Some`. Call the annotated method. Assert `Err(SecurityError::CapabilityNotEnabled)` and body does not execute. **Rationale**: `RuntimeInner.security_providers` is a single `Option<(authn, authz)>` tuple — there is no architectural path to have a live runtime with authn present but authz absent. The stub provider validates the error-mapping contract through `authorize_in_context` without requiring internal visibility into `RuntimeInner`.
**Acceptance criteria**: AC-5.3. Test passes without exposing `RuntimeInner` internals.
**Dependencies**: T-18
**Test task**: yes

---

### T-24 (GREEN)
**Title**: Integration test — exactly one `authorize_in_context` call per annotated method invocation
**File**: `crates/service-sdk/tests/authorization_integration.rs`
**Description**: Use an `AuthorizationProvider` stub that increments a counter on each `authorize` call. Invoke the `#[authorize]`-annotated method once. Assert counter equals 1.
**Acceptance criteria**: AC-4.2. Test passes.
**Dependencies**: T-18
**Test task**: yes

---

## Summary

| Phase | Tasks | Focus |
|-------|-------|-------|
| Phase 1 | T-01, T-02 | `RuntimeInner` accessor + unit tests |
| Phase 2 | T-03, T-04, T-05 | `AuthorizeArgs` parser/validator + unit tests |
| Phase 3 | T-06, T-07, T-08, T-09 | `#[service]` codegen extension + standalone rejection |
| Phase 4 | T-10 – T-17 | trybuild compile-fail/compile-pass fixtures |
| Phase 5 | T-18 – T-24 | Integration tests |
| **Total** | **24** | |

### Estimated changed lines by crate

| Crate | Est. lines |
|-------|-----------|
| `service-sdk` (`runtime_builder.rs`) | ~30 |
| `service-sdk-macros` (`lib.rs`, `tests.rs`) | ~200–250 |
| `service-sdk-macros/tests/` (9 fixture files + 8 `.stderr`) | ~150–180 |
| `service-sdk/tests/` (integration tests) | ~120–150 |
| **Total** | **~500–610** |

### Review Workload Forecast

- Estimated changed lines total: 500–610
- Lines by crate: service-sdk-macros: ~250, service-sdk: ~170, test fixtures: ~170
- Chained PRs recommended: Yes
- 400-line budget risk: High
- Decision needed before apply: Yes
