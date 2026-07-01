# Apply Progress: oauth2-oidc — OIDC Resource Server Framework

**Status**: done (16/16 tasks complete)
**Mode**: Strict TDD (red → green → refactor throughout)

---

## Task Status

- [x] T-01: ClaimSet + ClaimValue domain value objects
- [x] T-02: PrincipalMapper trait in security-sdk
- [x] T-03: CredentialExtractor, RequestContext, built-in extractors
- [x] T-04: AuthenticationInterceptor in security-sdk
- [x] T-05: Cargo.toml — new dependencies + test-kit feature
- [x] T-06: OidcProviderConfig, TokenFormat, MultiIssuerConfig
- [x] T-07: JwksKeyResolver, JwksProvider, HttpJwksProvider
- [x] T-08: DiscoveryProvider, HttpDiscoveryProvider, OidcEndpoints
- [x] T-09: IntrospectionProvider, HttpIntrospectionProvider, IntrospectionAuthenticationProvider
- [x] T-10: DefaultPrincipalMapper
- [x] T-11: JwtValidationEngine: inject PrincipalMapper + with_mapper builder
- [x] T-12: OidcAuthenticationProvider
- [x] T-13: MultiIssuerAuthenticationProvider, IssuerResolver, StaticIssuerResolver
- [x] T-14: TestKit (FakeIssuer, FakeDiscovery, FakeJwks, FakeIntrospection)
- [x] T-15: Integration tests (US-001 through US-008)
- [x] T-16: CI guard — no test-kit symbols in release build

---

## Final Verification

```
cargo test --workspace --features test-kit  → all passing, 0 failures
cargo build --release --no-default-features -p security-jwt  → OK, 0 test-kit symbols
```

Security-jwt test count: 176 lib tests + 23 integration tests = 199 total (all pass).

---

## Files Produced

### New
- `crates/domain/src/auth/claim_set.rs`
- `crates/security-sdk/src/principal_mapper.rs`
- `crates/security-sdk/src/credential_extractor.rs`
- `crates/security-sdk/src/authentication/interceptor.rs`
- `crates/security-jwt/src/oidc_config.rs`
- `crates/security-jwt/src/jwks.rs`
- `crates/security-jwt/src/discovery.rs`
- `crates/security-jwt/src/introspection.rs`
- `crates/security-jwt/src/principal_mapper.rs`
- `crates/security-jwt/src/oidc_provider.rs`
- `crates/security-jwt/src/multi_issuer.rs`
- `crates/security-jwt/src/test_kit/mod.rs`
- `crates/security-jwt/tests/oidc_integration.rs`

### Modified
- `crates/domain/src/auth/mod.rs`
- `crates/domain/src/lib.rs`
- `crates/security-sdk/src/lib.rs`
- `crates/security-sdk/src/authentication/mod.rs`
- `crates/security-jwt/src/validation.rs`
- `crates/security-jwt/src/authenticator.rs`
- `crates/security-jwt/src/lib.rs`
- `crates/security-jwt/Cargo.toml`

---

## Notable Decisions

- `DefaultPrincipalMapper` selectively removes consumed claim keys from custom (not unconditional) — fixes graceful-degradation tests for wrong-type `roles`/`tid`.
- `sub` integer → `InvalidToken` (not `MissingClaim`) via direct raw map check before `as_str()`.
- `std::sync::RwLock` used throughout JWKS cache (not `tokio::sync::RwLock`) to avoid executor conflicts (futures_executor + tokio).
- `authenticate_inner` made `pub(crate)` so `OidcAuthenticationProvider` reuses the exact same JWT validation path as single-algorithm providers.
- `OidcAuthenticationProvider::with_resolver` made `pub` to enable integration-test injection without HTTP.
- `DiscoveryProvider` remains `pub(crate)` as specified in design.
