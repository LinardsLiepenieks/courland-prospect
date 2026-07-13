use rusqlite::Row;
use serde::Serialize;

/// A snippet: a named text fragment that will later compose into messages. Owned
/// by exactly one place — a pitch (`pitch_id` set) or the global profile
/// (`pitch_id` None, so it reads as a profile-origin snippet). Output-only —
/// returned by commands, never accepted as input.
#[derive(Debug, Serialize)]
pub struct Snippet {
    pub id: i64,
    /// Owning pitch, or `None` when the snippet belongs to the global profile.
    pub pitch_id: Option<i64>,
    pub name: String,
    pub content: String,
    pub created_at: String,
}

impl Snippet {
    /// Map a DB row (columns as selected by the repository) into a `Snippet`.
    pub(super) fn from_row(row: &Row) -> rusqlite::Result<Self> {
        Ok(Snippet {
            id: row.get("id")?,
            pitch_id: row.get("pitch_id")?,
            name: row.get("name")?,
            content: row.get("content")?,
            created_at: row.get("created_at")?,
        })
    }
}
