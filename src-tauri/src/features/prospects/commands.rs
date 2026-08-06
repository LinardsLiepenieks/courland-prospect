//! Tauri commands for prospects. Reads are exposed here for the desktop UI's
//! Prospects tab. *Creation* happens over the loopback ingest server (the Chrome
//! extension POSTs captured prospects), not through a command, so there's no
//! `create_prospect` here by design — but the board can move a prospect through
//! the pipeline and re-tag which customer profile they match.

use tauri::{AppHandle, Manager, State};

use super::advance::{self, Priority};
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

/// Accept the analyzer's pending advance suggestion — the "✓" on the card. This
/// is deliberately not just a client-side `set_prospect_stage(suggested_id)`:
/// reading the suggestion and applying it happen under one connection lock, so a
/// suggestion superseded (or retracted) between render and click can't move
/// someone to a stage the app no longer believes in. A card whose suggestion has
/// since cleared reports it rather than moving anyone.
#[tauri::command]
pub fn accept_stage_suggestion(state: State<AppState>, id: i64) -> Result<Prospect, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    let prospect = repository::find(&conn, id)
        .map_err(|e| e.to_string())?
        .ok_or("Prospect not found.")?;
    let target = prospect
        .suggested_stage_id
        .ok_or("That suggestion is no longer current.")?;
    // `set_stage` clears the suggestion as part of the move.
    repository::set_stage(&conn, id, target)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "That suggestion points at a stage that no longer exists.".into())
}

/// Dismiss the pending suggestion without moving the prospect — the "✕" on the
/// card. Deliberately not remembered: the next message in the thread is new
/// evidence, and the analyzer is free to reach the same conclusion again. A
/// permanent "never suggest this" would need a reason we don't have.
#[tauri::command]
pub fn dismiss_stage_suggestion(state: State<AppState>, id: i64) -> Result<Prospect, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    repository::clear_suggestion(&conn, id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Prospect not found.".into())
}

/// Re-run the advance check for one prospect on demand — the card's "re-check"
/// action. The analyzer normally fires on its own whenever a message lands, so
/// this exists for the cases where nothing new will arrive to trigger it: you
/// moved someone by hand, or you just wrote the stage's goal and want it applied
/// to the threads already sitting there.
///
/// Runs at foreground priority (queues for a CLI permit instead of skipping when
/// the pool is busy) and surfaces its errors, because unlike the background pass
/// somebody is waiting on this one. Returns the prospect either way — a verdict
/// of "not yet" is a normal answer, not a failure, and leaves the row unchanged.
///
/// Takes only the `AppHandle`: this is an async command, so it can't hold a
/// `State` borrow across the CLI await — the analyzer resolves the state inside
/// each `spawn_blocking` instead.
#[tauri::command]
pub async fn recheck_prospect_stage(app: AppHandle, id: i64) -> Result<Prospect, String> {
    advance::analyze(&app, id, Priority::Foreground).await?;
    tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let conn = state.conn.lock().map_err(|e| e.to_string())?;
        repository::find(&conn, id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "Prospect not found.".to_string())
    })
    .await
    .map_err(|e| e.to_string())?
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
