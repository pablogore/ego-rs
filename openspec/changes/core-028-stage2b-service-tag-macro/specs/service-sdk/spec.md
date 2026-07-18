# Delta for service-sdk

Scope: CORE-028 Stage 2B. Gives the `#[service]` struct macro an optional
argument naming the trait a struct implements, so generated code can produce
the link between a service struct and its resolution Tag that today only a
caller-supplied coercion closure can express (see the companion
`application-composition` delta for the resulting `.service::<S>()` call
site). Additive only where the macro argument is absent: existing
`#[service]` usage on structs with no argument, and all trait/method-level
`#[service]` behavior, is unchanged.

## ADDED Requirements

### Requirement: Optional Struct-Macro Trait-Link Argument (`impl_of`)

The `#[service]` struct macro MUST accept an optional argument naming the
trait the struct implements, in the same explicit-argument style the macro
already uses elsewhere (illustratively `#[service(impl_of = Trait)]` — exact
argument syntax is a design decision). When this argument is present, the
macro MUST generate, at expansion time, a link from the struct to the
resolution Tag associated with the named trait, together with a concrete
coercion from an `Arc` of the struct to an `Arc<dyn Trait>`. This generated
link is what allows a single-type-parameter registration call to know, at
compile time, both which Tag to register under and how to produce the
trait-object coercion — information only macro-expansion-time code has,
because the caller cannot express "this struct implements whichever trait
underlies this Tag" as a Rust generic bound.

When the argument is absent, the macro's behavior on a struct MUST be
exactly what it is today: only the existing `Injectable`-related generation
occurs, with no trait link produced. Bare `#[service]` usage — including
testkit's — MUST compile and behave identically before and after this
change.

If the named trait argument does not name a trait the struct actually
implements, this MUST surface as a compile error at the macro-generated
code's location (an ordinary "trait not implemented"-shaped failure) — no
special macro diagnostic is required, but silently accepting a wrong or
unimplemented trait name, or deferring the mismatch to runtime, are not
acceptable outcomes.

#### Scenario: Bare `#[service]` struct usage is unaffected
- GIVEN a struct annotated `#[service]` with no trait-link argument, as
  written before this change
- WHEN the crate is compiled
- THEN it compiles and behaves exactly as before this change — no new
  required argument, no new generated trait link, no observable difference

#### Scenario: `#[service(impl_of = Trait)]` generates a usable trait link
- GIVEN a struct that implements `Trait`, annotated with the macro's optional
  trait-link argument naming `Trait`
- WHEN the crate is compiled
- THEN the macro generates a link from the struct to `Trait`'s resolution
  Tag and a coercion producing `Arc<dyn Trait>` from an `Arc` of the struct,
  and this link is what a single-type-parameter service registration call
  (see the companion `application-composition` delta) consumes to register
  and resolve the struct with no caller-supplied Tag or coercion closure

#### Scenario: A trait-link argument naming a trait the struct does not implement fails to compile
- GIVEN a struct annotated with the macro's trait-link argument naming a
  trait the struct does not implement
- WHEN the crate is compiled
- THEN compilation fails, identifying the trait the struct fails to satisfy
  — the mismatch is caught at compile time, not accepted and left to
  surface later as a runtime registration or resolution failure

## Non-Goals

- Any entity/aggregate-facing counterpart to this trait-link mechanism
  (`.entity::<E>()` or equivalent) — still blocked by CORE-006, unchanged
  from Stage 1 and Stage 2A.
- A runtime or link-time registry (`inventory`, `linkme`, `ctor`, or
  equivalent) to discover macro-linked services — the link is a
  compile-time-only construct consumed directly by generated code and the
  registration call; no new dependency is introduced.
- Inferring the implemented trait from the struct's name (e.g. stripping an
  `Impl` suffix) — the trait is only ever named explicitly through the
  macro argument.
- Any change to trait-level or method-level `#[service]` macro behavior, or
  to the existing `Injectable` contract itself.
