//! Feature modules — one folder per concept, each a self-contained vertical
//! slice (model + repository + commands). To add a feature: create a sibling
//! folder here following the `pitches` shape and register its commands in
//! `lib.rs`. Shared infrastructure (connection, migrations) lives in
//! `crate::database`, not here.

pub mod messages;
pub mod pitches;
pub mod profile;
pub mod prospects;
pub mod selectors;
pub mod snippets;
pub mod stages;
