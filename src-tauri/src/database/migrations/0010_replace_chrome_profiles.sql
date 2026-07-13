-- v10 — rework capture profiles. The previous model gave each profile its own
-- isolated --user-data-dir (the `chrome_profiles` list table). We now keep a
-- single capture --user-data-dir and switch between the Chrome profiles that
-- live inside it (--profile-directory), enumerated live from Chrome's own
-- `Local State`. So the DB no longer stores a profile list — only which
-- profile-directory is active.
--
-- Dropping the old table is safe: this feature never shipped (dev-only), and
-- the profile list is now sourced from Chrome, not the DB. The seed 'Default'
-- is Chrome's default profile-directory inside the existing capture data-dir,
-- so the user's current logged-in session stays active.
DROP TABLE IF EXISTS chrome_profiles;

CREATE TABLE capture_profile (
    id                  INTEGER PRIMARY KEY CHECK (id = 1),
    active_profile_dir  TEXT NOT NULL DEFAULT 'Default',
    updated_at          TEXT NOT NULL DEFAULT (datetime('now'))
);

INSERT INTO capture_profile (id) VALUES (1);
