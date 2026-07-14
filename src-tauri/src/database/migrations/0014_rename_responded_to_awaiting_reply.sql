-- The engagement flag changes meaning: from a durable "has this prospect ever
-- replied?" to a dynamic "has the prospect replied and we still owe them an
-- answer?" — i.e. their newest captured message is incoming. It now clears once
-- we reply. Rename the column to match (RENAME COLUMN preserves the data).
ALTER TABLE prospects RENAME COLUMN responded TO awaiting_reply;

-- Backfill existing rows to the new meaning. The old value ("ever replied") is
-- stale under the new semantics, so recompute from stored messages: a prospect
-- is awaiting a reply when their newest message (greatest id — insertion order
-- tracks the extension's top-to-bottom scrape) is incoming. COALESCE handles
-- prospects with no messages, keeping the NOT NULL column at 0.
UPDATE prospects SET awaiting_reply = COALESCE((
    SELECT m.direction = 'incoming'
    FROM messages m
    WHERE m.prospect_id = prospects.id
    ORDER BY m.id DESC
    LIMIT 1
), 0);
