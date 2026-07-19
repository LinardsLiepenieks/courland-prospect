//! All SQL for captured LinkedIn messages — both directions. Functions take
//! `&Connection` (a `&Transaction` derefs to it, so the caller can batch a whole
//! POST atomically) and stay free of Tauri types so they're unit-testable
//! against an in-memory DB.
//!
//! This slice is currently **write-only**: rows arrive from the Chrome extension
//! over the loopback ingest server (`crate::ingest`), never through a Tauri
//! command — which is why these are `pub(crate)`. This module is the single
//! centralized writer of extension-derived prospect state: a prospect's
//! `messages_sent` counter and its dynamic `awaiting_reply` flag are both derived
//! here (recomputed from the stored rows) rather than set by hand anywhere else.

use std::collections::HashSet;

use rusqlite::{params, Connection, OptionalExtension};

/// The captured direction of a message. Outgoing = you messaged the prospect
/// (drives `messages_sent`); incoming = the prospect replied. The two together,
/// ordered by capture, drive `awaiting_reply` (whether the newest message is
/// incoming). Anything not exactly `"incoming"` is normalized to outgoing when
/// stored, so a missing/garbled value fails safe to an outgoing message.
pub(crate) const INCOMING: &str = "incoming";
pub(crate) const OUTGOING: &str = "outgoing";

/// One captured message ready to store — borrowed, already-trimmed transport
/// input mapped to plain fields so the repository stays free of serde/Tauri types.
pub(crate) struct CapturedMessage<'a> {
    pub linkedin_url: &'a str,
    pub li_key: &'a str,
    pub body: &'a str,
    pub sent_at: Option<&'a str>,
    /// `"outgoing"` or `"incoming"`; normalized in `store`.
    pub direction: &'a str,
}

/// A genuinely new (first-seen) outgoing message, surfaced by [`store_batch`] for
/// downstream snippet proposal. Excludes re-scrapes/replays (deduped away) and
/// incoming replies — so the proposer only ever analyzes text the user actually
/// just sent for the first time.
pub(crate) struct NewOutgoing {
    pub prospect_id: i64,
    pub body: String,
}

/// The result of storing a batch: the counts the HTTP response reports, plus the
/// genuinely-new outgoing messages the caller fires snippet proposal on.
pub(crate) struct StoreOutcome {
    pub stored: usize,
    pub skipped: usize,
    pub new_outgoing: Vec<NewOutgoing>,
}

/// Store a whole captured batch and refresh the affected prospects' derived
/// state, in one call so the caller (the ingest route) just deserializes, trims,
/// and delegates.
///
/// The gate is uniform for both directions: a message is stored whenever its
/// person is already a prospect, at any stage. Both facts we derive need it —
/// an incoming reply is meaningful wherever they are, and we must record our
/// **outgoing** answers wherever they are too, or `awaiting_reply` could never
/// clear once a prospect advances past the messaging stage. Everything else
/// (blank identity, unknown person) is skipped — not an error. Derived state is
/// recomputed once per touched prospect, not per row. Idempotent via `store`'s
/// dedup, so replaying a batch never double-counts.
///
/// `new_outgoing` collects the first-seen outgoing messages (checked *before* the
/// upsert, so a replay reports none): the seam the snippet proposer hangs off of.
pub(crate) fn store_batch(
    conn: &Connection,
    items: &[CapturedMessage],
) -> rusqlite::Result<StoreOutcome> {
    let mut stored = 0usize;
    let mut skipped = 0usize;
    let mut new_outgoing = Vec::new();
    let mut affected = HashSet::new();

    for item in items {
        if item.linkedin_url.is_empty() || item.li_key.is_empty() {
            skipped += 1;
            continue;
        }
        match prospect_id_by_url(conn, item.linkedin_url)? {
            Some(pid) => {
                // Detect a first-seen message before the upsert (which can't tell
                // insert from update apart). Only fresh, non-blank OUTGOING text is
                // worth proposing snippets from.
                let is_new = !message_exists(conn, pid, item.li_key)?;
                store(conn, pid, item.li_key, item.body, item.sent_at, item.direction)?;
                if is_new && item.direction != INCOMING {
                    let body = item.body.trim();
                    if !body.is_empty() {
                        new_outgoing.push(NewOutgoing {
                            prospect_id: pid,
                            body: body.to_string(),
                        });
                    }
                }
                affected.insert(pid);
                stored += 1;
            }
            None => skipped += 1,
        }
    }

    for pid in &affected {
        recompute_derived(conn, *pid)?;
    }
    Ok(StoreOutcome { stored, skipped, new_outgoing })
}

/// Whether a message with this `(prospect_id, li_key)` is already stored — the
/// insert-vs-update discriminator `store`'s upsert can't provide on its own.
fn message_exists(conn: &Connection, prospect_id: i64, li_key: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT 1 FROM messages WHERE prospect_id = ?1 AND li_key = ?2",
        params![prospect_id, li_key],
        |_| Ok(()),
    )
    .optional()
    .map(|found| found.is_some())
}

/// The id of the prospect with this `linkedin_url`, or `None` if the person
/// isn't a prospect. The single storage gate for both directions: a message is
/// stored for any existing prospect, at any stage. A non-prospect resolves to
/// `None` and the message is silently ignored.
fn prospect_id_by_url(conn: &Connection, linkedin_url: &str) -> rusqlite::Result<Option<i64>> {
    conn.query_row(
        "SELECT id FROM prospects WHERE linkedin_url = ?1",
        [linkedin_url],
        |r| r.get(0),
    )
    .optional()
}

/// Upsert one captured message, deduped on `(prospect_id, li_key)`. Idempotent:
/// re-delivering the same message (offline-cache replay, thread re-scrape) never
/// duplicates — it just refreshes the body/timestamp. `direction` is normalized
/// so only the two canonical values ever hit the table.
///
/// `direction` is set on first insert and deliberately **not** overwritten on
/// conflict: a given `li_key` is one intrinsic message, so re-classification is
/// only ever noise from a mid-render re-scrape. Letting a later scrape flip the
/// stored direction would wrongly flip `awaiting_reply` (e.g. mark a reply as
/// answered) / skew `messages_sent` — the first capture (from a settled thread)
/// wins instead.
///
/// `body` and `sent_at` follow the same "a degraded re-scrape can't destroy a
/// good capture" rule: an empty body or a missing `sent_at` on conflict keeps
/// the stored value (LinkedIn doesn't always re-expose the timestamp, and a
/// mid-render scrape can catch an empty bubble). A *non-empty* body still wins,
/// so a genuine edit is not lost.
/// Does **not** touch the derived state; `store_batch` recomputes afterward.
fn store(
    conn: &Connection,
    prospect_id: i64,
    li_key: &str,
    body: &str,
    sent_at: Option<&str>,
    direction: &str,
) -> rusqlite::Result<()> {
    let direction = if direction == INCOMING { INCOMING } else { OUTGOING };
    conn.execute(
        "INSERT INTO messages (prospect_id, li_key, body, sent_at, direction)
              VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(prospect_id, li_key) DO UPDATE SET
              body = CASE WHEN excluded.body != '' THEN excluded.body ELSE body END,
              sent_at = COALESCE(excluded.sent_at, sent_at)",
        params![prospect_id, li_key, body, sent_at, direction],
    )?;
    Ok(())
}

/// Recompute a prospect's derived state from its stored messages — the single
/// source of truth for both facts. `messages_sent` counts outgoing messages;
/// `awaiting_reply` is whether the prospect's **newest** message is incoming
/// (they replied and we haven't answered). "Newest" is the greatest `id`:
/// insertion order tracks the extension's top-to-bottom (chronological) scrape,
/// and later captures always insert higher. `COALESCE` keeps a prospect with no
/// messages at 0. Called once per affected prospect after storing a batch.
fn recompute_derived(conn: &Connection, prospect_id: i64) -> rusqlite::Result<()> {
    // Direction literals come from the module consts so this query can't drift
    // from `store`'s vocabulary.
    let sql = format!(
        "UPDATE prospects SET
             messages_sent =
                 (SELECT count(*) FROM messages
                  WHERE prospect_id = ?1 AND direction = '{OUTGOING}'),
             awaiting_reply = COALESCE(
                 (SELECT direction = '{INCOMING}' FROM messages
                  WHERE prospect_id = ?1
                  ORDER BY id DESC LIMIT 1),
                 0)
         WHERE id = ?1"
    );
    conn.execute(&sql, [prospect_id])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::migrations;
    use crate::features::stages;

    fn setup() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        migrations::run(&mut conn).unwrap();
        conn
    }

    /// Seed a pitch with a full-cycle pipeline; return (pitch_id, ordered stage ids).
    fn seed_pitch_with_stages(conn: &Connection) -> (i64, Vec<i64>) {
        conn.execute("INSERT INTO pitches (name, skill) VALUES ('P', '')", [])
            .unwrap();
        let pitch = conn.last_insert_rowid();
        let created =
            stages::repository::create_many(conn, pitch, &stages::repository::full_cycle_template())
                .unwrap();
        (pitch, created.into_iter().map(|s| s.id).collect())
    }

    /// Insert a prospect directly on a given stage; return its id.
    fn seed_prospect(conn: &Connection, url: &str, pitch: i64, stage: i64) -> i64 {
        conn.execute(
            "INSERT INTO prospects (name, linkedin_url, pitch_id, stage_id) VALUES ('N', ?1, ?2, ?3)",
            params![url, pitch, stage],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn messages_sent(conn: &Connection, prospect_id: i64) -> i64 {
        conn.query_row(
            "SELECT messages_sent FROM prospects WHERE id = ?1",
            [prospect_id],
            |r| r.get(0),
        )
        .unwrap()
    }

    fn awaiting_reply(conn: &Connection, prospect_id: i64) -> bool {
        conn.query_row(
            "SELECT awaiting_reply FROM prospects WHERE id = ?1",
            [prospect_id],
            |r| r.get(0),
        )
        .unwrap()
    }

    /// Convenience for the tests: build an outgoing captured message.
    fn out<'a>(url: &'a str, key: &'a str, body: &'a str) -> CapturedMessage<'a> {
        CapturedMessage { linkedin_url: url, li_key: key, body, sent_at: None, direction: OUTGOING }
    }

    /// Convenience for the tests: build an incoming (reply) captured message.
    fn incoming<'a>(url: &'a str, key: &'a str, body: &'a str) -> CapturedMessage<'a> {
        CapturedMessage { linkedin_url: url, li_key: key, body, sent_at: None, direction: INCOMING }
    }

    #[test]
    fn resolves_any_prospect_regardless_of_stage() {
        let conn = setup();
        let (pitch, stages) = seed_pitch_with_stages(&conn);
        let url = "https://li/ada";
        let id = seed_prospect(&conn, url, pitch, stages[0]); // messaging stage

        assert_eq!(prospect_id_by_url(&conn, url).unwrap(), Some(id));

        // Advancing past messaging must NOT change resolution — the storage gate
        // is uniform now, so our answers keep landing wherever the prospect sits.
        conn.execute(
            "UPDATE prospects SET stage_id = ?1 WHERE id = ?2",
            params![stages[1], id],
        )
        .unwrap();
        assert_eq!(prospect_id_by_url(&conn, url).unwrap(), Some(id));

        // Unknown URL → None (a non-prospect's messages are dropped).
        assert_eq!(prospect_id_by_url(&conn, "https://li/nobody").unwrap(), None);
    }

    #[test]
    fn store_dedups_and_recompute_counts() {
        let conn = setup();
        let (pitch, stages) = seed_pitch_with_stages(&conn);
        let id = seed_prospect(&conn, "https://li/ada", pitch, stages[0]);

        store(&conn, id, "k1", "hello", Some("2026-07-12"), OUTGOING).unwrap();
        store(&conn, id, "k2", "again", None, OUTGOING).unwrap();
        // Re-deliver k1 (replay) with a refreshed body — must not duplicate.
        store(&conn, id, "k1", "hello (edited)", Some("2026-07-12"), OUTGOING).unwrap();

        recompute_derived(&conn, id).unwrap();
        assert_eq!(messages_sent(&conn, id), 2);

        let body: String = conn
            .query_row(
                "SELECT body FROM messages WHERE prospect_id = ?1 AND li_key = 'k1'",
                [id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(body, "hello (edited)");
    }

    #[test]
    fn store_batch_counts_and_skips_across_a_mixed_batch() {
        let conn = setup();
        let (pitch, stages) = seed_pitch_with_stages(&conn);
        let in_msg = seed_prospect(&conn, "https://li/ada", pitch, stages[0]); // messaging
        let advanced = seed_prospect(&conn, "https://li/grace", pitch, stages[1]); // past messaging

        let batch = [
            out("https://li/ada", "k1", "one"),
            out("https://li/ada", "k2", "two"),
            // Advanced prospect → stored now (the storage gate is stage-agnostic).
            out("https://li/grace", "k3", "x"),
            // Unknown person → skipped.
            out("https://li/nobody", "k4", "y"),
            // Blank identity → skipped.
            out("", "k5", "z"),
        ];

        let out = store_batch(&conn, &batch).unwrap();
        assert_eq!((out.stored, out.skipped), (3, 2));
        assert_eq!(messages_sent(&conn, in_msg), 2);
        assert_eq!(messages_sent(&conn, advanced), 1);
        // All three stored messages are first-seen outgoing → all surface.
        assert_eq!(out.new_outgoing.len(), 3);

        // Replaying the identical batch double-counts nothing (idempotent) and
        // surfaces NO new outgoing — every message already exists.
        let out2 = store_batch(&conn, &batch).unwrap();
        assert_eq!(out2.stored, 3);
        assert!(out2.new_outgoing.is_empty(), "a replay proposes nothing new");
        assert_eq!(messages_sent(&conn, in_msg), 2);
        assert_eq!(messages_sent(&conn, advanced), 1);
    }

    #[test]
    fn incoming_reply_sets_awaiting_reply_at_any_stage() {
        let conn = setup();
        let (pitch, stages) = seed_pitch_with_stages(&conn);
        // A prospect who has been advanced past the messaging stage.
        let advanced = seed_prospect(&conn, "https://li/ada", pitch, stages[2]);

        let out = store_batch(
            &conn,
            &[
                // Their reply — sets awaiting_reply even though they're not in messaging.
                incoming("https://li/ada", "r1", "sounds great"),
                // A reply from a non-prospect → skipped.
                incoming("https://li/nobody", "r2", "who?"),
            ],
        )
        .unwrap();
        assert_eq!((out.stored, out.skipped), (1, 1));
        // An incoming reply is never proposal material.
        assert!(out.new_outgoing.is_empty(), "incoming replies don't surface as new outgoing");
        assert!(awaiting_reply(&conn, advanced));
        // An incoming reply is not an outgoing message, so the count stays put.
        assert_eq!(messages_sent(&conn, advanced), 0);
    }

    #[test]
    fn awaiting_reply_clears_when_we_answer_and_toggles_back_on_the_next_reply() {
        let conn = setup();
        let (pitch, stages) = seed_pitch_with_stages(&conn);
        // Advanced past messaging — the case the old messaging-stage gate broke:
        // our answers must still be recorded so the flag can clear.
        let id = seed_prospect(&conn, "https://li/ada", pitch, stages[2]);

        // We message first — newest is ours → not awaiting.
        store_batch(&conn, &[out("https://li/ada", "o1", "hi there")]).unwrap();
        assert!(!awaiting_reply(&conn, id));

        // They reply — newest is theirs → awaiting.
        store_batch(&conn, &[incoming("https://li/ada", "r1", "sounds good")]).unwrap();
        assert!(awaiting_reply(&conn, id));

        // We answer — newest is ours again → clears.
        store_batch(&conn, &[out("https://li/ada", "o2", "great, let's talk")]).unwrap();
        assert!(!awaiting_reply(&conn, id), "answering must clear awaiting_reply");
        assert_eq!(messages_sent(&conn, id), 2, "both our messages count regardless of stage");

        // They reply again — toggles back on.
        store_batch(&conn, &[incoming("https://li/ada", "r2", "perfect")]).unwrap();
        assert!(awaiting_reply(&conn, id), "a fresh reply re-sets awaiting_reply");
    }

    #[test]
    fn rescrape_misclassifying_a_reply_cannot_unset_awaiting_reply() {
        let conn = setup();
        let (pitch, stages) = seed_pitch_with_stages(&conn);
        let id = seed_prospect(&conn, "https://li/ada", pitch, stages[0]);

        // Their reply lands as incoming under a stable key and is the newest.
        store(&conn, id, "urn:msg:1", "yes let's talk", None, INCOMING).unwrap();
        recompute_derived(&conn, id).unwrap();
        assert!(awaiting_reply(&conn, id));

        // A degraded re-scrape re-delivers the SAME key classified outgoing.
        // Direction is fixed at first insert, so the newest message stays incoming
        // — awaiting_reply must NOT wrongly clear, and the reply isn't counted as
        // one of our outgoing messages.
        store(&conn, id, "urn:msg:1", "yes let's talk", None, OUTGOING).unwrap();
        recompute_derived(&conn, id).unwrap();
        assert!(awaiting_reply(&conn, id), "a re-scrape must not wrongly clear awaiting_reply");
        assert_eq!(messages_sent(&conn, id), 0, "the reply must not be recounted as outgoing");
    }

    #[test]
    fn degraded_rescrape_cannot_clobber_a_good_body_or_timestamp() {
        let conn = setup();
        let (pitch, stages) = seed_pitch_with_stages(&conn);
        let id = seed_prospect(&conn, "https://li/ada", pitch, stages[0]);

        // A settled capture with a real body + timestamp.
        store(&conn, id, "k1", "let's chat next week", Some("2026-07-12"), OUTGOING).unwrap();

        // A degraded re-scrape re-delivers the SAME key with an empty body and no
        // timestamp (LinkedIn hadn't re-rendered them). Neither may overwrite.
        store(&conn, id, "k1", "", None, OUTGOING).unwrap();

        let (body, sent_at): (String, Option<String>) = conn
            .query_row(
                "SELECT body, sent_at FROM messages WHERE prospect_id = ?1 AND li_key = 'k1'",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(body, "let's chat next week", "empty re-scrape must not wipe the body");
        assert_eq!(sent_at.as_deref(), Some("2026-07-12"), "missing timestamp must not wipe it");

        // A genuine edit (non-empty body) still wins.
        store(&conn, id, "k1", "let's chat Thursday", None, OUTGOING).unwrap();
        let body: String = conn
            .query_row(
                "SELECT body FROM messages WHERE prospect_id = ?1 AND li_key = 'k1'",
                [id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(body, "let's chat Thursday", "a real edit must still update the body");
    }

    /// Invariant guard. A message resolves to its prospect purely by *exact*
    /// `linkedin_url` equality — the backend does no canonicalization. The whole
    /// capture pipeline is safe only because the Chrome extension is the sole
    /// producer of that string, running both the prospect URL and every message's
    /// URL through one `normalizeProfileUrl` (canonical form
    /// `https://www.linkedin.com/in/<slug>/`). This test pins that contract: the
    /// canonical form resolves, and near-miss variants (no trailing slash, a
    /// `?miniProfileUrn=` query, a differently-cased host) do NOT. If someone
    /// later adds backend canonicalization, these expectations must change
    /// deliberately — a silent divergence here means captured messages are
    /// dropped (`skipped`) and, because the extension clears its outbox on 2xx,
    /// permanently lost.
    #[test]
    fn resolution_requires_the_exact_canonical_url() {
        let conn = setup();
        let (pitch, stages) = seed_pitch_with_stages(&conn);
        let canonical = "https://www.linkedin.com/in/ada-lovelace/";
        let id = seed_prospect(&conn, canonical, pitch, stages[0]); // messaging stage

        // Exact canonical form resolves via the single storage-gate lookup.
        assert_eq!(prospect_id_by_url(&conn, canonical).unwrap(), Some(id));

        // Near-miss variants of the same profile must NOT resolve — the backend
        // has no safety net, so the extension must send the canonical form.
        for variant in [
            "https://www.linkedin.com/in/ada-lovelace",          // no trailing slash
            "https://www.linkedin.com/in/ada-lovelace/?miniProfileUrn=x", // query kept
            "https://WWW.linkedin.com/in/ada-lovelace/",         // host case
        ] {
            assert_eq!(
                prospect_id_by_url(&conn, variant).unwrap(),
                None,
                "variant unexpectedly resolved: {variant}",
            );
        }
    }

    #[test]
    fn deleting_prospect_cascades_messages() {
        let conn = setup();
        let (pitch, stages) = seed_pitch_with_stages(&conn);
        let id = seed_prospect(&conn, "https://li/ada", pitch, stages[0]);
        store(&conn, id, "k1", "hi", None, OUTGOING).unwrap();

        conn.execute("DELETE FROM prospects WHERE id = ?1", [id]).unwrap();
        let remaining: i64 = conn
            .query_row("SELECT count(*) FROM messages", [], |r| r.get(0))
            .unwrap();
        assert_eq!(remaining, 0);
    }
}
