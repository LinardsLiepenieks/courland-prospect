//! All SQL for snippets. Functions take `&Connection` and return domain types;
//! the command layer owns connection locking. Kept free of Tauri types so it
//! stays unit-testable against an in-memory database.

use rusqlite::{params, Connection};

use super::model::Snippet;

const COLUMNS: &str = "id, pitch_id, name, content, created_at";

/// List one owner's snippets, newest first. `Some(id)` returns that pitch's
/// snippets; `None` returns the profile snippets (rows with a NULL `pitch_id`).
/// The null-safe `IS` operator does both: it matches a pitch id for a bound
/// value and NULL rows for a bound NULL, so the two scopes never mix.
pub(crate) fn list(conn: &Connection, pitch_id: Option<i64>) -> rusqlite::Result<Vec<Snippet>> {
    let sql = format!(
        "SELECT {COLUMNS} FROM snippets WHERE pitch_id IS ?1 \
         ORDER BY created_at DESC, id DESC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([pitch_id], Snippet::from_row)?;
    rows.collect()
}

fn get(conn: &Connection, id: i64) -> rusqlite::Result<Snippet> {
    let sql = format!("SELECT {COLUMNS} FROM snippets WHERE id = ?1");
    conn.query_row(&sql, [id], Snippet::from_row)
}

/// Insert a blank snippet for `pitch_id` (or the profile when `None`) and return
/// it. Name and content default to empty — the frontend adds a blank card, then
/// fills it in via autosaved `update`s.
pub(super) fn create(conn: &Connection, pitch_id: Option<i64>) -> rusqlite::Result<Snippet> {
    conn.execute("INSERT INTO snippets (pitch_id) VALUES (?1)", [pitch_id])?;
    get(conn, conn.last_insert_rowid())
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
