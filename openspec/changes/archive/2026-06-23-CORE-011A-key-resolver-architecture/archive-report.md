# Archive Report: CORE-011A — Key Resolver Architecture

**Archived**: 2026-06-23
**Change**: CORE-011A-key-resolver-architecture
**Store mode**: openspec
**Archive path**: `openspec/changes/archive/2026-06-23-CORE-011A-key-resolver-architecture/`

---

## Task Completion Gate

| Check | Result |
|-------|--------|
| All tasks marked `[x]` in tasks.md | ✅ 24/24 complete |
| CRITICAL issues in verify-report | ❌ None |
| Verify report verdict | ✅ PASS |

The task completion gate passed without exceptions. No stale checkboxes, no critical issues.

---

## Specs Synced

| Domain | Action | Details |
|--------|--------|---------|
| `domain/auth` | Updated | `openspec/specs/domain/auth.md` — Infrastructure Implementation Reference rewritten to document the new KeyResolver architecture. JwtAlgorithm marked as marker enum, JwtConfig stripped of key material, JwtAuthenticator::new signature updated, new types documented (KeyResolver, VerificationKey, LocalKeyResolver). Future Capabilities updated: CORE-011A line removed (now archived), CORE-011B and ES256/EdDSA future work retained. |

### Delta Spec Sections Applied

| Section | Items |
|---------|-------|
| **ADDED** | KeyResolverError enum, VerificationKey enum, KeyResolver trait, LocalKeyResolver struct, ErrorMapping |
| **MODIFIED** | JwtConfig (key material removed), JwtAuthenticator::new (resolver parameter added) |
| **REMOVED** | JwtAlgorithm key-material variants (Reason: moved to VerificationKey behind resolver; Migration: callers must use LocalKeyResolver) |

### Frozen Invariants Preserved

All invariants listed in the delta spec (AuthenticationProvider trait, Credential enum, SecurityContext/Identity/Claims types, AuthenticationError variants, Clock trait, layers.toml assignments) were verified unchanged.

---

## Archive Contents

| Artifact | Status |
|----------|--------|
| proposal.md | ✅ |
| spec.md (delta) | ✅ |
| design.md | ✅ |
| tasks.md | ✅ (24/24 tasks complete) |
| verify-report.md | ✅ (PASS) |
| state.yaml | ✅ |
| archive-report.md | ✅ (this file) |

---

## Source of Truth Updated

- `openspec/specs/domain/auth.md` — Infrastructure Implementation Reference now reflects the Key Resolver Architecture introduced by CORE-011A

---

## Intentional Deviations

None. Standard archive workflow executed.

---

## Risks

1. **Breaking change**: `JwtAlgorithm` variant change — key material removed from enum variants. External consumers require migration to `LocalKeyResolver`. As noted in verify-report.
2. **Module structure deviation**: Design specified all types in `key_resolver.rs`; implementation split into `key_resolver.rs`, `key_resolver_error.rs`, `verification_key.rs`, `local_key_resolver.rs`. This deviation was resolved during verify — all types consolidated back into `key_resolver.rs`. The archived spec and design may not reflect the final consolidated structure; the implementation is the source of truth.
