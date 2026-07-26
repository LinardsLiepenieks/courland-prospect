//! Tauri commands for customer profiles — the "controller" the frontend
//! invokes. Thin by design: lock the shared connection, validate input, delegate
//! to `repository`, and map errors to strings the UI can display.
//!
//! No cross-feature reach here. The old `create_pitch` had to seed a pipeline in
//! the same transaction (a pitch could never exist without stages); a customer
//! profile owns no pipeline — there is exactly one, shared — so creating one is
//! a plain insert.

use tauri::State;

use super::model::Customer;
use super::repository;
use crate::database::AppState;
use crate::util::{bounded, MAX_NAME_LEN, MAX_TEXT_LEN};

/// Trim + bound the three free-text fields a customer profile carries. Only
/// `name` is required — a profile is useful the moment it's named, and who/pain/
/// goal get filled in as you learn them.
fn fields<'a>(
    who_they_are: &'a str,
    pain: &'a str,
    goal: &'a str,
) -> Result<(&'a str, &'a str, &'a str), String> {
    Ok((
        bounded(who_they_are, MAX_TEXT_LEN, "Who they are")?,
        bounded(pain, MAX_TEXT_LEN, "What they care about")?,
        bounded(goal, MAX_TEXT_LEN, "Goal")?,
    ))
}

#[tauri::command]
pub fn list_customers(state: State<AppState>) -> Result<Vec<Customer>, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    repository::list(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn create_customer(
    state: State<AppState>,
    name: String,
    who_they_are: String,
    pain: String,
    goal: String,
) -> Result<Customer, String> {
    let name = bounded(&name, MAX_NAME_LEN, "Customer name")?;
    if name.is_empty() {
        return Err("Customer name is required.".into());
    }
    let (who_they_are, pain, goal) = fields(&who_they_are, &pain, &goal)?;
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    repository::create(&conn, name, who_they_are, pain, goal).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn update_customer(
    state: State<AppState>,
    id: i64,
    name: String,
    who_they_are: String,
    pain: String,
    goal: String,
) -> Result<Customer, String> {
    let name = bounded(&name, MAX_NAME_LEN, "Customer name")?;
    if name.is_empty() {
        return Err("Customer name is required.".into());
    }
    let (who_they_are, pain, goal) = fields(&who_they_are, &pain, &goal)?;
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    match repository::update(&conn, id, name, who_they_are, pain, goal)
        .map_err(|e| e.to_string())?
    {
        Some(customer) => Ok(customer),
        None => Err("Customer not found.".into()),
    }
}

/// Delete a customer profile. Its prospects stay in the pipeline and simply
/// become unassigned (see `repository::delete`), so unlike the pitch delete this
/// replaced, nothing the user captured is lost — the frontend says as much
/// rather than warning about permanent data loss.
#[tauri::command]
pub fn delete_customer(state: State<AppState>, id: i64) -> Result<(), String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    match repository::delete(&conn, id).map_err(|e| e.to_string())? {
        0 => Err("Customer not found.".into()),
        _ => Ok(()),
    }
}
