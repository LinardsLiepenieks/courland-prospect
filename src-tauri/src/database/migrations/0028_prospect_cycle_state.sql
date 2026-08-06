-- v28 — the two pieces of per-prospect state the cycle needs: how long they've
-- been waiting on you, and whether the analyzer thinks they've outgrown their
-- current stage.
--
-- `last_outreach_at` — when you last sent them something. DERIVED from captured
-- messages (like `messages_sent` and `awaiting_reply` before it; all three are
-- recomputed together in features::messages), never set by hand. NULL means you
-- have never messaged them; the UI falls back to `created_at` for the age, so a
-- captured-but-never-contacted prospect still goes stale.
--
-- It prefers the message's scraped `sent_at`, falling back to `created_at` (our
-- capture time) when that is absent, unreadable, or later than the capture
-- itself. `datetime(x)` does the validation on the scraped value: NULL for
-- anything SQLite can't parse, and otherwise normalized to exactly
-- `datetime('now')`'s format.
--
-- `MAX()` therefore never compares strings of mixed shapes — but note WHY the
-- bare `created_at` in the ELSE branch is safe, because it isn't the `datetime()`
-- call: `messages.created_at` is `TEXT NOT NULL DEFAULT (datetime('now'))` (0007)
-- and no insert path ever supplies it (`features::messages::repository::store`
-- lists only prospect_id/li_key/body/sent_at/direction, and its ON CONFLICT
-- branch doesn't touch it), so every stored value is already canonical. An
-- insert that set `created_at` by hand — in ISO-8601 with a `T`/`Z`, say — would
-- break both this comparison and the `<= created_at` clamp above it, and would
-- need wrapping in `datetime()` here and in `LAST_OUTREACH_EXPR`.
--
-- Capture time alone is NOT good enough, which is the whole reason for the CASE:
-- adding someone to prospects posts their entire visible thread in one batch, and
-- every one of those rows is stamped `now` — so a conversation you last touched
-- three months ago would read as "messaged today" and sit permanently in the
-- fresh band, hiding precisely the person this column exists to surface.
--
-- The expression below is duplicated from `LAST_OUTREACH_EXPR` in
-- `features/messages/repository.rs` and the two MUST stay identical — an upgraded
-- database and a freshly-recomputed one disagreeing about the staleness clock
-- would be invisible and permanent. The Rust test
-- `migration_0028_backfill_matches_recompute` fails if they drift.
--
-- `suggested_stage_id` / `suggested_reason` — one pending advance suggestion per
-- prospect, written by the analyzer and cleared when the user accepts or
-- dismisses it. Columns rather than a table because there is at most one open
-- suggestion per prospect and it has no history to keep: accepting it IS the
-- stage move, and a dismissal is only meaningful until the next message arrives.
-- ON DELETE SET NULL so deleting the target stage retracts the suggestion
-- instead of stranding a card offering a move into a column that's gone.
ALTER TABLE prospects ADD COLUMN last_outreach_at TEXT;
ALTER TABLE prospects ADD COLUMN suggested_stage_id INTEGER REFERENCES stages(id) ON DELETE SET NULL;
ALTER TABLE prospects ADD COLUMN suggested_reason TEXT NOT NULL DEFAULT '';

-- Backfill from the threads already captured, so an existing board shows real
-- staleness on the first launch after the upgrade rather than treating every
-- prospect as never-contacted.
UPDATE prospects
SET last_outreach_at = (
    SELECT MAX(CASE WHEN datetime(sent_at) IS NOT NULL AND datetime(sent_at) <= created_at THEN datetime(sent_at) ELSE created_at END) FROM messages
    WHERE messages.prospect_id = prospects.id AND messages.direction = 'outgoing'
);
