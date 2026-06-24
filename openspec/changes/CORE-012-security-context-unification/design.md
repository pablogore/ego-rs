# Design: CORE-012 — Security Context Unification

## Technical Approach

Consolidate auth contracts under `security-sdk`: move `AuthenticationProvider` from `domain::auth`, eliminate `Identity` for `Principal`, replace `SecurityContext.scope` with `domain::auth::Claims`. Single `SecurityContext` consumed by both authN and authZ — no conversion layer.

## Architecture Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| `security-sdk` → `ego-domain` dep | **Depend on domain** | Domain owns `Claims`/`AuthenticationError` data models; security-sdk re-exports |
| `AuthenticationProvider` signature | **Sync, returns `AuthenticationError`** | AD-004: auth is CPU-bound. `SecurityError` reserved for authZ |
| `Principal.claims` | **Removed** | AD-003: Principal = identity only. All raw claims live in `SecurityContext.claims` |
| `SecurityContext.scope` | **Replaced by `claims`** | Claims are canonical transport metadata. Scope was an interim placeholder |
| `CredentialVerifier` signature | **Sync** | AD-004: auth performs no I/O. `CredentialVerifier` is an auth sub-component; consistency requires sync |

## Data Flow

```
Credential ──→ AuthenticationProvider ──→ SecurityContext
                                            ├── principal: Principal
                                            └── claims: Claims

SecurityContext
  └── ServiceContext.security (Option<Arc<SecurityContext>>)
        └── AuthorizationProvider::authorize(principal, request, ctx)
```

## Phase 1 — Contract Migration

Move `AuthenticationProvider` trait to `security-sdk`.

| File | Action | Description |
|------|--------|-------------|
| `crates/security-sdk/src/authentication/mod.rs` | Modify | Sync trait: `fn authenticate(&self, cred: &Credential) -> Result<SecurityContext, AuthenticationError>` |
| `crates/security-sdk/Cargo.toml` | Modify | Add `ego-domain` dep |
| `crates/security-sdk/src/lib.rs` | Modify | Re-export `AuthenticationError` from domain |
| `crates/domain/src/auth/provider.rs` | Delete | Trait moved to security-sdk |
| `crates/domain/src/auth/mod.rs` | Modify | Remove `provider` module + re-export |

## Phase 2 — Identity Unification

Remove `Identity`. `Principal` is canonical identity type.

| File | Action | Description |
|------|--------|-------------|
| `crates/domain/src/auth/identity.rs` | Delete | `Identity` removed |
| `crates/domain/src/auth/mod.rs` | Modify | Remove `identity` module + re-export |
| `crates/security-sdk/src/principal/principal.rs` | Modify | Add `tenant_id: Option<String>` + `with_tenant_id()`. Remove `claims: Vec<Claim>` + `with_claim()` |

## Phase 3 — SecurityContext Unification

Canonical `SecurityContext` lives in security-sdk only.

| File | Action | Description |
|------|--------|-------------|
| `crates/domain/src/auth/security_context.rs` | Delete | Type moved to security-sdk |
| `crates/domain/src/auth/mod.rs` | Modify | Remove `security_context` module + re-export |
| `crates/security-sdk/src/context/mod.rs` | Modify | Add `claims: Claims` field. `new(principal, claims)`. Remove `scope`. Expose `claims()`. Re-export `Claims`/`StandardClaims` from domain |

## Phase 4 — Runtime Wiring

End-to-end: `Credential → AuthenticationProvider → SecurityContext → ServiceContext → AuthorizationProvider`.

| File | Action | Description |
|------|--------|-------------|
| `crates/security-sdk/src/providers/basic/mod.rs` | Modify | Sync, returns `SecurityContext(principal, Claims::empty())` |
| `crates/security-jwt/src/authenticator.rs` | Modify | Implement `ego_security_sdk::AuthenticationProvider` (sync). Map `Identity` fields → `Principal.{tenant_id, roles}`, return `SecurityContext` |
| `crates/security-jwt/src/lib.rs` | Modify | Doc references → security-sdk |
| `crates/security-jwt/Cargo.toml` | Modify | Add `ego-security-sdk` dep |
| `crates/service-sdk/src/runtime/builder.rs` | Verify | Already imports security-sdk trait. Stub tests need return type update |
| `crates/service-sdk/src/context/mod.rs` | Already done | `security: Option<Arc<SecurityContext>>` exists |

## Phase 5 — Cleanup

Verify zero dead imports across workspace.

| File | Action | Description |
|------|--------|-------------|
| `crates/security-jwt/src/authenticator.rs` tests | Modify | `ctx.identity.*` → `ctx.principal().*` |
| `crates/security-sdk/src/providers/basic/mod.rs` tests | Modify | Sync mocks, return `SecurityContext` |
| `crates/domain/src/auth/claims.rs` | Keep | Pure data model, re-exported via security-sdk |
| `crates/domain/src/auth/credential.rs` | Keep | Pure data model |
| `crates/domain/src/auth/error.rs` | Keep | `AuthenticationError` — re-exported via security-sdk |
| `crates/domain/src/auth/clock.rs` | Keep | Unchanged |
| `crates/domain/src/auth/mod.rs` | Finalize | Only `error`, `credential`, `claims`, `clock` remain |

## Interfaces / Contracts

```rust
// security-sdk AuthenticationProvider (sync)
pub trait AuthenticationProvider: Send + Sync {
    fn authenticate(&self, credential: &Credential) -> Result<SecurityContext, AuthenticationError>;
}

// security-sdk SecurityContext (with claims)
pub struct SecurityContext {
    pub principal: Principal,
    pub claims: Claims,  // from domain::auth, re-exported
}

// Principal (no claims, has tenant_id)
pub struct Principal {
    pub kind: PrincipalKind,
    pub subject_id: SubjectId,
    pub tenant_id: Option<String>,
    pub roles: HashSet<Role>,
    pub attributes: HashMap<String, String>,
}
```

## Testing Strategy

| Layer | What | How |
|-------|------|-----|
| Unit | `SecurityContext::new(p, c)` | Assert `principal()` and `claims()` match inputs |
| Unit | Sync `AuthenticationProvider` | Arc-storability compile check; stub returns `SecurityContext` |
| Unit | `BasicAuthenticationProvider` sync path | Valid/invalid → `SecurityContext` or `AuthenticationError` |
| Unit | `JwtAuthenticator` returns `Principal` | Assert `principal().tenant_id()` maps from JWT `tid` |
| Unit | `Principal.claims` removed | Compile-time: no `.claims` on `Principal` |
| Integration | authN→authZ pipeline | Authenticate → `ServiceContext.security` → `authorize()` |
| Integration | `domain::auth` reduced | Grep `SecurityContext`/`Identity`/`AuthenticationProvider` in `crates/domain/` → zero matches |

## Migration

Compile-time refactor only. All callers update imports: `ego_domain::auth::X` → `ego_security_sdk::X`. Compiler catches every missed reference. Execute phases sequentially, `cargo test --workspace` after each.

## Open Questions

- [ ] `scope` field coexistence? (Decision: replaced by `claims` — no code reads `scope` outside tests.)
