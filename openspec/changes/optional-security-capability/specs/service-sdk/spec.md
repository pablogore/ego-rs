# Delta for service-sdk — Optional Security Capability

## ADDED Requirements

### Requirement: ServiceContext security accessor methods

`ServiceContext` MUST expose the existing `security()` method (returns `Option<&SecurityContext>`) for optional access (Layer 1), and MUST additionally expose `require_security()` for fail-fast controlled access (Layer 2):

```rust
// Layer 1 — existing, unchanged
pub fn security(&self) -> Option<&SecurityContext>;

// Layer 2 — new
pub fn require_security(&self) -> Result<&SecurityContext, SecurityError>;
```

`security()` returns `None` when the capability is not installed. `require_security()` returns `Err(SecurityError::CapabilityNotEnabled)` when not installed — never panics. Both read from the internal `security: Option<Arc<SecurityContext>>` field. No ambient or global state is consulted.

#### Scenario: Optional access returns None for unconfigured runtime

- GIVEN a `ServiceContext` with `security == None`
- WHEN `security()` is called
- THEN `None` is returned

#### Scenario: Optional access returns Some for configured runtime

- GIVEN a `ServiceContext` with `security == Some(Arc::new(security_ctx))`
- WHEN `security()` is called
- THEN `Some(&SecurityContext)` is returned referencing the expected security context

#### Scenario: Required access fails when security not installed

- GIVEN a `ServiceContext` with `security == None`
- WHEN `require_security()` is called
- THEN `Err(SecurityError::CapabilityNotEnabled)` is returned

#### Scenario: Required access succeeds when security installed

- GIVEN a `ServiceContext` with `security == Some(Arc::new(security_ctx))`
- WHEN `require_security()` is called
- THEN `Ok(&SecurityContext)` is returned

### Requirement: RuntimeBuilder optional security registration

`RuntimeBuilder` MUST support optional security provider registration:

```rust
pub fn with_security(
    self,
    authn: Arc<dyn AuthenticationProvider>,
    authz: Arc<dyn AuthorizationProvider>,
) -> Self;
```

`build()` MUST succeed whether or not `.with_security()` was called. When `.with_security()` IS called, every `ServiceContext` produced by the runtime MUST have `security: Some(...)` with the context derived from the registered providers. When NOT called, every `ServiceContext` MUST have `security == None`. No global or ambient provider state is introduced — capability is instance-scoped to the runtime.

#### Scenario: Build without security succeeds

- GIVEN `RuntimeBuilder::new()`
- WHEN `.build()` is called without calling `.with_security()`
- THEN a valid `Runtime` is returned with no security configured
- AND every `ServiceContext` in the runtime has `security == None`

#### Scenario: Build with security succeeds

- GIVEN `RuntimeBuilder::new()`
- WHEN `.with_security(authn_provider, authz_provider).build()` is called
- THEN a valid `Runtime` is returned with security configured
- AND every `ServiceContext` has `security: Some(...)`

#### Scenario: No global security state

- GIVEN a `Runtime` built with `.with_security()`
- WHEN grep gates are run for `static SECURITY_PROVIDER`, `lazy_static!`, `OnceCell`, `task_local!` in `crates/service-sdk/src/`
- THEN zero matches related to security or provider state are returned
