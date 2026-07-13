-- Snippets: named text fragments that will later compose into messages. A snippet
-- is owned by exactly one place — a pitch (pitch_id set, cascade-deleted with the
-- pitch) or the global profile (pitch_id NULL). Single ownership by contract: a
-- snippet never belongs to more than one pitch. `name` and `content` are free text
-- and may be empty — a freshly-added snippet starts blank and is filled in via the
-- editor's autosave.
CREATE TABLE snippets (
    id         INTEGER PRIMARY KEY,
    pitch_id   INTEGER REFERENCES pitches(id) ON DELETE CASCADE,
    name       TEXT NOT NULL DEFAULT '',
    content    TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Every list query filters by owner (a pitch id, or NULL for profile snippets),
-- so index the owning column. SQLite indexes NULLs, so profile snippets benefit too.
CREATE INDEX idx_snippets_pitch ON snippets(pitch_id);
