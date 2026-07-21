-- Enforce tenant scoping at the schema level.
--
-- The tenant_id columns were originally nullable, which permitted a shared,
-- un-scoped NULL partition. Combined with the backend fail-closed guard
-- (empty-string tenants are rejected as MissingTenant), tenant_id is now made
-- NOT NULL so the database itself refuses to persist an un-scoped row.
--
-- This is a forward migration: tables 001-003 are already shipped, so their
-- CREATE TABLE IF NOT EXISTS statements are no-ops on deployed databases. The
-- ALTER statements below bring both fresh and existing databases to the same
-- NOT NULL state and are idempotent (re-running SET NOT NULL on an already
-- NOT NULL column succeeds as a no-op).
ALTER TABLE events ALTER COLUMN tenant_id SET NOT NULL;
ALTER TABLE aggregates ALTER COLUMN tenant_id SET NOT NULL;
ALTER TABLE snapshots ALTER COLUMN tenant_id SET NOT NULL;
