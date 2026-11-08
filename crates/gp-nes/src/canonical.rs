//! The canonical console state: what the per-frame `state` hash covers.
//!
//! A fixed list of named fields, each a little-endian integer of a fixed
//! width or a byte region with a u32 length in front, written in the order of
//! [`FIELDS`] after an 8-byte header (`GPNS`, then [`VERSION`] as a u32). It
//! holds what the console's behaviour depends on and nothing about how
//! `tetanes-core` runs it: no settings (headless flags, sample rate, speed,
//! the concurrent D-pad option), no master-clock bookkeeping, no held
//! buttons. docs/formats/recording.md describes every field; the frame trace
//! will reuse this table.
//!
//! Any change to the fields, their order or their meaning bumps [`VERSION`],
//! which is part of the hashed bytes, so states from different versions never
//! compare equal by accident.

use tetanes_core::apu::envelope::Envelope;
use tetanes_core::apu::length_counter::LengthCounter;
use tetanes_core::apu::noise::ShiftMode;
use tetanes_core::apu::timer::Timer;
use tetanes_core::bus::Bus;
use tetanes_core::cpu::IrqFlags;
use tetanes_core::mapper::{Cnrom, Mapper, Mmc1, Mmc1Revision, Mmc3};
use tetanes_core::memory::Src;
use tetanes_core::ppu::Mirroring;

use crate::NesError;

pub const MAGIC: &[u8; 4] = b"GPNS";
/// Version of the field table. 1 is the first; 2 added the MMC1 and MMC3
/// registers; 3 widened `cpu.cycles` to 64 bits, added the open bus decay
/// and left PRG RAM out on NROM boards without a battery; 4 holds the open
/// bus as `tetanes-core` does (a decayed bit still shows through a palette
/// read until the bus is next driven) and keeps NROM's PRG RAM, which
/// `tetanes-core` maps and a program can use; 5 added CNROM's CHR bank
/// register.
pub const VERSION: u32 = 5;

/// How many bytes a field takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Width {
    /// A fixed number of bytes: an integer, little-endian, or a fixed-size array.
    Fixed(usize),
    /// A u32 byte count, then that many bytes. For memories whose size
    /// depends on the cartridge, and which are empty when it has none.
    Region,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Field {
    pub name: &'static str,
    pub width: Width,
}

const fn f(name: &'static str, n: usize) -> Field {
    Field {
        name,
        width: Width::Fixed(n),
    }
}

const fn region(name: &'static str) -> Field {
    Field {
        name,
        width: Width::Region,
    }
}

macro_rules! envelope_fields {
    ($ch:literal) => {
        [
            f(concat!($ch, ".envelope.start"), 1),
            f(concat!($ch, ".envelope.loop"), 1),
            f(concat!($ch, ".envelope.constant"), 1),
            f(concat!($ch, ".envelope.volume"), 1),
            f(concat!($ch, ".envelope.divider"), 1),
            f(concat!($ch, ".envelope.decay"), 1),
        ]
    };
}

macro_rules! length_fields {
    ($ch:literal) => {
        [
            f(concat!($ch, ".length.enabled"), 1),
            f(concat!($ch, ".length.halt"), 1),
            f(concat!($ch, ".length.counter"), 1),
        ]
    };
}

macro_rules! timer_fields {
    ($ch:literal) => {
        [
            f(concat!($ch, ".timer.period"), 2),
            f(concat!($ch, ".timer.counter"), 2),
        ]
    };
}

macro_rules! pulse_fields {
    ($ch:literal) => {
        concat_fields!(
            timer_fields!($ch),
            [
                f(concat!($ch, ".duty"), 1),
                f(concat!($ch, ".duty_step"), 1)
            ],
            length_fields!($ch),
            envelope_fields!($ch),
            [
                f(concat!($ch, ".sweep.enabled"), 1),
                f(concat!($ch, ".sweep.negate"), 1),
                f(concat!($ch, ".sweep.reload"), 1),
                f(concat!($ch, ".sweep.shift"), 1),
                f(concat!($ch, ".sweep.period"), 1),
                f(concat!($ch, ".sweep.divider"), 1),
            ]
        )
    };
}

/// Joins arrays of fields into one array, at compile time.
macro_rules! concat_fields {
    ($($part:expr),+ $(,)?) => {{
        const LEN: usize = 0 $(+ $part.len())+;
        const OUT: [Field; LEN] = {
            let mut out = [f("", 0); LEN];
            let mut i = 0;
            $(
                let part = $part;
                let mut j = 0;
                while j < part.len() {
                    out[i] = part[j];
                    i += 1;
                    j += 1;
                }
            )+
            let _ = i;
            out
        };
        OUT
    }};
}

const FIELD_ARRAY: [Field; 145] = concat_fields!(
    [
        f("cpu.a", 1),
        f("cpu.x", 1),
        f("cpu.y", 1),
        f("cpu.s", 1),
        f("cpu.p", 1),
        f("cpu.pc", 2),
        f("cpu.cycles", 8),
        f("cpu.interrupts", 1),
        f("cpu.dma", 1),
        f("cpu.oam_dma_page", 1),
        f("ram", 2048),
        f("ppu.scanline", 2),
        f("ppu.dot", 2),
        f("ppu.odd_frame", 1),
        f("ppu.ctrl", 1),
        f("ppu.mask", 1),
        f("ppu.status", 1),
        f("ppu.oamaddr", 1),
        f("ppu.v", 2),
        f("ppu.t", 2),
        f("ppu.x", 1),
        f("ppu.w", 1),
        f("ppu.read_buffer", 1),
        f("ppu.io_latch", 1),
        f("ppu.io_latch_age", 4),
        f("ppu.nmi_pending", 1),
        f("oam", 256),
        f("palette", 32),
        region("nametables"),
        region("chr_ram"),
        region("prg_ram"),
        f("mapper.number", 2),
        f("mapper.mirroring", 1),
    ],
    MMC1_FIELDS,
    MMC3_FIELDS,
    CNROM_FIELDS,
    pulse_fields!("apu.pulse1"),
    pulse_fields!("apu.pulse2"),
    timer_fields!("apu.triangle"),
    [f("apu.triangle.step", 1)],
    length_fields!("apu.triangle"),
    [
        f("apu.triangle.linear.reload", 1),
        f("apu.triangle.linear.control", 1),
        f("apu.triangle.linear.reload_value", 1),
        f("apu.triangle.linear.counter", 1),
    ],
    timer_fields!("apu.noise"),
    [f("apu.noise.mode", 1), f("apu.noise.shift", 2)],
    length_fields!("apu.noise"),
    envelope_fields!("apu.noise"),
    timer_fields!("apu.dmc"),
    [
        f("apu.dmc.irq_enabled", 1),
        f("apu.dmc.irq_pending", 1),
        f("apu.dmc.loop", 1),
        f("apu.dmc.sample_addr", 2),
        f("apu.dmc.sample_length", 2),
        f("apu.dmc.addr", 2),
        f("apu.dmc.bytes_remaining", 2),
        f("apu.dmc.sample_buffer", 1),
        f("apu.dmc.buffer_empty", 1),
        f("apu.dmc.output_level", 1),
        f("apu.dmc.shift", 1),
        f("apu.dmc.bits_remaining", 1),
        f("apu.dmc.silence", 1),
        f("apu.frame_counter.mode", 1),
        f("apu.frame_counter.step", 1),
        f("apu.frame_counter.cycle", 4),
        f("apu.frame_counter.irq_inhibit", 1),
        f("apu.frame_counter.irq_pending", 1),
        f("apu.frame_counter.write_pending", 1),
        f("apu.frame_counter.write_value", 1),
        f("apu.frame_counter.write_delay", 1),
        f("pad1.index", 1),
        f("pad1.strobe", 1),
        f("pad2.index", 1),
        f("pad2.strobe", 1),
    ],
);

/// MMC1's registers (mapper 1), all zero for other mappers. `control`,
/// `chr0`, `chr1` and `prg` are the four registers as last written through
/// the serial port, 5 bits each. `shift` and `shift_count` are the serial
/// port's partly filled value and how many bits it holds. `last_chr` says
/// which CHR register was written last (0 for $A000, 1 for $C000), which
/// picks the PRG bank's high bit on 512KB boards. `write_delay` counts the
/// CPU cycles, 2 down to 0, in which another write is ignored.
const MMC1_FIELDS: [Field; 8] = [
    f("mmc1.shift", 1),
    f("mmc1.shift_count", 1),
    f("mmc1.control", 1),
    f("mmc1.chr0", 1),
    f("mmc1.chr1", 1),
    f("mmc1.prg", 1),
    f("mmc1.last_chr", 1),
    f("mmc1.write_delay", 1),
];

/// MMC3's registers (mapper 4), all zero for other mappers. `bank_select`
/// is $8000 as written and `r0` to `r7` the eight bank registers. The
/// IRQ fields are the reload latch ($C000), the counter, the enable
/// ($E001/$E000), the reload request ($C001) and the pending IRQ. `a12_low`
/// is the A12 edge filter: 0 when A12 hasn't been seen low since it was
/// last high, otherwise 1 plus the CPU cycles it has been low, capped at 4
/// (the filter counts a rise after 4 or more). $A001 (PRG RAM protect)
/// isn't emulated, so it has no field.
const MMC3_FIELDS: [Field; 15] = [
    f("mmc3.bank_select", 1),
    f("mmc3.r0", 1),
    f("mmc3.r1", 1),
    f("mmc3.r2", 1),
    f("mmc3.r3", 1),
    f("mmc3.r4", 1),
    f("mmc3.r5", 1),
    f("mmc3.r6", 1),
    f("mmc3.r7", 1),
    f("mmc3.irq_latch", 1),
    f("mmc3.irq_counter", 1),
    f("mmc3.irq_enabled", 1),
    f("mmc3.irq_reload", 1),
    f("mmc3.irq_pending", 1),
    f("mmc3.a12_low", 1),
];

/// CNROM's register (mapper 3), zero for other mappers: the CHR bank as
/// last written to $8000-$FFFF, all 8 bits, as `tetanes-core` keeps it (a
/// bank past the CHR ROM's end wraps when the PPU reads it). The board
/// fixes PRG, so it has nothing else. `tetanes-core` doesn't emulate bus
/// conflicts, so the value is the one written, not the one ANDed with the
/// ROM byte under it.
const CNROM_FIELDS: [Field; 1] = [f("cnrom.chr_bank", 1)];

/// The fields of version [`VERSION`], in the order they're written.
pub const FIELDS: &[Field] = &FIELD_ARRAY;

/// [`FIELDS`] as a self-describing table, which the frame trace stores so a
/// reader can split states without this crate.
pub fn field_table() -> gp_evidence::trace::FieldTable {
    use gp_evidence::trace;
    trace::FieldTable {
        state_magic: *MAGIC,
        state_version: VERSION,
        fields: FIELDS
            .iter()
            .map(|f| trace::FieldDef {
                name: f.name.into(),
                width: match f.width {
                    Width::Fixed(n) => trace::Width::Fixed(n as u32),
                    Width::Region => trace::Width::Region,
                },
            })
            .collect(),
    }
}

/// Writes fields in order, checking each against [`FIELDS`] in debug builds.
struct Writer {
    out: Vec<u8>,
    next: usize,
}

impl Writer {
    fn put(&mut self, name: &str, bytes: &[u8]) {
        debug_assert_eq!(FIELDS[self.next].name, name, "field {}", self.next);
        debug_assert_eq!(FIELDS[self.next].width, Width::Fixed(bytes.len()), "{name}");
        self.next += 1;
        self.out.extend_from_slice(bytes);
    }

    fn u8(&mut self, name: &str, v: u8) {
        self.put(name, &[v]);
    }

    fn bool(&mut self, name: &str, v: bool) {
        self.put(name, &[u8::from(v)]);
    }

    fn u16(&mut self, name: &str, v: u16) {
        self.put(name, &v.to_le_bytes());
    }

    fn u32(&mut self, name: &str, v: u32) {
        self.put(name, &v.to_le_bytes());
    }

    fn region(&mut self, name: &str, bytes: &[u8]) {
        debug_assert_eq!(FIELDS[self.next].name, name, "field {}", self.next);
        debug_assert_eq!(FIELDS[self.next].width, Width::Region, "{name}");
        self.next += 1;
        let len = u32::try_from(bytes.len()).expect("memory regions are far below 4GB");
        self.out.extend_from_slice(&len.to_le_bytes());
        self.out.extend_from_slice(bytes);
    }

    fn timer(&mut self, ch: &str, t: &Timer) {
        self.u16(&format!("{ch}.timer.period"), t.period);
        self.u16(&format!("{ch}.timer.counter"), t.counter);
    }

    fn length(&mut self, ch: &str, l: &LengthCounter) {
        self.bool(&format!("{ch}.length.enabled"), l.enabled);
        self.bool(&format!("{ch}.length.halt"), l.halt);
        self.u8(&format!("{ch}.length.counter"), l.counter);
    }

    fn mmc1(&mut self, mmc1: Option<&Mmc1>) {
        let Some(m) = mmc1 else {
            for f in &MMC1_FIELDS {
                self.put(f.name, &[0]);
            }
            return;
        };
        let mirroring = match m.mirroring {
            Mirroring::SingleScreenA => 0,
            Mirroring::SingleScreenB => 1,
            Mirroring::Vertical => 2,
            // MMC1 can't select four-screen; tetanes-core never sets it.
            Mirroring::Horizontal | Mirroring::FourScreen => 3,
        };
        self.u8("mmc1.shift", m.write_buffer);
        self.u8("mmc1.shift_count", m.shift_count);
        self.u8(
            "mmc1.control",
            mirroring
                | u8::from(m.prg_bank_select) << 2
                | u8::from(m.prg_mode) << 3
                | u8::from(m.chr_mode) << 4,
        );
        self.u8("mmc1.chr0", m.chr0);
        self.u8("mmc1.chr1", m.chr1);
        self.u8("mmc1.prg", m.prg | u8::from(m.prg_ram_disabled) << 4);
        self.bool("mmc1.last_chr", m.last_chr_reg == 0xC000);
        self.u8("mmc1.write_delay", m.write_just_occurred);
    }

    fn mmc3(&mut self, mmc3: Option<&Mmc3>) {
        let Some(m) = mmc3 else {
            for f in &MMC3_FIELDS {
                self.put(f.name, &[0]);
            }
            return;
        };
        self.u8("mmc3.bank_select", m.bank_select);
        for (i, v) in m.bank_values.iter().enumerate() {
            self.u8(&format!("mmc3.r{i}"), *v);
        }
        self.u8("mmc3.irq_latch", m.irq_latch);
        self.u8("mmc3.irq_counter", m.irq_counter);
        self.bool("mmc3.irq_enabled", m.irq_enabled);
        self.bool("mmc3.irq_reload", m.irq_reload);
        self.bool("mmc3.irq_pending", m.irq_pending);
        // tetanes-core keeps the CPU cycle A12 went low at (0 for not low)
        // against a free-running count; only how long ago matters, and only
        // up to the filter's 4 cycles.
        let low = if m.a12_low_clock == 0 {
            0
        } else {
            1 + m.master_clock.wrapping_sub(m.a12_low_clock).min(4) as u8
        };
        self.u8("mmc3.a12_low", low);
    }

    fn cnrom(&mut self, cnrom: Option<&Cnrom>) {
        self.u8("cnrom.chr_bank", cnrom.map_or(0, |c| c.chr_bank));
    }

    fn envelope(&mut self, ch: &str, e: &Envelope) {
        self.bool(&format!("{ch}.envelope.start"), e.start);
        self.bool(&format!("{ch}.envelope.loop"), e.loops);
        self.bool(&format!("{ch}.envelope.constant"), e.constant_volume);
        self.u8(&format!("{ch}.envelope.volume"), e.volume);
        self.u8(&format!("{ch}.envelope.divider"), e.divider);
        self.u8(&format!("{ch}.envelope.decay"), e.counter);
    }
}

/// The mappers whose registers the table defines, as messages name them.
pub const MAPPERS: &str =
    "NROM (mapper 0), MMC1 (mapper 1, MMC1B and later), CNROM (mapper 3) and MMC3 (mapper 4)";

/// The canonical state of the console on `bus`, header included.
///
/// Fails for a mapper whose registers the table doesn't define yet (see
/// [`MAPPERS`]). Adding one is a new [`VERSION`].
pub fn encode(bus: &Bus) -> Result<Vec<u8>, NesError> {
    let unsupported = || {
        Err(NesError::Unsupported(format!(
            "the canonical state defines mapper registers for {MAPPERS} only"
        )))
    };
    match board(bus) {
        Some((number, mirroring)) => Ok(write(bus, number, mirroring)),
        None => unsupported(),
    }
}

/// The board's number and mirroring, where the table defines its registers.
fn board(bus: &Bus) -> Option<(u16, u8)> {
    Some(match &bus.mapper {
        Mapper::Nrom(nrom) => (0u16, nrom.mirroring as u8),
        Mapper::Sxrom(sxrom) if sxrom.mmc1.revision == Mmc1Revision::BC => {
            (1, sxrom.mmc1.mirroring as u8)
        }
        Mapper::Cnrom(cnrom) => (3, cnrom.mirroring as u8),
        Mapper::Txrom(txrom) if txrom.mapper_num == 4 => (4, txrom.mirroring as u8),
        _ => return None,
    })
}

/// [`encode`] on any board: on one the table doesn't define, the same
/// fields with `mapper.number` 0xFFFF and the board's own registers left
/// out (zero). Not a recording's state, and never stored: two runs of the
/// same runtime compare by its hash (`Nes::state_or_board_free_hash`) on a
/// board the table can't name yet.
pub fn encode_any(bus: &Bus) -> Vec<u8> {
    let (number, mirroring) = board(bus).unwrap_or((u16::MAX, 0));
    write(bus, number, mirroring)
}

fn write(bus: &Bus, number: u16, mirroring: u8) -> Vec<u8> {
    let mut w = Writer {
        out: Vec::with_capacity(8 + 4096 + 2048 + 512),
        next: 0,
    };
    w.out.extend_from_slice(MAGIC);
    w.out.extend_from_slice(&VERSION.to_le_bytes());

    let cpu = &bus.cpu;
    w.u8("cpu.a", cpu.acc);
    w.u8("cpu.x", cpu.x);
    w.u8("cpu.y", cpu.y);
    w.u8("cpu.s", cpu.sp);
    w.u8("cpu.p", cpu.status.bits());
    w.u16("cpu.pc", cpu.pc);
    w.put("cpu.cycles", &cpu.cycle.to_le_bytes());
    let interrupt_bits = IrqFlags::NMI
        | IrqFlags::PREV_NMI
        | IrqFlags::PREV_NMI_PENDING
        | IrqFlags::RUN_IRQ
        | IrqFlags::PREV_RUN_IRQ;
    w.u8("cpu.interrupts", (cpu.irq_flags & interrupt_bits).bits());
    let dma = u8::from(cpu.irq_flags.contains(IrqFlags::DMA_DMC))
        | u8::from(cpu.irq_flags.contains(IrqFlags::DMA_HALT)) << 1
        | u8::from(cpu.irq_flags.contains(IrqFlags::DMA_DUMMY_READ)) << 2
        | u8::from(cpu.dma_oam_addr.is_some()) << 3;
    w.u8("cpu.dma", dma);
    w.u8(
        "cpu.oam_dma_page",
        cpu.dma_oam_addr.map_or(0, |a| (a >> 8) as u8),
    );
    w.put("ram", &bus.wram[0..2048]);

    let ppu = &bus.ppu;
    w.u16("ppu.scanline", ppu.scanline);
    w.u16("ppu.dot", ppu.cycle);
    w.bool("ppu.odd_frame", ppu.frame.is_odd());
    // PPUCTRL as written, except bits 0-1 (the nametable select), which
    // live in t.
    let ctrl = u8::from(ppu.ctrl_vram_increment) << 2
        | u8::from(ppu.ctrl_spr_select != 0) << 3
        | u8::from(ppu.ctrl_bg_select != 0) << 4
        | u8::from(ppu.ctrl_spr_height == 16) << 5
        | (ppu.ctrl_master_slave & 1) << 6
        | u8::from(ppu.ctrl_nmi_enabled) << 7;
    w.u8("ppu.ctrl", ctrl);
    w.u8("ppu.mask", ppu.mask_bits.bits());
    w.u8("ppu.status", ppu.read_status_bits());
    w.u8("ppu.oamaddr", ppu.oamaddr);
    w.u16("ppu.v", ppu.scroll.v);
    w.u16("ppu.t", ppu.scroll.t);
    w.u8("ppu.x", ppu.scroll.fine_x as u8);
    w.bool("ppu.w", ppu.scroll.write_latch);
    w.u8("ppu.read_buffer", ppu.vram_buffer);
    // The open bus as tetanes-core holds it. A bit older than the decay
    // limit reads as 0 through every register but one: a palette read
    // returns the held bits 6 and 7 until the bus is next driven, as
    // Mesen's does. So the held value and each set bit's age (0 to 4, where
    // 4 is past the limit) both matter; a 0 bit's age doesn't, so it's 0.
    // Three bits per latch bit, bit 0's lowest.
    let latch = ppu.open_bus;
    let mut ages = 0u32;
    for bit in 0..8 {
        if latch & (1 << bit) != 0 {
            let age = ppu.open_bus_age[bit].min(tetanes_core::ppu::OPEN_BUS_DECAY_FRAMES + 1);
            ages |= u32::from(age) << (3 * bit);
        }
    }
    w.u8("ppu.io_latch", latch);
    w.u32("ppu.io_latch_age", ages);
    w.bool("ppu.nmi_pending", ppu.nmi_pending);
    w.put("oam", &ppu.oamdata[0..256]);
    let mut palette = [0u8; 32];
    for (i, p) in palette.iter_mut().enumerate() {
        *p = bus.ppu_bus_peek(0x3F00 + i as u16);
    }
    w.put("palette", &palette);
    w.region("nametables", bus.memory.region_ref(Src::CiRam));
    let chr_is_ram = bus.memory.chr_pages()[0].is_writable();
    w.region(
        "chr_ram",
        if chr_is_ram {
            bus.memory.region_ref(Src::Chr)
        } else {
            &[]
        },
    );
    // tetanes-core maps 8KB of PRG RAM on every NROM board, which most of
    // the boards don't have; a program can still use it (test ROMs report
    // there), so it's part of the state.
    w.region("prg_ram", bus.memory.region_ref(Src::PrgRam));
    w.u16("mapper.number", number);
    w.u8("mapper.mirroring", mirroring);
    w.mmc1(match &bus.mapper {
        Mapper::Sxrom(sxrom) => Some(&sxrom.mmc1),
        _ => None,
    });
    w.mmc3(match &bus.mapper {
        Mapper::Txrom(txrom) => Some(&txrom.mmc3),
        _ => None,
    });
    w.cnrom(match &bus.mapper {
        Mapper::Cnrom(cnrom) => Some(cnrom),
        _ => None,
    });

    let apu = &bus.apu;
    for (ch, p) in [("apu.pulse1", &apu.pulse1), ("apu.pulse2", &apu.pulse2)] {
        w.timer(ch, &p.timer);
        w.u8(&format!("{ch}.duty"), p.duty);
        w.u8(&format!("{ch}.duty_step"), p.duty_cycle);
        w.length(ch, &p.length);
        w.envelope(ch, &p.envelope);
        w.bool(&format!("{ch}.sweep.enabled"), p.sweep.enabled);
        w.bool(&format!("{ch}.sweep.negate"), p.sweep.negate);
        w.bool(&format!("{ch}.sweep.reload"), p.sweep.reload);
        w.u8(&format!("{ch}.sweep.shift"), p.sweep.shift);
        w.u8(&format!("{ch}.sweep.period"), p.sweep.period);
        w.u8(&format!("{ch}.sweep.divider"), p.sweep.divider);
    }
    let tri = &apu.triangle;
    w.timer("apu.triangle", &tri.timer);
    w.u8("apu.triangle.step", tri.sequence);
    w.length("apu.triangle", &tri.length);
    w.bool("apu.triangle.linear.reload", tri.linear.reload);
    w.bool("apu.triangle.linear.control", tri.linear.control);
    w.u8(
        "apu.triangle.linear.reload_value",
        tri.linear.counter_reload,
    );
    w.u8("apu.triangle.linear.counter", tri.linear.counter);
    let noise = &apu.noise;
    w.timer("apu.noise", &noise.timer);
    w.bool("apu.noise.mode", matches!(noise.shift_mode, ShiftMode::One));
    w.u16("apu.noise.shift", noise.shift);
    w.length("apu.noise", &noise.length);
    w.envelope("apu.noise", &noise.envelope);
    let dmc = &apu.dmc;
    w.timer("apu.dmc", &dmc.timer);
    w.bool("apu.dmc.irq_enabled", dmc.irq_enabled);
    w.bool("apu.dmc.irq_pending", dmc.irq_pending);
    w.bool("apu.dmc.loop", dmc.loops);
    w.u16("apu.dmc.sample_addr", dmc.sample_addr);
    w.u16("apu.dmc.sample_length", dmc.sample_length);
    w.u16("apu.dmc.addr", dmc.addr);
    w.u16("apu.dmc.bytes_remaining", dmc.bytes_remaining);
    w.u8("apu.dmc.sample_buffer", dmc.sample_buffer);
    w.bool("apu.dmc.buffer_empty", dmc.buffer_empty);
    w.u8("apu.dmc.output_level", dmc.output_level);
    w.u8("apu.dmc.shift", dmc.shift);
    w.u8("apu.dmc.bits_remaining", dmc.bits_remaining);
    w.bool("apu.dmc.silence", dmc.silence);
    let fc = &apu.frame_counter;
    w.u8("apu.frame_counter.mode", fc.mode);
    w.u8("apu.frame_counter.step", fc.step as u8);
    w.u32("apu.frame_counter.cycle", fc.cycle);
    w.bool("apu.frame_counter.irq_inhibit", fc.inhibit_irq);
    w.bool("apu.frame_counter.irq_pending", fc.irq_pending);
    w.bool("apu.frame_counter.write_pending", fc.write_buffer.is_some());
    w.u8(
        "apu.frame_counter.write_value",
        fc.write_buffer.unwrap_or(0),
    );
    w.u8("apu.frame_counter.write_delay", fc.write_delay);

    for (n, pad) in [
        ("pad1", &bus.input.joypads[0]),
        ("pad2", &bus.input.joypads[1]),
    ] {
        w.u8(&format!("{n}.index"), pad.index);
        w.bool(&format!("{n}.strobe"), pad.strobe);
    }
    debug_assert_eq!(w.next, FIELDS.len());
    w.out
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DecodeError {
    #[error("not a canonical NES state (missing GPNS header)")]
    Magic,
    #[error("canonical state version {0} is not supported")]
    Version(u32),
    #[error("canonical state is truncated in field {0}")]
    Truncated(&'static str),
    #[error("canonical state has {0} bytes after its last field")]
    Trailing(usize),
}

/// Splits a canonical state into its fields, in [`FIELDS`] order.
pub fn decode(bytes: &[u8]) -> Result<Vec<(&'static str, &[u8])>, DecodeError> {
    if bytes.len() < 8 || &bytes[..4] != MAGIC {
        return Err(DecodeError::Magic);
    }
    let version = u32::from_le_bytes(bytes[4..8].try_into().expect("4 bytes"));
    if version != VERSION {
        return Err(DecodeError::Version(version));
    }
    let mut rest = &bytes[8..];
    let mut out = Vec::with_capacity(FIELDS.len());
    for field in FIELDS {
        let n = match field.width {
            Width::Fixed(n) => n,
            Width::Region => {
                let len = rest.get(..4).ok_or(DecodeError::Truncated(field.name))?;
                rest = &rest[4..];
                u32::from_le_bytes(len.try_into().expect("4 bytes")) as usize
            }
        };
        let value = rest.get(..n).ok_or(DecodeError::Truncated(field.name))?;
        out.push((field.name, value));
        rest = &rest[n..];
    }
    if !rest.is_empty() {
        return Err(DecodeError::Trailing(rest.len()));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Nes;
    use gp_evidence::input_log::InputFrame;
    use gp_evidence::manifest::PowerOn;

    /// A 32KB-PRG iNES image with CHR RAM: `code` at `origin` in the last
    /// 8KB, which every one of these mappers has at $E000 at power-on, and
    /// all three vectors pointing at it.
    fn image(mapper: u8, origin: u16, code: &[u8]) -> Vec<u8> {
        let mut prg = vec![0xEAu8; 0x8000];
        let at = usize::from(origin - 0x8000);
        prg[at..at + code.len()].copy_from_slice(code);
        for v in [0x7FFA, 0x7FFC, 0x7FFE] {
            prg[v..v + 2].copy_from_slice(&origin.to_le_bytes());
        }
        let mut rom = vec![b'N', b'E', b'S', 0x1A, 2, 0, mapper << 4, mapper & 0xF0];
        rom.resize(16, 0);
        rom.extend(prg);
        rom
    }

    /// Five serial writes of A to `addr` (lo, hi), low bit first.
    fn serial(lo: u8, hi: u8) -> Vec<u8> {
        let sta = [0x8D, lo, hi];
        let mut out = sta.to_vec();
        for _ in 0..4 {
            out.push(0x4A); // LSR A
            out.extend(sta);
        }
        out
    }

    /// The state after one frame of `code`, which ends in a loop on itself.
    fn run(mapper: u8, origin: u16, code: &[u8]) -> Vec<(&'static str, u8, Vec<u8>)> {
        let mut code = code.to_vec();
        let here = origin + code.len() as u16;
        code.extend([0x4C, here as u8, (here >> 8) as u8]); // JMP here
        let mut nes = Nes::power_on(&image(mapper, origin, &code), PowerOn::default()).unwrap();
        nes.step(InputFrame::default()).unwrap();
        let state = nes.canonical_state().unwrap();
        decode(&state)
            .unwrap()
            .into_iter()
            .map(|(n, v)| (n, v[0], v.to_vec()))
            .collect()
    }

    fn get<'a>(fields: &'a [(&str, u8, Vec<u8>)], name: &str) -> &'a [u8] {
        &fields.iter().find(|f| f.0 == name).unwrap().2
    }

    #[test]
    fn holds_mmc1_registers() {
        let mut code = vec![0x78, 0xA9, 0x80, 0x8D, 0x00, 0x80]; // SEI; reset the serial port
        code.extend([0xA9, 0x1E]); // control: CHR 4KB, PRG mode 3, vertical
        code.extend(serial(0x00, 0x80));
        code.extend([0xA9, 0x05]); // CHR bank 0 = 5
        code.extend(serial(0x00, 0xA0));
        code.extend([0xA9, 0x13]); // PRG bank 3, PRG RAM disabled
        code.extend(serial(0x00, 0xE0));
        // Two bits of a third write: 1, then 1.
        code.extend([0xA9, 0x01, 0x8D, 0x00, 0xC0, 0xEA, 0x8D, 0x00, 0xC0]);
        let f = run(1, 0xE000, &code);
        assert_eq!(get(&f, "mapper.number"), [1, 0]);
        assert_eq!(get(&f, "mapper.mirroring"), [Mirroring::Vertical as u8]);
        assert_eq!(get(&f, "mmc1.control"), [0x1E]);
        assert_eq!(get(&f, "mmc1.chr0"), [5]);
        assert_eq!(get(&f, "mmc1.prg"), [0x13]);
        assert_eq!(get(&f, "mmc1.last_chr"), [0]);
        assert_eq!(get(&f, "mmc1.shift_count"), [2]);
        assert_eq!(get(&f, "mmc1.shift"), [0x18]);
        assert_eq!(get(&f, "mmc1.write_delay"), [0]);
        assert!(
            f.iter()
                .filter(|x| x.0.starts_with("mmc3."))
                .all(|x| x.2.iter().all(|b| *b == 0))
        );
    }

    #[test]
    fn holds_mmc3_registers() {
        let code = [
            0x78, // SEI
            0xA9, 0x46, 0x8D, 0x00, 0x80, // $8000 = $46: PRG mode 1, R6 next
            0xA9, 0x01, 0x8D, 0x01, 0x80, // R6 = 1
            0xA9, 0x01, 0x8D, 0x00, 0xA0, // horizontal mirroring
            0xA9, 0x07, 0x8D, 0x00, 0xC0, // IRQ latch 7
            0x8D, 0x01, 0xC0, // reload
            0x8D, 0x01, 0xE0, // IRQs on
        ];
        let f = run(4, 0xE000, &code);
        assert_eq!(get(&f, "mapper.number"), [4, 0]);
        assert_eq!(get(&f, "mapper.mirroring"), [Mirroring::Horizontal as u8]);
        assert_eq!(get(&f, "mmc3.bank_select"), [0x46]);
        assert_eq!(get(&f, "mmc3.r6"), [1]);
        assert_eq!(get(&f, "mmc3.irq_latch"), [7]);
        assert_eq!(get(&f, "mmc3.irq_enabled"), [1]);
        // Rendering is off, so A12 never rises and the reload waits.
        assert_eq!(get(&f, "mmc3.irq_reload"), [1]);
        assert_eq!(get(&f, "mmc3.irq_pending"), [0]);
        assert!(get(&f, "mmc3.a12_low")[0] <= 5);
        assert!(
            f.iter()
                .filter(|x| x.0.starts_with("mmc1."))
                .all(|x| x.1 == 0)
        );
    }

    #[test]
    fn holds_the_cnrom_chr_bank() {
        // An image with 32KB of CHR ROM, as CNROM boards have: four 8KB
        // banks, each tile's first byte its bank number.
        let code = [
            0xA9, 0x02, 0x8D, 0x00, 0x80, // CHR bank 2
            0xA9, 0x01, 0x8D, 0xFF, 0xFF, // then 1, written at the far end
        ];
        let mut rom = image(3, 0xE000, &{
            let mut c = code.to_vec();
            c.extend([0x4C, 0x0A, 0xE0]); // JMP here
            c
        });
        rom[5] = 4; // 4 x 8KB of CHR ROM
        for bank in 0..4u8 {
            rom.extend(std::iter::repeat_n(bank, 0x2000));
        }
        let mut nes = Nes::power_on(&rom, PowerOn::default()).unwrap();
        let before = nes.canonical_state().unwrap();
        let f = decode(&before).unwrap();
        let get = |f: &[(&str, &[u8])], n: &str| f.iter().find(|x| x.0 == n).unwrap().1.to_vec();
        assert_eq!(get(&f, "mapper.number"), [3, 0]);
        assert_eq!(get(&f, "cnrom.chr_bank"), [0], "power-on is bank 0");
        nes.step(InputFrame::default()).unwrap();
        let after = nes.canonical_state().unwrap();
        let f = decode(&after).unwrap();
        assert_eq!(get(&f, "cnrom.chr_bank"), [1]);
        assert_eq!(get(&f, "chr_ram"), Vec::<u8>::new(), "CHR ROM isn't state");
        assert!(
            f.iter()
                .filter(|x| x.0.starts_with("mmc1.") || x.0.starts_with("mmc3."))
                .all(|x| x.1.iter().all(|b| *b == 0))
        );
        // A save state round-trips the register: the bank and the hash.
        let snap = nes.snapshot().unwrap();
        let mut again = Nes::power_on(&rom, PowerOn::default()).unwrap();
        again.restore(&snap, 1, nes.held()).unwrap();
        assert_eq!(again.canonical_state().unwrap(), after);
        // So does the PPU's view: bank 1's byte at $0000.
        assert_eq!(again.bus().ppu_bus_peek(0x0000), 1);
    }

    #[test]
    fn the_open_bus_decays_after_three_frames() {
        // LDA #$C7; STA $2003 (drives every bit of the open bus); loop.
        let code = [0xA9, 0xC7, 0x8D, 0x03, 0x20, 0x4C, 0x05, 0xE0];
        let mut nes = Nes::power_on(&image(0, 0xE000, &code), PowerOn::default()).unwrap();
        let mut seen = Vec::new();
        for _ in 0..5 {
            nes.step(InputFrame::default()).unwrap();
            let state = nes.canonical_state().unwrap();
            let f = decode(&state).unwrap();
            let get = |n: &str| f.iter().find(|x| x.0 == n).unwrap().1.to_vec();
            seen.push((get("ppu.io_latch")[0], get("ppu.io_latch_age")));
        }
        // Driven in frame 1; each frame ends after scanline 240 ages it,
        // up to 4, past the limit. Nothing drives the bus again, so the
        // held value stays.
        let age = |a: u32| {
            let mut v = 0u32;
            for bit in 0..8 {
                if 0xC7 & (1 << bit) != 0 {
                    v |= a << (3 * bit);
                }
            }
            v.to_le_bytes().to_vec()
        };
        for (i, want) in [1, 2, 3, 4, 4].into_iter().enumerate() {
            assert_eq!(seen[i], (0xC7, age(want)), "frame {}", i + 1);
        }
        // Past the limit, a palette read still returns the held bits 6 and
        // 7, as Mesen's does, and the bus decays as it's driven; then
        // $2002's low bits read 0. A console whose bus held 0 would read 0
        // there, so the held bits had to be in the state.
        let mut reads = None;
        nes.step_with(InputFrame::default(), |bus| {
            if reads.is_none() {
                bus.ppu.scroll.v = 0x3F00;
                let palette = bus.read_data() & 0xC0;
                let held = bus.ppu.open_bus & 0xC0;
                let status = bus.ppu.read_status() & 0x1F;
                reads = Some((status, palette, held));
            }
        })
        .unwrap();
        assert_eq!(reads, Some((0x00, 0xC0, 0x00)));
        // NROM boards keep tetanes-core's 8KB of PRG RAM in the state.
        let state = nes.canonical_state().unwrap();
        let f = decode(&state).unwrap();
        assert_eq!(f.iter().find(|x| x.0 == "prg_ram").unwrap().1.len(), 8192);
    }

    #[test]
    fn refuses_a_mapper_it_doesnt_define() {
        // UxROM (mapper 2).
        let rom = image(2, 0xE000, &[0x4C, 0x00, 0xE0]);
        let nes = Nes::power_on(&rom, PowerOn::default()).unwrap();
        let err = nes.canonical_state().unwrap_err();
        assert!(err.to_string().contains(MAPPERS), "{err}");
    }

    #[test]
    fn the_a12_filter_is_held_without_the_cycle_count() {
        // Two consoles whose A12 went low at different absolute cycles
        // encode the same once both have been low for 4 cycles or more.
        let mut a = Mmc3::default();
        let mut b = Mmc3::default();
        a.master_clock = 1000;
        a.a12_low_clock = 990;
        b.master_clock = 5;
        b.a12_low_clock = 1;
        let enc = |m: &Mmc3| {
            let mut w = Writer {
                out: Vec::new(),
                next: FIELDS
                    .iter()
                    .position(|f| f.name == "mmc3.bank_select")
                    .unwrap(),
            };
            w.mmc3(Some(m));
            w.out
        };
        assert_eq!(enc(&a), enc(&b));
        b.a12_low_clock = 3;
        assert_ne!(enc(&a), enc(&b));
    }
}
