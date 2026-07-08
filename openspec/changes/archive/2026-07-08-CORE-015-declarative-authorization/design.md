# Design: CORE-015 Declarative Authorization & Service Security Integration

## Technical Approach

`#[authorize]` is a **marker attribute consumed by `#[service]`** (proposal AD-1, Option C). The `expand_service_trait` loop in `crates/service-sdk-macros/src/lib.rs` already iterates trait methods, detects `#[operation]`, and builds the forwarding proxy method body. CORE-015 extends that same loop to also detect and consume `#[authorize(...)]`, parse its arguments at expansion time, validate the permission literal structurally, and inject an authorization guard as the FIRST step of the generated proxy body — reusing the existing `self.runtime.upgrade()` pattern and the stable `authorize_in_context(...)` seam from CORE-014. `ServiceContext` is untouched (pure DTO). `RuntimeInner` gains one public accessor so generated code (which lives in the consumer crate, not in `service-sdk`) can reach the authorization provider.

## Architecture Decisions

### Decision AD-6: Named arguments over positional

**Choice**: `#[authorize(context = ctx, permission = "orders:read")]`.
**Alternatives considered**: positional `#[authorize(ctx, "orders:read")]` (current proposal examples).
**Rationale**: Named arguments match the existing `#[service(version = "1.0.0")]` convention, are self-documenting at the callsite, and let future optional args (`audit = true`, `provider = "secondary"`) be added without breaking existing callsites or reordering. Positional form couples meaning to position and breaks the moment a third argument is needed.

**Parser structure** (expansion-time):

```rust
struct AuthorizeArgs {
    context_ident: syn::Ident,   // from `context = ctx`
    resource: String,            // parsed from permission literal, before ':'
    action: String,              // parsed from permission literal, after ':'
    permission_span: proc_macro2::Span, // span of the literal, for diagnostics
}
```

Parse via `syn::meta::parser`: accept exactly `context = <ident>` and `permission = <str-lit>`. `context` must be an `Ident` (not an expression — AD-4). `permission` must be a string literal (not a const/path/macro — AD-4). Any other key → error E4. Both keys are required.

### Decision AD-7: `RuntimeInner` provider accessor (GAP-01 — accessor contract)

**Choice**: Add `pub fn authorization_provider(&self) -> Option<Arc<dyn AuthorizationProvider>>` to `RuntimeInner`.
**Alternatives considered**: returning `Option<&Arc<...>>` (borrow tied to `rt` guard lifetime); exposing the whole `security_providers` tuple.
**Rationale**: Generated code does `self.runtime.upgrade()` — `rt` is a temporary `Arc<RuntimeInner>` owned in that scope. Returning an owned `Arc` clone avoids fighting the borrow checker across the `.await` in `authorize_in_context`, and keeps the authentication provider private. Cheap (one `Arc::clone`). It maps `self.security_providers.as_ref().map(|(_, az)| Arc::clone(az))`.

**Accessor contract invariant** (GAP-01): `RuntimeInner` public accessors exist ONLY to serve generated macro code. They are NOT a dependency-resolution API and MUST NOT be called by hand-written application code.

- `RuntimeInner` already exposes `enforce_tenant`, `resolve_projection`, `resolve_adapter`, `resolve_config` for generated `Injectable` structs and `#[service]` proxies. `authorization_provider()` follows the same pattern.
- Future provider accessors (authentication, rate-limiting, etc.) each REQUIRE an explicit ADR before being added.
- No additional public accessor is ever added to `RuntimeInner` without an ADR, regardless of convenience.
- Application code accessing `RuntimeInner` directly is an architectural violation.

### Decision AD-8: Standalone `#[authorize]` rejection

**Choice**: Keep `#[authorize]` registered as a `#[proc_macro_attribute]` whose body ONLY emits a `compile_error!`. When used inside `#[service]`, the marker is stripped before the standalone macro ever runs, so the error never fires for valid usage; when used on a free fn, the standalone macro runs and emits error E5.
**Alternatives considered**: not registering `#[authorize]` at all (would yield a cryptic "cannot find attribute" error); inert-attribute registration (not stable on stable Rust here).
**Rationale**: Mirrors the existing `#[operation]` passthrough pattern (lines 391–395) but inverts intent — `#[operation]` passes through, `#[authorize]` errors. `#[service]` must strip `#[authorize]` from the clean output exactly as it strips `#[operation]`, otherwise the standalone macro fires on valid code.

```rust
#[proc_macro_attribute]
pub fn authorize(_args: TokenStream, input: TokenStream) -> TokenStream {
    let item: proc_macro2::TokenStream = input.into();
    quote! {
        compile_error!("#[authorize] can only be used on methods inside a #[service] trait");
        #item
    }.into()
}
```

### Decision AD-9: Authorization guards fail closed when security is enabled

**Choice**: Three-case policy distinguishing disabled security from misconfigured security.

| State | Guard behavior | Error variant |
|-------|---------------|--------------|
| Security capability disabled (`ctx.security()` is `None`) | No guard — skip entirely, consistent with CORE-009D optional capability model | — |
| Security enabled + provider present | Evaluate `authorize_in_context` normally | `AuthorizationDenied` on deny, `ProviderError` on backend failure |
| Security enabled — `Weak<RuntimeInner>::upgrade()` failed (runtime dropped) | **Fail closed** | `ProviderError("authorization provider unavailable: runtime dropped")` |
| Security enabled — runtime alive but `authorization_provider()` returns `None` | **Fail closed** | `CapabilityNotEnabled` |

**Error variant rationale** (GAP-02): The two fail-closed cases produce different error variants intentionally:
- `ProviderError` for a dropped runtime — this is a lifecycle/infrastructure failure; the application is in a bad state.
- `CapabilityNotEnabled` for a live runtime with no provider — this is a configuration omission; authorization was declared at the callsite but the capability was never wired up. Using a distinct machine-readable variant (not a free-form string) lets callers match on it, and aligns with how `authorize_in_context` itself uses `CapabilityNotEnabled` when the `SecurityContext` is absent.

**Stable error mapping for generated code**:

| Condition | `SecurityError` variant | Machine-matchable |
|-----------|------------------------|------------------|
| `ctx.security()` is `None` (security disabled) | Guard not emitted | n/a |
| `self.runtime.upgrade()` returns `None` | `ProviderError` (string payload) | Yes (variant) |
| `rt.authorization_provider()` returns `None` | `CapabilityNotEnabled` | Yes (unit variant) |
| Provider returns `Err(SecurityError)` | propagated as-is | Yes |
| Provider panics | `ProviderError` ("authorization provider panicked") — handled inside `authorize_in_context` via `catch_unwind` (CORE-014 / issue #95); the generated proxy itself never catches panics | Yes (variant) |
| Provider returns `Deny` | `AuthorizationDenied { reason }` | Yes |

Adding new `SecurityError` variants is out of scope for CORE-015. These two variants (`CapabilityNotEnabled`, `ProviderError`) are the stable surface for CORE-015's error contract.

**Rationale**: `#[authorize]` expresses an explicit developer intent — "this operation requires authorization." If the framework silently skips authorization because a provider is missing, it substitutes its own policy ("allow") for the developer's ("must authorize"). That is a configuration error, not a valid execution path. The `enforce_tenant` no-op pattern is NOT replicated here: `enforce_tenant` protects a runtime invariant, `#[authorize]` protects a declared operation contract — different semantic level, different failure policy.

The only legitimate pass-through is when the Security capability was intentionally disabled at build time (CORE-009D). The discriminator is `ctx.security().is_some()`: if a `SecurityContext` was attached to the request, the runtime is operating in security mode and the provider MUST be present.

**Invariant**: When `ctx.security().is_some()` and `#[authorize]` is present on the method, a missing or unavailable `AuthorizationProvider` MUST return an error (see stable error mapping above), never silently continue.

### Decision AD-10: Generated proxy internals are not public API (GAP-05)

**Choice**: All variable names, helper functions, and structural details emitted by `#[service]`/`#[authorize]` codegen are **implementation details** — not public API and not part of any stability contract.

Specifically, the following are internal-only and subject to change without notice:

| Generated name | Role |
|---------------|------|
| `__rt` | Temporary `Arc<RuntimeInner>` local in the proxy body |
| `__provider` | Temporary `Arc<dyn AuthorizationProvider>` local in the proxy body |
| `__assert_from_security_error` | Zero-size helper fn enforcing the `From<SecurityError>` bound at compile time |
| Expansion layout (order of `let` bindings, block structure) | Internal ordering within the generated proxy method body |

**Rationale**: Rust proc-macro output is visible in `cargo expand` output and in trybuild `.stderr` snapshots, which can create a false impression that naming is stable. By documenting instability explicitly, we prevent downstream code from pattern-matching on generated internals (e.g., in integration tests or `unsafe` code), and we reserve freedom to optimize or rename without a semver bump.

**Rules**:
- These names MUST NOT appear in hand-written application code. Using them is undefined behavior in the versioning sense — any refactor may break it silently.
- **`cargo expand` output is not a compatibility contract.** It is a debugging aid. Observing that `cargo expand` shows `__rt` today does not mean `__rt` will exist tomorrow.
- `.stderr` snapshots in trybuild tests MUST be regenerated when expansion layout changes. They are test fixtures, not contracts.
- If a generated helper needs to be stable for cross-crate access, it MUST be promoted to a named public function in `service-sdk` or `security-sdk` instead, with an explicit ADR.

### Decision AD-11: Generated authorization metadata reuses the existing `Resource`/`Action` API (GAP-03)

**Choice**: Generated code constructs `Resource { kind: "orders".to_string(), .. }` and `Action("read".to_string())` — two `String` allocations per call — rather than introducing parallel borrowed types (`&'static str`, `Cow<'static, str>`).

**Rationale**:
- `Resource.kind: String` and `Action(String)` are the current public API of `security-sdk`. They are owned by `authorize_in_context`'s contract, not by this macro.
- Changing ownership to `&'static str` or `Cow` would be a breaking change to `security-sdk` — wrong scope for a macro change. The macro MUST reuse the stable runtime contract instead of introducing a parallel representation that diverges from it.
- The two allocations are accepted because they preserve the stable `security-sdk` API. Performance optimization belongs in `security-sdk`, not in the macro expansion.

**Deferred**: Allocation-free authorization metadata (e.g., `Resource<'static>` or a `StaticPermission` type) is a future `security-sdk` API concern and requires its own ADR when the time comes. CORE-015 does not pre-design that shape.

**Invariant**: The macro MUST NOT introduce types or representations that bypass or duplicate the `security-sdk` `Resource`/`Action` contract, even if it would be more efficient.

## Marker Execution Order

The pipeline order is **fixed** (not user-configurable) and **independent of lexical attribute order** (proposal AD-5). Each slot has fixed semantics; reordering would break correctness (e.g. authorizing AFTER the body defeats the guard). `#[service]` applies markers by this pipeline, never by source order.

```
1. authorize              (CORE-015)            ← deny before any side effect
2. [future pre-body marker]                     ← slot reserved; ADR required to claim it
3. enforce_tenant         (existing, non-marker)
4. chain.on_request       (existing interceptor)
5. inner.method(args)     (service body)
6. chain.on_response / on_error (existing interceptor)
7. [future post-body marker]                    ← slot reserved; ADR required to claim it
8. return result
```

**Invariant**: any future marker MUST declare its fixed slot here. A marker that needs a *relative* ordering with another marker is an architectural smell and is rejected at expansion time. This contract governs ALL present and future `#[service]` markers.

## Generated Code Shape

Input:

```rust
#[service]
trait OrderService {
    #[operation]
    #[authorize(context = ctx, permission = "orders:read")]
    async fn get_order(&self, ctx: ServiceContext, id: OrderId) -> Result<Order, OrderError>;
}
```

At expansion: `"orders:read"` → resource `"orders"`, action `"read"`. `#[service]` validates `ctx` exists as a parameter ident in the signature (else E6), strips both `#[operation]` and `#[authorize]` from the clean output, and emits the proxy body with the authorize guard prepended.

Generated proxy method (full `quote!` body):

```rust
async fn get_order(&self, ctx: ServiceContext, id: OrderId) -> Result<Order, OrderError> {
    // 1. authorize (CORE-015) — AD-9: fail closed when security is enabled
    {
        // compile-time assertion: OrderError must implement From<SecurityError>
        fn __assert_from_security_error<E: ::core::convert::From<
            ::ego_security_sdk::SecurityError>>() {}
        __assert_from_security_error::<OrderError>();

        if ctx.security().is_some() {
            // Security capability is enabled — provider MUST be present (AD-9).
            let __rt = self.runtime.upgrade().ok_or_else(|| {
                <OrderError as ::core::convert::From<::ego_security_sdk::SecurityError>>::from(
                    ::ego_security_sdk::SecurityError::ProviderError(
                        "authorization provider unavailable: runtime dropped".into(),
                    ),
                )
            })?;
            let __provider = __rt.authorization_provider().ok_or_else(|| {
                <OrderError as ::core::convert::From<::ego_security_sdk::SecurityError>>::from(
                    ::ego_security_sdk::SecurityError::CapabilityNotEnabled,
                )
            })?;
            ::ego_security_sdk::authorization::authorize_in_context(
                ctx.security(),
                ::ego_security_sdk::authorization::Resource {
                    kind: "orders".to_string(),
                    id: None,
                },
                ::ego_security_sdk::authorization::Action("read".to_string()),
                __provider.as_ref(),
            )
            .await
            .map_err(<OrderError as ::core::convert::From<
                ::ego_security_sdk::SecurityError>>::from)?;
        }
        // else: Security capability disabled (CORE-009D) — no guard emitted.
    }
    // 2. enforce_tenant (existing)
    if let Some(rt) = self.runtime.upgrade() {
        rt.enforce_tenant(&ctx);
    }
    // 3. on_request + 4. inner call + 5. on_response/on_error (existing)
    let inner_ref = self.inner.clone();
    let chain_ref = self.chain.clone();
    let _ = chain_ref.on_request(&ctx).await;
    let result = inner_ref.get_order(ctx.clone(), id).await;
    match &result {
        Ok(_)  => { chain_ref.on_response(&ctx).await.ok(); }
        Err(e) => { chain_ref.on_error(&ctx, e as &dyn ServiceErrorTrait).await.ok(); }
    }
    result
}
```

Notes:
- The `__assert_from_security_error::<OrderError>()` monomorphization forces the `From<SecurityError>` bound at compile time with a span-targeted error (E_from below).
- When `ctx.security().is_some()`, the guard fails closed: a missing runtime or missing provider returns `Err(E::from(SecurityError::ProviderError(...)))` immediately — never silently passes (AD-9). This deliberately diverges from `enforce_tenant`'s no-op policy.
- When `ctx.security()` is `None`, the guard is skipped entirely — security capability is disabled (CORE-009D), no authorization is expected.
- Denial returns `Err(E::from(SecurityError::AuthorizationDenied{..}))` via `?` — exactly one `authorize_in_context` call per annotated method.
- `ctx` is referenced by the exact ident named in `context = ctx`; `ctx.security()` yields `Option<&SecurityContext>`, matching `authorize_in_context`'s first parameter.
- **Allocations**: Two `String` allocations per call (`Resource.kind`, `Action`) are accepted; see AD-11.

## Diagnostics

All errors are span-targeted (`syn::Error::new_spanned`) at the offending token.

| Error case | Message |
|---|---|
| E1 — permission has no `:` | `#[authorize] permission "foo" must have the form "resource:action"` |
| E1b — permission has >1 `:` | `#[authorize] permission "a:b:c" must have exactly one ':' (form "resource:action")` |
| E2 — empty resource (`":read"`) | `#[authorize] resource in ":read" must not be empty` |
| E3 — empty action (`"orders:"`) | `#[authorize] action in "orders:" must not be empty` |
| E4 — unknown named arg | `#[authorize] unknown argument 'foo'; expected 'context' and 'permission'` |
| E5 — used outside `#[service]` | `#[authorize] can only be used on methods inside a #[service] trait` |
| E6 — ctx param not in signature | `#[authorize] context parameter 'ctx' not found in method signature` |
| E_from — error type lacks `From<SecurityError>` | surfaced by the `__assert_from_security_error` bound: `OrderError: From<SecurityError> is not satisfied`. Documented remedy: `impl From<SecurityError> for OrderError`. |
| E4b — missing required arg | `#[authorize] missing required argument; both 'context' and 'permission' are required` |
| AD-4 — non-literal permission | `#[authorize] permission must be a string literal known at compile time` |
| AD-4 — non-ident context | `#[authorize] context must be a parameter name (identifier), not an expression` |

**Note on the "empty resource" prompt**: `"orders:read"` IS valid (non-empty resource and action) — E2 only fires for a genuinely empty resource such as `":read"`.

## trybuild Test Plan

Compile-fail cases under `crates/service-sdk-macros/tests/` (referenced from `tests.rs`):

| File | Asserts |
|---|---|
| `authorize_bad_format.rs` | E1 — permission missing `:` |
| `authorize_empty_resource.rs` | E2 — `":read"` |
| `authorize_empty_action.rs` | E3 — `"orders:"` |
| `authorize_missing_from.rs` | E_from — error type without `From<SecurityError>` |
| `authorize_outside_service.rs` | E5 — `#[authorize]` on a standalone fn |
| `authorize_unknown_ctx.rs` | E6 — `context = wrong` not matching any param |
| `authorize_unknown_arg.rs` | E4 — `#[authorize(context = ctx, perm = "...")]` |
| `authorize_non_literal.rs` | AD-4 — `permission = SOME_CONST` |

Each `.rs` has a paired `.stderr` with the exact message. A compile-pass smoke test (`authorize_ok.rs`) confirms a valid annotation expands and builds, guarding against false positives.

## RuntimeInner Extension

`crates/service-sdk/src/runtime/` — add one public accessor (the only change to `service-sdk`):

```rust
impl RuntimeInner {
    /// Returns a clone of the configured authorization provider, if any.
    /// Used by `#[service]`-generated proxies to enforce `#[authorize]`.
    pub fn authorization_provider(&self) -> Option<Arc<dyn AuthorizationProvider>> {
        self.security_providers
            .as_ref()
            .map(|(_, authz)| Arc::clone(authz))
    }
}
```

`security_providers` stays `pub(crate)`; the authentication provider stays private. Only the authorization `Arc` is exposed, by clone.

## File Changes

| File | Action | Description |
|------|--------|-------------|
| `crates/service-sdk-macros/src/lib.rs` | Modify | In `expand_service_trait`, detect `#[authorize]`; add `AuthorizeArgs` parser; validate literal (E1–E3, AD-4); validate `context` ident against signature params (E6); strip `#[authorize]` from clean output (like `#[operation]`); prepend authorize guard to the forwarding body. Change the `authorize` `#[proc_macro_attribute]` to emit `compile_error!` (E5). |
| `crates/service-sdk/src/runtime/` (mod hosting `RuntimeInner`) | Modify | Add `pub fn authorization_provider(&self)` accessor (AD-7). |
| `crates/service-sdk-macros/src/tests.rs` | Modify | Register the new `trybuild` compile-fail + compile-pass cases. |
| `crates/service-sdk-macros/tests/authorize_*.rs` + `.stderr` | Create | 8 compile-fail fixtures + 1 compile-pass fixture per the test plan. |
| `crates/security-sdk/src/authorization/mod.rs` | Consumed | `authorize_in_context`, `Resource`, `Action` reused unchanged. |
| `crates/service-sdk/src/context/mod.rs` | Unchanged | `ServiceContext` stays a pure DTO. |

## Testing Strategy

| Layer | What to Test | Approach |
|-------|-------------|----------|
| Unit (macro) | `AuthorizeArgs` parse + literal validation | Pure fn unit tests on the parser/validator helpers (no expansion). |
| Compile-fail | All diagnostics E1–E6, E_from, AD-4 | `trybuild` fixtures with `.stderr` snapshots. |
| Compile-pass | Valid annotation expands & builds | `authorize_ok.rs` `trybuild` pass case. |
| Integration | Guard denies before body; allows when permitted; one `authorize_in_context` call | Service with stub `AuthorizationProvider` (Allow/Deny) asserting body side-effect absent on deny. Per the project testing skill, runtime-backed integration tests use the established harness, not external resources. |

## Migration / Rollout

No migration. Purely additive — `#[service]` ignores methods without `#[authorize]`; existing services using manual `authorize_in_context` are unaffected. Rollback = revert the macros + the one `RuntimeInner` accessor.

## Open Questions

None. All open questions from the proposal are resolved:
- **AD-6**: Named syntax chosen.
- **AD-7**: `RuntimeInner::authorization_provider()` returns owned `Arc`. Accessor contract: NOT a DI API; generated code only; future accessors require explicit ADR.
- **AD-8**: Standalone `#[authorize]` emits `compile_error!`.
- **AD-9**: Fail-closed when security is enabled but provider is absent. Stable error mapping: `CapabilityNotEnabled` for missing provider, `ProviderError` for dropped runtime.
- **AD-10**: Generated proxy internals (`__rt`, `__provider`, `__assert_from_security_error`, layout) are implementation details, not public API.
- **Marker execution order**: Fixed, documented, lexical-order-independent.
- **AD-11**: Generated code reuses `Resource`/`Action` public API; two `String` allocations per call accepted; allocation-free deferred to a future `security-sdk` API change.
