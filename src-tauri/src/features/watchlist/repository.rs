//! All SQL for watched profiles. Functions take `&Connection` and return domain
//! types; the caller (command layer or the ingest HTTP server) owns connection
//! locking. Kept free of Tauri types so it stays unit-testable against an
//! in-memory DB.
//!
//! `list` is `pub(crate)` (not `pub(super)`) because the loopback ingest server
//! in `crate::ingest` reads it directly — the extension fetches the watchlist at
//! the start of a comment run.

use rusqlite::{params, Connection};

use super::model::WatchedProfile;

const COLUMNS: &str = "id, linkedin_url, name, created_at";

pub(crate) fn list(conn: &Connection) -> rusqlite::Result<Vec<WatchedProfile>> {
    let sql = format!("SELECT {COLUMNS} FROM watched_profiles ORDER BY created_at DESC, id DESC");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], WatchedProfile::from_row)?;
    rows.collect()
}

/// Add a profile to the watchlist, or if one with the same `linkedin_url` already
/// exists, refresh its label. Deduped on the URL so the same person can't be
/// watched twice; `created_at` is preserved on the existing row. Returns the row.
pub(super) fn add(
    conn: &Connection,
    linkedin_url: &str,
    name: &str,
) -> rusqlite::Result<WatchedProfile> {
    let sql = format!(
        "INSERT INTO watched_profiles (linkedin_url, name)
              VALUES (?1, ?2)
         ON CONFLICT(linkedin_url) DO UPDATE SET name = excluded.name
         RETURNING {COLUMNS}"
    );
    conn.query_row(&sql, params![linkedin_url, name], WatchedProfile::from_row)
}

/// Permanently remove a watched profile. Returns the number of rows deleted (0 if
/// no profile had that id) so the caller can distinguish a real delete from a
/// stale id.
pub(super) fn delete(conn: &Connection, id: i64) -> rusqlite::Result<usize> {
    conn.execute("DELETE FROM watched_profiles WHERE id = ?1", [id])
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
    fn add_inserts_then_lists() {
        let conn = setup();
        let w = add(&conn, "https://www.linkedin.com/in/ada/", "Ada").unwrap();
        assert!(w.id > 0);
        assert_eq!(w.linkedin_url, "https://www.linkedin.com/in/ada/");
        assert_eq!(w.name, "Ada");
        assert!(!w.created_at.is_empty());
        assert_eq!(list(&conn).unwrap().len(), 1);
    }

    #[test]
    fn add_dedups_on_url_and_updates_label() {
        let conn = setup();
        let url = "https://www.linkedin.com/in/grace/";
        let first = add(&conn, url, "Grace").unwrap();
        // Re-adding the same URL with a new label updates in place, not a dup.
        let second = add(&conn, url, "Grace H.").unwrap();
        assert_eq!(first.id, second.id);
        assert_eq!(list(&conn).unwrap().len(), 1);
        assert_eq!(second.name, "Grace H.");
        // created_at is preserved by the conflict update.
        assert_eq!(second.created_at, first.created_at);
    }

    #[test]
    fn add_allows_empty_label() {
        let conn = setup();
        let w = add(&conn, "https://www.linkedin.com/in/x/", "").unwrap();
        assert_eq!(w.name, "");
    }

    #[test]
    fn delete_removes_row_and_missing_returns_zero() {
        let conn = setup();
        let w = add(&conn, "https://www.linkedin.com/in/gone/", "").unwrap();
        assert_eq!(list(&conn).unwrap().len(), 1);
        assert_eq!(delete(&conn, w.id).unwrap(), 1);
        assert!(list(&conn).unwrap().is_empty());
        assert_eq!(delete(&conn, 999).unwrap(), 0);
    }

    #[test]
    fn list_orders_newest_first() {
        let conn = setup();
        let a = add(&conn, "https://li/a", "A").unwrap();
        let b = add(&conn, "https://li/b", "B").unwrap();
        let all = list(&conn).unwrap();
        // created_at can tie at second resolution; id DESC breaks the tie.
        assert_eq!(all[0].id, b.id);
        assert_eq!(all[1].id, a.id);
    }
}
