//! All SQL for the profile singleton. Functions take `&Connection` and return
//! domain types; the command layer owns connection locking. Kept free of Tauri
//! types so it stays unit-testable against an in-memory database.

use rusqlite::{params, Connection};

use super::model::Profile;

const COLUMNS: &str = "who_are_you, what_building, updated_at";

/// Read the singleton row. Migration 0003 seeds it, so this always finds a row.
pub(crate) fn get(conn: &Connection) -> rusqlite::Result<Profile> {
    let sql = format!("SELECT {COLUMNS} FROM profile WHERE id = 1");
    conn.query_row(&sql, [], Profile::from_row)
}

/// Update the singleton row's fields and bump `updated_at`, then return it.
pub(super) fn update(
    conn: &Connection,
    who_are_you: &str,
    what_building: &str,
) -> rusqlite::Result<Profile> {
    conn.execute(
        "UPDATE profile
            SET who_are_you = ?1, what_building = ?2, updated_at = datetime('now')
          WHERE id = 1",
        params![who_are_you, what_building],
    )?;
    get(conn)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::migrations;

    fn setup() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        migrations::run(&mut conn).unwrap();
        conn
    }

    #[test]
    fn get_returns_seeded_empty_singleton() {
        let conn = setup();
        let p = get(&conn).unwrap();
        assert_eq!(p.who_are_you, "");
        assert_eq!(p.what_building, "");
        assert!(!p.updated_at.is_empty());
    }

    #[test]
    fn update_persists_fields_and_returns_row() {
        let conn = setup();
        let p = update(&conn, "A founder", "A light CRM").unwrap();
        assert_eq!(p.who_are_you, "A founder");
        assert_eq!(p.what_building, "A light CRM");

        // A fresh read sees the same values — the update hit the singleton row.
        let reread = get(&conn).unwrap();
        assert_eq!(reread.who_are_you, "A founder");
        assert_eq!(reread.what_building, "A light CRM");
    }

    #[test]
    fn update_stays_a_singleton() {
        let conn = setup();
        update(&conn, "x", "y").unwrap();
        update(&conn, "z", "w").unwrap();
        let count: i64 = conn
            .query_row("SELECT count(*) FROM profile", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }
}
