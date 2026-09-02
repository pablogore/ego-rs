# PostgreSQL Persistence

Concrete PostgreSQL implementations of `ego-domain`'s persistence SPIs: event
store, snapshot store, operation reservation store, and (PROD-014B) the
durable read-side progress pair.

## Durable Read-Side Progress (PROD-014B)

`PostgreSQLOffsetStore` and `PostgreSQLDedupStore` (`src/postgres/read_side_offset.rs`,
`src/postgres/read_side_dedup.rs`) are the durable `OffsetStore`/`DedupStore`
pair a `Profile::Production` composition registers for its read-side
projections. Both `is_durable() -> true`.

**Adoption constraint — read before deploying**: safe operation depends on
exactly one writer per `(projection_id, tag, tenant)`. Neither store, nor
anything else in this workspace, enforces that across replicas — no leader
election, lock, lease, or fencing token exists here. Running two replicas of
the same projection concurrently is outside the guarantee these stores
provide, and nothing detects or refuses that configuration. This is not
exactly-once handler execution — see each adapter's own rustdoc for the full
storage-convergence-vs-execution-exclusion distinction. Closing that gap is
**PROD-014C — Atomic Read-Side Event Claiming**, a named, distinct follow-up
not designed or implemented here.

`projection_dedup` also grows unboundedly with unique events processed — no
purge, TTL, or eviction ships in this capability; row count is an
operational signal to observe, not a surprise.
