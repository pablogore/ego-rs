# Proposal: CORE-028D1 — Effect Store Duplicate Registration Guard

## Intent

Every `AppBuilder` registration method fails closed on duplicate registration
except the two PROD-002 (#339) added. Verified on `develop@HEAD`:

| Method | Location | Behavior on second call |
|--------|----------|-------------------------|
| `.adapter()` | `app/mod.rs:305` | latches `CompositionError::DuplicateAdapter` |
| `.projection()` | `app/mod.rs:352` | latches `CompositionError::Projection` |
| `.entity()` | `app/mod.rs:374` | latches `CompositionError::Entity` |
| `.effect_executor()` | `app/mod.rs:560` | latches `CompositionError::EffectExecutor` |
| `.effect_store()` | `app/mod.rs:533` | **silent overwrite, no diagnostic** |
| `.effect_retention_store()` | `app/mod.rs:546` | **silent overwrite, no diagnostic** |

Both delegate straight into `RuntimeBuilder` (`runtime/builder.rs:501`, `:468`),
which does a plain `Option` assignment (`self.effect_state_store = Some(store)`).
The composition-error diagnostics built in Stage 1/2A/2C never reach this path.

Second defect in the same two methods: they alone lack the
`if self.pending_error.is_some() { return self; }` short-circuit, so they keep
mutating the builder after an earlier composition error latched.

## Verified: facade-only is sufficient

`AppBuilder` already owns dup-detection state (`adapter_types: HashSet<TypeId>`,
`app/mod.rs:120`) and guards `.adapter()` with it **without touching
`RuntimeBuilder`**. The same holds here. **Zero `RuntimeBuilder` code change is
confirmed achievable.**

## Scope

### In Scope
- Duplicate guard on `.effect_store()` and `.effect_retention_store()`, latched
  in `pending_error`, surfaced at `.build()`.
- The missing `pending_error` short-circuit on both.
- New typed `CompositionError` variant(s) for these paths.
- Tests mirroring `duplicate_adapter_registration_is_rejected` (`app/mod.rs:819`),
  plus triangulation that the two stores stay independent of each other.

### Out of Scope
- Any `RuntimeBuilder` behavior change.
- COOKBOOK.md table relocation (DOC-028D1).
- Projection lifecycle ownership (CORE-028D2).
- Module/bundle composition (CORE-028D3).
- Stage 2D public-API audit and Stage 2 archive.
- Any `replace_effect_store` escape hatch — none requested.

## Capabilities

### New Capabilities
- None

### Modified Capabilities
- `application-composition`: two fail-closed duplicate-registration requirements
  mirroring the existing "Duplicate Projection/Entity Registration Through
  AppBuilder Fails Closed" (`spec.md:330`, `:358`).

## Approach

Reuse the `.adapter()` shape verbatim: track registration in `AppBuilder`, latch
the error on the second call, return `self` unmodified, surface at `.build()`.
State shape follows each signature — `.effect_store<T>` is generic so the
concrete type name is available for the diagnostic; `.effect_retention_store`
takes `Arc<dyn RetentionMaintenance>` with no type identity, so a plain flag.
No new abstraction: this guard already exists three times in the file.

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `crates/service-sdk/src/app/mod.rs` | Modified | Guard + short-circuit on both methods; new tests |
| `crates/service-sdk/src/app/error.rs` | Modified | New `CompositionError` variant(s) + round-trip tests |
| `crates/service-sdk/src/runtime/builder.rs` | Unchanged | Explicitly untouched |

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| New `CompositionError` variant breaks downstream exhaustive matches (enum is not `#[non_exhaustive]`) | Low | Same precedent as Stage 2C's `Entity` variant; pre-1.0 crate |
| Guard breaks the legitimate `.effect_store(s.clone()).effect_retention_store(s)` pairing documented at `app/mod.rs:544` | Low | Independent per-method state; triangulation test pins it |
| `RuntimeBuilder`-level overwrite stays unspecified | Low | See open question below; zero-code either way |

## Open Question

No spec governs `with_effect_store` duplication today, unlike `with_adapter`'s
explicitly-specified last-write-wins (`service-sdk/spec.md:511`). Facade-only
leaves the `RuntimeBuilder` level unspecified. Design decides: leave it, or
document it as explicit last-write-wins. Both are zero-code at `RuntimeBuilder`.

## Rollback Plan

Revert the single commit. The guard is additive: no persisted state, no wire
format, no signature change. Pre-change hosts that never double-register are
byte-identical in behavior.

## Dependencies

- PROD-002 (#339, #340) — merged; introduced both methods.

## Success Criteria

- [ ] First `.effect_store()` registration succeeds and resolves as before.
- [ ] Second `.effect_store()` call fails at `.build()` with a typed error naming
      the duplicated store.
- [ ] Same for `.effect_retention_store()`.
- [ ] The `.effect_store(s.clone()).effect_retention_store(s)` pairing still works.
- [ ] Both methods short-circuit on a pre-existing `pending_error`.
- [ ] `cargo test --workspace` green; `crates/service-sdk/src/runtime/builder.rs`
      shows zero diff.
