-- Fix the aggregates table's uniqueness scheme for the tenant-less
-- ("systemwide") scope.
--
-- `aggregate_id VARCHAR(255) PRIMARY KEY` (migration 002) is unique across the
-- whole table, tenant ignored entirely — the opposite failure from
-- `snapshots`' old index: instead of never colliding two NULL tenants, it
-- collides every tenant, including two different concrete tenants sharing an
-- `aggregate_id`. `PostgreSQLRepository::save`'s `ON CONFLICT (aggregate_id,
-- tenant_id)` has never matched this constraint at all, so today every save
-- fails outright with `there is no unique or exclusion constraint matching
-- the ON CONFLICT specification` (42P10) — not only the systemwide scope.
--
-- This is the AD-1 pattern already applied to `events` (migration 008),
-- `operation_receipts` (migration 011) and `snapshots` (migration 012): two
-- partial unique indexes over complementary predicates, since `NULLS NOT
-- DISTINCT` arrived in PostgreSQL 15 and the declared floor is 14. Every row
-- satisfies exactly one predicate, so together they cover the table with no
-- gap and no overlap — and, unlike the single table-wide `PRIMARY KEY` it
-- replaces, a tenant-scoped identity no longer collides across tenants.
--
-- NO DE-DUPLICATION NEEDED
--
-- Unlike `snapshots`, this table's existing `PRIMARY KEY` already forced
-- exactly one row per `aggregate_id` regardless of tenant, so no deployment
-- can hold two rows that would violate either new partial index. The
-- `DROP CONSTRAINT` below is therefore safe with nothing to clean up first.
--
-- `idx_aggregates_tenant` (migration 002) is left in place, matching the
-- `idx_events_aggregate` precedent: a non-unique secondary index answers
-- "which aggregates does this tenant own" and is unrelated to the identity
-- constraint being fixed here.
ALTER TABLE aggregates DROP CONSTRAINT aggregates_pkey;

CREATE UNIQUE INDEX IF NOT EXISTS ux_aggregates_identity_tenant
    ON aggregates (tenant_id, aggregate_id)
    WHERE tenant_id IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS ux_aggregates_identity_systemwide
    ON aggregates (aggregate_id)
    WHERE tenant_id IS NULL;
