# Tasks: CORE-028 Stage 2B — Service→Tag Macro Link (`.service::<S>()`)

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | ~300-370 (authored; generated `.stderr` excluded) |
| 400-line budget risk | Medium |
| Chained PRs recommended | Yes |
| Suggested split | PR 1: macro/trait layer → PR 2: AppBuilder wiring + call-site migration + docs |
| Delivery strategy | ask-on-risk |
| Chain strategy | pending (ask user: feature-branch-chain, matching Stage 1's PR1→PR2 precedent, vs stacked-to-main) |

Decision needed before apply: Yes
Chained PRs recommended: Yes
Chain strategy: pending
400-line budget risk: Medium

### Suggested Work Units

| Unit | Goal | Likely PR | Focused test command | Runtime harness | Rollback boundary |
|------|------|-----------|----------------------|-----------------|-------------------|
| 1 | `HasServiceTag` trait + macro `impl_of` codegen + trybuild fixture, self-contained (no `AppBuilder` change) | PR 1 | `cargo test -p ego-service-sdk-macros` | trybuild fixture run (`cargo test -p ego-service-sdk --test service_tag_codegen`) | Revert lib.rs/resolvable.rs/mod.rs + delete new fixture files; no other code depends on it yet |
| 2 | `AppBuilder::service_with_tag` rename + new `service::<S>()`, 4 call-site migration, docs | PR 2 | `cargo test -p ego-service-sdk --test app_composition` | `cargo test -p ego-service-sdk --test app_composition` (real `App::builder()` build+resolve) | Revert app/mod.rs + app_composition.rs + docs; PR 1's trait/macro layer stays intact and unused |

## Phase 1: Marker Trait Foundation

- [x] 1.1 Add `HasServiceTag` trait to `crates/service-sdk/src/runtime/resolvable.rs` (assoc `Tag: Resolvable + 'static`, `fn into_service(self: Arc<Self>) -> Arc<<Self::Tag as Resolvable>::Service>`)
- [x] 1.2 Re-export `HasServiceTag` from `crates/service-sdk/src/runtime/mod.rs`

## Phase 2: Macro Codegen (RED → GREEN)

- [x] 2.1 RED: unit tests in `crates/service-sdk-macros/src/tests.rs` — `ServiceArgs` parses `impl_of = Trait` (bare ident) AND `impl_of = crate::foo::Trait` (path-qualified) AND `version` + `impl_of` combined; assert Tag ident derives from the **final path segment** (`FooTag`) while the trait reference itself preserves the full path
- [x] 2.2 RED: create `crates/service-sdk/tests/compile_fail/service_without_tag.rs` (struct w/o `impl_of` failing the `HasServiceTag` bound directly — self-contained per this PR's work-unit boundary, `AppBuilder::service::<S>()` itself lands in PR2) + driver `crates/service-sdk/tests/service_tag_codegen.rs`; confirmed RED before GREEN
- [x] 2.3 GREEN: extend `ServiceArgs` with `impl_of: Option<syn::Path>` in `crates/service-sdk-macros/src/lib.rs`, comma-separated `key = value` parsing
- [x] 2.4 GREEN: pass `service_args` into `expand_service_struct` (lib.rs:66); when `impl_of` present, emit `HasServiceTag` impl — handle path-qualified `impl_of` by appending `Tag` only to the last segment, preserving the module path in the `dyn <path>` reference
- [x] 2.5 GREEN: regenerate `service_without_tag.rs.stderr` (+ `service_wrong_impl_of.rs.stderr`) via `TRYBUILD=overwrite cargo test -p ego-service-sdk --test service_tag_codegen` — did not hand-author stderr text
- [x] 2.6 RED→GREEN: added trybuild PASS fixture `crates/service-sdk/tests/compile_pass/service_impl_of_with_version.rs` — `#[service(version = "1.0.0", impl_of = GreetingService)] struct GreetingServiceImpl;` — proves the real macro expansion (not just `ServiceArgs` parsing in 2.1) doesn't let the new `impl_of` codegen interfere with existing `version` semantics
- [x] 2.7 REFACTOR: confirmed 2.1 unit tests, 2.2 fail-fixture, and 2.6 pass-fixture all pass; added `service_wrong_impl_of.rs` fixture — `impl_of` naming a trait the struct doesn't implement fails with ordinary `E0277` (no custom diagnostic)
- [x] 2.8 GREEN: `#[service(impl_of = Trait)]` on a `trait` annotation (instead of `struct`) was silently discarded in `expand_service_trait` — added a spanned `syn::Error` guard at the top of `expand_service_trait` (lib.rs) plus `service_impl_of_on_trait.rs` compile_fail fixture; regenerated its `.stderr` via `TRYBUILD=overwrite`

## Phase 3: AppBuilder Wiring (RED → GREEN)

- [ ] 3.1 RED: add scenario to `crates/service-sdk/tests/app_composition.rs` using `#[service(impl_of = GreetingService)]` + `.service::<S>()`, expecting it to resolve identically to the two-generic form — fails to compile (method absent)
- [ ] 3.2 GREEN: rename `pub fn service<S, Tag>` at `crates/service-sdk/src/app/mod.rs:446` to `service_with_tag`
- [ ] 3.3 GREEN: add `pub fn service<S>(mut self) -> Self where S: Injectable + HasServiceTag + 'static`, reusing the registrar path (`S::into_service`, `S::Tag`)
- [ ] 3.4 GREEN: migrate the 4 existing `AppBuilder` two-generic call sites (app_composition.rs lines 68, 85, 134, 228) to `service_with_tag` — do NOT touch line 242 (`ServiceTestFixture::builder().service::<LimitServiceImpl>()`, a different builder, out of scope)
- [ ] 3.5 confirm 3.1's scenario now compiles and passes; run full `cargo test -p ego-service-sdk`

## Phase 4: Documentation

- [ ] 4.1 Update COOKBOOK illustrative snippet(s) to show `impl_of` + `.service::<S>()` as the primary form
- [ ] 4.2 Update README composition example if it shows the two-generic form
- [ ] 4.3 Update PRD/ARCHITECTURE illustrative snippets per the proposal's affected-areas table; note `service_with_tag` as the permanent hand-rolled-`Injectable` path
