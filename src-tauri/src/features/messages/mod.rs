//! Messages feature — LinkedIn messages captured for a prospect in both
//! directions, the source of truth behind a prospect's derived `messages_sent`
//! counter (outgoing) and durable `responded` flag (incoming). This is the one
//! centralized writer of extension-derived prospect state.
//!
//! Unlike the reference `pitches` slice, this one is **write-only for now**: rows
//! are written by the Chrome extension over the loopback ingest server
//! (`crate::ingest`, which calls `repository` directly — hence `pub(crate)`), and
//! nothing in the desktop UI reads them yet. So there is deliberately no `model`
//! or `commands` file: a `Message` read-model + a `list_messages` command will
//! land here alongside the thread-viewer UI, when that ships.

pub(crate) mod repository;
