# Tasks: CORE-013 — JWT Providers + JwtValidationEngine

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | 900–1 100 (additions ~750, deletions ~400) |
| 400-line budget risk | High |
| Chained PRs recommended | Yes |
| Suggested split | PR 1 → Phase 0–1 · PR 2 → Phase 2–4 · PR 3 → Phase 5 |
| Delivery strategy | stacked-to-develop (each PR merges to develop in order) |
| Chain strategy | stacked-to-main |

### Suggested Work Units

| Unit | Goal | PR | Notes |
|------|------|----|-------|
| 1 | Fixtures + variants + JwtValidationEngine | PR 1 | Base = develop |
| 2 | Three AuthenticationProviders (HS256/RS256/ES256) | PR 2 | Base = develop (after PR 1 merged) |
| 3 | Remove JwtAuthenticator/JwtConfig + update exports | PR 3 | Base = develop (after PR 2 merged) |

---

## Phase 0 — Preparation (unblocks everything else)

- [ ] 0.1 Create `crates/security-jwt/tests/fixtures/` and commit RSA fixtures (`test_rsa_private.pem`, `test_rsa_public.pem`, `test_rsa_other_public.pem`) so `include_str!` in `authenticator.rs` compiles. **BLOCKER** — crate won't compile without these.
- [ ] 0.2 RED: add test `ec_pem_variant_stores_string` in `key_resolver.rs` asserting `VerificationKey::EcPem("...".into())` round-trips the PEM string — test must fail to compile (variant absent).
- [ ] 0.3 GREEN: add `EcPem(String)` variant to `VerificationKey` (`#[non_exhaustive]` additive) in `src/key_resolver.rs`; test passes. Update `LocalKeyResolver` to handle `Es256` algorithm resolving an `EcPem` key.
- [ ] 0.4 RED: add test `es256_variant_on_jwt_algorithm` asserting `JwtAlgorithm::Es256 == JwtAlgorithm::Es256` — fails to compile.
- [ ] 0.5 GREEN: add `Es256` variant to `JwtAlgorithm` in `src/config.rs`; test passes. Update `LocalKeyResolver` algorithm match.
- [ ] 0.6 Generate EC P-256 test fixtures with `openssl` and commit to `tests/fixtures/` (`test_ec_private.pem`, `test_ec_public.pem`, `test_ec_other_public.pem`). Add compile-only `include_str!` tests to pin them.

## Phase 1 — JwtValidationEngine (internal, `src/validation.rs`)

- [ ] 1.1 RED: add test module `validation_engine_tests` in new `src/validation.rs`. Write `engine_returns_security_context_for_valid_hs256_token` calling `JwtValidationEngine::validate(...)` — fails to compile (struct absent).
- [ ] 1.2 GREEN: create `src/validation.rs` with `pub(crate) struct ValidationParams<'a>`, `pub(crate) struct JwtValidationEngine`, and `pub(crate) fn validate(...)`. Move helpers verbatim from `authenticator.rs`: `RawClaims`, `build_standard_claims`, `extract_subject`, `extract_tenant_id`, `extract_roles`, `remove_standard_keys`, exp/nbf/iss/aud/sub/roles/tenant blocks. Test passes.
- [ ] 1.3 REFACTOR: port the full CLAR-005 claim-validation test matrix from `authenticator.rs` tests into `validation.rs` tests (via Hs256 provider stub or direct engine call). Cover all scenarios: exp, nbf, iss, aud, sub, roles, tenant_id/tid. `cargo test --workspace` green.
- [ ] 1.4 Add `mod validation;` to `src/lib.rs` (private, not exported).

## Phase 2 — Hs256AuthenticationProvider (`src/providers.rs`)

- [ ] 2.1 RED: in `src/providers.rs`, write `hs256_happy_path_returns_security_context` — fails to compile (`Hs256AuthenticationProvider` absent).
- [ ] 2.2 GREEN: add `pub struct Hs256Config { pub expected_iss: Option<String>, pub expected_aud: Option<Vec<String>> }` to `src/config.rs`. Implement `Hs256AuthenticationProvider` in `src/providers.rs`: `new(config, resolver, clock)`, `authenticate` — decode header, assert `alg == HS256`, `block_on(resolver.resolve(kid, Hs256))`, build `DecodingKey::from_secret`, delegate to `JwtValidationEngine::validate`. Test passes.
- [ ] 2.3 RED+GREEN: tests `hs256_wrong_secret_returns_invalid_signature`, `hs256_rejects_rs256_token` (AlgorithmNotSupported), `hs256_non_bearer_returns_invalid_token` (spec FR-021).
- [ ] 2.4 RED+GREEN: tests `hs256_key_not_found_maps_to_invalid_signature`, `hs256_algorithm_mismatch_maps_to_not_supported`, `hs256_invalid_key_material_maps_to_invalid_token` (spec FR-026). Use `FailingResolver` helper.
- [ ] 2.5 RED+GREEN: tests `hs256_kid_forwarded_to_resolver`, `hs256_no_kid_forwards_none` (spec FR-025). Use `CapturingResolver` helper.
- [ ] 2.6 Static assertion: `Hs256AuthenticationProvider: Send + Sync` via `Arc<dyn AuthenticationProvider>` binding (spec NFR-013-03).

## Phase 3 — Rs256AuthenticationProvider (`src/providers.rs`)

- [ ] 3.1 RED: write `rs256_happy_path_returns_security_context` — fails to compile.
- [ ] 3.2 GREEN: add `pub struct Rs256Config { ... }` to `src/config.rs`. Implement `Rs256AuthenticationProvider`: decode header, assert `alg == RS256`, `block_on` resolve, `DecodingKey::from_rsa_pem`, delegate to engine. Test passes.
- [ ] 3.3 RED+GREEN: tests `rs256_mismatched_key_returns_invalid_signature`, `rs256_rejects_hs256_token` (spec FR-022).
- [ ] 3.4 Static assertion: `Rs256AuthenticationProvider: Send + Sync` (spec NFR-013-03).

## Phase 4 — Es256AuthenticationProvider (`src/providers.rs`)

- [ ] 4.1 RED: write `es256_happy_path_returns_security_context` using EC P-256 fixtures — fails to compile.
- [ ] 4.2 GREEN: add `pub struct Es256Config { ... }` to `src/config.rs`. Implement `Es256AuthenticationProvider`: decode header, assert `alg == ES256`, `block_on` resolve, match `VerificationKey::EcPem(pem)` → `DecodingKey::from_ec_pem`, other variants → `InvalidToken` (spec FR-023), delegate to engine. Test passes.
- [ ] 4.3 RED+GREEN: tests `es256_invalid_signature_returns_invalid_signature`, `es256_rejects_hs256_token`, `es256_wrong_variant_hmac_key_returns_invalid_token` (spec FR-023).
- [ ] 4.4 Static assertion: `Es256AuthenticationProvider: Send + Sync` (spec NFR-013-03).

## Phase 5 — Migration + Cleanup

- [ ] 5.1 Rewrite `src/lib.rs`: expose three providers + three configs + `JwtAlgorithm` + `KeyResolver`, `KeyResolverError`, `LocalKeyResolver`, `VerificationKey`. Remove `JwtAuthenticator`, `JwtConfig` re-exports. Add `mod providers; mod validation;`. Update rustdoc example to use `Hs256AuthenticationProvider` (spec FR-027).
- [ ] 5.2 Delete `src/authenticator.rs`.
- [ ] 5.3 Drop `JwtConfig` from `src/config.rs`.
- [ ] 5.4 Run `cargo test --workspace` — all tests must pass.
- [ ] 5.5 Grep codebase for `JwtAuthenticator` and `JwtConfig` — both MUST return zero public-facing hits (spec FR-027). Record evidence.
- [ ] 5.6 Verify `cargo doc --no-deps -p security-jwt` compiles without `missing_docs` warnings.
