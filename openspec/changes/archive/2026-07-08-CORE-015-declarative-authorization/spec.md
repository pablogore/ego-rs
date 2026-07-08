# Spec: CORE-015 Declarative Authorization & Service Security Integration

> Delta spec — describes only what CORE-015 adds or changes.
> Existing behavior from CORE-014 and other crates is not re-specified.

---

## Functional Requirements

### FR-1 — `#[authorize]` syntax contract

The macro `#[authorize]` accepts exactly two named arguments: `context = <ident>` and `permission = "<resource>:<action>"`.

**Acceptance criteria:**

- AC-1.1: `#[authorize(context = ctx, permission = "orders:read")]` on a service method inside `#[service]` compiles and generates an authorization guard.
- AC-1.2: The named argument `context` receives an identifier, not an expression or path.
- AC-1.3: The named argument `permission` receives a string literal, not a const reference, macro call, or any other expression form.

---

### FR-2 — Named-argument form is the only accepted form; positional is rejected

**Acceptance criteria:**

- AC-2.1: `#[authorize(ctx, "orders:read")]` (positional) fails compilation with error E4 (`unknown argument`).
- AC-2.2: `#[authorize(context = ctx, perm = "orders:read")]` (unknown key name) fails compilation with error E4.
- AC-2.3: `#[authorize(context = ctx)]` (missing `permission`) fails compilation with error E4b.
- AC-2.4: `#[authorize(permission = "orders:read")]` (missing `context`) fails compilation with error E4b.

---

### FR-3 — Compile-time structural validation of the permission literal

The permission literal must satisfy: exactly one `:`, non-empty string before `:` (resource), non-empty string after `:` (action). No semantic constraints are applied beyond this structure.

**Acceptance criteria:**

- AC-3.1: A permission literal with no `:` (e.g., `"ordersread"`) fails compilation with error E1.
- AC-3.2: A permission literal with more than one `:` (e.g., `"a:b:c"`) fails compilation with error E1b.
- AC-3.3: A permission literal with an empty resource (e.g., `":read"`) fails compilation with error E2.
- AC-3.4: A permission literal with an empty action (e.g., `"orders:"`) fails compilation with error E3.
- AC-3.5: A non-literal value for `permission` (e.g., a const reference `PERM_CONST`) fails compilation with the AD-4 non-literal error.
- AC-3.6: A valid literal like `"orders:read"` does not trigger E2 (non-empty resource is correctly identified).

---

### FR-4 — Guard executes BEFORE the method body; exactly one `authorize_in_context` call per annotated method

**Acceptance criteria:**

- AC-4.1: When the authorization provider denies the request, the service method body does not execute (no observable side effect from the body).
- AC-4.2: The generated proxy contains exactly one call to `authorize_in_context` per `#[authorize]`-annotated method.
- AC-4.3: The authorization guard appears as the first executable step in the generated proxy body, before `enforce_tenant`, interceptor `on_request`, and the inner method call.

---

### FR-5 — Fail-closed policy when security is enabled (AD-9)

| Security state | Guard behavior | Error returned |
|---|---|---|
| `ctx.security()` is `None` (security capability disabled) | Guard not emitted; call proceeds | — |
| Security enabled; `runtime.upgrade()` returns `None` (runtime dropped) | Fail closed | `SecurityError::ProviderError("authorization provider unavailable: runtime dropped")` |
| Security enabled; authorization resolution yields `CapabilityNotEnabled` | Fail closed | `SecurityError::CapabilityNotEnabled` |
| Security enabled; provider present; provider denies | Fail closed | `SecurityError::AuthorizationDenied { .. }` (propagated from provider) |
| Security enabled; provider present; provider allows | Guard passes; body executes | — |

**Acceptance criteria:**

- AC-5.1: When `ctx.security()` is `None`, the method body executes without any authorization check.
- AC-5.2: When the runtime `Weak` reference has been dropped and `ctx.security()` is `Some`, the method returns `Err(E::from(SecurityError::ProviderError(...)))`.
- AC-5.3: When authorization resolution yields `SecurityError::CapabilityNotEnabled`, the generated guard propagates that error and the method body does not execute.
- AC-5.4: When the provider returns `Deny`, the method returns `Err(E::from(SecurityError::AuthorizationDenied { .. }))` and the body does not execute.
- AC-5.5: When the provider returns `Allow`, the method body executes and returns its result.

---

### FR-6 — Compile-time `From<SecurityError>` bound on the method's error type

**Acceptance criteria:**

- AC-6.1: A method whose `Result<_, E>` has an error type `E` that does not implement `From<SecurityError>` fails compilation with error E_from.
- AC-6.2: The compile error is rustc's standard trait bound diagnostic, triggered by the `__assert_from_security_error::<E>()` helper; the span targets the error type with a message identifying the missing `impl From<SecurityError> for E`. No custom `compile_error!` is emitted.

---

### FR-7 — `#[authorize]` outside `#[service]` emits a compile error (E5)

**Acceptance criteria:**

- AC-7.1: `#[authorize]` applied to a free function (outside any `#[service]` impl block) fails compilation with error E5.
- AC-7.2: `#[authorize]` applied to a function inside a plain `impl` block (not `#[service]`) fails compilation with error E5.
- AC-7.3: When `#[authorize]` is used correctly inside `#[service]`, error E5 is never emitted.

---

### FR-8 — Marker execution order is fixed and lexical-order-independent (AD-5 / AD-10)

The pipeline order is:

```
1. authorize
2. [future pre-body marker]
3. enforce_tenant
4. chain.on_request
5. inner.method(args)
6. chain.on_response / on_error
7. [future post-body marker]
8. return result
```

**Acceptance criteria:**

- AC-8.1: A method annotated `#[audit] #[authorize(...)]` generates the same proxy body as `#[authorize(...)] #[audit]` — the order of authorization relative to other markers is determined by the pipeline, not by lexical attribute position.
- AC-8.2: The generated proxy always places the authorization guard at slot 1 (before `enforce_tenant`, before interceptors).

---

### FR-9 — `ServiceContext` remains a pure DTO

**Acceptance criteria:**

- AC-9.1: No new methods, fields, or trait implementations are added to `ServiceContext` in this change.
- AC-9.2: `ServiceContext` does not expose a reference or accessor to any runtime provider.

---

### FR-10 — `RuntimeInner::authorization_provider()` accessor added (AD-7)

**Acceptance criteria:**

- AC-10.1: `RuntimeInner` exposes `pub fn authorization_provider(&self) -> Option<Arc<dyn AuthorizationProvider>>`.
- AC-10.2: The method returns `None` when no security providers are configured.
- AC-10.3: The method returns `Some(Arc<dyn AuthorizationProvider>)` (an owned clone) when an authorization provider is configured.
- AC-10.4: The authentication provider remains inaccessible; only the authorization `Arc` is exposed.

> **Accessibility contract (AD-7):** This accessor is `pub` solely to satisfy Rust's visibility rules for code generated by proc-macros. It is not part of the application programming model; application code must not call it directly. Any future public accessor on `RuntimeInner` requires an explicit ADR.

---

## Non-Functional Requirements

### NF-1 — No new public API beyond `RuntimeInner::authorization_provider()` and the `#[authorize]` marker

- No new types, traits, or functions are added to any public crate surface beyond those two items.

### NF-2 — Generated internals are not public API (AD-10)

The following generated identifiers are implementation details, not part of any stability contract:

| Identifier | Role |
|---|---|
| `__rt` | Temporary `Arc<RuntimeInner>` in the proxy body |
| `__provider` | Temporary `Arc<dyn AuthorizationProvider>` in the proxy body |
| `__assert_from_security_error` | Zero-size helper function enforcing the `From<SecurityError>` bound |

These names MUST NOT appear in hand-written application code. `cargo expand` output is a debugging aid, not a compatibility contract.

### NF-3 — Two `String` allocations per call are accepted; allocation-free is deferred (AD-11)

- Generated code constructs `Resource { kind: "...".to_string(), .. }` and `Action("...".to_string())` — two `String` allocations per authorized call.
- These allocations are intentional: they reuse the stable `security-sdk` `Resource`/`Action` owned API.
- Allocation-free variants (`&'static str`, `Cow`) require a future `security-sdk` API change and are out of scope.

---

## Diagnostics Contract

All errors are span-targeted at the offending token.

| Code | Trigger | Required message |
|---|---|---|
| E1 | Permission literal has no `:` | `#[authorize] permission "foo" must have the form "resource:action"` |
| E1b | Permission literal has more than one `:` | `#[authorize] permission "a:b:c" must have exactly one ':' (form "resource:action")` |
| E2 | Empty resource (e.g., `":read"`) | `#[authorize] resource in ":read" must not be empty` |
| E3 | Empty action (e.g., `"orders:"`) | `#[authorize] action in "orders:" must not be empty` |
| E4 | Unknown named argument | `#[authorize] unknown argument 'foo'; expected 'context' and 'permission'` |
| E4b | Missing required argument | `#[authorize] missing required argument; both 'context' and 'permission' are required` |
| E5 | `#[authorize]` used outside `#[service]` | `#[authorize] can only be used on methods inside a #[service] trait` |
| E6 | `context = <ident>` names a param not present in the method signature | `#[authorize] context parameter 'ctx' not found in method signature` |
| E_from | Method error type lacks `From<SecurityError>` | rustc trait bound error at error type (e.g., `the trait bound \`OrderError: From<SecurityError>\` is not satisfied`); emitted by `__assert_from_security_error::<E>()` helper — no custom message |
| AD-4 (non-literal) | `permission` value is not a string literal | `#[authorize] permission must be a string literal known at compile time` |
| AD-4 (non-ident) | `context` value is not an identifier | `#[authorize] context must be a parameter name (identifier), not an expression` |

---

## Test Scenarios

### Compile-fail (trybuild) — 8 fixtures

| Fixture file | Error asserted | Linked requirement |
|---|---|---|
| `authorize_bad_format.rs` | E1 — permission missing `:` | FR-3, AC-3.1 |
| `authorize_empty_resource.rs` | E2 — `":read"` | FR-3, AC-3.3 |
| `authorize_empty_action.rs` | E3 — `"orders:"` | FR-3, AC-3.4 |
| `authorize_missing_from.rs` | E_from — error type without `From<SecurityError>` | FR-6, AC-6.1 |
| `authorize_outside_service.rs` | E5 — `#[authorize]` on a standalone fn | FR-7, AC-7.1 |
| `authorize_unknown_ctx.rs` | E6 — `context = wrong` not matching any param | FR-1, AC-1.2 |
| `authorize_unknown_arg.rs` | E4 — unknown named key `perm` | FR-2, AC-2.2 |
| `authorize_non_literal.rs` | AD-4 non-literal — `permission = SOME_CONST` | FR-3, AC-3.5 |

Each fixture has a paired `.stderr` snapshot with the exact diagnostic message.

### Compile-pass (trybuild) — 1 fixture

| Fixture file | Purpose | Linked requirement |
|---|---|---|
| `authorize_ok.rs` | Valid annotation expands and builds — confirms no false positive | FR-1, FR-4 |

### Integration tests — runtime behavior

| Scenario | Condition | Expected outcome | Linked requirement |
|---|---|---|---|
| Allow path | Provider returns `Allow`; `ctx.security()` is `Some` | Method body executes; result returned normally | FR-5, AC-5.5 |
| Deny path | Provider returns `Deny`; `ctx.security()` is `Some` | `Err(AuthorizationDenied)` returned; body side-effect absent | FR-4, FR-5, AC-4.1, AC-5.4 |
| One call per method | Any annotated method call | Exactly one `authorize_in_context` invocation observable per call | FR-4, AC-4.2 |
| Security disabled | `ctx.security()` is `None` | Body executes; no authorization invoked | FR-5, AC-5.1 |
| Runtime dropped | `Weak::upgrade()` returns `None`; security enabled | `Err(ProviderError(...))` returned; body not executed | FR-5, AC-5.2 |
| Provider absent | Authorization resolves to `CapabilityNotEnabled`; security enabled | `Err(CapabilityNotEnabled)` returned; body not executed | FR-5, AC-5.3 |

Integration tests use a stub `AuthorizationProvider` (Allow/Deny). Runtime-backed tests use the established project test harness (no external resources required).

---

## Out of Scope

The following are explicitly deferred and MUST NOT be implemented as part of CORE-015:

- **Dynamic resource binding** (`"orders:{id}"` with runtime interpolation) — deferred to CORE-015B.
- **Actor / entity handler support** — requires a different codegen path outside `#[service]`.
- **Multi-permission composition** — `AND`/`OR` of permissions.
- **New authorization providers or policy languages** — owned by CORE-014 and future work.
- **`ServiceContext` modifications** — `ServiceContext` stays a pure DTO.
- **Allocation-free `Resource`/`Action`** — requires a future `security-sdk` API change (AD-11).
- **Additional `RuntimeInner` accessors** — any future accessor requires an explicit ADR.
