//! The original's runtime, by platform: gp-nes for an NES image, gp-arcade
//! for an arcade romset. Every place that starts the original (a lockstep,
//! a clip's score, the start cache, a scenario, `gp compare`, the mechanic
//! page's clip starts) gets it here, from the package's platform
//! ([`crate::package::Package::platform`]) or, where only the ROM file is
//! at hand, from the file's own form ([`Platform::of_file`]): a zip is a
//! romset, anything else an NES image. The two always agree, since a
//! romset's identity can only come from a zip.
//!
//! What only the NES has (random draws read from the 6502's code, the
//! 6502 analysis tools) asks [`OriginalRuntime::nes`] and refuses an
//! arcade original in words.

use anyhow::{Result, bail};
use gp_arcade::ArcadeRuntime;
use gp_evidence::manifest::PowerOn;
use gp_nes::protocol::NesRuntime;
use gp_runtime::{
    Capabilities, Entry, Event, InputFrame, LoadNote, PatchResult, Rect, Runtime, Until, Value,
};

use crate::clip::cache::Saved;
pub(crate) use crate::roms::Platform;

/// The original game running, on whichever emulator its platform needs.
/// Both variants are a few KB, held one or two at a time, so neither is
/// boxed.
#[derive(Clone)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum OriginalRuntime {
    Nes(NesRuntime),
    Arcade(ArcadeRuntime),
}

impl OriginalRuntime {
    /// Powers on `rom` (an NES image's or a romset zip's bytes) on the
    /// platform's emulator, as the tools do: NES RAM zeros, NTSC.
    pub(crate) fn load(platform: Platform, rom: &[u8]) -> Result<Self> {
        Ok(match platform {
            Platform::Nes => Self::Nes(
                NesRuntime::load(rom, PowerOn::default()).map_err(|e| anyhow::anyhow!("{e}"))?,
            ),
            Platform::Arcade => {
                if !gp_arcade::romset::looks_like_zip(rom) {
                    bail!("an arcade original is a romset zip, and this ROM isn't one");
                }
                Self::Arcade(ArcadeRuntime::load_zip(rom).map_err(anyhow::Error::msg)?)
            }
        })
    }

    /// [`Self::load`] on the platform the file's form says.
    pub(crate) fn of_rom(rom: &[u8]) -> Result<Self> {
        Self::load(Platform::of_file(rom), rom)
    }

    /// Powered on and restored to a start the cache kept, video off as
    /// after power-on.
    pub(crate) fn from_saved(rom: &[u8], saved: &Saved) -> Result<Self> {
        Ok(match Platform::of_file(rom) {
            Platform::Nes => Self::Nes(
                NesRuntime::new(saved.nes(rom, PowerOn::default())?).map_err(anyhow::Error::msg)?,
            ),
            Platform::Arcade => {
                let mut o = ArcadeRuntime::load_zip(rom).map_err(anyhow::Error::msg)?;
                o.restore(&saved.snap).map_err(anyhow::Error::msg)?;
                Self::Arcade(o)
            }
        })
    }

    /// Its state as the start cache keeps it.
    pub(crate) fn saved(&self) -> Result<Saved> {
        match self {
            Self::Nes(n) => Saved::of(n.nes()),
            Self::Arcade(a) => Ok(Saved {
                snap: a.snapshot(),
                frame: a.frame(),
                held: [0; 4],
            }),
        }
    }

    pub(crate) fn platform(&self) -> Platform {
        match self {
            Self::Nes(_) => Platform::Nes,
            Self::Arcade(_) => Platform::Arcade,
        }
    }

    /// The NES runtime, for what only the NES has.
    pub(crate) fn nes(&self) -> Option<&NesRuntime> {
        match self {
            Self::Nes(n) => Some(n),
            Self::Arcade(_) => None,
        }
    }

    /// The canonical state's whole hash in hex, as a clip's `state` field
    /// and a start's check keep it: gp-nes's recordings' hash, or the
    /// BLAKE3 of the arcade machine's snapshot.
    pub(crate) fn state_hex(&self) -> Result<String> {
        Ok(match self {
            Self::Nes(n) => n.nes().state_hash()?.to_hex(),
            Self::Arcade(a) => gp_evidence::Hash::of(&a.canonical_state()).to_hex(),
        })
    }

    /// Turns the drawing of its frames on or off. The arcade board always
    /// draws: its picture is part of its state.
    pub(crate) fn set_video(&mut self, on: bool) {
        if let Self::Nes(n) = self {
            n.set_video(on);
        }
    }
}

/// Refuses an arcade original for what only the NES has, naming it.
pub(crate) fn nes_only(platform: Platform, what: &str) -> Result<()> {
    if platform == Platform::Arcade {
        bail!("{what} is read from the NES's 6502 code: gp has no arcade counterpart yet");
    }
    Ok(())
}

macro_rules! each {
    ($self:expr, $o:ident => $e:expr) => {
        match $self {
            OriginalRuntime::Nes($o) => $e,
            OriginalRuntime::Arcade($o) => $e,
        }
    };
}

impl Runtime for OriginalRuntime {
    fn capabilities(&self) -> Capabilities {
        each!(self, o => o.capabilities())
    }
    fn step(&mut self, inputs: &[InputFrame]) -> Result<Vec<Event>, String> {
        each!(self, o => o.step(inputs))
    }
    fn run_until(
        &mut self,
        condition: &str,
        inputs: &[InputFrame],
        max_frames: u32,
    ) -> Result<Until, String> {
        each!(self, o => o.run_until(condition, inputs, max_frames))
    }
    fn frame(&self) -> u64 {
        each!(self, o => o.frame())
    }
    fn snapshot(&self) -> Vec<u8> {
        each!(self, o => o.snapshot())
    }
    fn restore(&mut self, snap: &[u8]) -> Result<(), String> {
        each!(self, o => o.restore(snap))
    }
    fn fork(&self) -> Self {
        self.clone()
    }
    fn canonical_state(&self) -> Vec<u8> {
        each!(self, o => o.canonical_state())
    }
    fn state_hash(&self) -> u64 {
        each!(self, o => o.state_hash())
    }
    fn load_state(&mut self, state: &[u8]) -> Result<Vec<LoadNote>, String> {
        each!(self, o => o.load_state(state))
    }
    fn query(&self, selector: &str) -> Result<Vec<Entry>, String> {
        each!(self, o => o.query(selector))
    }
    fn sample(
        &mut self,
        selectors: &[String],
        inputs: &[InputFrame],
        frames: u32,
    ) -> Result<Vec<Vec<Entry>>, String> {
        each!(self, o => o.sample(selectors, inputs, frames))
    }
    fn entities_at(&self, area: Rect) -> Vec<u64> {
        each!(self, o => o.entities_at(area))
    }
    fn patch(&mut self, model_diff: &str) -> PatchResult {
        each!(self, o => o.patch(model_diff))
    }
    fn set(&mut self, p: &str, v: Value) -> Result<(), String> {
        each!(self, o => o.set(p, v))
    }
    fn explain(&self, p: &str) -> Vec<String> {
        each!(self, o => o.explain(p))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The factory picks gp-nes for an NES image, and refuses a non-zip as
    /// an arcade original rather than guessing.
    #[test]
    fn the_platform_picks_the_emulator() {
        let rom = gp_nes::testrom::build();
        assert_eq!(Platform::of_file(&rom), Platform::Nes);
        let o = OriginalRuntime::of_rom(&rom).unwrap();
        assert_eq!(o.platform(), Platform::Nes);
        assert!(o.nes().is_some());
        let e = OriginalRuntime::load(Platform::Arcade, &rom).err().unwrap();
        assert!(e.to_string().contains("romset zip"), "{e}");
        assert!(nes_only(Platform::Arcade, "random draws").is_err());
        assert!(nes_only(Platform::Nes, "random draws").is_ok());
    }

    /// With the owner's rbtapper romset on this machine: the arcade
    /// original steps, reads `ram[...]` and the sound RAM, and a start
    /// saved for the cache restores to the same state. Skipped without it.
    #[test]
    fn an_arcade_original_steps_and_restores_from_a_start() {
        let Some(zip) = gp_arcade::romset::local_romset("rbtapper") else {
            eprintln!("skipped: no rbtapper romset");
            return;
        };
        let rom = std::fs::read(zip).unwrap();
        let mut o = OriginalRuntime::of_rom(&rom).unwrap();
        assert_eq!(o.platform(), Platform::Arcade);
        o.step(&vec![InputFrame::default(); 30]).unwrap();
        assert_eq!(o.query("ram[0xe000..0xe010]").unwrap().len(), 16);
        assert_eq!(o.query("sound_ram[0..4]").unwrap().len(), 4);
        let saved = o.saved().unwrap();
        let back = OriginalRuntime::from_saved(&rom, &saved).unwrap();
        assert_eq!(back.state_hex().unwrap(), o.state_hex().unwrap());
        assert_eq!(back.frame(), 30);
    }
}
