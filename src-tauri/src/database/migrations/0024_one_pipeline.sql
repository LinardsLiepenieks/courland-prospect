-- v24 — ONE pipeline. Stages were per-pitch (`stages.pitch_id`, cascade); every
-- prospect is now relevant to the same single product, so they share one funnel
-- and `pitch_id` goes away. Which stage a prospect sits in is orthogonal to which
-- customer profile they match: the board is the process, the customer is the
-- steering.
--
-- Rebuild, not `DROP COLUMN`: SQLite refuses to drop a column named by a foreign
-- key. The runner has foreign-key enforcement OFF for the whole migration run
-- (see `migrations::run`), which is what makes this safe — with it ON, `DROP
-- TABLE stages` would perform an implicit DELETE and fire `ON DELETE SET NULL`
-- against `prospects.stage_id`, wiping every prospect's position on the board
-- before we could remap it.
--
-- WHICH pipeline survives: the one belonging to the pitch with the most
-- prospects (ties broken by lowest id) — the funnel with the most real work in
-- it. Its stage ids are preserved, so prospects already on it don't move at all.

CREATE TABLE stages_new (
    id         INTEGER PRIMARY KEY,
    name       TEXT NOT NULL,
    kind       TEXT NOT NULL DEFAULT 'standard',
    position   INTEGER NOT NULL,
    color      TEXT NOT NULL DEFAULT 'gray',
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

INSERT INTO stages_new (id, name, kind, position, color, created_at)
SELECT id, name, kind, position, color, created_at
FROM stages
WHERE pitch_id = (
    SELECT p.id FROM pitches p
    WHERE EXISTS (SELECT 1 FROM stages s WHERE s.pitch_id = p.id)
    ORDER BY (SELECT count(*) FROM prospects x WHERE x.pitch_id = p.id) DESC, p.id ASC
    LIMIT 1
);

-- No pitches (or none with stages) — seed the built-in Full-cycle template so the
-- app always has a pipeline. Colors match `color_for_position`'s palette rotation,
-- same as migration 0006's backfill.
INSERT INTO stages_new (name, kind, position, color)
SELECT * FROM (
              SELECT 'Messaged'   AS name, 'messaging' AS kind, 0 AS position, 'blue'   AS color
    UNION ALL SELECT 'Meeting',   'standard', 1, 'amber'
    UNION ALL SELECT 'Onboarding','standard', 2, 'green'
    UNION ALL SELECT 'Feedback',  'standard', 3, 'purple'
)
WHERE NOT EXISTS (SELECT 1 FROM stages_new);

-- Remap everyone whose stage isn't in the surviving pipeline, BEFORE the old
-- table (and its positions) is dropped. Position = how far along the funnel;
-- land each prospect on the furthest surviving stage that is no further than
-- where they were, so a remap never advances anyone.
--
-- Nearest-at-or-before rather than exact equality, because positions are NOT
-- guaranteed contiguous: deleting a stage doesn't renumber its siblings, so a
-- live pipeline can read 0, 2, 3. Under exact matching a prospect at position 1
-- would match nothing and fall through to the COALESCE — and a fallback of
-- "the last stage" would silently move a step-2-of-4 prospect into the closing
-- column, which the board renders as a won deal. The remaining fallback is the
-- opposite direction: someone earlier than every surviving stage starts at the
-- front, never at the end.
UPDATE prospects
SET stage_id = COALESCE(
    (SELECT sn.id FROM stages_new sn
     WHERE sn.position <= (SELECT s.position FROM stages s WHERE s.id = prospects.stage_id)
     ORDER BY sn.position DESC, sn.id DESC
     LIMIT 1),
    (SELECT id FROM stages_new ORDER BY position, id LIMIT 1)
)
WHERE stage_id IS NOT NULL
  AND stage_id NOT IN (SELECT id FROM stages_new);

-- With one shared pipeline every prospect belongs somewhere, so anyone who had no
-- stage at all (captured without a pitch) starts at the messaging stage.
UPDATE prospects
SET stage_id = (SELECT id FROM stages_new WHERE kind = 'messaging' ORDER BY position, id LIMIT 1)
WHERE stage_id IS NULL;

DROP TABLE stages;
ALTER TABLE stages_new RENAME TO stages;

-- Every stage query is now "the pipeline, in order" — no owner to filter by.
CREATE INDEX idx_stages_position ON stages(position);
