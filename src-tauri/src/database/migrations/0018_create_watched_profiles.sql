-- Watched profiles: a hand-curated list of LinkedIn profiles the user wants a
-- comment run to check for new posts, in addition to the main feed. This is a
-- global list owned by the app (not scoped to a pitch and not per-prospect) —
-- the user adds/removes profile links in the Profile tab, and a comment run
-- visits each one's recent activity first (prioritized over the feed).
--
-- `linkedin_url` is the profile URL the user pasted, deduped so the same person
-- can't be watched twice. `name` is an optional label for the list UI.
CREATE TABLE watched_profiles (
    id           INTEGER PRIMARY KEY,
    linkedin_url TEXT NOT NULL UNIQUE,
    name         TEXT NOT NULL DEFAULT '',
    created_at   TEXT NOT NULL DEFAULT (datetime('now'))
);
