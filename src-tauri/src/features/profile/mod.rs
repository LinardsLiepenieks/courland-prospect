//! Profile feature — the user's global "skills": who they are and what they're
//! building. These are app-wide reference notes the AI reasons about, not tied
//! to any single pitch. (Distinct from `chrome_profiles`, which manages the
//! dedicated capture browser's sandboxes.)
//!
//! Instance of the standard per-feature layout (see `pitches` for the reference
//! shape): `model` + `repository` + `commands` + this `mod`. The one twist is
//! that profile is a *singleton* — a single row pinned at id = 1 — so the
//! repository exposes `get`/`update` rather than a create/list/delete set.

pub mod commands;
mod model;
// `pub(crate)` (not private) so the ingest server can read the profile as
// drafting material for `POST /draft` — mirrors the pitches slice.
pub(crate) mod repository;
