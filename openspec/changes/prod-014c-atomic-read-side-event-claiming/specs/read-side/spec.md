# Delta for read-side

> Canonical / source of truth. Spanish review companion: `spec.es.md` (1:1
> identifiers). Base capability spec: `openspec/specs/read-side/spec.md`
> (`Capability: read-side` section), the two PROD-014B requirements added
> there. This delta touches only those two; the third PROD-014B requirement,
> "Durable Dedup Bookkeeping Does Not Imply Exactly-Once Handler Execution,"
> is unaffected and MUST survive unchanged — it is not reproduced here
> because nothing about it changes.

Scope: PROD-014C. The single-writer adoption constraint PROD-014B documented
as external and unenforced is now enforced by the new
`read-side-event-claiming` capability. Both requirement titles are renamed
because their prior titles asserted the absence of enforcement — keeping the
old title with new, contradictory body text would misstate the capability;
`sdd-archive` MUST replace the old title's block with the renamed one, not
leave both present.

## RENAMED Requirements

### Requirement: Prevention of Double Handler Execution Rests on an Explicit, Unenforced Single-Writer Adoption Constraint → Prevention of Double Handler Execution Is Enforced By Atomic Claiming Across Replicas

(Reason: the constraint moved from an external, unenforced adoption
convention to a mechanism this framework itself enforces —
`read-side-event-claiming`'s atomic claim. The old title asserted
"unenforced," which is no longer true.)
(Migration: any doc or code comment citing the old title by name MUST be
updated to the new title; the guarantee it names has changed from absent to
enforced, not merely reworded.)

### Requirement: The Concurrency Gap Has a Named, Distinct Follow-Up → The Concurrency Gap Named In PROD-014B Is Discharged By Atomic Claiming

(Reason: the follow-up the old title pointed to — PROD-014C — has shipped.
The gap is closed, not merely tracked under a named follow-up.)
(Migration: any doc or code comment citing "PROD-014C" as an open follow-up
MUST be updated to state the gap is discharged, per
`read-side-event-claiming`.)

## MODIFIED Requirements

### Requirement: Prevention of Double Handler Execution Is Enforced By Atomic Claiming Across Replicas

Prevention of double handler execution for the same event, across concurrent
replicas of the same projection, MUST be enforced by the
`read-side-event-claiming` capability's atomic claim mechanism — never left
as an external, unenforced adoption constraint. At most one worker MUST hold
a valid processing claim for a given `(projection_id, tag, tenant)` at a
time; a worker without a valid claim MUST NOT invoke the handler for that
stream. This enforcement bounds handler-execution count only — it does not
bound what a handler's own external effect does (see "Durable Dedup
Bookkeeping Does Not Imply Exactly-Once Handler Execution," unchanged, and
`read-side-event-claiming`'s Non-Goals).
(Previously: stated this depended on an external, unenforced
single-writer-per-`(projection_id, tag, tenant)` adoption constraint, with no
leader election, lock, lease, or fencing mechanism enforcing it.)

#### Scenario: A two-replica deployment is inside the guarantee, and enforced

- GIVEN two replicas of the same projection process running concurrently
  against the same `(projection_id, tag, tenant)`
- WHEN both attempt to process at the same time
- THEN at most one holds a valid claim and invokes the handler; the other is
  refused and never invokes the handler for that tick — this configuration
  is inside this capability's guarantee, not outside it

#### Scenario: Enforcement never claims exactly-once handling

- GIVEN a worker holding a valid claim for a whole batch
- WHEN it crashes after the handler succeeds but before the batch is fully
  recorded, then resumes
- THEN the handler MAY run again for those events; enforcement of exclusion
  across replicas does not turn at-least-once handler execution into
  exactly-once, and no documentation may describe it as such

### Requirement: The Concurrency Gap Named In PROD-014B Is Discharged By Atomic Claiming

The gap PROD-014B named between durable dedup bookkeeping and prevention of
double handler execution across replicas MUST be treated as discharged:
`read-side-event-claiming` enforces exclusion before the handler runs.
Documentation describing this gap as open, unowned, or still pending a
follow-up MUST be treated as stale and corrected.
(Previously: stated the gap MUST be recorded as a distinct, named follow-up
— PROD-014C — Atomic Read-Side Event Claiming — rather than folded into
this capability's scope or silently left unowned.)

#### Scenario: A reader finds the mechanism discharged, not a pending follow-up

- GIVEN a reader of this capability's documentation looking for how double
  handler execution across replicas is prevented
- WHEN they look for the owning mechanism
- THEN they find atomic claiming, enforced by `read-side-event-claiming` —
  not a named-but-undelivered follow-up
