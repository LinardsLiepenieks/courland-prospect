//! Stages feature — THE pipeline: one ordered funnel every prospect moves
//! through. Standard per-feature layout (model / repository / commands).
//!
//! Stages used to be owned per-pitch, so each pitch seeded and carried its own
//! funnel. With a single product there is a single process, and which customer
//! profile a prospect matches is orthogonal to how far along they are — the
//! board is the process, the customer profile is the steering. Migration 0024
//! collapsed the per-pitch pipelines into this one and guarantees it is never
//! empty, so nothing here ever creates a pipeline from scratch.
//!
//! `repository` is `pub(crate)` because sibling features read the pipeline in
//! their tests, and `model` follows it: those functions return `Stage`, so the
//! type has to stay nameable crate-wide. `KIND_MESSAGING` itself never leaves
//! this feature — the one cross-feature reader (the prospects repository's
//! messaging-stage lookup) inlines the `'messaging'` literal in SQL.

pub mod commands;
pub(crate) mod model;
pub(crate) mod repository;
