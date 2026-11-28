//! An arcade original played live in the mechanic page: gp-play's
//! [`Player`] for gp-arcade.
//!
//! [`Player`] runs an MCR set a frame at a time with the controls a person
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
//! With the sound on ([`Player::set_audio`], on in the page), each frame
//! keeps what reached the two speakers ([`Player::audio`]): stereo, left
//! and right interleaved, at 48 kHz, as MAME makes it (gp-arcade's sound
//! module). Sound is presentation only, as drawing is.

use gp_arcade::attotime::Attotime;
use gp_arcade::ssio::{FRAME, SoundBoard};
use gp_arcade::video::{HEIGHT, WIDTH};
use gp_arcade::{Controls, FourWay, Mcr, Romset};
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

/// Samples a second of [`Player::audio`], each speaker.
pub const AUDIO_RATE: u32 = gp_arcade::sound::OUT_RATE;
/// The speakers: left (the first AY) and right (the second).
pub const AUDIO_CHANNELS: u32 = 2;

/// The screen, the frame rate and the buttons, as `gp_play_info` gives
/// them to the page: JSON.
pub fn info(system: &str) -> String {
    format!(
        "{{\"platform\":\"arcade\",\"system\":\"{system}\",\"width\":{WIDTH},\"height\":{HEIGHT},\"fps\":[30,1],\"columns\":\"{COLUMNS}\",\"buttons\":[\"coin\",\"start1\",\"right\",\"left\",\"down\",\"up\",\"button1\"],\"sound\":true,\"channels\":{AUDIO_CHANNELS}}}"
    )
}

/// The 32-bit FNV-1a hash of `bytes`: what the sound checks compare a
/// frame's samples by, since the page's JavaScript makes the same one
/// with nothing but `Math.imul` (scripts/web/check-play-arcade.mjs).
pub fn fnv1a(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0x811c_9dc5u32, |h, &b| {
        (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
    })
}

fn text<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

/// One machine being played, and what was played on it.
pub struct Player {
    mcr: Mcr,
    /// The frames played so far; `frames.len()` is the machine's frame.
    log: InputLog,
    /// Every frame's picture hash from frame 0 (the oracle's).
    pictures: Vec<u64>,
    /// A log being replayed, whose frames [`Player::advance`] plays.
    script: Option<InputLog>,
    rgba: Vec<u8>,
    /// The last frame's sound, left and right interleaved, when on.
    audio: Vec<f32>,
    /// MAME's 4-way joystick filter, between the person's keys and the
    /// board ([`Player::step`]).
    stick: FourWay,
    /// The random draws recorded at the game's draw sites, when given
    /// ([`Player::set_draw_sites`]).
    draws: Option<gp_arcade::draws::Recorder>,
    /// The last frame's draws, site and byte in order.
    last_draws: Vec<(String, u8)>,
}

impl Player {
    /// Powers on the set in a MAME zip.
    pub fn new(zip: &[u8]) -> Result<Self, String> {
        let rom = Romset::from_zip(zip).map_err(text)?;
        Ok(Self::from_romset(&rom))
    }

    /// Powers on a set already read (a test's made-up one,
    /// gp_arcade::testrom).
    pub fn from_romset(rom: &Romset) -> Self {
        let mcr = Mcr::power_on(rom);
        let pictures = vec![mcr.picture_hash()];
        Self {
            mcr,
            log: InputLog::new_arcade(rom.name, COLUMNS),
            pictures,
            script: None,
            rgba: vec![0; RGBA_BYTES],
            audio: Vec::new(),
            stick: FourWay::default(),
            draws: None,
            last_draws: Vec::new(),
        }
    }

    /// Records the random draws the game makes at these draw sites from
    /// the next frame on, one [`gp_arcade::draws::Site::line`] a line (a
    /// state mapping's `[[draws]]`, as `gp` records a clip's: gp-arcade's
    /// [`gp_arcade::draws::Recorder`]); an empty text stops. The page
    /// hands each frame's draws to the model beside the original, so the
    /// model makes the original's random choices.
    pub fn set_draw_sites(&mut self, text: &str) -> Result<(), String> {
        let sites = text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(gp_arcade::draws::Site::parse_line)
            .collect::<Result<Vec<_>, _>>()?;
        self.last_draws.clear();
        self.draws = if sites.is_empty() {
            None
        } else {
            Some(gp_arcade::draws::Recorder::new(sites, &self.mcr)?)
        };
        Ok(())
    }

    /// The draws of the last frame played, site and byte in order (none
    /// without draw sites).
    pub fn draws(&self) -> &[(String, u8)] {
        &self.last_draws
    }

    /// Powers on to replay `log`, an arcade log of the same set.
    pub fn replay(zip: &[u8], log: &str) -> Result<Self, String> {
        let mut p = Self::new(zip)?;
        p.script(log)?;
        Ok(p)
    }

    /// Takes `log`, an arcade log of the same set, as the log
    /// [`Player::advance`] and [`Player::run_to`] play: from the frame the
    /// player is at, so after a [`Player::restore`] it plays on from there.
    pub fn script(&mut self, log: &str) -> Result<(), String> {
        let script = InputLog::parse(log).map_err(text)?;
        match &script.arcade {
            Some(a) if Some(a) == self.log.arcade.as_ref() => {}
            Some(a) => {
                return Err(format!(
                    "the log is of {} with columns {}; this is {} with {COLUMNS}",
                    a.system,
                    a.columns,
                    self.system()
                ));
            }
            None => return Err("an NES input log can't play on an arcade board".into()),
        }
        self.script = Some(script);
        Ok(())
    }

    /// What was played, as an input log.
    pub fn log(&self) -> &InputLog {
        &self.log
    }

    /// The MAME set being played.
    pub fn system(&self) -> &str {
        self.log.arcade.as_ref().map_or("", |a| a.system.as_str())
    }

    /// Turns keeping each frame's sound on or off.
    pub fn set_audio(&mut self, on: bool) {
        self.mcr.set_audio(on);
        self.audio.clear();
    }

    /// The last frame's sound: left and right interleaved at
    /// [`AUDIO_RATE`], one or two of MAME's 20 ms pieces (960 samples a
    /// speaker each). Empty after a restore, so a jump is silent.
    pub fn audio(&self) -> &[f32] {
        &self.audio
    }

    /// Runs one frame with the controls a person holds (bits 0 to 6: coin,
    /// start, right, left, down, up, tap). The joystick goes through MAME's
    /// 4-way filter first, as it would for a player in MAME: two directions
    /// at once become one. The log keeps what reached the board, so it
    /// replays the same with or without the filter, in MAME as the oracle
    /// sets inputs. Returns whether the game read them.
    pub fn step(&mut self, bits: u32, reset: bool) -> Result<bool, String> {
        if reset {
            return Err("the arcade board has no reset button".into());
        }
        if bits > 0x7f {
            return Err(format!("buttons {bits:#x}: bits 0 to 6 only"));
        }
        let bits = self.stick.filter(Controls::from_bits(bits)).bits();
        self.play(bits);
        Ok(self.mcr.polled())
    }

    /// One frame with these bits on the board as they are.
    fn play(&mut self, bits: u32) {
        if let Some(r) = &self.draws {
            r.arm(&mut self.mcr);
        }
        self.mcr.step(Controls::from_bits(bits));
        self.last_draws.clear();
        if let Some(r) = &mut self.draws {
            self.last_draws = r.take(&mut self.mcr);
        }
        self.audio.clear();
        self.audio
            .extend(self.mcr.take_audio().iter().flat_map(|s| [s[0], s[1]]));
        // The game's own flag, as gp-play keeps it: false on the boot
        // frames before the game first reads its ports (94 in the first set
        // ported)
        // and on any other frame it doesn't. The text log can't say it, so
        // a log read back has every frame polled.
        self.log.frames.push(InputFrame {
            polled: self.mcr.polled(),
            pads: [bits as u8, 0, 0, 0],
            reset: false,
        });
        self.pictures.push(self.mcr.picture_hash());
    }

    /// Plays the next frame of the log being replayed. False when there's
    /// none left (or no log).
    pub fn advance(&mut self) -> Result<bool, String> {
        let i = self.log.frames.len();
        let Some(f) = self.script.as_ref().and_then(|s| s.frames.get(i)).copied() else {
            return Ok(false);
        };
        // A log holds what reached the board, already through the filter.
        let bits = u32::from(f.pads[0]);
        if bits > 0x7f {
            return Err(format!("buttons {bits:#x}: bits 0 to 6 only"));
        }
        self.play(bits);
        let held = Controls::from_bits(bits);
        self.stick = FourWay::default();
        self.stick.filter(held);
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
        let snap = self.mcr.snapshot();
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
        self.mcr.restore(&snap)?;
        self.mcr.take_audio();
        self.audio.clear();
        // A call that was waiting to return belongs to the frames left.
        if let Some(r) = &mut self.draws {
            r.forget();
        }
        self.last_draws.clear();
        // The filter's state isn't saved: it starts again from the last
        // frame's directions, which differs only while a diagonal is held.
        self.stick = FourWay::default();
        if let Some(f) = log.frames.last() {
            self.stick.filter(Controls::from_bits(u32::from(f.pads[0])));
        }
        self.log = log;
        self.pictures = pictures;
        Ok(())
    }

    /// Frames played since power-on.
    pub fn frame(&self) -> u64 {
        self.log.frames.len() as u64
    }

    /// Keeps each frame's latch writes as they reached the sound board
    /// ([`Player::take_latches`]), or stops.
    pub fn set_latch_log(&mut self, on: bool) {
        self.mcr.set_latch_log(on);
    }

    /// The last frames' latch writes as [`SoundPlayer::frame`] takes them,
    /// timed from the start of the frame that began at `frame_start`.
    pub fn take_latches(&mut self, frame_start: Attotime) -> Vec<u8> {
        latch_records(frame_start, &self.mcr.take_latch_log())
    }

    /// The emulated time.
    pub fn time(&self) -> Attotime {
        self.mcr.time()
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
        Hash::of(&self.mcr.snapshot())
    }

    /// The last frame's picture as RGBA, 512 by 480.
    pub fn rgba(&mut self) -> &[u8] {
        for (px, c) in self
            .rgba
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(self.mcr.picture())
        {
            *px = [(c >> 16) as u8, (c >> 8) as u8, c as u8, 255];
        }
        &self.rgba
    }
}

/// Bytes of one latch write as [`SoundPlayer::frame`] takes it: the
/// attoseconds from the frame's start (i64, little endian), the latch (0
/// to 3) and the byte.
pub const LATCH_RECORD: usize = 10;

/// Latch writes at absolute times as [`SoundPlayer::frame`]'s records,
/// timed from `frame_start`.
pub fn latch_records(frame_start: Attotime, writes: &[(Attotime, u8, u8)]) -> Vec<u8> {
    let mut out = Vec::with_capacity(writes.len() * LATCH_RECORD);
    for &(at, latch, value) in writes {
        let off = if at > frame_start {
            (at - frame_start).as_attoseconds()
        } else {
            0
        };
        out.extend_from_slice(&off.to_le_bytes());
        out.push(latch);
        out.push(value);
    }
    out
}

/// When a frame's sound requests reach the board, for a model that gives
/// only each latch and byte (gp_sim::Ext::sound_requests): the first MCR
/// game ported sends the queue from its interrupt early in the frame, a
/// write about every 10 microseconds (gp-arcade's latch log on its first
/// oracle capture: from 1.26 to 1.43 ms in). Timed from 1.30 ms on, 10
/// microseconds apart, the board alone makes the whole machine's sound on
/// every frame of that capture in four of the game's five sets; at the
/// frame's start it differs on a quarter of them. The fifth set's own
/// sound program hears the times closer: its AY writes are the machine's
/// on all but a few frames, its samples on three quarters.
pub const REQUEST_AT: i64 = 1_300_000_000_000_000;
/// The time between two requests of a frame ([`REQUEST_AT`]).
pub const REQUEST_STEP: i64 = 10_000_000_000_000;

/// A frame's sound requests, each a latch and a byte in order, as
/// [`SoundPlayer::frame`]'s records timed as [`REQUEST_AT`] says.
pub fn request_records(requests: &[(u8, u8)]) -> Vec<u8> {
    let mut out = Vec::with_capacity(requests.len() * LATCH_RECORD);
    for (i, &(latch, value)) in requests.iter().enumerate() {
        let at = REQUEST_AT + REQUEST_STEP * i as i64;
        out.extend_from_slice(&at.to_le_bytes());
        out.push(latch);
        out.push(value);
    }
    out
}

/// An arcade board's sound alone, for a page whose game is a model: the
/// board's own sound CPU and AY-8910s from the player's own romset
/// (gp_arcade::ssio), fed the latch writes the game makes, a 30 Hz frame
/// at a time. The samples are [`Player::audio`]'s: stereo at
/// [`AUDIO_RATE`], left and right interleaved.
pub struct SoundPlayer {
    board: SoundBoard,
    audio: Vec<f32>,
}

impl SoundPlayer {
    /// The sound board of the set in a MAME zip, at power-on.
    pub fn new(zip: &[u8]) -> Result<Self, String> {
        let rom = Romset::from_zip(zip).map_err(text)?;
        Ok(Self::from_romset(&rom))
    }

    pub fn from_romset(rom: &Romset) -> Self {
        let mut board = SoundBoard::power_on(rom);
        board.set_audio(true);
        Self {
            board,
            audio: Vec::new(),
        }
    }

    /// One frame with these latch writes ([`LATCH_RECORD`] bytes each, in
    /// time order); a write timed past the frame's end lands at its end.
    pub fn frame(&mut self, records: &[u8]) -> Result<(), String> {
        let (whole, rest) = records.as_chunks::<LATCH_RECORD>();
        if !rest.is_empty() {
            return Err(format!(
                "{} bytes of latch writes: {LATCH_RECORD} a write",
                records.len()
            ));
        }
        let writes: Vec<(Attotime, u8, u8)> = whole
            .iter()
            .map(|r| {
                let off = i64::from_le_bytes(r[..8].try_into().expect("8 bytes"));
                let off = off.clamp(0, FRAME.attoseconds);
                (Attotime::new(0, off), r[8] & 3, r[9])
            })
            .collect();
        self.board.frame(&writes);
        self.audio.clear();
        self.audio
            .extend(self.board.take_audio().iter().flat_map(|s| [s[0], s[1]]));
        Ok(())
    }

    /// One frame with a model's sound requests, each a latch and a byte,
    /// timed as [`REQUEST_AT`] says.
    pub fn requests(&mut self, requests: &[(u8, u8)]) -> Result<(), String> {
        self.frame(&request_records(requests))
    }

    /// The last frame's sound, left and right interleaved.
    pub fn audio(&self) -> &[f32] {
        &self.audio
    }

    /// The AY registers the last frame wrote: chip, register, value.
    pub fn ay_registers_written(&self) -> &[(u8, u8, u8)] {
        self.board.ay_registers_written()
    }
}

#[cfg(target_arch = "wasm32")]
pub mod abi;

#[cfg(test)]
mod tests {
    use super::*;

    /// The sound board alone, fed the whole machine's latch writes as
    /// records, makes the whole machine's sound on every frame (a made-up
    /// set, gp_arcade::testrom::sounding).
    #[test]
    fn the_sound_player_sounds_as_the_whole_player() {
        let rom = gp_arcade::testrom::sounding();
        let mut whole = Player::from_romset(&rom);
        whole.set_audio(true);
        whole.set_latch_log(true);
        let mut alone = SoundPlayer::from_romset(&rom);
        let mut loud = 0;
        for f in 0..60 {
            let start = whole.time();
            whole.step(0, false).unwrap();
            let records = whole.take_latches(start);
            assert_eq!(records.len() % LATCH_RECORD, 0);
            alone.frame(&records).unwrap();
            assert_eq!(alone.audio(), whole.audio(), "frame {f}");
            loud += whole.audio().iter().filter(|v| v.abs() > 0.01).count();
        }
        assert!(loud > 0);
        assert!(alone.frame(&[0; 3]).is_err());
    }

    /// Played with a coin and a start, the log replays to the same picture
    /// on every frame, and a save restores to the same digest (a made-up
    /// set that reads its inputs every frame).
    #[test]
    fn the_log_replays_to_the_same_pictures_and_a_save_restores() {
        let rom = gp_arcade::testrom::busy();
        let bits = |i: u32| match i {
            100..106 => 1,
            160..166 => 2,
            300.. if i % 40 < 6 => 0x40 | 0x08,
            _ => 0,
        };
        let mut p = Player::from_romset(&rom);
        let mut saved = Vec::new();
        for i in 0..420 {
            if i == 200 {
                saved = p.save().unwrap();
            }
            p.step(bits(i), false).unwrap();
        }
        let digest = p.digest();
        let mut r = Player::from_romset(&rom);
        r.script(&p.input_log()).unwrap();
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

    /// Up and right held at once from rest reach the board as right alone,
    /// as MAME's 4-way filter has it, and the log keeps right.
    #[test]
    fn a_diagonal_goes_through_the_4way_filter() {
        let mut p = Player::from_romset(&gp_arcade::testrom::busy());
        p.step(0, false).unwrap();
        p.step(0x04 | 0x20, false).unwrap();
        p.step(0x04 | 0x20, false).unwrap();
        let log = p.input_log();
        assert!(log.lines().any(|l| l.ends_with("..R....")), "{log}");
        assert!(!log.contains("..R..U."), "{log}");
    }

    /// The log keeps the game's polled flag, frame by frame: a set that
    /// never reads its ports polls on no frame, one that reads them every
    /// frame on its last (made-up sets).
    #[test]
    fn the_log_keeps_the_polled_flag_of_each_frame() {
        let mut p = Player::from_romset(&gp_arcade::testrom::busy());
        let read: Vec<bool> = (0..120).map(|_| p.step(0, false).unwrap()).collect();
        let kept: Vec<bool> = p.log.frames.iter().map(|f| f.polled).collect();
        assert_eq!(kept, read);
        assert!(kept[119]);
        let mut p = Player::from_romset(&gp_arcade::testrom::looping());
        let read: Vec<bool> = (0..30).map(|_| p.step(0, false).unwrap()).collect();
        let kept: Vec<bool> = p.log.frames.iter().map(|f| f.polled).collect();
        assert_eq!(kept, read);
        assert!(kept.iter().all(|p| !p));
    }

    /// With the sound on, a frame gives one or two 20 ms pieces of stereo,
    /// and some of it isn't silent on a made-up set that sounds; with it
    /// off, none.
    #[test]
    fn the_sound_comes_out_in_pieces() {
        let mut p = Player::from_romset(&gp_arcade::testrom::sounding());
        p.set_audio(true);
        let mut loud = 0usize;
        for i in 0..900 {
            p.step(if (100..106).contains(&i) { 1 } else { 0 }, false)
                .unwrap();
            let n = p.audio().len();
            // Frame 1 ends before MAME's first 20 ms stream update.
            assert!(
                n == 2 * 960 || n == 4 * 960 || (i == 0 && n == 0),
                "frame {i}: {n} samples"
            );
            loud += p.audio().iter().filter(|s| s.abs() > 0.01).count();
        }
        assert!(loud > 0, "900 frames with a coin dropped made no sound");
        p.set_audio(false);
        p.step(0, false).unwrap();
        assert!(p.audio().is_empty());
    }

    #[test]
    fn info_is_json_with_the_screen() {
        let i = info("madeup");
        assert!(
            i.contains("\"width\":512") && i.contains("\"fps\":[30,1]"),
            "{i}"
        );
    }
}
