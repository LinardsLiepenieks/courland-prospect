-- Outgoing messages captured from LinkedIn chats by the Chrome extension. One row
-- per distinct message you sent to a prospect, only while they're in the pitch's
-- messaging stage. `li_key` is a stable per-message identity scraped from the
-- LinkedIn DOM (falling back to a content hash); UNIQUE per prospect so re-scraping
-- the same thread upserts rather than duplicating.
--
-- A prospect's `messages_sent` counter is DERIVED from these rows (count per
-- prospect) — the desktop UI shows it read-only; there is no manual stepper.
CREATE TABLE messages (
    id           INTEGER PRIMARY KEY,
    prospect_id  INTEGER NOT NULL REFERENCES prospects(id) ON DELETE CASCADE,
    li_key       TEXT NOT NULL,
    body         TEXT NOT NULL DEFAULT '',
    sent_at      TEXT,
    created_at   TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(prospect_id, li_key)
);

CREATE INDEX idx_messages_prospect ON messages(prospect_id);
