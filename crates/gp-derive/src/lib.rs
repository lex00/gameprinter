//! Deriving a model's hand-written facts from the game's own source, its
//! data and its recordings (Milestone 2, step 7; docs/derivation.md).
//!
//! - [`asm`] reads 6502 assembly source in the dialects the pinned
//!   disassemblies use: labels, constants, data rows, instructions.
//! - [`eval`] propagates constants through a routine, so a value the code
//!   computes before a cited line (a store, a table load) can be read.
//! - [`encode`] reads a raw number as a model value in the encodings games
//!   use (sixteenths, 256ths, signed bytes, 8.8 words).
//! - [`derive`] matches each fact against what its evidence cites and
//!   scores a model.

pub mod asm;
pub mod derive;
pub mod encode;
pub mod eval;
