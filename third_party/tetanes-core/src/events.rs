//! A log of console events, kept only when someone asks for one.
//!
//! Game Printer records these alongside each frame: interrupts taken, sprite 0 hits, bank
//! switches, controller reads and writes to the PPU, APU and controller registers. The log lives
//! on the [`Ppu`](crate::ppu::Ppu) because the sprite 0 hit is found inside it and its scanline and
//! dot are every event's timestamp; events found on the bus push to the same log, so it stays in
//! the order things happened.
//!
//! Off, which is the default, every hook is one test of an `Option` that is `None`, and nothing
//! about emulation changes either way: the hooks only read state.

/// What happened.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u8)]
pub enum EventKind {
    /// The CPU took the NMI vector. `addr` is the handler's address.
    Nmi = 0,
    /// The CPU took the IRQ vector (not a `BRK`). `addr` is the handler's address.
    Irq = 1,
    /// The PPU set the sprite 0 hit flag.
    Sprite0Hit = 2,
    /// A CPU write to the cartridge changed the PRG or CHR page tables (a bank switch or a
    /// mirroring change). `addr` and `value` are the write's.
    BankSwitch = 3,
    /// The CPU read $4016 or $4017. `value` is what it read.
    ControllerRead = 4,
    /// The CPU wrote $4016, the controller strobe.
    ControllerWrite = 5,
    /// The CPU wrote a PPU register: $2000-$2007 (mirrors folded down) or $4014, OAM DMA.
    PpuWrite = 6,
    /// The CPU wrote an APU register: $4000-$4013, $4015 or $4017.
    ApuWrite = 7,
}

/// One event, at the PPU dot it happened on.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Event {
    /// What happened.
    pub kind: EventKind,
    /// The PPU's scanline when it happened.
    pub scanline: u16,
    /// The PPU's dot within the scanline.
    pub dot: u16,
    /// The register written or read, or the handler's address for an interrupt; 0 for a sprite 0
    /// hit.
    pub addr: u16,
    /// The value written or read; 0 for an interrupt or a sprite 0 hit.
    pub value: u8,
}

/// Appends an event to `log` if a log is attached. A free function over the field rather than a
/// method, so a caller holding a borrow of another part of the PPU can still record.
#[cold]
#[inline(never)]
pub fn record(
    log: &mut Option<Box<Vec<Event>>>,
    kind: EventKind,
    scanline: u16,
    dot: u16,
    addr: u16,
    value: u8,
) {
    if let Some(log) = log {
        log.push(Event {
            kind,
            scanline,
            dot,
            addr,
            value,
        });
    }
}
