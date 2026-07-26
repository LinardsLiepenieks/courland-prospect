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
    include_str!("0014_rename_responded_to_awaiting_reply.sql"),
    include_str!("0015_create_selectors.sql"),
    include_str!("0016_add_snippet_status.sql"),
    include_str!("0017_add_snippet_position_category.sql"),
    include_str!("0018_create_watched_profiles.sql"),
    include_str!("0019_create_commented_posts.sql"),
    include_str!("0020_create_comment_drafts.sql"),
    include_str!("0021_recreate_commented_posts.sql"),
    include_str!("0022_create_product.sql"),
    include_str!("0023_create_customers.sql"),
    include_str!("0024_one_pipeline.sql"),
    include_str!("0025_prospects_customer.sql"),
    include_str!("0026_one_snippet_library.sql"),
];

/// Apply every migration newer than the database's current `user_version`,
/// each in its own transaction so a failure leaves the DB on a clean version.
///
/// Foreign keys are enforcement-OFF for the duration of the run, then restored.
/// This is SQLite's own documented procedure for a table rebuild (see "Making
/// Other Kinds Of Table Schema Changes"), and a rebuild is unavoidable here: a
/// column named by a foreign key can't be dropped in place, and `DROP TABLE`
/// with enforcement ON performs an implicit `DELETE FROM` that fires
/// `ON DELETE` actions — which would silently cascade a table swap into the
/// user's messages. The pragma is a no-op inside a transaction, so it has to be
/// toggled out here rather than inside a migration.
///
/// `legacy_alter_table` goes with it: with it OFF (the default), `ALTER TABLE …
/// RENAME TO` re-parses and rewrites every other table's references, which
/// breaks mid-rebuild when the table being replaced doesn't exist yet. ON, a
/// rename is just a rename — which is what a rebuild's final swap wants.
///
/// After a successful run, `PRAGMA foreign_key_check` verifies the result: a
/// migration that left a dangling reference fails loud here rather than
/// shipping a corrupt graph into the app.
pub fn run(conn: &mut Connection) -> rusqlite::Result<()> {
    let applied: usize =
        conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))? as usize;
    // Nothing pending — don't touch pragmas on the common already-migrated open.
    if applied >= MIGRATIONS.len() {
        return Ok(());
    }

    let fk_was_on: bool = conn.query_row("PRAGMA foreign_keys", [], |r| r.get(0))?;
    conn.execute_batch("PRAGMA foreign_keys = OFF; PRAGMA legacy_alter_table = ON;")?;

    let result = apply_pending(conn, applied);

    // Restore the caller's enforcement setting whether or not the run succeeded,
    // so a failed migration doesn't leave the connection silently unenforced.
    conn.execute_batch("PRAGMA legacy_alter_table = OFF;")?;
    if fk_was_on {
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    }
    result?;

    if fk_was_on {
        assert_referential_integrity(conn)?;
    }
    Ok(())
}

/// Apply each pending migration in its own transaction, so a failure leaves the
/// database on the last cleanly-applied version rather than half-way through one.
fn apply_pending(conn: &mut Connection, applied: usize) -> rusqlite::Result<()> {
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(applied) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        // user_version can't be parameterized; the index is trusted (not user input).
        tx.pragma_update(None, "user_version", (i + 1) as i64)?;
        tx.commit()?;
    }
    Ok(())
}

/// Fail if the freshly-migrated schema holds any dangling foreign-key reference.
/// The migration contract is "fail loud, never discard the user's data" — a
/// rebuild that orphaned rows must surface as a refused open, not as rows that
/// quietly vanish from a view later.
fn assert_referential_integrity(conn: &Connection) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare("PRAGMA foreign_key_check")?;
    let mut rows = stmt.query([])?;
    if let Some(row) = rows.next()? {
        let table: String = row.get(0)?;
        let parent: String = row.get(2)?;
        return Err(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CONSTRAINT),
            Some(format!(
                "migration left a dangling foreign key: {table} references a missing row in {parent}"
            )),
        ));
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

        // The customers table exists and is queryable; `pitches` is gone (v26).
        let count: i64 = conn
            .query_row("SELECT count(*) FROM customers", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
        assert!(
            conn.query_row("SELECT 1 FROM pitches", [], |_| Ok(())).is_err(),
            "the pitches table is dropped by the rework"
        );

        // A fresh database still gets a pipeline (v24 seeds the template when
        // there were no pitches to inherit one from).
        let stages: i64 = conn
            .query_row("SELECT count(*) FROM stages", [], |r| r.get(0))
            .unwrap();
        assert_eq!(stages, 4, "an empty database is seeded with Full-cycle");
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

        // 0005 seeded Full-cycle for the pitch; 0024 later made that single
        // pipeline the global one, so the four stages survive un-owned.
        let stage_count: i64 = conn
            .query_row("SELECT count(*) FROM stages", [], |r| r.get(0))
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

        // Upgrade across the rename (0008 onward) and the later product split.
        run(&mut conn).unwrap();

        // The row survived under the new table name, with its data and id intact.
        let (id, who): (i64, String) = conn
            .query_row("SELECT id, who_are_you FROM profile", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(id, 1);
        assert_eq!(who, "Ada, founder");
        // 0022 moved "what are you building" into the product singleton rather
        // than discarding it — the same text, under the new home.
        let building: String = conn
            .query_row("SELECT description FROM product WHERE id = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(building, "a CRM");

        // The singleton CHECK survived the rename (a second row is rejected).
        assert!(
            conn.execute("INSERT INTO profile (id) VALUES (2)", []).is_err(),
            "the CHECK(id = 1) singleton constraint must survive the rename"
        );

        // A table named `product` exists again — but it is 0022's product
        // singleton, NOT the pre-0008 table this migration renamed away. The
        // distinction matters: if 0008 had somehow left the original behind, its
        // `who_are_you` column would still be queryable here.
        assert!(
            conn.query_row("SELECT who_are_you FROM product", [], |_| Ok(())).is_err(),
            "the pre-rename product table must be gone, not merely shadowed"
        );
    }

    /// The `responded` → `awaiting_reply` migration (0014) must both preserve the
    /// column's data (RENAME COLUMN) and re-derive it to the new meaning: a
    /// prospect is awaiting a reply when their newest stored message is incoming.
    /// The stale pre-upgrade value must be recomputed, not carried over — so this
    /// seeds deliberately wrong values and asserts the backfill overwrites them.
    #[test]
    fn awaiting_reply_migration_backfills_from_newest_message() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        // Apply everything up to (but not including) 0014 — it is index 13.
        for sql in &MIGRATIONS[..13] {
            conn.execute_batch(sql).unwrap();
        }
        conn.pragma_update(None, "user_version", 13i64).unwrap();

        // Three prospects as they'd exist at v13 (the column is still `responded`).
        let seed = |conn: &Connection, url: &str| -> i64 {
            conn.execute(
                "INSERT INTO prospects (name, linkedin_url) VALUES ('N', ?1)",
                [url],
            )
            .unwrap();
            conn.last_insert_rowid()
        };
        let awaiting = seed(&conn, "https://li/awaiting"); // newest incoming
        let answered = seed(&conn, "https://li/answered"); // newest outgoing
        let silent = seed(&conn, "https://li/silent"); // no messages

        let msg = |conn: &Connection, pid: i64, key: &str, dir: &str| {
            conn.execute(
                "INSERT INTO messages (prospect_id, li_key, direction) VALUES (?1, ?2, ?3)",
                rusqlite::params![pid, key, dir],
            )
            .unwrap();
        };
        // Insertion order == id order == chronology: the last-inserted is newest.
        msg(&conn, awaiting, "a1", "outgoing");
        msg(&conn, awaiting, "a2", "incoming");
        msg(&conn, answered, "b1", "incoming");
        msg(&conn, answered, "b2", "outgoing");

        // Seed the OLD flag with values that contradict the new meaning, to prove
        // the backfill recomputes rather than carrying the stale value forward.
        conn.execute("UPDATE prospects SET responded = 0 WHERE id = ?1", [awaiting])
            .unwrap();
        conn.execute("UPDATE prospects SET responded = 1 WHERE id = ?1", [answered])
            .unwrap();

        // Upgrade across the rename + backfill (0014).
        run(&mut conn).unwrap();

        let flag = |conn: &Connection, id: i64| -> i64 {
            conn.query_row(
                "SELECT awaiting_reply FROM prospects WHERE id = ?1",
                [id],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(flag(&conn, awaiting), 1, "newest incoming → awaiting a reply");
        assert_eq!(flag(&conn, answered), 0, "newest outgoing → not awaiting");
        assert_eq!(flag(&conn, silent), 0, "no messages → not awaiting (COALESCE)");

        // The old column name is gone (RENAME COLUMN, not a copy).
        assert!(
            conn.query_row("SELECT responded FROM prospects", [], |_| Ok(())).is_err(),
            "the pre-rename column must no longer exist"
        );
    }

    /// The position/category/manual migration (0017) must backfill every
    /// pre-existing snippet in place: mid-arc position (0.5), no category, not
    /// manual — non-destructive, like the status column before it. Guards against a
    /// future edit to 0017 that would fail to default existing rows (which would
    /// break every snippet read, since `from_row` expects the columns non-NULL).
    #[test]
    fn position_category_migration_backfills_existing_snippets() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        // Apply everything up to (but not including) 0017 — it is index 16.
        for sql in &MIGRATIONS[..16] {
            conn.execute_batch(sql).unwrap();
        }
        conn.pragma_update(None, "user_version", 16i64).unwrap();

        // A snippet as it'd exist at v16 (status column present; the three new
        // columns not yet). A NULL pitch_id (profile scope) needs no pitch row.
        conn.execute(
            "INSERT INTO snippets (name, content) VALUES ('Intro', 'saw your post')",
            [],
        )
        .unwrap();

        // Upgrade across 0017.
        run(&mut conn).unwrap();

        let (position, category, manual): (f64, String, i64) = conn
            .query_row(
                "SELECT position, category, manual FROM snippets WHERE name = 'Intro'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(position, 0.5, "existing row backfills to mid-arc");
        assert_eq!(category, "", "existing row backfills to uncategorized");
        assert_eq!(manual, 0, "existing row backfills to non-manual");

        // The content is untouched — the migration is non-destructive.
        let content: String = conn
            .query_row("SELECT content FROM snippets WHERE name = 'Intro'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(content, "saw your post");
    }

    /// Every migration up to (but not including) the one-product rework — the
    /// database exactly as it looked at v21, ready to be seeded with realistic
    /// pre-rework data and upgraded across 0022-0026.
    fn at_v21() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        for sql in &MIGRATIONS[..21] {
            conn.execute_batch(sql).unwrap();
        }
        conn.pragma_update(None, "user_version", 21i64).unwrap();
        conn
    }

    /// Seed a realistic pre-rework database: two pitches with *different*
    /// pipelines, prospects spread across both (plus one with no pitch at all),
    /// a captured thread, and snippets in all three old scopes — including a
    /// cross-scope duplicate, a blank editor card, and an unreviewed proposal.
    fn seed_pre_rework(conn: &Connection) {
        conn.execute_batch(
            "UPDATE profile SET who_are_you = 'Ada, founder',
                                what_building = 'A light CRM for founder-led sales'
             WHERE id = 1;

             INSERT INTO pitches (id, name, skill) VALUES
                (1, 'Design-in-code', 'for eng teams shipping UI'),
                (2, 'Agency ops',     'for small agencies');

             -- Pitch 1 is the busy one (and a longer funnel than pitch 2).
             INSERT INTO stages (id, pitch_id, name, kind, position, color) VALUES
                (10, 1, 'Messaged',   'messaging', 0, 'blue'),
                (11, 1, 'Meeting',    'standard',  1, 'amber'),
                (12, 1, 'Onboarding', 'standard',  2, 'green'),
                (13, 1, 'Feedback',   'standard',  3, 'purple'),
                (20, 2, 'Messaged',   'messaging', 0, 'blue'),
                (21, 2, 'Demo',       'standard',  1, 'amber'),
                (22, 2, 'Trial',      'standard',  2, 'green');

             INSERT INTO prospects
                (id, name, linkedin_url, headline, pitch_id, stage_id,
                 messages_sent, awaiting_reply, note) VALUES
                (100, 'Ada',    'https://li/ada',    'CTO',     1,    11, 3, 1, 'warm'),
                (101, 'Grace',  'https://li/grace',  'Admiral', 1,    10, 1, 0, ''),
                (102, 'Alan',   'https://li/alan',   'Owner',   2,    22, 7, 0, 'trialing'),
                (103, 'Edsger', 'https://li/edsger', '',        NULL, NULL, 0, 0, '');

             INSERT INTO messages (prospect_id, li_key, body, direction) VALUES
                (100, 'k1', 'hi there',     'outgoing'),
                (100, 'k2', 'tell me more', 'incoming'),
                (102, 'k3', 'sounds good',  'incoming');

             INSERT INTO snippets
                (id, pitch_id, name, content, status, position, category, manual) VALUES
                (200, 1,    'Cadence', 'We ship weekly.',        'approved', 0.4, 'Engaged',         0),
                (201, 2,    'Cadence', 'we ship weekly.',        'approved', 0.5, '',                0),
                (202, NULL, 'Ask',     'Worth 15 minutes?',      'approved', 0.9, 'Calling to meet', 1),
                (203, 1,    '',        '',                       'approved', 0.5, '',                0),
                (204, 1,    'New',     'We are SOC2 certified.', 'proposed', 0.5, '',                0);",
        )
        .unwrap();
    }

    /// The one-product rework (0022-0026) is the largest schema change this app
    /// has shipped: three tables are rebuilt and one is dropped. The contract is
    /// unchanged — never discard the user's data — so this asserts every seeded
    /// piece survives in its new home. `run` itself also enforces
    /// `PRAGMA foreign_key_check` afterwards, so a rebuild that stranded a row
    /// would fail this test at the `run` call rather than in an assertion.
    #[test]
    fn one_product_rework_preserves_every_piece_of_user_data() {
        let mut conn = at_v21();
        seed_pre_rework(&conn);

        run(&mut conn).unwrap();

        // --- The product story moved out of the profile, intact. ---
        let description: String = conn
            .query_row("SELECT description FROM product WHERE id = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(description, "A light CRM for founder-led sales");
        let who: String = conn
            .query_row("SELECT who_are_you FROM profile WHERE id = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(who, "Ada, founder", "the founder's own bio is untouched");
        assert!(
            conn.query_row("SELECT what_building FROM profile", [], |_| Ok(())).is_err(),
            "profile narrows to just who-you-are"
        );

        // --- Pitches became customers, keeping their ids. ---
        let customers: Vec<(i64, String, String)> = conn
            .prepare("SELECT id, name, who_they_are FROM customers ORDER BY id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(
            customers,
            vec![
                (1, "Design-in-code".to_string(), "for eng teams shipping UI".to_string()),
                (2, "Agency ops".to_string(), "for small agencies".to_string()),
            ],
            "each pitch's name and skill text carry into a customer profile"
        );

        // --- One pipeline: the busiest pitch's stages survived, ids intact. ---
        let stages: Vec<(i64, String, i64)> = conn
            .prepare("SELECT id, name, position FROM stages ORDER BY position")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(
            stages,
            vec![
                (10, "Messaged".to_string(), 0),
                (11, "Meeting".to_string(), 1),
                (12, "Onboarding".to_string(), 2),
                (13, "Feedback".to_string(), 3),
            ],
            "pitch 1 had the most prospects, so its pipeline became THE pipeline"
        );

        // --- Prospects: customer carried across, stage remapped by position. ---
        let prospect = |id: i64| -> (Option<i64>, Option<i64>, i64, i64, String) {
            conn.query_row(
                "SELECT customer_id, stage_id, messages_sent, awaiting_reply, note
                 FROM prospects WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .unwrap()
        };
        assert_eq!(
            prospect(100),
            (Some(1), Some(11), 3, 1, "warm".to_string()),
            "on the surviving pipeline: nobody moves, and the derived counters hold"
        );
        assert_eq!(prospect(101), (Some(1), Some(10), 1, 0, String::new()));
        assert_eq!(
            prospect(102),
            (Some(2), Some(12), 7, 0, "trialing".to_string()),
            "pitch 2's 'Trial' (position 2) remaps to the same position: 'Onboarding'"
        );
        assert_eq!(
            prospect(103),
            (None, Some(10), 0, 0, String::new()),
            "a prospect with no pitch stays unassigned but joins the pipeline"
        );

        // --- The prospects rebuild must NOT have cascaded the message history. ---
        let messages: i64 = conn
            .query_row("SELECT count(*) FROM messages", [], |r| r.get(0))
            .unwrap();
        assert_eq!(messages, 3, "captured threads survive the prospects rebuild");

        // --- One snippet library, cross-scope duplicate collapsed. ---
        let snippets: Vec<(i64, String, String, String, f64, i64)> = conn
            .prepare("SELECT id, content, status, category, position, manual FROM snippets ORDER BY id")
            .unwrap()
            .query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?))
            })
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(
            snippets,
            vec![
                (200, "We ship weekly.".to_string(), "approved".to_string(), "Engaged".to_string(), 0.4, 0),
                (202, "Worth 15 minutes?".to_string(), "approved".to_string(), "Calling to meet".to_string(), 0.9, 1),
                (203, String::new(), "approved".to_string(), String::new(), 0.5, 0),
                (204, "We are SOC2 certified.".to_string(), "proposed".to_string(), String::new(), 0.5, 0),
            ],
            "scopes merge; the case-differing copy (201) collapses into 200, keeping \
             its arc position, category and manual pin; the blank card and the \
             unreviewed proposal both survive"
        );

        // --- The concept itself is gone. ---
        assert!(conn.query_row("SELECT 1 FROM pitches", [], |_| Ok(())).is_err());
    }

    /// A duplicate set spanning both statuses must keep the APPROVED row, not
    /// merely the lowest id — collapsing onto the `proposed` copy would smuggle an
    /// unreviewed line into the drafting set, and collapsing an approved line away
    /// would silently demote material the user already blessed.
    #[test]
    fn snippet_merge_prefers_the_approved_copy_over_a_proposed_duplicate() {
        let mut conn = at_v21();
        conn.execute_batch(
            "INSERT INTO pitches (id, name, skill) VALUES (1, 'P', '');
             INSERT INTO snippets (id, pitch_id, name, content, status) VALUES
                (1, 1,    'Prop', 'We are SOC2 certified.', 'proposed'),
                (2, NULL, 'Real', 'We are SOC2 certified.', 'approved');",
        )
        .unwrap();

        run(&mut conn).unwrap();

        let kept: Vec<(i64, String)> = conn
            .prepare("SELECT id, status FROM snippets")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(kept, vec![(2, "approved".to_string())]);
    }

    /// With no pitch to inherit a pipeline from, v24 seeds the Full-cycle
    /// template — the app must never come up with an empty board — and any
    /// stage-less prospect lands in its messaging stage.
    #[test]
    fn one_pipeline_seeds_the_template_when_there_are_no_pitches() {
        let mut conn = at_v21();
        conn.execute_batch(
            "INSERT INTO prospects (id, name, linkedin_url) VALUES (1, 'Ada', 'https://li/ada');",
        )
        .unwrap();

        run(&mut conn).unwrap();

        let (name, kind): (String, String) = conn
            .query_row(
                "SELECT s.name, s.kind FROM prospects p JOIN stages s ON s.id = p.stage_id
                 WHERE p.id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((name.as_str(), kind.as_str()), ("Messaged", "messaging"));
        assert_eq!(
            conn.query_row("SELECT count(*) FROM stages", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            4
        );
    }

    /// A prospect further along than the surviving pipeline is long must land in
    /// its LAST stage, not be thrown back to the start — losing progress is the
    /// one thing a pipeline merge must not do.
    #[test]
    fn one_pipeline_clamps_a_prospect_past_the_end_to_the_final_stage() {
        let mut conn = at_v21();
        conn.execute_batch(
            "INSERT INTO pitches (id, name, skill) VALUES (1, 'Short', ''), (2, 'Long', '');
             INSERT INTO stages (id, pitch_id, name, kind, position, color) VALUES
                (10, 1, 'Messaged', 'messaging', 0, 'blue'),
                (11, 1, 'Meeting',  'standard',  1, 'amber'),
                (20, 2, 'Messaged', 'messaging', 0, 'blue'),
                (21, 2, 'Demo',     'standard',  1, 'amber'),
                (22, 2, 'Trial',    'standard',  2, 'green'),
                (23, 2, 'Renewal',  'standard',  3, 'purple');
             -- Pitch 1 wins the pipeline (2 prospects vs 1).
             INSERT INTO prospects (id, name, linkedin_url, pitch_id, stage_id) VALUES
                (100, 'A', 'https://li/a', 1, 10),
                (101, 'B', 'https://li/b', 1, 11),
                (102, 'C', 'https://li/c', 2, 23);",
        )
        .unwrap();

        run(&mut conn).unwrap();

        let stage: String = conn
            .query_row(
                "SELECT s.name FROM prospects p JOIN stages s ON s.id = p.stage_id WHERE p.id = 102",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stage, "Meeting", "position 3 clamps to the surviving pipeline's last stage");
    }
}
