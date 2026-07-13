use rusqlite::Row;
use serde::Serialize;

/// The user's global profile — who they are and what they're building — used as
/// context when drafting/polishing outreach. A singleton: exactly one row exists
/// (id = 1). Output-only — returned by commands, never accepted as input.
#[derive(Debug, Serialize)]
pub struct Profile {
    /// Who the user is — background, role, voice.
    pub who_are_you: String,
    /// What the user is building — the product, its shape and audience.
    pub what_building: String,
    pub updated_at: String,
}

impl Profile {
    /// Map a DB row (columns as selected by the repository) into a `Profile`.
    pub(super) fn from_row(row: &Row) -> rusqlite::Result<Self> {
        Ok(Profile {
            who_are_you: row.get("who_are_you")?,
            what_building: row.get("what_building")?,
            updated_at: row.get("updated_at")?,
        })
    }
}
