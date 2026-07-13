//! Tauri commands for pipeline stages — the frontend's entry point for editing
//! a pitch's funnel. Thin: lock the shared connection, validate input, delegate
//! to `repository`, and map errors to strings the UI can display.

use std::collections::HashSet;

use tauri::{AppHandle, Emitter, State};

use super::model::{is_valid_color, Stage, KIND_MESSAGING};
use super::repository;
use crate::database::AppState;
use crate::util::{bounded, MAX_NAME_LEN};

/// Emitted after any pipeline edit so views rendering the pipeline elsewhere
/// (the Prospects board) re-fetch instead of going stale. A delete reassigns
/// prospects, so listeners re-fetch prospects too. Read-only `list_stages`
/// doesn't emit.
const STAGES_CHANGED: &str = "stages://changed";

#[tauri::command]
pub fn list_stages(state: State<AppState>, pitch_id: i64) -> Result<Vec<Stage>, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    repository::list_by_pitch(&conn, pitch_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn create_stage(
    app: AppHandle,
    state: State<AppState>,
    pitch_id: i64,
    name: String,
) -> Result<Stage, String> {
    let name = bounded(&name, MAX_NAME_LEN, "Stage name")?;
    if name.is_empty() {
        return Err("Stage name is required.".into());
    }
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    let stage = repository::append(&conn, pitch_id, name).map_err(|e| e.to_string())?;
    let _ = app.emit(STAGES_CHANGED, ());
    Ok(stage)
}

#[tauri::command]
pub fn rename_stage(
    app: AppHandle,
    state: State<AppState>,
    id: i64,
    name: String,
) -> Result<Stage, String> {
    let name = bounded(&name, MAX_NAME_LEN, "Stage name")?;
    if name.is_empty() {
        return Err("Stage name is required.".into());
    }
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    let stage = repository::rename(&conn, id, name)
        .map_err(|e| e.to_string())?
        .ok_or("Stage not found.")?;
    let _ = app.emit(STAGES_CHANGED, ());
    Ok(stage)
}

/// Set a stage's color to a palette token (validated against the known set).
#[tauri::command]
pub fn set_stage_color(
    app: AppHandle,
    state: State<AppState>,
    id: i64,
    color: String,
) -> Result<Stage, String> {
    if !is_valid_color(&color) {
        return Err(format!("Unknown stage color: {color}"));
    }
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    let stage = repository::set_color(&conn, id, &color)
        .map_err(|e| e.to_string())?
        .ok_or("Stage not found.")?;
    let _ = app.emit(STAGES_CHANGED, ());
    Ok(stage)
}

/// Delete a stage, moving any prospects in it to the previous stage. The
/// messaging stage can't be deleted, nor can the last remaining stage.
#[tauri::command]
pub fn delete_stage(app: AppHandle, state: State<AppState>, id: i64) -> Result<(), String> {
    let mut conn = state.conn.lock().map_err(|e| e.to_string())?;
    let stage = repository::get(&conn, id)
        .map_err(|e| e.to_string())?
        .ok_or("Stage not found.")?;
    if stage.kind == KIND_MESSAGING {
        return Err("The messaging stage can't be deleted.".into());
    }
    if repository::count_for_pitch(&conn, stage.pitch_id).map_err(|e| e.to_string())? <= 1 {
        return Err("A pipeline needs at least one stage.".into());
    }
    // Any non-messaging stage sits after the messaging stage (position 0), so a
    // previous stage always exists here.
    let prev = repository::previous_id(&conn, stage.pitch_id, stage.position)
        .map_err(|e| e.to_string())?
        .ok_or("No earlier stage to move prospects into.")?;

    let tx = conn.transaction().map_err(|e| e.to_string())?;
    repository::reassign_and_delete(&tx, id, prev).map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    // A delete also reassigns prospects — listeners re-fetch both.
    let _ = app.emit(STAGES_CHANGED, ());
    Ok(())
}

/// Reorder a pitch's stages to match `ordered_ids`. The ids must be exactly the
/// pitch's stages, and the messaging stage must stay first.
#[tauri::command]
pub fn reorder_stages(
    app: AppHandle,
    state: State<AppState>,
    pitch_id: i64,
    ordered_ids: Vec<i64>,
) -> Result<Vec<Stage>, String> {
    let mut conn = state.conn.lock().map_err(|e| e.to_string())?;
    let current = repository::list_by_pitch(&conn, pitch_id).map_err(|e| e.to_string())?;

    let current_ids: HashSet<i64> = current.iter().map(|s| s.id).collect();
    let incoming_ids: HashSet<i64> = ordered_ids.iter().copied().collect();
    if incoming_ids.len() != ordered_ids.len() || current_ids != incoming_ids {
        return Err("Reorder must list exactly the pitch's stages, once each.".into());
    }
    let messaging_id = current.iter().find(|s| s.kind == KIND_MESSAGING).map(|s| s.id);
    if ordered_ids.first().copied() != messaging_id {
        return Err("The messaging stage must stay first.".into());
    }

    let tx = conn.transaction().map_err(|e| e.to_string())?;
    repository::reorder(&tx, &ordered_ids).map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    let reordered = repository::list_by_pitch(&conn, pitch_id).map_err(|e| e.to_string())?;
    let _ = app.emit(STAGES_CHANGED, ());
    Ok(reordered)
}
