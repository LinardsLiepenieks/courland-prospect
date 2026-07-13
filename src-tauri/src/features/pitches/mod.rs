//! Pitches feature — the thing you're selling (design-in-code, ideas, ...).
//!
//! Instance of the standard per-feature ("controller") layout. Every feature
//! folder has the same four files:
//!   - `model`      — the struct(s) + row mapping
//!   - `repository` — all SQL (`&Connection` in, no Tauri types)
//!   - `commands`   — the `#[tauri::command]` handlers the frontend invokes
//!   - `mod`        — declares the submodules and re-exports the public surface
//!
//! New features (prospects, stages, ...) drop in as sibling folders with this
//! exact shape; register their commands in `lib.rs`.

pub mod commands;
mod model;
// `pub(crate)` (not private) because the ingest HTTP server in `crate::ingest`
// serves the pitch list to the extension's dropdown by calling `repository::list`.
pub(crate) mod repository;
