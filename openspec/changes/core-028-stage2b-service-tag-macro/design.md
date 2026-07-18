# Design: CORE-028 Stage 2B — Service→Tag Macro Link (`.service::<S>()`)

## Technical Approach

Give the struct `#[service]` macro an optional `impl_of = Trait` argument. When
present it generates one marker impl (`HasServiceTag`) carrying the associated
`Tag` type plus a **concrete** `Arc<Self> → Arc<dyn Trait>` coercion — the macro
writes `dyn Trait` literally at expansion time, which is what dissolves the
`E0405` blocker (a generic `S: Tag::Service` bound is invalid Rust; a concrete
unsize coercion is ordinary). `AppBuilder` gains `service::<S>()` bounded on that
marker; the old two-generic + closure form is renamed `service_with_tag`. Bare
`#[service]` structs and `RuntimeBuilder::with_service` are untouched.

## Architecture Decisions

### Decision: Marker trait shape — one trait, `HasServiceTag`
**Choice**: A single trait in `crates/service-sdk/src/runtime/resolvable.rs`
(re-exported via `runtime`), sibling to `Resolvable`:
```rust
pub trait HasServiceTag: 'static {
    type Tag: Resolvable + 'static;
    fn into_service(self: Arc<Self>) -> Arc<<Self::Tag as Resolvable>::Service>;
}
```
Generated impl for `#[service(impl_of = GreetingService)] struct GreetingServiceImpl`:
```rust
impl ego_service_sdk::runtime::HasServiceTag for GreetingServiceImpl {
    type Tag = GreetingServiceTag;                       // {Trait}Tag
    fn into_service(self: Arc<Self>) -> Arc<dyn GreetingService> { self }
}
```
`Arc<dyn GreetingService>` is the same type as `Arc<<GreetingServiceTag as
Resolvable>::Service>` (the trait macro sets `type Service = dyn GreetingService`),
so the impl signature matches after projection; `self` coerces concretely.
**Alternatives**: two traits (Tag link + separate coercion) — rejected, the
associated type and coercion always travel together; naming inference — rejected
by AD-4. **Rationale**: single associated type + one method is the minimal
surface; no `Injectable` supertrait (kept on the method bound) so the marker
stays coupling-free and 2C-safe.

### Decision: New/renamed method signatures on `AppBuilder`
**Choice**:
```rust
pub fn service<S>(mut self) -> Self
where S: Injectable + HasServiceTag + 'static { /* validate→build→into_service→with_service::<S::Tag> */ }

pub fn service_with_tag<S, Tag>(mut self, to_trait_object: fn(Arc<S>) -> Arc<Tag::Service>) -> Self
where Tag: Resolvable + 'static, S: Injectable + 'static { /* today's body, verbatim */ }
```
The new method reuses the exact `service_registrars` + scratch-runtime path;
only the closure call becomes `S::into_service(Arc::new(instance))` and the tag
becomes `S::Tag`. **Alternatives**: keep old name, new form secondary — rejected
by AD-1. **Rationale**: Rust forbids same-name inherent methods of differing
arity; `service_with_tag` is the permanent hand-rolled-`Injectable` path.

### Decision: `ServiceArgs` parsing
**Choice**: Extend `ServiceArgs` with `impl_of: Option<syn::Path>`, parsed as
comma-separated `key = value` (`version` and/or `impl_of`). Pass `service_args`
into `expand_service_struct` (today it receives nothing — `lib.rs:66`). Tag ident
= final path segment + `Tag` via `format_ident!`. **Rationale**: reuses the
channel already parsed-then-discarded for structs; matches the explicit-argument
convention (`version = "..."`).

### Decision: Wrong `impl_of` diagnostic — deferred (AD-2)
**Choice**: No extra static assertion. The generated `into_service` body (`self`)
already fails at compile time with "the trait `Trait` is not implemented for `S`"
when the struct doesn't implement the named trait — it names both types and is
near-zero-cost inherent. An added `const _: fn()` assert would be redundant.

**Note**: this deferral is scoped to `impl_of` naming the wrong trait on a
*struct* annotation. `impl_of` written on a *trait* annotation instead of a
struct annotation is a distinct misuse (the argument has no receiver to attach
to) and is rejected with an explicit spanned macro error in
`expand_service_trait`, not left to `E0277` — see
`tests/compile_fail/service_impl_of_on_trait.rs`.

## Data Flow

    #[service(impl_of=T)] struct S  ──macro──▶  impl Injectable + impl HasServiceTag<Tag={T}Tag>
                                                        │
    App::builder().service::<S>()  ──registrar──▶ S::validate → S::build → S::into_service
                                                        │
                                              builder.with_service::<S::Tag>(Arc<dyn T>)

## File Changes

| File | Action | Description |
|------|--------|-------------|
| `crates/service-sdk-macros/src/lib.rs` | Modify | `ServiceArgs.impl_of`; pass args to `expand_service_struct`; emit `HasServiceTag` impl |
| `crates/service-sdk/src/runtime/resolvable.rs` | Modify | Add `HasServiceTag` trait |
| `crates/service-sdk/src/runtime/mod.rs` | Modify | Re-export `HasServiceTag` |
| `crates/service-sdk/src/app/mod.rs` | Modify | New `service::<S>()`; rename to `service_with_tag` |
| `crates/service-sdk/tests/app_composition.rs` | Modify | 4 sites → `service_with_tag`; add `impl_of` + `.service::<S>()` scenario |
| `crates/service-sdk/tests/compile_fail/service_without_tag.rs` (+`.stderr`) | Create | Unlinked `S` fails `E0277` |
| `crates/service-sdk/tests/service_tag_codegen.rs` | Create | trybuild driver for the fixture |
| `crates/service-sdk-macros/src/tests.rs` | Modify | Unit-test `impl_of` parsing |
| COOKBOOK/README/PRD/ARCHITECTURE | Modify | Illustrative snippets |

## Testing Strategy

| Layer | What | Approach |
|-------|------|----------|
| Unit | `ServiceArgs` parses `impl_of`, `version`, both, neither | `#[cfg(test)]` in `service-sdk-macros/src/tests.rs` |
| Integration | `#[service(impl_of=T)]` struct registers via `.service::<S>()` and resolves identically; bare `#[service]` struct unchanged; 4 renamed sites still pass | `tests/app_composition.rs` (macro output only resolves from `tests/`) |
| Compile-fail | `.service::<S>()` with an `S` lacking `HasServiceTag` → `E0277` at call site | Existing **trybuild** (`Cargo.toml:36`), new fixture + `.stderr` |

Strict TDD: write the RED behavioral test and the trybuild fixture first (both
fail — method/trait absent), then implement macro → trait → method. No new
dev-dependency: trybuild is already present with a working `compile_fail/` harness.

## Threat Matrix

N/A — no routing, shell, subprocess, VCS/PR automation, executable-file
classification, or process-integration boundary. Pure library API + proc-macro codegen.

## Migration / Rollout

One atomic in-repo rename (pre-1.0, no external callers). Macro-annotated impl
structs adopt `impl_of` + `.service::<S>()`; hand-rolled `Injectable` structs use
`service_with_tag` permanently. No data/runtime migration.

## Open Questions

- None blocking.
