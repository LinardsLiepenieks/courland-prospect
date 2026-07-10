//! Feature modules — one folder per concept, each a self-contained vertical
//! slice (model + repository + commands). To add a feature: create a sibling
//! folder here following the `pitches` shape and register its commands in
//! `lib.rs`. Shared infrastructure (connection, migrations) lives in
//! `crate::database`, not here.

pub mod pitches;
