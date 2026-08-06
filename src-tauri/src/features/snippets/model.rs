use rusqlite::Row;
use serde::Serialize;

/// A snippet: a named text fragment that composes into messages. There is one
/// library — a snippet has no owner, and every draft sees all of them, so the
/// model can pick whichever lines move a given customer profile toward its goal.
/// Output-only — returned by commands, never accepted as input.
#[derive(Debug, Serialize)]
pub struct Snippet {
    pub id: i64,
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
    /// The conversation STAGE this snippet belongs to — a reusable label many snippets
    /// share (empty = unstaged). AI-derived, unless the user set it by hand (see
    /// `manual`). The primary axis: the library groups by it and drafts order by it.
    pub category: String,
    /// What the snippet is ABOUT — Security, Pricing, Integrations (empty = no clear
    /// subject). Orthogonal to `category`: that says *when* in a thread a line belongs,
    /// this says *what about*. AI-derived with no hand-set counterpart, which is why
    /// there's no `manual` flag for it — a draft uses it to prefer staying on the
    /// thread's current subject.
    pub topic: String,
    /// Set when the user hand-picked the category. Covers `category` (and the `position`
    /// that belongs with it) and nothing else: the background classify pass still runs on a
    /// manual snippet and still writes its `topic`, which is never hand-set and so has
    /// nothing to protect. Enforced per column in `repository::set_classification`.
    pub manual: bool,
    pub created_at: String,
}

impl Snippet {
    /// Map a DB row (columns as selected by the repository) into a `Snippet`.
    pub(super) fn from_row(row: &Row) -> rusqlite::Result<Self> {
        Ok(Snippet {
            id: row.get("id")?,
            name: row.get("name")?,
            content: row.get("content")?,
            status: row.get("status")?,
            position: row.get("position")?,
            category: row.get("category")?,
            topic: row.get("topic")?,
            manual: row.get("manual")?,
            created_at: row.get("created_at")?,
        })
    }
}
