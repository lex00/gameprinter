//! gp-events: what a ROM does when it runs, kept as levels
//! (docs/research/rom-event-model.md).
//!
//! A run is its recipe: the ROM, a start and the presses. gp-nes is
//! deterministic, so what a run does is a function of the recipe and of how
//! closely it is watched, and nothing has to be kept at full detail. The
//! levels built so far:
//!
//! | Level | Module | What |
//! |---|---|---|
//! | L0, footprints | [`footprint`] | PRG bytes executed and instruction starts, by PRG offset (so by bank); RAM and PRG RAM bytes written |
//! | L1, edges | [`edges`] | Every transfer of control, by PRG offset, with its count, first frame and, for `jmp (ptr)`, the pointer's values |
//! | L2, frame events | [`frames`] | Per frame: the console's event log, code and edges first seen, RAM bytes changed, writes to watched addresses |
//! | L2's code activity | [`activity`] | Per frame: routines entered with their call counts, every RAM write with its writer and routine, VRAM and APU writes by writer, calls to watched routines (`gp gaps`' code signals) |
//! | L3, detail | [`detail`] | Never kept: a window of a recipe run again and traced instruction by instruction |
//! | L3's counts | [`profile`] | A window's count of each instruction and the ways each branch went, kept by the store |
//!
//! [`Collector`] watches a console through the cartridge shim in the fork
//! (`tetanes_core::shim`), or, when an observer needs every instruction,
//! through [`gp_nes::Nes::step_with`] (a host observer), with the same
//! levels either way; it changes nothing but `Bus::interrupt_taken`, so a
//! run watched by it reaches the same states as one that isn't. [`run`] runs a
//! ROM from power-on under input frames with a collector.

pub mod activity;
pub mod bits;
pub mod collector;
pub mod detail;
pub mod edges;
pub mod footprint;
pub mod frames;
pub mod layout;
pub mod levels;
pub mod names;
pub mod profile;
pub mod run;
pub mod store;
pub mod watch;

pub use bits::Bits;
pub use collector::Collector;
pub use edges::{EdgeKind, EdgeStat, Edges, NO_ROM};
pub use footprint::Footprint;
pub use frames::{FrameEvents, Hooked, Watched, run_frames, run_frames_with};
pub use layout::{Layout, LayoutError, RAM_BYTES};
pub use levels::{EdgeRow, Levels};
pub use run::{Run, RunError, run};

/// The version of what a collector records. A change to what any level
/// holds bumps it, and derived data keyed by it is made again.
pub const COLLECTOR_VERSION: u32 = 2;
