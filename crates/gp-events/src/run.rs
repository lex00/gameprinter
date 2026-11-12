//! Running a ROM from power-on under input frames, with a collector.

use gp_evidence::input_log::InputFrame;
use gp_evidence::manifest::PowerOn;
use gp_nes::{Nes, NesError};

use crate::collector::Collector;
use crate::layout::{Layout, LayoutError};

#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error(transparent)]
    Nes(#[from] NesError),
    #[error(transparent)]
    Layout(#[from] LayoutError),
    /// A console handed in stands past the window's start.
    #[error("the console stands after frame {at}, past the window's first frame {first}")]
    Start { at: usize, first: u64 },
}

/// A finished run: its levels and how many frames it ran.
#[derive(Debug, Clone)]
pub struct Run {
    pub collector: Collector,
    pub frames: u64,
}

/// Runs `rom` from power-on through `inputs`, one frame each, watched by a
/// collector of L0 and, when `edges`, L1.
pub fn run(
    rom: &[u8],
    power_on: PowerOn,
    inputs: &[InputFrame],
    edges: bool,
) -> Result<Run, RunError> {
    let mut nes = Nes::power_on(rom, power_on)?;
    let layout = Layout::of(nes.bus())?;
    let mut c = Collector::new(layout, edges);
    c.start(nes.bus_mut_for_observers());
    c.attach(nes.bus_mut_for_observers());
    for i in inputs {
        let input = InputFrame {
            polled: true,
            pads: i.pads,
            reset: i.reset,
        };
        c.step(&mut nes, input)?;
    }
    Ok(Run {
        collector: c,
        frames: inputs.len() as u64,
    })
}
