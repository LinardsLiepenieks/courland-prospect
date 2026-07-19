-- LinkedIn DOM selector OVERRIDES for the Chrome extension's self-heal.
--
-- The extension ships compiled-in default selectors. When LinkedIn rotates its
-- DOM and a selector stops matching, the extension sends the live page to
-- `POST /heal-selectors`, which asks the local Claude Code CLI for a replacement
-- and stores it here. `GET /selectors` serves these overrides, which the
-- extension merges over its defaults at startup — so a fix takes effect without
-- rebuilding/reloading the extension.
--
-- A singleton (id = 1), like `profile`: a single JSON object of {key: value}
-- overrides, where value is a CSS-selector string or an array of fallback
-- strings. Empty `{}` means "no overrides — use the extension's defaults".
CREATE TABLE selectors (
    id         INTEGER PRIMARY KEY CHECK (id = 1),
    overrides  TEXT NOT NULL DEFAULT '{}',
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

INSERT INTO selectors (id, overrides) VALUES (1, '{}');
