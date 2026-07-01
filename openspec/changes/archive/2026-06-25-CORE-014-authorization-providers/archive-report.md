# Archive Report: CORE-014 Built-in Authorization Providers

**Change**: CORE-014-authorization-providers
**Project**: ego-rs
**Archived**: 2026-07-01
**Verdict**: PASS — All phases complete, no CRITICAL issues, ready for closure

## Artifact Lineage

| Artifact | Topic Key | Engram ID | Status |
|----------|-----------|-----------|--------|
| Proposal | sdd/CORE-014-authorization-providers/proposal | 984 | COMPLETE |
| Spec | sdd/CORE-014-authorization-providers/spec | 986 | COMPLETE |
| Design | sdd/CORE-014-authorization-providers/design | 987 | COMPLETE |
| Tasks | sdd/CORE-014-authorization-providers/tasks | 988 | COMPLETE |
| Apply Progress | sdd/CORE-014-authorization-providers/apply-progress | 989 | COMPLETE |
| Verify Report | sdd/CORE-014-authorization-providers/verify-report | 991 | PASS (0 CRITICAL, 0 WARNING, 0 SUGGESTION) |
| Archive Report | sdd/CORE-014-authorization-providers/archive-report | 992 | COMPLETE |

## Scope Delivery

**Accomplished:**
- Built-in reference implementations: `AllowAllAuthorizationProvider` and `DenyAllAuthorizationProvider`
- Both public, documented, Send + Sync, Arc<dyn AuthorizationProvider>-injectable
- Removed private test stubs (`AlwaysAllow`/`AlwaysDeny`)
- Crate-root re-exports alongside `RbacProvider` (FR-019)
- Zero missing-docs warnings (FR-020)
- Strict TDD: RED → GREEN → REFACTOR for each provider
- All 79 security-sdk tests passing (0 regressions)

**Not In Scope (deferred correctly):**
- `#[authorize("resource:action")]` proc-macro → CORE-015
- ABAC, ReBAC, OpenFGA, SpiceDB → future Level-2/3 crates
- Composite provider → reserved for future change

## Files Changed

| File | Action |
|------|--------|
| `crates/security-sdk/src/providers/allow_all/mod.rs` | Created |
| `crates/security-sdk/src/providers/deny_all/mod.rs` | Created |
| `crates/security-sdk/src/providers/mod.rs` | Modified (pub mod + pub use) |
| `crates/security-sdk/src/lib.rs` | Modified (crate-root re-exports) |
| `crates/security-sdk/src/authorization/mod.rs` | Modified (removed private stubs) |

## Verification Summary

**Build/Test Evidence:**
- `cargo test --workspace`: PASS (79 security-sdk tests, 0 failures)
- `cargo doc --no-deps | grep warning`: PASS (0 warnings)
- Private stubs removal: PASS (rg clean)

**Spec Compliance:**
- FR-017 (AllowAll): PASS (public struct, AuthorizationProvider impl, Send + Sync, Arc-injectable)
- FR-018 (DenyAll): PASS (public struct, AuthorizationProvider impl returning Deny { reason: "deny-all" }, Send + Sync, Arc-injectable)
- FR-019 (Re-exports): PASS (available at crate root alongside RbacProvider)
- FR-020 (Documentation): PASS (missing_docs compliant, zero warnings)

## Known Decisions

- Single PR delivery: change well under 400-line budget (160 estimated lines across 5 files)
- Module layout: directory modules (`allow_all/`, `deny_all/`) match existing convention (`basic/`, `rbac/`)
- Test location: inline `#[cfg(test)] mod tests` per existing pattern in `basic/mod.rs` and `rbac/mod.rs`
- TS-019 assertion: uses `crate::` path (equivalent to crate-root reachability within the crate)

## Issues Encountered and Resolved

1. **Branch naming**: Pre-commit hook rejected `feat/CORE-014-*`. Resolved with `opsx/CORE-014-authorization-providers`.
2. **Stale module doc**: DenyAll reference appeared in doc before module existed. Fixed in apply phase.
3. **Orphaned TS-019 comment**: Deferred test body until Phase 6. Fixed when Phase 6 wiring completed.

## Migration / Rollback

- **No breaking changes**: Purely additive (two new modules + re-exports) plus removal of test-only stubs.
- **Rollback**: Revert the commit; no public API outside the new exports, so prior `cargo test --workspace` green state is restored immediately.

## Next Phase

None. Change is archived and closed. All success criteria met. The authorization provider ecosystem is now ready for:
- CORE-015: `#[authorize("resource:action")]` proc-macro
- Future Level-2/3 implementations (ABAC, ReBAC, external integrations)

**Closure**: This change is complete and ready for production use.
