//! Tauri commands for snippets — the "controller" the frontend invokes. Thin by
//! design: lock the shared connection, trim input, delegate to `repository`, and
//! map errors to strings the UI can display.
//!
//! `pitch_id` is `Option<i64>`: `Some(id)` scopes to that pitch, `None` to the
//! global profile. Names and contents are intentionally *not* required — a
//! freshly-added snippet is blank and filled in later — so nothing here rejects
//! an empty string.

use tauri::State;

use super::model::Snippet;
use super::repository;
use crate::database::AppState;
use crate::util::{bounded, MAX_NAME_LEN, MAX_TEXT_LEN};

#[tauri::command]
pub fn list_snippets(state: State<AppState>, pitch_id: Option<i64>) -> Result<Vec<Snippet>, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    repository::list(&conn, pitch_id).map_err(|e| e.to_string())
}

/// Create a blank snippet owned by `pitch_id` (or the profile when `None`) and
/// return it so the UI can render the new card with its server id.
#[tauri::command]
pub fn create_snippet(state: State<AppState>, pitch_id: Option<i64>) -> Result<Snippet, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    repository::create(&conn, pitch_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn update_snippet(
    state: State<AppState>,
    id: i64,
    name: String,
    content: String,
) -> Result<Snippet, String> {
    let name = bounded(&name, MAX_NAME_LEN, "Snippet name")?;
    let content = bounded(&content, MAX_TEXT_LEN, "Snippet content")?;
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    match repository::update(&conn, id, name, content).map_err(|e| e.to_string())? {
        Some(snippet) => Ok(snippet),
        None => Err("Snippet not found.".into()),
    }
}

#[tauri::command]
pub fn delete_snippet(state: State<AppState>, id: i64) -> Result<(), String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    match repository::delete(&conn, id).map_err(|e| e.to_string())? {
        0 => Err("Snippet not found.".into()),
        _ => Ok(()),
    }
}
