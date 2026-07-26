-- v26 — ONE snippet library, and the end of `pitches`.
--
-- Snippets were owned by exactly one scope: a pitch, or the global profile
-- (`pitch_id NULL`). That split existed because each pitch was a separate story
-- with its own material. With a single product there is a single body of
-- material, and the AI's job changes: it sees EVERY snippet and picks the ones
-- that move THIS customer profile toward THEIR goal. Scoping the library would
-- pre-empt exactly the choice we now want the model to make.
--
-- Losing `pitch_id` also retires the whole copy-a-snippet-between-scopes
-- machinery, which only existed to work around the split.
--
-- Rebuild for the usual reason (the column is named by a foreign key), under the
-- runner's foreign-key-off window.

CREATE TABLE snippets_new (
    id         INTEGER PRIMARY KEY,
    name       TEXT NOT NULL DEFAULT '',
    content    TEXT NOT NULL DEFAULT '',
    status     TEXT NOT NULL DEFAULT 'approved',
    position   REAL NOT NULL DEFAULT 0.5,
    category   TEXT NOT NULL DEFAULT '',
    manual     INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Merge every scope into the one library. The old model let the same line exist
-- in two scopes at once (copying a snippet to another pitch duplicated the row),
-- and merging would surface both to the model as if they were separate material —
-- so collapse exact-content duplicates, case- and whitespace-insensitively.
--
-- Which row of a duplicate set survives, in order: `approved` over `proposed`
-- (never promote an unreviewed line past the approve gate, and never demote a
-- reviewed one); then the row carrying the most curation — a `manual` pin, then
-- a name, then a category; then the lowest id as a stable last resort. Keeping
-- the manual pin is the load-bearing part: it's the only thing that stops the
-- auto-classifier from overwriting a placement the user chose by hand, so
-- discarding it on an id coin-flip would lose a deliberate decision rather than
-- a redundant row.
--
-- Blank-content rows are unfilled editor cards rather than duplicates of each
-- other, so every one of them is kept.
INSERT INTO snippets_new (id, name, content, status, position, category, manual, created_at)
SELECT s.id, s.name, s.content, s.status, s.position, s.category, s.manual, s.created_at
FROM snippets s
WHERE trim(s.content) = ''
   OR s.id = (
        SELECT d.id FROM snippets d
        WHERE lower(trim(d.content)) = lower(trim(s.content))
        ORDER BY (d.status = 'approved') DESC,
                 (d.manual = 1)        DESC,
                 (trim(d.name) <> '')     DESC,
                 (trim(d.category) <> '') DESC,
                 d.id ASC
        LIMIT 1
   );

DROP TABLE snippets;
ALTER TABLE snippets_new RENAME TO snippets;

-- The editor sorts proposals to the top and drafting filters to approved-only;
-- both now filter on status alone, with no owner to lead the index.
CREATE INDEX idx_snippets_status ON snippets(status);

-- Nothing references pitches any more: stages (v24), prospects (v25) and
-- snippets (here) were all rebuilt without it, and its rows live on as
-- `customers` (v23). The concept is gone.
DROP TABLE pitches;
