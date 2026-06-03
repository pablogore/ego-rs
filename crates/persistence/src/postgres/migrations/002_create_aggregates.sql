-- Create aggregates table for repository pattern
CREATE TABLE IF NOT EXISTS aggregates (
    aggregate_id VARCHAR(255) PRIMARY KEY,
    tenant_id VARCHAR(255),
    version BIGINT NOT NULL,
    payload JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_aggregates_tenant ON aggregates(tenant_id);
