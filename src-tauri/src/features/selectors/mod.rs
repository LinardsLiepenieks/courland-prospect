//! Selectors feature — persisted overrides for the Chrome extension's LinkedIn
//! DOM selectors (the self-heal store).
//!
//! Like `messages`, this is an **ingest-driven** slice: the loopback server
//! (`crate::ingest`) reads it for `GET /selectors` and writes it from
//! `POST /heal-selectors`, and nothing in the desktop UI touches it — so there's
//! deliberately no `model` or `commands`, just a `repository` (hence
//! `pub(crate)`). It's a singleton (a single JSON blob of `{key: value}`
//! overrides pinned at id = 1), mirroring the `profile` shape.
pub(crate) mod repository;
