//! Tauri commands for the product singleton — the "controller" the frontend
//! invokes. Thin by design: lock the shared connection, trim input, delegate to
//! `repository`, and map errors to strings the UI can display.

use tauri::State;

use super::model::Product;
use super::repository;
use crate::database::AppState;
use crate::util::{bounded, MAX_NAME_LEN, MAX_TEXT_LEN};

#[tauri::command]
pub fn get_product(state: State<AppState>) -> Result<Product, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    repository::get(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn update_product(
    state: State<AppState>,
    name: String,
    description: String,
) -> Result<Product, String> {
    let name = bounded(&name, MAX_NAME_LEN, "Product name")?;
    let description = bounded(&description, MAX_TEXT_LEN, "Product description")?;
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    repository::update(&conn, name, description).map_err(|e| e.to_string())
}

/// Polish the product description through the local Claude Code CLI, returning
/// the rewritten version. Touches no DB — the UI drops the result into the
/// editor for the user to review and save. Async + off-thread so a multi-second
/// generation never blocks the UI.
#[tauri::command]
pub async fn polish_product(text: String) -> Result<String, String> {
    crate::ai::client::polish(
        text,
        crate::ai::Prompt::polish_product,
        "Nothing to polish yet — describe your product first.",
    )
    .await
}
