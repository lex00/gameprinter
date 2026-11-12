//! Names for PRG offsets as the static harvest writes them: `C123` in a
//! region nothing switches, `03:8123` in a switched one (the bank number
//! being the offset over the region's size), so a run's addresses and the
//! harvest's code map compare as text.

use gp_nes::codemap::Board;

/// Names PRG offsets of one ROM.
#[derive(Debug, Clone)]
pub struct Namer {
    board: Board,
}

impl Namer {
    /// For an iNES ROM; `None` if it isn't one.
    pub fn of_ines(rom: &[u8]) -> Option<Namer> {
        let (prg, mapper) = gp_nes::harvest::ines_prg(rom)?;
        Some(Namer {
            board: Board::of(prg, mapper),
        })
    }

    /// For a board as given (one with regions runs showed switched).
    pub fn of_board(board: Board) -> Namer {
        Namer { board }
    }

    /// The regions runs show switched that the board's usual layout fixes:
    /// a region of the switch size (16 KB on UxROM and MMC1, 8 KB on MMC3)
    /// where some PRG 1 KB page ran that the power-on view doesn't put
    /// there. `windows` is L0's: for each PRG 1 KB page, the CPU 1 KB pages
    /// it ran at.
    pub fn observed_switched(board: &Board, windows: &[u64]) -> Vec<(u16, u32)> {
        let size: u32 = match board.mapper {
            1 | 2 => 0x4000,
            4 => 0x2000,
            _ => return Vec::new(),
        };
        if !gp_nes::codemap::banked(&board.prg) {
            return Vec::new();
        }
        let mut out: Vec<(u16, u32)> = Vec::new();
        for (page, &cpus) in windows.iter().enumerate() {
            for cp in 32..64u32 {
                if cpus & (1u64 << cp) == 0 {
                    continue;
                }
                let cpu = (cp * 0x400) as u16;
                if board.region(cpu).is_some() {
                    continue;
                }
                // Where the fixed view puts this CPU page.
                let fixed = board.prg_offset(u32::from(cpu)).map(|o| o / 0x400);
                if fixed == Some(page) {
                    continue;
                }
                let start = (u32::from(cpu) & !(size - 1)) as u16;
                if !out.contains(&(start, size)) {
                    out.push((start, size));
                }
            }
        }
        out.sort_unstable();
        out
    }

    /// PRG offset `offset`, which ran at CPU address `cpu`, as the
    /// harvest's banked address (`gp_nes::codemap::Va`): the bank and the
    /// CPU address in a switched region, the CPU address alone elsewhere.
    /// `None` for code outside PRG ROM.
    pub fn va(&self, offset: u32, cpu: u16) -> Option<u32> {
        if offset == crate::edges::NO_ROM {
            return None;
        }
        let v = match self.board.region(cpu) {
            Some((_, size)) => ((offset / size + 1) << 16) | u32::from(cpu),
            None => u32::from(cpu),
        };
        // Only an address that names this very byte: a board whose banks
        // the harvest doesn't model (AxROM, CNROM's PRG, an MMC3 mode it
        // doesn't follow) can run a byte where the board's view has another.
        (self.board.prg_offset(v) == Some(offset as usize)).then_some(v)
    }

    /// The name of PRG offset `offset`, which ran at CPU address `cpu`: the
    /// harvest's name for it when [`Namer::va`] gives one, else the PRG
    /// offset with the CPU address (`prg:1C123@8123`).
    pub fn name(&self, offset: u32, cpu: u16) -> String {
        if offset == crate::edges::NO_ROM {
            return format!("ram:{cpu:04X}");
        }
        match self.va(offset, cpu) {
            Some(v) => gp_nes::codemap::va_hex(v),
            None => format!("prg:{offset:05X}@{cpu:04X}"),
        }
    }
}
