use rusqlite::Row;
use serde::Serialize;

/// The user's global profile — who they are — used as context when
/// drafting/polishing outreach. A singleton: exactly one row exists (id = 1).
///
/// Deliberately narrow: this is the *person writing*, not the thing being sold.
/// "What are you building" moved to [`crate::features::product`] in migration
/// 0022. Keeping them apart is what lets the commenter borrow the founder's
/// persona without any risk of a public comment reading as pitch copy.
///
/// Output-only — returned by commands, never accepted as input.
#[derive(Debug, Serialize)]
pub struct Profile {
    /// Who the user is — background, role, voice.
    pub who_are_you: String,
    pub updated_at: String,
}

impl Profile {
    /// Map a DB row (columns as selected by the repository) into a `Profile`.
    pub(super) fn from_row(row: &Row) -> rusqlite::Result<Self> {
        Ok(Profile {
            who_are_you: row.get("who_are_you")?,
            updated_at: row.get("updated_at")?,
        })
    }
}
