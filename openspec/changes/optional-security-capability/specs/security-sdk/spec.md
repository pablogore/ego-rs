# Delta for security-sdk — Optional Security Capability

## ADDED Requirements

### Requirement: SecurityError::CapabilityNotEnabled variant

`SecurityError` MUST include a `CapabilityNotEnabled` variant with no payload fields. This variant represents "security was never installed in the runtime" — distinct from `MissingContext` which represents "capability exists but was not propagated".

#### Scenario: Variant matches correctly

- GIVEN a `SecurityError::CapabilityNotEnabled` value
- WHEN the value is pattern-matched
- THEN it matches the `CapabilityNotEnabled` arm (no payload)

#### Scenario: Returned when runtime has no security

- GIVEN a runtime built without `.with_security()`
- WHEN `authorize_in_context` is called on a `ServiceContext` with `security == None`
- THEN `Err(SecurityError::CapabilityNotEnabled)` is returned

## MODIFIED Requirements

### Requirement: FR-012 — SecurityContext explicit propagation through ServiceContext

The existing field `security: Option<Arc<SecurityContext>>` on `ServiceContext` (introduced by the Security SDK) is unchanged. This change defines the semantics and modifies `authorize_in_context` behavior:

- `security == None` means "security capability not installed in this runtime" — a valid deployment state, not a propagation failure
- `security == Some(arc_ctx)` means "capability installed; `arc_ctx.principal()` is always valid"
- The field MUST default to `None` when not set
- The field MUST be propagated unchanged through all runtime execution paths
- The field MUST never be resolved via thread-local, task-local, or any global mechanism

When `security == None`, `authorize_in_context` MUST return `SecurityError::CapabilityNotEnabled` instead of `SecurityError::MissingContext`. The `MissingContext` variant is retained in the enum for potential future internal-invariant detection.
(Previously: `authorize_in_context` returned `SecurityError::MissingContext` when `security == None`)

#### Scenario: Backward compatibility — field defaults to None

- GIVEN an existing test that constructs a `ServiceContext` without specifying `security`
- WHEN the test is compiled after this change
- THEN it compiles without errors or warnings (the field defaults to `None`)

#### Scenario: Security propagates through call chain unchanged

- GIVEN a `SecurityContext` wrapped in `Some(Arc::new(ctx))`
- WHEN a `ServiceContext` carrying that field is passed through a `RuntimeBuilder`-wired call chain
- THEN the receiving component reads the same `SecurityContext` from `service_ctx.security`

#### Scenario: authorize_in_context returns CapabilityNotEnabled for unconfigured runtime

- GIVEN a `ServiceContext` with `security == None` (valid state: no security installed)
- WHEN `authorize_in_context` is called
- THEN `Err(SecurityError::CapabilityNotEnabled)` is returned

#### Scenario: MissingContext retained in enum

- GIVEN the `SecurityError` enum definition
- WHEN all variants are enumerated
- THEN `MissingContext` is present alongside `CapabilityNotEnabled`

### Error Conditions (updated)

| Condition | Trigger | Expected Result |
|-----------|---------|-----------------|
| Security not installed | `authorize_in_context` with `security == None` | `Err(SecurityError::CapabilityNotEnabled)` |
