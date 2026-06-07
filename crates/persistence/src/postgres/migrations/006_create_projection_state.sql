-- Migration: Create projection state table
-- This migration creates the table for storing projection state information

CREATE TABLE IF NOT EXISTS projection_state (
    projection_name VARCHAR NOT NULL,
    tag VARCHAR NOT NULL,
    version BIGINT NOT NULL,
    state JSONB,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (projection_name, tag)
);

-- Create indexes for performance
CREATE INDEX IF NOT EXISTS idx_projection_state_projection_tag ON projection_state (projection_name, tag);
CREATE INDEX IF NOT EXISTS idx_projection_state_updated_at ON projection_state (updated_at);