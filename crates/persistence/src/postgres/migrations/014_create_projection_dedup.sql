-- Durable read-side dedup bookkeeping: one row per (projection_id, tag, event_id)
-- already handled, used to converge repeated `mark_seen` calls onto one record.
--
-- NO TENANT COLUMN, DELIBERATELY
--
-- Dedup identity is (projection_id, tag, event_id) exactly (PROD-014B AD-7). The
-- `OffsetStore`/`DedupStore` SPI's `seen`/`mark_seen` take no tenant, and this
-- table is not tenant-scoped data — it stores no tenant-owned value, only the
-- presence of an event identifier. `projection_offsets` is the tenant-scoped
-- table in this pair and binds `tenant` in every statement instead.
--
-- The primary key is the identity `seen()` reads and `mark_seen()` inserts into
-- via `ON CONFLICT (...) DO NOTHING` — storage-level convergence to one row, not
-- execution exclusion (PROD-014B AD-6).
--
-- `created_at` is operational only; no query reads it today. It is what a
-- future retention follow-up (F-2) would scan — this migration ships no index
-- for it, since indexing for a scan nobody performs designs that pass in
-- advance.

CREATE TABLE IF NOT EXISTS projection_dedup (
    projection_id VARCHAR(255) NOT NULL,
    tag           VARCHAR(255) NOT NULL,
    event_id      VARCHAR(255) NOT NULL,
    created_at    TIMESTAMPTZ  NOT NULL DEFAULT NOW(),

    CONSTRAINT projection_dedup_identity
        PRIMARY KEY (projection_id, tag, event_id)
);
