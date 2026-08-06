//! AI infrastructure — the app's single path to the user's local Claude Code.
//!
//! This is cross-cutting infra (like `crate::database` and `crate::ingest`),
//! not a vertical feature slice: there's no data to persist, so there's no
//! repository. Any feature can build a structured `Prompt` and run it through
//! the `client`.
//!
//!  - `prompt` — a reusable `Prompt` (instruction + input) with named
//!               constructors per use (`Prompt::polish_product`); `render`s to
//!               the single string handed to the CLI.
//!  - `client` — runs a `Prompt` through the local `claude` CLI (headless
//!               `-p` mode), reusing the user's own Claude Code install/auth.
//!  - `parse`  — the other end: locates the JSON in a reply before a feature
//!               reads its fields. Shared because every machine-parsed prompt
//!               faces the same preamble/fences problem.

pub mod client;
pub mod commands;
// `pub(crate)` (not public): finding the JSON in a reply is infrastructure for the
// features that parse one, not part of the AI module's outward surface.
pub(crate) mod parse;
pub mod prompt;

pub use prompt::{
    comment_is_skip, AdvanceContext, BrokenSelector, ClassifyContext, CommentContext, DedupContext,
    DraftContext, DraftCustomer, DraftMessage, DraftSnippet, DraftStage, Prompt, ProposeContext,
    ReviewContext,
};
