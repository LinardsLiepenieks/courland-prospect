//! Tauri commands for the comment inbox — the Comments tab's entry point. Thin by
//! design: lock the shared connection, validate input, delegate to `repository`,
//! map errors to strings the UI can show, and emit `comments://changed` on a
//! mutation so any open view (and changes the extension makes over the loopback
//! server) reconcile live. The extension side uses the HTTP routes in
//! `crate::ingest`, not these.

use tauri::{AppHandle, Emitter, State};

use super::model::{CommentDraft, CommentRun};
use super::{repository, COMMENTS_CHANGED};
use crate::database::AppState;
use crate::util::bounded;
use crate::util::MAX_TEXT_LEN;

/// Bounds on a scrape's placed-draft budget. Matches the UI's 5–50 choices with
/// headroom; clamped (not rejected) so a stale/hand-crafted value still runs.
const MIN_COUNT: i64 = 1;
const MAX_COUNT: i64 = 100;

#[tauri::command]
pub fn list_comment_drafts(state: State<AppState>) -> Result<Vec<CommentDraft>, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    repository::list(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_comment_run(state: State<AppState>) -> Result<CommentRun, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    repository::get_run(&conn).map_err(|e| e.to_string())
}

/// Ask the extension to run a scrape: records the budget + watchlist choice and
/// flips the run to `requested`. The extension picks it up on its next poll — near
/// instant if a LinkedIn tab is focused, else within its alarm interval.
#[tauri::command]
pub fn request_comment_scrape(
    app: AppHandle,
    state: State<AppState>,
    count: i64,
    include_watchlist: bool,
) -> Result<CommentRun, String> {
    let count = count.clamp(MIN_COUNT, MAX_COUNT);
    let run = {
        let conn = state.conn.lock().map_err(|e| e.to_string())?;
        repository::request_scrape(&conn, count, include_watchlist).map_err(|e| e.to_string())?;
        repository::get_run(&conn).map_err(|e| e.to_string())?
    };
    let _ = app.emit(COMMENTS_CHANGED, ());
    Ok(run)
}

/// Save an edited comment. Rejected once the draft is posting/posted (the
/// repository only updates editable rows) — surfaced as a not-found error so the
/// UI reloads and drops its stale edit affordance.
#[tauri::command]
pub fn update_comment_draft(
    app: AppHandle,
    state: State<AppState>,
    id: i64,
    comment: String,
) -> Result<CommentDraft, String> {
    let comment = bounded(&comment, MAX_TEXT_LEN, "Comment")?;
    let updated = {
        let conn = state.conn.lock().map_err(|e| e.to_string())?;
        repository::update_comment(&conn, id, comment).map_err(|e| e.to_string())?
    };
    match updated {
        Some(draft) => {
            let _ = app.emit(COMMENTS_CHANGED, ());
            Ok(draft)
        }
        None => Err("That draft can no longer be edited.".into()),
    }
}

#[tauri::command]
pub fn delete_comment_draft(
    app: AppHandle,
    state: State<AppState>,
    id: i64,
) -> Result<(), String> {
    let deleted = {
        let conn = state.conn.lock().map_err(|e| e.to_string())?;
        repository::delete(&conn, id).map_err(|e| e.to_string())?
    };
    match deleted {
        0 => Err("Draft not found.".into()),
        _ => {
            let _ = app.emit(COMMENTS_CHANGED, ());
            Ok(())
        }
    }
}

/// The "Post all" action: queue every editable, non-empty draft for the extension
/// to post. Returns how many were queued (0 when nothing was ready). Blank drafts
/// and already-posted ones are left as-is.
#[tauri::command]
pub fn queue_comment_drafts(app: AppHandle, state: State<AppState>) -> Result<usize, String> {
    let queued = {
        let conn = state.conn.lock().map_err(|e| e.to_string())?;
        repository::queue_all(&conn).map_err(|e| e.to_string())?
    };
    if queued > 0 {
        let _ = app.emit(COMMENTS_CHANGED, ());
    }
    Ok(queued)
}
