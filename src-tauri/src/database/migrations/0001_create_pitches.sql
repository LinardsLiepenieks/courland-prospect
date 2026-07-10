-- v1 — pitches: what you're selling. Prospects attach to these later.
CREATE TABLE pitches (
    id          INTEGER PRIMARY KEY,
    name        TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);
