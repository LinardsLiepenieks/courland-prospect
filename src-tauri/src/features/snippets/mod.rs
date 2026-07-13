//! Snippets feature — named text fragments that will later compose into
//! messages. A snippet is owned by exactly one place: a pitch (`pitch_id` set)
//! or the global profile (`pitch_id` None). Single ownership — a snippet never
//! belongs to more than one pitch — so its origin (pitch vs profile) is always
//! knowable from `pitch_id`.
//!
//! Instance of the standard per-feature layout (see `features::pitches`): the
//! same four files — `model`, `repository`, `commands`, `mod`. Register its
//! commands in `lib.rs`.

pub mod commands;
mod model;
// `pub(crate)` (not private) so the ingest server can read snippets as drafting
// material for `POST /draft` — mirrors the pitches slice.
pub(crate) mod repository;
