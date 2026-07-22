use rusqlite::Row;
use serde::Serialize;

/// One drafted comment in the review inbox: a scraped LinkedIn post plus the
/// comment the AI composed for it, carried through review and posting. Output-only
/// — returned by commands / the ingest HTTP server, never accepted as input (the
/// transport layers take plain fields). See `0020_create_comment_drafts.sql` for
/// the `status` lifecycle.
#[derive(Debug, Serialize)]
pub struct CommentDraft {
    pub id: i64,
    /// The post's canonical permalink — the per-post identity / dedup key.
    pub permalink: String,
    pub author_name: String,
    pub post_text: String,
    /// The composed comment; editable while not yet posted.
    pub comment: String,
    /// draft | queued | posting | posted | failed.
    pub status: String,
    /// Last failure reason (empty unless `status` is `failed`).
    pub error: String,
    pub created_at: String,
    /// Set only once `status` is `posted`.
    pub posted_at: Option<String>,
}

impl CommentDraft {
    /// Map a DB row (columns as the repository selects them) into a `CommentDraft`.
    pub(super) fn from_row(row: &Row) -> rusqlite::Result<Self> {
        Ok(CommentDraft {
            id: row.get("id")?,
            permalink: row.get("permalink")?,
            author_name: row.get("author_name")?,
            post_text: row.get("post_text")?,
            comment: row.get("comment")?,
            status: row.get("status")?,
            error: row.get("error")?,
            created_at: row.get("created_at")?,
            posted_at: row.get("posted_at")?,
        })
    }
}

/// The single-row commenter control record: how the app asks the extension to run
/// a scrape (the app can't push to the extension, so the extension polls this) and
/// how the UI reflects run progress. See `0020_create_comment_drafts.sql`.
#[derive(Debug, Serialize)]
pub struct CommentRun {
    /// idle | requested | scraping.
    pub status: String,
    /// The placed-draft budget for a run.
    pub count: i64,
    /// Whether a run visits watched profiles before the feed.
    pub include_watchlist: bool,
    pub updated_at: String,
}

impl CommentRun {
    pub(super) fn from_row(row: &Row) -> rusqlite::Result<Self> {
        Ok(CommentRun {
            status: row.get("status")?,
            count: row.get("count")?,
            // Stored as INTEGER 0/1 (SQLite has no bool); map to a real bool.
            include_watchlist: row.get::<_, i64>("include_watchlist")? != 0,
            updated_at: row.get("updated_at")?,
        })
    }
}
