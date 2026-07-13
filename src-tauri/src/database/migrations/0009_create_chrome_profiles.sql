-- v9 — chrome_profiles: the dedicated capture browser's sandboxes. Each row is
-- one isolated Chrome `--user-data-dir` (a separate LinkedIn login) the user can
-- switch between from the Profile tab. Exactly one row is active at a time; the
-- ingest gate launches whichever that is.
--
-- `dir_name` is a path relative to the app-data dir. The seeded "Default" row
-- points at the pre-existing `chrome-profile` directory so upgrading installs
-- keep their current logged-in session — nothing is moved on disk.
CREATE TABLE chrome_profiles (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    label      TEXT NOT NULL,
    dir_name   TEXT NOT NULL UNIQUE,
    is_active  INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

INSERT INTO chrome_profiles (label, dir_name, is_active)
VALUES ('Default', 'chrome-profile', 1);
