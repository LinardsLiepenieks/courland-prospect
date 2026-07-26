//! Tauri commands for prospects. Reads are exposed here for the desktop UI's
//! Prospects tab. *Creation* happens over the loopback ingest server (the Chrome
//! extension POSTs captured prospects), not through a command, so there's no
//! `create_prospect` here by design — but the board can move a prospect through
//! the pipeline and re-tag which customer profile they match.

use tauri::State;

use super::model::Prospect;
use super::repository;
use crate::database::AppState;

#[tauri::command]
pub fn list_prospects(state: State<AppState>) -> Result<Vec<Prospect>, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    repository::list(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_prospect(state: State<AppState>, id: i64) -> Result<(), String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    match repository::delete(&conn, id).map_err(|e| e.to_string())? {
        0 => Err("Prospect not found.".into()),
        _ => Ok(()),
    }
}

/// Move a prospect to a different stage of the pipeline (drag-and-drop or the
/// card's stage menu).
#[tauri::command]
pub fn set_prospect_stage(
    state: State<AppState>,
    id: i64,
    stage_id: i64,
) -> Result<Prospect, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    match repository::set_stage(&conn, id, stage_id).map_err(|e| e.to_string())? {
        Some(prospect) => Ok(prospect),
        None => Err("Prospect or stage not found.".into()),
    }
}

/// Re-tag which customer profile a prospect matches, or clear it (`None`). This
/// only changes how their drafts are steered — it never moves them on the board —
/// so the UI offers it inline on the row with no confirmation.
#[tauri::command]
pub fn set_prospect_customer(
    state: State<AppState>,
    id: i64,
    customer_id: Option<i64>,
) -> Result<Prospect, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    match repository::set_customer(&conn, id, customer_id) {
        Ok(Some(prospect)) => Ok(prospect),
        Ok(None) => Err("Prospect not found.".into()),
        // The profile was deleted between opening the menu and picking it, so
        // the update trips the foreign key. Surface a plain message instead of
        // the raw "FOREIGN KEY constraint failed".
        Err(e) if is_foreign_key_violation(&e) => Err("That customer profile no longer exists.".into()),
        Err(e) => Err(e.to_string()),
    }
}

/// True when a rusqlite error is specifically a foreign-key constraint violation
/// — the signal that the chosen customer profile no longer exists.
fn is_foreign_key_violation(e: &rusqlite::Error) -> bool {
    matches!(
        e,
        rusqlite::Error::SqliteFailure(f, _)
            if f.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_FOREIGNKEY
    )
}
