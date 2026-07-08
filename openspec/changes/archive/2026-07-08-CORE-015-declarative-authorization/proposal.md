# Proposal: CORE-015 Declarative Authorization & Service Security Integration

## Intent

CORE-014 delivered runtime authorization providers (`AllowAll`, `DenyAll`, `Rbac`) and the stable seam `authorize_in_context(...)`. But authorizing a service method today means hand-writing the boilerplate: pull `SecurityContext` from `ServiceContext`, fetch the provider from the runtime, call `authorize_in_context`, map the error. This is verbose, easy to forget, and inconsistent across services. CORE-015 closes that gap with a declarative `#[authorize(context = ctx, permission = "resource:action")]` macro that injects the guard at compile time, making the security intent visible at the callsite and impossible to skip silently.

## Scope

### In Scope

- `#[authorize(context = ctx, permission = "resource:action")]` attribute macro in `ego-service-sdk-macros`.
- **Syntax**: named arguments — `context = <param-ident>` and `permission = "resource:action"`. The context parameter name is explicit, not inferred by type matching.
- Targets service methods inside `#[service]` impl blocks. `#[authorize]` is consumed by `#[service]` during normal compilation. A standalone proc-macro registration exists only to emit a friendly diagnostic (E5) when the attribute is used outside a `#[service]` trait. Valid usages are always consumed by `#[service]` before the standalone macro is invoked.
- Compile-time `resource:action` string literals only (e.g. `#[authorize(context = ctx, permission = "orders:read")]`).
- Compile-time validation of the permission literal: exactly one `:`, non-empty resource, non-empty action.
- Generated guard short-circuits BEFORE the body runs, returning `Err(E::from(SecurityError::AuthorizationDenied { .. }))`.
- Compile-time error (actionable message) when the fn error type does not implement `From<SecurityError>`.
- Compile-time error when `#[authorize]` is placed outside a `#[service]` impl block.
- Execution order is defined by the fixed marker pipeline (see design). Authorization always occupies slot 1.
- Instrumented with `trybuild` compile-fail tests.

### Out of Scope

- Actor / entity handler support (service methods only).
- Runtime interpolation / dynamic resource binding (`orders:{id}`) — deferred to **CORE-015B**.
- New authorization providers or policy languages (owned by CORE-014 / future work).
- Multi-permission / boolean composition (`AND`/`OR` of permissions).
- `ServiceContext` modification — `ServiceContext` remains a pure DTO.

## Macro Composition Architecture (AD-1 — Resolved)

`#[authorize]` is a **marker attribute consumed by `#[service]`** during proxy codegen. A standalone `#[proc_macro_attribute]` registration exists solely to emit diagnostic E5 when the attribute appears outside a `#[service]` block; valid usages are always stripped by `#[service]` before the standalone macro is ever invoked.

**Decision: Option C.**

```rust
#[service]
impl OrderService {
    #[authorize(context = ctx, permission = "orders:read")]
    async fn get_order(&self, ctx: ServiceContext, id: OrderId) -> Result<Order, MyError> {
        // body
    }
}
```

`#[service]` already parses the full `impl` block and generates the proxy. It reads `#[authorize(context = ctx, permission = "orders:read")]` on each method and emits the authorization guard in the proxy method body before forwarding to the original. No second pass, no attribute stacking, no duplicate span handling.

**Why not Option A (separate macro reading #[service] output):** `#[service]` already owns codegen — coupling a second macro to its output creates ordering fragility and duplicates all the syn complexity (async, generics, where clauses, lifetimes, cfg, doc comments).

**Why not Option B (standalone wrapper):** Forces `ServiceContext` to grow a `ctx.authorization_provider()` accessor. That contaminates a DTO with a service-locator concern. Once that door opens, `ctx.logger()`, `ctx.metrics()`, `ctx.clock()` follow. `ServiceContext` must stay a pure data carrier.

**Why named arguments over positional** (`context = ctx, permission = "orders:read"` instead of `ctx, "orders:read"`): Named form matches `#[service(version = "1.0.0")]` convention, is self-documenting, and lets future optional args (`audit = true`) be added non-breakingly. Positional couples meaning to position.

## Architecture Decision Records

### AD-3: Macro validates form; provider owns semantics

**Decision**: `#[authorize(context = ctx, permission = "orders:read")]` validates that the permission string has the form `resource:action` (exactly one `:`, both parts non-empty) and nothing more. The macro does not interpret, normalize, or restrict the content of `resource` or `action`.

**Rationale**: The semantic meaning of a permission string belongs exclusively to the `AuthorizationProvider` implementation. Different providers have legitimate reasons to use different conventions:

- RBAC: `"orders:read"`, `"invoice:approve"`
- ReBAC / Zanzibar: `"document#viewer"`, `"folder#editor"`
- Hierarchical tenancy: `"tenant/admin"`, `"org/billing:write"`

If the macro validated semantics (e.g., enforced lowercase, prohibited `/` or `#`), migrating to a different authorization model would require changing callsites across the entire codebase instead of only the provider. By owning only structural validation, the macro becomes provider-agnostic and future-proof.

**Invariant**: The macro MUST NOT add semantic constraints beyond one `:`, non-empty resource, non-empty action. Any future structural changes to this validation require an explicit ADR revision.

### AD-5: Marker expansion is deterministic and order-independent

**Decision**: The execution order of markers consumed by `#[service]` is determined solely by the framework's defined pipeline order (see design-phase task above). It is **independent of the lexical order** in which attributes appear on a method.

These two are semantically identical:
```rust
#[audit]
#[authorize(context = ctx, permission = "orders:read")]
async fn get_order(...) { ... }

#[authorize(context = ctx, permission = "orders:read")]
#[audit]
async fn get_order(...) { ... }
```

Both generate the same expansion: `authorize → audit(before) → body → audit(after)`.

**Rationale**: Attribute ordering surprises are among the hardest bugs to diagnose in macro-heavy codebases. A developer reading `#[audit] #[authorize]` should not need to know which one "wins" or runs first — the framework owns that contract, not the call site. Lexical order is visual convention, not execution policy.

**Invariant**: `#[service]` MUST apply markers in pipeline order regardless of their lexical position. Any marker that requires a specific relative ordering with another marker is an architectural smell and MUST be rejected at expansion time with an explicit error.

### AD-4: Authorization metadata is static

**Decision**: `#[authorize]` accepts only metadata known at compile time. The macro expansion does not inspect parameter values, evaluate expressions, or generate logic that depends on runtime data.

Valid:
```rust
#[authorize(context = ctx, permission = "orders:read")]
```

Not valid — rejected at expansion time:
```rust
#[authorize(context = ctx, permission = format!("orders:{}:read", id))]  // runtime expression
#[authorize(context = ctx, permission = order.permission())]              // method call
#[authorize(context = ctx, permission = PERMISSION_CONST)]                // const ref — not a literal
```

**Rationale**:
- Keeps expansion deterministic and fully analyzable at compile time.
- Avoids introducing a mini expression language inside the attribute — that complexity belongs to the `AuthorizationProvider`, not the macro.
- Any authorization that depends on runtime data (resource ownership, row-level security, relationship-based rules) belongs inside the provider's `authorize()` implementation, which already receives the full `Principal`, `AccessRequest`, and `SecurityContext`.
- Dynamic resource binding (e.g., `"orders:{id}:read"`) is explicitly deferred to CORE-015B, which will define its own syntax and expansion rules.

**Invariant**: The macro MUST NOT evaluate or interpolate any expression at expansion time. Every argument to `#[authorize]` must be a literal token.

### AD-1: Compile-time guard over runtime middleware

**Decision**: Authorization is injected at compile time via codegen, not enforced at runtime as a middleware layer.

**Rationale**:
- Zero cost: no allocation, no dynamic dispatch beyond the `AuthorizationProvider` call already required.
- Visible intent: `#[authorize]` at the callsite is auditable; a runtime interceptor is invisible until it bites you.
- No silent omissions: if the attribute is absent, the compiler does not enforce the guard — but the omission is deliberate and reviewable in code.
- Deterministic codegen: the generated proxy method is a single, readable expansion with a known call order.
- Avoids implicit middleware ordering surprises that runtime filter chains typically produce.

This decision should survive multiple major versions. Revisit only if runtime policy hot-reload becomes a requirement.

## Capabilities

### New Capabilities

- `declarative-authorization`: compile-time `#[authorize]` guard for service methods, its `resource:action` contract, compile-time literal validation, error-mapping rules, execution-order guarantee, and `trybuild` compile-fail coverage.

### Modified Capabilities

- `ego-service-sdk-macros`: `#[service]` codegen extended to read and consume `#[authorize]` markers.
- `ServiceContext`: **unchanged** — stays a pure DTO.

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `crates/service-sdk-macros/src/lib.rs` | Modified | `#[service]` codegen reads `#[authorize]`, emits guard before body |
| `crates/service-sdk-macros/src/tests.rs` | Modified | Add `trybuild` compile-fail tests for bad literals, missing `From`, wrong placement |
| `crates/security-sdk/src/authorization/mod.rs` | Consumed | `authorize_in_context` seam reused, unchanged |

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| Poor compile-error messages frustrate adoption | Med | Span-targeted `syn` diagnostics with actionable text; `trybuild` tests verify error messages |
| Missing `From<SecurityError>` surprises users | Med | Explicit diagnostic: "`MyError` must implement `From<SecurityError>` to use `#[authorize]`. Add: `impl From<SecurityError> for MyError { ... }`" |
| `#[service]` codegen complexity grows with more markers | Med | Keep each marker's codegen self-contained; define a stable marker-reading protocol in `#[service]` |
| Invalid permission literal ships silently | Low | Validate at expansion time: one `:`, non-empty parts; emit compile error |

## Rollback Plan

Purely additive — `#[service]` ignores unknown markers today (verify this in design). Revert the `ego-service-sdk-macros` change. All existing services using manual `authorize_in_context` are unaffected.

## Dependencies

- **CORE-014** (archived): provides `authorize_in_context`, `Resource`, `Action`, `AuthorizationProvider`, providers.
- **`ego-service-sdk-macros`**: existing proc-macro crate; `#[service]` is the macro extended here.

## Future Evolution

- **CORE-015B**: dynamic resource binding (`#[authorize(context = ctx, permission = "orders:{id}")]` with argument interpolation).
- Additional markers (`#[audit]`, `#[rate_limit]`) following the same `#[service]`-reads-marker pattern.
- Actor/entity handler support — would require a different codegen path (not `#[service]`).

## Success Criteria

- [ ] `#[authorize(context = ctx, permission = "orders:read")]` on a service method short-circuits with `AuthorizationDenied` before the body runs when denied, and runs the body when allowed.
- [ ] Invalid permission literal (missing `:`, empty resource, empty action) fails compilation.
- [ ] Compile-time error with actionable message when the fn error type lacks `From<SecurityError>`.
- [ ] Compile-time error when `#[authorize]` appears outside a `#[service]` impl block.
- [ ] Authorization executes before any user-defined body code in the generated proxy.
- [ ] Generated code performs exactly one `authorize_in_context()` invocation per annotated method.
- [ ] No changes to `ServiceContext` — it remains a pure DTO.
- [ ] No changes required to existing `AuthorizationProvider` implementations.
- [ ] No ambient runtime access introduced beyond the proxy's existing `Weak<RuntimeInner>`.
- [ ] `cargo test --workspace` green, including `trybuild` compile-fail tests.

## Resolved Design-Phase Questions

**Syntax** (resolved — AD-6): Named-argument form chosen: `#[authorize(context = ctx, permission = "orders:read")]`. Matches `#[service(version = "1.0.0")]` convention; future optional args are non-breaking.

**Marker execution order** (resolved — design.md): Fixed pipeline documented; lexical order of attributes has no effect on generated pipeline. See design document for the full slot table.
