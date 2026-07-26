//! All SQL for snippets. Functions take `&Connection` and return domain types;
//! the command layer owns connection locking. Kept free of Tauri types so it
//! stays unit-testable against an in-memory database.
//!
//! One library, so nothing here takes a scope. That also retires the old
//! `copy`-a-snippet-between-scopes path, which existed only to work around the
//! split.

use rusqlite::{params, Connection, OptionalExtension};

use super::model::Snippet;

const COLUMNS: &str = "id, name, content, status, position, category, manual, created_at";

/// A normal, usable snippet: editable and available to compose drafts.
pub(crate) const APPROVED: &str = "approved";
/// An AI-proposed snippet, extracted verbatim from a sent message and awaiting the
/// user's approve/reject. Shown in a distinct color; never composes a draft until
/// approved.
pub(crate) const PROPOSED: &str = "proposed";

/// The library, as the editor shows it — proposed snippets first (they're the
/// actionable thing), then approved ones in conversation-arc order (`position`
/// ascending: openers → closers).
pub(crate) fn list(conn: &Connection) -> rusqlite::Result<Vec<Snippet>> {
    let sql = format!(
        "SELECT {COLUMNS} FROM snippets \
         ORDER BY (status = '{PROPOSED}') DESC, position ASC, created_at DESC, id DESC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], Snippet::from_row)?;
    rows.collect()
}

/// The approved snippets — the drafting material. Proposed ones are excluded so
/// an unreviewed proposal can never leak into a reply; only after the user
/// approves it does it join this set. Returned in conversation-arc order, which
/// is the order the draft prompt presents them in.
pub(crate) fn list_approved(conn: &Connection) -> rusqlite::Result<Vec<Snippet>> {
    let sql = format!(
        "SELECT {COLUMNS} FROM snippets WHERE status = '{APPROVED}' \
         ORDER BY position ASC, created_at DESC, id DESC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], Snippet::from_row)?;
    rows.collect()
}

/// Every approved, content-bearing snippet, newest-first. The commenter uses
/// these purely as a VOICE/STYLE corpus — samples of how the founder writes,
/// never content to reuse. Blank cards and unreviewed proposals are excluded, so
/// the caller gets only usable prose.
pub(crate) fn list_all_approved(conn: &Connection) -> rusqlite::Result<Vec<Snippet>> {
    let sql = format!(
        "SELECT {COLUMNS} FROM snippets WHERE status = '{APPROVED}' AND trim(content) != '' \
         ORDER BY created_at DESC, id DESC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], Snippet::from_row)?;
    rows.collect()
}

/// The distinct non-empty category labels already in use — the set the classify
/// pass shows the model so it reuses a fitting category instead of minting a
/// near-duplicate.
pub(crate) fn existing_categories(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT category FROM snippets WHERE trim(category) != '' ORDER BY category",
    )?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    rows.collect()
}

/// Every non-blank snippet content a proposal is deduped against — all statuses,
/// so already-approved and already-proposed lines alike block a duplicate. Read
/// and compared under the same lock as the proposing insert, so concurrent
/// propose passes can't each insert the same content.
pub(crate) fn dedup_contents(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT content FROM snippets WHERE trim(content) != ''")?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    rows.collect()
}

/// Insert a `proposed` snippet with its extracted name + content, and return it.
/// Unlike `create` (which starts blank for the editor), a proposal arrives fully
/// formed from the analysis of a sent message.
pub(crate) fn create_proposed(
    conn: &Connection,
    name: &str,
    content: &str,
) -> rusqlite::Result<Snippet> {
    conn.execute(
        "INSERT INTO snippets (name, content, status) VALUES (?1, ?2, ?3)",
        params![name, content, PROPOSED],
    )?;
    get(conn, conn.last_insert_rowid())
}

/// Approve a proposed snippet: flip its status to `approved`, after which it is a
/// normal snippet — editable and used to compose drafts. Returns the updated row,
/// or `None` when no row matched. Idempotent: approving an already-approved
/// snippet is a harmless no-op that still returns it.
pub(super) fn approve(conn: &Connection, id: i64) -> rusqlite::Result<Option<Snippet>> {
    let changed = conn.execute(
        "UPDATE snippets SET status = ?1 WHERE id = ?2",
        params![APPROVED, id],
    )?;
    if changed == 0 {
        return Ok(None);
    }
    get(conn, id).map(Some)
}

/// Fetch a snippet by id, erroring if it's absent — the internal getter that
/// `create`/`update`/`approve`/… use to return the row they just wrote.
fn get(conn: &Connection, id: i64) -> rusqlite::Result<Snippet> {
    find(conn, id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)
}

/// Fetch a snippet by id, or `None` when it doesn't exist (deleted mid-flight).
/// Used by the classify pass to read the current row — including its `manual`,
/// `status`, and latest `content` — before deciding whether to write.
pub(crate) fn find(conn: &Connection, id: i64) -> rusqlite::Result<Option<Snippet>> {
    let sql = format!("SELECT {COLUMNS} FROM snippets WHERE id = ?1");
    conn.query_row(&sql, [id], Snippet::from_row).optional()
}

/// Insert a blank snippet and return it. Name and content default to empty — the
/// frontend adds a blank card, then fills it in via autosaved `update`s.
pub(super) fn create(conn: &Connection) -> rusqlite::Result<Snippet> {
    conn.execute("INSERT INTO snippets DEFAULT VALUES", [])?;
    get(conn, conn.last_insert_rowid())
}

/// Update a snippet's name + content in place. Returns `None` when no row matched.
pub(super) fn update(
    conn: &Connection,
    id: i64,
    name: &str,
    content: &str,
) -> rusqlite::Result<Option<Snippet>> {
    let changed = conn.execute(
        "UPDATE snippets SET name = ?1, content = ?2 WHERE id = ?3",
        params![name, content, id],
    )?;
    if changed == 0 {
        return Ok(None);
    }
    get(conn, id).map(Some)
}

pub(super) fn delete(conn: &Connection, id: i64) -> rusqlite::Result<usize> {
    conn.execute("DELETE FROM snippets WHERE id = ?1", [id])
}

/// Write the AI-derived classification (arc `position` + `category`) for a snippet,
/// but ONLY when it isn't a manual row — the `manual = 0` guard is what makes the
/// background pass unable to stomp a category the user picked by hand. Returns the
/// updated row, or `None` when nothing matched (missing id, or a manual row left
/// untouched). Runs under the caller's lock alongside its freshness checks.
pub(crate) fn set_classification(
    conn: &Connection,
    id: i64,
    position: f64,
    category: &str,
) -> rusqlite::Result<Option<Snippet>> {
    let changed = conn.execute(
        "UPDATE snippets SET position = ?1, category = ?2 WHERE id = ?3 AND manual = 0",
        params![position, category, id],
    )?;
    if changed == 0 {
        return Ok(None);
    }
    get(conn, id).map(Some)
}

/// Force-write an AI classification, overriding a manual pin and resetting the row
/// to auto (`manual = 0`). Unlike `set_classification` (which the per-edit auto pass
/// uses and which the `manual = 0` guard protects), this is the "re-score & re-organize
/// everything" path: it deliberately overwrites a hand-picked category and hands the
/// snippet back to auto-classification. Returns the updated row, or `None` when no row
/// matched (deleted mid-batch).
pub(crate) fn force_classification(
    conn: &Connection,
    id: i64,
    position: f64,
    category: &str,
) -> rusqlite::Result<Option<Snippet>> {
    let changed = conn.execute(
        "UPDATE snippets SET position = ?1, category = ?2, manual = 0 WHERE id = ?3",
        params![position, category, id],
    )?;
    if changed == 0 {
        return Ok(None);
    }
    get(conn, id).map(Some)
}

/// Set a snippet's category by hand. A non-empty category marks the row `manual`
/// (the auto pass will leave it alone); clearing it back to empty un-sets `manual`,
/// re-enabling auto-classification. Returns the updated row, or `None` when no row
/// matched.
pub(super) fn set_category(
    conn: &Connection,
    id: i64,
    category: &str,
) -> rusqlite::Result<Option<Snippet>> {
    let manual = !category.trim().is_empty();
    let changed = conn.execute(
        "UPDATE snippets SET category = ?1, manual = ?2 WHERE id = ?3",
        params![category, manual, id],
    )?;
    if changed == 0 {
        return Ok(None);
    }
    get(conn, id).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::migrations;

    fn setup() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        migrations::run(&mut conn).unwrap();
        conn
    }

    #[test]
    fn create_starts_blank_then_update_fills_it() {
        let conn = setup();
        let s = create(&conn).unwrap();
        assert!(s.id > 0);
        assert_eq!(s.name, "");
        assert_eq!(s.content, "");
        assert!(!s.created_at.is_empty());

        let updated = update(&conn, s.id, "Intro", "Hi there").unwrap().unwrap();
        assert_eq!(updated.id, s.id);
        assert_eq!(updated.name, "Intro");
        assert_eq!(updated.content, "Hi there");
        assert_eq!(updated.created_at, s.created_at);
    }

    #[test]
    fn update_missing_returns_none() {
        let conn = setup();
        assert!(update(&conn, 999, "x", "y").unwrap().is_none());
    }

    #[test]
    fn delete_removes_row_and_missing_returns_zero() {
        let conn = setup();
        let s = create(&conn).unwrap();
        assert_eq!(delete(&conn, s.id).unwrap(), 1);
        assert!(list(&conn).unwrap().is_empty());
        assert_eq!(delete(&conn, 999).unwrap(), 0);
    }

    #[test]
    fn proposed_snippets_sort_first_and_are_excluded_from_approved_list() {
        let conn = setup();
        let approved = create(&conn).unwrap();
        update(&conn, approved.id, "Kept", "we ship weekly").unwrap();
        let proposed = create_proposed(&conn, "New", "we are SOC2 compliant").unwrap();
        assert_eq!(proposed.status, PROPOSED);

        // The editor list surfaces the proposal at the top, then the approved ones.
        let listed = list(&conn).unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].id, proposed.id, "proposed sorts to the top");
        assert_eq!(listed[1].id, approved.id);

        // Drafting material never includes an unreviewed proposal.
        let approved_only = list_approved(&conn).unwrap();
        assert_eq!(approved_only.len(), 1);
        assert_eq!(approved_only[0].id, approved.id);
    }

    #[test]
    fn approve_flips_status_into_the_drafting_set() {
        let conn = setup();
        let p = create_proposed(&conn, "New", "we are SOC2 compliant").unwrap();
        assert!(list_approved(&conn).unwrap().is_empty());

        let approved = approve(&conn, p.id).unwrap().unwrap();
        assert_eq!(approved.status, APPROVED);
        assert_eq!(approved.content, "we are SOC2 compliant");
        let drafting = list_approved(&conn).unwrap();
        assert_eq!(drafting.len(), 1);
        assert_eq!(drafting[0].id, p.id);

        assert!(approve(&conn, 999).unwrap().is_none());
    }

    #[test]
    fn dedup_contents_spans_statuses_and_skips_blanks() {
        let conn = setup();
        let a = create(&conn).unwrap();
        update(&conn, a.id, "A", "approved line").unwrap();
        create_proposed(&conn, "P", "proposed line").unwrap();
        create(&conn).unwrap(); // blank card — excluded

        let mut contents = dedup_contents(&conn).unwrap();
        contents.sort();
        assert_eq!(contents, vec!["approved line", "proposed line"]);
    }

    #[test]
    fn new_snippets_default_to_mid_arc_uncategorized_and_auto() {
        let conn = setup();
        let s = create(&conn).unwrap();
        assert_eq!(s.position, 0.5);
        assert_eq!(s.category, "");
        assert!(!s.manual);
    }

    #[test]
    fn set_classification_writes_position_and_category_but_not_over_manual() {
        let conn = setup();
        let s = create(&conn).unwrap();
        update(&conn, s.id, "S", "we ship weekly").unwrap();

        // Auto pass classifies an un-pinned snippet.
        let out = set_classification(&conn, s.id, 0.8, "Timeline").unwrap().unwrap();
        assert_eq!(out.position, 0.8);
        assert_eq!(out.category, "Timeline");

        // User pins a category by hand → manual.
        set_category(&conn, s.id, "Cadence").unwrap().unwrap();

        // A later auto pass must NOT overwrite the manual row (returns None).
        assert!(set_classification(&conn, s.id, 0.2, "Intro").unwrap().is_none());
        let after = find(&conn, s.id).unwrap().unwrap();
        assert_eq!(after.category, "Cadence", "manual category survives the auto pass");
        assert_eq!(after.position, 0.8, "manual guard leaves position untouched too");
    }

    #[test]
    fn force_classification_overrides_a_manual_pin_and_resets_to_auto() {
        let conn = setup();
        let s = create(&conn).unwrap();
        update(&conn, s.id, "S", "book a call?").unwrap();
        set_category(&conn, s.id, "Scheduling").unwrap();
        assert!(find(&conn, s.id).unwrap().unwrap().manual);

        let out = force_classification(&conn, s.id, 0.9, "Close").unwrap().unwrap();
        assert_eq!(out.position, 0.9);
        assert_eq!(out.category, "Close");
        assert!(!out.manual, "forced reclassify hands the row back to auto");

        // A later normal auto pass can now touch it again (no longer pinned).
        assert!(set_classification(&conn, s.id, 0.85, "Closing").unwrap().is_some());

        assert!(force_classification(&conn, 999, 0.5, "X").unwrap().is_none());
    }

    #[test]
    fn set_category_toggles_manual_and_clearing_re_enables_auto() {
        let conn = setup();
        let s = create(&conn).unwrap();

        let pinned = set_category(&conn, s.id, "Security").unwrap().unwrap();
        assert_eq!(pinned.category, "Security");
        assert!(pinned.manual, "picking a category pins the snippet");

        let cleared = set_category(&conn, s.id, "").unwrap().unwrap();
        assert_eq!(cleared.category, "");
        assert!(!cleared.manual, "blanking the category un-pins it");

        assert!(set_category(&conn, 999, "X").unwrap().is_none());
    }

    #[test]
    fn existing_categories_are_distinct_and_non_empty() {
        let conn = setup();
        let a = create(&conn).unwrap();
        set_category(&conn, a.id, "Security").unwrap();
        let b = create(&conn).unwrap();
        set_category(&conn, b.id, "Security").unwrap(); // duplicate — collapses
        let c = create(&conn).unwrap();
        set_category(&conn, c.id, "Pricing").unwrap();
        create(&conn).unwrap(); // uncategorized — excluded

        assert_eq!(existing_categories(&conn).unwrap(), vec!["Pricing", "Security"]);
    }

    #[test]
    fn lists_are_ordered_by_arc_position() {
        let conn = setup();
        let closer = create(&conn).unwrap();
        update(&conn, closer.id, "Close", "book a call?").unwrap();
        set_classification(&conn, closer.id, 0.9, "CTA").unwrap();
        let intro = create(&conn).unwrap();
        update(&conn, intro.id, "Intro", "saw your post").unwrap();
        set_classification(&conn, intro.id, 0.1, "Opener").unwrap();

        for listed in [list(&conn).unwrap(), list_approved(&conn).unwrap()] {
            assert_eq!(listed[0].id, intro.id, "opener (low position) first");
            assert_eq!(listed[1].id, closer.id, "closer (high position) last");
        }
    }

    /// The whole library is drafting material — there is no scope to filter by,
    /// so every approved, content-bearing line reaches both the draft composer
    /// and the commenter's voice corpus.
    #[test]
    fn the_library_is_one_undivided_set() {
        let conn = setup();
        for content in ["we ship weekly", "book a call?", "we are SOC2 certified"] {
            let s = create(&conn).unwrap();
            update(&conn, s.id, "", content).unwrap();
        }
        create(&conn).unwrap(); // blank card
        create_proposed(&conn, "P", "unreviewed line").unwrap();

        assert_eq!(list(&conn).unwrap().len(), 5, "the editor sees everything");
        assert_eq!(list_approved(&conn).unwrap().len(), 4, "minus the proposal");
        assert_eq!(
            list_all_approved(&conn).unwrap().len(),
            3,
            "the voice corpus also drops the blank card"
        );
    }
}
