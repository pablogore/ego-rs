# Pre-Plan Validation Checklist: Persistent Entity Runtime and SDK

**Purpose**: Validate specification completeness, architectural consistency, domain correctness, concurrency model, determinism, and future compatibility before proceeding to `/speckit.plan`
**Created**: 2026-06-07
**Feature**: [spec.md](/Users/pablogore/workspace/pablogore/ego-rs/specs/006-persistent-entity-runtime/spec.md)

## 1. Completeness

- [ ] CHK001 Are user stories covering all entity lifecycle phases (creation, mutation, query, recovery, passivation)? [Completeness, Spec US1-US7]
- [ ] CHK002 Is the explicit entity creation flow (FR-018) covered by acceptance scenarios in both success (US2-S3) and error (US2-S2) paths? [Completeness, Spec US2]
- [ ] CHK003 Are all FRs (1-25) traceable to at least one acceptance scenario or edge case? [Completeness, Spec FR-001–FR-025]
- [ ] CHK004 Does every Edge Case have a corresponding functional requirement that governs the behavior? [Completeness, Spec Edge Cases]
- [x] CHK005 Are error paths for the EventPublisher SPI failure defined (the SPI is invoked after commit — failure is logged, entity stays ACTIVE, publication retried asynchronously)? [Gap, Spec §4 Failure & Concurrency Model]
- [x] CHK006 Are passivation triggers (inactivity, memory pressure, explicit) specified with enough detail to implement FR-014, including the passivation interaction with the mailbox? [Clarity, Spec FR-014, §5 Passivation Interaction]
- [ ] CHK007 Are success criteria covering all 6 evaluation dimensions (completeness, architecture, domain, concurrency, determinism, future)? [Completeness, Spec SC-001–SC-013]

## 2. Architectural Consistency

- [ ] CHK008 Does the command lifecycle order in FR-004 (recover → execute → persist → apply → snapshot → publish → respond) reflect the clarified snapshot timing (post-commit, outside atomic unit)? [Consistency, Spec FR-004, Q2]
- [ ] CHK009 Does the in-memory caching model (FR-005) align with the replay-only-during-recovery rule (FR-012)? Entities are cached after first access but should never replay during normal command execution. [Consistency, Spec FR-005, FR-012]
- [ ] CHK010 Does the EventPublisher SPI invocation (FR-009) occur at the correct point in FR-004 lifecycle (after commit, before response)? [Consistency, Spec FR-004, FR-009]
- [x] CHK011 Are the zero-event semantics (FR-019) compatible with the FR-004 lifecycle? The lifecycle short-circuits before persist for zero-event commands. [Consistency, Spec FR-004, FR-019]
- [ ] CHK012 Does the explicit creation rule (FR-018) integrate consistently with the EntityRef API (FR-003)? Creation commands flow through the same EntityRef interface as mutation commands. [Consistency, Spec FR-003, FR-018]
- [ ] CHK013 Does the single-writer guarantee (FR-007) by (tenant, entity_type, entity_id) correctly exclude cross-tenant or cross-entity blocking? [Consistency, Spec FR-007, US5]
- [ ] CHK014 Is the single-tenant mode (tenant_id = None) consistently handled across FR-011, US5, and the assumptions section? [Consistency, Spec FR-011, US5]

## 3. Domain Correctness

- [ ] CHK015 Is EntityNotFound error referenced consistently in user stories (US2-S2), functional requirements (FR-018), edge cases, and key entities? [Consistency, Spec US2, FR-018, Edge Cases, Key Entities]
- [ ] CHK016 Are explicit entity creation rules applied uniformly: creation command succeeds, non-creation commands fail with EntityNotFound? [Completeness, Spec FR-018]
- [ ] CHK017 Do zero-event semantics apply to all command types that produce no events, not just explicitly labeled "queries"? [Clarity, Spec FR-019]
- [ ] CHK018 Is the CommandContext (FR-016) available at all pipeline stages that need it (command handler, event metadata, persistence)? [Completeness, Spec FR-016]
- [x] CHK019 Are causation_id semantics defined clearly? (Deferred to plan phase — precise semantics documented as low-uncertainty deferred item) [Clarity, Spec FR-016, Deferred]
- [ ] CHK020 Are the error types (EntityNotFound, VersionConflict, EntityPassivating, MailboxFull, ReentrancyNotAllowed) the complete set of runtime-level errors, or are additional errors needed (e.g., ShardMoved)? [Completeness, Spec Key Entities]

## 4. Concurrency Model

- [x] CHK021 Does FR-007 explicitly define the queuing mechanism for commands that arrive while another is executing (bounded queue, backpressure, rejection)? [Clarity, Spec FR-007, FR-020, Mailbox Model]
- [x] CHK022 Is the optimistic concurrency version conflict (FR-008) behavior consistent across creation (version 0 → 1) and mutation (version N → N+1) flows? [Consistency, Spec FR-008, §6 Versioning & Snapshot] — Resolved: version start = 0, first event transitions 0→1. VersionConflict on creation uses expected_version=0, on mutation uses expected_version=current. Same optimistic concurrency mechanism applies to both.
- [x] CHK023 Is the retry responsibility for version conflicts clearly assigned (manual retry by caller vs automatic retry by runtime)? [Clarity, Spec US7, §6.5] — Resolved: caller is responsible for refreshing version and retrying. Runtime does NOT auto-retry version conflicts. VersionConflict response includes current version.
- [x] CHK024 Does the single-writer guarantee interact correctly with the in-memory cache (FR-005) — are cache lookups and command execution properly synchronized via the mailbox model? [Consistency, Spec FR-005, FR-007, FR-025]

## 5. Determinism

- [ ] CHK025 Does the Handler Safety Contract enumerate all operations that BREAK determinism, not just examples? [Completeness, Spec Handler Safety Contract]
- [ ] CHK026 Are the replay safety rules (FR-012) consistent with the Handler Safety Contract's "forbidden during recovery replay" section? [Consistency, Spec FR-012, Handler Safety Contract]
- [ ] CHK027 Is the CI guard scope clearly defined — which patterns should it detect vs what is infeasible to detect statically? [Clarity, Spec Handler Safety Contract]
- [ ] CHK028 Is the deterministic function contract `(state, command, context) -> (events | error)` consistently enforced for both command handlers and event appliers? [Consistency, Spec Handler Safety Contract]

## 6. Future Compatibility

- [ ] CHK029 Do the defined SPIs (EventPublisher, CommandContext, SnapshotStrategy) cleanly support future cluster sharding (CORE-007) without breaking changes? [Completeness, Spec Key Entities]
- [ ] CHK030 Are there any infrastructure type leaks (Tokio, gRPC, Kafka, database drivers) in the spec-defined contracts? [Consistency, Spec §Out of Scope]
- [ ] CHK031 Does the entity identifier triple (tenant, type, ID) in FR-011 naturally extend to include a shard ID or node affinity key? [Assumption, Spec FR-011]
- [ ] CHK032 Is the passivation extension point (FR-014) defined abstractly enough to later support passivation coordinated across cluster nodes? [Completeness, Spec FR-014]

## Notes (continued)

- This is a pre-plan validation checklist. Items marked as [Gap] indicate aspects that may need resolution during planning or implementation.
- Each CHK item tests the QUALITY of requirements writing, not the correctness of any implementation.
- All items reference specific spec sections or clarified decisions (Q1–Q8).

## 7. Execution Model Completeness

- [ ] CHK033 Does the entity lifecycle state machine (RECOVERING, ACTIVE, PASSIVATING, PASSIVATED, FAILED) cover all transitions in the Failure & Concurrency Model table? [Completeness, Spec Entity Lifecycle Model, §4 Failure & Concurrency Model]
- [ ] CHK034 Is the mailbox bounded capacity consistently enforced across all lifecycle states — are there code paths where a command bypasses the mailbox (e.g., during recovery)? [Consistency, Spec FR-020, Mailbox Model]
- [ ] CHK035 Does the passivation interaction section (PASSIVATING rejection, PASSIVATED reactivation, passivation cancellation) cover all corner cases, including simultaneous command arrival and passivation trigger? [Completeness, Spec §5 Passivation Interaction]
- [ ] CHK036 Is the reentrancy prohibition (FR-024) enforceable at the runtime level, or does it require developer cooperation? The spec says "the runtime MUST return ReentrancyNotAllowed" — is there a mechanism to detect handler-to-self at the mailbox level? [Clarity, Spec FR-024]
- [ ] CHK037 Does the thread-local entity state model (FR-025) conflict with any shared-memory assumptions in the persistence SPI (CORE-001) or the effect API (CORE-003)? [Consistency, Spec FR-025]
- [ ] CHK038 Are the new success criteria (SC-009–SC-013) independently verifiable without real infrastructure (i.e., testable with in-memory backend)? [Completeness, Spec SC-009–SC-013]
