//! Coverage: what a recording, or a corpus of them, exercised. Four
//! measures, all derived by replaying recordings (docs/coverage.md):
//!
//! - executed ROM bytes, by PRG ROM offset, with the CPU addresses each
//!   1KB page ran at, so a bank shows where it was mapped;
//! - RAM bytes written, internal RAM and PRG RAM;
//! - screens and levels visited, and objects seen, as a per-game
//!   definition reads them from RAM ([`definition`]);
//! - controller button combinations on frames the game read them.
//!
//! [`measure::measure`] covers one recording, [`Coverage::union`] combines
//! recordings of one ROM, [`cache`] keeps each recording's result in the
//! store, and [`report`] builds what `gp coverage --json` prints.

pub mod activity;
pub mod cache;
pub mod definition;
pub mod listing;
pub mod measure;
pub mod report;

use std::collections::BTreeMap;

use gp_evidence::Hash;
use gp_evidence::input_log::button;
use serde::{Deserialize, Serialize};

pub use definition::{Definition, Thresholds};
pub use gp_events::Bits;
pub use gp_events::bits;

/// The version of how coverage is measured. Bump it with any change that
/// gives a recording different coverage; stored results of another version
/// are measured again.
pub const COVERAGE_VERSION: u32 = 1;

/// Where a measure was seen: on how many frames, in how many recordings,
/// and, for one recording, the first frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Seen {
    pub frames: u64,
    pub recordings: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first: Option<u64>,
}

impl Seen {
    fn at(frame: u64) -> Self {
        Self {
            frames: 1,
            recordings: 1,
            first: Some(frame),
        }
    }

    fn add_frame(&mut self, frame: u64) {
        self.frames += 1;
        self.first = Some(self.first.map_or(frame, |f| f.min(frame)));
    }
}

/// Which definition named the screens, levels and objects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefinitionRef {
    pub source: String,
    pub hash: Hash,
}

/// Coverage of one recording, or of several recordings of one ROM.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Coverage {
    pub coverage_version: u32,
    /// The ROM: headerless SHA-256 and iNES mapper number.
    pub sha256: String,
    pub mapper: u16,
    /// The recordings covered, by ID.
    pub recordings: Vec<Hash>,
    pub frames: u64,
    pub instructions: u64,
    /// Instructions run from outside PRG ROM (RAM, usually).
    pub outside_rom: u64,
    /// Every byte of every instruction run, by PRG ROM offset.
    pub executed: Bits,
    /// The first byte of each instruction run, by PRG ROM offset.
    pub starts: Bits,
    /// For each 1KB page of PRG ROM, the CPU addresses of the pages it ran
    /// at: `[32768]` for a page seen only at $8000.
    pub windows: Vec<Vec<u16>>,
    /// Internal RAM bytes written, $0000-$07FF.
    pub ram_written: Bits,
    /// PRG RAM bytes written, by offset (empty for a cartridge without).
    pub prg_ram_written: Bits,
    /// Button combinations, by name, on frames the game read the
    /// controllers: port 1 then port 2.
    pub inputs: [BTreeMap<String, u64>; 2],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition: Option<DefinitionRef>,
    #[serde(default)]
    pub screens: BTreeMap<String, Seen>,
    #[serde(default)]
    pub levels: BTreeMap<String, Seen>,
    #[serde(default)]
    pub objects: BTreeMap<String, Seen>,
}

impl Coverage {
    /// Combines coverage of recordings of the same ROM, measured with the
    /// same definition. A recording listed twice counts once.
    pub fn union(items: &[Coverage]) -> Result<Coverage, String> {
        let (first, rest) = items
            .split_first()
            .ok_or("there's no coverage to combine")?;
        let mut out = first.clone();
        let mut seen: Vec<Hash> = first.recordings.clone();
        for c in rest {
            if c.sha256 != out.sha256 {
                return Err(format!(
                    "coverage of two ROMs ({}... and {}...) can't be combined",
                    &out.sha256[..12],
                    &c.sha256[..12]
                ));
            }
            if c.definition != out.definition {
                return Err("coverage measured with two different definitions can't be combined; measure again with one".into());
            }
            if c.coverage_version != out.coverage_version {
                return Err("coverage of two coverage versions can't be combined".into());
            }
            if c.recordings.iter().all(|r| seen.contains(r)) {
                continue;
            }
            if c.recordings.iter().any(|r| seen.contains(r)) {
                return Err("coverage that shares some recordings with another can't be combined without counting them twice; combine single recordings".into());
            }
            seen.extend(c.recordings.iter().copied());
            out.recordings.extend(c.recordings.iter().copied());
            out.frames += c.frames;
            out.instructions += c.instructions;
            out.outside_rom += c.outside_rom;
            out.executed.union(&c.executed)?;
            out.starts.union(&c.starts)?;
            for (a, b) in out.windows.iter_mut().zip(&c.windows) {
                for addr in b {
                    if !a.contains(addr) {
                        a.push(*addr);
                    }
                }
                a.sort_unstable();
            }
            out.ram_written.union(&c.ram_written)?;
            out.prg_ram_written.union(&c.prg_ram_written)?;
            for port in 0..2 {
                for (combo, frames) in &c.inputs[port] {
                    *out.inputs[port].entry(combo.clone()).or_default() += frames;
                }
            }
            for (mine, theirs) in [
                (&mut out.screens, &c.screens),
                (&mut out.levels, &c.levels),
                (&mut out.objects, &c.objects),
            ] {
                for (name, s) in theirs {
                    let e = mine.entry(name.clone()).or_insert(Seen {
                        frames: 0,
                        recordings: 0,
                        first: None,
                    });
                    e.frames += s.frames;
                    e.recordings += s.recordings;
                }
            }
        }
        if out.recordings.len() > 1 {
            for map in [&mut out.screens, &mut out.levels, &mut out.objects] {
                for s in map.values_mut() {
                    s.first = None;
                }
            }
        }
        Ok(out)
    }

    /// The bank size coverage is reported in: 8KB for MMC3, which switches
    /// 8KB banks, and 16KB otherwise.
    pub fn bank_size(&self) -> usize {
        match self.mapper {
            4 => 0x2000,
            _ => 0x4000,
        }
    }

    /// The numbers [`report`] and the metrics show.
    pub fn summary(&self) -> Summary {
        let bank = self.bank_size();
        let banks = (0..self.executed.len().div_ceil(bank))
            .map(|b| {
                let start = b * bank;
                let end = (start + bank).min(self.executed.len());
                BankSummary {
                    bank: b,
                    size: end - start,
                    executed: (start..end).filter(|&i| self.executed.contains(i)).count(),
                }
            })
            .collect();
        let prg = self.executed.len() as u64;
        Summary {
            recordings: self.recordings.len(),
            frames: self.frames,
            instructions: self.instructions,
            prg_rom_bytes: prg,
            executed_bytes: self.executed.count(),
            executed_percent: percent(self.executed.count(), prg),
            instruction_starts: self.starts.count(),
            bank_size: bank,
            banks,
            ram_bytes: self.ram_written.len() as u64,
            ram_written_bytes: self.ram_written.count(),
            prg_ram_bytes: self.prg_ram_written.len() as u64,
            prg_ram_written_bytes: self.prg_ram_written.count(),
            screens: self.screens.len(),
            levels: self.levels.len(),
            objects: self.objects.len(),
            input_combinations: self.inputs[0].len(),
        }
    }

    /// Checks the thresholds against this coverage.
    pub fn check(&self, t: &Thresholds) -> Check {
        let missing = |want: &[String], have: &BTreeMap<String, Seen>| Requirement {
            required: want.len(),
            missing: want
                .iter()
                .filter(|w| !have.contains_key(*w))
                .cloned()
                .collect(),
        };
        let inputs = Requirement {
            required: t.inputs.len(),
            missing: t
                .inputs
                .iter()
                .filter(|w| {
                    let name = parse_combo(w).map(combo_name).unwrap_or_default();
                    !self.inputs[0].contains_key(&name)
                })
                .cloned()
                .collect(),
        };
        let screens = missing(&t.screens, &self.screens);
        let levels = missing(&t.levels, &self.levels);
        let objects = missing(&t.objects, &self.objects);
        Check {
            passed: screens.missing.is_empty()
                && levels.missing.is_empty()
                && objects.missing.is_empty()
                && inputs.missing.is_empty(),
            levels,
            objects,
            screens,
            inputs,
        }
    }
}

fn percent(n: u64, of: u64) -> f64 {
    if of == 0 {
        0.0
    } else {
        // Two decimals, so the JSON doesn't carry float noise.
        (n as f64 * 10000.0 / of as f64).round() / 100.0
    }
}

/// Counts from a [`Coverage`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    pub recordings: usize,
    pub frames: u64,
    pub instructions: u64,
    pub prg_rom_bytes: u64,
    pub executed_bytes: u64,
    pub executed_percent: f64,
    pub instruction_starts: u64,
    pub bank_size: usize,
    pub banks: Vec<BankSummary>,
    pub ram_bytes: u64,
    pub ram_written_bytes: u64,
    pub prg_ram_bytes: u64,
    pub prg_ram_written_bytes: u64,
    pub screens: usize,
    pub levels: usize,
    pub objects: usize,
    pub input_combinations: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BankSummary {
    pub bank: usize,
    pub size: usize,
    pub executed: usize,
}

/// The result of checking [`Thresholds`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    pub passed: bool,
    pub screens: Requirement,
    pub levels: Requirement,
    pub objects: Requirement,
    pub inputs: Requirement,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Requirement {
    pub required: usize,
    pub missing: Vec<String>,
}

/// Button names in the order input logs write them (Right, Left, Down, Up,
/// Start, Select, B, A), with their bits.
const BUTTONS: [(&str, u8); 8] = [
    ("right", button::RIGHT),
    ("left", button::LEFT),
    ("down", button::DOWN),
    ("up", button::UP),
    ("start", button::START),
    ("select", button::SELECT),
    ("b", button::B),
    ("a", button::A),
];

/// A combination's name: the buttons held, joined with `+` in input log
/// order, such as `right+b+a`; `none` for no buttons.
pub fn combo_name(pad: u8) -> String {
    let names: Vec<&str> = BUTTONS
        .iter()
        .filter(|(_, bit)| pad & bit != 0)
        .map(|(name, _)| *name)
        .collect();
    if names.is_empty() {
        "none".into()
    } else {
        names.join("+")
    }
}

/// The buttons a combination names, in any order.
pub fn parse_combo(text: &str) -> Result<u8, String> {
    if text == "none" {
        return Ok(0);
    }
    let mut pad = 0;
    for part in text.split('+') {
        let bit = BUTTONS
            .iter()
            .find(|(name, _)| *name == part)
            .map(|(_, bit)| *bit)
            .ok_or_else(|| {
                format!(
                    "input combination {text:?}: {part:?} isn't a button (right, left, down, up, start, select, b, a, or none alone)"
                )
            })?;
        pad |= bit;
    }
    Ok(pad)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combinations_have_one_name() {
        assert_eq!(combo_name(0), "none");
        assert_eq!(
            combo_name(button::A | button::RIGHT | button::B),
            "right+b+a"
        );
        for pad in 0..=255u8 {
            assert_eq!(parse_combo(&combo_name(pad)).unwrap(), pad);
        }
        assert_eq!(parse_combo("a+right").unwrap(), button::A | button::RIGHT);
        assert!(parse_combo("jump").is_err());
    }
}
