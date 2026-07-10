//! Tiny versioned migration runner.
//!
//! Each entry in `MIGRATIONS` is one forward step — a `.sql` file in this
//! directory, embedded at compile time. The number of applied steps is tracked
//! in SQLite's `user_version` pragma.
//!
//! To add a migration: create the next `NNNN_name.sql` file here and append
//! `include_str!("NNNN_name.sql")` to the list below. No extra crate.

use rusqlite::Connection;

/// Ordered, append-only list of schema migrations. Never edit or reorder an
/// existing entry once it has shipped — only append new ones.
const MIGRATIONS: &[&str] = &[
    include_str!("0001_create_pitches.sql"),
    include_str!("0002_rename_description_to_skill.sql"),
];

/// Apply every migration newer than the database's current `user_version`,
/// each in its own transaction so a failure leaves the DB on a clean version.
pub fn run(conn: &mut Connection) -> rusqlite::Result<()> {
    let applied: usize =
        conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))? as usize;

    for (i, sql) in MIGRATIONS.iter().enumerate().skip(applied) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        // user_version can't be parameterized; the index is trusted (not user input).
        tx.pragma_update(None, "user_version", (i + 1) as i64)?;
        tx.commit()?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_applies_all_and_is_idempotent() {
        let mut conn = Connection::open_in_memory().unwrap();
        run(&mut conn).unwrap();
        run(&mut conn).unwrap(); // second run should apply nothing

        let version: usize = conn
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap() as usize;
        assert_eq!(version, MIGRATIONS.len());

        // pitches table exists and is queryable.
        let count: i64 = conn
            .query_row("SELECT count(*) FROM pitches", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }
}
