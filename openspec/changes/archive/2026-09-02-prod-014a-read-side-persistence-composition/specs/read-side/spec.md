# Delta for read-side

> Canonical / source of truth. Spanish review companion: `spec.es.md` (1:1
> identifiers). Base capability spec:
> `openspec/specs/read-side/spec.md` (CORE-026). This delta applies on top
> of that spec's existing "Non-Goals" section, in particular the bullet
> "Constructing a dedup store, an offset store, or a tag-discovery
> mechanism is out of scope."

Scope: PROD-014A. This is a boundary clarification (D-5), not a
renegotiation of CORE-026. Two axes: the framework constructing or
defaulting read-side stores remains a non-goal, unaffected; the
composition root accepting, classifying, and validating a
host-constructed pair is new, in scope. The registration surface and the
Production refusal mechanics themselves are specified by the
`application-composition` and `production-composition-hardening` deltas —
this delta states only that the boundary permits them.

## ADDED Requirements

### Requirement: Composition-Root Acceptance Of A Host-Constructed Durable Progress Pair Is In Scope; Framework Construction Remains Out Of Scope

A projection's durable progress pair — its `OffsetStore` and `DedupStore`
— MAY be composed at the composition root: accepted, classified by
durability, and refused there under `Profile::Production` when not
durable. This is orthogonal to, and does not reverse, CORE-026's existing
non-goal that the framework constructs or defaults these stores
internally — that non-goal remains fully in force. The composition root
never internally constructs an `OffsetStore`, `DedupStore`, or
tag-discovery mechanism on the application's behalf; it only accepts,
classifies, and validates a pair the application already built.

#### Scenario: The composition root classifies and validates without constructing

- GIVEN an application that has already constructed its own
  `OffsetStore`/`DedupStore` pair
- WHEN it registers that pair at the composition root
- THEN the composition root classifies and validates the pair's
  durability without itself constructing either store

#### Scenario: An application that registers nothing is unaffected

- GIVEN an application that never registers a durable progress pair at
  the composition root
- WHEN it composes its read-side wiring exactly as before this change
- THEN nothing about that wiring is required or performed by this
  capability, unchanged from before

#### Scenario: The refusal never reaches the scheduler engine

- GIVEN a registered pair refused under `Profile::Production`
- WHEN that refusal occurs
- THEN it occurs at the composition root, never inside
  `ProjectionSpec::new()`, `TagSchedulerImpl::spawn()`, or the first
  poll batch

## Non-Goals

- Introducing `Profile` into `ProjectionSpec`, `TagSchedulerImpl`,
  `ReadSideSession`, or `ReadSideRunner` remains out of scope. No change
  to polling, dedup, offset, or ordering semantics. The refusal this
  delta permits happens only at the composition root.
- The framework constructing or defaulting `OffsetStore`, `DedupStore`,
  or a tag-discovery mechanism remains out of scope entirely — this is
  the existing CORE-026 non-goal above, unaffected and not renegotiated
  by this delta.
- `ReadSideStore` (the event source a projection polls) durability is not
  governed by this delta — it is a read view of the upstream event
  store, not resume state.
