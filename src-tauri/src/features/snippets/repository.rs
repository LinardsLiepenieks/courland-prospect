//! All SQL for snippets. Functions take `&Connection` and return domain types;
//! the command layer owns connection locking. Kept free of Tauri types so it
//! stays unit-testable against an in-memory database.

use rusqlite::{params, Connection, OptionalExtension};

use super::model::Snippet;

const COLUMNS: &str =
    "id, pitch_id, name, content, status, position, category, manual, created_at";

/// A normal, usable snippet: editable and available to compose drafts.
pub(crate) const APPROVED: &str = "approved";
/// An AI-proposed snippet, extracted verbatim from a sent message and awaiting the
/// user's approve/reject. Shown in a distinct color; never composes a draft until
/// approved.
pub(crate) const PROPOSED: &str = "proposed";

/// List one owner's snippets for the editor — proposed ones first (they're the
/// actionable thing), then approved ones in conversation-arc order (`position`
/// ascending: openers → closers). `Some(id)` returns that pitch's snippets; `None`
/// returns the profile snippets (rows with a NULL `pitch_id`). The null-safe `IS`
/// operator does both: it matches a pitch id for a bound value and NULL rows for a
/// bound NULL, so the two scopes never mix.
pub(crate) fn list(conn: &Connection, pitch_id: Option<i64>) -> rusqlite::Result<Vec<Snippet>> {
    let sql = format!(
        "SELECT {COLUMNS} FROM snippets WHERE pitch_id IS ?1 \
         ORDER BY (status = '{PROPOSED}') DESC, position ASC, created_at DESC, id DESC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([pitch_id], Snippet::from_row)?;
    rows.collect()
}

/// List one owner's approved snippets — the drafting-material view. Proposed
/// snippets are excluded so an unreviewed proposal can never leak into a reply;
/// only after the user approves it does it join this set. The conversation-arc
/// ordering happens where it's needed: `draft_reply` merges the pitch and profile
/// sets and sorts the whole thing by `position`, so this query just returns a
/// stable newest-first order for that merge to re-sort.
pub(crate) fn list_approved(
    conn: &Connection,
    pitch_id: Option<i64>,
) -> rusqlite::Result<Vec<Snippet>> {
    let sql = format!(
        "SELECT {COLUMNS} FROM snippets WHERE pitch_id IS ?1 AND status = '{APPROVED}' \
         ORDER BY created_at DESC, id DESC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([pitch_id], Snippet::from_row)?;
    rows.collect()
}

/// The distinct non-empty category labels already in use for a scope — the set the
/// classify pass shows the model so it reuses a fitting category instead of minting
/// a near-duplicate. Scoped like `list`: a pitch sees its own categories, the
/// profile sees the profile's, so the two libraries' category sets stay independent.
pub(crate) fn existing_categories(
    conn: &Connection,
    pitch_id: Option<i64>,
) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT category FROM snippets \
         WHERE pitch_id IS ?1 AND trim(category) != '' ORDER BY category",
    )?;
    let rows = stmt.query_map([pitch_id], |r| r.get::<_, String>(0))?;
    rows.collect()
}

/// Every non-blank snippet content a proposal for this pitch is deduped against:
/// the pitch's own snippets (all statuses — already-approved and already-proposed
/// alike) PLUS the global profile snippets (`pitch_id IS NULL`). The profile is
/// included because a draft for this pitch composes from pitch snippets *and*
/// profile snippets, so proposing a line already in the profile would duplicate it
/// in the draft. Read and compared under the same lock as the proposing insert, so
/// concurrent propose passes can't each insert the same content.
pub(crate) fn dedup_contents(conn: &Connection, pitch_id: i64) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT content FROM snippets \
         WHERE (pitch_id = ?1 OR pitch_id IS NULL) AND trim(content) != ''",
    )?;
    let rows = stmt.query_map([pitch_id], |r| r.get::<_, String>(0))?;
    rows.collect()
}

/// Insert a `proposed` snippet for a pitch with its extracted name + content, and
/// return it. Unlike `create` (which starts blank for the editor), a proposal
/// arrives fully formed from the analysis of a sent message. Proposals always
/// belong to a pitch — never the profile — so `pitch_id` is required.
pub(crate) fn create_proposed(
    conn: &Connection,
    pitch_id: i64,
    name: &str,
    content: &str,
) -> rusqlite::Result<Snippet> {
    insert(conn, Some(pitch_id), name, content, PROPOSED)
}

/// Insert a fully-formed snippet (name + content + status) into a scope and return
/// it — the shared body behind `create_proposed` and `copy`. (`create` uses a
/// different, all-defaults column set, so it doesn't route through here.)
fn insert(
    conn: &Connection,
    pitch_id: Option<i64>,
    name: &str,
    content: &str,
    status: &str,
) -> rusqlite::Result<Snippet> {
    conn.execute(
        "INSERT INTO snippets (pitch_id, name, content, status) VALUES (?1, ?2, ?3, ?4)",
        params![pitch_id, name, content, status],
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

/// Insert a blank snippet for `pitch_id` (or the profile when `None`) and return
/// it. Name and content default to empty — the frontend adds a blank card, then
/// fills it in via autosaved `update`s.
pub(super) fn create(conn: &Connection, pitch_id: Option<i64>) -> rusqlite::Result<Snippet> {
    conn.execute("INSERT INTO snippets (pitch_id) VALUES (?1)", [pitch_id])?;
    get(conn, conn.last_insert_rowid())
}

/// Copy a snippet into another scope as an independent duplicate. Reads the
/// source's name + content and inserts a fresh `approved` row owned by
/// `target_pitch_id` (a pitch id, or `None` for the global profile). Only name and
/// content carry over: `position`/`category`/`manual` reset to their column defaults
/// so the copy is re-classified in its new scope (categories are per-scope, so the
/// source's label may not fit the target). The two rows share nothing after this —
/// editing one never touches the other. Returns the new snippet, or `None` when the
/// source id doesn't exist.
pub(super) fn copy(
    conn: &Connection,
    source_id: i64,
    target_pitch_id: Option<i64>,
) -> rusqlite::Result<Option<Snippet>> {
    let Some(source) = find(conn, source_id)? else {
        return Ok(None);
    };
    // Only an approved snippet can be copied. Duplicating a `proposed` row would mint
    // an `approved` one, smuggling an unreviewed line past the verbatim/approve gate
    // the rest of the feature enforces — so a non-approved source is treated as
    // absent (the shipped UI only offers copy on approved cards anyway).
    if source.status != APPROVED {
        return Ok(None);
    }
    insert(conn, target_pitch_id, &source.name, &source.content, APPROVED).map(Some)
}

/// Update a snippet's name + content in place; ownership (`pitch_id`) is fixed at
/// creation and never changes. Returns `None` when no row matched.
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
        // Match the runtime open(): FK enforcement is what makes the pitch → snippet
        // cascade delete fire.
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        migrations::run(&mut conn).unwrap();
        conn
    }

    fn new_pitch(conn: &Connection) -> i64 {
        conn.execute("INSERT INTO pitches (name, skill) VALUES ('P', '')", [])
            .unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn create_starts_blank_then_update_fills_it() {
        let conn = setup();
        let s = create(&conn, None).unwrap();
        assert!(s.id > 0);
        assert_eq!(s.name, "");
        assert_eq!(s.content, "");
        assert_eq!(s.pitch_id, None);
        assert!(!s.created_at.is_empty());

        let updated = update(&conn, s.id, "Intro", "Hi there").unwrap().unwrap();
        assert_eq!(updated.id, s.id);
        assert_eq!(updated.name, "Intro");
        assert_eq!(updated.content, "Hi there");
        // Ownership + created_at are preserved by the UPDATE.
        assert_eq!(updated.pitch_id, None);
        assert_eq!(updated.created_at, s.created_at);
    }

    #[test]
    fn list_is_scoped_by_owner() {
        let conn = setup();
        let pitch = new_pitch(&conn);
        create(&conn, None).unwrap(); // profile snippet
        create(&conn, Some(pitch)).unwrap(); // pitch snippet
        create(&conn, Some(pitch)).unwrap(); // pitch snippet

        let profile = list(&conn, None).unwrap();
        assert_eq!(profile.len(), 1, "profile scope sees only its own");
        assert_eq!(profile[0].pitch_id, None);

        let pitch_snippets = list(&conn, Some(pitch)).unwrap();
        assert_eq!(pitch_snippets.len(), 2, "pitch scope sees only its own");
        assert!(pitch_snippets.iter().all(|s| s.pitch_id == Some(pitch)));
    }

    #[test]
    fn list_orders_newest_first() {
        let conn = setup();
        let a = create(&conn, None).unwrap();
        let b = create(&conn, None).unwrap();
        let all = list(&conn, None).unwrap();
        // created_at can tie at second resolution; id DESC breaks the tie.
        assert_eq!(all[0].id, b.id);
        assert_eq!(all[1].id, a.id);
    }

    #[test]
    fn update_missing_returns_none() {
        let conn = setup();
        assert!(update(&conn, 999, "x", "y").unwrap().is_none());
    }

    #[test]
    fn delete_removes_row_and_missing_returns_zero() {
        let conn = setup();
        let s = create(&conn, None).unwrap();
        assert_eq!(delete(&conn, s.id).unwrap(), 1);
        assert!(list(&conn, None).unwrap().is_empty());
        assert_eq!(delete(&conn, 999).unwrap(), 0);
    }

    #[test]
    fn copy_duplicates_name_and_content_into_target_scope_as_approved() {
        let conn = setup();
        let src_pitch = new_pitch(&conn);
        let dst_pitch = new_pitch(&conn);
        let src = create(&conn, Some(src_pitch)).unwrap();
        update(&conn, src.id, "Intro", "we ship weekly").unwrap();
        // Pin a manual category on the source — it must NOT carry into the copy.
        set_category(&conn, src.id, "Cadence").unwrap();

        // Copy into another pitch.
        let dup = copy(&conn, src.id, Some(dst_pitch)).unwrap().unwrap();
        assert_ne!(dup.id, src.id, "copy is a distinct row");
        assert_eq!(dup.pitch_id, Some(dst_pitch), "owned by the target pitch");
        assert_eq!(dup.name, "Intro");
        assert_eq!(dup.content, "we ship weekly");
        assert_eq!(dup.status, APPROVED, "copy is immediately usable");
        // Organizing axes reset for re-classification in the new scope.
        assert_eq!(dup.category, "");
        assert!(!dup.manual);
        assert_eq!(dup.position, 0.5);

        // The copy lives in the target scope, not the source.
        assert_eq!(list(&conn, Some(dst_pitch)).unwrap().len(), 1);
        assert_eq!(list(&conn, Some(src_pitch)).unwrap().len(), 1);
    }

    #[test]
    fn copy_into_profile_and_independence_from_source() {
        let conn = setup();
        let pitch = new_pitch(&conn);
        let src = create(&conn, Some(pitch)).unwrap();
        update(&conn, src.id, "Ask", "book a call?").unwrap();

        // `None` target = the global profile.
        let dup = copy(&conn, src.id, None).unwrap().unwrap();
        assert_eq!(dup.pitch_id, None);

        // Editing the copy leaves the source untouched (independent duplicate).
        update(&conn, dup.id, "Ask", "grab 15 minutes?").unwrap();
        assert_eq!(get(&conn, src.id).unwrap().content, "book a call?");
        assert_eq!(get(&conn, dup.id).unwrap().content, "grab 15 minutes?");

        // Copying a missing source is a clean None.
        assert!(copy(&conn, 999, None).unwrap().is_none());

        // A proposed source can't be copied (it would bypass the approve gate).
        let prop = create_proposed(&conn, pitch, "P", "proposed line").unwrap();
        assert!(copy(&conn, prop.id, None).unwrap().is_none());
    }

    #[test]
    fn proposed_snippets_sort_first_and_are_excluded_from_approved_list() {
        let conn = setup();
        let pitch = new_pitch(&conn);
        let approved = create(&conn, Some(pitch)).unwrap();
        update(&conn, approved.id, "Kept", "we ship weekly").unwrap();
        let proposed = create_proposed(&conn, pitch, "New", "we are SOC2 compliant").unwrap();
        assert_eq!(proposed.status, PROPOSED);

        // The editor list surfaces the proposal at the top, then the approved ones.
        let listed = list(&conn, Some(pitch)).unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].id, proposed.id, "proposed sorts to the top");
        assert_eq!(listed[1].id, approved.id);

        // Drafting material never includes an unreviewed proposal.
        let approved_only = list_approved(&conn, Some(pitch)).unwrap();
        assert_eq!(approved_only.len(), 1);
        assert_eq!(approved_only[0].id, approved.id);
    }

    #[test]
    fn approve_flips_status_into_the_drafting_set() {
        let conn = setup();
        let pitch = new_pitch(&conn);
        let p = create_proposed(&conn, pitch, "New", "we are SOC2 compliant").unwrap();
        assert!(list_approved(&conn, Some(pitch)).unwrap().is_empty());

        let approved = approve(&conn, p.id).unwrap().unwrap();
        assert_eq!(approved.status, APPROVED);
        assert_eq!(approved.content, "we are SOC2 compliant");
        // Now it composes drafts.
        let drafting = list_approved(&conn, Some(pitch)).unwrap();
        assert_eq!(drafting.len(), 1);
        assert_eq!(drafting[0].id, p.id);

        // Approving a missing row is a clean None.
        assert!(approve(&conn, 999).unwrap().is_none());
    }

    #[test]
    fn dedup_contents_spans_pitch_statuses_and_profile_and_skips_blanks() {
        let conn = setup();
        let pitch = new_pitch(&conn);
        let a = create(&conn, Some(pitch)).unwrap();
        update(&conn, a.id, "A", "approved line").unwrap();
        create_proposed(&conn, pitch, "P", "proposed line").unwrap();
        create(&conn, Some(pitch)).unwrap(); // blank card — excluded
        // A global profile snippet must also be deduped against (a draft for this
        // pitch composes from profile snippets too).
        let prof = create(&conn, None).unwrap();
        update(&conn, prof.id, "Prof", "profile line").unwrap();
        // Another pitch's snippet must NOT leak into this pitch's dedup set.
        let other = new_pitch(&conn);
        let o = create(&conn, Some(other)).unwrap();
        update(&conn, o.id, "O", "other pitch line").unwrap();

        let mut contents = dedup_contents(&conn, pitch).unwrap();
        contents.sort();
        assert_eq!(contents, vec!["approved line", "profile line", "proposed line"]);
    }

    #[test]
    fn new_snippets_default_to_mid_arc_uncategorized_and_auto() {
        let conn = setup();
        let s = create(&conn, None).unwrap();
        assert_eq!(s.position, 0.5);
        assert_eq!(s.category, "");
        assert!(!s.manual);
    }

    #[test]
    fn set_classification_writes_position_and_category_but_not_over_manual() {
        let conn = setup();
        let pitch = new_pitch(&conn);
        let s = create(&conn, Some(pitch)).unwrap();
        update(&conn, s.id, "S", "we ship weekly").unwrap();

        // Auto pass classifies an un-pinned snippet.
        let out = set_classification(&conn, s.id, 0.8, "Timeline").unwrap().unwrap();
        assert_eq!(out.position, 0.8);
        assert_eq!(out.category, "Timeline");

        // User pins a category by hand → manual.
        set_category(&conn, s.id, "Cadence").unwrap().unwrap();

        // A later auto pass must NOT overwrite the manual row (returns None).
        assert!(set_classification(&conn, s.id, 0.2, "Intro").unwrap().is_none());
        let after = get(&conn, s.id).unwrap();
        assert_eq!(after.category, "Cadence", "manual category survives the auto pass");
        assert_eq!(after.position, 0.8, "manual guard leaves position untouched too");
    }

    #[test]
    fn set_category_toggles_manual_and_clearing_re_enables_auto() {
        let conn = setup();
        let s = create(&conn, None).unwrap();

        let pinned = set_category(&conn, s.id, "Security").unwrap().unwrap();
        assert_eq!(pinned.category, "Security");
        assert!(pinned.manual, "picking a category pins the snippet");

        // Clearing it back to empty re-enables auto-classification.
        let cleared = set_category(&conn, s.id, "").unwrap().unwrap();
        assert_eq!(cleared.category, "");
        assert!(!cleared.manual, "blanking the category un-pins it");

        assert!(set_category(&conn, 999, "X").unwrap().is_none());
    }

    #[test]
    fn existing_categories_are_distinct_non_empty_and_scoped() {
        let conn = setup();
        let pitch = new_pitch(&conn);
        let a = create(&conn, Some(pitch)).unwrap();
        set_category(&conn, a.id, "Security").unwrap();
        let b = create(&conn, Some(pitch)).unwrap();
        set_category(&conn, b.id, "Security").unwrap(); // duplicate — collapses
        let c = create(&conn, Some(pitch)).unwrap();
        set_category(&conn, c.id, "Pricing").unwrap();
        create(&conn, Some(pitch)).unwrap(); // uncategorized — excluded
        let prof = create(&conn, None).unwrap();
        set_category(&conn, prof.id, "Bio").unwrap(); // profile scope — separate

        assert_eq!(existing_categories(&conn, Some(pitch)).unwrap(), vec!["Pricing", "Security"]);
        assert_eq!(existing_categories(&conn, None).unwrap(), vec!["Bio"]);
    }

    #[test]
    fn list_orders_approved_by_position_ascending() {
        let conn = setup();
        let pitch = new_pitch(&conn);
        let closer = create(&conn, Some(pitch)).unwrap();
        update(&conn, closer.id, "Close", "book a call?").unwrap();
        set_classification(&conn, closer.id, 0.9, "CTA").unwrap();
        let intro = create(&conn, Some(pitch)).unwrap();
        update(&conn, intro.id, "Intro", "saw your post").unwrap();
        set_classification(&conn, intro.id, 0.1, "Opener").unwrap();

        let listed = list(&conn, Some(pitch)).unwrap();
        assert_eq!(listed[0].id, intro.id, "opener (low position) first");
        assert_eq!(listed[1].id, closer.id, "closer (high position) last");
    }

    #[test]
    fn deleting_a_pitch_cascades_to_its_snippets() {
        let conn = setup();
        let pitch = new_pitch(&conn);
        create(&conn, Some(pitch)).unwrap();
        create(&conn, None).unwrap(); // profile snippet — must survive

        conn.execute("DELETE FROM pitches WHERE id = ?1", [pitch])
            .unwrap();

        assert!(
            list(&conn, Some(pitch)).unwrap().is_empty(),
            "pitch's snippets are cascade-deleted"
        );
        assert_eq!(
            list(&conn, None).unwrap().len(),
            1,
            "profile snippets are untouched by a pitch delete"
        );
    }
}
