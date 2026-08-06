//! All SQL for prospects. Functions take `&Connection` and return domain types;
//! the caller (command layer or the ingest HTTP server) owns connection locking.
//! Kept free of Tauri types so it stays unit-testable against an in-memory DB.
//!
//! `list`/`upsert`/`find_by_url` are `pub(crate)` (not `pub(super)`) because the
//! loopback ingest server in `crate::ingest` calls them directly, from outside
//! `features`.

use rusqlite::{params, Connection, OptionalExtension};

use super::model::Prospect;

const COLUMNS: &str = "id, name, linkedin_url, headline, customer_id, stage_id, \
     messages_sent, awaiting_reply, last_outreach_at, suggested_stage_id, \
     suggested_reason, note, created_at";

/// Subquery yielding the pipeline's messaging (first) stage id. There is one
/// pipeline now, so this takes no owner — a freshly captured prospect always has
/// the same place to land.
const MESSAGING_STAGE: &str =
    "(SELECT id FROM stages WHERE kind = 'messaging' ORDER BY position, id LIMIT 1)";

fn get(conn: &Connection, id: i64) -> rusqlite::Result<Option<Prospect>> {
    let sql = format!("SELECT {COLUMNS} FROM prospects WHERE id = ?1");
    conn.query_row(&sql, [id], Prospect::from_row).optional()
}

/// One prospect by id, for callers outside this feature (the advance analyzer
/// needs their stage and customer before it can judge a thread). Read-only.
pub(crate) fn find(conn: &Connection, id: i64) -> rusqlite::Result<Option<Prospect>> {
    get(conn, id)
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
/// Lets the extension resolve, for the open thread, whether this person is
/// already a prospect and which customer profile they match — so it can show
/// "Prospect · <customer>" instead of the add control, and draft each reply
/// steered toward that profile's goal.
pub(crate) fn find_by_url(
    conn: &Connection,
    linkedin_url: &str,
) -> rusqlite::Result<Option<Prospect>> {
    let sql = format!("SELECT {COLUMNS} FROM prospects WHERE linkedin_url = ?1");
    conn.query_row(&sql, [linkedin_url], Prospect::from_row)
        .optional()
}

pub(crate) fn list(conn: &Connection) -> rusqlite::Result<Vec<Prospect>> {
    let sql = format!("SELECT {COLUMNS} FROM prospects ORDER BY created_at DESC, id DESC");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], Prospect::from_row)?;
    rows.collect()
}

/// Insert a prospect, or if one with the same `linkedin_url` already exists,
/// refresh its `customer_id` and `headline`. Never errors on duplicate — this is
/// the low-friction "add to prospects" path. `created_at`, `name`, and `note`
/// are preserved on the existing row.
///
/// A re-capture can SET or CHANGE the customer profile but never CLEARS it: a
/// `None` leaves whatever the row already had. Unassigned is the extension's
/// default pick, so without this an accidental re-add — or one made while the
/// picker hadn't loaded — would wipe a tag the user set deliberately, and the
/// widget switches to its read-only pill afterwards, so it couldn't be repaired
/// from LinkedIn. Clearing a profile is an app-side action (the prospect row's
/// customer menu), where it's explicit and reversible.
///
/// A fresh insert lands in the messaging stage. A re-capture NEVER moves anyone:
/// with one shared pipeline, re-tagging someone's customer profile says nothing
/// about how far along the conversation is, so their stage is left exactly where
/// it was. (Under the old per-pitch pipelines a pitch change had to relocate them,
/// because the stage they sat in belonged to a different funnel.) `messages_sent`
/// is never touched here either — it's derived from captured `messages` (see
/// `features::messages`).
pub(crate) fn upsert(
    conn: &Connection,
    name: &str,
    linkedin_url: &str,
    headline: &str,
    customer_id: Option<i64>,
    note: &str,
) -> rusqlite::Result<Prospect> {
    let sql = format!(
        "INSERT INTO prospects (name, linkedin_url, headline, customer_id, note, stage_id)
              VALUES (?1, ?2, ?3, ?4, ?5, {MESSAGING_STAGE})
         ON CONFLICT(linkedin_url) DO UPDATE SET
              customer_id = COALESCE(excluded.customer_id, prospects.customer_id),
              headline    = excluded.headline
         RETURNING {COLUMNS}"
    );
    conn.query_row(
        &sql,
        params![name, linkedin_url, headline, customer_id, note],
        Prospect::from_row,
    )
}

/// Move a prospect to `stage_id`. The stage must exist (enforced in SQL) —
/// otherwise no row changes and this returns `None`, which the command surfaces
/// as an error. With one shared pipeline there's no per-owner check left to make.
///
/// Any move clears a pending advance suggestion, whatever its target. The
/// suggestion was reasoning about the stage the prospect was in a moment ago; the
/// instant they're somewhere else it's stale, and leaving it up would offer to
/// re-advance someone you just moved by hand — or, worse, keep offering a move
/// you already took.
pub(super) fn set_stage(
    conn: &Connection,
    id: i64,
    stage_id: i64,
) -> rusqlite::Result<Option<Prospect>> {
    let changed = conn.execute(
        "UPDATE prospects
         SET stage_id = ?1, suggested_stage_id = NULL, suggested_reason = ''
         WHERE id = ?2 AND EXISTS (SELECT 1 FROM stages WHERE stages.id = ?1)",
        params![stage_id, id],
    )?;
    if changed == 0 {
        return Ok(None);
    }
    get(conn, id)
}

/// Record the analyzer's verdict: this prospect looks ready for `stage_id`,
/// because `reason`. Overwrites any previous pending suggestion — the newest
/// read of the thread supersedes an older one.
///
/// Guarded in SQL against suggesting the stage the prospect is *already* in,
/// which would render a card offering to move it nowhere. That's a cheap
/// backstop for a confused model rather than an expected case; the analyzer only
/// ever asks about the next stage.
pub(crate) fn suggest_stage(
    conn: &Connection,
    id: i64,
    stage_id: i64,
    reason: &str,
) -> rusqlite::Result<Option<Prospect>> {
    let changed = conn.execute(
        "UPDATE prospects
         SET suggested_stage_id = ?1, suggested_reason = ?2
         WHERE id = ?3
           AND stage_id IS NOT ?1
           AND EXISTS (SELECT 1 FROM stages WHERE stages.id = ?1)",
        params![stage_id, reason, id],
    )?;
    if changed == 0 {
        return Ok(None);
    }
    get(conn, id)
}

/// Retract a pending suggestion without moving anyone — the "dismiss" half of
/// the card's accept/dismiss pair. Idempotent: dismissing a prospect with no
/// suggestion is a no-op that still returns the row, so a double-click can't
/// turn into an error.
pub(super) fn clear_suggestion(conn: &Connection, id: i64) -> rusqlite::Result<Option<Prospect>> {
    conn.execute(
        "UPDATE prospects SET suggested_stage_id = NULL, suggested_reason = '' WHERE id = ?1",
        [id],
    )?;
    get(conn, id)
}

/// Re-tag which customer profile a prospect matches, or clear it (`None`).
/// Re-tagging is deliberately cheap and reversible: it changes only how their
/// drafts are steered, never their place on the board. Returns `None` when the
/// prospect doesn't exist; a `customer_id` that doesn't exist trips the foreign
/// key, which the command maps to a plain message.
pub(super) fn set_customer(
    conn: &Connection,
    id: i64,
    customer_id: Option<i64>,
) -> rusqlite::Result<Option<Prospect>> {
    let changed = conn.execute(
        "UPDATE prospects SET customer_id = ?1 WHERE id = ?2",
        params![customer_id, id],
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
    use crate::features::stages::repository as stages_repo;

    fn setup() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        migrations::run(&mut conn).unwrap();
        conn
    }

    fn seed_customer(conn: &Connection, name: &str) -> i64 {
        conn.execute("INSERT INTO customers (name) VALUES (?1)", [name])
            .unwrap();
        conn.last_insert_rowid()
    }

    /// The one shared pipeline's stage ids, in funnel order.
    fn stage_ids(conn: &Connection) -> Vec<i64> {
        stages_repo::list(conn).unwrap().into_iter().map(|s| s.id).collect()
    }

    #[test]
    fn upsert_inserts_then_lists() {
        let conn = setup();
        let customer = seed_customer(&conn, "Solo agencies");
        let p = upsert(
            &conn,
            "Ada Lovelace",
            "https://www.linkedin.com/in/ada/",
            "Analyst",
            Some(customer),
            "",
        )
        .unwrap();
        assert!(p.id > 0);
        assert_eq!(p.name, "Ada Lovelace");
        assert_eq!(p.linkedin_url, "https://www.linkedin.com/in/ada/");
        assert_eq!(p.customer_id, Some(customer));
        assert!(!p.created_at.is_empty());

        assert_eq!(list(&conn).unwrap().len(), 1);
    }

    #[test]
    fn upsert_dedups_on_url_and_updates_customer() {
        let conn = setup();
        let a = seed_customer(&conn, "A");
        let b = seed_customer(&conn, "B");
        let url = "https://www.linkedin.com/in/grace/";

        let first = upsert(&conn, "Grace", url, "Rear Admiral", Some(a), "note").unwrap();
        // Re-add the same person under a different profile + refreshed headline.
        let second = upsert(&conn, "Grace H.", url, "Computer Scientist", Some(b), "").unwrap();

        // Same row (dedup), not a duplicate.
        assert_eq!(first.id, second.id);
        assert_eq!(list(&conn).unwrap().len(), 1);
        // customer_id + headline were updated...
        assert_eq!(second.customer_id, Some(b));
        assert_eq!(second.headline, "Computer Scientist");
        // ...while name, note, and created_at were preserved.
        assert_eq!(second.name, "Grace");
        assert_eq!(second.note, "note");
        assert_eq!(second.created_at, first.created_at);
    }

    /// The extension's picker defaults to "no profile" and can be re-added by
    /// accident (or with an unloaded list), so a `None` must never clear a tag the
    /// user set. Only the app can unassign someone.
    #[test]
    fn recapture_without_a_customer_keeps_the_existing_one() {
        let conn = setup();
        let a = seed_customer(&conn, "Solo agencies");
        let url = "https://www.linkedin.com/in/ada/";

        upsert(&conn, "Ada", url, "Analyst", Some(a), "").unwrap();
        let again = upsert(&conn, "Ada", url, "Head of Analysis", None, "").unwrap();

        assert_eq!(again.customer_id, Some(a), "an unassigned re-add wiped the tag");
        // The headline still refreshes — only the profile is protected.
        assert_eq!(again.headline, "Head of Analysis");
    }

    #[test]
    fn upsert_allows_no_customer() {
        let conn = setup();
        let p = upsert(&conn, "Unassigned", "https://www.linkedin.com/in/x/", "", None, "").unwrap();
        assert_eq!(p.customer_id, None);
        // Still in the pipeline — that's the point of one shared funnel.
        assert_eq!(p.stage_id, Some(stage_ids(&conn)[0]));
    }

    #[test]
    fn upsert_lands_new_prospect_in_the_messaging_stage() {
        let conn = setup();
        let p = upsert(&conn, "Ada", "https://li/ada", "", None, "").unwrap();
        assert_eq!(p.stage_id, Some(stage_ids(&conn)[0]));
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

    /// The behavior change the shared pipeline buys: re-capturing someone under a
    /// DIFFERENT customer profile must not drag them back to the top of the
    /// funnel. Under per-pitch pipelines it had to (their stage belonged to
    /// another funnel); now the stage means the same thing for everyone, so
    /// re-tagging is purely a steering change.
    #[test]
    fn recapture_never_moves_a_prospect_on_the_board() {
        let conn = setup();
        let a = seed_customer(&conn, "A");
        let b = seed_customer(&conn, "B");
        let stages = stage_ids(&conn);
        let url = "https://li/ada";

        let p = upsert(&conn, "Ada", url, "", Some(a), "").unwrap();
        set_stage(&conn, p.id, stages[2]).unwrap();
        seed_messages_sent(&conn, p.id, 3);

        // Same profile, then a different one — neither disturbs their position.
        let same = upsert(&conn, "Ada", url, "New headline", Some(a), "").unwrap();
        assert_eq!(same.stage_id, Some(stages[2]));
        assert_eq!(same.messages_sent, 3);

        let moved = upsert(&conn, "Ada", url, "", Some(b), "").unwrap();
        assert_eq!(moved.customer_id, Some(b), "the profile is re-tagged");
        assert_eq!(moved.stage_id, Some(stages[2]), "their progress is not");
        assert_eq!(moved.messages_sent, 3);
    }

    #[test]
    fn set_stage_accepts_any_pipeline_stage_and_rejects_a_missing_one() {
        let conn = setup();
        let stages = stage_ids(&conn);
        let p = upsert(&conn, "Ada", "https://li/ada", "", None, "").unwrap();
        assert_eq!(
            set_stage(&conn, p.id, stages[1]).unwrap().unwrap().stage_id,
            Some(stages[1])
        );
        assert!(set_stage(&conn, p.id, 9999).unwrap().is_none());
    }

    #[test]
    fn set_customer_retags_and_can_clear() {
        let conn = setup();
        let customer = seed_customer(&conn, "Agencies");
        let stages = stage_ids(&conn);
        let p = upsert(&conn, "Ada", "https://li/ada", "", None, "").unwrap();
        set_stage(&conn, p.id, stages[2]).unwrap();

        let tagged = set_customer(&conn, p.id, Some(customer)).unwrap().unwrap();
        assert_eq!(tagged.customer_id, Some(customer));
        assert_eq!(tagged.stage_id, Some(stages[2]), "re-tagging never moves them");

        let cleared = set_customer(&conn, p.id, None).unwrap().unwrap();
        assert_eq!(cleared.customer_id, None);

        assert!(set_customer(&conn, 999, None).unwrap().is_none());
    }

    #[test]
    fn set_customer_rejects_an_unknown_profile() {
        let conn = setup();
        let p = upsert(&conn, "Ada", "https://li/ada", "", None, "").unwrap();
        assert!(
            set_customer(&conn, p.id, Some(9999)).is_err(),
            "the foreign key rejects a customer profile that doesn't exist"
        );
    }

    #[test]
    fn suggest_stage_records_the_target_and_reason() {
        let conn = setup();
        let stages = stage_ids(&conn);
        let p = upsert(&conn, "Ada", "https://li/ada", "", None, "").unwrap();
        assert_eq!(p.suggested_stage_id, None, "a fresh prospect has no suggestion");
        assert_eq!(p.suggested_reason, "");

        let suggested = suggest_stage(&conn, p.id, stages[1], "they asked for a call")
            .unwrap()
            .unwrap();
        assert_eq!(suggested.suggested_stage_id, Some(stages[1]));
        assert_eq!(suggested.suggested_reason, "they asked for a call");
        assert_eq!(suggested.stage_id, Some(stages[0]), "suggesting never moves them");

        // A newer read of the thread supersedes the older suggestion.
        let again = suggest_stage(&conn, p.id, stages[2], "they signed").unwrap().unwrap();
        assert_eq!(again.suggested_stage_id, Some(stages[2]));
        assert_eq!(again.suggested_reason, "they signed");
    }

    #[test]
    fn suggest_stage_refuses_the_stage_theyre_already_in_or_one_that_doesnt_exist() {
        let conn = setup();
        let stages = stage_ids(&conn);
        let p = upsert(&conn, "Ada", "https://li/ada", "", None, "").unwrap();

        // A card offering to move someone where they already are is not a card.
        assert!(suggest_stage(&conn, p.id, stages[0], "hm").unwrap().is_none());
        assert!(suggest_stage(&conn, p.id, 9999, "hm").unwrap().is_none());
        assert!(suggest_stage(&conn, 9999, stages[1], "hm").unwrap().is_none());
        assert_eq!(find(&conn, p.id).unwrap().unwrap().suggested_stage_id, None);
    }

    /// Moving someone — by hand OR by accepting the suggestion — invalidates the
    /// analyzer's read of where they were. The card must not keep offering it.
    #[test]
    fn any_stage_move_clears_a_pending_suggestion() {
        let conn = setup();
        let stages = stage_ids(&conn);
        let p = upsert(&conn, "Ada", "https://li/ada", "", None, "").unwrap();
        suggest_stage(&conn, p.id, stages[1], "they asked for a call").unwrap();

        // Accepting it: move to the suggested stage.
        let accepted = set_stage(&conn, p.id, stages[1]).unwrap().unwrap();
        assert_eq!(accepted.stage_id, Some(stages[1]));
        assert_eq!(accepted.suggested_stage_id, None);
        assert_eq!(accepted.suggested_reason, "");

        // A move to some OTHER stage clears it too — the suggestion reasoned
        // about a position they've now left.
        suggest_stage(&conn, p.id, stages[2], "onboarding time").unwrap();
        let elsewhere = set_stage(&conn, p.id, stages[3]).unwrap().unwrap();
        assert_eq!(elsewhere.suggested_stage_id, None);
    }

    #[test]
    fn clear_suggestion_dismisses_without_moving_and_is_idempotent() {
        let conn = setup();
        let stages = stage_ids(&conn);
        let p = upsert(&conn, "Ada", "https://li/ada", "", None, "").unwrap();
        suggest_stage(&conn, p.id, stages[1], "they asked for a call").unwrap();

        let dismissed = clear_suggestion(&conn, p.id).unwrap().unwrap();
        assert_eq!(dismissed.suggested_stage_id, None);
        assert_eq!(dismissed.suggested_reason, "");
        assert_eq!(dismissed.stage_id, Some(stages[0]), "dismissing never moves them");

        // A second dismissal is a no-op, not an error — a double-click is cheap.
        assert!(clear_suggestion(&conn, p.id).unwrap().is_some());
        assert!(clear_suggestion(&conn, 9999).unwrap().is_none());
    }

    /// Re-tagging the customer profile must not disturb a pending suggestion:
    /// the analyzer reasons about the STAGE goal, which is customer-independent.
    #[test]
    fn retagging_the_customer_leaves_a_suggestion_standing() {
        let conn = setup();
        let stages = stage_ids(&conn);
        let customer = seed_customer(&conn, "Agencies");
        let p = upsert(&conn, "Ada", "https://li/ada", "", None, "").unwrap();
        suggest_stage(&conn, p.id, stages[1], "they asked for a call").unwrap();

        let retagged = set_customer(&conn, p.id, Some(customer)).unwrap().unwrap();
        assert_eq!(retagged.suggested_stage_id, Some(stages[1]));
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

    /// Deleting a customer profile unassigns its prospects rather than deleting
    /// them — the SET NULL safety net, which is now the *intended* behavior
    /// rather than a fallback (the old pitch delete removed its prospects).
    #[test]
    fn deleting_a_customer_leaves_the_prospect_in_the_pipeline() {
        let conn = setup();
        let customer = seed_customer(&conn, "Temp");
        let url = "https://www.linkedin.com/in/y/";
        upsert(&conn, "Y", url, "", Some(customer), "").unwrap();

        conn.execute("DELETE FROM customers WHERE id = ?1", [customer])
            .unwrap();

        let after = &list(&conn).unwrap()[0];
        assert_eq!(after.customer_id, None);
        assert!(after.stage_id.is_some(), "still on the board");
    }
}
