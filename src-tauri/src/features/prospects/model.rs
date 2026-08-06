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
    /// The customer profile they match — what steers their drafts toward a goal.
    /// `None` when unassigned, which is a valid resting state: everyone is in the
    /// same pipeline regardless, their drafts simply get no customer block.
    /// Deleting a customer profile leaves its prospects here, unassigned.
    pub customer_id: Option<i64>,
    /// The pipeline stage they're currently in. `None` only if the stage was
    /// deleted out from under them via the SET NULL safety net.
    pub stage_id: Option<i64>,
    /// Outreach counter shown in the messaging stage — how many messages sent.
    pub messages_sent: i64,
    /// Whether the prospect has replied and we still owe them an answer — i.e.
    /// their newest captured message is incoming. Dynamic and derived from
    /// captured messages (see `features::messages`): a reply at any stage sets
    /// it, and our answer clears it again.
    pub awaiting_reply: bool,
    /// When you last sent this prospect a message. Derived from captured messages
    /// alongside `messages_sent` (see `features::messages`), never set by hand.
    /// `None` means you have never messaged them — the UI ages those from
    /// `created_at` instead, so a captured-but-never-contacted prospect still
    /// goes stale rather than sitting fresh forever.
    pub last_outreach_at: Option<String>,
    /// The stage the advance analyzer thinks this prospect has outgrown into,
    /// pending your accept/dismiss. `None` when there's no open suggestion —
    /// which is the resting state; suggestions are never applied on their own.
    pub suggested_stage_id: Option<i64>,
    /// The analyzer's one-line justification, shown on the card so the
    /// suggestion can be judged without reopening the thread. Empty when there's
    /// no suggestion.
    pub suggested_reason: String,
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
            customer_id: row.get("customer_id")?,
            stage_id: row.get("stage_id")?,
            messages_sent: row.get("messages_sent")?,
            awaiting_reply: row.get("awaiting_reply")?,
            last_outreach_at: row.get("last_outreach_at")?,
            suggested_stage_id: row.get("suggested_stage_id")?,
            suggested_reason: row.get("suggested_reason")?,
            note: row.get("note")?,
            created_at: row.get("created_at")?,
        })
    }
}
