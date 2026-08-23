-- Enforce uniqueness of the event stream identity in the database.
--
-- Until now nothing but an in-process read-then-compare stood between two
-- concurrent appends and two rows at the same version: both could read the same
-- MAX(version) and both could insert. The identity is
-- (tenant_id, aggregate_type, aggregate_id, version).
--
-- WHY TWO INDEXES AND NOT ONE
--
-- A conventional UNIQUE treats every NULL as distinct from every other NULL, so
-- a single index over the four columns would permit unlimited duplicate rows in
-- the tenant-less ("systemwide") partition — exactly the partition where history
-- duplication was already found. `NULLS NOT DISTINCT` fixes that in one index,
-- but it arrived in PostgreSQL 15 and this workspace declares PostgreSQL 14 as
-- its floor (README.md). Verified against the pinned 14-alpine image rather than
-- assumed: `CREATE UNIQUE INDEX ... NULLS NOT DISTINCT` is a syntax error there,
-- and pg_index has no indnullsnotdistinct column to inspect.
--
-- So the equivalent strategy is stated explicitly instead: two partial unique
-- indexes over complementary predicates. Every row satisfies exactly one of
-- `tenant_id IS NOT NULL` and `tenant_id IS NULL`, so together they cover the
-- table with no gap and no overlap, and each enforces uniqueness within its own
-- partition. That is the same semantics the store's queries use when they compare
-- with IS NOT DISTINCT FROM: two NULLs are the same tenant, a NULL and a
-- concrete tenant are not.
--
-- The systemwide index deliberately omits tenant_id from its column list. The
-- predicate already fixes that column to NULL for every row the index contains,
-- so including it would index a constant and discriminate nothing. The asymmetry
-- is intentional and is pinned by the catalog assertion rather than left to be
-- rediscovered.
--
-- IF THIS MIGRATION FAILS
--
-- CREATE UNIQUE INDEX refuses to build over data that already violates it, so on
-- a deployment that accumulated duplicates this fails and the process does not
-- start. That is the intended outcome: the alternative is an index that silently
-- does not exist. PostgreSQL's own error names the index, the column list and the
-- duplicated key, which is enough to find the rows. Resolve them, then start
-- again.

CREATE UNIQUE INDEX IF NOT EXISTS ux_events_identity_tenant
    ON events (tenant_id, aggregate_type, aggregate_id, version)
    WHERE tenant_id IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS ux_events_identity_systemwide
    ON events (aggregate_type, aggregate_id, version)
    WHERE tenant_id IS NULL;
