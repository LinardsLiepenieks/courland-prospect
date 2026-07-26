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

const COLUMNS: &str = "id, name, kind, position, color, created_at";

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
        "UPDATE prospects SET stage_id = ?1 WHERE stage_id = ?2",
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
}
