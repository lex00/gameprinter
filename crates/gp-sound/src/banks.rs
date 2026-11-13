//! Sound bank mappings (docs/formats/sound-banks.md) and the banks found
//! on disk.
//!
//! A mapping says what each captured instrument sounds like: the faithful
//! APU synthesizer (the default), a General MIDI program, or a patch of a free sound bank (an SF2 SoundFont preset or an SFZ
//! file's region). Rules match an instrument by name, by role, or both, in
//! a song or every song; the first rule that matches wins, and an
//! instrument no rule matches plays on the APU. A mix gives each channel a
//! gain, and mutes or solos channels. A theme's restyled-audio layer is a
//! mapping too (docs/formats/theme.md), so a theme remasters the music
//! without touching the notes or the simulation.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::apu::Channel;
use crate::song::{Instrument, SongSet};

pub const FORMAT: &str = "gp-sound-banks";
pub const VERSION: &str = "0.1.0";

/// The roles an instrument can have.
pub const ROLES: [&str; 5] = ["lead", "bass", "harmony", "drums", "sfx"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BankKind {
    /// The APU synthesizer itself.
    Apu,
    Sf2,
    Sfz,
}

/// A bank a mapping can name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bank {
    pub name: String,
    pub kind: BankKind,
    /// The file, relative to a bank folder or absolute.
    pub path: String,
    /// Its licence, as its author gives it (SPDX where there is one).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
}

/// What an instrument plays through.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum Target {
    /// The faithful APU synthesizer.
    Apu,
    /// A General MIDI program (0-127), for GM banks.
    Gm { program: u8 },
    /// A bank's patch: an SF2 preset by name or bank and program, or an
    /// SFZ file (its regions play by key).
    Bank {
        bank: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        preset: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        program: Option<u8>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bank_number: Option<u16>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instrument: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub song: Option<String>,
    pub target: Target,
    /// Loudness against the APU's, 1.0 as it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gain: Option<f64>,
    /// Semitones up (or down) when played through a bank.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transpose: Option<i8>,
}

/// Channel gains, mutes and solos.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mix {
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub gain: std::collections::BTreeMap<String, f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mute: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub solo: Vec<String>,
    /// Gains by instrument role.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub role_gain: std::collections::BTreeMap<String, f64>,
}

impl Mix {
    /// Per-channel gains for the synthesizer.
    pub fn gains(&self) -> crate::synth::Gains {
        std::array::from_fn(|i| {
            let c = Channel::ALL[i].name();
            let solo = !self.solo.is_empty() && !self.solo.iter().any(|x| x == c);
            if solo || self.mute.iter().any(|x| x == c) {
                0.0
            } else {
                self.gain.get(c).copied().unwrap_or(1.0)
            }
        })
    }
}

/// A mapping file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mapping {
    pub format: String,
    pub version: String,
    #[serde(default, rename = "bank", skip_serializing_if = "Vec::is_empty")]
    pub banks: Vec<Bank>,
    #[serde(default, rename = "map", skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<Rule>,
    #[serde(default)]
    pub mix: Mix,
}

impl Default for Mapping {
    fn default() -> Self {
        Self {
            format: FORMAT.into(),
            version: VERSION.into(),
            banks: Vec::new(),
            rules: Vec::new(),
            mix: Mix::default(),
        }
    }
}

impl Mapping {
    pub fn parse(text: &str) -> Result<Self, String> {
        let m: Mapping = toml::from_str(text).map_err(|e| format!("sound banks: {e}"))?;
        if m.format != FORMAT {
            return Err(format!("not a {FORMAT} file"));
        }
        if !m.version.starts_with("0.1.") {
            return Err(format!(
                "{FORMAT} {} isn't readable here ({VERSION})",
                m.version
            ));
        }
        m.check()?;
        Ok(m)
    }

    pub fn to_text(&self) -> String {
        toml::to_string(self).expect("a mapping serializes")
    }

    /// Rules name banks that exist, roles that exist, and channels that
    /// exist.
    pub fn check(&self) -> Result<(), String> {
        for r in &self.rules {
            if r.instrument.is_none() && r.role.is_none() && r.song.is_none() {
                return Err("a rule names an instrument, a role or a song".into());
            }
            if let Some(role) = &r.role
                && !ROLES.contains(&role.as_str())
            {
                return Err(format!("role `{role}` isn't one of {}", ROLES.join(", ")));
            }
            if let Target::Bank { bank, .. } = &r.target
                && !self.banks.iter().any(|b| &b.name == bank)
            {
                return Err(format!("a rule names bank `{bank}`, which isn't listed"));
            }
            if let Target::Gm { program } = r.target
                && program > 127
            {
                return Err(format!("GM program {program} is past 127"));
            }
        }
        for c in self
            .mix
            .gain
            .keys()
            .chain(&self.mix.mute)
            .chain(&self.mix.solo)
        {
            if Channel::parse(c).is_none() {
                return Err(format!("`{c}` isn't a channel"));
            }
        }
        Ok(())
    }

    /// The rule for an instrument in a song: by name first, then by role.
    pub fn rule_for(&self, song: &str, ins: &Instrument) -> Option<&Rule> {
        let fits_song = |r: &&Rule| r.song.as_deref().is_none_or(|s| s == song);
        self.rules
            .iter()
            .filter(fits_song)
            .find(|r| r.instrument.as_deref() == Some(ins.name.as_str()))
            .or_else(|| {
                self.rules.iter().filter(fits_song).find(|r| {
                    r.instrument.is_none()
                        && r.role.is_some()
                        && r.role.as_deref() == ins.role.as_deref()
                })
            })
            .or_else(|| {
                self.rules
                    .iter()
                    .filter(fits_song)
                    .find(|r| r.instrument.is_none() && r.role.is_none())
            })
    }

    /// Whether any instrument of a song plays through a bank.
    pub fn uses_banks(&self, set: &SongSet, song: &str) -> bool {
        set.instruments.iter().any(|i| {
            matches!(
                self.rule_for(song, i).map(|r| &r.target),
                Some(Target::Bank { .. })
            )
        })
    }
}

/// A patch of a bank on disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Patch {
    pub name: String,
    pub bank: u16,
    pub program: u8,
    /// For SFZ: the region's key range.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keys: Option<[u8; 2]>,
}

/// A bank found on disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Found {
    pub name: String,
    pub kind: BankKind,
    pub path: PathBuf,
    pub license: Option<String>,
    pub patches: Vec<Patch>,
}

/// The APU synthesizer as a bank: one patch per voice.
pub fn apu_bank() -> Found {
    Found {
        name: "apu".into(),
        kind: BankKind::Apu,
        path: PathBuf::new(),
        license: Some("Apache-2.0 (Game Printer)".into()),
        patches: ["pulse", "triangle", "noise", "dmc"]
            .iter()
            .enumerate()
            .map(|(i, n)| Patch {
                name: (*n).into(),
                bank: 0,
                program: i as u8,
                keys: None,
            })
            .collect(),
    }
}

fn riff_chunks(b: &[u8]) -> Vec<(&[u8], &[u8])> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 8 <= b.len() {
        let id = &b[i..i + 4];
        let len = u32::from_le_bytes([b[i + 4], b[i + 5], b[i + 6], b[i + 7]]) as usize;
        let end = (i + 8 + len).min(b.len());
        out.push((id, &b[i + 8..end]));
        i = end + (len & 1);
    }
    out
}

fn cstr(b: &[u8]) -> String {
    let n = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..n]).trim().to_owned()
}

/// An SF2 file's name, copyright and presets.
pub fn read_sf2(bytes: &[u8]) -> Result<(Option<String>, Option<String>, Vec<Patch>), String> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"sfbk" {
        return Err("not an SF2 file".into());
    }
    let mut name = None;
    let mut copyright = None;
    let mut patches = Vec::new();
    for (id, body) in riff_chunks(&bytes[12..]) {
        if id != b"LIST" || body.len() < 4 {
            continue;
        }
        let kind = &body[..4];
        for (sub, data) in riff_chunks(&body[4..]) {
            match (kind, sub) {
                (b"INFO", b"INAM") => name = Some(cstr(data)),
                (b"INFO", b"ICOP") => copyright = Some(cstr(data)),
                (b"pdta", b"phdr") => {
                    // 38-byte records; the last is the terminal EOP.
                    let n = data.len() / 38;
                    for k in 0..n.saturating_sub(1) {
                        let r = &data[k * 38..k * 38 + 38];
                        patches.push(Patch {
                            name: cstr(&r[..20]),
                            program: u16::from_le_bytes([r[20], r[21]]).min(127) as u8,
                            bank: u16::from_le_bytes([r[22], r[23]]),
                            keys: None,
                        });
                    }
                }
                _ => {}
            }
        }
    }
    patches.sort_by_key(|p| (p.bank, p.program));
    Ok((name, copyright, patches))
}

/// A noise note's General MIDI drum key, for a bank's drum kit: high
/// periods (low index) are hats, the middle snares, the low end kicks.
pub fn drum_key(index: u8, len: u32, mode: u8) -> u8 {
    if mode == 1 {
        return 56; // Cowbell: the short, metallic loop
    }
    match index {
        0..=5 if len > 8 => 46, // Open hi-hat
        0..=5 => 42,            // Closed hi-hat
        6..=10 => 38,           // Acoustic snare
        _ => 36,                // Bass drum 1
    }
}

/// A key named as SFZ files may name it (`C4`, `F#-1`; MIDI 12 is C0).
fn parse_note_name(s: &str) -> Option<u8> {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let split = s.find(|c: char| c.is_ascii_digit() || c == '-')?;
    let (n, o) = s.split_at(split);
    let i = NAMES.iter().position(|x| *x == n)? as i32;
    let o: i32 = o.parse().ok()?;
    u8::try_from((o + 1) * 12 + i).ok()
}

/// An SFZ file's regions: each `<region>`'s sample and key range.
pub fn read_sfz(text: &str) -> Vec<Patch> {
    let mut out = Vec::new();
    let mut group_lo = 0u8;
    let mut group_hi = 127u8;
    let mut cur: Option<(String, u8, u8)> = None;
    let key = |v: &str| -> Option<u8> {
        v.parse()
            .ok()
            .or_else(|| parse_note_name(&v.to_ascii_uppercase()))
    };
    let push = |cur: &mut Option<(String, u8, u8)>, out: &mut Vec<Patch>| {
        if let Some((s, lo, hi)) = cur.take() {
            out.push(Patch {
                name: s,
                bank: 0,
                program: out.len().min(127) as u8,
                keys: Some([lo, hi]),
            });
        }
    };
    for raw in text.lines() {
        let line = raw.split("//").next().unwrap_or_default();
        for tok in line.split_whitespace() {
            match tok {
                "<region>" => {
                    push(&mut cur, &mut out);
                    cur = Some((String::new(), group_lo, group_hi));
                }
                "<group>" | "<global>" | "<master>" => {
                    push(&mut cur, &mut out);
                    group_lo = 0;
                    group_hi = 127;
                }
                _ => {
                    let Some((k, v)) = tok.split_once('=') else {
                        continue;
                    };
                    let target = |lo: &mut u8, hi: &mut u8| match k {
                        "lokey" => *lo = key(v).unwrap_or(*lo),
                        "hikey" => *hi = key(v).unwrap_or(*hi),
                        "key" => {
                            if let Some(x) = key(v) {
                                *lo = x;
                                *hi = x;
                            }
                        }
                        _ => {}
                    };
                    match &mut cur {
                        Some((s, lo, hi)) => {
                            if k == "sample" {
                                *s = v.to_owned();
                            }
                            target(lo, hi);
                        }
                        None => target(&mut group_lo, &mut group_hi),
                    }
                }
            }
        }
    }
    push(&mut cur, &mut out);
    out
}

/// A licence file next to a bank: `<file>.license`, `LICENSE` or
/// `LICENSE.txt` in its folder, first line.
fn license_near(path: &Path) -> Option<String> {
    let mut c = vec![PathBuf::from(format!("{}.license", path.display()))];
    if let Some(d) = path.parent() {
        c.push(d.join("LICENSE"));
        c.push(d.join("LICENSE.txt"));
    }
    c.iter().find_map(|p| {
        std::fs::read_to_string(p).ok().and_then(|t| {
            t.lines()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .map(str::to_owned)
        })
    })
}

/// Every SF2 and SFZ file in the folders (not recursing), with the APU
/// first.
pub fn discover(folders: &[PathBuf]) -> Vec<Found> {
    let mut out = vec![apu_bank()];
    for d in folders {
        let Ok(rd) = std::fs::read_dir(d) else {
            continue;
        };
        let mut files: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
        files.sort();
        for p in files {
            let ext = p
                .extension()
                .and_then(|e| e.to_str())
                .map(str::to_ascii_lowercase);
            let stem = p
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("bank")
                .to_owned();
            match ext.as_deref() {
                Some("sf2") => {
                    let Ok(bytes) = std::fs::read(&p) else {
                        continue;
                    };
                    let Ok((name, cop, patches)) = read_sf2(&bytes) else {
                        continue;
                    };
                    out.push(Found {
                        name: stem,
                        kind: BankKind::Sf2,
                        license: license_near(&p).or(cop).or(name),
                        path: p,
                        patches,
                    });
                }
                Some("sfz") => {
                    let Ok(text) = std::fs::read_to_string(&p) else {
                        continue;
                    };
                    out.push(Found {
                        name: stem,
                        kind: BankKind::Sfz,
                        license: license_near(&p),
                        path: p,
                        patches: read_sfz(&text),
                    });
                }
                _ => {}
            }
        }
    }
    out
}

/// The folders banks are looked for in: `$GP_SOUND_BANKS` (paths split
/// as the platform splits PATH), then `$GP_HOME/sound-banks`.
pub fn folders(home: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::env::var_os("GP_SOUND_BANKS")
        .map(|s| std::env::split_paths(&s).collect())
        .unwrap_or_default();
    v.push(home.join("sound-banks"));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mapping_parses_resolves_and_refuses() {
        let text = r#"
format = "gp-sound-banks"
version = "0.1.0"

[[bank]]
name = "free"
kind = "sf2"
path = "free.sf2"
license = "CC0-1.0"

[[map]]
role = "lead"
target = { kind = "bank", bank = "free", preset = "Square" }

[[map]]
instrument = "triangle-1"
target = { kind = "gm", program = 33 }

[mix]
mute = ["noise"]
gain = { pulse2 = 0.5 }
"#;
        let m = Mapping::parse(text).unwrap();
        let ins = |name: &str, role: &str| Instrument {
            name: name.into(),
            voice: crate::song::Voice::Pulse,
            role: Some(role.into()),
            volume: vec![],
            duty: vec![],
            pitch: vec![],
            sample: None,
            notes: 0,
        };
        assert!(matches!(
            m.rule_for("s", &ins("pulse-1", "lead")).unwrap().target,
            Target::Bank { .. }
        ));
        assert_eq!(
            m.rule_for("s", &ins("triangle-1", "bass")).unwrap().target,
            Target::Gm { program: 33 }
        );
        assert!(m.rule_for("s", &ins("pulse-2", "harmony")).is_none());
        assert_eq!(m.mix.gains(), [1.0, 0.5, 1.0, 0.0, 1.0]);
        assert_eq!(Mapping::parse(&m.to_text()).unwrap(), m);
        let bad = text.replace("bank = \"free\", preset", "bank = \"nope\", preset");
        assert!(Mapping::parse(&bad).unwrap_err().contains("nope"));
        let bad = text.replace("role = \"lead\"", "role = \"kazoo\"");
        assert!(Mapping::parse(&bad).is_err());
    }

    #[test]
    fn sfz_regions() {
        let r = read_sfz(
            "<group> lokey=40\n<region> sample=a.wav hikey=59 // low\n<region> sample=b.wav key=c4\n",
        );
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].keys, Some([40, 59]));
        assert_eq!(r[1].keys, Some([60, 60]));
    }
}
