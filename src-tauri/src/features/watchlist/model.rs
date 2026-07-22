use rusqlite::Row;
use serde::Serialize;

/// A watched LinkedIn profile: one entry in the list a comment run checks for new
/// posts, besides the feed. Output-only — returned by commands/HTTP, never
/// accepted as input directly (the command layer takes plain fields).
#[derive(Debug, Serialize)]
pub struct WatchedProfile {
    pub id: i64,
    /// The LinkedIn profile URL the user pasted — the natural identity/dedup key.
    pub linkedin_url: String,
    /// An optional label for the list UI (may be empty).
    pub name: String,
    pub created_at: String,
}

impl WatchedProfile {
    /// Map a DB row (columns as selected by the repository) into a `WatchedProfile`.
    pub(super) fn from_row(row: &Row) -> rusqlite::Result<Self> {
        Ok(WatchedProfile {
            id: row.get("id")?,
            linkedin_url: row.get("linkedin_url")?,
            name: row.get("name")?,
            created_at: row.get("created_at")?,
        })
    }
}
