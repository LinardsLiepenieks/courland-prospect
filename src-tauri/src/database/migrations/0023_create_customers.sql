-- v23 — customer profiles (ICPs). One product, sold to several kinds of buyer:
-- the product never changes, but who you're talking to does, so each customer
-- profile carries what the AI needs to steer a thread for THAT buyer.
--
--   who_they_are — how you recognize one (role, company shape, where you find them)
--   pain         — what they care about / what's broken for them today
--   goal         — what you want out of a thread with them (the destination)
--
-- The goal is deliberately free prose, not a pointer at a pipeline stage: the
-- pipeline describes YOUR process and is shared by every customer, while the goal
-- describes intent. The draft prompt reads the prospect's stage to know where the
-- thread sits and the goal to know where it's headed.
--
-- Backfilled from `pitches`, REUSING THE SAME ids — so `prospects.pitch_id`
-- converts to `customer_id` as a straight value carry in v25, with no mapping
-- table. A pitch's free-text `skill` (what you were selling, and to whom) lands in
-- `who_they_are` verbatim for the user to split into who/pain/goal; nothing is
-- discarded or guessed at.
CREATE TABLE customers (
    id           INTEGER PRIMARY KEY,
    name         TEXT NOT NULL,
    who_they_are TEXT NOT NULL DEFAULT '',
    pain         TEXT NOT NULL DEFAULT '',
    goal         TEXT NOT NULL DEFAULT '',
    created_at   TEXT NOT NULL DEFAULT (datetime('now'))
);

INSERT INTO customers (id, name, who_they_are, created_at)
SELECT id, name, skill, created_at FROM pitches;
