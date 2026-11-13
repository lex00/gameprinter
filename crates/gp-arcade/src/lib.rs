//! Arcade boards, emulated to match MAME frame by frame. The first is
//! Midway's MCR with the 91490 CPU board, as in Tapper (MAME's `rbtapper`).
//!
//! # Why a port of MAME's own code
//!
//! The oracle (docs/oracle-mame.md) records every frame's RAM and picture
//! from MAME 0.276. To match it on every frame, this crate does what MAME
//! does, in MAME's order, down to the cycle:
//!
//! - The Z80 is MAME's (z80.lst, turned into Rust by tools/gen_z80.py).
//!   MAME stops a CPU between the memory cycles of one instruction when its
//!   timeslice runs out, and a frame can end there. No Rust Z80 crate (`z80`,
//!   `iz80`) stops mid-instruction, so either would write some bytes a frame
//!   early. The generated core is pinned to MAME 0.276's z80.lst, copied
//!   into mame/.
//! - The scheduler follows MAME's: timeslices that end at the next timer,
//!   CPUs that overshoot by the rest of a step, timers set "now" when one
//!   CPU talks to another or raises an interrupt.
//! - The CTC, the SSIO board, the screen timing and the video are ports of
//!   mcr.cpp, mcr_m.cpp, mcr_v.cpp, midway.cpp, z80ctc.cpp and screen.cpp.
//!
//! - The sound (sound.rs) is MAME's AY-8910 and the SSIO's volumes, made
//!   in MAME's 50 Hz stream updates and resampled to 48 kHz as MAME does.
//!
//! Every MAME file ported is BSD-3-Clause; each Rust file names the files
//! it comes from and keeps their copyright holders.
//!
//! # Using it
//!
//! ```no_run
//! # use gp_arcade::{Controls, Romset, Tapper};
//! let rom = Romset::load_zip("rbtapper.zip".as_ref()).unwrap();
//! let mut t = Tapper::power_on(&rom);
//! t.step(Controls { coin: true, ..Controls::default() });
//! let ram = t.ram_record(); // the oracle's 5,760-byte layout
//! let snap = t.snapshot();
//! t.restore(&snap).unwrap();
//! # let _ = (ram, t.picture_hash(), t.state_hash());
//! ```
//!
//! [`Tapper`] has what gp_runtime::Runtime needs, in the shape gp-nes's
//! `Nes` gives it to `NesRuntime`: `step` runs one frame (one 30 Hz video
//! frame, ending where MAME's does, at the start of VBLANK) with a frame's
//! controls; `frame`, `peek` and `ram_record` read it; `snapshot`,
//! `restore` and `state_hash` cover the whole machine, the CPUs stopped
//! mid-instruction and the pending timers included. An `InputFrame`'s
//! `buttons` maps to [`Controls::from_bits`]: bits 0 to 6 are coin, start,
//! right, left, down, up and tap, the oracle log's columns. The runtime
//! wrapper is [`ArcadeRuntime`] ([`protocol`], with its selectors).
//!
//! examples/compare.rs plays an oracle capture's input log and checks
//! every frame against MAME; tests/rbtapper.rs does the same in
//! `cargo test` when the romset and captures are on the machine.
//!
//! # The crate's name
//!
//! gp-arcade rather than gp-mcr: the scheduler, the Z80, the CTC and the
//! romset reader serve any Z80 arcade board, and the next boards will want
//! them. The MCR board itself is one module, `mcr`.

pub mod attotime;
pub mod autoplay;
pub mod canonical;
pub mod ctc;
pub mod dasm;
pub mod events;
pub mod harvest;
pub mod inp;
pub mod irq;
pub mod joystick;
pub mod levels;
pub mod mcr;
pub mod oracle;
pub mod protocol;
pub mod romset;
pub mod sched;
pub mod sha1;
pub mod sound;
pub mod state;
pub mod testrom;
pub mod video;
pub mod z80;

pub use joystick::FourWay;
pub use mcr::{Controls, Tapper};

/// What made a snapshot: a start cache keys its snapshots by it, so a
/// change to the board's emulation or its snapshot bumps it, by hand.
/// mcr 2: the AYs make sound, and their generators are in the snapshot.
/// mcr 3: the watchdog resets the board; the tilt switch.
pub const RUNTIME_VERSION: &str = concat!("gp-arcade ", env!("CARGO_PKG_VERSION"), " mcr 3");
pub use protocol::ArcadeRuntime;
pub use romset::Romset;
