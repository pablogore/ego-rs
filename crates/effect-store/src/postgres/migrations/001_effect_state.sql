-- PROD-002 Phase 5 (AD-10): ego-effect-store's own migration sequence,
-- starting at 001 — separate from ego-persistence's 001-006 (different
-- crate, different tables, no shared version ledger).
--
-- claim_owner/claim_expires_at/claim_epoch back the multi-node lease model
-- (design.md AD-2/§3.1): claim_owner + a live claim_expires_at guard every
-- transition; claim_epoch is observability-only (AD-14), never checked in a
-- guard (§3.1's accepted G2 limitation).
CREATE TABLE IF NOT EXISTS effect_state (
    effect_id UUID PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    effect_type TEXT NOT NULL,
    destination TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    payload BYTEA NOT NULL,
    attempt INTEGER NOT NULL,
    state TEXT NOT NULL,
    next_at TIMESTAMPTZ,
    terminal_reason TEXT,
    settled_at TIMESTAMPTZ,
    claim_owner UUID,
    claim_expires_at TIMESTAMPTZ,
    claim_epoch BIGINT NOT NULL DEFAULT 0
);

-- Speeds up claim_due's due-row scan (pending/retryable_failed ordered by
-- next_at) and recover_in_flight's expired-lease scan.
CREATE INDEX IF NOT EXISTS idx_effect_state_claimable
    ON effect_state (next_at)
    WHERE state IN ('pending', 'retryable_failed');

CREATE INDEX IF NOT EXISTS idx_effect_state_in_flight_lease
    ON effect_state (claim_expires_at)
    WHERE state = 'in_flight';
