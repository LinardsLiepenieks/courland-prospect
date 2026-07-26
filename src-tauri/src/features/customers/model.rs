use rusqlite::Row;
use serde::Serialize;

/// A customer profile (ICP): one kind of buyer for the single product.
///
/// The three text fields are what the draft prompt steers on, and they are
/// deliberately distinct jobs — a goal alone tells the model the destination but
/// not the terrain, so it can't discriminate between two snippets:
///   - `who_they_are` — how you recognize one (role, company shape, where found)
///   - `pain`         — what they care about / what's broken for them today
///   - `goal`         — what you want out of a thread with them
///
/// `goal` is free prose rather than a pointer at a pipeline stage: the pipeline
/// describes *your* process and is shared by every customer, while the goal
/// describes intent. The prompt reads the prospect's stage to know where the
/// thread sits and the goal to know where it's headed.
///
/// Output-only — returned by commands, never accepted as input.
#[derive(Debug, Serialize)]
pub struct Customer {
    pub id: i64,
    pub name: String,
    pub who_they_are: String,
    pub pain: String,
    pub goal: String,
    pub created_at: String,
}

impl Customer {
    /// Map a DB row (columns as selected by the repository) into a `Customer`.
    pub(super) fn from_row(row: &Row) -> rusqlite::Result<Self> {
        Ok(Customer {
            id: row.get("id")?,
            name: row.get("name")?,
            who_they_are: row.get("who_they_are")?,
            pain: row.get("pain")?,
            goal: row.get("goal")?,
            created_at: row.get("created_at")?,
        })
    }
}
