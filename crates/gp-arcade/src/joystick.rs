// license:BSD-3-Clause
// copyright-holders:Aaron Giles
//
// A port of digital_joystick::frame_update from MAME 0.276
// (src/emu/ioport.cpp), with the 4-way mask ioport_field::frame_update
// applies to each direction of a PORT_4WAY joystick.

//! MAME's 4-way joystick filter: what a player's joystick or keys put on
//! Tapper's four direction bits.
//!
//! MAME filters what a player presses, not what a script sets: a field
//! given a value with `set_value` (the oracle's default) skips the filter,
//! so a log that holds two directions reaches the board as both. The
//! oracle can send a log through the filter instead (`joystick 4way` in
//! its header, docs/oracle-mame.md), and so can the mechanic page, where a
//! person plays. Either way the filter runs once a frame, on that frame's
//! directions, and keeps what it last chose.

use crate::mcr::Controls;

const UP: u8 = 1 << 0;
const DOWN: u8 = 1 << 1;
const LEFT: u8 = 1 << 2;
const RIGHT: u8 = 1 << 3;

/// One digital joystick's state across frames.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FourWay {
    current: u8,
    current4way: u8,
    previous: u8,
}

impl FourWay {
    /// The directions the board sees this frame, from those held: opposing
    /// directions cancel; a diagonal keeps the direction that just changed,
    /// or the horizontal one when both did; a diagonal held on keeps the
    /// choice it made. The other controls pass through.
    pub fn filter(&mut self, held: Controls) -> Controls {
        self.previous = self.current;
        self.current = 0;
        for (on, bit) in [
            (held.up, UP),
            (held.down, DOWN),
            (held.left, LEFT),
            (held.right, RIGHT),
        ] {
            if on {
                self.current |= bit;
            }
        }
        if self.current & (UP | DOWN) == UP | DOWN {
            self.current &= !(UP | DOWN);
        }
        if self.current & (LEFT | RIGHT) == LEFT | RIGHT {
            self.current &= !(LEFT | RIGHT);
        }
        if self.current != self.previous {
            self.current4way = self.current;
            let diagonal = |v: u8| v & (UP | DOWN) != 0 && v & (LEFT | RIGHT) != 0;
            if diagonal(self.current4way) {
                self.current4way ^= self.current4way & self.previous;
            }
            if diagonal(self.current4way) {
                self.current4way &= !(UP | DOWN);
            }
        }
        // ioport_field::frame_update: a held direction counts only when the
        // 4-way mask has it, so a direction not held never appears.
        let mask = self.current4way;
        Controls {
            up: held.up && mask & UP != 0,
            down: held.down && mask & DOWN != 0,
            left: held.left && mask & LEFT != 0,
            right: held.right && mask & RIGHT != 0,
            ..held
        }
    }

    /// The state, for a snapshot: current, current4way, previous.
    pub fn save(&self) -> [u8; 3] {
        [self.current, self.current4way, self.previous]
    }

    pub fn load(v: [u8; 3]) -> Self {
        Self {
            current: v[0],
            current4way: v[1],
            previous: v[2],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dirs(s: &str) -> Controls {
        Controls {
            up: s.contains('U'),
            down: s.contains('D'),
            left: s.contains('L'),
            right: s.contains('R'),
            ..Controls::default()
        }
    }

    fn run(seq: &[&str]) -> Vec<String> {
        let mut f = FourWay::default();
        seq.iter()
            .map(|s| {
                let c = f.filter(dirs(s));
                [(c.up, 'U'), (c.down, 'D'), (c.left, 'L'), (c.right, 'R')]
                    .iter()
                    .filter(|(on, _)| *on)
                    .map(|(_, ch)| *ch)
                    .collect()
            })
            .collect()
    }

    #[test]
    fn a_diagonal_from_rest_goes_horizontal() {
        assert_eq!(run(&["", "UR", "UR"]), ["", "R", "R"]);
    }

    #[test]
    fn a_diagonal_keeps_the_direction_that_changed() {
        assert_eq!(run(&["L", "UL", "UL", "U"]), ["L", "U", "U", "U"]);
        assert_eq!(run(&["U", "UL", "L"]), ["U", "L", "L"]);
    }

    #[test]
    fn opposing_directions_cancel() {
        assert_eq!(run(&["U", "UD", "LR", "LRU"]), ["U", "", "", "U"]);
    }

    #[test]
    fn single_directions_pass_through() {
        assert_eq!(run(&["U", "D", "", "R", "L"]), ["U", "D", "", "R", "L"]);
    }
}
