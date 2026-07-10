//! Tauri commands for pitches — the "controller" the frontend invokes. Thin by
//! design: lock the shared connection, validate input, delegate to
//! `repository`, and map errors to strings the UI can display.

use tauri::State;

use super::model::Pitch;
use super::repository;
use crate::database::AppState;

#[tauri::command]
pub fn list_pitches(state: State<AppState>) -> Result<Vec<Pitch>, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    repository::list(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn create_pitch(
    state: State<AppState>,
    name: String,
    skill: String,
) -> Result<Pitch, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Pitch name is required.".into());
    }
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    repository::create(&conn, name, skill.trim()).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn update_pitch(
    state: State<AppState>,
    id: i64,
    name: String,
    skill: String,
) -> Result<Pitch, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Pitch name is required.".into());
    }
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    match repository::update(&conn, id, name, skill.trim()).map_err(|e| e.to_string())? {
        Some(pitch) => Ok(pitch),
        None => Err("Pitch not found.".into()),
    }
}

#[tauri::command]
pub fn delete_pitch(state: State<AppState>, id: i64) -> Result<(), String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    match repository::delete(&conn, id).map_err(|e| e.to_string())? {
        0 => Err("Pitch not found.".into()),
        _ => Ok(()),
    }
}
