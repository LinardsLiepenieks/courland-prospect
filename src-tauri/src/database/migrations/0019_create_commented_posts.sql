-- Commented posts: the durable record of LinkedIn posts a comment run has
-- already handled (drafted a comment for and placed in the post's comment box
-- for review). A comment run reads this set to skip posts it has already
-- surfaced, so re-running never re-drafts the same post — "check for new posts
-- we haven't commented on yet." A post is recorded when its draft is PLACED for
-- review, not when the user actually submits the comment.
--
-- `permalink` is the post URL (the natural per-post identity), deduped so a
-- replay can't double-insert.
CREATE TABLE commented_posts (
    id           INTEGER PRIMARY KEY,
    permalink    TEXT NOT NULL UNIQUE,
    commented_at TEXT NOT NULL DEFAULT (datetime('now'))
);
