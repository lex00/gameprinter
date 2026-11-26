//! gp_controls.wasm's exports, wasm32 only: numbers in, numbers out, no
//! memory shared with the page. scripts/web/page/controls.js is the one
//! caller; every page that plays a game imports it.

#![allow(unsafe_code)]

use std::cell::RefCell;

use crate::{Controls, Layout, Options, RawPad};

thread_local! {
    static CONTROLS: RefCell<Controls> = RefCell::new(Controls::default());
}

/// Starts the controls over in a layout: 1 an arcade cabinet's, else the
/// NES's. The settings stay.
#[unsafe(no_mangle)]
pub extern "C" fn gp_controls_layout(arcade: u32) {
    CONTROLS.with(|c| {
        let mut c = c.borrow_mut();
        let opts = c.options();
        let layout = if arcade == 1 {
            Layout::Arcade
        } else {
            Layout::Nes
        };
        *c = Controls::new(layout, opts);
    });
}

/// The timing's settings: the lead and the assist in ms, and the lever's
/// travel as a percent of its reach. Each is kept inside its range.
#[unsafe(no_mangle)]
pub extern "C" fn gp_controls_options(lead_ms: f64, assist_ms: f64, stick_travel: f64) {
    CONTROLS.with(|c| {
        c.borrow_mut().set_options(Options {
            lead_ms,
            assist_ms,
            stick_travel,
        });
    });
}

/// A connected gamepad as it reads now: the browser's index, its pressed
/// buttons (bit i for `buttons[i]`) and its first four axes (NaN for one
/// it hasn't got). Called for each pad in the browser's order, before
/// [`gp_controls_set`].
#[unsafe(no_mangle)]
pub extern "C" fn gp_controls_pad(index: u32, buttons: u32, a0: f32, a1: f32, a2: f32, a3: f32) {
    CONTROLS.with(|c| {
        c.borrow_mut().pad(RawPad {
            index,
            buttons,
            axes: [a0, a1, a2, a3],
        });
    });
}

/// The controls at time `t` (ms): the pads handed in since the last call
/// with the keyboard's buttons `keys` in the layout's bits. The buttons
/// they give.
#[unsafe(no_mangle)]
pub extern "C" fn gp_controls_set(keys: u32, t: f64) -> u32 {
    CONTROLS.with(|c| c.borrow_mut().set(keys, t))
}

/// The buttons the game frame at time `t` reads, at `now`.
#[unsafe(no_mangle)]
pub extern "C" fn gp_controls_take(t: f64, now: f64) -> u32 {
    CONTROLS.with(|c| c.borrow_mut().take(t, now))
}
