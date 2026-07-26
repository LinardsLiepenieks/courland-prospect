//! All SQL for the profile singleton. Functions take `&Connection` and return
//! domain types; the command layer owns connection locking. Kept free of Tauri
//! types so it stays unit-testable against an in-memory database.

use rusqlite::{params, Connection};

use super::model::Profile;

const COLUMNS: &str = "who_are_you, updated_at";

/// Read the singleton row. Migration 0003 seeds it, so this always finds a row.
pub(crate) fn get(conn: &Connection) -> rusqlite::Result<Profile> {
    let sql = format!("SELECT {COLUMNS} FROM profile WHERE id = 1");
    conn.query_row(&sql, [], Profile::from_row)
}

/// Update the singleton row and bump `updated_at`, then return it.
pub(super) fn update(conn: &Connection, who_are_you: &str) -> rusqlite::Result<Profile> {
    conn.execute(
        "UPDATE profile SET who_are_you = ?1, updated_at = datetime('now') WHERE id = 1",
        params![who_are_you],
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
        assert!(!p.updated_at.is_empty());
    }

    #[test]
    fn update_persists_and_returns_row() {
        let conn = setup();
        let p = update(&conn, "A founder").unwrap();
        assert_eq!(p.who_are_you, "A founder");

        // A fresh read sees the same value — the update hit the singleton row.
        assert_eq!(get(&conn).unwrap().who_are_you, "A founder");
    }

    #[test]
    fn update_stays_a_singleton() {
        let conn = setup();
        update(&conn, "x").unwrap();
        update(&conn, "z").unwrap();
        let count: i64 = conn
            .query_row("SELECT count(*) FROM profile", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    /// The profile carries the founder's bio and nothing about the product —
    /// the split that keeps a public comment from ever reading as pitch copy.
    #[test]
    fn profile_no_longer_carries_the_product_story() {
        let conn = setup();
        assert!(
            conn.query_row("SELECT what_building FROM profile", [], |_| Ok(())).is_err(),
            "what-are-you-building lives in the product singleton now"
        );
    }
}
