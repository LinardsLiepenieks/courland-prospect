//! All SQL for customer profiles. Functions take `&Connection` and return
//! domain types; the caller (command layer or the ingest HTTP server) owns
//! connection locking. Kept free of Tauri types so it stays unit-testable
//! against an in-memory database.

use rusqlite::{params, Connection, OptionalExtension};

use super::model::Customer;

const COLUMNS: &str = "id, name, who_they_are, pain, goal, created_at";

/// Every customer profile, oldest first — a hand-curated list of a handful of
/// buyer types, so it reads in the order they were defined rather than
/// newest-first (which would reshuffle the list every time one is added).
pub(crate) fn list(conn: &Connection) -> rusqlite::Result<Vec<Customer>> {
    let sql = format!("SELECT {COLUMNS} FROM customers ORDER BY created_at ASC, id ASC");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], Customer::from_row)?;
    rows.collect()
}

/// One customer profile, or `None` when the id doesn't exist (deleted between
/// the extension reading the dropdown and the draft request landing). The draft
/// path treats a missing customer as "unassigned" rather than an error.
pub(crate) fn find(conn: &Connection, id: i64) -> rusqlite::Result<Option<Customer>> {
    let sql = format!("SELECT {COLUMNS} FROM customers WHERE id = ?1");
    conn.query_row(&sql, [id], Customer::from_row).optional()
}

/// Fetch by id, erroring when absent — the internal getter `create`/`update` use
/// to return the row they just wrote.
fn get(conn: &Connection, id: i64) -> rusqlite::Result<Customer> {
    find(conn, id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)
}

pub(super) fn create(
    conn: &Connection,
    name: &str,
    who_they_are: &str,
    pain: &str,
    goal: &str,
) -> rusqlite::Result<Customer> {
    conn.execute(
        "INSERT INTO customers (name, who_they_are, pain, goal) VALUES (?1, ?2, ?3, ?4)",
        params![name, who_they_are, pain, goal],
    )?;
    get(conn, conn.last_insert_rowid())
}

pub(super) fn update(
    conn: &Connection,
    id: i64,
    name: &str,
    who_they_are: &str,
    pain: &str,
    goal: &str,
) -> rusqlite::Result<Option<Customer>> {
    let changed = conn.execute(
        "UPDATE customers SET name = ?1, who_they_are = ?2, pain = ?3, goal = ?4 WHERE id = ?5",
        params![name, who_they_are, pain, goal, id],
    )?;
    if changed == 0 {
        return Ok(None);
    }
    get(conn, id).map(Some)
}

/// Delete a customer profile. Unlike the pitch delete this replaced, this does
/// NOT remove the prospects attached to it: with one shared pipeline every
/// prospect stays relevant regardless of which profile they matched, so the
/// column's `ON DELETE SET NULL` simply leaves them unassigned. Returns the
/// number of rows deleted (0 if the id didn't exist).
pub(super) fn delete(conn: &Connection, id: i64) -> rusqlite::Result<usize> {
    conn.execute("DELETE FROM customers WHERE id = ?1", [id])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::migrations;

    fn setup() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        // Match production: the SET NULL on prospects only fires with FKs on.
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        migrations::run(&mut conn).unwrap();
        conn
    }

    #[test]
    fn create_list_update_delete_roundtrip() {
        let conn = setup();
        let c = create(&conn, "Solo agencies", "1-5 person shops", "outreach eats hours", "book a call")
            .unwrap();
        assert!(c.id > 0);
        assert_eq!(c.name, "Solo agencies");
        assert_eq!(c.who_they_are, "1-5 person shops");
        assert_eq!(c.pain, "outreach eats hours");
        assert_eq!(c.goal, "book a call");
        assert!(!c.created_at.is_empty());

        let updated = update(&conn, c.id, "Agencies", "1-10 person shops", "no time", "get a trial")
            .unwrap()
            .unwrap();
        assert_eq!(updated.id, c.id);
        assert_eq!(updated.name, "Agencies");
        assert_eq!(updated.pain, "no time");
        assert_eq!(updated.goal, "get a trial");
        assert_eq!(updated.created_at, c.created_at, "created_at is preserved");

        assert_eq!(list(&conn).unwrap().len(), 1);
        assert_eq!(delete(&conn, c.id).unwrap(), 1);
        assert!(list(&conn).unwrap().is_empty());
    }

    #[test]
    fn list_orders_oldest_first() {
        let conn = setup();
        let a = create(&conn, "First", "", "", "").unwrap();
        let b = create(&conn, "Second", "", "", "").unwrap();
        let all = list(&conn).unwrap();
        // created_at ties at second resolution; id ASC breaks the tie.
        assert_eq!(all[0].id, a.id);
        assert_eq!(all[1].id, b.id);
    }

    #[test]
    fn find_returns_none_for_a_missing_id() {
        let conn = setup();
        assert!(find(&conn, 999).unwrap().is_none());
        assert!(update(&conn, 999, "x", "", "", "").unwrap().is_none());
        assert_eq!(delete(&conn, 999).unwrap(), 0);
    }

    /// Deleting a customer profile must leave its prospects in the pipeline,
    /// merely unassigned. This is the deliberate break from the old pitch
    /// delete, which removed the pitch's prospects and their whole message
    /// history along with it.
    #[test]
    fn delete_unassigns_prospects_instead_of_removing_them() {
        let conn = setup();
        let c = create(&conn, "Agencies", "", "", "").unwrap();
        conn.execute(
            "INSERT INTO prospects (name, linkedin_url, customer_id) VALUES ('Ada', 'https://li/ada', ?1)",
            [c.id],
        )
        .unwrap();
        let prospect = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO messages (prospect_id, li_key, body, direction) \
             VALUES (?1, 'k', 'hi', 'outgoing')",
            [prospect],
        )
        .unwrap();

        delete(&conn, c.id).unwrap();

        let (count, customer): (i64, Option<i64>) = conn
            .query_row(
                "SELECT count(*), max(customer_id) FROM prospects WHERE id = ?1",
                [prospect],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(count, 1, "the prospect survives the customer delete");
        assert_eq!(customer, None, "they're simply unassigned now");

        let messages: i64 = conn
            .query_row("SELECT count(*) FROM messages", [], |r| r.get(0))
            .unwrap();
        assert_eq!(messages, 1, "their captured history is untouched");
    }
}
