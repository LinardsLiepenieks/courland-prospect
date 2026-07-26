//! Profile feature — who the *user* is: their background, role, and voice.
//! App-wide reference notes the AI reasons about when writing as them.
//!
//! Deliberately narrow. "What are you building" used to live here too, but the
//! product story is now its own singleton (`features::product`, migration 0022)
//! — the person writing and the thing being sold are different jobs, and the
//! commenter depends on that separation to borrow the founder's persona without
//! ever letting a public comment read as pitch copy. (Also distinct from the
//! Chrome profiles the capture browser launches into, and from a *customer*
//! profile, which describes a buyer.)
//!
//! Instance of the standard per-feature layout (see `customers` for the
//! reference shape): `model` + `repository` + `commands` + this `mod`. The one
//! twist is that profile is a *singleton* — a single row pinned at id = 1 — so
//! the repository exposes `get`/`update` rather than a create/list/delete set.

pub mod commands;
mod model;
// `pub(crate)` (not private) so the ingest server can read the profile as
// drafting material for `POST /draft` — mirrors the customers slice.
pub(crate) mod repository;
