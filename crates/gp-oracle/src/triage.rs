//! Triage: what each divergence class is, as data.
//!
//! `oracle/triage.toml` in the repo holds records of the classes diffs have
//! reported ([`crate::diff`]): a field name, `frame-shift`, a boundary
//! class, `lag` or `events.<kind>`. Each says whether the class is a bug
//! in our fork of `tetanes-core`, a difference in Mesen, or an accepted
//! difference in how the two are configured, with a note giving the
//! evidence, and where it applies: which sources (registry IDs) and which
//! frames. A divergence outside every record's scope is untriaged, so a
//! later difference in the same field isn't accepted in advance. Classes
//! are about the emulators, not any game, so the file is committed;
//! `gp oracle triage` writes it, and `gp oracle golden` refuses a
//! recording whose diff has a divergence that isn't triaged or is a fork
//! bug.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The file's path relative to the repo root.
pub const FILE: &str = "oracle/triage.toml";
pub const SCHEMA: u32 = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Verdict {
    /// Our fork behaves differently from the hardware; it needs a fix.
    ForkBug,
    /// Mesen behaves differently from the hardware, or the two runtimes
    /// show the same behaviour differently.
    MesenDifference,
    /// The two run with different settings, on purpose or because one
    /// can't be configured like the other.
    AcceptedConfiguration,
}

impl Verdict {
    pub const ALL: [Self; 3] = [
        Self::ForkBug,
        Self::MesenDifference,
        Self::AcceptedConfiguration,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::ForkBug => "fork-bug",
            Self::MesenDifference => "mesen-difference",
            Self::AcceptedConfiguration => "accepted-configuration",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.name() == name)
    }

    /// Whether a recording with this class can still be golden.
    pub fn accepted(self) -> bool {
        self != Self::ForkBug
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub class: String,
    pub verdict: Verdict,
    /// The sources (registry IDs, such as `smb1-ntsc-prg0`) it applies to;
    /// empty for any.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<String>,
    /// The first and last frame it applies to; absent for any frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frames: Option<[u64; 2]>,
    /// The evidence: what differs, why, and what settled it.
    pub note: String,
    /// When it was triaged, `YYYY-MM-DD`.
    pub date: String,
}

impl Record {
    /// Whether the record covers a divergence of its class at `frame` of a
    /// recording of `source`.
    pub fn covers(&self, source: Option<&str>, frame: u64) -> bool {
        let source_ok =
            self.sources.is_empty() || source.is_some_and(|s| self.sources.iter().any(|x| x == s));
        let frame_ok = self.frames.is_none_or(|[a, b]| (a..=b).contains(&frame));
        source_ok && frame_ok
    }

    fn same_scope(&self, other: &Self) -> bool {
        self.class == other.class && self.sources == other.sources && self.frames == other.frames
    }

    /// The scope as words, for messages.
    pub fn scope(&self) -> String {
        let sources = if self.sources.is_empty() {
            "any source".to_owned()
        } else {
            self.sources.join(", ")
        };
        match self.frames {
            Some([a, b]) if a == b => format!("{sources}, frame {a}"),
            Some([a, b]) => format!("{sources}, frames {a} to {b}"),
            None => format!("{sources}, any frame"),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Triage {
    pub schema: u32,
    #[serde(default, rename = "class")]
    pub records: Vec<Record>,
}

#[derive(Debug, thiserror::Error)]
pub enum TriageError {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{path}: {message}")]
    Parse { path: PathBuf, message: String },
    #[error(
        "{0} doesn't exist; give the triage file with --triage (or --file), or create it with `gp oracle triage --file`"
    )]
    Missing(PathBuf),
}

const PREAMBLE: &str = "# Triage of the divergence classes `gp oracle diff` reports (docs/oracle.md).\n# Written by `gp oracle triage`; one [[class]] per class and scope, sorted.\n# verdict is fork-bug, mesen-difference or accepted-configuration. sources
# (registry IDs) and frames ([first, last]) limit where a record applies;
# without them it applies to every source or frame.\n\n";

impl Triage {
    /// Reads the file, which must exist.
    pub fn load(path: &Path) -> Result<Self, TriageError> {
        Self::read(path, false)
    }

    /// Reads the file, or starts an empty one if it doesn't exist.
    pub fn load_or_new(path: &Path) -> Result<Self, TriageError> {
        Self::read(path, true)
    }

    fn read(path: &Path, or_new: bool) -> Result<Self, TriageError> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return if or_new {
                    Ok(Self::default())
                } else {
                    Err(TriageError::Missing(path.into()))
                };
            }
            Err(source) => {
                return Err(TriageError::Io {
                    path: path.into(),
                    source,
                });
            }
        };
        Self::parse(&text).map_err(|message| TriageError::Parse {
            path: path.into(),
            message,
        })
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let t: Self = toml::from_str(text).map_err(|e| e.to_string())?;
        if t.schema != SCHEMA {
            return Err(format!("triage schema {} isn't {SCHEMA}", t.schema));
        }
        for (i, r) in t.records.iter().enumerate() {
            if t.records[..i].iter().any(|o| o.same_scope(r)) {
                return Err(format!(
                    "class {:?} is triaged twice for {}",
                    r.class,
                    r.scope()
                ));
            }
            if let Some([a, b]) = r.frames
                && a > b
            {
                return Err(format!(
                    "class {:?}: frames {a} to {b} run backwards",
                    r.class
                ));
            }
        }
        Ok(t)
    }

    pub fn to_text(&self) -> String {
        let mut sorted = self.clone();
        sorted
            .records
            .sort_by(|a, b| (&a.class, a.frames).cmp(&(&b.class, b.frames)));
        format!(
            "{PREAMBLE}{}",
            toml::to_string(&sorted).expect("triage serializes")
        )
    }

    pub fn save(&self, path: &Path) -> Result<(), TriageError> {
        let io = |source| TriageError::Io {
            path: path.into(),
            source,
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(io)?;
        }
        std::fs::write(path, self.to_text()).map_err(io)
    }

    /// The records of a class.
    pub fn of<'t>(&'t self, class: &'t str) -> impl Iterator<Item = &'t Record> + 't {
        self.records.iter().filter(move |r| r.class == class)
    }

    /// Adds a record, or replaces the one with the same class and scope.
    pub fn set(&mut self, record: Record) {
        match self.records.iter_mut().find(|r| r.same_scope(&record)) {
            Some(r) => *r = record,
            None => self.records.push(record),
        }
    }

    /// Checks each class's divergences, given as the frames they were
    /// found at, against the records, for a recording of `source`. A class
    /// with a frame no record covers is untriaged; otherwise one with a
    /// frame covered by a fork bug is a fork bug; otherwise it's accepted.
    pub fn check<'a>(
        &self,
        source: Option<&str>,
        classes: impl IntoIterator<Item = (&'a str, &'a [u64])>,
    ) -> Check {
        let mut out = Check::default();
        for (class, frames) in classes {
            let covering = |f: u64| self.of(class).find(|r| r.covers(source, f));
            let uncovered: Vec<u64> = frames
                .iter()
                .copied()
                .filter(|&f| covering(f).is_none())
                .collect();
            if let Some(&f) = uncovered.first() {
                out.untriaged.push(class.to_owned());
                out.uncovered.push(format!(
                    "{class}: {} divergences outside every record's scope, first at frame {f}",
                    uncovered.len()
                ));
            } else if frames
                .iter()
                .any(|&f| covering(f).is_some_and(|r| !r.verdict.accepted()))
            {
                out.fork_bugs.push(class.to_owned());
            } else {
                out.accepted.push(class.to_owned());
            }
        }
        out
    }

    /// The verdict a class's records give, for display: one name, or
    /// `mixed` when records of different verdicts cover it.
    pub fn verdict_of(&self, class: &str) -> Option<String> {
        let mut v: Vec<&str> = self.of(class).map(|r| r.verdict.name()).collect();
        v.sort_unstable();
        v.dedup();
        match v.as_slice() {
            [] => None,
            [one] => Some((*one).to_owned()),
            _ => Some("mixed".into()),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Check {
    pub accepted: Vec<String>,
    pub untriaged: Vec<String>,
    pub fork_bugs: Vec<String>,
    /// For each untriaged class, what lies outside the records' scopes.
    pub uncovered: Vec<String>,
}

impl Check {
    pub fn passed(&self) -> bool {
        self.untriaged.is_empty() && self.fork_bugs.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(class: &str, verdict: Verdict) -> Record {
        Record {
            class: class.into(),
            verdict,
            sources: Vec::new(),
            frames: None,
            note: "because".into(),
            date: "2026-10-27".into(),
        }
    }

    #[test]
    fn round_trips_sorted_and_checks_classes() {
        let mut t = Triage::default();
        t.set(record("ram", Verdict::ForkBug));
        t.set(record("cpu.p", Verdict::MesenDifference));
        t.set(record("ram", Verdict::AcceptedConfiguration));
        let text = t.to_text();
        assert!(text.find("cpu.p").unwrap() < text.find("\"ram\"").unwrap());
        let back = Triage::parse(&text).unwrap();
        assert_eq!(back.records.len(), 2);
        let c = back.check(
            None,
            [("ram", &[3][..]), ("cpu.p", &[1][..]), ("oam", &[4][..])],
        );
        assert_eq!(c.untriaged, vec!["oam"]);
        assert_eq!(c.accepted, vec!["ram", "cpu.p"]);
        assert!(!c.passed());
    }

    #[test]
    fn a_scope_limits_what_a_record_accepts() {
        let mut t = Triage::default();
        t.set(Record {
            sources: vec!["smb1-ntsc-prg0".into()],
            frames: Some([0, 0]),
            ..record("ppu.dot", Verdict::AcceptedConfiguration)
        });
        t.set(Record {
            sources: vec!["smb1-ntsc-prg0".into()],
            frames: Some([2, 2]),
            ..record("cpu.a", Verdict::ForkBug)
        });
        let smb = Some("smb1-ntsc-prg0");
        assert!(t.check(smb, [("ppu.dot", &[0][..])]).passed());
        // Another frame, or another game, is outside the scope.
        let c = t.check(smb, [("ppu.dot", &[0, 500][..])]);
        assert_eq!(c.untriaged, vec!["ppu.dot"]);
        assert!(
            c.uncovered[0].contains("first at frame 500"),
            "{:?}",
            c.uncovered
        );
        assert_eq!(
            t.check(Some("other"), [("ppu.dot", &[0][..])]).untriaged,
            vec!["ppu.dot"]
        );
        assert_eq!(
            t.check(None, [("ppu.dot", &[0][..])]).untriaged,
            vec!["ppu.dot"]
        );
        // The fork bug is known at frame 2 only.
        assert_eq!(t.check(smb, [("cpu.a", &[2][..])]).fork_bugs, vec!["cpu.a"]);
        assert_eq!(
            t.check(smb, [("cpu.a", &[2, 9][..])]).untriaged,
            vec!["cpu.a"]
        );
        // Two scopes of one class are two records; the same scope replaces.
        t.set(Record {
            frames: Some([5, 9]),
            ..record("ppu.dot", Verdict::MesenDifference)
        });
        assert_eq!(t.of("ppu.dot").count(), 2);
        assert_eq!(t.verdict_of("ppu.dot").as_deref(), Some("mixed"));
        let back = Triage::parse(&t.to_text()).unwrap();
        assert_eq!(back.records.len(), t.records.len());
    }

    #[test]
    fn refuses_a_scope_twice_and_a_missing_file() {
        let text = "schema = 0\n[[class]]\nclass = \"a\"\nverdict = \"fork-bug\"\nnote = \"\"\ndate = \"\"\n[[class]]\nclass = \"a\"\nverdict = \"fork-bug\"\nnote = \"\"\ndate = \"\"\n";
        assert!(Triage::parse(text).unwrap_err().contains("twice"));
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("triage.toml");
        assert!(matches!(
            Triage::load(&missing),
            Err(TriageError::Missing(_))
        ));
        assert_eq!(Triage::load_or_new(&missing).unwrap(), Triage::default());
    }
}
