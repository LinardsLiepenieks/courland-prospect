-- v25 — a prospect belongs to a CUSTOMER PROFILE, not a pitch. Same rebuild
-- reasoning as v24: `pitch_id` is named by a foreign key, so it can't be dropped
-- in place, and the runner's foreign-key-off window is what keeps `DROP TABLE
-- prospects` from cascading `messages` (ON DELETE CASCADE) into oblivion.
--
-- v23 gave customers the same ids as the pitches they came from, so the
-- conversion is a straight value carry — pitch 4 becomes customer 4. A prospect
-- whose pitch was already deleted carries NULL, which stays valid: an unassigned
-- prospect is still in the pipeline, their drafts simply get no customer block.
--
-- `stage_id` keeps pointing at the same rows (v24 preserved the surviving
-- pipeline's ids and remapped everyone else), so nobody moves on the board.

CREATE TABLE prospects_new (
    id             INTEGER PRIMARY KEY,
    name           TEXT NOT NULL,
    linkedin_url   TEXT NOT NULL UNIQUE,
    headline       TEXT NOT NULL DEFAULT '',
    customer_id    INTEGER REFERENCES customers(id) ON DELETE SET NULL,
    note           TEXT NOT NULL DEFAULT '',
    stage_id       INTEGER REFERENCES stages(id) ON DELETE SET NULL,
    messages_sent  INTEGER NOT NULL DEFAULT 0,
    awaiting_reply INTEGER NOT NULL DEFAULT 0,
    created_at     TEXT NOT NULL DEFAULT (datetime('now'))
);

INSERT INTO prospects_new
    (id, name, linkedin_url, headline, customer_id, note, stage_id,
     messages_sent, awaiting_reply, created_at)
SELECT
     id, name, linkedin_url, headline, pitch_id,  note, stage_id,
     messages_sent, awaiting_reply, created_at
FROM prospects;

DROP TABLE prospects;
ALTER TABLE prospects_new RENAME TO prospects;

-- The board groups by stage. The customer index is the foreign key's child-key
-- index: without it, deleting a customer profile has to full-scan `prospects` to
-- apply `ON DELETE SET NULL`.
CREATE INDEX idx_prospects_stage ON prospects(stage_id);
CREATE INDEX idx_prospects_customer ON prospects(customer_id);
