//! The original game played live in the workspace (docs/show.md).
//!
//! [`Player`] runs `gp-nes` a frame at a time with the buttons a person
//! holds, keeps what they played as an input log (format 1,
//! docs/formats/recording.md) and every frame's hash, and draws each frame
//! as RGBA for a canvas. Built for `wasm32-unknown-unknown`, the [`abi`]
//! module exports it to JavaScript as plain functions over linear memory,
//! as `gp-web` exports the interpreter preview.
//!
//! The emulation never reads a clock: one call to [`Player::step`] is one
//! NES frame, and the page decides when to call it (60 times a second, or
//! as fast as it can to reach a frame). So the input log replays to the
//! same frames natively, and `gp serve` checks that by re-recording it with
//! `gp record` and comparing [`Player::digest`] with the recording's frame
//! hashes.
//!
//! A player can also replay a log ([`Player::replay`]): it powers on, runs
//! the log's frames to a frame without drawing, then steps the rest one
//! frame per call, for the workspace's replay of a marked moment.
//!
//! A player saves its whole state as bytes ([`Player::save`]) and goes back
//! to them ([`Player::restore`]), for free play's save points
//! (docs/show.md, "Free play"). The bytes hold the console's snapshot, the
//! input log from power-on and every frame's hashes, so a restored player
//! keeps the log and the digest of the frames that led there, and new
//! presses branch from that frame.
//!
//! A player can also mix the game's sound ([`Player::set_audio`], on in the
//! browser build): each frame's samples are kept until the next frame
//! ([`Player::audio`]). Sound is presentation only, as drawing is: the
//! hashes, the log and the saved bytes are the same with it or without.

use gp_evidence::Hash;
use gp_evidence::frame_hashes::{self, FrameHash};
use gp_evidence::input_log::{InputFrame, InputLog};
use gp_evidence::manifest::PowerOn;
use gp_nes::Nes;
use gp_nes::video::{HEIGHT, WIDTH};

/// Leads a player's saved state ([`Player::save`]), then a version byte.
/// `GPPS`, since 2026-11-04: the model runtimes' save record has `GPSV`
/// (`gp_sim::save`), and the two formats share nothing.
const SAVE_MAGIC: &[u8; 4] = b"GPPS";
/// The header a player's saved state had before, read still so states a
/// server kept (a start's `.play` file) load as they did.
const OLD_SAVE_MAGIC: &[u8; 4] = b"GPSV";
const SAVE_VERSION: u8 = 1;

/// Bytes of one RGBA frame.
pub const RGBA_BYTES: usize = WIDTH * HEIGHT * 4;

/// One console being played, and what was played on it.
pub struct Player {
    nes: Nes,
    /// The frames played so far; `frames.len()` is the console's frame.
    log: InputLog,
    /// Every frame's hashes from frame 0, as a recording keeps them.
    hashes: Vec<FrameHash>,
    /// A log being replayed, whose frames [`Player::advance`] plays.
    script: Option<InputLog>,
    rgb: Box<[u8; WIDTH * HEIGHT * 3]>,
    rgba: Vec<u8>,
    /// Whether the console mixes sound ([`Player::set_audio`]).
    audio: bool,
}

/// Samples a second of [`Player::audio`].
pub const AUDIO_RATE: u32 = gp_nes::AUDIO_RATE;

/// The screen, the frame rate and the buttons, as `gp_play_info` gives
/// them to the page: JSON. The arcade player (gp-arcade-play) gives its
/// own, so the page takes its numbers from whichever module it loaded.
pub fn info() -> String {
    format!(
        "{{\"platform\":\"nes\",\"width\":{WIDTH},\"height\":{HEIGHT},\"fps\":[39375000,655171],\"buttons\":[\"a\",\"b\",\"select\",\"start\",\"up\",\"down\",\"left\",\"right\"],\"sound\":true}}"
    )
}

fn text<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

impl Player {
    /// Powers the ROM on with the power-on state `gp record` uses by
    /// default (RAM zeros, NTSC), with video drawing on. Drawing doesn't
    /// change the canonical state, so the hashes are the ones a headless
    /// recording gets.
    pub fn new(rom: &[u8]) -> Result<Self, String> {
        Self::with_power_on(rom, PowerOn::default())
    }

    pub fn with_power_on(rom: &[u8], power_on: PowerOn) -> Result<Self, String> {
        let mut nes = Nes::power_on(rom, power_on).map_err(text)?;
        nes.set_video(true);
        let mut p = Self {
            nes,
            log: InputLog::new(1),
            hashes: Vec::new(),
            script: None,
            rgb: Box::new([0; WIDTH * HEIGHT * 3]),
            rgba: vec![0; RGBA_BYTES],
            audio: false,
        };
        p.push_hash()?;
        Ok(p)
    }

    /// Powers on to replay `log`: [`Player::advance`] then plays its frames
    /// in order.
    pub fn replay(rom: &[u8], log: &str) -> Result<Self, String> {
        let script = InputLog::parse(log).map_err(text)?;
        let mut p = Self::new(rom)?;
        p.script = Some(script);
        Ok(p)
    }

    /// Turns sound on or off. On, each frame [`Player::step`] runs mixes
    /// its samples, which [`Player::audio`] gives until the next frame.
    pub fn set_audio(&mut self, on: bool) {
        self.audio = on;
        self.nes.set_output(true, on);
        self.nes.clear_audio();
    }

    /// The last frame's sound: mono samples at [`AUDIO_RATE`], about 800
    /// of them. Empty with sound off, and after a restore or a
    /// [`Player::run_to`] until the next frame.
    pub fn audio(&self) -> &[f32] {
        self.nes.audio_samples()
    }

    fn push_hash(&mut self) -> Result<(), String> {
        self.hashes.push(FrameHash {
            state: self.nes.state_or_board_free_hash().map_err(text)?,
            ram: self.nes.wram_hash(),
        });
        Ok(())
    }

    /// Runs one frame with controller 1's buttons (input log bits: A, B,
    /// Select, Start, Up, Down, Left, Right from bit 0) and, when `reset`,
    /// the reset button pressed first. Returns whether the game read the
    /// controller.
    pub fn step(&mut self, pad: u8, reset: bool) -> Result<bool, String> {
        self.step_pads([pad, 0], reset)
    }

    /// [`Player::step`] with controller 2's buttons too. The log keeps one
    /// controller until controller 2 first holds a button, then two (its
    /// earlier frames with controller 2 empty), so a one-player play's log
    /// is the same as before.
    pub fn step_pads(&mut self, pads: [u8; 2], reset: bool) -> Result<bool, String> {
        if pads[1] != 0 && self.log.controllers < 2 {
            self.log.controllers = 2;
        }
        let mut frame = InputFrame {
            polled: false,
            pads: [pads[0], pads[1], 0, 0],
            reset,
        };
        self.nes.clear_audio();
        frame.polled = self.nes.step(frame).map_err(text)?.polled;
        self.log.frames.push(frame);
        self.push_hash()?;
        Ok(frame.polled)
    }

    /// Plays the next frame of the log being replayed. False when there's
    /// none left (or no log).
    pub fn advance(&mut self) -> Result<bool, String> {
        let i = self.log.frames.len();
        let Some(f) = self.script.as_ref().and_then(|s| s.frames.get(i)).copied() else {
            return Ok(false);
        };
        self.step_pads([f.pads[0], f.pads[1]], f.reset)?;
        Ok(true)
    }

    /// Plays the replayed log up to `frame` without drawing anything, so
    /// the page can show a moment at once. Stops early at the log's end.
    pub fn run_to(&mut self, frame: u64) -> Result<u64, String> {
        self.nes.set_output(false, false);
        let r = (|| {
            while self.frame() < frame.saturating_sub(1) {
                if !self.advance()? {
                    break;
                }
            }
            Ok::<(), String>(())
        })();
        self.nes.set_output(true, self.audio);
        r?;
        // The last frame is drawn, so the picture is that frame's.
        if self.frame() < frame {
            self.advance()?;
        }
        // A jump has no sound of its own.
        self.nes.clear_audio();
        Ok(self.frame())
    }

    /// The player's whole state as bytes: the console's snapshot, the
    /// buttons the last frame ran with, the input log and every frame's
    /// hashes. Only the same build of gp-nes restores the snapshot.
    pub fn save(&self) -> Result<Vec<u8>, String> {
        let snap = self.nes.snapshot().map_err(text)?;
        let log = self.log.to_text();
        let hashes = frame_hashes::encode(&self.hashes);
        let mut out = Vec::with_capacity(32 + snap.len() + log.len() + hashes.len());
        out.extend_from_slice(SAVE_MAGIC);
        out.push(SAVE_VERSION);
        out.extend_from_slice(&self.frame().to_le_bytes());
        out.extend_from_slice(&self.nes.held());
        for part in [&snap[..], log.as_bytes(), &hashes] {
            out.extend_from_slice(&(part.len() as u64).to_le_bytes());
            out.extend_from_slice(part);
        }
        Ok(out)
    }

    /// Goes back to bytes [`Player::save`] made. A log being replayed is
    /// dropped: from here on the frames are the ones [`Player::step`] gets.
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
        let magic = take(4)?;
        if magic != SAVE_MAGIC && magic != OLD_SAVE_MAGIC {
            return Err("not a saved player state".into());
        }
        let v = take(1)?[0];
        if v != SAVE_VERSION {
            return Err(format!("saved state version {v} isn't supported"));
        }
        let frame = u64::from_le_bytes(take(8)?.try_into().expect("8 bytes"));
        let held: [u8; 4] = take(4)?.try_into().expect("4 bytes");
        let mut part = || -> Result<Vec<u8>, String> {
            let n = u64::from_le_bytes(take(8)?.try_into().expect("8 bytes"));
            Ok(take(usize::try_from(n).map_err(text)?)?.to_vec())
        };
        let snap = part()?;
        let log = part()?;
        let hashes = part()?;
        let log = InputLog::parse(std::str::from_utf8(&log).map_err(text)?).map_err(text)?;
        let hashes = frame_hashes::decode(&hashes).map_err(text)?;
        if log.frames.len() as u64 != frame || hashes.len() as u64 != frame + 1 {
            return Err("the saved state's log and hashes don't reach its frame".into());
        }
        self.nes.restore(&snap, frame, held).map_err(text)?;
        self.nes.set_output(true, self.audio);
        self.nes.clear_audio();
        self.log = log;
        self.hashes = hashes;
        // A replayed log keeps playing from the restored frame (a clip loop
        // restores its start); new presses still branch off it with step.
        Ok(())
    }

    /// Frames played since power-on.
    pub fn frame(&self) -> u64 {
        self.log.frames.len() as u64
    }

    /// The frames left in the log being replayed.
    pub fn script_frames(&self) -> u64 {
        self.script.as_ref().map_or(0, |s| s.frames.len() as u64)
    }

    /// What was played, as input log text.
    pub fn input_log(&self) -> String {
        self.log.to_text()
    }

    pub fn log(&self) -> &InputLog {
        &self.log
    }

    /// The BLAKE3 hash of the frame hash file a recording of this input
    /// would store (`frame_hashes::encode`, frames 0 to now): equal to the
    /// recording manifest's `frame_hashes` when every frame agrees.
    pub fn digest(&self) -> Hash {
        Hash::of(&frame_hashes::encode(&self.hashes))
    }

    pub fn hashes(&self) -> &[FrameHash] {
        &self.hashes
    }

    /// The current frame's state hash.
    pub fn state_hash(&self) -> Hash {
        self.hashes.last().map_or(Hash::of(&[]), |h| h.state)
    }

    /// The last frame's picture as RGBA, 256 by 240, for a canvas's
    /// `ImageData`.
    pub fn rgba(&mut self) -> &[u8] {
        self.nes.frame_rgb(&mut self.rgb);
        for (px, rgb) in self
            .rgba
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(self.rgb.as_chunks::<3>().0)
        {
            px[..3].copy_from_slice(rgb);
            px[3] = 255;
        }
        &self.rgba
    }
}

#[cfg(target_arch = "wasm32")]
pub mod abi;

#[cfg(test)]
mod tests {
    use super::*;
    use gp_evidence::input_log::button;
    use gp_nes::testrom;

    #[test]
    fn the_log_replays_to_the_same_hashes_without_video() {
        let rom = testrom::build();
        let mut p = Player::new(&rom).unwrap();
        for i in 0..120u32 {
            let pad = if i % 7 < 3 { button::A } else { button::RIGHT };
            p.step(pad, i == 50).unwrap();
        }
        let log = InputLog::parse(&p.input_log()).unwrap();
        assert_eq!(log.frames.len(), 120);
        assert!(log.frames[50].reset);
        // A headless console on the same log, as gp record runs it.
        let mut nes = Nes::power_on(&rom, PowerOn::default()).unwrap();
        let mut hashes = vec![FrameHash {
            state: nes.state_hash().unwrap(),
            ram: nes.wram_hash(),
        }];
        for f in &log.frames {
            nes.step(*f).unwrap();
            hashes.push(FrameHash {
                state: nes.state_hash().unwrap(),
                ram: nes.wram_hash(),
            });
        }
        assert_eq!(hashes, p.hashes());
        assert_eq!(Hash::of(&frame_hashes::encode(&hashes)), p.digest());
        // Polled flags are the game's: the test program reads the pad on
        // every other frame.
        assert!(log.frames.iter().any(|f| f.polled) && log.frames.iter().any(|f| !f.polled));
    }

    #[test]
    fn controller_two_joins_the_log_when_it_first_holds_a_button() {
        let rom = testrom::build();
        let mut p = Player::new(&rom).unwrap();
        for _ in 0..10 {
            p.step(button::A, false).unwrap();
        }
        assert!(p.input_log().contains("controllers 1\n"));
        for i in 0..10u8 {
            p.step_pads([0, if i % 2 == 0 { button::START } else { 0 }], false)
                .unwrap();
        }
        let log = InputLog::parse(&p.input_log()).unwrap();
        assert_eq!(log.controllers, 2);
        assert_eq!(log.frames[0].pads[..2], [button::A, 0]);
        assert_eq!(log.frames[10].pads[..2], [0, button::START]);
        // A replay of the log plays controller 2 back.
        let mut q = Player::replay(&rom, &p.input_log()).unwrap();
        while q.advance().unwrap() {}
        assert_eq!(q.input_log(), p.input_log());
        assert_eq!(q.digest(), p.digest());
    }

    #[test]
    fn a_replay_reaches_a_frame_and_plays_on() {
        let rom = testrom::build();
        let mut p = Player::new(&rom).unwrap();
        for i in 0..90u32 {
            p.step(if i % 2 == 0 { button::B } else { 0 }, false)
                .unwrap();
        }
        let log = p.input_log();
        let mut r = Player::replay(&rom, &log).unwrap();
        assert_eq!(r.script_frames(), 90);
        assert_eq!(r.run_to(40).unwrap(), 40);
        assert_eq!(r.state_hash(), p.hashes()[40].state);
        while r.advance().unwrap() {}
        assert_eq!(r.frame(), 90);
        assert_eq!(r.digest(), p.digest());
        assert_eq!(r.rgba().len(), RGBA_BYTES);
    }

    fn pad(i: u32) -> u8 {
        match i % 11 {
            0..=3 => button::RIGHT,
            4..=5 => button::A | button::RIGHT,
            6 => button::B,
            _ => 0,
        }
    }

    #[test]
    fn save_step_restore_step_reaches_the_same_frames() {
        let rom = testrom::build();
        let mut p = Player::new(&rom).unwrap();
        for i in 0..70u32 {
            p.step(pad(i), false).unwrap();
        }
        let saved = p.save().unwrap();
        let k = 45u32;
        let mut first = Vec::new();
        for i in 0..k {
            p.step(pad(i + 3), false).unwrap();
            first.push(p.state_hash());
        }
        let (digest, picture) = (p.digest(), p.rgba().to_vec());
        p.restore(&saved).unwrap();
        assert_eq!(p.frame(), 70);
        let mut second = Vec::new();
        for i in 0..k {
            p.step(pad(i + 3), false).unwrap();
            second.push(p.state_hash());
        }
        assert_eq!(first, second);
        assert_eq!(p.digest(), digest);
        assert_eq!(p.rgba(), &picture[..]);
        // Another player restores the same bytes and branches: its log is
        // the 70 frames to the save, then its own presses.
        let mut q = Player::new(&rom).unwrap();
        q.restore(&saved).unwrap();
        q.step(button::B, false).unwrap();
        let log = InputLog::parse(&q.input_log()).unwrap();
        assert_eq!(log.frames.len(), 71);
        assert_eq!(
            log.frames[..70],
            InputLog::parse(&p.input_log()).unwrap().frames[..70]
        );
        // The branch's log replays natively to the hashes the player has.
        let mut nes = Nes::power_on(&rom, PowerOn::default()).unwrap();
        for f in &log.frames {
            nes.step(*f).unwrap();
        }
        assert_eq!(nes.state_hash().unwrap(), q.state_hash());
    }

    #[test]
    fn sound_changes_no_hash_log_or_saved_byte() {
        let rom = testrom::build();
        let mut quiet = Player::new(&rom).unwrap();
        let mut loud = Player::new(&rom).unwrap();
        loud.set_audio(true);
        let mut heard = 0;
        for i in 0..150u32 {
            quiet.step(pad(i), i == 60).unwrap();
            loud.step(pad(i), i == 60).unwrap();
            assert!(quiet.audio().is_empty());
            heard += loud.audio().len();
            if i % 50 == 49 {
                assert_eq!(quiet.save().unwrap(), loud.save().unwrap(), "frame {i}");
            }
        }
        // About 800 samples a frame at 48 kHz.
        assert!((150 * 790..150 * 810).contains(&heard), "{heard}");
        assert_eq!(quiet.hashes(), loud.hashes());
        assert_eq!(quiet.input_log(), loud.input_log());
        assert_eq!(quiet.digest(), loud.digest());
        // A restore leaves no samples behind, and the frames after it are
        // the same as without sound.
        let saved = loud.save().unwrap();
        loud.restore(&saved).unwrap();
        assert!(loud.audio().is_empty());
        quiet.restore(&saved).unwrap();
        for i in 0..30u32 {
            quiet.step(pad(i), false).unwrap();
            loud.step(pad(i), false).unwrap();
            assert!(!loud.audio().is_empty());
        }
        assert_eq!(quiet.hashes(), loud.hashes());
    }

    #[test]
    fn a_restore_refuses_bytes_it_did_not_save() {
        let rom = testrom::build();
        let mut p = Player::new(&rom).unwrap();
        assert!(p.restore(b"nope").is_err());
        let mut saved = p.save().unwrap();
        // Its own header, not the model runtimes' save record's.
        assert_eq!(&saved[..5], b"GPPS\x01");
        // A state saved before 2026-11-04, under GPSV, still restores.
        let mut old = saved.clone();
        old[..4].copy_from_slice(b"GPSV");
        assert!(p.restore(&old).is_ok());
        saved.truncate(saved.len() - 1);
        assert!(p.restore(&saved).is_err());
    }
}
