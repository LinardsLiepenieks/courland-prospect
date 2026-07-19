-- Two AI-derived organizing axes for snippets, plus a manual-override flag.
--
--   position — where on the conversation arc a snippet belongs, 0.0 (an opener /
--              intro) → 1.0 (a closing ask / call to book). The editor sorts by it
--              and the draft prompt composes snippets in this order. Existing rows
--              default to 0.5 (mid-arc) and are classified lazily on their next
--              edit — no eager backfill (that would burst one LLM call per snippet
--              on upgrade).
--   category — a reusable group label many snippets share (one category per
--              snippet, many snippets per category). A plain column, not its own
--              table: the "category set" for a scope is SELECT DISTINCT category,
--              and rename/merge are UPDATEs. Empty = uncategorized.
--   manual   — set when the user hand-picks a category (or otherwise organizes a
--              snippet). The background classify pass NEVER overwrites a manual
--              row, so a deliberate choice is never stomped.
--
-- All three carry constant DEFAULTs, so every existing snippet backfills in place
-- (non-destructive, like the status column before it).
ALTER TABLE snippets ADD COLUMN position REAL NOT NULL DEFAULT 0.5;
ALTER TABLE snippets ADD COLUMN category TEXT NOT NULL DEFAULT '';
ALTER TABLE snippets ADD COLUMN manual INTEGER NOT NULL DEFAULT 0;
