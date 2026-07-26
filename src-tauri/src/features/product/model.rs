use rusqlite::Row;
use serde::Serialize;

/// The single product being sold — the invariant half of every message. Paired
/// with a customer profile (who you're talking to and what you want from them),
/// this is what the AI composes from. A singleton: exactly one row exists
/// (id = 1). Output-only — returned by commands, never accepted as input.
#[derive(Debug, Serialize)]
pub struct Product {
    /// What the product is called. May be empty on a fresh install.
    pub name: String,
    /// The long-form product story: what it is, who it's for, how it works, why
    /// it beats the alternative. The AI's single source of product truth.
    pub description: String,
    pub updated_at: String,
}

impl Product {
    /// Map a DB row (columns as selected by the repository) into a `Product`.
    pub(super) fn from_row(row: &Row) -> rusqlite::Result<Self> {
        Ok(Product {
            name: row.get("name")?,
            description: row.get("description")?,
            updated_at: row.get("updated_at")?,
        })
    }
}
