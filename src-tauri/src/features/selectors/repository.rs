//! All SQL for the selector-overrides singleton. Functions take `&Connection`;
//! the caller (the ingest server) owns connection locking. No Tauri types, so it
//! stays unit-testable against an in-memory database.
//!
//! The value is an opaque JSON string — a `{key: value}` object of overrides.
//! This layer never parses or merges it (that's the ingest handler's job); it
//! only reads and replaces the blob.

use rusqlite::{params, Connection};

/// Read the overrides JSON blob. Migration 0015 seeds a `'{}'` row, so this
/// always finds one.
pub(crate) fn get_overrides(conn: &Connection) -> rusqlite::Result<String> {
    conn.query_row("SELECT overrides FROM selectors WHERE id = 1", [], |r| r.get(0))
}

/// Replace the overrides JSON blob and bump `updated_at`. The caller passes the
/// already-merged, serialized object.
pub(crate) fn set_overrides(conn: &Connection, overrides_json: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE selectors SET overrides = ?1, updated_at = datetime('now') WHERE id = 1",
        params![overrides_json],
    )?;
    Ok(())
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
    fn get_returns_seeded_empty_object() {
        let conn = setup();
        assert_eq!(get_overrides(&conn).unwrap(), "{}");
    }

    #[test]
    fn set_persists_and_stays_a_singleton() {
        let conn = setup();
        set_overrides(&conn, r#"{"composeRoot":".foo"}"#).unwrap();
        assert_eq!(get_overrides(&conn).unwrap(), r#"{"composeRoot":".foo"}"#);

        // A second write replaces (doesn't append a row).
        set_overrides(&conn, r#"{"composeRoot":".bar"}"#).unwrap();
        assert_eq!(get_overrides(&conn).unwrap(), r#"{"composeRoot":".bar"}"#);
        let count: i64 = conn
            .query_row("SELECT count(*) FROM selectors", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }
}
