//! Agent extraction (docs/mechanics.md in game-printer): from a
//! recording's tracks to model facts.
//!
//! - [`propose`]: the measurements as proposed facts by model path. Only
//!   its path steps are here yet; the measuring (`feel`) and the proposals
//!   come with gp-track in a later wave of the port.
//! - [`questions`]: the questions file a working copy keeps.
//! - [`apply`]: facts written into a model with their evidence.
//!
//! The refinement loop that runs these is `gp model refine --lockstep`
//! (crates/gp-cli/src/refine/).

pub mod apply;
pub mod propose;
pub mod questions;
