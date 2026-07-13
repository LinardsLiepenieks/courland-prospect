-- Pipeline stages: a per-pitch, ordered funnel a prospect moves through.
-- Each pitch owns its own stages (ON DELETE CASCADE — deleting a pitch removes
-- its pipeline). `kind` is 'standard' or 'messaging'; every pipeline has exactly
-- one 'messaging' stage, always first, where a prospect's `messages_sent` counter
-- is tracked. `position` orders the stages (0-based, ascending).
CREATE TABLE stages (
    id         INTEGER PRIMARY KEY,
    pitch_id   INTEGER NOT NULL REFERENCES pitches(id) ON DELETE CASCADE,
    name       TEXT NOT NULL,
    kind       TEXT NOT NULL DEFAULT 'standard',
    position   INTEGER NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_stages_pitch ON stages(pitch_id, position);

-- A prospect's current stage. Nullable + ON DELETE SET NULL so a stage delete
-- never deletes a prospect (the app reassigns to the previous stage before
-- deleting, but this is the safety net). `messages_sent` is the outreach counter
-- shown in the messaging stage.
ALTER TABLE prospects ADD COLUMN stage_id INTEGER REFERENCES stages(id) ON DELETE SET NULL;
ALTER TABLE prospects ADD COLUMN messages_sent INTEGER NOT NULL DEFAULT 0;

-- Backfill: seed the Full-cycle template for every existing pitch so no pitch is
-- left without a pipeline.
INSERT INTO stages (pitch_id, name, kind, position)
SELECT p.id, s.name, s.kind, s.position
FROM pitches p
JOIN (
              SELECT 'Messaged'   AS name, 'messaging' AS kind, 0 AS position
    UNION ALL SELECT 'Meeting',   'standard', 1
    UNION ALL SELECT 'Onboarding','standard', 2
    UNION ALL SELECT 'Feedback',  'standard', 3
) s;

-- Place every existing prospect in its pitch's messaging (first) stage.
UPDATE prospects
SET stage_id = (
    SELECT id FROM stages
    WHERE stages.pitch_id = prospects.pitch_id AND kind = 'messaging'
    ORDER BY position, id
    LIMIT 1
)
WHERE pitch_id IS NOT NULL;
