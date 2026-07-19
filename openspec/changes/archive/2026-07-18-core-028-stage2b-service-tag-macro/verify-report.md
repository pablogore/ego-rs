# Verify Report: CORE-028 Stage 2B — Service→Tag Macro Link

**Verdict**: PASS WITH WARNINGS

## Scope Verified

- PR1 `opsx/core-028-stage2b-pr1-service-tag-trait` (#192) → `develop`, tip `ffb443d` (includes an undocumented follow-up fix commit, see Warning 1).
- PR2 `opsx/core-028-stage2b-pr2-appbuilder-service` (#193), stacked on PR1, tip `3aa70f1`.
- Tested both individually and as a merged combined tree (PR1+fix+PR2) in an isolated worktree — both configurations compile and pass.

## Task Completion (tasks.md)

All 20 checkboxes across Phases 1-4 are checked, **plus one item (2.8) added after the reviewed apply-progress artifact was written** — see Warning 1. Spot-checked against the diff; every checked item has corresponding code:

| Task | Evidence |
|---|---|
| 1.1/1.2 | `HasServiceTag` trait in `resolvable.rs`, re-exported in `runtime/mod.rs` |
| 2.1 | 6 unit tests in `service-sdk-macros/src/tests.rs` (bare ident, path-qualified, combined, order-independent, tag-derivation) |
| 2.2 | `compile_fail/service_without_tag.rs` + `.stderr` |
| 2.3/2.4 | `ServiceArgs.impl_of: Option<syn::Path>`, comma-separated parse loop, `tag_path_from_impl_of` helper, codegen in `expand_service_struct` |
| 2.5 | `.stderr` files present, driven by `service_tag_codegen.rs` |
| 2.6 | `compile_pass/service_impl_of_with_version.rs` (version+impl_of combined, real macro expansion) |
| 2.7 | `compile_fail/service_wrong_impl_of.rs` (+`.stderr`) |
| 2.8 (added) | `compile_fail/service_impl_of_on_trait.rs` (+`.stderr`) + spanned-error guard in `expand_service_trait` |
| 3.1-3.5 | `app_composition.rs` new `macro_linked_service_registers_with_single_type_parameter_and_resolves_identically` test; `AppBuilder::service_with_tag` rename; new `AppBuilder::service<S>()`; 4 call sites migrated |
| 4.1-4.3 | Confirmed no-op (grepped, no `App::builder`/`AppBuilder` doc content exists anywhere in the repo) |

## Runtime Test Evidence

- `cargo test -p ego-service-sdk-macros -p ego-service-sdk`: all green (171 macro-crate unit tests unaffected + all service-sdk integration suites, including `app_composition.rs` 7/7 and `service_tag_codegen.rs` 2/2 trybuild tests).
- `cargo test --workspace`: green, no regressions elsewhere.
- Re-ran the full suite in a detached worktree with PR2 merged on top of PR1's latest tip (including the 2.8 fix commit not yet incorporated into PR2's base) — merges cleanly, all green. This confirms the eventual sequential-merge state (develop → PR1 → PR2) is sound even though PR2's branch was not rebased after PR1's follow-up commit.
- `cargo clippy -p ego-service-sdk -p ego-service-sdk-macros --all-targets`: only pre-existing, unrelated warnings (no new lints from this change's code).

## Spec Compliance Matrix

| Scenario (delta spec) | Covering test | Status |
|---|---|---|
| Bare `#[service]` struct unaffected | existing testkit/reference-app bare-`#[service]` usage compiles unchanged; no new required argument | PASS |
| Macro-linked service registers with single type param, no closure | `app_composition.rs::macro_linked_service_registers_with_single_type_parameter_and_resolves_identically` | PASS |
| Unlinked S fails to compile against macro-linked call | `compile_fail/service_without_tag.rs` | PASS |
| Hand-rolled Injectable registers via renamed explicit-Tag form | `app_composition.rs` 4 sites now use `service_with_tag::<S,Tag>(closure)` | PASS |
| `impl_of` generates usable trait link | `compile_pass/service_impl_of_with_version.rs` | PASS |
| Wrong `impl_of` target fails to compile | `compile_fail/service_wrong_impl_of.rs` | PASS |
| (unspecified by delta spec, caught during implementation) `impl_of` on a trait annotation is rejected, not silently ignored | `compile_fail/service_impl_of_on_trait.rs` | PASS — genuine gap-fix, correctly out-of-spec-scope but consistent with AD-2's explicit-argument intent |

## Design Coherence

- `HasServiceTag` trait shape matches design.md verbatim (assoc `Tag: Resolvable + 'static`, `into_service(self: Arc<Self>) -> Arc<<Self::Tag as Resolvable>::Service>`).
- Generated `into_service` body writes `Arc<dyn #trait_path>` **literally** (confirmed in `lib.rs` diff), not the associated-type-projected form — matches design.md's stated rationale that this is what makes it an ordinary concrete unsize coercion rather than an invalid `S: Tag::Service` generic bound (E0405).
- `ServiceArgs` parsing matches design.md: comma-separated `key=value`, `impl_of` as `syn::Path`, Tag ident = final path segment + `Tag`, module path preserved for the `dyn` reference — confirmed by both unit tests and the path-qualified fixture behavior.
- `AppBuilder::service<S>()` / `service_with_tag<S,Tag>()` signatures match design.md's sketch exactly.
- No `const _: fn()` static assertion was added for wrong-`impl_of` (per design.md's explicit deferral) — correct, matches design intent.

## Scope Creep Check

- No `.entity::<E>()` / CORE-006 touches anywhere in the diff (`rg ".entity::<"` — zero hits).
- No new `Cargo.toml` changes in either PR — `trybuild` dev-dependency was already present; nothing new added.
- `ServiceTestFixture::builder().service::<LimitServiceImpl>()` (testkit's unrelated same-named method) correctly left untouched, as tasks.md explicitly notes.
- `reference-app` untouched (grep confirms no diff in that crate) — proposal explicitly notes reference-app never calls `.service()`.

## Main-Spec Merge Investigation (explicit check requested)

`git diff --stat` against `develop` shows `openspec/specs/application-composition/spec.md` (+28) and `openspec/specs/service-sdk/spec.md` (+53) modified, plus renames of `openspec/changes/core-028-stage2/*` into `openspec/changes/archive/2026-07-18-core-028-stage2-projection-registration/*`.

**Finding: this is NOT premature archive-phase work done during this change's `sdd-apply`.** Isolating the diff to only the two commits that constitute Stage 2B (`afa448b..3aa70f1`, i.e. the actual PR1+PR2 payload) shows **zero** changes to `openspec/specs/**` or to `openspec/changes/core-028-stage2/**`. Those main-spec merges and the stage-2 archive rename come entirely from a separate, already-completed commit, `afa448b` ("chore(sdd): archive core-028 stage 2 slice 2A projection registration (#191)") — the archive step for the **prior** slice (Stage 2A), which Stage 2B's branches happen to be stacked on top of because `afa448b` has not yet been fast-forward-merged into `develop` at the time these branches were cut. This is base-branch lag noise in the `develop...branch` diff view, not new work by this change's apply/verify cycle. **No action needed on Stage 2B's account** — when Stage 2B is archived, its own delta specs (`specs/application-composition/spec.md`, `specs/service-sdk/spec.md` under this change's folder) still need to be merged into the main specs at that time, as normal; they have not been touched yet.

## Issues

### WARNING

1. **Apply-progress artifact (Engram id 1285) is stale relative to current branch state.** It documents PR1 (`3315312`) and PR2 (`3aa70f1`) but was saved before a third commit, `ffb443d` ("fix(service-sdk-macros): reject impl_of on a #[service] trait annotation"), was pushed to PR1's branch. `ffb443d` is a legitimate, well-tested gap-fix (task 2.8, added to tasks.md and design.md with rationale) and does not change the verdict, but the persisted apply-progress record undercounts the shipped work. Recommend either a supplementary apply-progress note or accepting tasks.md (which does list 2.8) as the source of truth at archive time.
2. **PR2's branch was not rebased onto PR1's post-2.8 tip.** The stacked PR2 (`3aa70f1`) still targets PR1 at `3315312`; GitHub will recompute PR2's diff against PR1's current tip automatically, and a manual merge test in this verification confirms no conflict and all green — but if PR1 is squash-merged rather than fast-forwarded, PR2's base may need an explicit rebase before its own merge. Not a defect in the code; a sequencing note for whoever merges the PRs.

### SUGGESTION

- None.

## Final Verdict

**PASS WITH WARNINGS** (2 warnings, 0 critical, 0 suggestions). No CRITICAL issues. Both warnings are process/documentation lag, not code or spec defects — safe to proceed to archive once the two PRs are actually merged (archive should still wait for merge, per standard SDD sequencing).
