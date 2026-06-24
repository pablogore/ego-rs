# Proposal: CORE-012 — Security Context Unification

## Intent

Eliminate the dual `SecurityContext` model (`ego_domain::auth::SecurityContext` vs `ego_security_sdk::context::SecurityContext`) that breaks the authN→authZ pipeline. Today `JwtAuthenticator` produces a domain `SecurityContext` that `AuthorizationProvider` cannot consume — they speak different models.

## Scope

### In Scope
- Move `AuthenticationProvider` trait from `domain/auth` to `security-sdk` (Opción A)
- Canonical `SecurityContext` lives in `security-sdk` (removes `domain/auth` copy)
- Unify `Identity ↔ Principal`: add `tenant_id` to `Principal`, map JWT roles to `HashSet<Role>`, preserve `Claims` (JWT raw) in `SecurityContext`
- `JwtAuthenticator` implements the security-sdk `AuthenticationProvider` (sync contract)
- `AuthenticationProvider::authenticate()` returns `SecurityContext` (not bare `Principal`)
- End-to-end pipeline: `Credential → AuthenticationProvider → SecurityContext → AuthorizationProvider → AuthorizationDecision`
- Remove `domain::auth::SecurityContext` and `domain::auth::AuthenticationProvider`
- All existing tests pass

### Out of Scope
- ES256 / EdDSA algorithm support
- JWKS remote key resolver (CORE-011B)
- `#[authorize(...)]` attribute macro
- Permission model redesign
- Principal model structural redesign (extends, does not replace)

## Capabilities

### Modified Capabilities
- `security-sdk`: `AuthenticationProvider` returns `SecurityContext` instead of `Principal`; `SecurityContext` gains `claims: Claims` field
- `domain/auth`: Remove `SecurityContext` and `AuthenticationProvider` (types move to security-sdk; `Identity`, `Claims`, `Credential`, `AuthenticationError` remain as pure models)

## Approach

Move `AuthenticationProvider` into `security-sdk` so both authN and authZ traits share the same `SecurityContext`. Extend `Principal` with `tenant_id` from `Identity`. `JwtAuthenticator` exposes synchronous `authenticate()` semantics. Async runtimes are responsible for wrapping synchronous authentication when needed. Domain keeps pure data models (`Identity`, `Claims`, etc.) without traits. No conversion layer survives archive.

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `domain/src/auth/provider.rs` | Removed | Trait moves to security-sdk |
| `domain/src/auth/security_context.rs` | Removed | Canonical type moves to security-sdk |
| `security-sdk/src/authentication/mod.rs` | Modified | Returns SecurityContext, not Principal |
| `security-sdk/src/context/mod.rs` | Modified | Add `claims: Claims` field |
| `security-sdk/src/principal/principal.rs` | Modified | Add `tenant_id: Option<String>` |
| `security-jwt/src/authenticator.rs` | Modified | Implements security-sdk trait (sync) |
| `service-sdk/src/runtime/builder.rs` | Adapted | Uses security-sdk AuthenticationProvider |

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| Changing AuthenticationProvider from async to sync may require migration of existing callers | Low | Synchronous authentication is the canonical contract; async callers wrap explicitly |
| BasicAuthenticationProvider returns Principal, needs to build SecurityContext | Low | Construct SecurityContext from Principal + empty Claims |
| Consumers must migrate to canonical security-sdk AuthenticationProvider | Med | No alternate auth abstraction survives — single contract enforces uniformity |

## Rollback Plan

Revert the single commit that removes `domain::auth::SecurityContext` and `domain::auth::AuthenticationProvider`. Domain trait still compiles. All consumers fall back to previous trait. Duplicate models reappear but nothing breaks.

## Dependencies

- CORE-011A (KeyResolver) — already archived and merged

## Success Criteria

- [ ] Single `SecurityContext` type in workspace
- [ ] `AuthenticationProvider` and `AuthorizationProvider` share same `SecurityContext`
- [ ] JWT authentication produces context consumed directly by authorization
- [ ] No `impl From<A> for B` conversion layer survives
- [ ] All existing tests pass across workspace
