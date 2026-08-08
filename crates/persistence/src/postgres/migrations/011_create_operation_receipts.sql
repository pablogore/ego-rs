-- Durable operation receipts: one row per operation that already completed
-- against one aggregate.
--
-- WHY THIS TABLE EXISTS AT ALL
--
-- A command that succeeds without emitting a single event has no event row to
-- carry its completion, so without a receipt it is indistinguishable from a
-- command that never ran. That case is normative, not an edge, which is why the
-- receipt could not be columns on `events` plus a partial index: such a row can
-- only exist where an event exists.
--
-- IDENTITY IS PER AGGREGATE
--
-- The logical identity is (tenant_id, aggregate_type, aggregate_id,
-- operation_key) — four columns, not two. The same operation key addressed at
-- two different aggregates is two distinct operations, and collapsing them to
-- (tenant_id, operation_key) would let one aggregate's completion suppress
-- another aggregate's work. That is cross-aggregate suppression, and it would be
-- silent.
--
-- THE NULL TENANT
--
-- `tenant_id` is nullable because the tenant-less "systemwide" scope is a
-- supported mode, and a conventional UNIQUE treats every NULL as distinct from
-- every other NULL — permitting unlimited receipts for one identity in exactly
-- the scope where a duplicate is least visible. `NULLS NOT DISTINCT` would say
-- this in one index but arrived in PostgreSQL 15, while the declared floor is 14.
-- So the AD-1 pattern applies: two partial unique indexes over complementary
-- predicates. Every row satisfies exactly one, so together they cover the table
-- with no gap and no overlap.
--
-- THE FINGERPRINT IS NOT PART OF THE IDENTITY
--
-- It is deliberately outside both indexes. Including it would make a different
-- request reusing an operation key insert a *second* row rather than collide,
-- and the two would then both be valid answers to the same lookup. Excluding it
-- means the second write raises a uniqueness violation the adapter reports as a
-- conflict — the retry is refused instead of silently overwriting a receipt, or
-- silently replaying someone else's result.
--
-- RETENTION
--
-- Receipts are permanently retained under a lifecycle distinct from `events`.
-- The ordinary reservation purge job must never target this table; only an
-- explicit aggregate or tenant deletion removes a receipt.

CREATE TABLE IF NOT EXISTS operation_receipts (
    id             BIGSERIAL PRIMARY KEY,
    tenant_id      VARCHAR(255),
    aggregate_type VARCHAR(255) NOT NULL,
    aggregate_id   VARCHAR(255) NOT NULL,
    operation_key  VARCHAR(255) NOT NULL,
    fingerprint    VARCHAR(255) NOT NULL,
    response       BYTEA        NOT NULL,
    created_at     TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX IF NOT EXISTS ux_operation_receipts_identity_tenant
    ON operation_receipts (tenant_id, aggregate_type, aggregate_id, operation_key)
    WHERE tenant_id IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS ux_operation_receipts_identity_systemwide
    ON operation_receipts (aggregate_type, aggregate_id, operation_key)
    WHERE tenant_id IS NULL;
