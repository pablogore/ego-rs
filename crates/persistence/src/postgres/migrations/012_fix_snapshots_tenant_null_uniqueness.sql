-- Fix the snapshots table's uniqueness scheme for the tenant-less
-- ("systemwide") scope.
--
-- `CREATE UNIQUE INDEX idx_snapshots_aggregate ON snapshots(aggregate_id,
-- tenant_id)` (migration 003) treats every NULL `tenant_id` as distinct from
-- every other NULL, per ordinary SQL NULL semantics. Two consequences, both
-- silent:
--
-- - `ON CONFLICT (aggregate_id, tenant_id) DO UPDATE` never fires for a
--   systemwide-scope aggregate, because Postgres never considers two NULL
--   tenants a conflict against that index. Every save for such an aggregate
--   inserts a fresh row instead of updating the existing one.
-- - A plain `WHERE tenant_id = $2` predicate is never true when `$2` is NULL,
--   so a systemwide-scope `load_snapshot`/existing-version lookup can never
--   find a row it just wrote, even though the row exists.
--
-- This is the AD-1 pattern already applied to `events` (migration 008) and
-- `operation_receipts` (migration 011): two partial unique indexes over
-- complementary predicates, since `NULLS NOT DISTINCT` arrived in PostgreSQL
-- 15 and the declared floor is 14. Every row satisfies exactly one predicate,
-- so together they cover the table with no gap and no overlap.
--
-- DE-DUPLICATION FIRST
--
-- The bug this migration fixes is precisely what can have produced duplicate
-- rows: because `idx_snapshots_aggregate` never treated two NULL tenants as a
-- conflict, `save_snapshot`'s `ON CONFLICT` never fired for a systemwide-scope
-- aggregate, and every save inserted a fresh row instead of updating the
-- existing one. `CREATE UNIQUE INDEX` refuses to build over data that already
-- violates it, so a deployment old enough to have hit that bug would fail
-- outright below with no rows removed. Only `tenant_id IS NULL` rows are at
-- risk — the old index already enforced uniqueness for every non-null tenant.
-- Keep the highest `version` per `aggregate_id` (ties broken by the most
-- recently written row), matching what `load_snapshot`'s own
-- `ORDER BY version DESC LIMIT 1` already treats as the current snapshot, so
-- this changes no observable behavior for a deployment that reads through
-- that method.
DELETE FROM snapshots s
USING (
    SELECT id,
           ROW_NUMBER() OVER (
               PARTITION BY aggregate_id
               ORDER BY version DESC, created_at DESC, id DESC
           ) AS rank
    FROM snapshots
    WHERE tenant_id IS NULL
) ranked
WHERE s.id = ranked.id
  AND ranked.rank > 1;

DROP INDEX IF EXISTS idx_snapshots_aggregate;

CREATE UNIQUE INDEX IF NOT EXISTS ux_snapshots_identity_tenant
    ON snapshots (tenant_id, aggregate_id)
    WHERE tenant_id IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS ux_snapshots_identity_systemwide
    ON snapshots (aggregate_id)
    WHERE tenant_id IS NULL;
