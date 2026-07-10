-- v2 — rename pitches.description → pitches.skill.
-- A pitch's descriptive field is the "skill" it's about (what you're selling),
-- which is the language the UI and domain now use. RENAME COLUMN preserves all
-- existing rows (SQLite ≥ 3.25).
ALTER TABLE pitches RENAME COLUMN description TO skill;
