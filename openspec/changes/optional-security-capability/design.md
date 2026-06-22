# Design: CORE-010B — Optional Security Capability

## Technical Approach

Make security an opt-in runtime capability through five changes: (1) add `SecurityError::CapabilityNotEnabled` variant, (2) change `authorize_in_context` to return it (not `MissingContext`) when `security == None`, (3) add `require_security()` on `ServiceContext` (existing `security()` accessor is unchanged), (4) create `RuntimeBuilder` with optional `.with_security(authn, authz)`, (5) migrate call sites from mandatory to optional assumptions.

All resolved decisions (D1/B, GAP-008, D3/A, GAP-009, A1) are inherited unchanged — see proposal for rationale. This design implements, not re-debates, them.

## Architecture Decisions

| Decision | Choice | Alternatives | Rationale |
|----------|--------|--------------|-----------|
| `RuntimeBuilder` location | New file `runtime/builder.rs` | Inline in `runtime_builder.rs` | Keeps builder separate from `RuntimeInner` state. `RuntimeInner` stays internal; `Runtime`/`RuntimeBuilder` public. |
| Provider storage in runtime | `RuntimeInner` gains `Option<(Arc<dyn AuthenticationProvider>, Arc<dyn AuthorizationProvider>)>` | New `SecurityCapability` struct; separate `Arc` per field | Tuple matches `with_security()` signature. No new type needed. |
| Existing `security` field on `ServiceContext` | **Untouched** — remains `pub` | Make private with getters | Spec says unchanged. The field was already `pub` before CORE-010B; changing visibility adds unnecessary churn. |

## Data Flow

```
RuntimeBuilder::new()               → Runtime { inner: Arc<RuntimeInner> }
RuntimeBuilder::new()                → Runtime { inner (security_providers: None) }
  .with_security(authn, authz)
  .build()                           → Runtime { inner (security_providers: Some(...)) }

ServiceContext::security()           → None                                (providers not installed)
ServiceContext::security()           → Some(&SecurityContext)              (providers installed + auth'd)
ServiceContext::require_security()   → Err(CapabilityNotEnabled)           (providers not installed)
ServiceContext::require_security()   → Ok(&SecurityContext)                (providers installed + auth'd)

authorize_in_context(None, ...)      → Err(CapabilityNotEnabled)           [changed from MissingContext]
authorize_in_context(Some(ctx), ...) → Ok(()) | Err(AuthorizationDenied)   [unchanged behavior]
```

## File Changes

| File | Action | Description |
|------|--------|-------------|
| `crates/security-sdk/src/error/mod.rs` | Modify | Add `CapabilityNotEnabled` variant + display test |
| `crates/security-sdk/src/authorization/mod.rs` | Modify | `authorize_in_context` returns `CapabilityNotEnabled` on `None`; update doc comment + unit test |
| `crates/security-sdk/tests/declarative_authz.rs` | Modify | `none_security_returns_missing_context` → `none_security_returns_capability_not_enabled` |
| `crates/service-sdk/src/context/mod.rs` | Modify | Add `require_security()` method (existing `security()` unchanged) |
| `crates/service-sdk/src/runtime/builder.rs` | **Create** | `RuntimeBuilder` + `Runtime` public types |
| `crates/service-sdk/src/runtime/runtime_builder.rs` | Modify | Add optional `security_providers` field to `RuntimeInner` |
| `crates/service-sdk/src/runtime/mod.rs` | Modify | Export `Runtime`, `RuntimeBuilder` from new module |

## Interfaces / Contracts

```rust
// ── security-sdk: error/mod.rs ──────────────────────────────────────────
pub enum SecurityError {
    // ... existing variants unchanged ...
    /// Security capability was not installed in this runtime.
    #[error("security capability not enabled")]
    CapabilityNotEnabled,
}

// ── security-sdk: authorization/mod.rs (authorize_in_context) ───────────
// Before: security.ok_or(SecurityError::MissingContext)?
// After:  security.ok_or(SecurityError::CapabilityNotEnabled)?

// ── service-sdk: context/mod.rs ─────────────────────────────────────────
// `security()` already exists and is unchanged.
// Only `require_security()` is added.
impl ServiceContext {
    /// Returns the [`SecurityContext`] or fails with [`SecurityError::CapabilityNotEnabled`].
    pub fn require_security(&self) -> Result<&SecurityContext, SecurityError> {
        self.security.as_deref().ok_or(SecurityError::CapabilityNotEnabled)
    }
}

// ── service-sdk: runtime/builder.rs ─────────────────────────────────────
pub struct RuntimeBuilder {
    registry: ServiceRegistry,
    interceptor_chain: Arc<InterceptorChain>,
    authn: Option<Arc<dyn AuthenticationProvider>>,
    authz: Option<Arc<dyn AuthorizationProvider>>,
}

impl RuntimeBuilder {
    pub fn new() -> Self;
    pub fn with_security(
        self,
        authn: Arc<dyn AuthenticationProvider>,
        authz: Arc<dyn AuthorizationProvider>,
    ) -> Self;
    pub fn build(self) -> Runtime;
}

pub struct Runtime {
    inner: Arc<RuntimeInner>,
}
```

## Testing Strategy

| Layer | What | How |
|-------|------|-----|
| Unit | `CapabilityNotEnabled` variant | Display test + pattern-matching test in `error/mod.rs` |
| Unit | `authorize_in_context` with `None` | Expect `Err(CapabilityNotEnabled)` (update existing test) |
| Unit | `ServiceContext::require_security()` | `Err(CapabilityNotEnabled)` when unset; `Ok` when set |
| Unit | `RuntimeBuilder::build()` w/o security | Returns `Runtime`; no providers stored |
| Unit | `RuntimeBuilder::build()` w/ security | Returns `Runtime`; providers stored in `RuntimeInner` |
| Integration | Declarative authz with `None` | Existing test updated to expect correct variant |

## Migration / Rollout

No migration required. Pure additive refactoring with one behavior change (`authorize_in_context` error variant). The one caller (`declarative_authz.rs` integration test) is updated in the same change. All existing construction sites compile unchanged.

## Open Questions

- None. All architecture decisions resolved in proposal.
