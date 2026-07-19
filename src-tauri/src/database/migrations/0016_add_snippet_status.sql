-- Snippet lifecycle status. Two values:
--   'approved' — a normal snippet, editable and used to compose drafts.
--   'proposed' — an AI-proposed snippet extracted verbatim from a sent message,
--                awaiting the user's approve/reject. Proposed snippets are shown
--                in a distinct color and are NEVER used to compose a draft until
--                approved.
-- Every existing snippet is a normal one, so the column defaults to 'approved'
-- and the backfill is implicit (the DEFAULT applies to all current rows).
ALTER TABLE snippets ADD COLUMN status TEXT NOT NULL DEFAULT 'approved';

-- List queries in a pitch's editor sort proposed snippets to the top and drafting
-- filters to approved-only, so both paths filter/sort on status alongside the
-- existing pitch scope. Index it together with the owner. This composite has
-- `pitch_id` as its leading column, so it fully covers every query the old
-- `pitch_id`-only index served (`WHERE pitch_id IS/= ?`) — drop that one rather
-- than maintain two overlapping indexes on every write.
DROP INDEX idx_snippets_pitch;
CREATE INDEX idx_snippets_status ON snippets(pitch_id, status);
