//! All SQL for the product singleton. Functions take `&Connection` and return
//! domain types; the command layer owns connection locking. Kept free of Tauri
//! types so it stays unit-testable against an in-memory database.

use rusqlite::{params, Connection};

use super::model::Product;

const COLUMNS: &str = "name, description, updated_at";

/// Read the singleton row. Migration 0022 seeds it, so this always finds a row.
pub(crate) fn get(conn: &Connection) -> rusqlite::Result<Product> {
    let sql = format!("SELECT {COLUMNS} FROM product WHERE id = 1");
    conn.query_row(&sql, [], Product::from_row)
}

/// Update the singleton row's fields and bump `updated_at`, then return it.
pub(super) fn update(
    conn: &Connection,
    name: &str,
    description: &str,
) -> rusqlite::Result<Product> {
    conn.execute(
        "UPDATE product
            SET name = ?1, description = ?2, updated_at = datetime('now')
          WHERE id = 1",
        params![name, description],
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
        assert_eq!(p.name, "");
        assert_eq!(p.description, "");
        assert!(!p.updated_at.is_empty());
    }

    #[test]
    fn update_persists_fields_and_returns_row() {
        let conn = setup();
        let p = update(&conn, "Courland", "A light CRM for founder-led sales").unwrap();
        assert_eq!(p.name, "Courland");
        assert_eq!(p.description, "A light CRM for founder-led sales");

        // A fresh read sees the same values — the update hit the singleton row.
        let reread = get(&conn).unwrap();
        assert_eq!(reread.name, "Courland");
        assert_eq!(reread.description, "A light CRM for founder-led sales");
    }

    #[test]
    fn update_stays_a_singleton() {
        let conn = setup();
        update(&conn, "a", "b").unwrap();
        update(&conn, "c", "d").unwrap();
        let count: i64 = conn
            .query_row("SELECT count(*) FROM product", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    // The 0022 backfill — the product description inheriting the profile's old
    // "what are you building" — is asserted in `database::migrations::tests`,
    // alongside every other upgrade-path assertion and against a v21 database
    // seeded the same way production opens one (foreign keys ON). Re-testing it
    // here meant a second, lower-fidelity copy of that setup.
}
