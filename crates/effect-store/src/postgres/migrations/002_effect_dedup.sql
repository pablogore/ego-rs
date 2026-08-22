-- PROD-002 Phase 5 (AD-8/AD-10): scoped idempotency reservations.
--
-- PK (tenant_id, effect_type, idempotency_key) — the same scope key used by
-- `DedupScope` — is what `reserve`'s `ON CONFLICT` matches against.
CREATE TABLE IF NOT EXISTS effect_dedup (
    tenant_id TEXT NOT NULL,
    effect_type TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    effect_id UUID NOT NULL,
    fingerprint BYTEA NOT NULL,
    succeeded BOOLEAN NOT NULL DEFAULT FALSE,
    reserved_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    settled_at TIMESTAMPTZ,
    PRIMARY KEY (tenant_id, effect_type, idempotency_key)
);
