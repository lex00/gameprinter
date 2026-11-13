//! What the static reader knows of a board without running it: where the
//! program ROM ends, which addresses are RAM, and what the I/O ports are.
//! The Midway MCR 91490 CPU board's map is mcr.rs's (`main_read`,
//! `main_write`, `main_io_read`, `main_io_write`).

/// A CPU's view of its board.
#[derive(Debug, Clone)]
pub struct Board {
    /// The program ROM from address 0.
    pub rom: Vec<u8>,
    /// The first address past the ROM.
    pub rom_end: u32,
    /// Work RAM: where the game's variables live.
    pub ram: (u16, u16),
    /// Other RAM the CPU writes: sprite RAM, video RAM, the palette, with a
    /// name each.
    pub other: Vec<(u16, u16, &'static str)>,
    /// The CTC's four channel ports, channel 0 first, when the board has
    /// one: an IM 2 vector is written to channel 0's.
    pub ctc: Option<u8>,
    /// Input ports: a port whose low byte masked with `.0` is in `.1`.
    pub inputs: (u8, std::ops::RangeInclusive<u8>),
    /// Named output ports.
    pub outputs: Vec<(std::ops::RangeInclusive<u8>, &'static str)>,
}

impl Board {
    /// The MCR main CPU (Tapper's): ROM at 0000-DFFF, work RAM E000-E7FF.
    pub fn mcr_main(rom: &[u8]) -> Board {
        Board {
            rom: rom.to_vec(),
            rom_end: 0xe000,
            ram: (0xe000, 0xe800),
            other: vec![
                (0xe800, 0xec00, "sprite RAM"),
                (0xf000, 0xf800, "video RAM"),
                (0xf800, 0x0000, "palette"),
            ],
            ctc: Some(0xf0),
            inputs: (!0x18, 0x00..=0x04),
            outputs: vec![
                (0x00..=0x03, "SSIO output"),
                (0x1c..=0x1f, "sound latch"),
                (0xe0..=0xe0, "watchdog"),
                (0xe8..=0xe8, "board control"),
                (0xf0..=0xf3, "CTC"),
            ],
        }
    }

    /// A flat board for tests: `rom` from 0, RAM at `ram`.
    pub fn flat(rom: &[u8], ram: (u16, u16)) -> Board {
        Board {
            rom: rom.to_vec(),
            rom_end: rom.len() as u32,
            ram,
            other: Vec::new(),
            ctc: Some(0xf0),
            inputs: (0xff, 0x00..=0x04),
            outputs: Vec::new(),
        }
    }

    pub fn is_rom(&self, a: u16) -> bool {
        u32::from(a) < self.rom_end
    }

    pub fn is_ram(&self, a: u16) -> bool {
        (self.ram.0..self.ram.1).contains(&a)
    }

    /// The ROM byte at `a` (0 past the end).
    pub fn read(&self, a: u16) -> u8 {
        self.rom.get(usize::from(a)).copied().unwrap_or(0)
    }

    pub fn word(&self, a: u16) -> u16 {
        u16::from_le_bytes([self.read(a), self.read(a.wrapping_add(1))])
    }

    /// Up to four bytes from `a`, for the decoders.
    pub fn bytes(&self, a: u16) -> [u8; 4] {
        [0u16, 1, 2, 3].map(|i| self.read(a.wrapping_add(i)))
    }

    /// Whether the port is an input the board's players or switches drive.
    pub fn is_input(&self, port: u8) -> bool {
        self.inputs.1.contains(&(port & self.inputs.0))
    }

    /// The name of an output port.
    pub fn output(&self, port: u8) -> Option<&'static str> {
        self.outputs
            .iter()
            .find(|(r, _)| r.contains(&port))
            .map(|(_, n)| *n)
    }

    /// The RAM region an address is in, by name.
    pub fn region(&self, a: u16) -> &'static str {
        if self.is_rom(a) {
            return "ROM";
        }
        if self.is_ram(a) {
            return "work RAM";
        }
        self.other
            .iter()
            .find(|(s, e, _)| a >= *s && (*e == 0 || a < *e))
            .map_or("unmapped", |r| r.2)
    }
}
