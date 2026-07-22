//! Watchlist feature — the hand-curated list of LinkedIn profiles a comment run
//! checks for new posts, on top of the main feed. A global list (not scoped to a
//! pitch, not per-prospect); the user manages it from the Profile tab.
//!
//! Standard per-feature layout (see `features::pitches`): `model` / `repository` /
//! `commands` / `mod`. Register its commands in `lib.rs`.
//!
//! `model` and `repository` are `pub(crate)` (not private) because the loopback
//! ingest server in `crate::ingest` reads the list directly (the Chrome extension
//! fetches it at the start of a comment run to know which profiles to visit).

pub mod commands;
pub(crate) mod model;
pub(crate) mod repository;
