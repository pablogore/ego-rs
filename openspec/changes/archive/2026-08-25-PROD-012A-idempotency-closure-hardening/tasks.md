# Tasks: PROD-012A — Idempotency Closure Hardening

## Closed

- [x] **Fix 1 — Structural bypass closed.** New file
  `crates/service-sdk/tests/idempotent_marker_lint.rs`, mirroring the
  existing `tenant_scoped_lint.rs` mechanism: a `syn`-AST scan over
  workspace `src/`, run as a `cargo test`, that fails the build if any
  `#[operation]` lacks `#[idempotent]`. Scans `crates/*/src` and
  `examples/*/src`, skips `#[cfg(test)]` fixture modules. Root cause:
  `crates/service-sdk-macros/src/lib.rs:258-259` hardcodes `mutating: true`
  for every `#[operation]`, with nothing at the SDK level enforcing
  `mutating ⇒ idempotent`.
  Satisfies: the end-to-end no-bypass requirement (every mutating operation
  is idempotency-marked, structurally, not just where a narrower
  reference-app test happened to look).
  TDD: proven red (attribute-less mutating op fails the new lint) then
  green. Gates: `cargo fmt --check` clean on the new file, `cargo check
  --workspace` clean, `cargo clippy --workspace --all-targets -- -D
  warnings` clean, `cargo test --workspace` exit 0, zero failures.

- [x] **Fix 2 — Real multi-node race proof.**
  `integration-tests/tests/infrastructure/concurrent_replicas_postgres.rs`:
  both racing replicas now open a real
  `EntityEventStores::open(pool.clone())` and use the production
  `compose_entity_runtimes` wiring, instead of each replica writing through
  a private in-memory event store. New test
  `two_replicas_racing_one_key_yield_exactly_one_execution` asserts exactly
  one durable event set and one confirmed receipt exists in real Postgres
  for the contended key — the losing replica wrote nothing durable.
  Satisfies: scenario 8, "two owners/nodes racing" — proves the reservation
  fencing that already existed also gates the actual durable writes, not
  just reservation ownership.
  Verified green against real Postgres (colima/Docker), run 3 consecutive
  times, no flakes.

- [x] **Fix 3 — Single-aggregate crash-after-commit proof.** New file
  `integration-tests/tests/infrastructure/single_aggregate_crash_recovery_postgres.rs`
  (566 lines): a real child OS process commits an operation to real
  Postgres (verified via direct SQL), is then killed for real
  (`std::process::abort()` / SIGABRT — not simulated), and a retry from a
  fresh process/pool/owner is proven to replay the prior result with zero
  handler re-execution and zero duplicate rows.
  Satisfies: scenario 14, "crash after commit, before responding" — for the
  single-aggregate case, previously proven only for the dual-aggregate
  case.
  Verified green against real Postgres, run 3 consecutive times, no flakes.

- [x] **Fix 4 — Real-Postgres isolation proof.** New file
  `integration-tests/tests/infrastructure/receipt_identity_isolation_postgres.rs`
  (4 tests): holds 3 of the 4 identity fields (`tenant_id`,
  `aggregate_type`, `aggregate_id`, `operation_key`) fixed and varies one at
  a time against real Postgres receipts, proving no cross-contamination.
  Includes a negative control in the same scope (same fingerprint replays,
  different fingerprint conflicts) so the isolation proof isn't vacuous,
  and covers the NULL/systemwide-tenant vs. scoped-tenant partition
  explicitly.
  Satisfies: scenarios 17/18/19, tenant/type/id isolation — previously only
  structural/catalog-level (`schema_index_assertion.rs`), never functional.
  Verified green against real Postgres; part of a 39-41/41 passing full
  suite run (1 ignored test is pre-existing/unrelated).

- [x] **Documentation correction.** `ROADMAP.md` §7.12 and
  `openspec/specs/idempotent-command-processing/spec.md` corrected: the
  "two conforming adapters — HTTP and gRPC" language overclaimed a working
  second dispatch transport. Reworded to state the gRPC adapter
  (`GrpcMetadataCarrier`) is carrier/extraction-only — it passes the shared
  conformance harness for reading the key out of metadata, but no gRPC
  service/socket/command dispatch path exists in the workspace
  (`crates/transport/src/lib.rs:10-32`).

## Deliberately Not Checked — Residual, Documented Debt

- [ ] **Dual-aggregate write atomicity.** Not a defect — stated non-goal in
  the original PROD-012 spec, unchanged. A retry after partial failure
  resumes; it does not repeat.
- [ ] **Coalesced/duplicate idempotency keys, first-value-wins.** Not a
  defect — stated non-goal, unchanged. Behaviour is measured and asserted
  on both adapters rather than closed.
- [ ] **Generic reservation-conformance harness not Postgres-parametrized.**
  Genuinely deferred, not a non-goal. The parametrized `testkit` harness
  that both the in-memory double and the Postgres adapter run through is
  still not driven against real Postgres as one of its parametrized
  targets. Narrower than the tenant-isolation gap Fix 4 closed at the
  receipt layer; remains open.
- [ ] **`EntityRuntimeBuilder::build()` silent in-memory default**
  (`crates/persistent-entity/src/builder.rs:279-281`). Genuinely deferred,
  newly noted by this audit (not previously documented). No production
  path hits this today, but it is an unguarded footgun for a future host.
  Scoped as future composition-root hardening, explicitly not a PROD-012
  blocker and out of scope for this change.
