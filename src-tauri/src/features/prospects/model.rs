use rusqlite::Row;
use serde::Serialize;

/// A prospect: a person captured from LinkedIn into the pipeline.
/// Output-only — returned by commands/HTTP, never accepted as input directly.
#[derive(Debug, Serialize)]
pub struct Prospect {
    pub id: i64,
    pub name: String,
    /// The LinkedIn profile URL — the natural identity/dedup key.
    pub linkedin_url: String,
    /// Their headline/title as scraped, if any.
    pub headline: String,
    /// The pitch being run on them. `None` if the pitch was later deleted.
    pub pitch_id: Option<i64>,
    /// The pipeline stage they're currently in. `None` if unassigned (no pitch,
    /// or the stage was deleted out from under them via the SET NULL safety net).
    pub stage_id: Option<i64>,
    /// Outreach counter shown in the messaging stage — how many messages sent.
    pub messages_sent: i64,
    /// Whether the prospect has replied and we still owe them an answer — i.e.
    /// their newest captured message is incoming. Dynamic and derived from
    /// captured messages (see `features::messages`): a reply at any stage sets
    /// it, and our answer clears it again.
    pub awaiting_reply: bool,
    pub note: String,
    pub created_at: String,
}

impl Prospect {
    /// Map a DB row (columns as selected by the repository) into a `Prospect`.
    pub(super) fn from_row(row: &Row) -> rusqlite::Result<Self> {
        Ok(Prospect {
            id: row.get("id")?,
            name: row.get("name")?,
            linkedin_url: row.get("linkedin_url")?,
            headline: row.get("headline")?,
            pitch_id: row.get("pitch_id")?,
            stage_id: row.get("stage_id")?,
            messages_sent: row.get("messages_sent")?,
            awaiting_reply: row.get("awaiting_reply")?,
            note: row.get("note")?,
            created_at: row.get("created_at")?,
        })
    }
}
