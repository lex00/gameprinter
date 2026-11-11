//! The player's exports to JavaScript, wasm32 only, in the same shape as
//! `gp-web`'s: bytes go in through a buffer from `gp_alloc`; text comes
//! back in the output buffer, read with `gp_out_ptr` and `gp_out_len`. A
//! function returning `i32` returns 0 when it worked and 1 when it didn't,
//! with the error's text in the output buffer. One module holds two
//! players: slot 0 for playing, slot 1 for a replay beside the model.
//!
//! The browser runs a module on one thread, so the cells below are never
//! reached from two places at once.

#![allow(unsafe_code)]

use std::cell::RefCell;

use crate::Player;

thread_local! {
    static PLAYERS: RefCell<[Option<Player>; 2]> = const { RefCell::new([None, None]) };
    static OUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

fn set_out(bytes: &[u8]) {
    OUT.with(|o| {
        let mut o = o.borrow_mut();
        o.clear();
        o.extend_from_slice(bytes);
    });
}

fn result(r: Result<String, String>) -> i32 {
    match r {
        Ok(text) => {
            set_out(text.as_bytes());
            0
        }
        Err(e) => {
            set_out(e.as_bytes());
            1
        }
    }
}

fn with_player<T>(
    slot: u32,
    f: impl FnOnce(&mut Player) -> Result<T, String>,
) -> Result<T, String> {
    PLAYERS.with(|p| match p.borrow_mut().get_mut(slot as usize) {
        Some(Some(p)) => f(p),
        Some(None) => Err(format!("no game in slot {slot}: call gp_play_load first")),
        None => Err(format!("no slot {slot}: 0 and 1")),
    })
}

fn put(slot: u32, p: Result<Player, String>) -> i32 {
    result(p.and_then(|p| {
        PLAYERS.with(|c| match c.borrow_mut().get_mut(slot as usize) {
            Some(s) => {
                *s = Some(p);
                Ok(String::new())
            }
            None => Err(format!("no slot {slot}: 0 and 1")),
        })
    }))
}

/// Takes back the `len` bytes JavaScript wrote at `ptr`, a buffer from
/// `gp_alloc`.
fn input(ptr: *mut u8, len: usize) -> Box<[u8]> {
    // SAFETY: `ptr` and `len` are a boxed slice `gp_alloc` leaked, which
    // JavaScript filled and hands back here exactly once.
    unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len)) }
}

/// A buffer of `len` bytes for JavaScript to fill. The function it's passed
/// to frees it.
#[unsafe(no_mangle)]
pub extern "C" fn gp_alloc(len: usize) -> *mut u8 {
    Box::into_raw(vec![0u8; len].into_boxed_slice()) as *mut u8
}

#[unsafe(no_mangle)]
pub extern "C" fn gp_out_ptr() -> *const u8 {
    OUT.with(|o| o.borrow().as_ptr())
}

#[unsafe(no_mangle)]
pub extern "C" fn gp_out_len() -> usize {
    OUT.with(|o| o.borrow().len())
}

/// A player for the page: sound on ([`Player::set_audio`]).
fn sounding(p: Result<Player, String>) -> Result<Player, String> {
    p.map(|mut p| {
        p.set_audio(true);
        p
    })
}

/// Powers on the ROM (file bytes) in a slot.
#[unsafe(no_mangle)]
pub extern "C" fn gp_play_load(slot: u32, ptr: *mut u8, len: usize) -> i32 {
    let rom = input(ptr, len);
    put(slot, sounding(Player::new(&rom)))
}

/// Powers on the ROM to replay an input log: `rom_len` bytes of ROM then
/// the log's text, in one buffer.
#[unsafe(no_mangle)]
pub extern "C" fn gp_play_replay(slot: u32, ptr: *mut u8, len: usize, rom_len: usize) -> i32 {
    let bytes = input(ptr, len);
    if rom_len > bytes.len() {
        return result(Err("rom_len is past the buffer".into()));
    }
    let (rom, log) = bytes.split_at(rom_len);
    put(
        slot,
        std::str::from_utf8(log)
            .map_err(|e| e.to_string())
            .and_then(|log| sounding(Player::replay(rom, log))),
    )
}

/// Runs one frame with controller 1's bits in the low byte of `pad` and
/// controller 2's in the next; 1 when the game read the pad,
/// 0 when it didn't, -1 on an error (its text in the output buffer).
#[unsafe(no_mangle)]
pub extern "C" fn gp_play_step(slot: u32, pad: u32, reset: u32) -> i32 {
    match with_player(slot, |p| {
        p.step_pads([pad as u8, (pad >> 8) as u8], reset != 0)
    }) {
        Ok(polled) => i32::from(polled),
        Err(e) => {
            set_out(e.as_bytes());
            -1
        }
    }
}

/// Plays the next frame of a replayed log: 1 if it did, 0 at its end.
#[unsafe(no_mangle)]
pub extern "C" fn gp_play_advance(slot: u32) -> i32 {
    match with_player(slot, |p| p.advance()) {
        Ok(more) => i32::from(more),
        Err(e) => {
            set_out(e.as_bytes());
            -1
        }
    }
}

/// Plays a replayed log to a frame without drawing; returns the frame
/// reached.
#[unsafe(no_mangle)]
pub extern "C" fn gp_play_run_to(slot: u32, frame: u32) -> i32 {
    match with_player(slot, |p| p.run_to(u64::from(frame))) {
        Ok(f) => f as i32,
        Err(e) => {
            set_out(e.as_bytes());
            -1
        }
    }
}

/// The player's whole state ([`Player::save`]) at the returned pointer,
/// `gp_out_len` bytes, valid until the next call; null on an error, with
/// its text in the output buffer.
#[unsafe(no_mangle)]
pub extern "C" fn gp_play_save(slot: u32) -> *const u8 {
    match with_player(slot, |p| p.save()) {
        Ok(bytes) => {
            set_out(&bytes);
            gp_out_ptr()
        }
        Err(e) => {
            set_out(e.as_bytes());
            std::ptr::null()
        }
    }
}

/// Goes back to bytes `gp_play_save` gave, in a buffer from `gp_alloc`.
#[unsafe(no_mangle)]
pub extern "C" fn gp_play_restore(slot: u32, ptr: *mut u8, len: usize) -> i32 {
    let bytes = input(ptr, len);
    result(with_player(slot, |p| {
        p.restore(&bytes).map(|()| String::new())
    }))
}

/// Frames played since power-on.
#[unsafe(no_mangle)]
pub extern "C" fn gp_play_frame(slot: u32) -> u32 {
    with_player(slot, |p| Ok(p.frame() as u32)).unwrap_or(0)
}

/// The last frame's picture: 256 by 240 RGBA at the returned pointer, valid
/// until the next call. Null without a game.
#[unsafe(no_mangle)]
pub extern "C" fn gp_play_rgba(slot: u32) -> *const u8 {
    PLAYERS.with(|c| match c.borrow_mut().get_mut(slot as usize) {
        Some(Some(p)) => p.rgba().as_ptr(),
        _ => std::ptr::null(),
    })
}

/// What was played, as input log text, in the output buffer.
#[unsafe(no_mangle)]
pub extern "C" fn gp_play_input_log(slot: u32) -> i32 {
    result(with_player(slot, |p| Ok(p.input_log())))
}

/// The frame hash digest (hex), in the output buffer.
#[unsafe(no_mangle)]
pub extern "C" fn gp_play_digest(slot: u32) -> i32 {
    result(with_player(slot, |p| Ok(p.digest().to_hex())))
}

/// The current frame's state hash (hex), in the output buffer.
#[unsafe(no_mangle)]
pub extern "C" fn gp_play_hash(slot: u32) -> i32 {
    result(with_player(slot, |p| Ok(p.state_hash().to_hex())))
}

/// The last frame's sound in the output buffer: mono f32 samples (little
/// endian) at `gp_play_audio_rate` a second, `gp_out_len() / 4` of them,
/// about 800 a frame. Copy them out before the next call. Null on an
/// error, with its text in the output buffer. Empty after a restore or a
/// `gp_play_run_to` until the next frame, so a jump has no sound.
#[unsafe(no_mangle)]
pub extern "C" fn gp_play_audio(slot: u32) -> *const u8 {
    match with_player(slot, |p| {
        Ok(p.audio()
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect::<Vec<u8>>())
    }) {
        Ok(bytes) => {
            set_out(&bytes);
            gp_out_ptr()
        }
        Err(e) => {
            set_out(e.as_bytes());
            std::ptr::null()
        }
    }
}

/// Samples a second of `gp_play_audio`.
#[unsafe(no_mangle)]
pub extern "C" fn gp_play_audio_rate() -> u32 {
    crate::AUDIO_RATE
}
