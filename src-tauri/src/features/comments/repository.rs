//! All SQL for the comment inbox (`comment_drafts`) and the commenter control
//! record (`comment_run`). Functions take `&Connection` and return domain types;
//! the caller (the Tauri command layer, or the loopback ingest server the
//! extension drives) owns connection locking. Free of Tauri types so it stays
//! unit-testable against an in-memory DB.
//!
//! `pub(crate)` (not `pub(super)`) because `crate::ingest` reads and writes these
//! directly — the extension is the headless worker behind this data.

use rusqlite::{params, Connection, OptionalExtension};

use super::model::{CommentDraft, CommentRun};

const DRAFT_COLUMNS: &str =
    "id, permalink, author_name, post_text, comment, status, error, created_at, posted_at";
const RUN_COLUMNS: &str = "status, count, include_watchlist, updated_at";

// The `comment_drafts.status` lifecycle values (see the 0020 migration). Kept as
// constants so the repository, and the state machine it enforces, read in one place.
pub(crate) const STATUS_DRAFT: &str = "draft";
pub(crate) const STATUS_QUEUED: &str = "queued";
pub(crate) const STATUS_POSTING: &str = "posting";
pub(crate) const STATUS_POSTED: &str = "posted";
pub(crate) const STATUS_FAILED: &str = "failed";

// ── The inbox (comment_drafts) ───────────────────────────────────────────────

/// Every drafted comment, newest-first. The set grows slowly (one row per scraped
/// post) and the inbox shows all of it, so returning everything is fine.
pub(crate) fn list(conn: &Connection) -> rusqlite::Result<Vec<CommentDraft>> {
    let sql = format!("SELECT {DRAFT_COLUMNS} FROM comment_drafts ORDER BY created_at DESC, id DESC");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], CommentDraft::from_row)?;
    rows.collect()
}

/// Whether a post is already handled — either still in the inbox (a draft row in
/// any status) OR in the durable `commented_posts` ledger (a comment we've already
/// posted, even if its draft was since deleted). A future scrape skips these so it
/// only surfaces posts we haven't engaged yet. The ledger half is what keeps a
/// deleted-but-posted draft from resurfacing and getting a SECOND public comment —
/// `comment_drafts` alone can't, since deleting the row removes it from the filter.
pub(crate) fn exists(conn: &Connection, permalink: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT 1 FROM comment_drafts WHERE permalink = ?1
         UNION ALL
         SELECT 1 FROM commented_posts WHERE permalink = ?1
         LIMIT 1",
        [permalink],
        |_| Ok(()),
    )
    .optional()
    .map(|found| found.is_some())
}

/// Whether we've already posted a comment on this post (present in the durable
/// `commented_posts` ledger). The post-claim path consults this so a draft that
/// somehow re-reaches `queued` for a post we already commented on is never posted
/// again — a defensive backstop behind the extension's own idempotency check.
pub(crate) fn already_commented(conn: &Connection, permalink: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT 1 FROM commented_posts WHERE permalink = ?1",
        [permalink],
        |_| Ok(()),
    )
    .optional()
    .map(|found| found.is_some())
}

/// Insert a freshly-generated draft (status `draft`). Deduped on `permalink`:
/// returns the new row, or `None` if a row for that permalink already existed
/// (a concurrent scrape won the race) — the caller treats `None` as "already
/// handled", never an error.
pub(crate) fn insert_generated(
    conn: &Connection,
    permalink: &str,
    author_name: &str,
    post_text: &str,
    comment: &str,
) -> rusqlite::Result<Option<CommentDraft>> {
    let sql = format!(
        "INSERT INTO comment_drafts (permalink, author_name, post_text, comment, status)
              VALUES (?1, ?2, ?3, ?4, '{STATUS_DRAFT}')
         ON CONFLICT(permalink) DO NOTHING
         RETURNING {DRAFT_COLUMNS}"
    );
    conn.query_row(&sql, params![permalink, author_name, post_text, comment], CommentDraft::from_row)
        .optional()
}

/// Save an edited comment's text. Allowed only while the draft is still editable
/// (`draft`, `queued`, or `failed`) — never for one that's `posting`/`posted`, so
/// an edit can't race a post in flight or rewrite history. Returns the updated row,
/// or `None` when no editable row has that id.
pub(crate) fn update_comment(
    conn: &Connection,
    id: i64,
    comment: &str,
) -> rusqlite::Result<Option<CommentDraft>> {
    let sql = format!(
        "UPDATE comment_drafts SET comment = ?2
          WHERE id = ?1 AND status IN ('{STATUS_DRAFT}', '{STATUS_QUEUED}', '{STATUS_FAILED}')
         RETURNING {DRAFT_COLUMNS}"
    );
    conn.query_row(&sql, params![id, comment], CommentDraft::from_row)
        .optional()
}

/// Dismiss a draft. Returns rows deleted (0 = stale id) so the caller can tell a
/// real delete from a no-op. A dismissed post can resurface on a later scrape
/// (there's no "never again" record — deleting it removes it from the skip set).
pub(crate) fn delete(conn: &Connection, id: i64) -> rusqlite::Result<usize> {
    conn.execute("DELETE FROM comment_drafts WHERE id = ?1", [id])
}

/// Approve for posting: move every editable, non-empty draft (`draft` or a
/// retried `failed`) to `queued`, clearing any prior error. Skips blank comments
/// (nothing to post) and rows already `queued`/`posting`/`posted`. Returns how many
/// were queued.
pub(crate) fn queue_all(conn: &Connection) -> rusqlite::Result<usize> {
    conn.execute(
        &format!(
            "UPDATE comment_drafts SET status = '{STATUS_QUEUED}', error = ''
              WHERE status IN ('{STATUS_DRAFT}', '{STATUS_FAILED}') AND trim(comment) <> ''"
        ),
        [],
    )
}

/// Claim up to `limit` queued drafts for posting: atomically flip them
/// `queued` → `posting` and return them (oldest first, so posting matches scrape
/// order). The flip IS the claim — a second poller (or a re-fired alarm) fetching
/// after this gets the next batch, never these, so an evicted-and-restarted worker
/// can't double-post. Returns an empty vec when nothing is queued.
pub(crate) fn claim_queued(conn: &Connection, limit: u32) -> rusqlite::Result<Vec<CommentDraft>> {
    let sql = format!(
        "UPDATE comment_drafts SET status = '{STATUS_POSTING}'
          WHERE id IN (
              SELECT id FROM comment_drafts
               WHERE status = '{STATUS_QUEUED}'
                 AND permalink NOT IN (SELECT permalink FROM commented_posts)
              ORDER BY created_at ASC, id ASC LIMIT ?1
          )
         RETURNING {DRAFT_COLUMNS}"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([limit], CommentDraft::from_row)?;
    rows.collect()
}

/// Record the outcome of a post attempt. `status` is `posted` (sets `posted_at`,
/// clears `error`) or `failed` (records `error`, leaves it retryable via
/// {@link queue_all}). Returns rows changed (0 = stale id). The caller validates
/// `status` is one of those two.
///
/// On `posted`, the post's permalink is also written to the durable
/// `commented_posts` ledger (deduped) — the "never again" record that survives the
/// draft row being deleted, so a re-scrape can't re-surface the post and comment on
/// it twice. Both writes run under the same held connection lock, so they can't
/// interleave with another writer.
pub(crate) fn set_status(
    conn: &Connection,
    id: i64,
    status: &str,
    error: &str,
) -> rusqlite::Result<usize> {
    let changed = conn.execute(
        &format!(
            "UPDATE comment_drafts
                SET status = ?2,
                    error = ?3,
                    posted_at = CASE WHEN ?2 = '{STATUS_POSTED}' THEN datetime('now') ELSE posted_at END
              WHERE id = ?1"
        ),
        params![id, status, error],
    )?;
    if status == STATUS_POSTED && changed > 0 {
        conn.execute(
            "INSERT INTO commented_posts (permalink)
                 SELECT permalink FROM comment_drafts WHERE id = ?1
             ON CONFLICT(permalink) DO NOTHING",
            [id],
        )?;
    }
    Ok(changed)
}

// ── The control record (comment_run) ─────────────────────────────────────────

/// The singleton run record. Present from the migration's seed insert, so this
/// always finds its row.
pub(crate) fn get_run(conn: &Connection) -> rusqlite::Result<CommentRun> {
    conn.query_row(
        &format!("SELECT {RUN_COLUMNS} FROM comment_run WHERE id = 1"),
        [],
        CommentRun::from_row,
    )
}

/// Request a scrape: force the run to `requested` with the given budget/watchlist
/// choice, regardless of prior state. Forcing (not gating on `idle`) self-heals a
/// run wedged in `scraping` — e.g. the browser closed mid-scrape — so the button
/// always works on the next click.
pub(crate) fn request_scrape(
    conn: &Connection,
    count: i64,
    include_watchlist: bool,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE comment_run
            SET status = 'requested', count = ?1, include_watchlist = ?2, updated_at = datetime('now')
          WHERE id = 1",
        params![count, include_watchlist as i64],
    )?;
    Ok(())
}

/// Finish a scrape: return the run to `idle`, but ONLY if it's still `scraping`.
/// The guard is what makes a re-request safe: if the user asked for another scrape
/// while this one was running (status flipped back to `requested`), the finishing
/// worker must not clobber that fresh request with `idle` — so it matches nothing
/// and the new run survives to be claimed. Returns rows changed (0 = a newer
/// request took over, or the run was already idle).
pub(crate) fn finish_scrape(conn: &Connection) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE comment_run SET status = 'idle', updated_at = datetime('now')
          WHERE id = 1 AND status = 'scraping'",
        [],
    )
}

/// The extension's scrape claim: if a run is `requested`, flip it to `scraping`
/// and return it (so the worker learns the `count` + `include_watchlist` to use);
/// otherwise `None`. The flip IS the claim — two polling workers can't both start
/// the same run.
pub(crate) fn take_requested(conn: &Connection) -> rusqlite::Result<Option<CommentRun>> {
    conn.query_row(
        &format!(
            "UPDATE comment_run SET status = 'scraping', updated_at = datetime('now')
              WHERE id = 1 AND status = 'requested'
             RETURNING {RUN_COLUMNS}"
        ),
        [],
        CommentRun::from_row,
    )
    .optional()
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

    fn insert(conn: &Connection, permalink: &str, comment: &str) -> CommentDraft {
        insert_generated(conn, permalink, "Ada", "a post", comment)
            .unwrap()
            .expect("fresh insert returns the row")
    }

    #[test]
    fn insert_dedups_on_permalink() {
        let conn = setup();
        let first = insert(&conn, "https://li/p/1", "nice");
        assert!(first.id > 0);
        assert_eq!(first.status, STATUS_DRAFT);
        // A second insert for the same permalink inserts nothing and returns None.
        assert!(insert_generated(&conn, "https://li/p/1", "x", "y", "again")
            .unwrap()
            .is_none());
        assert_eq!(list(&conn).unwrap().len(), 1);
        assert!(exists(&conn, "https://li/p/1").unwrap());
        assert!(!exists(&conn, "https://li/p/2").unwrap());
    }

    #[test]
    fn update_comment_only_while_editable() {
        let conn = setup();
        let d = insert(&conn, "https://li/p/1", "draft text");
        let updated = update_comment(&conn, d.id, "edited").unwrap().unwrap();
        assert_eq!(updated.comment, "edited");
        // Once posting, an edit is rejected (returns None, row unchanged).
        set_status(&conn, d.id, STATUS_POSTING, "").unwrap();
        assert!(update_comment(&conn, d.id, "too late").unwrap().is_none());
    }

    #[test]
    fn queue_skips_blank_and_moves_editable() {
        let conn = setup();
        let a = insert(&conn, "https://li/p/1", "ready");
        let _blank = insert(&conn, "https://li/p/2", "   ");
        // A posted row must not be re-queued.
        let done = insert(&conn, "https://li/p/3", "already");
        set_status(&conn, done.id, STATUS_POSTED, "").unwrap();

        assert_eq!(queue_all(&conn).unwrap(), 1, "only the one non-blank editable draft");
        let queued = list(&conn).unwrap();
        let a_row = queued.iter().find(|r| r.id == a.id).unwrap();
        assert_eq!(a_row.status, STATUS_QUEUED);
    }

    #[test]
    fn claim_queued_flips_to_posting_and_wont_reclaim() {
        let conn = setup();
        insert(&conn, "https://li/p/1", "one");
        insert(&conn, "https://li/p/2", "two");
        queue_all(&conn).unwrap();

        let first = claim_queued(&conn, 1).unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].status, STATUS_POSTING);
        // The claimed one isn't handed out again; the next claim gets the other.
        let second = claim_queued(&conn, 10).unwrap();
        assert_eq!(second.len(), 1);
        assert_ne!(second[0].id, first[0].id);
        // Nothing left queued.
        assert!(claim_queued(&conn, 10).unwrap().is_empty());
    }

    #[test]
    fn set_status_posted_stamps_time_and_failed_retries() {
        let conn = setup();
        let d = insert(&conn, "https://li/p/1", "hi");
        queue_all(&conn).unwrap();
        claim_queued(&conn, 1).unwrap();

        set_status(&conn, d.id, STATUS_FAILED, "network").unwrap();
        let failed = list(&conn).unwrap().pop().unwrap();
        assert_eq!(failed.status, STATUS_FAILED);
        assert_eq!(failed.error, "network");
        assert!(failed.posted_at.is_none());

        // A failed row is retryable — queue_all picks it back up.
        assert_eq!(queue_all(&conn).unwrap(), 1);
        claim_queued(&conn, 1).unwrap();
        set_status(&conn, d.id, STATUS_POSTED, "").unwrap();
        let posted = list(&conn).unwrap().pop().unwrap();
        assert_eq!(posted.status, STATUS_POSTED);
        assert_eq!(posted.error, "");
        assert!(posted.posted_at.is_some());
    }

    #[test]
    fn posting_records_ledger_and_survives_delete() {
        let conn = setup();
        let d = insert(&conn, "https://li/p/1", "hi");
        queue_all(&conn).unwrap();
        claim_queued(&conn, 1).unwrap();
        set_status(&conn, d.id, STATUS_POSTED, "").unwrap();

        // The permalink is now in the durable ledger…
        assert!(already_commented(&conn, "https://li/p/1").unwrap());
        // …so even after the draft row is deleted, the post still reads as handled
        // (a re-scrape won't re-surface it and comment a second time).
        assert_eq!(delete(&conn, d.id).unwrap(), 1);
        assert!(exists(&conn, "https://li/p/1").unwrap(), "ledger keeps it handled after delete");
        assert!(!exists(&conn, "https://li/p/2").unwrap());
    }

    #[test]
    fn claim_skips_already_commented_permalink() {
        let conn = setup();
        // A draft is queued, but its post is already in the ledger (e.g. a prior
        // attempt confirmed as posted out-of-band). It must never be claimed again.
        let d = insert(&conn, "https://li/p/1", "again?");
        conn.execute(
            "INSERT INTO commented_posts (permalink) VALUES ('https://li/p/1')",
            [],
        )
        .unwrap();
        assert_eq!(queue_all(&conn).unwrap(), 1, "queue itself doesn't consult the ledger");
        assert!(
            claim_queued(&conn, 10).unwrap().is_empty(),
            "but the claim excludes ledgered permalinks — no double-post"
        );
        // The row is left queued (not posting); it never goes out.
        let row = list(&conn).unwrap().into_iter().find(|r| r.id == d.id).unwrap();
        assert_eq!(row.status, STATUS_QUEUED);
    }

    #[test]
    fn ledger_insert_is_idempotent_on_repost() {
        let conn = setup();
        let d = insert(&conn, "https://li/p/1", "hi");
        queue_all(&conn).unwrap();
        claim_queued(&conn, 1).unwrap();
        // Two posted reports for the same row (a duplicate status POST) must not
        // error on the ledger's UNIQUE(permalink).
        set_status(&conn, d.id, STATUS_POSTED, "").unwrap();
        set_status(&conn, d.id, STATUS_POSTED, "").unwrap();
        let n: i64 = conn
            .query_row("SELECT count(*) FROM commented_posts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn run_request_and_claim_roundtrip() {
        let conn = setup();
        // Seeded idle by the migration.
        assert_eq!(get_run(&conn).unwrap().status, "idle");

        request_scrape(&conn, 10, false).unwrap();
        let run = get_run(&conn).unwrap();
        assert_eq!(run.status, "requested");
        assert_eq!(run.count, 10);
        assert!(!run.include_watchlist);

        // The worker claims it exactly once.
        let claimed = take_requested(&conn).unwrap().unwrap();
        assert_eq!(claimed.count, 10);
        assert_eq!(get_run(&conn).unwrap().status, "scraping");
        assert!(take_requested(&conn).unwrap().is_none(), "no second claim");

        assert_eq!(finish_scrape(&conn).unwrap(), 1);
        assert_eq!(get_run(&conn).unwrap().status, "idle");
    }

    #[test]
    fn request_scrape_forces_over_a_stuck_scraping_run() {
        let conn = setup();
        request_scrape(&conn, 5, true).unwrap();
        take_requested(&conn).unwrap(); // now "scraping" (simulate a wedged run)
        // A fresh request must override it rather than be ignored.
        request_scrape(&conn, 20, true).unwrap();
        assert_eq!(get_run(&conn).unwrap().status, "requested");
        assert_eq!(get_run(&conn).unwrap().count, 20);
    }

    #[test]
    fn finish_scrape_wont_clobber_a_fresh_request() {
        let conn = setup();
        // A run is in flight (scraping)…
        request_scrape(&conn, 10, true).unwrap();
        take_requested(&conn).unwrap();
        // …and the user asks for another while it runs.
        request_scrape(&conn, 20, false).unwrap();
        // The finishing worker's release must NOT drop the new request to idle.
        assert_eq!(finish_scrape(&conn).unwrap(), 0, "no scraping row to finish");
        let run = get_run(&conn).unwrap();
        assert_eq!(run.status, "requested", "the fresh request survives");
        assert_eq!(run.count, 20);
        // The new run is then claimable as normal.
        assert!(take_requested(&conn).unwrap().is_some());
    }
}
