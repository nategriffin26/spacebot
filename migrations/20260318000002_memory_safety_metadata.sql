-- Memory freshness / contradiction metadata for safer autonomous recall.

ALTER TABLE memories ADD COLUMN verified_at TIMESTAMP;
ALTER TABLE memories ADD COLUMN stale_after_secs INTEGER;
ALTER TABLE memories ADD COLUMN superseded_by TEXT;
ALTER TABLE memories ADD COLUMN verification_source TEXT;
ALTER TABLE memories ADD COLUMN redacted_at TIMESTAMP;

CREATE INDEX IF NOT EXISTS idx_memories_verified_at ON memories(verified_at);
CREATE INDEX IF NOT EXISTS idx_memories_superseded_by ON memories(superseded_by);
