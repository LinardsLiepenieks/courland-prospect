//! Product feature — the ONE thing this app sells.
//!
//! The app is built around a single product, so its story is app-wide context
//! rather than something restated per campaign: the AI reads it to understand
//! what is being sold, and each [`crate::features::customers`] profile describes
//! who it's being sold TO and what a thread with them should achieve.
//!
//! Instance of the standard per-feature slice layout (see `customers` for the
//! reference shape). Like `profile` and `selectors`, product is a *singleton* —
//! one row pinned at id = 1 — so the repository exposes `get`/`update` rather
//! than a create/list/delete set.

pub mod commands;
mod model;
// `pub(crate)` (not private) so the ingest server can read the product as
// drafting material for `POST /draft` — mirrors the profile slice.
pub(crate) mod repository;
