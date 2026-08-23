-- Durable operation reservations: one row per client-supplied operation, holding
-- the lease that decides who may execute it.
--
-- IDENTITY AND THE NULL TENANT
--
-- A reservation is identified by (tenant_id, operation_key). `tenant_id` is
-- nullable because the tenant-less "systemwide" scope is a supported mode, and a
-- conventional UNIQUE would treat every NULL as distinct from every other NULL —
-- permitting unlimited reservations for one key in exactly the scope where a
-- duplicate is least visible. `NULLS NOT DISTINCT` says this in one index and
-- arrived in PostgreSQL 15, while the declared floor is 14, so the AD-1 pattern
-- applies: two partial unique indexes over complementary predicates. Every row
-- satisfies exactly one, so together they cover the table with no gap and no
-- overlap.
--
-- STATE
--
-- `state` is 'in_progress' or 'completed', constrained rather than left to
-- convention, because every guarantee this table exists to provide is stated in
-- terms of that distinction: a lease may only be taken over from an in-progress
-- row, and only a completed row is ever purged.
--
-- `completed_at` and `response` are NULL exactly while the row is in progress, and
-- non-NULL exactly once it completes. The CHECK ties them to `state` rather than
-- trusting writers to keep three columns consistent — purge eligibility is
-- measured from `completed_at`, so a completed row with no timestamp would be
-- unpurgeable forever and an in-progress row with one would be purgeable while
-- still held.
--
-- `fencing_token` is BIGINT and strictly increases on every takeover, and the
-- boundary that matters is **this column's**, not the domain counter's. The domain
-- counts in u64; BIGINT is i64. At i64::MAX the domain's own increment still
-- succeeds — u64 has room — so an unchecked conversion would land on i64::MIN, a
-- value this column would happily accept and which is *less* than the token it
-- displaced. The adapter converts with a checked cast and reports exhaustion at the
-- storage limit; the CHECK below is the table's own guard, for anything that writes
-- here without going through the adapter.
--
-- A token is therefore always positive. Zero is excluded too: the sequence starts at
-- one, so zero could only arrive from a writer that did not mint it.

CREATE TABLE IF NOT EXISTS operation_reservations (
    id            BIGSERIAL PRIMARY KEY,
    tenant_id     VARCHAR(255),
    operation_key VARCHAR(255) NOT NULL,
    fingerprint   VARCHAR(255) NOT NULL,
    owner_id      VARCHAR(255) NOT NULL,
    fencing_token BIGINT       NOT NULL,
    lease_until   TIMESTAMPTZ  NOT NULL,
    state         VARCHAR(16)  NOT NULL,
    completed_at  TIMESTAMPTZ,
    response      BYTEA,
    created_at    TIMESTAMPTZ  NOT NULL DEFAULT NOW(),

    CONSTRAINT operation_reservations_state_known
        CHECK (state IN ('in_progress', 'completed')),
    CONSTRAINT operation_reservations_fencing_token_is_positive
        CHECK (fencing_token > 0),
    CONSTRAINT operation_reservations_completion_is_consistent
        CHECK (
            (state = 'in_progress' AND completed_at IS NULL AND response IS NULL)
         OR (state = 'completed'   AND completed_at IS NOT NULL AND response IS NOT NULL)
        )
);

CREATE UNIQUE INDEX IF NOT EXISTS ux_operation_reservations_identity_tenant
    ON operation_reservations (tenant_id, operation_key)
    WHERE tenant_id IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS ux_operation_reservations_identity_systemwide
    ON operation_reservations (operation_key)
    WHERE tenant_id IS NULL;

-- Purge scans by state and completion time. The index carries both so an eligible
-- row is found without reading in-progress rows at all.
--
-- Note what this index is *not*: a promise about which eligible rows a purge call
-- chooses. Selection within a batch is deliberately outside the contract (AD-11);
-- this index exists so the scan is cheap, and a caller must not read an ordering
-- into it.
CREATE INDEX IF NOT EXISTS idx_operation_reservations_purge
    ON operation_reservations (state, completed_at);
