//! The machine blind discovery and its search play: gp-nes for an NES
//! image, gp-arcade for a romset (port, Wave T). Both have 2 KB of work
//! RAM, which is what discovery searches (the NES's $0000-$07FF; the MCR
//! board's E000-E7FF, read here from index 0). The sprites are boxes in
//! the game's own units: the NES's 8-pixel sprites from its sprite table,
//! the MCR board's 32x32 sprites from gp-arcade's sprite list, 16x16 in
//! its half-pixel units on a 256x240 screen. The board has no scroll.
//!
//! The buttons come from the device the platform's model uses
//! ([`gp_model::Device`]): the stick's four ways, the start button, and
//! the rest as the actions discovery tries for jump and run, in the
//! device's order. A frame's input is an [`InputFrame`] with the device's
//! bits in `pads[0]`, as stage 0's arcade starts write them.

use std::path::Path;

use anyhow::{Result, anyhow, bail};
use gp_evidence::input_log::InputFrame;
use gp_model::model::{Button, Device};

use super::support as m;

/// The buttons of a device, as bits of `pads[0]`.
#[derive(Clone, Debug)]
pub(crate) struct Pads {
    pub device: Device,
    pub right: u8,
    pub left: u8,
    pub up: u8,
    pub down: u8,
    pub start: u8,
    /// Bits that put a coin in before Start (none on the NES).
    pub coin: u8,
    /// The buttons that aren't the stick, start, select or coin, in the
    /// device's order: what discovery tries for jump and run.
    pub actions: Vec<u8>,
}

impl Pads {
    pub(crate) fn of(device: Device) -> Pads {
        let bit = |b: Button| device.bit(b).unwrap_or(0);
        let actions = device
            .buttons()
            .iter()
            .filter(|b| {
                !matches!(
                    b,
                    Button::Up
                        | Button::Down
                        | Button::Left
                        | Button::Right
                        | Button::Start
                        | Button::Start1
                        | Button::Select
                        | Button::Coin
                )
            })
            .map(|b| bit(*b))
            .collect();
        Pads {
            device,
            right: bit(Button::Right),
            left: bit(Button::Left),
            up: bit(Button::Up),
            down: bit(Button::Down),
            start: bit(Button::Start) | bit(Button::Start1),
            coin: bit(Button::Coin),
            actions,
        }
    }

    /// The (jump, run) pairs discovery tries, in turn: each action as jump
    /// with the next as run, then Up as jump with the first action as run.
    /// The NES pad gives (A, B), (B, A), (Up, A), the order game-printer
    /// had; the arcade device's one button gives (tap, none), (Up, tap).
    pub(crate) fn jump_tries(&self) -> Vec<(u8, u8)> {
        let n = self.actions.len();
        let mut out: Vec<(u8, u8)> = (0..n)
            .map(|i| {
                (
                    self.actions[i],
                    if n > 1 { self.actions[(i + 1) % n] } else { 0 },
                )
            })
            .collect();
        out.push((self.up, self.actions.first().copied().unwrap_or(0)));
        out
    }

    /// A button's name in the model's `input.actions`.
    pub(crate) fn name(&self, bit: u8) -> &'static str {
        let i = bit.trailing_zeros() as usize;
        match self.device.buttons().get(i) {
            Some(b) => match b {
                Button::A => "a",
                Button::B => "b",
                Button::Select => "select",
                Button::Start => "start",
                Button::Up => "up",
                Button::Down => "down",
                Button::Left => "left",
                Button::Right => "right",
                Button::Coin => "coin",
                Button::Start1 => "start1",
                Button::Button1 => "button1",
            },
            None => "none",
        }
    }
}

/// The ROM's identity: an NES image's SHA-256, a romset's identity (the
/// SHA-256 of its chips' SHA-1s, gp_arcade::romset).
pub(crate) fn rom_sha256(bytes: &[u8]) -> Result<String> {
    Ok(match crate::original::Platform::of_file(bytes) {
        crate::original::Platform::Nes => gp_evidence::source::identify_nes(bytes)?.sha256,
        crate::original::Platform::Arcade => {
            gp_arcade::Romset::from_zip(bytes)
                .map_err(|e| anyhow!("{e}"))?
                .identity
        }
    })
}

/// The original running, on the emulator its file's form says.
#[allow(clippy::large_enum_variant)]
#[derive(Clone)]
pub(crate) enum Machine {
    Nes(gp_nes::Nes),
    Arcade(Box<gp_arcade::Tapper>),
}

impl Machine {
    /// Powers on an NES image (with `power_on`'s RAM) or a romset zip.
    pub(crate) fn power_on(bytes: &[u8], power_on: gp_evidence::manifest::PowerOn) -> Result<Self> {
        Ok(match crate::original::Platform::of_file(bytes) {
            crate::original::Platform::Nes => {
                let mut nes = gp_nes::Nes::power_on(bytes, power_on)?;
                nes.set_video(false);
                Machine::Nes(nes)
            }
            crate::original::Platform::Arcade => {
                let set = gp_arcade::Romset::from_zip(bytes).map_err(|e| anyhow!("{e}"))?;
                Machine::Arcade(Box::new(gp_arcade::Tapper::power_on(&set)))
            }
        })
    }

    /// The NES underneath, for the tools that only the NES has (the sweep).
    pub(crate) fn as_nes(&self) -> Result<&gp_nes::Nes> {
        match self {
            Machine::Nes(n) => Ok(n),
            Machine::Arcade(_) => {
                bail!("this runs on the NES only, and the ROM is an arcade romset")
            }
        }
    }

    pub(crate) fn is_arcade(&self) -> bool {
        matches!(self, Machine::Arcade(_))
    }

    pub(crate) fn pads(&self) -> Pads {
        Pads::of(match self {
            Machine::Nes(_) => Device::NesPad,
            Machine::Arcade(_) => Device::Arcade,
        })
    }

    pub(crate) fn step(&mut self, f: InputFrame) -> Result<()> {
        match self {
            Machine::Nes(n) => {
                n.step(f)?;
            }
            Machine::Arcade(t) => {
                if f.reset {
                    bail!("the arcade board's reset isn't modelled");
                }
                t.step(gp_arcade::Controls::from_bits(u32::from(f.pads[0])));
            }
        }
        Ok(())
    }

    /// The 2 KB of work RAM discovery searches.
    pub(crate) fn wram(&self) -> &[u8] {
        match self {
            Machine::Nes(n) => &n.wram()[..0x800],
            Machine::Arcade(t) => &t.board.nvram[..],
        }
    }

    /// Where the work RAM's first byte is in the CPU's address space.
    pub(crate) fn ram_base(&self) -> usize {
        match self {
            Machine::Nes(_) => 0,
            Machine::Arcade(_) => 0xE000,
        }
    }

    /// Bytes of work RAM that change around anything and hold no state:
    /// the NES's stack page. The board has none set apart.
    pub(crate) fn noise(&self, a: usize) -> bool {
        match self {
            Machine::Nes(_) => (0x100..0x200).contains(&a),
            Machine::Arcade(_) => false,
        }
    }

    /// The NES's scratch bytes ($00-$0F, where programs keep a copy of the
    /// player's x while drawing it): passed over when anything else will
    /// do. The board has no such habit.
    pub(crate) fn scratch(&self, a: usize) -> bool {
        match self {
            Machine::Nes(_) => a < 0x10,
            Machine::Arcade(_) => false,
        }
    }

    /// The frame's sprite boxes, those that touch merged, as [x, y, w, h].
    pub(crate) fn boxes(&self) -> Vec<[i32; 4]> {
        match self {
            Machine::Nes(n) => m::sprite_boxes(&n.oam(), if n.tall_sprites() { 16 } else { 8 }),
            Machine::Arcade(t) => m::arcade_boxes(&t.board.spriteram),
        }
    }

    /// Each sprite's own box, unmerged, [x, y, w, h]: the NES's 8x8 cells
    /// from its sprite table (as `gp model blind` measures size from), the
    /// board's 16x16 sprites.
    pub(crate) fn cells(&self) -> Vec<[i64; 4]> {
        match self {
            Machine::Nes(n) => n
                .oam()
                .chunks(4)
                .filter(|e| e[0] < 0xEF)
                .map(|e| [i64::from(e[3]), i64::from(e[0]) + 1, 8, 8])
                .collect(),
            Machine::Arcade(t) => gp_arcade::video::sprite_list(&t.board.spriteram)
                .iter()
                .map(|s| [i64::from(s.x), i64::from(s.y), 16, 16])
                .collect(),
        }
    }

    /// The left edges discovery's "drawn" test looks for x at: the NES's
    /// merged boxes (a body of 8-pixel sprites), the board's sprites one by
    /// one. Merged on the board, the player's box takes in a customer or a
    /// mug it touches, its left edge then theirs, and x failed the test
    /// whenever it walked into one.
    pub(crate) fn drawn_xs(&self) -> Vec<i64> {
        match self {
            Machine::Nes(_) => self.boxes().iter().map(|b| i64::from(b[0])).collect(),
            Machine::Arcade(_) => self.cells().iter().map(|c| c[0]).collect(),
        }
    }

    /// The horizontal scroll in pixels (0 on the board).
    pub(crate) fn scroll(&self) -> i64 {
        match self {
            Machine::Nes(n) => {
                let sc = &n.bus().ppu.scroll;
                i64::from(
                    (sc.t & 31) * 8 + u16::from(sc.fine_x as u8 & 7) + ((sc.t >> 10) & 1) * 256,
                )
            }
            Machine::Arcade(_) => 0,
        }
    }

    pub(crate) fn snapshot(&self) -> Result<Vec<u8>> {
        Ok(match self {
            Machine::Nes(n) => n.snapshot()?,
            Machine::Arcade(t) => t.snapshot(),
        })
    }

    pub(crate) fn frame(&self) -> u64 {
        match self {
            Machine::Nes(n) => n.frame(),
            Machine::Arcade(t) => t.frame(),
        }
    }

    pub(crate) fn held(&self) -> [u8; 4] {
        match self {
            Machine::Nes(n) => n.held(),
            Machine::Arcade(_) => [0; 4],
        }
    }

    /// Back to a snapshot; the board's holds its own frame and inputs.
    pub(crate) fn restore(&mut self, snap: &[u8], frame: u64, held: [u8; 4]) -> Result<()> {
        match self {
            Machine::Nes(n) => n.restore(snap, frame, held)?,
            Machine::Arcade(t) => t.restore(snap).map_err(|e| anyhow!("{e}"))?,
        }
        Ok(())
    }

    /// A picture of the next frame as a PNG; the machine is put back.
    pub(crate) fn shot(&mut self, to: &Path) -> Result<()> {
        let (snap, frame, held) = (self.snapshot()?, self.frame(), self.held());
        let (w, h, rgb) = match self {
            Machine::Nes(n) => {
                n.set_video(true);
                n.step(InputFrame::default())?;
                let mut rgb = Box::new([0u8; 256 * 240 * 3]);
                n.frame_rgb(&mut rgb);
                n.set_video(false);
                (256, 240, rgb.to_vec())
            }
            Machine::Arcade(t) => {
                t.step(gp_arcade::Controls::default());
                let rgb = t
                    .picture()
                    .iter()
                    .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8])
                    .collect();
                (
                    gp_arcade::video::WIDTH as u32,
                    gp_arcade::video::HEIGHT as u32,
                    rgb,
                )
            }
        };
        self.restore(&snap, frame, held)?;
        if let Some(d) = to.parent() {
            std::fs::create_dir_all(d)?;
        }
        let mut enc = png::Encoder::new(std::fs::File::create(to)?, w, h);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header()?.write_image_data(&rgb)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_devices_give_discovery_their_buttons() {
        let nes = Pads::of(Device::NesPad);
        assert_eq!(
            (nes.right, nes.left, nes.start, nes.coin),
            (0x80, 0x40, 0x08, 0)
        );
        assert_eq!(nes.jump_tries(), [(0x01, 0x02), (0x02, 0x01), (0x10, 0x01)]);
        assert_eq!((nes.name(0x01), nes.name(0x02)), ("a", "b"));
        let arc = Pads::of(Device::Arcade);
        assert_eq!(
            (arc.right, arc.left, arc.up, arc.start, arc.coin),
            (0x04, 0x08, 0x20, 0x02, 0x01)
        );
        assert_eq!(arc.jump_tries(), [(0x40, 0), (0x20, 0x40)]);
        assert_eq!(arc.name(0x40), "button1");
    }
}
