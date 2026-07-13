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
    include_str!("0003_create_product.sql"),
    include_str!("0004_create_prospects.sql"),
    include_str!("0005_create_stages.sql"),
    include_str!("0006_add_stage_color.sql"),
    include_str!("0007_create_messages.sql"),
    include_str!("0008_rename_product_to_profile.sql"),
    include_str!("0009_create_chrome_profiles.sql"),
    include_str!("0010_replace_chrome_profiles.sql"),
    include_str!("0011_drop_capture_profile.sql"),
    include_str!("0012_add_message_direction.sql"),
    include_str!("0013_create_snippets.sql"),
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

    /// The stages migration (0005) must backfill pre-existing pitches with a
    /// Full-cycle pipeline and place existing prospects in the messaging stage.
    #[test]
    fn stages_migration_backfills_existing_data() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        // Apply everything up to (but not including) the stages migration.
        for sql in &MIGRATIONS[..4] {
            conn.execute_batch(sql).unwrap();
        }
        conn.pragma_update(None, "user_version", 4i64).unwrap();

        // Seed a pitch + prospect as they'd exist before the upgrade.
        conn.execute("INSERT INTO pitches (name, skill) VALUES ('Old', '')", [])
            .unwrap();
        let pitch = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO prospects (name, linkedin_url, pitch_id) VALUES ('Ada', 'u', ?1)",
            [pitch],
        )
        .unwrap();

        // Upgrade — 0005 onward apply (the test asserts 0005's + 0006's effects).
        run(&mut conn).unwrap();

        let stage_count: i64 = conn
            .query_row(
                "SELECT count(*) FROM stages WHERE pitch_id = ?1",
                [pitch],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stage_count, 4, "Full-cycle seeded for the existing pitch");

        let (kind, color): (String, String) = conn
            .query_row(
                "SELECT s.kind, s.color FROM prospects p JOIN stages s ON s.id = p.stage_id \
                 WHERE p.name = 'Ada'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(kind, "messaging", "existing prospect lands in messaging stage");
        // 0006 backfills color by position; the messaging stage is position 0.
        assert_eq!(color, "blue", "stage color backfilled by position");
    }

    /// The rename in 0008 must preserve the seeded singleton row and its data —
    /// the migration contract is "never discard the user's data". Guards against
    /// a future "simplification" to DROP+CREATE that would silently wipe every
    /// saved profile. (0008 uses `ALTER TABLE ... RENAME TO`, which preserves
    /// the rows, columns, and the CHECK(id = 1) constraint.)
    #[test]
    fn profile_rename_preserves_the_product_row() {
        let mut conn = Connection::open_in_memory().unwrap();
        // Apply everything up to (but not including) the rename — 0008 is index 7.
        for sql in &MIGRATIONS[..7] {
            conn.execute_batch(sql).unwrap();
        }
        conn.pragma_update(None, "user_version", 7i64).unwrap();

        // The user has filled in their profile (still the `product` table at v7).
        conn.execute(
            "UPDATE product SET who_are_you = 'Ada, founder', what_building = 'a CRM' WHERE id = 1",
            [],
        )
        .unwrap();

        // Upgrade across the rename (0008 onward).
        run(&mut conn).unwrap();

        // The row survived under the new table name, with its data and id intact.
        let (id, who, building): (i64, String, String) = conn
            .query_row(
                "SELECT id, who_are_you, what_building FROM profile",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(id, 1);
        assert_eq!(who, "Ada, founder");
        assert_eq!(building, "a CRM");

        // The singleton CHECK survived the rename (a second row is rejected).
        assert!(
            conn.execute("INSERT INTO profile (id) VALUES (2)", []).is_err(),
            "the CHECK(id = 1) singleton constraint must survive the rename"
        );

        // The old table name no longer exists.
        assert!(conn.query_row("SELECT 1 FROM product", [], |_| Ok(())).is_err());
    }
}
