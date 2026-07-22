-- Reworks the LinkedIn commenter from a browser-only flow (drafts placed straight
-- into LinkedIn comment boxes) into an in-app review inbox: the app is the cockpit
-- and the extension is a headless worker driven over the loopback server.
--
-- `commented_posts` (0019) recorded only "already handled" permalinks — the old
-- flow kept the drafts themselves in the extension's chrome.storage. The inbox
-- needs the drafts persisted app-side, so that table is replaced by `comment_drafts`
-- (which subsumes the "already handled" set: a post present here, in any status, is
-- skipped by a future scrape). Dropping it is safe — it never held user-authored
-- data, only machine-derived permalinks, and this is pre-release, uncommitted work.
DROP TABLE IF EXISTS commented_posts;

-- One drafted comment awaiting review / posting. `permalink` is the post's
-- canonical URL — the natural per-post identity, deduped so a re-scrape can't
-- double-list a post and so the set doubles as the "already handled" filter.
-- `status` walks: draft (generated, editable) → queued (user approved for posting)
-- → posting (the extension claimed it) → posted | failed. `error` holds the last
-- failure reason (empty otherwise); `posted_at` is set only once posted.
CREATE TABLE comment_drafts (
    id          INTEGER PRIMARY KEY,
    permalink   TEXT NOT NULL UNIQUE,
    author_name TEXT NOT NULL DEFAULT '',
    post_text   TEXT NOT NULL DEFAULT '',
    comment     TEXT NOT NULL DEFAULT '',
    status      TEXT NOT NULL DEFAULT 'draft',
    error       TEXT NOT NULL DEFAULT '',
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    posted_at   TEXT
);

-- Single-row control record the app writes to request a scrape and the extension
-- reads to know what to do (the app can't push to the extension, so the extension
-- polls this). `status`: idle → requested (app asked for a run) → scraping (the
-- extension claimed it) → idle (done). `count` is the placed-draft budget;
-- `include_watchlist` toggles visiting watched profiles before the feed. The
-- CHECK(id = 1) makes it a singleton, mirroring the `profile` table.
CREATE TABLE comment_run (
    id                INTEGER PRIMARY KEY CHECK (id = 1),
    status            TEXT NOT NULL DEFAULT 'idle',
    count             INTEGER NOT NULL DEFAULT 20,
    include_watchlist INTEGER NOT NULL DEFAULT 1,
    updated_at        TEXT NOT NULL DEFAULT (datetime('now'))
);
INSERT INTO comment_run (id) VALUES (1);
