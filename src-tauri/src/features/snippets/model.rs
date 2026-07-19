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
    /// Lifecycle status: `"approved"` (a normal, usable snippet) or `"proposed"`
    /// (an AI-proposed snippet awaiting the user's approve/reject — shown in a
    /// distinct color and excluded from drafting until approved).
    pub status: String,
    /// Where on the conversation arc this snippet belongs: 0.0 (an opener/intro)
    /// → 1.0 (a closing ask). AI-derived; the primary editor sort and the order
    /// drafts compose in. 0.5 until classified.
    pub position: f64,
    /// A reusable group label many snippets share (empty = uncategorized).
    /// AI-derived, unless the user set it by hand (see `manual`).
    pub category: String,
    /// Set when the user hand-picked the category. The background classify pass
    /// never overwrites a manual snippet.
    pub manual: bool,
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
            status: row.get("status")?,
            position: row.get("position")?,
            category: row.get("category")?,
            manual: row.get("manual")?,
            created_at: row.get("created_at")?,
        })
    }
}
