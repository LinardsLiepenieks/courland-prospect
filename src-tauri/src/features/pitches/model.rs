use rusqlite::Row;
use serde::Serialize;

/// A pitch: a distinct thing you're selling, that prospects attach to.
/// Output-only — returned by commands, never accepted as input.
#[derive(Debug, Serialize)]
pub struct Pitch {
    pub id: i64,
    pub name: String,
    /// What the pitch is about — the skill/angle you're selling.
    pub skill: String,
    pub created_at: String,
}

impl Pitch {
    /// Map a DB row (columns as selected by the repository) into a `Pitch`.
    pub(super) fn from_row(row: &Row) -> rusqlite::Result<Self> {
        Ok(Pitch {
            id: row.get("id")?,
            name: row.get("name")?,
            skill: row.get("skill")?,
            created_at: row.get("created_at")?,
        })
    }
}
