//! Writing canonical-state fields into a running console.
//!
//! The oracle diff (`gp oracle diff`, docs/oracle.md) resyncs after a
//! divergence by copying the other runtime's values into `gp-nes` and
//! running on, so it can find the next divergence instead of a cascade of
//! the first. [`write_field`] takes a field by its canonical name
//! ([`crate::canonical`]) with the bytes a frame trace stores for it, and
//! writes it through [`crate::Nes::step_with`]'s bus hook.
//!
//! Every canonical field can be written except palette RAM (its storage is
//! private to `tetanes-core`) and the mapper fields: `mapper.*` is fixed by
//! the cartridge, and writing `mmc1.*`, `mmc3.*` or `cnrom.*` would need the board to
//! remap its banks. The PPU position and the CPU
//! cycle count are written as numbers: the PPU carries on from the written
//! dot as the CPU clocks it, which is what copying another console's
//! position means. Nothing here changes how `gp-nes` powers on, steps or
//! hashes; it only runs when a caller asks for it.

use tetanes_core::apu::envelope::Envelope;
use tetanes_core::apu::length_counter::LengthCounter;
use tetanes_core::apu::noise::ShiftMode;
use tetanes_core::apu::timer::Timer;
use tetanes_core::bus::Bus;
use tetanes_core::cpu::{IrqFlags, Status};
use tetanes_core::memory::Src;

/// The canonical fields [`write_field`] can't write.
pub const NOT_COPYABLE: &[&str] = &["palette", "mapper.number", "mapper.mirroring"];

/// Prefixes of further fields [`write_field`] can't write: the mapper chips'
/// registers, since writing them would need the board to remap its banks.
pub const NOT_COPYABLE_PREFIXES: &[&str] = &["mmc1.", "mmc3.", "cnrom."];

/// Whether [`write_field`] accepts the field.
pub fn copyable(name: &str) -> bool {
    !NOT_COPYABLE.contains(&name)
        && !NOT_COPYABLE_PREFIXES.iter().any(|p| name.starts_with(p))
        && crate::canonical::FIELDS.iter().any(|f| f.name == name)
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PokeError {
    #[error("field {0} can't be written into a running gp-nes console")]
    NotCopyable(String),
    #[error("field {name} has {got} bytes; this console holds {want}")]
    Size {
        name: String,
        got: usize,
        want: usize,
    },
}

fn exact<const N: usize>(name: &str, value: &[u8]) -> Result<[u8; N], PokeError> {
    value.try_into().map_err(|_| PokeError::Size {
        name: name.into(),
        got: value.len(),
        want: N,
    })
}

fn region(bus: &mut Bus, src: Src, name: &str, value: &[u8]) -> Result<(), PokeError> {
    let want = bus.memory.region_ref(src).len();
    if value.len() != want {
        return Err(PokeError::Size {
            name: name.into(),
            got: value.len(),
            want,
        });
    }
    for (i, &b) in value.iter().enumerate() {
        bus.memory.region_write(src, i, b);
    }
    Ok(())
}

fn timer(t: &mut Timer, part: &str, v: u16) -> bool {
    match part {
        "timer.period" => t.period = v,
        "timer.counter" => t.counter = v,
        _ => return false,
    }
    true
}

fn length(l: &mut LengthCounter, part: &str, v: u8) -> bool {
    match part {
        "length.enabled" => l.enabled = v != 0,
        "length.halt" => l.halt = v != 0,
        "length.counter" => l.counter = v,
        _ => return false,
    }
    true
}

fn envelope(e: &mut Envelope, part: &str, v: u8) -> bool {
    match part {
        "envelope.start" => e.start = v != 0,
        "envelope.loop" => e.loops = v != 0,
        "envelope.constant" => e.constant_volume = v != 0,
        "envelope.volume" => e.volume = v,
        "envelope.divider" => e.divider = v,
        "envelope.decay" => e.counter = v,
        _ => return false,
    }
    true
}

/// Writes one canonical field. `value` is the field's bytes as a frame
/// trace stores them: little-endian integers, and a region's bytes without
/// its length.
pub fn write_field(bus: &mut Bus, name: &str, value: &[u8]) -> Result<(), PokeError> {
    let not = || PokeError::NotCopyable(name.to_owned());
    if !copyable(name) {
        return Err(not());
    }
    // Every fixed field is an integer of its width (or an array, below).
    let int = |n: usize| -> Result<u64, PokeError> {
        if value.len() != n {
            return Err(PokeError::Size {
                name: name.into(),
                got: value.len(),
                want: n,
            });
        }
        Ok(value
            .iter()
            .enumerate()
            .fold(0, |acc, (i, b)| acc | u64::from(*b) << (8 * i)))
    };
    let u8v = || int(1).map(|v| v as u8);
    let u16v = || int(2).map(|v| v as u16);
    let flag = || u8v().map(|v| v != 0);
    let cpu = &mut bus.cpu;
    match name {
        "cpu.a" => cpu.acc = u8v()?,
        "cpu.x" => cpu.x = u8v()?,
        "cpu.y" => cpu.y = u8v()?,
        "cpu.s" => cpu.sp = u8v()?,
        "cpu.p" => cpu.status = Status::from_bits_retain(u8v()?),
        "cpu.pc" => cpu.pc = u16v()?,
        "cpu.cycles" => cpu.cycle = int(8)?,
        "cpu.interrupts" => {
            let bits = IrqFlags::NMI
                | IrqFlags::PREV_NMI
                | IrqFlags::PREV_NMI_PENDING
                | IrqFlags::RUN_IRQ
                | IrqFlags::PREV_RUN_IRQ;
            let v = IrqFlags::from_bits_retain(u8v()?) & bits;
            cpu.irq_flags = (cpu.irq_flags - bits) | v;
        }
        "cpu.dma" => {
            let v = u8v()?;
            cpu.irq_flags.set(IrqFlags::DMA_DMC, v & 1 != 0);
            cpu.irq_flags.set(IrqFlags::DMA_HALT, v & 2 != 0);
            cpu.irq_flags.set(IrqFlags::DMA_DUMMY_READ, v & 4 != 0);
            if v & 8 == 0 {
                cpu.dma_oam_addr = None;
            } else if cpu.dma_oam_addr.is_none() {
                cpu.dma_oam_addr = Some(0);
            }
        }
        "cpu.oam_dma_page" => {
            let page = u16::from(u8v()?) << 8;
            if let Some(a) = &mut cpu.dma_oam_addr {
                *a = page | (*a & 0xFF);
            }
        }
        "ram" => {
            let ram: [u8; 2048] = exact(name, value)?;
            bus.wram[0..2048].copy_from_slice(&ram);
        }
        "ppu.scanline" => bus.ppu.scanline = u16v()?,
        "ppu.dot" => bus.ppu.cycle = u16v()?,
        "ppu.odd_frame" => {
            let odd = u32::from(flag()?);
            bus.ppu.frame.count = (bus.ppu.frame.count & !1) | odd;
        }
        // Bits 0-1 live in t, written with ppu.t.
        "ppu.ctrl" => bus.ppu.write_ctrl(u8v()?),
        "ppu.mask" => bus.ppu.write_mask(u8v()?),
        "ppu.status" => {
            let v = u8v()?;
            bus.ppu.status_spr_overflow = v & 0x20 != 0;
            bus.ppu.status_spr_zero_hit = v & 0x40 != 0;
            bus.ppu.status_in_vblank = v & 0x80 != 0;
        }
        "ppu.oamaddr" => bus.ppu.oamaddr = u8v()?,
        "ppu.v" => bus.ppu.scroll.v = u16v()?,
        "ppu.t" => bus.ppu.scroll.t = u16v()?,
        "ppu.x" => bus.ppu.scroll.fine_x = u16::from(u8v()?),
        "ppu.w" => bus.ppu.scroll.write_latch = flag()?,
        "ppu.read_buffer" => bus.ppu.vram_buffer = u8v()?,
        "ppu.io_latch" => bus.ppu.open_bus = u8v()?,
        "ppu.io_latch_age" => {
            let packed = int(4)?;
            for (bit, age) in bus.ppu.open_bus_age.iter_mut().enumerate() {
                *age = (packed >> (3 * bit) & 7) as u8;
            }
        }
        "ppu.nmi_pending" => bus.ppu.nmi_pending = flag()?,
        "oam" => {
            let oam: [u8; 256] = exact(name, value)?;
            bus.ppu.oamdata[0..256].copy_from_slice(&oam);
        }
        "nametables" => region(bus, Src::CiRam, name, value)?,
        "chr_ram" => {
            let ram = bus.memory.chr_pages()[0].is_writable();
            if ram {
                region(bus, Src::Chr, name, value)?;
            } else if !value.is_empty() {
                return Err(PokeError::Size {
                    name: name.into(),
                    got: value.len(),
                    want: 0,
                });
            }
        }
        "prg_ram" => region(bus, Src::PrgRam, name, value)?,
        _ => {
            return write_apu_or_pad(bus, name, value)
                .and_then(|ok| ok.then_some(()).ok_or_else(not));
        }
    }
    Ok(())
}

fn write_apu_or_pad(bus: &mut Bus, name: &str, value: &[u8]) -> Result<bool, PokeError> {
    let width = |n: usize| -> Result<u32, PokeError> {
        if value.len() != n {
            return Err(PokeError::Size {
                name: name.into(),
                got: value.len(),
                want: n,
            });
        }
        Ok(value
            .iter()
            .enumerate()
            .fold(0, |acc, (i, b)| acc | u32::from(*b) << (8 * i)))
    };
    let field_width = crate::canonical::FIELDS
        .iter()
        .find(|f| f.name == name)
        .map(|f| match f.width {
            crate::canonical::Width::Fixed(n) => n,
            crate::canonical::Width::Region => 0,
        })
        .unwrap_or(0);
    let v = width(field_width)?;
    let (b8, b16) = (v as u8, v as u16);
    let apu = &mut bus.apu;
    for (ch, p) in [
        ("apu.pulse1.", &mut apu.pulse1),
        ("apu.pulse2.", &mut apu.pulse2),
    ] {
        let Some(part) = name.strip_prefix(ch) else {
            continue;
        };
        let done = timer(&mut p.timer, part, b16)
            || length(&mut p.length, part, b8)
            || envelope(&mut p.envelope, part, b8);
        if done {
            return Ok(true);
        }
        match part {
            "duty" => p.duty = b8,
            "duty_step" => p.duty_cycle = b8,
            "sweep.enabled" => p.sweep.enabled = b8 != 0,
            "sweep.negate" => p.sweep.negate = b8 != 0,
            "sweep.reload" => p.sweep.reload = b8 != 0,
            "sweep.shift" => p.sweep.shift = b8,
            "sweep.period" => p.sweep.period = b8,
            "sweep.divider" => p.sweep.divider = b8,
            _ => return Ok(false),
        }
        return Ok(true);
    }
    if let Some(part) = name.strip_prefix("apu.triangle.") {
        let t = &mut apu.triangle;
        if timer(&mut t.timer, part, b16) || length(&mut t.length, part, b8) {
            return Ok(true);
        }
        match part {
            "step" => t.sequence = b8,
            "linear.reload" => t.linear.reload = b8 != 0,
            "linear.control" => t.linear.control = b8 != 0,
            "linear.reload_value" => t.linear.counter_reload = b8,
            "linear.counter" => t.linear.counter = b8,
            _ => return Ok(false),
        }
        return Ok(true);
    }
    if let Some(part) = name.strip_prefix("apu.noise.") {
        let n = &mut apu.noise;
        if timer(&mut n.timer, part, b16)
            || length(&mut n.length, part, b8)
            || envelope(&mut n.envelope, part, b8)
        {
            return Ok(true);
        }
        match part {
            "mode" => {
                n.shift_mode = if b8 != 0 {
                    ShiftMode::One
                } else {
                    ShiftMode::Zero
                }
            }
            "shift" => n.shift = b16,
            _ => return Ok(false),
        }
        return Ok(true);
    }
    if let Some(part) = name.strip_prefix("apu.dmc.") {
        let d = &mut apu.dmc;
        if timer(&mut d.timer, part, b16) {
            return Ok(true);
        }
        match part {
            "irq_enabled" => d.irq_enabled = b8 != 0,
            "irq_pending" => d.irq_pending = b8 != 0,
            "loop" => d.loops = b8 != 0,
            "sample_addr" => d.sample_addr = b16,
            "sample_length" => d.sample_length = b16,
            "addr" => d.addr = b16,
            "bytes_remaining" => d.bytes_remaining = b16,
            "sample_buffer" => d.sample_buffer = b8,
            "buffer_empty" => d.buffer_empty = b8 != 0,
            "output_level" => d.output_level = b8,
            "shift" => d.shift = b8,
            "bits_remaining" => d.bits_remaining = b8,
            "silence" => d.silence = b8 != 0,
            _ => return Ok(false),
        }
        return Ok(true);
    }
    if let Some(part) = name.strip_prefix("apu.frame_counter.") {
        let fc = &mut apu.frame_counter;
        match part {
            "mode" => fc.mode = b8,
            "step" => fc.step = usize::from(b8),
            "cycle" => fc.cycle = v,
            "irq_inhibit" => fc.inhibit_irq = b8 != 0,
            "irq_pending" => fc.irq_pending = b8 != 0,
            "write_pending" => {
                if b8 == 0 {
                    fc.write_buffer = None;
                } else if fc.write_buffer.is_none() {
                    fc.write_buffer = Some(0);
                }
            }
            "write_value" => {
                if let Some(w) = &mut fc.write_buffer {
                    *w = b8;
                }
            }
            "write_delay" => fc.write_delay = b8,
            _ => return Ok(false),
        }
        return Ok(true);
    }
    for (pad, i) in [("pad1.", 0), ("pad2.", 1)] {
        if let Some(part) = name.strip_prefix(pad) {
            let j = &mut bus.input.joypads[i];
            match part {
                "index" => j.index = b8,
                "strobe" => j.strobe = b8 != 0,
                _ => return Ok(false),
            }
            return Ok(true);
        }
    }
    Ok(false)
}

/// Writes every copyable field of a canonical state. The order matters in
/// two places: `ppu.ctrl` before `ppu.t` (writing PPUCTRL doesn't touch t
/// here, but t holds its nametable bits), and `cpu.dma` before
/// `cpu.oam_dma_page`, as in [`crate::canonical::FIELDS`].
pub fn write_state(bus: &mut Bus, fields: &[(&str, &[u8])]) -> Result<(), PokeError> {
    for (name, value) in fields {
        if copyable(name) {
            write_field(bus, name, value)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Nes, canonical, testrom};
    use gp_evidence::InputFrame;
    use gp_evidence::manifest::PowerOn;

    /// Copying every field of one console's state into another gives the
    /// first console's canonical state back, palette and mapper aside,
    /// which the two share here.
    #[test]
    fn a_whole_state_written_reads_back() {
        let rom = testrom::build();
        let mut a = Nes::power_on(&rom, PowerOn::default()).unwrap();
        let pressed = InputFrame {
            pads: [0x81, 0, 0, 0],
            ..InputFrame::default()
        };
        for i in 0..77 {
            a.step(if i % 3 == 0 {
                pressed
            } else {
                InputFrame::default()
            })
            .unwrap();
        }
        let want = a.canonical_state().unwrap();
        let fields = canonical::decode(&want).unwrap();
        let mut b = Nes::power_on(&rom, PowerOn::default()).unwrap();
        b.step(InputFrame::default()).unwrap();
        let mut checked = false;
        b.step_with(InputFrame::default(), |bus| {
            if checked {
                return;
            }
            write_state(bus, &fields).unwrap();
            let got = canonical::encode(bus).unwrap();
            assert_eq!(got, want);
            checked = true;
        })
        .unwrap();
        assert!(checked);
    }

    #[test]
    fn every_field_but_palette_and_mapper_is_copyable() {
        for f in canonical::FIELDS {
            assert_eq!(
                copyable(f.name),
                !NOT_COPYABLE.contains(&f.name)
                    && !f.name.starts_with("mmc1.")
                    && !f.name.starts_with("mmc3.")
                    && !f.name.starts_with("cnrom."),
                "{}",
                f.name
            );
        }
        assert!(!copyable("cpu.cycles64"));
    }

    #[test]
    fn refuses_what_it_cant_write() {
        let rom = testrom::build();
        let mut nes = Nes::power_on(&rom, PowerOn::default()).unwrap();
        let mut result = None;
        nes.step_with(InputFrame::default(), |bus| {
            if result.is_none() {
                result = Some((
                    write_field(bus, "palette", &[0; 32]),
                    write_field(bus, "ram", &[0; 3]),
                ));
            }
        })
        .unwrap();
        let (palette, short) = result.unwrap();
        assert_eq!(palette, Err(PokeError::NotCopyable("palette".into())));
        assert!(matches!(short, Err(PokeError::Size { want: 2048, .. })));
    }
}
