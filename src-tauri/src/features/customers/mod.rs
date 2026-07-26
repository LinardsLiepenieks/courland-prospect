//! Customers feature — the ideal customer profiles (ICPs) you sell the one
//! product to. Replaces the old `pitches` slice, and inverts what it modelled:
//! a pitch was a separate *thing being sold*, each with its own pipeline and its
//! own snippet library. There is only one product, so what actually varies is
//! the buyer — who they are, what hurts, and what you want out of a thread with
//! them.
//!
//! A customer profile owns nothing. It is pure context the AI reads: the
//! pipeline is shared (`features::stages`), the snippet library is shared
//! (`features::snippets`), and a prospect merely points at one
//! (`prospects.customer_id`, nullable — an unassigned prospect is still in the
//! pipeline, their drafts just get no customer block).
//!
//! Instance of the standard per-feature ("controller") layout. Every feature
//! folder has the same four files:
//!   - `model`      — the struct(s) + row mapping
//!   - `repository` — all SQL (`&Connection` in, no Tauri types)
//!   - `commands`   — the `#[tauri::command]` handlers the frontend invokes
//!   - `mod`        — declares the submodules and re-exports the public surface

pub mod commands;
mod model;
// `pub(crate)` (not private) because the ingest HTTP server in `crate::ingest`
// serves the customer list to the extension's dropdown and resolves a prospect's
// customer when drafting.
pub(crate) mod repository;
