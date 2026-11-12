//! An arcade original played live in the mechanic page: gp-play's
//! [`Player`] for gp-arcade.
//!
//! [`Player`] runs Tapper a frame at a time with the controls a person
//! holds, keeps what they played as an arcade input log (gp-evidence's
//! `InputLog` in the oracle's spelling, `gp-arcade-input 0`) and every
//! frame's picture hash, and draws each frame as RGBA for a canvas. Built
//! for `wasm32-unknown-unknown`, the [`abi`] module exports it to
//! JavaScript with the same plain functions as gp-play, plus
//! `gp_play_info` (the screen, the frame rate and the buttons) and
//! `gp_play_pictures` (every frame's picture hash).
//!
//! As in gp-play, nothing reads a clock: one [`Player::step`] is one
//! frame (one 30 Hz video frame), so the log replays natively to the same
//! frames. `just play-web-arcade` checks that: a capture's input played in
//! the wasm build and natively must give the same picture hash on every
//! frame.
//!
//! The board makes no sound yet, so [`Player::audio`] is always empty.

use gp_arcade::video::{HEIGHT, WIDTH};
use gp_arcade::{Controls, Romset, Tapper};
use gp_evidence::Hash;
use gp_evidence::input_log::{InputFrame, InputLog};

/// Leads a player's saved state ([`Player::save`]), then a version byte.
const SAVE_MAGIC: &[u8; 4] = b"GPAP";
const SAVE_VERSION: u8 = 1;

/// The button columns of the log, one letter each from bit 0: coin, start,
/// right, left, down, up, tap.
pub const COLUMNS: &str = "CSRLDUT";

/// Bytes of one RGBA frame.
pub const RGBA_BYTES: usize = WIDTH * HEIGHT * 4;

/// Samples a second of [`Player::audio`], which is empty for now.
pub const AUDIO_RATE: u32 = 48_000;

/// The screen, the frame rate and the buttons, as `gp_play_info` gives
/// them to the page: JSON.
pub fn info(system: &str) -> String {
    format!(
        "{{\"platform\":\"arcade\",\"system\":\"{system}\",\"width\":{WIDTH},\"height\":{HEIGHT},\"fps\":[30,1],\"columns\":\"{COLUMNS}\",\"buttons\":[\"coin\",\"start1\",\"right\",\"left\",\"down\",\"up\",\"button1\"],\"sound\":false}}"
    )
}

fn text<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

/// One machine being played, and what was played on it.
pub struct Player {
    tapper: Tapper,
    /// The frames played so far; `frames.len()` is the machine's frame.
    log: InputLog,
    /// Every frame's picture hash from frame 0 (the oracle's).
    pictures: Vec<u64>,
    /// A log being replayed, whose frames [`Player::advance`] plays.
    script: Option<InputLog>,
    rgba: Vec<u8>,
}

impl Player {
    /// Powers on the set in a MAME zip.
    pub fn new(zip: &[u8]) -> Result<Self, String> {
        let rom = Romset::from_zip(zip).map_err(text)?;
        let tapper = Tapper::power_on(&rom);
        let pictures = vec![tapper.picture_hash()];
        Ok(Self {
            tapper,
            log: InputLog::new_arcade(rom.name, COLUMNS),
            pictures,
            script: None,
            rgba: vec![0; RGBA_BYTES],
        })
    }

    /// Powers on to replay `log`, an arcade log of the same set.
    pub fn replay(zip: &[u8], log: &str) -> Result<Self, String> {
        let script = InputLog::parse(log).map_err(text)?;
        let mut p = Self::new(zip)?;
        match &script.arcade {
            Some(a) if Some(a) == p.log.arcade.as_ref() => {}
            Some(a) => {
                return Err(format!(
                    "the log is of {} with columns {}; this is {} with {COLUMNS}",
                    a.system,
                    a.columns,
                    p.system()
                ));
            }
            None => return Err("an NES input log can't play on an arcade board".into()),
        }
        p.script = Some(script);
        Ok(p)
    }

    /// The MAME set being played.
    pub fn system(&self) -> &str {
        self.log.arcade.as_ref().map_or("", |a| a.system.as_str())
    }

    /// Kept for gp-play's shape: the board has no sound to turn on yet.
    pub fn set_audio(&mut self, _on: bool) {}

    /// The last frame's sound: none yet.
    pub fn audio(&self) -> &[f32] {
        &[]
    }

    /// Runs one frame with these controls (bits 0 to 6: coin, start,
    /// right, left, down, up, tap). Returns whether the game read them.
    pub fn step(&mut self, bits: u32, reset: bool) -> Result<bool, String> {
        if reset {
            return Err("the arcade board's reset isn't modelled".into());
        }
        if bits > 0x7f {
            return Err(format!("buttons {bits:#x}: bits 0 to 6 only"));
        }
        self.tapper.step(Controls::from_bits(bits));
        self.log.frames.push(InputFrame {
            polled: true,
            pads: [bits as u8, 0, 0, 0],
            reset: false,
        });
        self.pictures.push(self.tapper.picture_hash());
        Ok(self.tapper.polled())
    }

    /// Plays the next frame of the log being replayed. False when there's
    /// none left (or no log).
    pub fn advance(&mut self) -> Result<bool, String> {
        let i = self.log.frames.len();
        let Some(f) = self.script.as_ref().and_then(|s| s.frames.get(i)).copied() else {
            return Ok(false);
        };
        self.step(u32::from(f.pads[0]), false)?;
        Ok(true)
    }

    /// Plays the replayed log up to `frame`. Stops early at the log's end.
    pub fn run_to(&mut self, frame: u64) -> Result<u64, String> {
        while self.frame() < frame {
            if !self.advance()? {
                break;
            }
        }
        Ok(self.frame())
    }

    /// The player's whole state as bytes: the machine's snapshot, the
    /// input log and every frame's picture hash.
    pub fn save(&self) -> Result<Vec<u8>, String> {
        let snap = self.tapper.snapshot();
        let log = self.log.to_text();
        let pictures: Vec<u8> = self.pictures.iter().flat_map(|h| h.to_le_bytes()).collect();
        let mut out = Vec::with_capacity(32 + snap.len() + log.len() + pictures.len());
        out.extend_from_slice(SAVE_MAGIC);
        out.push(SAVE_VERSION);
        out.extend_from_slice(&self.frame().to_le_bytes());
        for part in [&snap[..], log.as_bytes(), &pictures] {
            out.extend_from_slice(&(part.len() as u64).to_le_bytes());
            out.extend_from_slice(part);
        }
        Ok(out)
    }

    /// Goes back to bytes [`Player::save`] made, on the same set.
    pub fn restore(&mut self, bytes: &[u8]) -> Result<(), String> {
        let mut r = bytes;
        let mut take = |n: usize| -> Result<&[u8], String> {
            if r.len() < n {
                return Err("the saved state is cut short".into());
            }
            let (a, b) = r.split_at(n);
            r = b;
            Ok(a)
        };
        if take(4)? != SAVE_MAGIC {
            return Err("not a saved arcade player state".into());
        }
        let v = take(1)?[0];
        if v != SAVE_VERSION {
            return Err(format!("saved state version {v} isn't supported"));
        }
        let frame = u64::from_le_bytes(take(8)?.try_into().expect("8 bytes"));
        let mut part = || -> Result<Vec<u8>, String> {
            let n = u64::from_le_bytes(take(8)?.try_into().expect("8 bytes"));
            Ok(take(usize::try_from(n).map_err(text)?)?.to_vec())
        };
        let snap = part()?;
        let log = part()?;
        let pictures = part()?;
        let log = InputLog::parse(std::str::from_utf8(&log).map_err(text)?).map_err(text)?;
        if log.arcade != self.log.arcade {
            return Err("the saved state is of another set".into());
        }
        let (whole, rest) = pictures.as_chunks::<8>();
        if !rest.is_empty() {
            return Err("the saved state's picture hashes are cut short".into());
        }
        let pictures: Vec<u64> = whole.iter().map(|c| u64::from_le_bytes(*c)).collect();
        if log.frames.len() as u64 != frame || pictures.len() as u64 != frame + 1 {
            return Err("the saved state's log and hashes don't reach its frame".into());
        }
        self.tapper.restore(&snap)?;
        self.log = log;
        self.pictures = pictures;
        Ok(())
    }

    /// Frames played since power-on.
    pub fn frame(&self) -> u64 {
        self.log.frames.len() as u64
    }

    /// What was played, as an arcade input log's text.
    pub fn input_log(&self) -> String {
        self.log.to_text()
    }

    /// Every frame's picture hash, frame 0 first.
    pub fn pictures(&self) -> &[u64] {
        &self.pictures
    }

    /// The pictures' digest: the BLAKE3 of every frame's picture hash,
    /// little-endian, frame 0 first.
    pub fn digest(&self) -> Hash {
        let bytes: Vec<u8> = self.pictures.iter().flat_map(|h| h.to_le_bytes()).collect();
        Hash::of(&bytes)
    }

    /// The machine's state hash now: the BLAKE3 of its snapshot, as a
    /// clip's `state` keeps it.
    pub fn state_hash(&self) -> Hash {
        Hash::of(&self.tapper.snapshot())
    }

    /// The last frame's picture as RGBA, 512 by 480.
    pub fn rgba(&mut self) -> &[u8] {
        for (px, c) in self
            .rgba
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(self.tapper.picture())
        {
            *px = [(c >> 16) as u8, (c >> 8) as u8, c as u8, 255];
        }
        &self.rgba
    }
}

#[cfg(target_arch = "wasm32")]
pub mod abi;

#[cfg(test)]
mod tests {
    use super::*;

    fn zip() -> Option<Vec<u8>> {
        let Some(p) = gp_arcade::romset::local_romset("rbtapper") else {
            eprintln!("skipped: no rbtapper romset");
            return None;
        };
        Some(std::fs::read(p).unwrap())
    }

    /// Played with a coin and a start, the log replays to the same picture
    /// on every frame, and a save restores to the same digest.
    #[test]
    fn the_log_replays_to_the_same_pictures_and_a_save_restores() {
        let Some(zip) = zip() else { return };
        let bits = |i: u32| match i {
            100..106 => 1,
            160..166 => 2,
            300.. if i % 40 < 6 => 0x40 | 0x08,
            _ => 0,
        };
        let mut p = Player::new(&zip).unwrap();
        let mut saved = Vec::new();
        for i in 0..420 {
            if i == 200 {
                saved = p.save().unwrap();
            }
            p.step(bits(i), false).unwrap();
        }
        let digest = p.digest();
        let mut r = Player::replay(&zip, &p.input_log()).unwrap();
        assert_eq!(r.run_to(u64::MAX).unwrap(), 420);
        assert_eq!(r.pictures(), p.pictures());
        assert_eq!(r.state_hash(), p.state_hash());
        p.restore(&saved).unwrap();
        assert_eq!(p.frame(), 200);
        for i in 200..420 {
            p.step(bits(i), false).unwrap();
        }
        assert_eq!(p.digest(), digest);
        assert_eq!(p.rgba().len(), RGBA_BYTES);
        assert!(p.step(0, true).is_err());
        assert!(p.step(0x80, false).is_err());
    }

    #[test]
    fn info_is_json_with_the_screen() {
        let i = info("rbtapper");
        assert!(
            i.contains("\"width\":512") && i.contains("\"fps\":[30,1]"),
            "{i}"
        );
    }
}
