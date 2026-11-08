//! A gamepad's analog stick read as the NES D-pad, for the runtimes that
//! take a gamepad (the generated `play`). The stick only adds directions to
//! the buttons held, the same bits an input log carries (Up, Down, Left,
//! Right in bits 4 to 7), so the game sees a pad and never a float.

/// How far the stick must lean, of its full travel, to hold a direction.
pub const STICK_HOLD: f32 = 0.5;

/// The D-pad bits a stick at (`x`, `y`) holds, `y` up as Bevy reads it:
/// Up is bit 4, Down 5, Left 6 and Right 7. A diagonal past
/// [`STICK_HOLD`] on both axes holds both.
pub fn stick_buttons(x: f32, y: f32) -> u8 {
    let mut b = 0;
    if y >= STICK_HOLD {
        b |= 1 << 4;
    }
    if y <= -STICK_HOLD {
        b |= 1 << 5;
    }
    if x <= -STICK_HOLD {
        b |= 1 << 6;
    }
    if x >= STICK_HOLD {
        b |= 1 << 7;
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stick_holds_the_directions_it_leans() {
        assert_eq!(stick_buttons(0.0, 0.0), 0);
        assert_eq!(stick_buttons(0.3, -0.4), 0);
        assert_eq!(stick_buttons(0.0, 1.0), 0x10);
        assert_eq!(stick_buttons(0.0, -0.5), 0x20);
        assert_eq!(stick_buttons(-0.9, 0.1), 0x40);
        assert_eq!(stick_buttons(0.7, 0.0), 0x80);
        assert_eq!(stick_buttons(0.7, 0.7), 0x90);
        assert_eq!(stick_buttons(-0.6, -0.6), 0x60);
    }
}
