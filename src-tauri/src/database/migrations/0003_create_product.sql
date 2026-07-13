-- v3 — product: the global "skills" the user gives the AI to reason about them
-- (who they are, what they're building). Not tied to any pitch — app-wide.
--
-- Singleton table: exactly one row, pinned at id = 1. The CHECK keeps it a
-- singleton at the schema level, and the seed row means `get` always finds it,
-- so the app never has to branch on "not created yet".
CREATE TABLE product (
    id            INTEGER PRIMARY KEY CHECK (id = 1),
    who_are_you   TEXT NOT NULL DEFAULT '',
    what_building TEXT NOT NULL DEFAULT '',
    updated_at    TEXT NOT NULL DEFAULT (datetime('now'))
);

INSERT INTO product (id) VALUES (1);
