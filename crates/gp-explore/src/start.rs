//! A start: a recipe's prefix run once, kept as a snapshot, so the search
//! runs each candidate from it instead of from power-on. A candidate found
//! from a start is still a recipe: the prefix's presses, then its own.

use gp_events::{Collector, Layout, Levels, RunError};
use gp_evidence::input_log::InputFrame;
use gp_evidence::manifest::PowerOn;
use gp_nes::Nes;

/// A prefix and the console's state at its end.
#[derive(Debug, Clone)]
pub struct Start {
    pub power_on: PowerOn,
    /// The prefix's input, from power-on.
    pub prefix: Vec<InputFrame>,
    snapshot: Vec<u8>,
    frame: u64,
    held: [u8; 4],
    /// The return addresses calls in the prefix left pending, so a run from
    /// the snapshot tells their returns from jumps.
    returns: [Option<u16>; 256],
}

impl Start {
    /// Runs `prefix` from power-on and keeps the state at its end.
    pub fn of(rom: &[u8], power_on: PowerOn, prefix: &[InputFrame]) -> Result<Start, RunError> {
        let mut nes = Nes::power_on(rom, power_on)?;
        let mut c = Collector::new(Layout::of(nes.bus())?, true);
        c.start(nes.bus_mut_for_observers());
        c.attach(nes.bus_mut_for_observers());
        for i in prefix {
            c.step(&mut nes, *i)?;
        }
        Ok(Start {
            power_on,
            prefix: prefix.to_vec(),
            snapshot: nes.snapshot()?,
            frame: nes.frame(),
            held: nes.held(),
            returns: c.edges.as_ref().map_or([None; 256], |e| e.returns()),
        })
    }
}

/// Runs `pads` (one controller byte a frame) from `start`, watched by a
/// collector of L0 and L1.
pub fn run_from(rom: &[u8], start: &Start, pads: &[u8]) -> Result<Levels, RunError> {
    let mut nes = Nes::power_on(rom, start.power_on)?;
    nes.restore(&start.snapshot, start.frame, start.held)?;
    let layout = Layout::of(nes.bus())?;
    let mut c = Collector::new(layout, true);
    if let Some(e) = &mut c.edges {
        e.set_returns(start.returns);
    }
    c.start(nes.bus_mut_for_observers());
    c.attach(nes.bus_mut_for_observers());
    for i in crate::presses::log_frames(pads) {
        c.step(&mut nes, i)?;
    }
    Ok(Levels::of(&c, pads.len() as u64))
}
