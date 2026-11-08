//! The deterministic simulation that every model runtime shares
//! (docs/interpreter.md). The interpreter (`gp-interp`) compiles a model
//! into a [`Spec`] and steps a [`World`] with [`step`]; a generated build
//! does the same with the same functions, so the two agree frame for frame
//! and their canonical state hashes match.
//!
//! - [`fx`]: fixed-point numbers, the only numbers game logic reads.
//! - [`rng`]: the seeded random number generator kept in the world.
//! - [`spec`]: a game as the simulation runs it.
//! - [`world`]: the state that changes, with its canonical bytes.
//! - [`systems`]: one function per system, in [`Spec::order`], and [`step`].
//! - [`springboard`]: the springboard component, which holds and throws
//!   a player.
//! - [`draws`]: a random draws script, the original's draws a lockstep
//!   rebuild takes in place of its generator's.
//! - [`ext`]: the `ext` boundary, hand-written hooks as an [`ext::Ext`] trait.
//! - [`mechanic`]: mechanic records, what each component and hook is, as data.
//! - [`save`]: the save record a player's save slot holds.
//! - [`reads`]: what a run reads of its spec, noted for the scorer.
//!
//! Determinism: a fixed timestep (one call to [`step`] per frame), the
//! random number generator's state in the world, no clock, no floats, no
//! hash-map iteration, and systems in an explicit order.

pub mod draws;
pub mod ext;
pub mod fx;
pub mod layout;
pub mod mechanic;
pub mod reads;
pub mod rng;
pub mod save;
pub mod spec;
pub mod springboard;
pub mod systems;
pub mod world;

pub use draws::DrawScript;
pub use ext::{Ext, ExtCx, Exts};
pub use fx::Fx;
pub use spec::Spec;
pub use systems::{Event, step, step_pads, step_with_draws, step_with_ext};
pub use world::World;
