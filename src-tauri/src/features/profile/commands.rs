//! Tauri commands for profile — the "controller" the frontend invokes. Thin by
//! design: lock the shared connection, trim input, delegate to `repository`,
//! and map errors to strings the UI can display.

use tauri::State;

use super::model::Profile;
use super::repository;
use crate::database::AppState;
use crate::util::{bounded, MAX_TEXT_LEN};

#[tauri::command]
pub fn get_profile(state: State<AppState>) -> Result<Profile, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    repository::get(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn update_profile(
    state: State<AppState>,
    who_are_you: String,
    what_building: String,
) -> Result<Profile, String> {
    let who_are_you = bounded(&who_are_you, MAX_TEXT_LEN, "Who are you")?;
    let what_building = bounded(&what_building, MAX_TEXT_LEN, "What are you building")?;
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    repository::update(&conn, who_are_you, what_building).map_err(|e| e.to_string())
}

/// Polish the "who are you" context through the local Claude Code CLI, returning
/// the rewritten version. Touches no DB — the UI drops the result into the editor
/// for the user to review and (auto)save. Async + off-thread so a multi-second
/// generation never blocks the UI.
#[tauri::command]
pub async fn polish_who(text: String) -> Result<String, String> {
    crate::ai::client::polish(
        text,
        crate::ai::Prompt::polish_profile_who,
        "Nothing to polish yet — write something first.",
    )
    .await
}

/// Polish the "what are you building" context. See `polish_who`.
#[tauri::command]
pub async fn polish_building(text: String) -> Result<String, String> {
    crate::ai::client::polish(
        text,
        crate::ai::Prompt::polish_profile_building,
        "Nothing to polish yet — write something first.",
    )
    .await
}
