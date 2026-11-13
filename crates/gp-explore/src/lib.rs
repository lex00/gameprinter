//! gp-explore: finding presses that make a ROM do something new
//! (docs/research/rom-event-model.md, section 8).
//!
//! | Module | What |
//! |---|---|
//! | [`presses`] | Press sequences, the seeded generator, mutations, spans |
//! | [`arcade`] | The arcade board's chord palette and starts (coin, start, the stick, tap) |
//! | [`start`] | A start: a recipe prefix run once and kept as a snapshot, and runs from it watched by gp-events |
//! | [`graph`] | The static harvest's routines and calls as a graph over PRG offsets, for distances |
//! | [`search`] | The one search loop ([`drive`]) every search in gp runs on, with each search's [`Plan`]; `gp explore`'s plan, scored by a plug-in |
//!
//! The scores here read only what a run did (gp-events' levels) and the
//! code map, so they need no model. Scores that read a model's measures
//! live with the clip tools and plug in the same way.

pub mod arcade;
pub mod graph;
pub mod presses;
pub mod search;
pub mod start;

pub use graph::CallGraph;
pub use search::{
    Drive, Found, NewCode, Opts, Plan, Progress, Score, Toward, drive, drive_with, search,
    search_watched,
};
pub use start::{Start, run_from};
