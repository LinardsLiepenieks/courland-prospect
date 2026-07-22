//! All SQL for prospects. Functions take `&Connection` and return domain types;
//! the caller (command layer or the ingest HTTP server) owns connection locking.
//! Kept free of Tauri types so it stays unit-testable against an in-memory DB.
//!
//! `list`/`upsert` are `pub(crate)` (not `pub(super)`) because the loopback
//! ingest server in `crate::ingest` calls them directly, from outside `features`.

use rusqlite::{params, Connection, OptionalExtension};

use super::model::Prospect;

const COLUMNS: &str =
    "id, name, linkedin_url, headline, pitch_id, stage_id, messages_sent, awaiting_reply, note, created_at";

/// Subquery yielding a pitch's messaging (first) stage id, or NULL if the pitch
/// has no stages / is NULL. `?N` is the pitch_id bind position at the call site.
fn messaging_stage_sql(pitch_bind: &str) -> String {
    format!(
        "(SELECT id FROM stages WHERE pitch_id = {pitch_bind} AND kind = 'messaging' \
         ORDER BY position, id LIMIT 1)"
    )
}

fn get(conn: &Connection, id: i64) -> rusqlite::Result<Option<Prospect>> {
    let sql = format!("SELECT {COLUMNS} FROM prospects WHERE id = ?1");
    conn.query_row(&sql, [id], Prospect::from_row).optional()
}

/// Whether a prospect with this `linkedin_url` already exists. Lets the caller
/// tell the user "added" vs "already a prospect — updated" around an `upsert`.
pub(crate) fn exists(conn: &Connection, linkedin_url: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT 1 FROM prospects WHERE linkedin_url = ?1",
        [linkedin_url],
        |_| Ok(()),
    )
    .optional()
    .map(|found| found.is_some())
}

/// The prospect with this `linkedin_url`, or `None` if the person isn't tracked.
/// Lets the extension resolve, for the open thread, whether this person is already
/// a prospect and which pitch they're on — so it can show "Prospect of <pitch>"
/// instead of the add control, and draft each reply from that prospect's own pitch.
pub(crate) fn find_by_url(
    conn: &Connection,
    linkedin_url: &str,
) -> rusqlite::Result<Option<Prospect>> {
    let sql = format!("SELECT {COLUMNS} FROM prospects WHERE linkedin_url = ?1");
    conn.query_row(&sql, [linkedin_url], Prospect::from_row)
        .optional()
}

/// The pitch a prospect is running, or `None` when the prospect has no pitch (or
/// doesn't exist). Used by the snippet proposer to route a proposal to the right
/// pitch's library — a prospect with no pitch has no library to propose into.
pub(crate) fn pitch_id(conn: &Connection, prospect_id: i64) -> rusqlite::Result<Option<i64>> {
    conn.query_row(
        "SELECT pitch_id FROM prospects WHERE id = ?1",
        [prospect_id],
        |r| r.get::<_, Option<i64>>(0),
    )
    .optional()
    .map(Option::flatten)
}

pub(crate) fn list(conn: &Connection) -> rusqlite::Result<Vec<Prospect>> {
    let sql = format!("SELECT {COLUMNS} FROM prospects ORDER BY created_at DESC, id DESC");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], Prospect::from_row)?;
    rows.collect()
}

/// Insert a prospect, or if one with the same `linkedin_url` already exists,
/// refresh its `pitch_id` and `headline`. Never errors on duplicate — this is
/// the low-friction "add to prospects" path. `created_at`, `name`, and `note`
/// are preserved on the existing row.
///
/// A fresh insert lands in the pitch's messaging stage. On a dedup update the
/// stage is preserved when the pitch is unchanged (don't yank someone back to
/// the top of their funnel on re-capture); when the pitch actually changes, the
/// prospect moves to the new pitch's messaging stage. `messages_sent` is never
/// touched here — it's derived from captured `messages` (see `features::messages`)
/// and persists across a pitch change, so it always reflects real outreach.
pub(crate) fn upsert(
    conn: &Connection,
    name: &str,
    linkedin_url: &str,
    headline: &str,
    pitch_id: Option<i64>,
    note: &str,
) -> rusqlite::Result<Prospect> {
    let insert_stage = messaging_stage_sql("?4");
    let update_stage = messaging_stage_sql("excluded.pitch_id");
    let sql = format!(
        "INSERT INTO prospects (name, linkedin_url, headline, pitch_id, note, stage_id)
              VALUES (?1, ?2, ?3, ?4, ?5, {insert_stage})
         ON CONFLICT(linkedin_url) DO UPDATE SET
              pitch_id = excluded.pitch_id,
              headline = excluded.headline,
              stage_id = CASE
                  WHEN prospects.pitch_id IS excluded.pitch_id THEN prospects.stage_id
                  ELSE {update_stage}
              END
         RETURNING {COLUMNS}"
    );
    conn.query_row(
        &sql,
        params![name, linkedin_url, headline, pitch_id, note],
        Prospect::from_row,
    )
}

/// Move a prospect to `stage_id`. The stage must belong to the prospect's own
/// pitch (enforced in SQL) — otherwise no row changes and this returns `None`,
/// which the command surfaces as an error. Returns the updated prospect.
pub(super) fn set_stage(
    conn: &Connection,
    id: i64,
    stage_id: i64,
) -> rusqlite::Result<Option<Prospect>> {
    let changed = conn.execute(
        "UPDATE prospects SET stage_id = ?1
         WHERE id = ?2
           AND EXISTS (
               SELECT 1 FROM stages
               WHERE stages.id = ?1 AND stages.pitch_id = prospects.pitch_id
           )",
        params![stage_id, id],
    )?;
    if changed == 0 {
        return Ok(None);
    }
    get(conn, id)
}

/// Permanently remove a prospect. Returns the number of rows deleted (0 if no
/// prospect had that id) so the caller can distinguish a real delete from a
/// stale id.
pub(super) fn delete(conn: &Connection, id: i64) -> rusqlite::Result<usize> {
    conn.execute("DELETE FROM prospects WHERE id = ?1", [id])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::migrations;

    fn setup() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        migrations::run(&mut conn).unwrap();
        conn
    }

    fn seed_pitch(conn: &Connection, name: &str) -> i64 {
        conn.execute("INSERT INTO pitches (name, skill) VALUES (?1, '')", [name])
            .unwrap();
        conn.last_insert_rowid()
    }

    /// Seed a pitch together with a full-cycle pipeline, returning the pitch id
    /// and its ordered stage ids.
    fn seed_pitch_with_stages(conn: &Connection, name: &str) -> (i64, Vec<i64>) {
        let pitch = seed_pitch(conn, name);
        let stages = crate::features::stages::repository::create_many(
            conn,
            pitch,
            &crate::features::stages::repository::full_cycle_template(),
        )
        .unwrap();
        (pitch, stages.into_iter().map(|s| s.id).collect())
    }

    #[test]
    fn upsert_inserts_then_lists() {
        let conn = setup();
        let pitch = seed_pitch(&conn, "Design-in-code");
        let p = upsert(
            &conn,
            "Ada Lovelace",
            "https://www.linkedin.com/in/ada/",
            "Analyst",
            Some(pitch),
            "",
        )
        .unwrap();
        assert!(p.id > 0);
        assert_eq!(p.name, "Ada Lovelace");
        assert_eq!(p.linkedin_url, "https://www.linkedin.com/in/ada/");
        assert_eq!(p.pitch_id, Some(pitch));
        assert!(!p.created_at.is_empty());

        assert_eq!(list(&conn).unwrap().len(), 1);
    }

    #[test]
    fn upsert_dedups_on_url_and_updates_pitch() {
        let conn = setup();
        let pitch_a = seed_pitch(&conn, "A");
        let pitch_b = seed_pitch(&conn, "B");
        let url = "https://www.linkedin.com/in/grace/";

        let first = upsert(&conn, "Grace", url, "Rear Admiral", Some(pitch_a), "note").unwrap();
        // Re-add the same person with a different pitch + refreshed headline.
        let second = upsert(&conn, "Grace H.", url, "Computer Scientist", Some(pitch_b), "").unwrap();

        // Same row (dedup), not a duplicate.
        assert_eq!(first.id, second.id);
        assert_eq!(list(&conn).unwrap().len(), 1);
        // pitch_id + headline were updated...
        assert_eq!(second.pitch_id, Some(pitch_b));
        assert_eq!(second.headline, "Computer Scientist");
        // ...while name, note, and created_at were preserved.
        assert_eq!(second.name, "Grace");
        assert_eq!(second.note, "note");
        assert_eq!(second.created_at, first.created_at);
    }

    #[test]
    fn upsert_allows_null_pitch() {
        let conn = setup();
        let p = upsert(&conn, "No Pitch", "https://www.linkedin.com/in/x/", "", None, "").unwrap();
        assert_eq!(p.pitch_id, None);
    }

    #[test]
    fn upsert_lands_new_prospect_in_messaging_stage() {
        let conn = setup();
        let (pitch, stages) = seed_pitch_with_stages(&conn, "P");
        let p = upsert(&conn, "Ada", "https://li/ada", "", Some(pitch), "").unwrap();
        assert_eq!(p.stage_id, Some(stages[0])); // Messaged
        assert_eq!(p.messages_sent, 0);
    }

    /// Seed a prospect's derived `messages_sent` directly (the counter is
    /// normally maintained by `features::messages`; tests just need a value).
    fn seed_messages_sent(conn: &Connection, id: i64, n: i64) {
        conn.execute(
            "UPDATE prospects SET messages_sent = ?1 WHERE id = ?2",
            params![n, id],
        )
        .unwrap();
    }

    #[test]
    fn upsert_preserves_stage_on_same_pitch_recapture() {
        let conn = setup();
        let (pitch, stages) = seed_pitch_with_stages(&conn, "P");
        let url = "https://li/ada";
        let p = upsert(&conn, "Ada", url, "", Some(pitch), "").unwrap();
        // Advance them into the pipeline and rack up messages.
        set_stage(&conn, p.id, stages[2]).unwrap();
        seed_messages_sent(&conn, p.id, 3);
        // Re-capture on the same pitch must not reset stage/messages.
        let again = upsert(&conn, "Ada", url, "New headline", Some(pitch), "").unwrap();
        assert_eq!(again.stage_id, Some(stages[2]));
        assert_eq!(again.messages_sent, 3);
        assert_eq!(again.headline, "New headline");
    }

    #[test]
    fn upsert_moves_stage_on_pitch_change_but_keeps_message_count() {
        let conn = setup();
        let (pitch_a, stages_a) = seed_pitch_with_stages(&conn, "A");
        let (pitch_b, stages_b) = seed_pitch_with_stages(&conn, "B");
        let url = "https://li/ada";
        let p = upsert(&conn, "Ada", url, "", Some(pitch_a), "").unwrap();
        set_stage(&conn, p.id, stages_a[2]).unwrap();
        seed_messages_sent(&conn, p.id, 5);
        // Re-capture onto a different pitch: land in B's messaging stage. The
        // derived count is NOT reset — it reflects real messages sent to them.
        let moved = upsert(&conn, "Ada", url, "", Some(pitch_b), "").unwrap();
        assert_eq!(moved.stage_id, Some(stages_b[0]));
        assert_eq!(moved.messages_sent, 5);
    }

    #[test]
    fn set_stage_rejects_stage_from_another_pitch() {
        let conn = setup();
        let (pitch_a, stages_a) = seed_pitch_with_stages(&conn, "A");
        let (_pitch_b, stages_b) = seed_pitch_with_stages(&conn, "B");
        let p = upsert(&conn, "Ada", "https://li/ada", "", Some(pitch_a), "").unwrap();
        // A stage from pitch B is not valid for a pitch-A prospect.
        assert!(set_stage(&conn, p.id, stages_b[1]).unwrap().is_none());
        // A stage from its own pitch works.
        assert_eq!(
            set_stage(&conn, p.id, stages_a[1]).unwrap().unwrap().stage_id,
            Some(stages_a[1])
        );
    }

    #[test]
    fn find_by_url_returns_prospect_with_pitch_or_none() {
        let conn = setup();
        let pitch = seed_pitch(&conn, "Design-in-code");
        let url = "https://www.linkedin.com/in/ada/";
        upsert(&conn, "Ada", url, "Analyst", Some(pitch), "").unwrap();

        let found = find_by_url(&conn, url).unwrap().expect("prospect exists");
        assert_eq!(found.name, "Ada");
        assert_eq!(found.pitch_id, Some(pitch));

        // An untracked person has no row.
        assert!(find_by_url(&conn, "https://www.linkedin.com/in/nobody/")
            .unwrap()
            .is_none());
    }

    #[test]
    fn delete_removes_row() {
        let conn = setup();
        let p = upsert(&conn, "Gone", "https://www.linkedin.com/in/gone/", "", None, "").unwrap();
        assert_eq!(list(&conn).unwrap().len(), 1);
        assert_eq!(delete(&conn, p.id).unwrap(), 1);
        assert!(list(&conn).unwrap().is_empty());
    }

    #[test]
    fn delete_missing_returns_zero() {
        let conn = setup();
        assert_eq!(delete(&conn, 999).unwrap(), 0);
    }

    #[test]
    fn deleting_pitch_nulls_prospect_link() {
        let conn = setup();
        let pitch = seed_pitch(&conn, "Temp");
        let url = "https://www.linkedin.com/in/y/";
        upsert(&conn, "Y", url, "", Some(pitch), "").unwrap();

        conn.execute("DELETE FROM pitches WHERE id = ?1", [pitch]).unwrap();

        let after = &list(&conn).unwrap()[0];
        assert_eq!(after.pitch_id, None); // ON DELETE SET NULL kept the prospect.
    }
}
