//! gp_board.wasm's exports, wasm32 only, in gp-web's shape: bytes go in
//! through buffers from `gp_alloc`, and what comes back is in the output
//! buffer (`gp_out_ptr`, `gp_out_len`). A function returns 0 when it
//! worked and 1 with the error's text when it didn't.

#![allow(unsafe_code)]

use std::cell::RefCell;

use gp_look::board::{BoardTheme, Companion};

thread_local! {
    static BOARD: RefCell<Option<Companion>> = const { RefCell::new(None) };
    static THEME: RefCell<Option<BoardTheme>> = const { RefCell::new(None) };
    static OUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

fn set_out(bytes: &[u8]) {
    OUT.with(|o| {
        let mut o = o.borrow_mut();
        o.clear();
        o.extend_from_slice(bytes);
    });
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

/// Opens the board art (`crate::open`): the model bundle the game was
/// loaded from, `assets/present.txt` and the packed lifted files. The
/// output buffer holds `1` when the model has board art, `0` when it
/// names none.
#[unsafe(no_mangle)]
pub extern "C" fn gp_board_open(
    bundle: *mut u8,
    bundle_len: usize,
    present: *mut u8,
    present_len: usize,
    files: *mut u8,
    files_len: usize,
) -> i32 {
    let (b, p, f) = (
        input(bundle, bundle_len),
        input(present, present_len),
        input(files, files_len),
    );
    THEME.with(|t| *t.borrow_mut() = None);
    match crate::open(&b, &p, &f) {
        Ok(c) => {
            set_out(if c.is_some() { b"1" } else { b"0" });
            BOARD.with(|x| *x.borrow_mut() = c);
            0
        }
        Err(e) => {
            BOARD.with(|x| *x.borrow_mut() = None);
            set_out(e.as_bytes());
            1
        }
    }
}

/// Reads the open model's positions as the board's own units (`on` 1, its
/// state mapping's `board_units`, which preview.json's `board_unit` says)
/// or as the sprite RAM's bytes (0, how it opens). Returns 0, or 2 with no
/// board art open.
#[unsafe(no_mangle)]
pub extern "C" fn gp_board_units(on: i32) -> i32 {
    BOARD.with(|x| match x.borrow_mut().as_mut() {
        Some(c) => {
            c.set_board_units(on != 0);
            0
        }
        None => 2,
    })
}

/// The board's picture of a frame (`crate::picture_bytes`) from the game's
/// canonical state and which bodies its scene shows (a byte each, 1 shown).
/// Returns 0 with the picture, 2 when there is none (no board art open, or
/// a level without it), 1 on an error.
#[unsafe(no_mangle)]
pub extern "C" fn gp_board_frame(
    state: *mut u8,
    state_len: usize,
    shown: *mut u8,
    shown_len: usize,
) -> i32 {
    let (st, sh) = (input(state, state_len), input(shown, shown_len));
    let shown: Vec<bool> = sh.iter().map(|b| *b != 0).collect();
    let r = BOARD.with(|x| match x.borrow().as_ref() {
        Some(c) => c.frame(&st, &shown),
        None => Ok(None),
    });
    match r {
        Ok(Some(pic)) => {
            set_out(&crate::picture_bytes(&pic));
            0
        }
        Ok(None) => {
            set_out(b"");
            2
        }
        Err(e) => {
            set_out(e.as_bytes());
            1
        }
    }
}

/// Loads a large-art theme's rules (`gp-art 1`, the file its `look.txt`
/// names with `art`) over the board's named pictures
/// (docs/formats/board-art.md, "Themes"); an empty text takes them away.
thread_local! {
    /// The seed the next theme's rooms pick their pieces' alternatives by.
    static ROOM_SEED: std::cell::RefCell<u64> = const { std::cell::RefCell::new(0) };
}

/// Sets the seed the next `gp_board_load_art` picks a room's piece
/// alternatives by (`#+ room_box picture a|b`): the page gives a random one
/// before each load, so each alternative is as likely. Two halves, low first.
#[unsafe(no_mangle)]
pub extern "C" fn gp_board_room_seed(lo: u32, hi: u32) {
    ROOM_SEED.with(|s| *s.borrow_mut() = u64::from(lo) | (u64::from(hi) << 32));
}

/// The output buffer holds the art files the frames name by number, as
/// `gp_load_art` gives them: `{"files": [...], "stages": []}`, and the
/// rooms it built over the board's screens (`"rooms"`, `crate::files_json`).
#[unsafe(no_mangle)]
pub extern "C" fn gp_board_load_art(text: *mut u8, text_len: usize) -> i32 {
    let t = input(text, text_len);
    let Ok(t) = std::str::from_utf8(&t) else {
        set_out(b"the art rules aren't UTF-8");
        return 1;
    };
    if t.trim().is_empty() {
        THEME.with(|x| *x.borrow_mut() = None);
        set_out(br#"{"files":[],"stages":[]}"#);
        return 0;
    }
    let r = BOARD.with(|x| match x.borrow().as_ref() {
        Some(c) => c.art().theme_seeded(t, ROOM_SEED.with(|s| *s.borrow())),
        None => Err("no board art is open".to_owned()),
    });
    match r {
        Ok(theme) => {
            set_out(crate::files_json(&theme).as_bytes());
            THEME.with(|x| *x.borrow_mut() = Some(theme));
            0
        }
        Err(e) => {
            set_out(e.as_bytes());
            1
        }
    }
}

/// A frame drawn in the theme `gp_board_load_art` loaded
/// (`crate::themed_bytes`), from the same state and shown bodies as
/// `gp_board_frame`. Returns 0 with it, 2 when there is none (no board
/// art or no theme loaded, or a level without board art), 1 on an error.
#[unsafe(no_mangle)]
pub extern "C" fn gp_board_art_frame(
    state: *mut u8,
    state_len: usize,
    shown: *mut u8,
    shown_len: usize,
) -> i32 {
    let (st, sh) = (input(state, state_len), input(shown, shown_len));
    let shown: Vec<bool> = sh.iter().map(|b| *b != 0).collect();
    let r = BOARD.with(|x| {
        THEME.with(|t| match (x.borrow().as_ref(), t.borrow_mut().as_mut()) {
            (Some(c), Some(theme)) => c.themed(theme, &st, &shown),
            _ => Ok(None),
        })
    });
    match r {
        Ok(Some(t)) => {
            set_out(&crate::themed_bytes(&t));
            0
        }
        Ok(None) => {
            set_out(b"");
            2
        }
        Err(e) => {
            set_out(e.as_bytes());
            1
        }
    }
}
