# Proposal: CORE-028 Stage 2B — Service→Tag Macro Link (`.service::<S>()`)

> Second Stage 2 slice, after 2A (projection registration, shipped). Pays the
> DX debt Stage 1 AD-3 formally accepted: `.service::<S, Tag>(|arc| arc)`
> exists only because `#[service]` on a struct doesn't know which trait the
> struct implements. 2C (`.entity::<E>()`) stays blocked by CORE-006 —
> untouched here.

## Intent

Registering a service today requires naming two types and writing a coercion
closure that is always `|arc| arc`. The closure is not ergonomic noise — it
exists because `S: Injectable + Tag::Service` is not valid Rust (confirmed
`E0405`): the caller cannot express "S implements whatever trait underlies
this Tag" as a generic bound. Only macro-generated code, which knows the
concrete trait name at expansion time, can write that coercion. This slice
gives the struct macro that knowledge and collapses registration to
`.service::<S>()` — the end-state shape Stage 1 AD-3 already named.

## Architecture Decisions

### AD-1 — `.service::<S>()` takes the primary name; explicit-Tag form is renamed, kept indefinitely

Rust forbids two inherent methods named `service` with different generic
arity, so coexistence under one name is impossible. Decision: the new
single-generic form owns `.service::<S>()` (no closure — observable
contract: S is built via its existing `Injectable` contract and registered
under its macro-linked Tag). The current two-generic + closure form is
renamed (illustratively `.service_with_tag::<S, Tag>(closure)`; exact
identifier → design) and is a **permanent** public API, not a deprecation:
hand-rolled `Injectable` structs with no macro annotation can never derive a
Tag and remain supported. Rationale for giving the new form the primary
name: it is the recommended path and the shape Stage 1 recorded as the
target; the break is confined to 4 in-tree test call sites
(`crates/service-sdk/tests/app_composition.rs`) — reference-app never calls
`.service()`. Rejected alternative: keep the old name and give the new form
a secondary name — would freeze the worse name on the recommended path
forever to spare a one-file mechanical rename.

### AD-2 — Explicit macro argument `#[service(impl_of = Trait)]`, no inference

The struct macro gains an optional argument naming the implemented trait.
When present, it generates a marker link (`HasServiceTag`-style: Tag
association + a concrete `Arc<S> → Arc<dyn Trait>` coercion written inside
generated code, which is what dissolves the E0405 blocker). When absent,
behavior is exactly today's (Injectable only) — bare `#[service]` struct
usage, including testkit's, is unaffected. Rationale: matches the
workspace's explicit-argument convention (`version = "..."`); reuses the
channel where `ServiceArgs` is currently parsed-then-discarded for structs.
A wrong `impl_of` surfaces as a normal trait-not-implemented compile error,
not a pretty macro diagnostic — accepted; design may add a friendlier static
assertion.

### AD-3 — Migration story

- Macro-annotated impl structs: add `impl_of = Trait`, switch call site to
  `.service::<S>()`.
- Hand-rolled `Injectable` structs: switch to the renamed explicit-Tag
  method — supported forever, no migration deadline.
- No deprecation window machinery: one atomic in-repo rename (pre-1.0, no
  external callers).

### AD-4 — Explicit non-goals

- No `.entity::<E>()` / entity assumptions in the marker trait (2C,
  CORE-006-blocked).
- No runtime/link-time registry dependency (`inventory`/`linkme`/`ctor`) —
  keeps DI resolution synchronous, compile-time-shaped, dependency-free.
- No naming-convention inference (strip-`Impl` magic) — implicit, breaks
  silently, contradicts the explicit-argument convention.

## Scope

### In Scope
- Struct-macro `impl_of` argument generating the Tag link + concrete coercion.
- New `AppBuilder::service::<S>()`; rename of the two-generic form on
  `AppBuilder` (and its `RuntimeBuilder` delegation target if the same
  arity collision applies there — design confirms).
- Migrate the 4 `app_composition.rs` call sites; update illustrative docs
  (COOKBOOK, README, PRD, ARCHITECTURE) referencing the closure form.
- Compile-visible failure when `.service::<S>()` is called with an S that
  has no macro-generated Tag link (missing-bound error, not runtime).

### Out of Scope (non-goals)
- Everything in AD-4; any change to `Resolvable`, `DepKey`, projection or
  adapter registration; testkit's `ServiceTestFixture::service` (different
  mechanism, coincidental name).

## Capabilities

### New Capabilities
- None.

### Modified Capabilities
- `application-composition`: service registration gains the single-generic
  form; explicit-Tag form renamed, retained.
- `service-sdk`: `#[service]` struct contract gains the optional trait-link
  argument and the marker-trait guarantee it generates.

## Affected Areas

| Area | Impact | Description |
|---|---|---|
| `crates/service-sdk-macros/src/lib.rs` | Modified | `ServiceArgs` for structs; Tag-link codegen |
| `crates/service-sdk/src/app/mod.rs` | Modified | New `.service::<S>()`; rename two-generic form |
| `crates/service-sdk/src/runtime/resolvable.rs` (or sibling) | Modified/New | Marker-trait home |
| `crates/service-sdk/tests/app_composition.rs` | Modified | 4 call sites migrated/renamed |
| Docs (COOKBOOK/README/PRD/ARCHITECTURE) | Modified | Illustrative snippets |

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| Renaming `.service` breaks unknown callers | Low | Confirmed in-tree-only call sites; pre-1.0 |
| `impl_of` typo yields unfriendly compile error | Med | Documented; optional static assertion → design |
| Marker trait bakes in service-only assumption awkward for 2C | Low | Keep trait scoped to service Tags; AD-4 forbids entity coupling |

## Rollback Plan

Revert the macro argument and the new method; restore the original
`.service::<S, Tag>(closure)` name. Purely additive-plus-rename — no runtime,
DI-resolution, or stored-data changes to unwind.

## Dependencies

- None. (2C depends on CORE-006; this slice does not.)

## Success Criteria

- [ ] `#[service(impl_of = Trait)]` struct registers via `.service::<S>()`
      with no Tag parameter and no closure; resolves identically to the old
      form.
- [ ] Bare `#[service]` struct (no argument) compiles and behaves exactly as
      today.
- [ ] Hand-rolled `Injectable` structs register via the renamed explicit-Tag
      method; all 4 existing composition scenarios still pass.
- [ ] `.service::<S>()` with an unlinked S fails at compile time.
- [ ] `cargo test --workspace` green.
