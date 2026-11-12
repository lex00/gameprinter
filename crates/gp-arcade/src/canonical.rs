//! The state a recording keeps for each frame of an arcade run: named
//! fields a reader can split without this crate, and the whole machine
//! after them.
//!
//! The layout is a 4-byte magic (`GPAR`), a u32 version, then the fields of
//! [`FIELDS`] in order: little-endian numbers and fixed byte arrays. The
//! fields up to `ports` repeat parts of the machine by name, so `gp trace
//! frame` and `gp trace diff` can say which part changed; the last,
//! `machine`, is the BLAKE3 hash of the snapshot ([`Tapper::snapshot`]),
//! so the state's hash covers everything a restore needs while every
//! frame's state keeps one size (a trace chunk wants that; the snapshot's
//! timer list and pending sound writes vary). The snapshots themselves are
//! the recording's checkpoints. A recording's field table is made from [`FIELDS`] (gp-cli's
//! arcade recordings), the same way gp-nes's canonical state has its own.

use crate::Tapper;

pub const STATE_MAGIC: [u8; 4] = *b"GPAR";
/// Bumped when a field is added, removed or changes width.
pub const STATE_VERSION: u32 = 2;
/// The `machine` field's size: a BLAKE3 hash.
pub const MACHINE_BYTES: u32 = 32;

/// Each field's name and width in bytes (`None` would be a region; the
/// arcade state has none).
pub const FIELDS: &[(&str, Option<u32>)] = &[
    ("frame", Some(8)),
    ("cpu.pc", Some(2)),
    ("cpu.sp", Some(2)),
    ("cpu.af", Some(2)),
    ("cpu.bc", Some(2)),
    ("cpu.de", Some(2)),
    ("cpu.hl", Some(2)),
    ("cpu.ix", Some(2)),
    ("cpu.iy", Some(2)),
    ("cpu.i", Some(1)),
    ("cpu.iff1", Some(1)),
    ("cpu.im", Some(1)),
    ("cpu.halt", Some(1)),
    ("sound_cpu.pc", Some(2)),
    ("sound_cpu.sp", Some(2)),
    ("sound_cpu.af", Some(2)),
    ("sound_cpu.bc", Some(2)),
    ("sound_cpu.de", Some(2)),
    ("sound_cpu.hl", Some(2)),
    ("sound_cpu.ix", Some(2)),
    ("sound_cpu.iy", Some(2)),
    ("sound_cpu.i", Some(1)),
    ("sound_cpu.iff1", Some(1)),
    ("sound_cpu.im", Some(1)),
    ("sound_cpu.halt", Some(1)),
    // e000-e7ff, the work RAM (battery backed).
    ("ram", Some(0x800)),
    ("sprite_ram", Some(0x200)),
    ("video_ram", Some(0x800)),
    ("palette_ram", Some(0x80)),
    ("sound_ram", Some(0x400)),
    ("sound_latch", Some(4)),
    ("sound_status", Some(1)),
    ("ay0", Some(16)),
    ("ay1", Some(16)),
    ("ports", Some(5)),
    ("machine", Some(MACHINE_BYTES)),
];

impl Tapper {
    /// The recorded state ([`FIELDS`]).
    pub fn recorded_state(&self) -> Vec<u8> {
        let snap = self.snapshot();
        let b = &self.board;
        let mut out = Vec::with_capacity(8 + 0x1900);
        out.extend_from_slice(&STATE_MAGIC);
        out.extend_from_slice(&STATE_VERSION.to_le_bytes());
        out.extend_from_slice(&self.frame().to_le_bytes());
        for z in [&self.main, &self.ssio] {
            for v in [z.pc, z.sp, z.af, z.bc, z.de, z.hl, z.ix, z.iy] {
                out.extend_from_slice(&v.to_le_bytes());
            }
            out.extend_from_slice(&[z.i, z.iff1, z.im, z.halt]);
        }
        out.extend_from_slice(&b.nvram);
        out.extend_from_slice(&b.spriteram);
        out.extend_from_slice(&b.videoram);
        out.extend_from_slice(&b.paletteram);
        out.extend_from_slice(&b.ssio_ram);
        out.extend_from_slice(&b.ssio_data);
        out.push(b.ssio_status);
        out.extend_from_slice(&b.sound.bus[0].regs);
        out.extend_from_slice(&b.sound.bus[1].regs);
        out.extend_from_slice(&b.ports);
        out.extend_from_slice(blake3::hash(&snap).as_bytes());
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fields before `machine` add up to where the snapshot starts.
    #[test]
    fn the_fields_cover_the_state() {
        let fixed: u32 = FIELDS.iter().map(|f| f.1.unwrap()).sum();
        let rom = crate::testrom::looping();
        let t = Tapper::power_on(&rom);
        let s = t.recorded_state();
        assert_eq!(s.len(), 8 + fixed as usize);
        let at = s.len() - MACHINE_BYTES as usize;
        assert_eq!(&s[at..], blake3::hash(&t.snapshot()).as_bytes());
    }
}
