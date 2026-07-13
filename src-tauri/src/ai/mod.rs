//! AI infrastructure — the app's single path to the user's local Claude Code.
//!
//! This is cross-cutting infra (like `crate::database` and `crate::ingest`),
//! not a vertical feature slice: there's no data to persist, so there's no
//! repository. Any feature can build a structured `Prompt` and run it through
//! the `client`.
//!
//!  - `prompt` — a reusable `Prompt` (instruction + input) with named
//!               constructors per use (`Prompt::polish_skill`); `render`s to the
//!               single string handed to the CLI.
//!  - `client` — runs a `Prompt` through the local `claude` CLI (headless
//!               `-p` mode), reusing the user's own Claude Code install/auth.

pub mod client;
pub mod commands;
pub mod prompt;

pub use prompt::{DraftContext, DraftMessage, Prompt};
