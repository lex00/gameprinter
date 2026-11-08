//! A demo hook for tests and docs, registered for the model `hooks_demo`:
//! `demo_turn` counts frames in its body's first `ext` word and turns the
//! entity round every 30 frames.

use gp_sim::{Ext, ExtCx, Exts};

struct Turn;

impl Ext for Turn {
    fn step(&self, cx: &mut ExtCx<'_>) {
        cx.me.ext[0] += 1;
        if cx.me.ext[0] % 30 == 0 {
            cx.me.facing_left = !cx.me.facing_left;
        }
    }
}

pub fn exts() -> Exts {
    let mut e = Exts::new();
    e.insert("demo_turn".into(), Box::new(Turn));
    e
}
