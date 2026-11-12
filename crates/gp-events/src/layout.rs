//! How a cartridge's PRG ROM and PRG RAM sit in the console's memory, so
//! a CPU address read now can be named by its PRG offset: the bank and the
//! address together. Every level of a run is keyed by PRG offset.

use tetanes_core::bus::Bus;
use tetanes_core::memory::Src;

/// Where `tetanes-core` puts PRG ROM in its memory arena: right after the
/// reserved zero page that unmapped pages point at (`Memory::new`). The
/// arena's region offsets aren't public, so [`Layout::of`] checks this
/// against the console it is given.
const PRG_ROM_START: usize = 1024;

/// Size of one page of the CPU address space, as `tetanes-core` maps it.
pub const PAGE: usize = 1024;

/// Internal RAM at $0000-$07FF, mirrored to $1FFF.
pub const RAM_BYTES: usize = 0x800;

#[derive(Debug, thiserror::Error)]
pub enum LayoutError {
    #[error(
        "tetanes-core's memory layout isn't the one coverage expects: CPU ${addr:04X} reads PRG ROM byte {offset} and gets {got:#04x}, not {want:#04x}"
    )]
    Layout {
        addr: u16,
        offset: usize,
        got: u8,
        want: u8,
    },
}

/// How a cartridge's PRG ROM and PRG RAM sit in the console's memory arena.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    pub prg_rom: usize,
    prg_ram_start: usize,
    pub prg_ram: usize,
}

impl Layout {
    /// Reads the layout from a console, checking that every CPU page mapped
    /// to PRG ROM reads the ROM bytes the layout says it does.
    pub fn of(bus: &Bus) -> Result<Self, LayoutError> {
        let rom = bus.memory.region_ref(Src::PrgRom);
        let ram = bus.memory.region_ref(Src::PrgRam);
        // Regions are slices of one arena, so their distance is their
        // distance in it.
        let prg_ram_start =
            PRG_ROM_START + (ram.as_ptr() as usize).wrapping_sub(rom.as_ptr() as usize);
        let layout = Self {
            prg_rom: rom.len(),
            prg_ram_start,
            prg_ram: ram.len(),
        };
        for page in 0..64u16 {
            let base = page << 10;
            for i in 0..PAGE as u16 {
                let addr = base | i;
                if let Some(offset) = layout.rom_offset(bus, addr) {
                    let (got, want) = (bus.memory.prg_peek(addr), rom[offset]);
                    if got != want {
                        return Err(LayoutError::Layout {
                            addr,
                            offset,
                            got,
                            want,
                        });
                    }
                }
            }
        }
        Ok(layout)
    }

    /// A layout of `prg_rom` bytes and no PRG RAM, for tests that never
    /// read a console.
    #[doc(hidden)]
    pub fn for_tests(prg_rom: usize) -> Self {
        Self {
            prg_rom,
            prg_ram_start: 0,
            prg_ram: 0,
        }
    }

    /// PRG ROM's and PRG RAM's place in the memory arena, as
    /// `tetanes_core::shim::Shim::new` takes them: start and length each.
    pub fn arena(&self) -> (usize, usize, usize, usize) {
        (
            PRG_ROM_START,
            self.prg_rom,
            self.prg_ram_start,
            self.prg_ram,
        )
    }

    /// The PRG ROM offset CPU `addr` reads now, if it reads PRG ROM.
    #[inline]
    pub fn rom_offset(&self, bus: &Bus, addr: u16) -> Option<usize> {
        if addr < 0x4020 {
            return None;
        }
        let offset = bus.memory.prg_offset(addr)?.checked_sub(PRG_ROM_START)?;
        (offset < self.prg_rom).then_some(offset)
    }

    /// The PRG RAM offset CPU `addr` reaches now, if it reaches PRG RAM.
    #[inline]
    pub fn ram_offset(&self, bus: &Bus, addr: u16) -> Option<usize> {
        if addr < 0x4020 || self.prg_ram == 0 {
            return None;
        }
        let offset = bus
            .memory
            .prg_offset(addr)?
            .checked_sub(self.prg_ram_start)?;
        (offset < self.prg_ram).then_some(offset)
    }
}
