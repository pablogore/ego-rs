# Constitutional Compliance Mapping

This document maps each constitutional rule to its enforcement mechanisms and validation strategies.

## Constitutional Rules and Enforcement

### Architecture Rules

| Rule ID | Rule | Primary Enforcement Layer | Enforcement Mechanism |
|---------|------|---------------------------|----------------------|
| FO-R1 | No component may assume a responsibility assigned to another component | CI | detect-violations.sh |
| FO-R2 | Concurrency ownership is exclusively held by BatchExecutor | Compile-time | ConcurrencyToken type |
| FO-R3 | A component MUST NOT import or call APIs from a component it does not directly interface with in the pipeline | CI | detect-violations.sh |
| UoW-R1 | A UoW MUST NOT be split. One TagDecision produces exactly one BatchCommand, one Session, one commit | CI | detect-violations.sh |
| UoW-R2 | A UoW MUST NOT be retried. Failure is final. A new decision for the same tag creates a new UoW | CI | detect-violations.sh |
| UoW-R3 | Handler invocations within a UoW MUST be sequential and single-threaded | Compile-time | ConcurrencyToken type |
| UoW-R4 | A UoW owns its tag exclusively during EXECUTING and COMMITTING. No second UoW for the same tag may be ASSIGNED until the first is COMPLETED or FAILED | CI | detect-violations.sh |
| UoW-R5 | A UoW's event range `[offset_before, offset_after)` MUST be contiguous and non-overlapping with any other UoW for the same tag | CI | detect-violations.sh |
| B-R1 | A batch MUST contain events for exactly one tag | CI | detect-violations.sh |
| B-R2 | Events within a batch MUST be in stream order | CI | detect-violations.sh |
| B-R3 | Dedup is applied within the batch: duplicate keys are counted but not re-processed | CI | detect-violations.sh |
| B-R4 | Batch size is bounded by `batch_size` from `BatchCommand`. BatchExecutor MUST NOT fetch beyond this bound | CI | detect-violations.sh |
| B-R5 | A batch MUST NOT outlive its UoW | CI | detect-violations.sh |

### State and Consistency Rules

| Rule ID | Rule | Primary Enforcement Layer | Enforcement Mechanism |
|---------|------|---------------------------|----------------------|
| OM-R1 | Offset MUST be loaded from persistent storage BEFORE fetching events | CI | detect-violations.sh |
| OM-R2 | Offset MUST NOT be updated during handler execution | CI | detect-violations.sh |
| OM-R3 | Offset MUST be persisted ONLY during the commit phase | CI | detect-violations.sh |
| OM-R4 | On restart, the authoritative offset is the last COMMITTED offset | Test | Fault injection + mock store tests |
| DD-R1 | Dedup state MUST be checked BEFORE handler execution | CI | detect-violations.sh |
| DD-R2 | Dedup MUST be persisted ONLY during the commit phase | CI | detect-violations.sh |
| DD-R3 | Dedup MUST NOT be checked or persisted outside the commit boundary | CI | detect-violations.sh |
| DD-R4 | Dedup entries are created only on COMPLETED. A FAILED UoW produces no dedup entries | CI | detect-violations.sh |
| AC-R1 | Commit MUST be atomic: offset + dedup persisted together in one transaction | Runtime + Test | AtomicityGuard + fault injection |
| AC-R2 | Partial commits (offset or dedup alone) are FORBIDDEN | Runtime + Test | AtomicityGuard + fault injection |
| AC-R3 | Commit failure MUST roll back both offset and dedup | Runtime + Test | AtomicityGuard + fault injection |
| FS-R1 | Ambiguous states produce rejection, never silent continuation | Runtime | PhaseGuard, fail-closed mode |
| FS-R2 | Partial failures are explicit errors, never retried | Runtime | PhaseGuard, fail-closed mode |
| FS-R3 | Unknown inputs, undefined transitions, and inconsistent states MUST terminate the current execution cycle immediately | Runtime | PhaseGuard, fail-closed mode |
| FS-R4 | Handler panic during EXECUTING terminates the UoW. Tag does not advance. Next UoW re-processes from the last committed offset | Runtime | PhaseGuard, fail-closed mode |

### External Effect Rules

| Rule ID | Rule | Primary Enforcement Layer | Enforcement Mechanism |
|---------|------|---------------------------|----------------------|
| EE-R1 | Handlers MUST describe external effects as intents (`ExternalEffectDescription`). Direct calls to external systems from handlers are FORBIDDEN | CI | detect-violations.sh |
| EE-R2 | External effect intents MUST be collected in the commit payload and persisted atomically with offset + dedup | Compile-time + Test | Type system + contract tests |
| EE-R3 | Every external effect MUST carry an `IdempotencyKey` derived from the UoW identity | Compile-time | Type-system (required field) |
| EE-R4 | Dispatch MUST occur AFTER commit succeeds | Runtime + Test | BatchExecutor post-commit dispatch + fault injection tests |
| EE-R5 | External effect failure MUST NOT roll back the commit | Runtime + Test | BatchExecutor post-commit dispatch + fault injection tests |
| EE-R6 | Only BatchExecutor may dispatch external effects | Runtime + Test | BatchExecutor post-commit dispatch + fault injection tests |

### Immutability Rules

| Rule ID | Rule | Primary Enforcement Layer | Enforcement Mechanism |
|---------|------|---------------------------|----------------------|
| IM-R1 | All domain data structures MUST be treated as immutable values | CI | detect-violations.sh |
| IM-R2 | Event stores MUST be append-only | CI | detect-violations.sh |
| IM-R3 | Read-side projections MUST be derived from immutable event streams | CI | detect-violations.sh |
| IM-R4 | Any mutable structure requires explicit justification in the design document | CI | detect-violations.sh |

### Testing Rules

| Rule ID | Rule | Primary Enforcement Layer | Enforcement Mechanism |
|---------|------|---------------------------|----------------------|
| MT-R1 | Every test MUST validate an observable behavior | Test | detect-test-smells.sh |
| MT-R2 | Every test MUST be able to detect at least one realistic defect | Test | detect-test-smells.sh |
| MT-R3 | Mock verification MAY be used as supporting evidence | Test | detect-test-smells.sh |
| MT-R4 | A test MUST fail if the protected behavior is intentionally broken | Test | detect-test-smells.sh |
| MT-R5 | Test names MUST describe the behavior being validated | Test | detect-test-smells.sh |
| MT-R6 | Testing effort MUST prioritize business invariants | Test | detect-test-smells.sh |
| MT-R7 | Coverage metrics are necessary but not sufficient | Test | verify-coverage.sh |
| MT-R8 | A Pull Request containing tests whose sole purpose is increasing coverage without validating meaningful behavior MUST be rejected | Test | detect-test-smells.sh |
| PC-R1 | A feature MUST NOT be considered tested if only the success path is validated | Test | detect-test-smells.sh |
| PC-R2 | Every conditional branch MUST have tests covering both outcomes | Test | detect-test-smells.sh |
| PC-R3 | Every operation capable of failure MUST have tests validating failure behavior | Test | detect-test-smells.sh |
| PC-R4 | Components responsible for persistence, consistency, replay, offset management, deduplication, or execution recovery MUST include recovery-path tests | Test | detect-test-smells.sh |
| PC-R5 | Boundary conditions MUST be tested | Test | detect-test-smells.sh |
| PC-R6 | Public APIs MUST validate behavior for invalid inputs | Test | detect-test-smells.sh |
| PC-R7 | Concurrency-sensitive components MUST test single execution, concurrent execution, capacity exhaustion, backpressure activation, contention scenarios | Test | detect-test-smells.sh |
| PC-R8 | Every state machine MUST validate valid transitions, invalid transitions, terminal states, recovery transitions | Test | detect-test-smells.sh |
| PC-R9 | Every constitutional invariant MUST have at least one test proving enforcement | Test | detect-test-smells.sh |

### Documentation Rules

| Rule ID | Rule | Primary Enforcement Layer | Enforcement Mechanism |
|---------|------|---------------------------|----------------------|
| DOC-R0 | All public Rust APIs MUST include rustdoc documentation covering public structs, enums, traits, functions, and modules | Compile-time | rustc with `#![deny(missing_docs)]` |
| DOC-R1 | All Rust source files MUST contain rustdoc documentation | CI | detect-missing-docs.sh |
| DOC-R2 | Documentation is mandatory for both public and private APIs | CI | detect-missing-docs.sh |
| DOC-R3 | Components participating in Scheduler, Worker, BatchExecutor, Session, Offset Store, Dedup Store, Runtime MUST document ownership, invariants, failure semantics, constitutional references | CI | detect-missing-docs.sh |
| DOC-R4 | Undocumented source files are constitutional violations | CI | detect-missing-docs.sh |
| DOC-R5 | CI MUST fail if required rustdoc is missing | CI | detect-missing-docs.sh |
| DOC-R6 | Documentation exemptions MUST be explicit | CI | detect-missing-docs.sh |
| DOC-R7 | A task MUST NOT be considered complete unless code compiles, tests pass, documentation compiles, missing_docs check passes | CI | detect-missing-docs.sh |
| DOC-R8 | Architectural components MUST document responsibilities, ownership, failure semantics, constitutional rules enforced | CI | detect-missing-docs.sh |

### Governance Rules

| Rule ID | Rule | Primary Enforcement Layer | Enforcement Mechanism |
|---------|------|---------------------------|----------------------|
| G-R1 | All tasks MUST contain exact file path, modification type, target symbol, expected outcome, validation criteria | CI | verify-constitution-mapping.sh |
| G-R2 | Evidence requirements must be met for task completion | CI | verify-constitution-mapping.sh |
| G-R3 | All tasks complete with evidence | CI | verify-constitution-mapping.sh |
| G-R4 | Coverage >= 85% | CI | verify-coverage.sh |
| G-R5 | cargo test, cargo clippy, and cargo fmt all pass | CI | verify-constitution-mapping.sh |
| G-R6 | Feature archived with incomplete tasks | CI | verify-constitution-mapping.sh |
| G-R7 | Workflow stages skipped | CI | verify-constitution-mapping.sh |
| G-R8 | Contract version not bumped on breaking change | CI | verify-constitution-mapping.sh |