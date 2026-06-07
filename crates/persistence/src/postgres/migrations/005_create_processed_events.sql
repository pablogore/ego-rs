-- Migration: Create processed events table
-- This migration creates the table for storing processed events for deduplication

CREATE TABLE IF NOT EXISTS processed_events (
    projection_name VARCHAR NOT NULL,
    event_id UUID NOT NULL,
    processed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (projection_name, event_id)
);

-- Create indexes for performance
CREATE INDEX IF NOT EXISTS idx_processed_events_projection ON processed_events (projection_name);
CREATE INDEX IF NOT EXISTS idx_processed_events_processed_at ON processed_events (processed_at);