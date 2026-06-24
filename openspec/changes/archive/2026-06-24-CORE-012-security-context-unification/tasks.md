# Tasks: CORE-012 — Security Context Unification

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | 350–450 |
| 400-line budget risk | Medium |
| Chained PRs recommended | No |
| Suggested split | Single PR |
| Delivery strategy | single-pr |

Decision needed before apply: No
Chained PRs recommended: No
Chain strategy: size-exception
400-line budget risk: Medium

## Phase 1 — Contract Migration

- [x] 1.1 `crates/security-sdk/Cargo.toml`: Add `ego-domain` dependency
- [x] 1.2 `crates/security-sdk/src/authentication/mod.rs`: Rewrite `AuthenticationProvider` as sync trait returning `Result<SecurityContext, AuthenticationError>`. Remove `async_trait` and `#[async_trait]`. Update doc comments
- [x] 1.3 `crates/security-sdk/src/lib.rs`: Re-export `AuthenticationError` from `domain::auth`
- [x] 1.4 `crates/domain/src/auth/provider.rs`: Delete file
- [x] 1.5 `crates/domain/src/auth/mod.rs`: Remove `pub mod provider` and `pub use provider::AuthenticationProvider`
- [x] 1.6 `cargo test --workspace`: Verify compilation after trait removal

## Phase 2 — Identity Unification

- [x] 2.1 `crates/security-sdk/src/principal/principal.rs`: Add `tenant_id: Option<String>` field and `with_tenant_id()` builder. Remove `claims: Vec<Claim>` and `with_claim()`. Update tests
- [x] 2.2 `crates/domain/src/auth/identity.rs`: Delete file
- [x] 2.3 `crates/domain/src/auth/mod.rs`: Remove `pub mod identity` and `pub use identity::Identity`
- [x] 2.4 `cargo test --workspace`: Verify compilation after Identity removal

## Phase 3 — SecurityContext Unification

- [x] 3.1 `crates/domain/src/auth/security_context.rs`: Delete file
- [x] 3.2 `crates/domain/src/auth/mod.rs`: Remove `pub mod security_context` and re-export. Keep only `error`, `credential`, `claims`, `clock`
- [x] 3.3 `crates/security-sdk/src/context/mod.rs`: Add `claims: Claims` field to `SecurityContext`. Constructor: `new(principal, claims)`. Remove `scope` field. Add `claims()` accessor. Re-export `Claims`/`StandardClaims` from domain. Update tests
- [x] 3.4 `cargo test --workspace`: Verify compilation after SecurityContext unification

## Phase 4 — Runtime Wiring

- [x] 4.1 `crates/security-sdk/src/providers/basic/mod.rs`: Make `AuthenticationProvider` impl sync. Return `SecurityContext(principal, Claims::empty())`. Make `CredentialVerifier` sync. Update tests
- [x] 4.2 `crates/security-jwt/Cargo.toml`: Add `ego-security-sdk` dependency
- [x] 4.3 `crates/security-jwt/src/authenticator.rs`: Change import from `ego_domain::auth::AuthenticationProvider` to `ego_security_sdk::AuthenticationProvider`. Map `Identity` fields → `Principal.{subject_id, tenant_id, roles}`. Build `SecurityContext(principal, claims)`. Remove `Identity` usage. Replace `ctx.identity.*` with `ctx.principal().*` in tests. Update doc references
- [x] 4.4 `crates/service-sdk/src/runtime/builder.rs`: Verify stub tests compile with new `SecurityContext` signature. Update if needed
- [x] 4.5 `cargo test --workspace`: Full pipeline verification

## Phase 5 — Cleanup

- [x] 5.1 Grep `crates/domain/` for `SecurityContext`, `Identity`, `AuthenticationProvider` — zero matches in source (doc references to archived ADs OK)
- [x] 5.2 Grep `crates/security-jwt/` for `Identity`, `domain::auth::SecurityContext` — zero references
- [x] 5.3 Grep `crates/` for `SecurityContext.scope` or bare `scope` field usage — zero references outside authorization tests
- [x] 5.4 Grep `crates/security-sdk/` for `async_trait` in authentication — only in authorization
- [x] 5.4 `cargo clippy --workspace`: Zero new warnings (pre-existing only)
- [x] 5.5 `cargo test --workspace`: Final green
