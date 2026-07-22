//! Tauri commands for snippets — the "controller" the frontend invokes. Thin by
//! design: lock the shared connection, trim input, delegate to `repository`, and
//! map errors to strings the UI can display.
//!
//! `pitch_id` is `Option<i64>`: `Some(id)` scopes to that pitch, `None` to the
//! global profile. Names and contents are intentionally *not* required — a
//! freshly-added snippet is blank and filled in later — so nothing here rejects
//! an empty string.
//!
//! Two commands (`update_snippet`, `approve_snippet`) additionally kick off the
//! background `classify` pass after they persist, so a snippet's arc `position` and
//! `category` are (re)derived whenever its content or status meaningfully changes.
//! That pass is fire-and-forget: it never blocks the command's response, and the UI
//! learns of the result via the `snippets://changed` event.

use tauri::{AppHandle, Emitter, State};

use super::model::Snippet;
use super::{classify, repository, SNIPPETS_CHANGED};
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

/// Persist a snippet's name + content, then (if it now has content) fire the
/// classify pass to re-derive its arc position and category. The pass skips manual
/// rows, so a hand-organized snippet is left alone.
#[tauri::command]
pub fn update_snippet(
    app: AppHandle,
    state: State<AppState>,
    id: i64,
    name: String,
    content: String,
) -> Result<Snippet, String> {
    let name = bounded(&name, MAX_NAME_LEN, "Snippet name")?;
    let content = bounded(&content, MAX_TEXT_LEN, "Snippet content")?;
    let (snippet, content_changed) = {
        let conn = state.conn.lock().map_err(|e| e.to_string())?;
        // Whether the content actually changed vs. what's stored — so a name-only
        // edit doesn't needlessly re-run the classify pass (which could otherwise
        // jitter the arc position on an edit that never touched the text).
        let content_changed = repository::find(&conn, id)
            .map_err(|e| e.to_string())?
            .map_or(true, |prior| prior.content.trim() != content.trim());
        match repository::update(&conn, id, name, content).map_err(|e| e.to_string())? {
            Some(snippet) => (snippet, content_changed),
            None => return Err("Snippet not found.".into()),
        }
    };
    if content_changed && !content.trim().is_empty() {
        classify::spawn(app, id);
    }
    Ok(snippet)
}

#[tauri::command]
pub fn delete_snippet(state: State<AppState>, id: i64) -> Result<(), String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    match repository::delete(&conn, id).map_err(|e| e.to_string())? {
        0 => Err("Snippet not found.".into()),
        _ => Ok(()),
    }
}

/// Approve an AI-proposed snippet — flip its status to `approved` so it becomes a
/// normal, editable snippet that composes drafts, then classify it (a proposal has
/// no arc position/category until it joins the library). Rejecting a proposal reuses
/// `delete_snippet`. Returns the updated snippet so the UI can re-render the card.
#[tauri::command]
pub fn approve_snippet(app: AppHandle, state: State<AppState>, id: i64) -> Result<Snippet, String> {
    let snippet = {
        let conn = state.conn.lock().map_err(|e| e.to_string())?;
        match repository::approve(&conn, id).map_err(|e| e.to_string())? {
            Some(snippet) => snippet,
            None => return Err("Snippet not found.".into()),
        }
    };
    classify::spawn(app, id);
    Ok(snippet)
}

/// Set a snippet's category by hand. A non-empty category pins the snippet (`manual`
/// = true), so the auto pass won't touch it; clearing it back to empty un-pins it
/// and re-runs the classify pass to re-derive a category. Returns the updated
/// snippet.
#[tauri::command]
pub fn set_snippet_category(
    app: AppHandle,
    state: State<AppState>,
    id: i64,
    category: String,
) -> Result<Snippet, String> {
    let category = bounded(&category, MAX_NAME_LEN, "Category")?;
    let cleared = category.trim().is_empty();
    let snippet = {
        let conn = state.conn.lock().map_err(|e| e.to_string())?;
        match repository::set_category(&conn, id, category).map_err(|e| e.to_string())? {
            Some(snippet) => snippet,
            None => return Err("Snippet not found.".into()),
        }
    };
    // Clearing re-enables auto-classification; re-derive a category now.
    if cleared {
        classify::spawn(app, id);
    }
    Ok(snippet)
}

/// Copy a snippet into another scope as an independent duplicate: a fresh `approved`
/// row owned by `target_pitch_id` (a pitch id, or `None` for the global profile)
/// carrying the source's name + content. The copy re-classifies in its new scope, so
/// after persisting we fire the classify pass for the new id, then emit
/// `SNIPPETS_CHANGED` for the target scope so an open editor there folds it in. The
/// source is untouched (single-owner model preserved — this adds a row, never moves
/// one). Returns the new snippet.
#[tauri::command]
pub fn copy_snippet(
    app: AppHandle,
    state: State<AppState>,
    id: i64,
    target_pitch_id: Option<i64>,
) -> Result<Snippet, String> {
    let snippet = {
        let conn = state.conn.lock().map_err(|e| e.to_string())?;
        match repository::copy(&conn, id, target_pitch_id) {
            Ok(Some(snippet)) => snippet,
            Ok(None) => return Err("Snippet not found.".into()),
            // The target pitch was deleted between opening the menu and picking it,
            // so the insert trips the foreign-key constraint. Surface a plain
            // message instead of the raw "FOREIGN KEY constraint failed".
            Err(e) if is_foreign_key_violation(&e) => {
                return Err("That pitch no longer exists.".into())
            }
            Err(e) => return Err(e.to_string()),
        }
    };
    let _ = app.emit(SNIPPETS_CHANGED, target_pitch_id);
    classify::spawn(app, snippet.id);
    Ok(snippet)
}

/// Re-score AND re-categorize every approved snippet in a scope through the AI — the
/// "reorganize my whole library" button. A full reset: it overrides hand-picked
/// (`manual`) categories and hands each row back to auto-classification. Runs to
/// completion on the interactive CLI path and emits `snippets://changed` ONCE when
/// the whole batch finishes (not per snippet), so any other open editor for the
/// scope reconciles in a single reshuffle; returns how many snippets it changed.
/// `pitch_id` scopes it (a pitch id, or `None` for the profile) — mirrors
/// `list_snippets`.
#[tauri::command]
pub async fn reclassify_snippets(app: AppHandle, pitch_id: Option<i64>) -> Result<usize, String> {
    classify::reclassify_all(app, pitch_id).await
}

/// True when a rusqlite error is specifically a foreign-key constraint violation —
/// the signal that a copy's target pitch no longer exists.
fn is_foreign_key_violation(e: &rusqlite::Error) -> bool {
    matches!(
        e,
        rusqlite::Error::SqliteFailure(f, _)
            if f.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_FOREIGNKEY
    )
}
