-- Record which client operation produced each event.
--
-- Nullable, and it will stay nullable: not every event comes from an
-- externally-keyed operation. An event replayed by a projection, or one produced
-- by an internal timer, has no client operation behind it, so a mandatory column
-- would force adapters to invent a value for the absence — which is the shape of
-- mistake that turns "no key" into a key that looks real.
--
-- VARCHAR(255) against a key that OperationKey caps at 255 *bytes*. The units
-- differ and the bound still holds in the safe direction: Postgres counts
-- characters, and a 255-byte UTF-8 string is at most 255 characters, so every key
-- the domain admits fits. The reverse is what would not hold — a 255-character
-- multi-byte string exceeds 255 bytes and the domain rejects it before it ever
-- reaches this column.
--
-- Deliberately unindexed. Nothing queries events by operation key: suppression
-- decisions read the receipt table, and adding an index for a query that does not
-- exist costs every write to serve none.

ALTER TABLE events ADD COLUMN IF NOT EXISTS operation_key VARCHAR(255);
