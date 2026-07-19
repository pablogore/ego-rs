# Delta for application-composition

> Base capability spec: `openspec/specs/application-composition/spec.md`
> (Stage 1, archived via PR #189). This delta modifies that spec's "Service
> Registration Follows The Existing Injectable Contract" requirement only.
> Adapter, config, security, lifecycle, and projection registration
> requirements in the base spec are unchanged by this delta.

Scope: CORE-028 Stage 2B. Today, registering a macro-annotated service
requires naming both the service type and its resolution Tag, plus a
caller-supplied identity coercion closure (`.service::<S, Tag>(|arc| arc)`),
because `#[service]` on a struct carries no link to the trait it implements.
This delta gives the struct macro that link (see the companion `service-sdk`
delta) and collapses registration for macro-linked services to a single type
parameter with no closure: `.service::<S>()`. The prior two-generic
explicit-Tag form is renamed and remains permanently supported for
hand-rolled `Injectable` structs that carry no macro-generated trait link —
it is not deprecated and has no removal date.

## MODIFIED Requirements

### Requirement: Service Registration Follows The Existing Injectable Contract

Registering a service MUST validate and construct it through the same
service-construction contract the framework already uses in production
(`Injectable`) — no parallel construction path is introduced. A missing
dependency MUST produce an error naming both the missing dependency's type
and the service that required it, matching the attribution `try_build`
already provides today.

The composition root MUST offer exactly two named forms for registering a
service, distinguished by whether the service type carries a
macro-generated link to its resolution Tag:

1. **Macro-linked registration (primary form).** For a service type that
   carries a macro-generated trait link (produced by the companion
   `service-sdk` delta's optional struct-macro argument), the composition
   root MUST provide a registration call that takes only that one service
   type as a generic parameter — no Tag parameter, and no caller-supplied
   coercion closure. It MUST construct the service through the existing
   `Injectable` contract and register it resolvable under its linked Tag,
   with construction and error-attribution behavior identical to today's
   two-generic form.
2. **Explicit-Tag registration (renamed, permanent).** The form that takes
   both the service type and its Tag as separate generic parameters, plus a
   caller-supplied coercion closure, MUST remain available under a new name
   (exact identifier is a design decision, not fixed by this spec) for
   service types with no macro-generated trait link — chiefly hand-rolled
   `Injectable` structs the macro never touched. This form MUST NOT be
   deprecated, time-boxed, or scheduled for removal: it is the only route a
   hand-rolled `Injectable` struct has to registration, since such a struct
   can never carry a macro-generated link.

A service type with no macro-generated trait link MUST NOT be accepted by
the macro-linked registration call — this MUST be rejected at compile time
(an unsatisfied trait/type bound), never accepted and left to fail at
runtime or during `build()`/`try_build()`.

#### Scenario: A registered service with satisfied dependencies resolves
- GIVEN a service registered with all its declared dependencies also
  registered
- WHEN the application is constructed
- THEN the service is constructed successfully and resolvable

#### Scenario: A missing dependency names both the missing type and the requester
- GIVEN a service registered whose declared dependency is not itself
  registered
- WHEN the application is constructed
- THEN construction fails with an error identifying the missing dependency's
  type and the service that requested it

#### Scenario: A bare `#[service]` struct with no trait link is unaffected by this change
- GIVEN a struct annotated only `#[service]` (no macro trait-link argument),
  exactly as before this change
- WHEN it is registered through the existing registration path it already
  used before this change
- THEN it compiles and registers exactly as it did before this change — no
  new required argument, no behavior difference

#### Scenario: A macro-linked service registers with a single type parameter and no closure
- GIVEN a service struct carrying a macro-generated trait link
- WHEN it is registered using the macro-linked registration call, naming
  only that one service type
- THEN registration succeeds with no Tag parameter and no coercion closure
  supplied by the caller, and the service resolves identically to how the
  prior two-generic form would have resolved it

#### Scenario: A service type with no trait link fails to compile against the macro-linked call
- GIVEN a service struct with no macro-generated trait link (e.g. a
  hand-rolled `Injectable` struct, or a bare `#[service]` struct with no
  trait-link argument)
- WHEN that type is passed as the single generic parameter to the
  macro-linked registration call
- THEN the code fails to compile — the missing link surfaces as a compile
  error, never a runtime registration failure or a successfully-built
  application

#### Scenario: A hand-rolled Injectable struct still registers through the renamed explicit-Tag form
- GIVEN a hand-rolled `Injectable` struct with no macro annotation and
  therefore no trait link
- WHEN it is registered using the renamed explicit-Tag registration form,
  naming both the struct's type and its Tag with a coercion closure, exactly
  as the pre-rename two-generic form required
- THEN registration succeeds and the service resolves under that Tag,
  identically to how the pre-rename form resolved it

*Open → design.md*: the exact identifier for the renamed explicit-Tag form,
and the exact registration-to-construction ordering and tag-binding
mechanics, are not fixed by this spec — only the two observable forms above,
and the compile-time rejection of an unlinked type, are required.

## Non-Goals

- `.entity::<E>()` / any per-aggregate entity registration coupling to the
  trait-link mechanism this delta introduces — still blocked by CORE-006,
  unchanged from Stage 1 and Stage 2A.
- Any runtime or link-time service registry/discovery mechanism (e.g. an
  `inventory`-, `linkme`-, or `ctor`-style crate) to locate macro-linked
  services — the trait link is resolved entirely at compile time through
  the type system; DI resolution stays synchronous.
- Naming-convention–based inference of the implemented trait (e.g. stripping
  an `Impl` suffix) — the trait link is only ever established through the
  explicit `impl_of` macro argument (see the companion `service-sdk` delta),
  never inferred from a struct's name.
- A deprecation window or migration deadline for the renamed explicit-Tag
  form — it is a permanent, first-class registration path, not a
  transitional one.
