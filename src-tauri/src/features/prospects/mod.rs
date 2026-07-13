//! Prospects feature — people captured from LinkedIn into the pipeline.
//!
//! Same four-file slice shape as `pitches`. Note that prospect *creation* is not
//! a Tauri command: the Chrome extension writes prospects over the loopback
//! ingest server (`crate::ingest`), which calls `repository::upsert` directly.
//! That's why `repository`'s `list`/`upsert` are `pub(crate)`.

pub mod commands;
mod model;
pub(crate) mod repository;
