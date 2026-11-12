//! Reading what the MAME oracle writes (docs/oracle-mame.md): the input
//! log, `frames.tsv`, `ram.bin` and `ay.tsv`, and comparing a run of ours
//! with it.

use std::path::{Path, PathBuf};

use crate::mcr::{Controls, RAM_RECORD, Tapper};

/// The oracle's input log: one entry per frame from frame 1, as bits in
/// the order of its `columns` line (CSRLDUT for Tapper).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputLog {
    pub system: String,
    pub columns: String,
    pub frames: Vec<u32>,
}

impl InputLog {
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut lines = text.lines().enumerate();
        match lines.next() {
            Some((_, l)) if l.trim_end() == "gp-arcade-input 0" => {}
            _ => return Err("line 1: expected `gp-arcade-input 0`".into()),
        }
        let mut system = None;
        let mut columns: Option<String> = None;
        let mut frames = Vec::new();
        for (n, raw) in lines {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            if let Some(s) = line.strip_prefix("system ") {
                system = Some(s.trim().to_string());
                continue;
            }
            if let Some(c) = line.strip_prefix("columns ") {
                columns = Some(c.trim().to_string());
                continue;
            }
            let cols = columns
                .as_deref()
                .ok_or(format!("line {}: frames before `columns`", n + 1))?;
            let mut parts = line.split_whitespace();
            let count: usize = parts
                .next()
                .and_then(|c| c.parse().ok())
                .ok_or(format!("line {}: expected `<frames> <buttons>`", n + 1))?;
            let buttons = parts.next().ok_or(format!("line {}: no buttons", n + 1))?;
            if buttons.len() != cols.len() {
                return Err(format!(
                    "line {}: {} buttons, the columns are {cols}",
                    n + 1,
                    buttons.len()
                ));
            }
            let mut bits = 0;
            for (i, (b, c)) in buttons.chars().zip(cols.chars()).enumerate() {
                if b == c {
                    bits |= 1 << i;
                } else if b != '.' {
                    return Err(format!(
                        "line {}: column {} is `{b}`, expected `.` or `{c}`",
                        n + 1,
                        i + 1
                    ));
                }
            }
            frames.extend(std::iter::repeat_n(bits, count));
        }
        Ok(Self {
            system: system.ok_or("no `system` line")?,
            columns: columns.ok_or("no `columns` line")?,
            frames,
        })
    }

    /// The controls held in frame `n` (1 is the first frame).
    pub fn controls(&self, n: usize) -> Controls {
        Controls::from_bits(self.frames.get(n.wrapping_sub(1)).copied().unwrap_or(0))
    }
}

/// One line of frames.tsv.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameRow {
    pub frame: u64,
    pub time: String,
    pub ports: [u8; 5],
    pub picture: u64,
    pub main_pc: u16,
    pub ssio_pc: u16,
    pub shot: Option<String>,
}

/// A capture folder.
pub struct Capture {
    pub dir: PathBuf,
    pub rows: Vec<FrameRow>,
    pub ram: Vec<u8>,
    /// Each frame's AY writes as `ay.tsv` spells them ([`ay_text`]), when
    /// the capture has the file (captures from before it don't).
    pub ay: Option<Vec<String>>,
}

impl Capture {
    pub fn load(dir: &Path) -> Result<Self, String> {
        let tsv = std::fs::read_to_string(dir.join("frames.tsv"))
            .map_err(|e| format!("{}: {e}", dir.join("frames.tsv").display()))?;
        let mut rows = Vec::new();
        for (n, line) in tsv.lines().enumerate().skip(1) {
            let f: Vec<&str> = line.split('\t').collect();
            if f.len() != 12 {
                return Err(format!("frames.tsv line {}: {} fields", n + 1, f.len()));
            }
            let hex8 = |s: &str| {
                u8::from_str_radix(s, 16).map_err(|e| format!("frames.tsv line {}: {e}", n + 1))
            };
            let hex16 = |s: &str| {
                u16::from_str_radix(s, 16).map_err(|e| format!("frames.tsv line {}: {e}", n + 1))
            };
            rows.push(FrameRow {
                frame: f[0]
                    .parse()
                    .map_err(|e| format!("frames.tsv line {}: {e}", n + 1))?,
                time: f[1].to_string(),
                ports: [
                    hex8(f[3])?,
                    hex8(f[4])?,
                    hex8(f[5])?,
                    hex8(f[6])?,
                    hex8(f[7])?,
                ],
                picture: u64::from_str_radix(f[8], 16)
                    .map_err(|e| format!("frames.tsv line {}: {e}", n + 1))?,
                main_pc: hex16(f[9])?,
                ssio_pc: hex16(f[10])?,
                shot: (f[11] != "-").then(|| f[11].to_string()),
            });
        }
        let ram = std::fs::read(dir.join("ram.bin"))
            .map_err(|e| format!("{}: {e}", dir.join("ram.bin").display()))?;
        if ram.len() != rows.len() * RAM_RECORD {
            return Err(format!(
                "ram.bin holds {} bytes, {} frames of {RAM_RECORD} need {}",
                ram.len(),
                rows.len(),
                rows.len() * RAM_RECORD
            ));
        }
        let ay = match std::fs::read_to_string(dir.join("ay.tsv")) {
            Ok(text) => {
                let mut v = Vec::with_capacity(rows.len());
                for (n, line) in text.lines().enumerate().skip(1) {
                    let (frame, writes) = line
                        .split_once('\t')
                        .ok_or(format!("ay.tsv line {}: no tab", n + 1))?;
                    if frame.parse::<usize>().ok() != Some(v.len()) {
                        return Err(format!("ay.tsv line {}: frame {frame} out of order", n + 1));
                    }
                    v.push(if writes == "-" {
                        String::new()
                    } else {
                        writes.to_string()
                    });
                }
                if v.len() != rows.len() {
                    return Err(format!(
                        "ay.tsv has {} frames, frames.tsv {}",
                        v.len(),
                        rows.len()
                    ));
                }
                Some(v)
            }
            Err(_) => None,
        };
        Ok(Self {
            dir: dir.to_path_buf(),
            rows,
            ram,
            ay,
        })
    }

    pub fn ram(&self, frame: usize) -> &[u8] {
        &self.ram[frame * RAM_RECORD..(frame + 1) * RAM_RECORD]
    }
}

/// Where in the RAM record a byte is, as the main or sound CPU addresses it.
pub fn ram_address(offset: usize) -> String {
    match offset {
        0..2048 => format!("main {:04x} (work RAM)", 0xe000 + offset),
        2048..2560 => format!("main {:04x} (sprite RAM)", 0xe800 + offset - 2048),
        2560..4608 => format!("main {:04x} (video RAM)", 0xf000 + offset - 2560),
        4608..4736 => format!("main {:04x} (palette RAM)", 0xf800 + offset - 4608),
        _ => format!("ssio {:04x} (sound RAM)", 0x8000 + offset - 4736),
    }
}

/// A frame's AY writes as `ay.tsv` spells them: the chip, `a` (address
/// latch) or `d` (data latch) and the value in hex, a space between.
pub fn ay_text(writes: &[(u8, u8, u8)]) -> String {
    writes
        .iter()
        .map(|&(chip, latch, v)| format!("{chip}{}{v:02x}", if latch == 0 { 'a' } else { 'd' }))
        .collect::<Vec<_>>()
        .join(" ")
}

/// How one frame of ours differs from the capture's.
#[derive(Debug, Clone, Default)]
pub struct FrameDiff {
    /// (offset in the RAM record, ours, MAME's)
    pub ram: Vec<(usize, u8, u8)>,
    pub ports: Option<([u8; 5], [u8; 5])>,
    pub picture: Option<(u64, u64)>,
    /// (ours, MAME's) AY writes, as [`ay_text`] spells them.
    pub ay: Option<(String, String)>,
}

impl FrameDiff {
    pub fn is_empty(&self) -> bool {
        self.ram.is_empty() && self.ports.is_none() && self.picture.is_none() && self.ay.is_none()
    }

    pub fn describe(&self, max_bytes: usize) -> String {
        let mut out = Vec::new();
        if let Some((ours, theirs)) = self.ports {
            out.push(format!("ports: ours {ours:02x?}, MAME's {theirs:02x?}"));
        }
        if let Some((ours, theirs)) = self.picture {
            out.push(format!("picture: ours {ours:016x}, MAME's {theirs:016x}"));
        }
        if let Some((ours, theirs)) = &self.ay {
            out.push(format!("AY writes: ours [{ours}], MAME's [{theirs}]"));
        }
        if !self.ram.is_empty() {
            out.push(format!("{} RAM bytes differ:", self.ram.len()));
            for &(off, a, b) in self.ram.iter().take(max_bytes) {
                out.push(format!(
                    "  {}: ours {a:02x}, MAME's {b:02x}",
                    ram_address(off)
                ));
            }
            if self.ram.len() > max_bytes {
                out.push(format!("  and {} more", self.ram.len() - max_bytes));
            }
        }
        out.join("\n")
    }
}

/// Compares our machine, at the end of frame `n`, with the capture's.
pub fn compare(t: &Tapper, cap: &Capture, n: usize) -> FrameDiff {
    let ours = t.ram_record();
    let theirs = cap.ram(n);
    let ram = ours
        .iter()
        .zip(theirs)
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(i, (&a, &b))| (i, a, b))
        .collect();
    let row = &cap.rows[n];
    let ports = (n > 0 && t.ports() != row.ports).then_some((t.ports(), row.ports));
    let ph = t.picture_hash();
    let picture = (ph != row.picture).then_some((ph, row.picture));
    let ay = cap.ay.as_ref().and_then(|v| {
        let ours = ay_text(t.ay_writes());
        (ours != v[n]).then(|| (ours, v[n].clone()))
    });
    FrameDiff {
        ram,
        ports,
        picture,
        ay,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_input_log() {
        let log = InputLog::parse(
            "gp-arcade-input 0\n# a comment\nsystem rbtapper\ncolumns CSRLDUT\n2 .......\n1 C.....T # coin and tap\n",
        )
        .unwrap();
        assert_eq!(log.system, "rbtapper");
        assert_eq!(log.frames, vec![0, 0, 0b100_0001]);
        assert_eq!(
            log.controls(3),
            Controls {
                coin: true,
                tap: true,
                ..Controls::default()
            }
        );
        assert_eq!(log.controls(9), Controls::default());
        assert!(InputLog::parse("gp-arcade-input 0\nsystem x\ncolumns CS\n1 X.\n").is_err());
    }

    #[test]
    fn spells_ay_writes_as_the_oracle() {
        assert_eq!(
            ay_text(&[(0, 0, 7), (0, 2, 0x38), (1, 2, 0xf0)]),
            "0a07 0d38 1df0"
        );
        assert_eq!(ay_text(&[]), "");
    }

    #[test]
    fn names_ram_addresses() {
        assert_eq!(ram_address(0), "main e000 (work RAM)");
        assert_eq!(ram_address(4608 + 0x7f), "main f87f (palette RAM)");
        assert_eq!(ram_address(4736), "ssio 8000 (sound RAM)");
    }
}
