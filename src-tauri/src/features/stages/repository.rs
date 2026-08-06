//! All SQL for the pipeline. Functions take `&Connection` (so a caller can pass
//! a `&Transaction`, which derefs to it) and return domain types; the command
//! layer owns connection locking, validation, and transaction scope. Kept free
//! of Tauri types so it stays unit-testable against an in-memory DB.
//!
//! There is exactly one pipeline, so nothing here takes an owner. Nothing here
//! creates a pipeline either: migration 0024 collapsed the old per-pitch funnels
//! into this one and seeds the Full-cycle template on a fresh database, so a
//! pipeline always exists by the time any of this runs.

use rusqlite::{params, Connection, OptionalExtension};

use super::model::{color_for_position, Stage, KIND_STANDARD};

const COLUMNS: &str =
    "id, name, kind, position, color, goal, warn_days, stale_days, created_at";

/// The pipeline, in funnel order.
pub(crate) fn list(conn: &Connection) -> rusqlite::Result<Vec<Stage>> {
    let sql = format!("SELECT {COLUMNS} FROM stages ORDER BY position, id");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], Stage::from_row)?;
    rows.collect()
}

/// Fetch a single stage by id (used internally and by the command layer's
/// delete guard). Returns `None` if no stage has that id.
pub(super) fn get(conn: &Connection, id: i64) -> rusqlite::Result<Option<Stage>> {
    let sql = format!("SELECT {COLUMNS} FROM stages WHERE id = ?1");
    conn.query_row(&sql, [id], Stage::from_row).optional()
}

/// Append a standard stage to the end of the pipeline, colored by its position
/// (rotating the palette) so it lands with a distinct default.
pub(super) fn append(conn: &Connection, name: &str) -> rusqlite::Result<Stage> {
    let next: i64 = conn.query_row(
        "SELECT COALESCE(MAX(position), -1) + 1 FROM stages",
        [],
        |r| r.get(0),
    )?;
    conn.execute(
        "INSERT INTO stages (name, kind, position, color) VALUES (?1, ?2, ?3, ?4)",
        params![name, KIND_STANDARD, next, color_for_position(next)],
    )?;
    // The row was just inserted, so get() should never be None. Surface a
    // missing row as a query error rather than panicking: this runs while the
    // command layer holds the connection Mutex, and a panic here would poison
    // that lock and brick every subsequent DB call for the rest of the session.
    get(conn, conn.last_insert_rowid())?.ok_or(rusqlite::Error::QueryReturnedNoRows)
}

pub(super) fn rename(conn: &Connection, id: i64, name: &str) -> rusqlite::Result<Option<Stage>> {
    let changed = conn.execute(
        "UPDATE stages SET name = ?1 WHERE id = ?2",
        params![name, id],
    )?;
    if changed == 0 {
        return Ok(None);
    }
    get(conn, id)
}

/// Set a stage's color (already validated as a known palette token by the
/// caller). Returns the updated stage, or `None` if no stage had that id.
pub(super) fn set_color(
    conn: &Connection,
    id: i64,
    color: &str,
) -> rusqlite::Result<Option<Stage>> {
    let changed = conn.execute(
        "UPDATE stages SET color = ?1 WHERE id = ?2",
        params![color, id],
    )?;
    if changed == 0 {
        return Ok(None);
    }
    get(conn, id)
}

/// Set a stage's goal — what this step of the cycle is for. Already trimmed and
/// length-checked by the caller. Returns the updated stage, or `None` if no
/// stage had that id.
pub(super) fn set_goal(conn: &Connection, id: i64, goal: &str) -> rusqlite::Result<Option<Stage>> {
    let changed = conn.execute(
        "UPDATE stages SET goal = ?1 WHERE id = ?2",
        params![goal, id],
    )?;
    if changed == 0 {
        return Ok(None);
    }
    get(conn, id)
}

/// Set a stage's staleness thresholds (already validated as an in-range,
/// correctly-ordered pair by the caller). Returns the updated stage, or `None`
/// if no stage had that id.
pub(super) fn set_thresholds(
    conn: &Connection,
    id: i64,
    warn_days: i64,
    stale_days: i64,
) -> rusqlite::Result<Option<Stage>> {
    let changed = conn.execute(
        "UPDATE stages SET warn_days = ?1, stale_days = ?2 WHERE id = ?3",
        params![warn_days, stale_days, id],
    )?;
    if changed == 0 {
        return Ok(None);
    }
    get(conn, id)
}

/// The stage immediately AFTER `position` in the funnel — the one an advance
/// suggestion would point at. `None` when the prospect is already in the last
/// stage, which is how the analyzer knows there's nowhere left to go.
///
/// Mirrors [`previous_id`]'s nearest-neighbour form rather than `position + 1`,
/// because positions are not guaranteed contiguous: a stage delete shifts its
/// siblings down, but nothing renumbers after a partial failure, so the pipeline
/// can legitimately read 0, 2, 3.
pub(crate) fn next_stage(conn: &Connection, position: i64) -> rusqlite::Result<Option<Stage>> {
    let sql = format!(
        "SELECT {COLUMNS} FROM stages WHERE position > ?1 ORDER BY position, id LIMIT 1"
    );
    conn.query_row(&sql, [position], Stage::from_row).optional()
}

/// Fetch one stage by id, for callers outside this feature (the advance analyzer
/// needs the prospect's current stage and its goal). Read-only.
pub(crate) fn find(conn: &Connection, id: i64) -> rusqlite::Result<Option<Stage>> {
    get(conn, id)
}

pub(super) fn count(conn: &Connection) -> rusqlite::Result<i64> {
    conn.query_row("SELECT count(*) FROM stages", [], |r| r.get(0))
}

/// The id of the stage immediately before `position` in the pipeline (the
/// greatest position less than it), if any.
pub(super) fn previous_id(conn: &Connection, position: i64) -> rusqlite::Result<Option<i64>> {
    conn.query_row(
        "SELECT id FROM stages WHERE position < ?1 ORDER BY position DESC, id DESC LIMIT 1",
        [position],
        |r| r.get(0),
    )
    .optional()
}

/// Move every prospect in `from_stage` to `to_stage`, then delete `from_stage`.
/// The caller wraps this in a transaction so the reassignment and delete commit
/// together (a prospect is never stranded on a deleted stage).
///
/// The reassignment also clears any pending advance suggestion, upholding the
/// invariant `prospects::repository::set_stage` documents: **any** move
/// invalidates a verdict reached about where they used to be. This has to be
/// restated here because the UPDATE below moves prospects without going through
/// `set_stage` — and the `ON DELETE SET NULL` on `suggested_stage_id` doesn't
/// cover it either, since that only retracts suggestions pointing AT the deleted
/// stage, not ones held by prospects who were standing IN it.
///
/// Left unhandled it doesn't just leave a stale chip: the reassigned prospect
/// lands one step earlier, which usually makes their old suggestion target their
/// NEW next stage — and `advance::gather` skips a prospect whose suggestion
/// already points there, so the analyzer would never look at them again.
///
/// Closing the hole the delete leaves is part of the same operation. `position`
/// is meant to read as "step N of the funnel", and every other writer assumes
/// that: `append` takes `MAX(position) + 1`, `color_for_position` indexes the
/// palette by it, and a migration remapping one pipeline onto another matches on
/// it. Leaving 0, 2, 3 behind makes each of those quietly wrong, and nothing
/// else repairs it — `reorder` only runs when the user drags a stage by hand.
pub(super) fn reassign_and_delete(
    conn: &Connection,
    from_stage: i64,
    to_stage: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE prospects
         SET stage_id = ?1, suggested_stage_id = NULL, suggested_reason = ''
         WHERE stage_id = ?2",
        params![to_stage, from_stage],
    )?;
    // Read the doomed stage's position before the row is gone — the shift below
    // needs to know where the hole will be.
    let gap: Option<i64> = conn
        .query_row(
            "SELECT position FROM stages WHERE id = ?1",
            [from_stage],
            |r| r.get(0),
        )
        .optional()?;
    conn.execute("DELETE FROM stages WHERE id = ?1", [from_stage])?;
    if let Some(gap) = gap {
        // Close the hole: every stage after it shifts down one step.
        conn.execute(
            "UPDATE stages SET position = position - 1 WHERE position > ?1",
            [gap],
        )?;
    }
    Ok(())
}

/// Set each stage's position to its index in `ordered_ids`. The caller validates
/// that the ids are exactly the pipeline's stages and that the messaging stage
/// stays first; this just writes the positions (in a transaction).
pub(super) fn reorder(conn: &Connection, ordered_ids: &[i64]) -> rusqlite::Result<()> {
    for (position, id) in ordered_ids.iter().enumerate() {
        conn.execute(
            "UPDATE stages SET position = ?1 WHERE id = ?2",
            params![position as i64, id],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::migrations;
    use crate::features::stages::model::KIND_MESSAGING;

    /// A fresh database already has a pipeline: 0024 seeds the Full-cycle
    /// template when there's no pitch to inherit one from. Tests build on that
    /// rather than creating a pipeline, because nothing in the app does either.
    fn setup() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        migrations::run(&mut conn).unwrap();
        conn
    }

    #[test]
    fn a_fresh_database_comes_up_with_the_full_cycle_pipeline() {
        let conn = setup();
        let stages = list(&conn).unwrap();
        let names: Vec<&str> = stages.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Messaged", "Meeting", "Onboarding", "Feedback"]);
        assert_eq!(stages[0].kind, KIND_MESSAGING);
        assert_eq!(stages[0].position, 0);
        assert_eq!(stages[0].color, "blue");
        assert_eq!(stages[3].position, 3);
        assert_eq!(stages[3].color, "purple");
    }

    #[test]
    fn append_adds_standard_stage_at_end() {
        let conn = setup();
        let added = append(&conn, "Negotiation").unwrap();
        assert_eq!(added.kind, KIND_STANDARD);
        assert_eq!(added.position, 4);
        assert_eq!(added.color, "teal"); // position 4 in the palette rotation
        assert_eq!(list(&conn).unwrap().len(), 5);
        assert_eq!(count(&conn).unwrap(), 5);
    }

    #[test]
    fn set_color_updates_and_missing_returns_none() {
        let conn = setup();
        let stages = list(&conn).unwrap();
        let updated = set_color(&conn, stages[1].id, "red").unwrap().unwrap();
        assert_eq!(updated.color, "red");
        assert!(set_color(&conn, 9999, "red").unwrap().is_none());
    }

    #[test]
    fn rename_updates_and_missing_returns_none() {
        let conn = setup();
        let stages = list(&conn).unwrap();
        let renamed = rename(&conn, stages[1].id, "Call").unwrap().unwrap();
        assert_eq!(renamed.name, "Call");
        assert!(rename(&conn, 9999, "x").unwrap().is_none());
    }

    #[test]
    fn reassign_and_delete_moves_prospects_and_removes_stage() {
        let conn = setup();
        let stages = list(&conn).unwrap();
        let (messaged, meeting) = (stages[0].id, stages[1].id);
        conn.execute(
            "INSERT INTO prospects (name, linkedin_url, stage_id) VALUES ('A', 'u', ?1)",
            [meeting],
        )
        .unwrap();

        let prev = previous_id(&conn, stages[1].position).unwrap().unwrap();
        assert_eq!(prev, messaged);

        reassign_and_delete(&conn, meeting, prev).unwrap();
        assert_eq!(list(&conn).unwrap().len(), 3);
        let moved: i64 = conn
            .query_row("SELECT stage_id FROM prospects WHERE name = 'A'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(moved, messaged);
    }

    /// Regression. Deleting a stage moves its prospects with a raw UPDATE rather
    /// than through `set_stage`, so it has to clear their suggestions itself.
    /// Leaving one behind is worse than a stale chip: the reassigned prospect
    /// lands one step earlier, their old target becomes their new NEXT stage, and
    /// `advance::gather`'s "already suggesting this" short-circuit then skips them
    /// on every future message — permanently.
    #[test]
    fn reassign_and_delete_clears_the_suggestions_of_the_prospects_it_moves() {
        let conn = setup();
        let stages = list(&conn).unwrap();
        let (messaged, meeting, onboarding) = (stages[0].id, stages[1].id, stages[2].id);

        // Ada stands in Meeting with a pending suggestion for Onboarding.
        conn.execute(
            "INSERT INTO prospects (name, linkedin_url, stage_id, suggested_stage_id, suggested_reason)
             VALUES ('Ada', 'u', ?1, ?2, 'they confirmed Thursday')",
            params![meeting, onboarding],
        )
        .unwrap();

        reassign_and_delete(&conn, meeting, messaged).unwrap();

        let (stage_id, suggested, reason): (i64, Option<i64>, String) = conn
            .query_row(
                "SELECT stage_id, suggested_stage_id, suggested_reason FROM prospects WHERE name = 'Ada'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(stage_id, messaged, "she is moved back a step");
        assert_eq!(
            suggested, None,
            "the verdict was about a stage she never reached, and Onboarding is now \
             her next stage — leaving it would freeze her out of every future check"
        );
        assert_eq!(reason, "");
    }

    /// The FK's `ON DELETE SET NULL` covers the other direction — a suggestion
    /// pointing at the doomed stage, held by a prospect standing somewhere else.
    /// Pinned here beside its sibling so the two cases stay visibly distinct.
    #[test]
    fn deleting_a_stage_also_retracts_suggestions_that_pointed_at_it() {
        let conn = setup();
        let stages = list(&conn).unwrap();
        let (messaged, meeting) = (stages[0].id, stages[1].id);

        conn.execute(
            "INSERT INTO prospects (name, linkedin_url, stage_id, suggested_stage_id, suggested_reason)
             VALUES ('Grace', 'u2', ?1, ?2, 'ready to meet')",
            params![messaged, meeting],
        )
        .unwrap();

        reassign_and_delete(&conn, meeting, messaged).unwrap();

        let suggested: Option<i64> = conn
            .query_row("SELECT suggested_stage_id FROM prospects WHERE name = 'Grace'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(suggested, None, "no card may offer a move into a deleted column");
    }

    #[test]
    fn reorder_writes_positions_by_index() {
        let conn = setup();
        let stages = list(&conn).unwrap();
        // Keep messaging first; swap the last two.
        let ids = [stages[0].id, stages[1].id, stages[3].id, stages[2].id];
        reorder(&conn, &ids).unwrap();
        let after = list(&conn).unwrap();
        assert_eq!(after[2].name, "Feedback");
        assert_eq!(after[3].name, "Onboarding");
    }

    #[test]
    fn previous_id_is_none_before_the_first_stage() {
        let conn = setup();
        assert!(previous_id(&conn, 0).unwrap().is_none());
    }

    #[test]
    fn set_goal_updates_and_missing_returns_none() {
        let conn = setup();
        let stages = list(&conn).unwrap();
        let updated = set_goal(&conn, stages[0].id, "Get a reply.").unwrap().unwrap();
        assert_eq!(updated.goal, "Get a reply.");
        // Clearing it back to empty is valid — a stage with no goal simply
        // doesn't steer drafts and is never advanced out of.
        assert_eq!(set_goal(&conn, stages[0].id, "").unwrap().unwrap().goal, "");
        assert!(set_goal(&conn, 9999, "x").unwrap().is_none());
    }

    #[test]
    fn set_thresholds_updates_and_missing_returns_none() {
        let conn = setup();
        let stages = list(&conn).unwrap();
        // Fresh stages come up on the built-in cadence (migration 0027's default).
        assert_eq!((stages[0].warn_days, stages[0].stale_days), (3, 7));

        let updated = set_thresholds(&conn, stages[0].id, 2, 5).unwrap().unwrap();
        assert_eq!((updated.warn_days, updated.stale_days), (2, 5));
        assert!(set_thresholds(&conn, 9999, 2, 5).unwrap().is_none());
    }

    /// A newly appended stage inherits the built-in cadence and an empty goal, so
    /// adding a column to the board never demands two more decisions up front.
    #[test]
    fn an_appended_stage_starts_with_no_goal_and_the_default_cadence() {
        let conn = setup();
        let added = append(&conn, "Negotiation").unwrap();
        assert_eq!(added.goal, "");
        assert_eq!((added.warn_days, added.stale_days), (3, 7));
    }

    #[test]
    fn next_stage_walks_the_funnel_and_stops_at_the_end() {
        let conn = setup();
        let stages = list(&conn).unwrap();

        let after_first = next_stage(&conn, stages[0].position).unwrap().unwrap();
        assert_eq!(after_first.id, stages[1].id);
        // The last stage has nowhere to advance to — how the analyzer knows to
        // stop rather than proposing a move off the end of the board.
        assert!(next_stage(&conn, stages[3].position).unwrap().is_none());
    }

    /// Positions are not contiguous after a delete, so `next_stage` must find the
    /// nearest following stage rather than assuming `position + 1` exists.
    #[test]
    fn next_stage_skips_a_gap_left_by_a_deleted_stage() {
        let conn = setup();
        let stages = list(&conn).unwrap();
        // Punch a hole: leave positions 0, 2, 3 with nothing at 1.
        conn.execute("UPDATE stages SET position = 9 WHERE id = ?1", [stages[1].id])
            .unwrap();
        conn.execute("DELETE FROM stages WHERE id = ?1", [stages[1].id])
            .unwrap();

        let after_first = next_stage(&conn, stages[0].position).unwrap().unwrap();
        assert_eq!(after_first.id, stages[2].id, "the gap at position 1 is stepped over");
    }
}
