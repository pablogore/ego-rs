# Apply Progress — jwt-authentication-provider

**Change**: jwt-authentication-provider (CORE-011)
**Mode**: Strict TDD (RED → GREEN → REFACTOR)
**Status**: All tasks complete

## TDD Cycle Evidence

| Task | RED (tests written first) | GREEN (impl passes) | REFACTOR |
|------|--------------------------|---------------------|----------|
| TASK-001: Domain auth module | Tests in each file written before module was declared in lib.rs | `cargo test -p ego-domain` → 151 passed | Fixed unreachable_patterns warning |
| TASK-002: security-jwt crate | 26 tests written in authenticator.rs covering all spec scenarios | `cargo test -p security-jwt` → 26 passed | Removed unused helper, fixed mut warning |
| TASK-003: Workspace update | N/A (config change) | Build succeeds | — |
| TASK-004: layers.toml | N/A (config change) | Layer constraint documented | — |
| TASK-005: Final verification | N/A | `cargo test --workspace` → all passed | — |

## Completed Tasks

- [x] TASK-001: Domain auth module (`crates/domain/src/auth/`)
- [x] TASK-002: Create security-jwt crate (`crates/security-jwt/`)
- [x] TASK-003: Update workspace Cargo.toml
- [x] TASK-004: Update layers.toml
- [x] TASK-005: Final verification — cargo test --workspace passes

## Files Created

| File | Action |
|------|--------|
| `crates/domain/src/auth/mod.rs` | Created |
| `crates/domain/src/auth/error.rs` | Created |
| `crates/domain/src/auth/credential.rs` | Created |
| `crates/domain/src/auth/identity.rs` | Created |
| `crates/domain/src/auth/clock.rs` | Created |
| `crates/domain/src/auth/claims.rs` | Created |
| `crates/domain/src/auth/security_context.rs` | Created |
| `crates/domain/src/auth/provider.rs` | Created |
| `crates/domain/src/lib.rs` | Modified (added `pub mod auth` + re-exports) |
| `crates/security-jwt/Cargo.toml` | Created |
| `crates/security-jwt/src/lib.rs` | Created |
| `crates/security-jwt/src/config.rs` | Created |
| `crates/security-jwt/src/authenticator.rs` | Created |
| `crates/security-jwt/tests/fixtures/test_rsa_private.pem` | Created |
| `crates/security-jwt/tests/fixtures/test_rsa_public.pem` | Created |
| `crates/security-jwt/tests/fixtures/test_rsa_other_private.pem` | Created |
| `crates/security-jwt/tests/fixtures/test_rsa_other_public.pem` | Created |
| `Cargo.toml` | Modified (added security-jwt workspace member) |
| `layers.toml` | Modified (added security-jwt = infrastructure) |

## Test Results

- ego-domain: **151 unit tests** + 9 doc tests — all pass
- security-jwt: **26 unit tests** + 1 doc test — all pass
- Full workspace: **all pass**, 0 failures

## Deviations from Design

None — implementation matches spec exactly.

## Spec Compliance

- FR-001: Identity with BTreeSet/BTreeMap — implemented
- FR-002: StandardClaims + Claims separation — implemented
- FR-003: SecurityContext concrete struct (Clone/Debug/PartialEq) — implemented
- FR-004: Credential #[non_exhaustive] BearerToken — implemented
- FR-004 + CLAR-001: AuthenticationProvider: Send + Sync, sync fn, Credential by value — implemented
- FR-005: JwtAuthenticator with JwtConfig + Arc<dyn Clock> — implemented
- FR-006: HS256 — implemented and tested
- FR-007: RS256 — implemented and tested
- FR-008: exp check (exp <= now → ExpiredToken) — implemented and tested
- FR-009: Clock trait Send + Sync — implemented
- FR-010: BTreeMap everywhere, no HashMap in public API — implemented and tested
- FR-011: nbf check — implemented and tested
- FR-012: Credential #[non_exhaustive] — implemented
- FR-013: AuthenticationError with context — implemented
- FR-014: JwtConfig/JwtAlgorithm — implemented
- CLAR-003: Graceful degradation for wrong-type claims — implemented and tested (sub, roles, tenant_id)
- CLAR-004: No ego-security-sdk import — verified (Cargo.toml has no such dep)
- NFR-001: layers.toml updated, security-jwt = infrastructure — done
- NFR-002: All public types Send + Sync — implemented (trait bounds + Arc usage)
- NFR-003: #![deny(missing_docs)] in security-jwt — implemented
- NFR-004: Injectable Clock in all time-sensitive tests — implemented (FixedClock)
