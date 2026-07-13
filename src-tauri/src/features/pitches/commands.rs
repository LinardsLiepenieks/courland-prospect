//! Tauri commands for pitches — the "controller" the frontend invokes. Thin by
//! design: lock the shared connection, validate input, delegate to
//! `repository`, and map errors to strings the UI can display.

use tauri::State;

use super::model::Pitch;
use super::repository;
use crate::database::AppState;
use crate::features::stages::model::{validate_inputs, StageInput};
use crate::features::stages::repository as stages_repo;
use crate::util::{bounded, MAX_NAME_LEN, MAX_TEXT_LEN};

// Cross-feature coupling (deliberate): `create_pitch` reaches into the sibling
// `stages` repository directly so the pitch insert and its stage seeding share
// ONE transaction — a pitch must never exist without a pipeline. Routing through
// the stages *command* layer would re-lock the connection and deadlock, so we
// call its repository within our own `tx`. This is the one sanctioned reach
// across the feature boundary; keep it confined to seeding at creation.

#[tauri::command]
pub fn list_pitches(state: State<AppState>) -> Result<Vec<Pitch>, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    repository::list(&conn).map_err(|e| e.to_string())
}

/// Create a pitch and seed its pipeline in one transaction, so a pitch never
/// exists without stages. `stages` is the pipeline the user set up in the create
/// flow; an empty list falls back to the built-in Full-cycle template.
#[tauri::command]
pub fn create_pitch(
    state: State<AppState>,
    name: String,
    skill: String,
    stages: Vec<StageInput>,
) -> Result<Pitch, String> {
    let name = bounded(&name, MAX_NAME_LEN, "Pitch name")?;
    if name.is_empty() {
        return Err("Pitch name is required.".into());
    }
    let skill = bounded(&skill, MAX_TEXT_LEN, "Skill")?;
    let seeds = if stages.is_empty() {
        stages_repo::full_cycle_template()
    } else {
        validate_inputs(stages)?
    };

    let mut conn = state.conn.lock().map_err(|e| e.to_string())?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let pitch = repository::create(&tx, name, skill).map_err(|e| e.to_string())?;
    stages_repo::create_many(&tx, pitch.id, &seeds).map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(pitch)
}

#[tauri::command]
pub fn update_pitch(
    state: State<AppState>,
    id: i64,
    name: String,
    skill: String,
) -> Result<Pitch, String> {
    let name = bounded(&name, MAX_NAME_LEN, "Pitch name")?;
    if name.is_empty() {
        return Err("Pitch name is required.".into());
    }
    let skill = bounded(&skill, MAX_TEXT_LEN, "Skill")?;
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    match repository::update(&conn, id, name, skill).map_err(|e| e.to_string())? {
        Some(pitch) => Ok(pitch),
        None => Err("Pitch not found.".into()),
    }
}

/// Delete a pitch and all work scoped to it (its prospects, their messages, and
/// its stages) in one transaction. The frontend confirms this is permanent.
#[tauri::command]
pub fn delete_pitch(state: State<AppState>, id: i64) -> Result<(), String> {
    let mut conn = state.conn.lock().map_err(|e| e.to_string())?;
    match repository::delete(&mut conn, id).map_err(|e| e.to_string())? {
        0 => Err("Pitch not found.".into()),
        _ => Ok(()),
    }
}

/// Polish a pitch's skill text through the local Claude Code CLI, returning the
/// rewritten version. Touches no DB — the UI drops the result into the editor
/// for the user to review and save. Async + off-thread so a multi-second
/// generation never blocks the UI.
#[tauri::command]
pub async fn polish_skill(text: String) -> Result<String, String> {
    crate::ai::client::polish(
        text,
        crate::ai::Prompt::polish_skill,
        "Nothing to polish yet — write a skill first.",
    )
    .await
}
