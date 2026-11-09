//! The model interpreter (Milestone 2, step 3; docs/interpreter.md): runs
//! a game model directly, as a runtime of the runtime protocol, so a
//! hand-written model is playable and checkable before any engine code is
//! generated from it.
//!
//! - [`lifted`]: the lifted assets a model reads, and the bundle a
//!   recording stores.
//! - [`compile`]: a model and its assets into a `gp_sim::Spec`.
//! - [`runtime`]: [`Interp`], the `gp_runtime::Runtime` over a model.
//! - [`state`]: the state named by model paths.
//! - [`recording`]: recording and replaying model runs in the evidence
//!   store.
//! - [`modeltrace`]: the frame trace of a model run, a view of its state by
//!   model path.
//! - [`mechanics`]: the model's mechanic catalog, from the records the
//!   simulation's components and the model's hooks declare.
//! - [`reach`]: which places a level's player can stand on, and whether
//!   the goal is one, by the player's own moves (a research spike).
//! - [`difficulty`]: a level's difficulty from a searched run: each
//!   required input's frame window, scored per section (a research spike).
//! - [`mapping`] and [`compare`]: a state mapping from the original's RAM
//!   to model paths, and a frame-by-frame comparison of the original and
//!   the model through it.
//! - [`translate`]: the original's state as a model state, through the
//!   mapping read in reverse, with a note on each part; and [`lockstep`],
//!   both games on the same input measured apart on every mapped path, put
//!   back together at each split or left to drift.
//! - [`scenario`]: sequences, a few frames of presses run on both from the
//!   original's state at one frame, generated in tiers from the model's
//!   actions and mechanics or played by a person; each with its score,
//!   what it needs passed first, and a pass bar.
//! - [`bodies`]: the bodies other than the player paired across the two
//!   games for a lockstep, measured, and their turns and changes as events.
//! - [`screen`]: the two games' pictures compared frame by frame, as a
//!   background layer and a sprite layer, with the places they differ
//!   grouped into boxes.
//! - [`variant`]: the searches' edits of a model, as the spec edit and as
//!   the model text a game's module loads.
//! - [`reads`]: what a run read of the spec and which hooks it called, by
//!   name, as a clip's score keeps it.
//! - [`external`]: an external runtime's trace (one that can't load the
//!   original's state but starts at a level start and logs each frame,
//!   such as Super Mario Bros. Remastered under our probe), read through a
//!   trace map and stepped as a runtime, so the lockstep scores it as it
//!   scores the model.
//!
//! The simulation itself is `gp-sim`, which generated builds share.

pub mod baseline;
pub mod bodies;
pub mod carry;
pub mod compare;
pub mod compile;
pub mod corpus;
pub mod describe;
pub mod difficulty;
pub mod external;
pub mod lifted;
pub mod lockstep;
pub mod mapping;
pub mod mapping_features;
pub mod mechanics;
pub mod modeltrace;
pub mod moments;
pub mod reach;
pub mod reads;
pub mod recording;
pub mod runtime;
pub mod scenario;
pub mod screen;
pub mod stage;
pub mod starts;
pub mod state;
pub mod translate;
pub mod variant;

pub use lifted::{Bundle, Lifted};

#[cfg(test)]
mod tests;
pub use runtime::Interp;

/// The runtime's name in recordings.
pub const RUNTIME_NAME: &str = "gp-interp";

/// What the interpreter does, as a number. Any change to how it powers on,
/// steps or writes its canonical state bumps it, and recordings made before
/// replay as stale.
pub const BEHAVIOUR_VERSION: u32 = 1;
