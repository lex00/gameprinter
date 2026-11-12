//! L3, detail: never kept, made again on request by running a recipe to a
//! window and tracing every instruction in it, in the layout of
//! `gp trace instructions` (the same stepping loop,
//! `gp_nes::instructions::trace_frames`).

use std::io::Write;

use gp_evidence::input_log::InputFrame;
use gp_evidence::manifest::PowerOn;
use gp_nes::Nes;
use gp_nes::instructions::{TraceError, Traced, trace_frames};

/// Runs `rom` from power-on through `inputs` and traces frames `first` to
/// `last` (frame 1 is the first frame run), writing the lines to `out`.
pub fn window(
    rom: &[u8],
    power_on: PowerOn,
    inputs: &[InputFrame],
    first: u64,
    last: u64,
    out: &mut impl Write,
) -> Result<Traced, TraceError> {
    let mut nes = Nes::power_on(rom, power_on).map_err(gp_nes::seek::SeekError::from)?;
    let first = first.max(1);
    let last = last.min(inputs.len() as u64);
    for i in &inputs[..usize::try_from(first - 1)
        .unwrap_or(usize::MAX)
        .min(inputs.len())]
    {
        nes.step(*i).map_err(gp_nes::seek::SeekError::from)?;
    }
    let from = usize::try_from(first - 1)
        .unwrap_or(usize::MAX)
        .min(inputs.len());
    let to = usize::try_from(last)
        .unwrap_or(usize::MAX)
        .min(inputs.len())
        .max(from);
    trace_frames(&mut nes, &inputs[from..to], out)
}
