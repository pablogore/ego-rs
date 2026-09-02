# Delta Spec: CORE-PERSIST-B — Re-Scoping `persistence-api-surface`'s CORE-PERSIST-A-Only Statements

> Canonical / English. Spanish companion: `spec.es.md` (1:1 requirement IDs and scenarios).
> This delta touches exactly two statements in the shipped `persistence-api-surface` spec: one
> Requirement and one Non-Goals bullet. Both were written to describe CORE-PERSIST-A's own
> boundary but are phrased as standing, timeless absolutes. Read as written, they would make
> CORE-PERSIST-B's legitimate, separately-scoped consumer edits and implementation relocation
> look like violations of this capability. No other requirement, scenario, or Non-Goals bullet in
> `openspec/specs/persistence-api-surface/spec.md` is touched by this delta. No requirement about
> port shape, path resolution, or trait identity changes.

## Capability: `persistence-api-surface` (MODIFIED)

## MODIFIED Requirements

### Requirement: No Consumer Outside The Two Crates Is Edited By CORE-PERSIST-A

This requirement binds CORE-PERSIST-A specifically: no crate other than `ego-domain` and
`ego-persistence-api` MUST have had an edited `use` statement or an added `Cargo.toml`
dependency as a result of CORE-PERSIST-A. It constrains CORE-PERSIST-A's own historical diff and
MUST NOT be read as a standing prohibition binding every future change to this capability. A
later, independently-proposed change MAY edit consumers outside these two crates for its own
explicitly-stated relocation and compatibility strategy, provided that change states its own
scope.

(Previously: phrased as an unqualified, timeless rule — "No crate other than `ego-domain` and
`ego-persistence-api` MUST have an edited `use` statement or an added `Cargo.toml` dependency as
a result of this change" — with no textual anchor identifying "this change" as CORE-PERSIST-A
specifically, which let a later change's own legitimate consumer edits read as a violation of
this requirement.)

#### Scenario: A downstream consumer compiles unedited under CORE-PERSIST-A

- GIVEN a crate importing a relocated item only through `ego_domain::*`
- WHEN the workspace is rebuilt after CORE-PERSIST-A
- THEN that crate's source and `Cargo.toml` are byte-identical to before CORE-PERSIST-A

#### Scenario: A later change may edit consumers within its own stated scope

- GIVEN a separately-proposed, separately-scoped change (for example CORE-PERSIST-B) that states
  its own consumer-edit scope and compatibility strategy
- WHEN that change edits `use` statements in crates other than `ego-domain` and
  `ego-persistence-api` (for example `examples/reference-app` or `ego-testkit`)
- THEN this requirement is not violated, because it binds CORE-PERSIST-A's own diff, not every
  future change to this capability

## MODIFIED Non-Goals

- No implementation move was in scope for CORE-PERSIST-A — every `InMemory*` and
  `PostgreSQL*`/`Postgres*` adapter stayed in its current crate as of CORE-PERSIST-A. A later,
  separately-scoped and separately-reviewed change (for example CORE-PERSIST-B) MAY relocate an
  implementation; this bullet does not bind it.

  (Previously: phrased as an unqualified, standing statement — "No implementation move — every
  `InMemory*` and `PostgreSQL*`/`Postgres*` adapter stays in its current crate" — with no
  CORE-PERSIST-A anchor, which read as a permanent prohibition on ever relocating an
  implementation rather than a statement of what CORE-PERSIST-A itself left untouched.)
