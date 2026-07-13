//! All SQL for pipeline stages. Functions take `&Connection` (so a caller can
//! pass a `&Transaction`, which derefs to it) and return domain types; the
//! command layer owns connection locking, validation, and transaction scope.
//! Kept free of Tauri types so it stays unit-testable against an in-memory DB.

use rusqlite::{params, Connection, OptionalExtension};

use super::model::{color_for_position, Stage, StageInput, KIND_MESSAGING, KIND_STANDARD};

const COLUMNS: &str = "id, pitch_id, name, kind, position, color, created_at";

/// The built-in "Full-cycle" template seeded into a new pitch's pipeline. The
/// first entry is the messaging stage; the rest are standard funnel steps. Colors
/// come from `color_for_position` (the palette rotation) so they stay in step
/// with it and never need hand-syncing. Frontend mirror: `fullCycleDraft` in
/// `src/pitches/StageEditor.tsx` (the names must match; colors derive the same way).
pub(crate) fn full_cycle_template() -> Vec<StageInput> {
    [
        ("Messaged", KIND_MESSAGING),
        ("Meeting", KIND_STANDARD),
        ("Onboarding", KIND_STANDARD),
        ("Feedback", KIND_STANDARD),
    ]
    .iter()
    .enumerate()
    .map(|(i, (name, kind))| StageInput {
        name: (*name).to_string(),
        kind: (*kind).to_string(),
        color: color_for_position(i as i64).to_string(),
    })
    .collect()
}

pub(crate) fn list_by_pitch(conn: &Connection, pitch_id: i64) -> rusqlite::Result<Vec<Stage>> {
    let sql = format!("SELECT {COLUMNS} FROM stages WHERE pitch_id = ?1 ORDER BY position, id");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([pitch_id], Stage::from_row)?;
    rows.collect()
}

/// Fetch a single stage by id (used internally and by the command layer's
/// delete guard). Returns `None` if no stage has that id.
pub(super) fn get(conn: &Connection, id: i64) -> rusqlite::Result<Option<Stage>> {
    let sql = format!("SELECT {COLUMNS} FROM stages WHERE id = ?1");
    conn.query_row(&sql, [id], Stage::from_row).optional()
}

/// Insert an ordered list of stages for a pitch, position = array index. Used to
/// seed a pipeline at pitch creation. Returns the created stages in order.
pub(crate) fn create_many(
    conn: &Connection,
    pitch_id: i64,
    stages: &[StageInput],
) -> rusqlite::Result<Vec<Stage>> {
    for (position, stage) in stages.iter().enumerate() {
        conn.execute(
            "INSERT INTO stages (pitch_id, name, kind, position, color) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![pitch_id, stage.name, stage.kind, position as i64, stage.color],
        )?;
    }
    list_by_pitch(conn, pitch_id)
}

/// Append a standard stage to the end of a pitch's pipeline, colored by its
/// position (rotating the palette) so it lands with a distinct default.
pub(super) fn append(conn: &Connection, pitch_id: i64, name: &str) -> rusqlite::Result<Stage> {
    let next: i64 = conn.query_row(
        "SELECT COALESCE(MAX(position), -1) + 1 FROM stages WHERE pitch_id = ?1",
        [pitch_id],
        |r| r.get(0),
    )?;
    conn.execute(
        "INSERT INTO stages (pitch_id, name, kind, position, color) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![pitch_id, name, KIND_STANDARD, next, color_for_position(next)],
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

pub(super) fn count_for_pitch(conn: &Connection, pitch_id: i64) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT count(*) FROM stages WHERE pitch_id = ?1",
        [pitch_id],
        |r| r.get(0),
    )
}

/// The id of the stage immediately before `position` in the pitch's pipeline
/// (the greatest position less than it), if any.
pub(super) fn previous_id(
    conn: &Connection,
    pitch_id: i64,
    position: i64,
) -> rusqlite::Result<Option<i64>> {
    conn.query_row(
        "SELECT id FROM stages WHERE pitch_id = ?1 AND position < ?2 ORDER BY position DESC, id DESC LIMIT 1",
        params![pitch_id, position],
        |r| r.get(0),
    )
    .optional()
}

/// Move every prospect in `from_stage` to `to_stage`, then delete `from_stage`.
/// The caller wraps this in a transaction so the reassignment and delete commit
/// together (a prospect is never stranded on a deleted stage).
pub(super) fn reassign_and_delete(
    conn: &Connection,
    from_stage: i64,
    to_stage: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE prospects SET stage_id = ?1 WHERE stage_id = ?2",
        params![to_stage, from_stage],
    )?;
    conn.execute("DELETE FROM stages WHERE id = ?1", [from_stage])?;
    Ok(())
}

/// Set each stage's position to its index in `ordered_ids`. The caller validates
/// that the ids are exactly the pitch's stages and that the messaging stage
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

    #[test]
    fn create_many_seeds_ordered_pipeline() {
        let conn = setup();
        let pitch = seed_pitch(&conn, "P");
        let stages = create_many(&conn, pitch, &full_cycle_template()).unwrap();
        assert_eq!(stages.len(), 4);
        assert_eq!(stages[0].name, "Messaged");
        assert_eq!(stages[0].kind, KIND_MESSAGING);
        assert_eq!(stages[0].position, 0);
        assert_eq!(stages[0].color, "blue");
        assert_eq!(stages[3].name, "Feedback");
        assert_eq!(stages[3].position, 3);
        assert_eq!(stages[3].color, "purple");
    }

    #[test]
    fn append_adds_standard_stage_at_end() {
        let conn = setup();
        let pitch = seed_pitch(&conn, "P");
        create_many(&conn, pitch, &full_cycle_template()).unwrap();
        let added = append(&conn, pitch, "Negotiation").unwrap();
        assert_eq!(added.kind, KIND_STANDARD);
        assert_eq!(added.position, 4);
        assert_eq!(added.color, "teal"); // position 4 in the palette rotation
        assert_eq!(list_by_pitch(&conn, pitch).unwrap().len(), 5);
    }

    #[test]
    fn set_color_updates_and_missing_returns_none() {
        let conn = setup();
        let pitch = seed_pitch(&conn, "P");
        let stages = create_many(&conn, pitch, &full_cycle_template()).unwrap();
        let updated = set_color(&conn, stages[1].id, "red").unwrap().unwrap();
        assert_eq!(updated.color, "red");
        assert!(set_color(&conn, 9999, "red").unwrap().is_none());
    }

    #[test]
    fn rename_updates_and_missing_returns_none() {
        let conn = setup();
        let pitch = seed_pitch(&conn, "P");
        let stages = create_many(&conn, pitch, &full_cycle_template()).unwrap();
        let renamed = rename(&conn, stages[1].id, "Call").unwrap().unwrap();
        assert_eq!(renamed.name, "Call");
        assert!(rename(&conn, 9999, "x").unwrap().is_none());
    }

    #[test]
    fn reassign_and_delete_moves_prospects_and_removes_stage() {
        let conn = setup();
        let pitch = seed_pitch(&conn, "P");
        let stages = create_many(&conn, pitch, &full_cycle_template()).unwrap();
        let meeting = stages[1].id;
        let messaged = stages[0].id;
        conn.execute(
            "INSERT INTO prospects (name, linkedin_url, pitch_id, stage_id) VALUES ('A', 'u', ?1, ?2)",
            params![pitch, meeting],
        )
        .unwrap();

        let prev = previous_id(&conn, pitch, stages[1].position)
            .unwrap()
            .unwrap();
        assert_eq!(prev, messaged);

        reassign_and_delete(&conn, meeting, prev).unwrap();
        assert_eq!(list_by_pitch(&conn, pitch).unwrap().len(), 3);
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
        let pitch = seed_pitch(&conn, "P");
        let stages = create_many(&conn, pitch, &full_cycle_template()).unwrap();
        // Keep messaging first; swap the last two.
        let ids = [stages[0].id, stages[1].id, stages[3].id, stages[2].id];
        reorder(&conn, &ids).unwrap();
        let after = list_by_pitch(&conn, pitch).unwrap();
        assert_eq!(after[2].name, "Feedback");
        assert_eq!(after[3].name, "Onboarding");
    }

    #[test]
    fn deleting_pitch_cascades_stages() {
        let conn = setup();
        let pitch = seed_pitch(&conn, "P");
        create_many(&conn, pitch, &full_cycle_template()).unwrap();
        conn.execute("DELETE FROM pitches WHERE id = ?1", [pitch])
            .unwrap();
        assert_eq!(list_by_pitch(&conn, pitch).unwrap().len(), 0);
    }
}
