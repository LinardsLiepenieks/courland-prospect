//! Tauri commands for prospects. Reads are exposed here for the desktop UI's
//! Prospects tab. Writes happen over the loopback ingest server (the Chrome
//! extension POSTs captured prospects), not through a command, so there's no
//! `create_prospect` here by design.

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

/// Move a prospect to a different stage of its own pipeline (drag-and-drop or
/// the card's stage menu). The stage must belong to the prospect's pitch.
#[tauri::command]
pub fn set_prospect_stage(
    state: State<AppState>,
    id: i64,
    stage_id: i64,
) -> Result<Prospect, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    match repository::set_stage(&conn, id, stage_id).map_err(|e| e.to_string())? {
        Some(prospect) => Ok(prospect),
        None => Err("Prospect or stage not found for this pipeline.".into()),
    }
}
