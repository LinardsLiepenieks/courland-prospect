-- Re-introduce `commented_posts` as the DURABLE record of posts we've actually
-- commented on. 0019 first created it, then 0020 dropped it (the inbox's
-- `comment_drafts` was meant to subsume the "already handled" set). That works
-- only while a draft row survives: `comment_drafts.exists()` skips a re-scrape of
-- a post already in the inbox — but a *posted* draft that the user later deletes
-- leaves no trace, so a later scrape would re-surface the post and could post a
-- SECOND public comment on it. This ledger is that missing "never again" record:
-- a permalink is written here when its comment is confirmed POSTED, and both the
-- scrape/draft path and the post-claim path skip permalinks present here — so
-- deleting a posted draft can't lead to a duplicate comment.
--
-- Recorded only on POSTED (not merely drafted): a dismissed, never-posted draft
-- is still allowed to resurface on a later scrape, which is the intended behavior.
--
-- `IF NOT EXISTS` because 0019 may or may not still have the table depending on
-- whether a given database ran 0020's DROP — this migration converges both to
-- "present", and never touches its rows if it's already there. `permalink` is the
-- post URL (the natural per-post identity), deduped so a replay can't double-insert.
CREATE TABLE IF NOT EXISTS commented_posts (
    id           INTEGER PRIMARY KEY,
    permalink    TEXT NOT NULL UNIQUE,
    commented_at TEXT NOT NULL DEFAULT (datetime('now'))
);
