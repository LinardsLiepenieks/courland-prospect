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
// `pub(crate)` (not private) so the ingest server can propose snippets from a
// captured outgoing message. Turns a sent message + the pitch's existing snippets
// into `proposed` rows via the local Claude Code CLI.
pub(crate) mod proposals;
// The classify pass: on a snippet add/edit, one background LLM call places it on
// the conversation arc (`position`) and groups it (`category`). Sibling of
// `proposals`; both are triggered fire-and-forget and emit `SNIPPETS_CHANGED`.
pub(crate) mod classify;
// `pub(crate)` (not private) so the ingest server can read snippets as drafting
// material for `POST /draft` — mirrors the pitches slice.
pub(crate) mod repository;

/// Emitted (with the affected scope — a `pitch_id`, or `None` for the global
/// profile) whenever a background pass changes a scope's snippets: a new proposal
/// lands, or a classify pass updates a snippet's position/category. An open editor
/// for that scope reloads. Shared by `proposals` and `classify`.
pub(crate) const SNIPPETS_CHANGED: &str = "snippets://changed";
