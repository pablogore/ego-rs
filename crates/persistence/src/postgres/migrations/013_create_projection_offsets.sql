-- Durable read-side projection offsets: one row per (projection_id, tag, tenant),
-- holding the last position a projection has advanced to.
--
-- IDENTITY AND TENANT
--
-- The primary key is the full identity the read-side `OffsetStore` SPI reads and
-- writes by: (projection_id, tag, tenant). `tenant` is NOT NULL — unlike the
-- nullable/systemwide-tenant handling used elsewhere in this crate for write-side
-- stores (see `010_create_operation_reservations.sql`), the read-side SPI's
-- parameter is `tenant: &str`, never `Option<&str>`, so there is no systemwide
-- scope to model here and no partial-index pair is needed (PROD-014B AD-1).
--
-- `offset_value`, never `offset`: `OFFSET` is a reserved word in PostgreSQL.
--
-- `updated_at` is operational only; no query reads it.

CREATE TABLE IF NOT EXISTS projection_offsets (
    projection_id VARCHAR(255) NOT NULL,
    tag           VARCHAR(255) NOT NULL,
    tenant        VARCHAR(255) NOT NULL,
    offset_value  BIGINT       NOT NULL,
    updated_at    TIMESTAMPTZ  NOT NULL DEFAULT NOW(),

    CONSTRAINT projection_offsets_identity
        PRIMARY KEY (projection_id, tag, tenant)
);
