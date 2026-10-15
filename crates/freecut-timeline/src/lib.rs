//! Pure edit model for FreeCut: tracks, clips, edit operations and undo/redo.
//!
//! This crate does no I/O and has no FFmpeg or GPU dependencies, so every
//! edit can be unit-tested in isolation.
//!
//! The flow of an edit: the app builds an [`edit::EditRequest`], asks the
//! [`Editor`] to [`plan`](Editor::plan) it (pure; the plan's preview can be
//! drawn as a ghost), then [`commit`](Editor::commit)s it, which records it in
//! the undo history.
#![forbid(unsafe_code)]

pub mod edit;
mod editor;
pub mod export;
pub mod history;
pub mod ids;
pub mod mix;
pub mod model;
pub mod motion;
pub mod plan;
mod probe;
pub mod select;
pub mod snap;
pub mod time;

/// The 2-D math types used in render plans.
pub use glam;

pub use editor::{CommitError, CommitMeta, Committed, Editor, HistoryError, Planner, Restored};

#[cfg(test)]
mod regressions;
