-- Prospects: people captured from LinkedIn (via the Chrome extension) into the
-- pipeline. `linkedin_url` is the natural identity key, so it's UNIQUE — re-adding
-- the same person upserts rather than duplicating. `pitch_id` ties a prospect to
-- the pitch you're running on them; nullable and ON DELETE SET NULL as a safety
-- net. NOTE: deleting a pitch does NOT rely on SET NULL — the pitches repository
-- deletes the pitch's prospects explicitly in the same transaction, so they're
-- never stranded as invisible NULL-pitch rows.
CREATE TABLE prospects (
    id           INTEGER PRIMARY KEY,
    name         TEXT NOT NULL,
    linkedin_url TEXT NOT NULL UNIQUE,
    headline     TEXT NOT NULL DEFAULT '',
    pitch_id     INTEGER REFERENCES pitches(id) ON DELETE SET NULL,
    note         TEXT NOT NULL DEFAULT '',
    created_at   TEXT NOT NULL DEFAULT (datetime('now'))
);
