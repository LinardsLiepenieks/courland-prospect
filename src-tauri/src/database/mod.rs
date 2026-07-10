//! Database infrastructure — shared by every feature module.
//!
//! Holds the single local SQLite connection (in [`AppState`]) and the schema
//! [`migrations`]. Feature modules never open their own connection: they lock
//! this one and run SQL through their own `repository`. All SQL lives in Rust;
//! the frontend only calls typed Tauri commands.

pub mod migrations;

use std::path::Path;
use std::sync::Mutex;

use rusqlite::Connection;

/// Managed Tauri state holding the single local SQLite connection.
///
/// The app is single-user and local, so one connection behind a `Mutex` is
/// plenty — commands lock it for the duration of a query.
pub struct AppState {
    pub conn: Mutex<Connection>,
}

/// Open (creating if needed) the local database at `path`, apply pragmas, and
/// run any pending migrations. Returns a ready-to-use connection.
pub fn open(path: &Path) -> rusqlite::Result<Connection> {
    let mut conn = Connection::open(path)?;
    // WAL gives better read/write concurrency; foreign_keys prepares us for
    // related tables (prospects, stages) landing in later migrations;
    // busy_timeout avoids spurious SQLITE_BUSY if a second instance is opening.
    conn.execute_batch(
        "PRAGMA busy_timeout = 5000; PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;",
    )?;
    migrations::run(&mut conn)?;
    Ok(conn)
}
