//! Stages feature — a pitch's ordered pipeline (the funnel a prospect moves
//! through). Standard per-feature layout (model / repository / commands).
//!
//! `model` and `repository` are `pub(crate)` (not private) because pitch
//! creation seeds a pipeline: `features::pitches::commands::create_pitch`
//! orchestrates pitch + stage inserts in one transaction, calling
//! `repository::create_many` with `model::StageInput`.

pub mod commands;
pub(crate) mod model;
pub(crate) mod repository;
