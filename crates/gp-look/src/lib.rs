//! What a player draws and plays from a model and its lifted assets: the
//! pictures, the looks a game offers, and the sound bundle. Every player
//! uses this code, so a look drawn at build time and one drawn in the
//! browser from a person's ROM are the same bytes.

use std::path::Path;

use gp_interp::{Bundle, Lifted};
use gp_model::Model;
use gp_sim::Spec;

pub mod looks;
pub mod present;
pub mod sound;

/// The frames file's header line.
pub const FRAMES_HEADER: &str = "gp-frames 1";

/// BLAKE3 of a spec's debug form.
pub fn spec_digest(spec: &Spec) -> String {
    gp_sim::save::spec_digest(spec)
}

/// Reads the lifted files the simulation reads into a bundle.
pub fn bundle(model: &Model, assets: &Path) -> Result<Bundle, String> {
    Ok(Bundle::new(model, Lifted::read_dir(assets, model)?))
}

/// One line per frame: the controller bits in hex, then `r` on a reset.
pub fn frames_file(frames: &[(u8, bool)]) -> String {
    let mut s = format!("{FRAMES_HEADER}\n");
    for (b, r) in frames {
        s.push_str(&format!("{b:02x}{}\n", if *r { " r" } else { "" }));
    }
    s
}
