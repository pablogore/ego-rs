-- Read-side processing claims: one row per (projection_id, tag, tenant),
-- naming the worker that currently holds single valid processing ownership
-- of that stream, until when, and under which fencing token.
--
-- IDENTITY — the primary key is byte-for-byte `projection_offsets`' identity
-- (013), which is the claim identity PROD-014C D-1 fixes. `tenant` is NOT NULL
-- for 013's reason: the read-side SPI's parameter is `tenant: &str`, never
-- `Option<&str>`, so there is no systemwide scope to model and no partial-index
-- pair is needed.
--
-- RELEASE IS AN EXPIRY, NOT A DELETE. A released claim is a row whose
-- `lease_until` is set to now — never removed — keeping the fencing token
-- strictly monotone across the release boundary and needing no `state` column.
CREATE TABLE IF NOT EXISTS projection_claims (
    projection_id VARCHAR(255) NOT NULL,
    tag           VARCHAR(255) NOT NULL,
    tenant        VARCHAR(255) NOT NULL,
    owner_id      VARCHAR(255) NOT NULL,
    fencing_token BIGINT       NOT NULL,
    lease_until   TIMESTAMPTZ  NOT NULL,
    claimed_at    TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    CONSTRAINT projection_claims_identity
        PRIMARY KEY (projection_id, tag, tenant),
    CONSTRAINT projection_claims_fencing_token_positive
        CHECK (fencing_token > 0)
);
