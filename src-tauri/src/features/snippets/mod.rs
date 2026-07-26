//! Snippets feature — named text fragments that compose into messages. ONE
//! library, shared by every customer profile.
//!
//! Snippets used to be owned by a scope (a pitch, or the global profile), because
//! each pitch was a separate story with its own material. With a single product
//! there is a single body of material, and the model's job is precisely to pick
//! from all of it: given a customer profile's pain and goal, choose the lines that
//! move THIS thread toward THAT goal. Pre-scoping the library would make that
//! choice for it.
//!
//! Two axes still organize the library, and both are orthogonal to who the buyer
//! is: `position` (where a line sits on the conversation arc) and `category` (the
//! conversation stage it fits). Those say *when* in a thread a line belongs, not
//! *whom* it's for.
//!
//! Instance of the standard per-feature layout (see `features::customers`): the
//! same four files — `model`, `repository`, `commands`, `mod`. Register its
//! commands in `lib.rs`.

pub mod commands;
mod model;
// `pub(crate)` (not private) so the ingest server can propose snippets from a
// captured outgoing message. Turns a sent message + the existing library into
// `proposed` rows via the local Claude Code CLI.
pub(crate) mod proposals;
// The classify pass: on a snippet add/edit, one background LLM call places it on
// the conversation arc (`position`) and groups it (`category`). Sibling of
// `proposals`; both are triggered fire-and-forget and emit `SNIPPETS_CHANGED`.
pub(crate) mod classify;
// `pub(crate)` (not private) so the ingest server can read snippets as drafting
// material for `POST /draft` — mirrors the product slice.
pub(crate) mod repository;

/// Emitted whenever a background pass changes the library: a new proposal lands,
/// or a classify pass updates a snippet's position/category. An open editor
/// reloads. Carries no payload — there is one library, so there is nothing to
/// scope the event to. Shared by `proposals` and `classify`.
pub(crate) const SNIPPETS_CHANGED: &str = "snippets://changed";
