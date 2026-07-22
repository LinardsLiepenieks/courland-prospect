//! Comments feature — the in-app LinkedIn comment inbox. The app is the cockpit:
//! the Comments tab requests a scrape, reviews/edits the drafted comments, and
//! approves them for posting. The Chrome extension is a headless worker that polls
//! the app over the loopback ingest server (`crate::ingest`) — it scrapes the feed
//! + watchlist, asks the app to draft a comment per post, and auto-posts approved
//! drafts, paced.
//!
//! A vertical slice like `features::pitches`: `model` (data shape), `repository`
//! (all SQL), `commands` (the Tauri entry points the Comments tab calls). The
//! repository is `pub(crate)` because the ingest server drives the same data — the
//! extension's half of the flow.

pub mod commands;
pub mod model;
pub(crate) mod repository;

/// Tauri event emitted whenever the inbox or the run state changes — from a
/// command (the app) or from the ingest server (the extension). An open Comments
/// tab listens and re-fetches, so both halves of the flow stay reflected live.
pub(crate) const COMMENTS_CHANGED: &str = "comments://changed";
