# Archive Report: CORE-013 — JWT Providers + KeyResolver

## Status
ARCHIVED — 2026-06-25

## Verification Verdict
PASS WITH WARNINGS (0 CRITICALs, 2 WARNINGs, 1 SUGGESTION)

## Change Summary
Replaced the monolithic `JwtAuthenticator`/`JwtConfig` with three single-algorithm
`AuthenticationProvider` implementations (`Hs256AuthenticationProvider`,
`Rs256AuthenticationProvider`, `Es256AuthenticationProvider`) over the existing
`KeyResolver` abstraction. Extracted shared claim/time validation into an internal
`JwtValidationEngine`. Added `JwtAlgorithm::Es256` and `VerificationKey::EcPem` variants.

## Artifacts (Engram Observation IDs)

| Artifact | Engram ID |
|----------|-----------|
| Proposal | #970 |
| Design | #972 |
| Spec | #973 |
| Tasks | #974 |
| Apply-progress Phase 0-1 | #976 |
| Apply-progress Phase 2-5 | #977 |
| Verify-report | #979 |

## Main Spec Sync

Updated: `openspec/specs/domain/auth.md`
- Infrastructure Implementation Reference section: replaced JwtAuthenticator/JwtConfig with
  the three single-algorithm providers and JwtValidationEngine
- JwtAlgorithm: now includes Es256
- VerificationKey: now includes EcPem(String)
- Future Capabilities: removed ES256 (delivered), added CORE-014 and EdDSA as future
- CLAR-005 scenarios: replaced `JwtAuthenticator::authenticate` with `AuthenticationProvider::authenticate`
- References: added CORE-013 entry

## Warnings (Non-Blocking, Safe to Archive)

### WARNING-1: Provider module is authenticator.rs, not providers.rs
Design said create `src/providers.rs`. Actual: three providers implemented inside
`src/authenticator.rs`. Impact: zero — spec says nothing about module name.
All types are correctly exported. Public API is identical.

### WARNING-2: XxxConfig are type aliases, not distinct structs
Design says "Plain structs". Actual: `pub type Hs256Config = JwtProviderConfig` (etc.).
Type aliases are importable and usable exactly as structs at call sites.
Impact: minimal. Safe to archive. A future change may convert to newtypes if needed.

## Test Evidence
- `cargo test --workspace`: PASS — 0 failures, 0 errors
- `cargo test -p security-jwt`: 66 unit tests + 1 doc-test PASS
- `rg "JwtAuthenticator|JwtConfig" crates/security-jwt/src/`: 0 hits
- All FR-019–FR-027 and NFR-013-01–NFR-013-04 requirements: PASS

## Decisions Absorbed
AD-008, AD-009, AD-010, AD-011, AD-012, AD-013, AD-014, AD-015, AD-016, AD-017, AD-018, AD-019
