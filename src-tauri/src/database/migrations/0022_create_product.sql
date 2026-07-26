-- v22 — the ONE product. The app sells a single thing, so the product story is
-- app-wide context rather than something re-typed per campaign: the AI reads it
-- to understand what is being sold, and every customer profile (v23) describes
-- who it's being sold TO.
--
-- Singleton (id = 1), like `profile` and `selectors`: the CHECK keeps it a
-- singleton at the schema level and the seed row means `get` always finds one,
-- so the app never branches on "not created yet".
--
-- `description` is seeded from `profile.what_building`, which held exactly this
-- text before the split — nothing is retyped and nothing is lost. `profile`
-- narrows to just "who you are" (the founder's voice/persona, which the
-- commenter needs kept separate from anything that reads as pitch copy).
CREATE TABLE product (
    id          INTEGER PRIMARY KEY CHECK (id = 1),
    name        TEXT NOT NULL DEFAULT '',
    description TEXT NOT NULL DEFAULT '',
    updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

INSERT INTO product (id, description)
SELECT 1, what_building FROM profile WHERE id = 1;

-- Defensive: if the profile singleton were somehow absent, still seed the row so
-- `get` can't fail. (Unreachable in practice — 0003 seeds it and 0008 renamed the
-- table with the row intact.)
INSERT OR IGNORE INTO product (id) VALUES (1);

ALTER TABLE profile DROP COLUMN what_building;
