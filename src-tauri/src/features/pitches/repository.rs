//! All SQL for pitches. Functions take `&Connection` and return domain types;
//! the command layer owns connection locking. Keep this module free of Tauri
//! types so it stays unit-testable against an in-memory database.

use rusqlite::{params, Connection};

use super::model::Pitch;

const COLUMNS: &str = "id, name, skill, created_at";

pub(crate) fn list(conn: &Connection) -> rusqlite::Result<Vec<Pitch>> {
    let sql = format!("SELECT {COLUMNS} FROM pitches ORDER BY created_at DESC, id DESC");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], Pitch::from_row)?;
    rows.collect()
}

pub(crate) fn get(conn: &Connection, id: i64) -> rusqlite::Result<Pitch> {
    let sql = format!("SELECT {COLUMNS} FROM pitches WHERE id = ?1");
    conn.query_row(&sql, [id], Pitch::from_row)
}

pub(super) fn create(conn: &Connection, name: &str, skill: &str) -> rusqlite::Result<Pitch> {
    conn.execute(
        "INSERT INTO pitches (name, skill) VALUES (?1, ?2)",
        params![name, skill],
    )?;
    get(conn, conn.last_insert_rowid())
}

pub(super) fn update(
    conn: &Connection,
    id: i64,
    name: &str,
    skill: &str,
) -> rusqlite::Result<Option<Pitch>> {
    let changed = conn.execute(
        "UPDATE pitches SET name = ?1, skill = ?2 WHERE id = ?3",
        params![name, skill, id],
    )?;
    if changed == 0 {
        return Ok(None);
    }
    get(conn, id).map(Some)
}

/// Delete a pitch and everything scoped to it, atomically. Prospects are
/// `ON DELETE SET NULL` (a *stage* delete must never orphan them), so a *pitch*
/// delete has to remove them explicitly — otherwise they'd become invisible,
/// unrecoverable NULL-pitch rows no view lists. Their messages cascade with
/// them, and the pitch's stages cascade with the pitch. Returns the number of
/// pitch rows deleted (0 if the id didn't exist).
pub(super) fn delete(conn: &mut Connection, id: i64) -> rusqlite::Result<usize> {
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM prospects WHERE pitch_id = ?1", [id])?;
    let deleted = tx.execute("DELETE FROM pitches WHERE id = ?1", [id])?;
    tx.commit()?;
    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::migrations;

    fn setup() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        // Match production: cascades (a prospect's messages, a pitch's stages)
        // only fire with foreign keys enforced.
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        migrations::run(&mut conn).unwrap();
        conn
    }

    #[test]
    fn create_list_delete_roundtrip() {
        let mut conn = setup();
        let p = create(&conn, "Design-in-code", "For eng teams").unwrap();
        assert!(p.id > 0);
        assert_eq!(p.name, "Design-in-code");
        assert_eq!(p.skill, "For eng teams");
        assert!(!p.created_at.is_empty());

        assert_eq!(list(&conn).unwrap().len(), 1);
        assert_eq!(delete(&mut conn, p.id).unwrap(), 1);
        assert!(list(&conn).unwrap().is_empty());
    }

    #[test]
    fn delete_cascades_prospects_and_their_messages() {
        let mut conn = setup();
        // `create` inserts only the pitch row (stage seeding lives in the
        // create_pitch command), so attach a prospect + message directly.
        let p = create(&conn, "P", "").unwrap();
        conn.execute(
            "INSERT INTO prospects (name, linkedin_url, pitch_id) VALUES ('N', 'u', ?1)",
            [p.id],
        )
        .unwrap();
        let prospect = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO messages (prospect_id, li_key, body, direction) VALUES (?1, 'k', 'hi', 'outgoing')",
            [prospect],
        )
        .unwrap();

        assert_eq!(delete(&mut conn, p.id).unwrap(), 1);

        // The pitch's prospects and their (cascaded) messages are gone — no
        // invisible NULL-pitch orphans left behind.
        for (table, sql) in [
            ("prospects", "SELECT count(*) FROM prospects"),
            ("messages", "SELECT count(*) FROM messages"),
        ] {
            let n: i64 = conn.query_row(sql, [], |r| r.get(0)).unwrap();
            assert_eq!(n, 0, "{table} rows should be gone after the pitch delete");
        }
    }

    #[test]
    fn list_orders_newest_first() {
        let conn = setup();
        let a = create(&conn, "First", "").unwrap();
        let b = create(&conn, "Second", "").unwrap();
        let all = list(&conn).unwrap();
        // created_at can tie at second resolution; id DESC breaks the tie.
        assert_eq!(all[0].id, b.id);
        assert_eq!(all[1].id, a.id);
    }

    #[test]
    fn delete_missing_returns_zero() {
        let mut conn = setup();
        assert_eq!(delete(&mut conn, 999).unwrap(), 0);
    }

    #[test]
    fn update_changes_fields_and_returns_row() {
        let conn = setup();
        let p = create(&conn, "Old", "old skill").unwrap();
        let updated = update(&conn, p.id, "New", "new skill").unwrap().unwrap();
        assert_eq!(updated.id, p.id);
        assert_eq!(updated.name, "New");
        assert_eq!(updated.skill, "new skill");
        // created_at is preserved by the UPDATE.
        assert_eq!(updated.created_at, p.created_at);
    }

    #[test]
    fn update_missing_returns_none() {
        let conn = setup();
        assert!(update(&conn, 999, "x", "y").unwrap().is_none());
    }
}
