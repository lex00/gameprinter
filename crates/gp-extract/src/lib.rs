//! Agent extraction (docs/mechanics.md): from a
//! recording's tracks to model facts.
//!
//! - [`feel`]: the player's walk and run speeds, acceleration, jump arc
//!   (speed, gravity, held gravity, fall limit, rise by hold time) and
//!   enemies' steps, periods and speeds, measured from `gp track`'s
//!   tracks and the controller.
//! - [`propose`]: the measurements as proposed facts by model path, with
//!   the model's stated facts as accuracy, and questions where they can't
//!   decide.
//! - [`questions`]: the questions file a working copy keeps.
//! - [`apply`]: facts written into a model with their evidence.
//!
//! The refinement loop that runs these with validation is `gp model
//! refine` (crates/gp-cli/src/refine/).

pub mod apply;
pub mod feel;
pub mod propose;
pub mod questions;
