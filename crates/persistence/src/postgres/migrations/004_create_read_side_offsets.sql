-- Migration: Create read-side offsets table
-- This migration creates the table for storing projection offsets

CREATE TABLE IF NOT EXISTS read_side_offsets (
    projection_id VARCHAR NOT NULL,
    tag VARCHAR NOT NULL,
    tenant_id VARCHAR NOT NULL,
    offset_version BIGINT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (projection_id, tag, tenant_id)
);

-- Create indexes for performance
CREATE INDEX IF NOT EXISTS idx_read_side_offsets_projection_tag ON read_side_offsets (projection_id, tag);
CREATE INDEX IF NOT EXISTS idx_read_side_offsets_updated_at ON read_side_offsets (updated_at);