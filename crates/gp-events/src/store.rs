//! The store of runs' levels: each run's L0 and L1, and its L2 when it was
//! asked for, kept by what makes it, so a recipe run once isn't run again.
//!
//! A run is keyed by the collector's version, the ROM's SHA-256 and the
//! hash of its input from power-on (an input log's text); L2 also by the
//! addresses watched, and a window's counts ([`crate::profile`]) by the
//! window and the profile's version, and a window cut where a byte changes
//! by the window, the byte and the segments' version. Files are JSON (L0
//! and L1) and NDJSON (L2, a frame a line) under one folder, by default
//! `$GP_HOME/work/events`. Everything in it can be made again from the ROM
//! and the recipe, so a missing or unreadable file is run again, never an
//! error.

use std::fs;
use std::path::{Path, PathBuf};

use crate::COLLECTOR_VERSION;
use crate::frames::FrameEvents;
use crate::levels::Levels;
use crate::profile::{Profile, Segment};

/// A store folder.
#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
}

impl Store {
    pub fn at(root: impl Into<PathBuf>) -> Store {
        Store { root: root.into() }
    }

    fn dir(&self, rom_sha256: &str) -> PathBuf {
        self.root
            .join(format!("v{COLLECTOR_VERSION}"))
            .join(&rom_sha256[..rom_sha256.len().min(16)])
    }

    fn levels_path(&self, rom: &str, input: &str) -> PathBuf {
        self.dir(rom).join(format!("{input}.levels.json"))
    }

    fn frames_path(&self, rom: &str, input: &str, watch: &str) -> PathBuf {
        self.dir(rom).join(format!("{input}.{watch}.frames.ndjson"))
    }

    /// A run's L0 and L1, if kept.
    pub fn levels(&self, rom: &str, input: &str) -> Option<Levels> {
        let text = fs::read_to_string(self.levels_path(rom, input)).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn put_levels(&self, rom: &str, input: &str, l: &Levels) -> std::io::Result<()> {
        write(&self.levels_path(rom, input), &serde_json::to_vec(l)?)
    }

    fn profile_path(&self, rom: &str, input: &str, first: u64, last: u64) -> PathBuf {
        self.dir(rom).join(format!(
            "{input}.p{}-{first}-{last}.profile.json",
            crate::profile::PROFILE_VERSION
        ))
    }

    /// A window's counts (frames `first` to `last` of a run), if kept.
    pub fn profile(&self, rom: &str, input: &str, first: u64, last: u64) -> Option<Profile> {
        let text = fs::read_to_string(self.profile_path(rom, input, first, last)).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn put_profile(
        &self,
        rom: &str,
        input: &str,
        first: u64,
        last: u64,
        p: &Profile,
    ) -> std::io::Result<()> {
        write(
            &self.profile_path(rom, input, first, last),
            &serde_json::to_vec(p)?,
        )
    }

    fn segments_path(&self, rom: &str, input: &str, first: u64, last: u64, byte: u16) -> PathBuf {
        self.dir(rom).join(format!(
            "{input}.s{}-{first}-{last}-{byte:04x}.segments.json",
            crate::profile::SEGMENTS_VERSION
        ))
    }

    /// A window's counts cut where a byte changes
    /// ([`crate::profile::run_segments`]), if kept.
    pub fn segments(
        &self,
        rom: &str,
        input: &str,
        first: u64,
        last: u64,
        byte: u16,
    ) -> Option<Vec<Segment>> {
        let text = fs::read_to_string(self.segments_path(rom, input, first, last, byte)).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn put_segments(
        &self,
        rom: &str,
        input: &str,
        first: u64,
        last: u64,
        byte: u16,
        s: &[Segment],
    ) -> std::io::Result<()> {
        write(
            &self.segments_path(rom, input, first, last, byte),
            &serde_json::to_vec(s)?,
        )
    }

    /// Whether a run's L2 with this watch set is kept.
    pub fn has_frames(&self, rom: &str, input: &str, watch: &str) -> bool {
        self.frames_path(rom, input, watch).is_file()
    }

    /// The path of a run's L2, a frame a line.
    pub fn frames_file(&self, rom: &str, input: &str, watch: &str) -> PathBuf {
        self.frames_path(rom, input, watch)
    }

    pub fn put_frames(
        &self,
        rom: &str,
        input: &str,
        watch: &str,
        f: &[FrameEvents],
    ) -> std::io::Result<()> {
        let mut text = Vec::new();
        for e in f {
            serde_json::to_writer(&mut text, e)?;
            text.push(b'\n');
        }
        write(&self.frames_path(rom, input, watch), &text)
    }
}

/// Writes through a temporary file, so a reader never sees half a file.
fn write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(d) = path.parent() {
        fs::create_dir_all(d)?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(tmp, path)
}
