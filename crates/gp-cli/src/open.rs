//! Finding a ROM by hash and opening a stored recording. Every command that
//! needs a recording's ROM, or a ROM named by registry ID, goes through
//! here, so there's one lookup and one set of error messages.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use gp_evidence::source::{NesImage, identify_nes};
use gp_evidence::{Paths, Store};

/// A ROM and its identity, found by hash in $GP_ROM_DIR or given as a path:
/// an NES image, or an arcade romset (a MAME zip).
pub(crate) struct FoundRom {
    pub bytes: Vec<u8>,
    /// The NES image's identity; `None` for a romset.
    pub image: Option<NesImage>,
    /// The romset, loaded; `None` for an NES image.
    pub set: Option<gp_arcade::Romset>,
    /// The headerless SHA-256, or the romset's identity.
    pub sha256: String,
    pub source_id: Option<String>,
}

/// The NES ROMs in $GP_ROM_DIR by headerless SHA-256, scanned once on first
/// use (by `gp_evidence::source::index_nes_roms`, which `gp-wasm-check` also
/// uses), so checking many recordings or baselines reads the folder once.
pub(crate) struct RomFolder<'a> {
    paths: &'a Paths,
    index: Option<HashMap<String, PathBuf>>,
}

impl<'a> RomFolder<'a> {
    pub fn new(paths: &'a Paths) -> Self {
        Self { paths, index: None }
    }

    /// The file whose headerless SHA-256 is `sha256`, or `None` if no file
    /// in the folder has it. A missing folder is an error, so a mistyped
    /// $GP_ROM_DIR can't make every check skip.
    pub fn find(&mut self, sha256: &str) -> Result<Option<&Path>> {
        self.scan()?;
        Ok(self
            .index
            .as_ref()
            .and_then(|index| index.get(sha256))
            .map(PathBuf::as_path))
    }

    /// Reads the folder now, if it hasn't been read, so a missing or
    /// unreadable folder fails here rather than at the first lookup.
    pub fn scan(&mut self) -> Result<()> {
        if self.index.is_none() {
            let index = gp_evidence::source::index_nes_roms(&self.paths.roms)
                .with_context(|| format!("reading {}", self.paths.roms.display()))?;
            self.index = Some(index);
        }
        Ok(())
    }

    /// The bytes of [`RomFolder::find`]'s file.
    pub fn read(&mut self, sha256: &str) -> Result<Option<Vec<u8>>> {
        let Some(path) = self.find(sha256)? else {
            return Ok(None);
        };
        let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        Ok(Some(bytes))
    }
}

/// The ROM entries of every game's roms.toml in the games checkout, which
/// name ROMs by ID (`--source`) and recordings by source. In the old tool
/// this was the repo's sources/ folder; gameprinter keeps the entries with
/// each game. Empty with no games checkout.
pub(crate) fn load_registry() -> Result<gp_evidence::registry::Registry> {
    if let Some(dir) = std::env::var_os("GP_SOURCES").filter(|v| !v.is_empty()) {
        return Ok(gp_evidence::registry::Registry::load_dir(Path::new(&dir))?);
    }
    Ok(crate::roms_cmd::sources(None)?.0)
}

/// Now in UTC as RFC 3339 with six fractional digits, as recordings and
/// their index lines keep it (the old tool's gp_metrics::run::rfc3339).
pub(crate) fn now_rfc3339() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    let day = secs % 86_400;
    let date = crate::game_new::date_of_days((secs / 86_400) as i64);
    format!(
        "{date}T{:02}:{:02}:{:02}.{:06}Z",
        day / 3600,
        day / 60 % 60,
        day % 60,
        now.subsec_micros()
    )
}

/// Identifies `bytes` and names its registry entry, if any.
fn identified(bytes: Vec<u8>) -> Result<FoundRom> {
    let (image, set, sha256) = if gp_arcade::romset::looks_like_zip(&bytes) {
        let set = gp_arcade::Romset::from_zip(&bytes)?;
        let id = set.identity.clone();
        (None, Some(set), id)
    } else {
        let image = identify_nes(&bytes)?;
        let id = image.sha256.clone();
        (Some(image), None, id)
    };
    let source_id = load_registry()?.by_sha256(&sha256).map(|k| k.id.clone());
    Ok(FoundRom {
        bytes,
        image,
        set,
        sha256,
        source_id,
    })
}

/// The ROM `gp record` runs: the file `rom` if given, otherwise the one in
/// $GP_ROM_DIR with the hash the registry gives `source`.
pub(crate) fn rom_for_source(
    source: Option<&str>,
    rom: Option<&Path>,
    paths: &Paths,
) -> Result<FoundRom> {
    if let Some(path) = rom {
        let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        return identified(bytes);
    }
    let Some(id) = source else {
        bail!("give --source or --rom");
    };
    let sha256 = load_registry()?
        .by_id(id)
        .map(|k| k.sha256.clone())
        .with_context(|| {
            format!(
                "{id} isn't a ROM entry of any game's roms.toml; `gp roms list` shows the known IDs"
            )
        })?;
    rom_by_hash(&sha256, paths)
}

/// The ROM in $GP_ROM_DIR whose headerless SHA-256 is `sha256`.
pub(crate) fn rom_by_hash(sha256: &str, paths: &Paths) -> Result<FoundRom> {
    let short = &sha256[..16.min(sha256.len())];
    if !paths.roms.is_dir() {
        bail!(
            "no ROM folder at {}, so no ROM with SHA-256 {short}… was found; add one with `gp roms add`",
            paths.roms.display()
        );
    }
    if let Some(bytes) = RomFolder::new(paths).read(sha256)? {
        return identified(bytes);
    }
    // A romset's identity isn't an NES hash: look for the zip that has it.
    let wanted = [sha256.to_owned()];
    match crate::roms::find(&wanted, &[], &paths.roms)?.remove(sha256) {
        Some(rom) => identified(rom.bytes),
        None => bail!(
            "no ROM in {} has SHA-256 {short}…; add it with `gp roms add`",
            paths.roms.display()
        ),
    }
}

/// A stored recording, found by ID prefix, with its manifest.
pub(crate) struct Opened {
    pub store: Store,
    pub entry: gp_evidence::recordings::IndexEntry,
    pub manifest: gp_evidence::manifest::RecordingManifest,
}

impl Opened {
    pub fn new(paths: &Paths, id: &str) -> Result<Self> {
        let store = Store::open(&paths.store)?;
        let entry = gp_evidence::recordings::resolve(&store, id)?;
        let manifest = gp_evidence::recordings::load_manifest(&store, &entry.id)?;
        Ok(Self {
            store,
            entry,
            manifest,
        })
    }

    /// [`Opened::new`]: the old tool's name, kept so the commands read as
    /// they did.
    pub fn open(paths: &Paths, id: &str) -> Result<Self> {
        Self::new(paths, id)
    }

    /// The recording's name and short ID, as commands print it.
    pub fn short(&self) -> String {
        format!("{} ({})", self.entry.name, &self.entry.id.to_hex()[..12])
    }

    /// The headerless SHA-256 of the ROM the recording was made from.
    /// For a model recording, which has no ROM, the model bundle's hash.
    pub fn sha256(&self) -> &str {
        match &self.manifest.source {
            gp_evidence::manifest::SourceRef::NesRom { sha256, .. } => sha256,
            gp_evidence::manifest::SourceRef::Romset { identity, .. } => identity,
            gp_evidence::manifest::SourceRef::Model { .. } => "",
        }
    }

    /// The ROM the recording was made from: the file `rom` if given, which
    /// must have the recording's hash, otherwise found by hash in
    /// $GP_ROM_DIR. Recorded as the run's `rom` input.
    pub fn rom(&self, rom: Option<&Path>, paths: &Paths) -> Result<FoundRom> {
        if let gp_evidence::manifest::SourceRef::Model { name, .. } = &self.manifest.source {
            bail!(
                "{} is a run of the model `{name}` in {}, which has no ROM; this command reads ROM recordings",
                self.short(),
                self.manifest.runtime.name
            );
        }
        let sha256 = self.sha256();
        let found = match rom {
            Some(path) => {
                let bytes =
                    fs::read(path).with_context(|| format!("reading {}", path.display()))?;
                let found = identified(bytes)?;
                if found.sha256 != sha256 {
                    bail!(
                        "{} isn't the ROM this recording was made from (SHA-256 {}…)",
                        path.display(),
                        &sha256[..16.min(sha256.len())]
                    );
                }
                found
            }
            None => rom_by_hash(sha256, paths)?,
        };
        Ok(found)
    }

    /// The romset an arcade recording was made from, loaded.
    pub fn romset(&self, rom: Option<&Path>, paths: &Paths) -> Result<gp_arcade::Romset> {
        match self.rom(rom, paths)?.set {
            Some(set) => Ok(set),
            None => bail!("{} isn't a recording of an arcade romset", self.short()),
        }
    }

    /// Refuses the recording unless it is of the machine of the game at
    /// `dir` (`arcade_game` says which), saying what the command reads in
    /// `reads`.
    pub fn of_platform(&self, dir: &Path, arcade_game: bool, reads: &str) -> Result<()> {
        if arcade_game != self.is_arcade() {
            bail!(
                "{} is {}, and {} is a recording of {}: {reads}",
                dir.display(),
                if arcade_game {
                    "an arcade game"
                } else {
                    "an NES game"
                },
                self.short(),
                if self.is_arcade() {
                    "an arcade romset"
                } else {
                    "an NES ROM"
                }
            );
        }
        Ok(())
    }

    /// Whether this is a recording of an arcade romset.
    pub fn is_arcade(&self) -> bool {
        crate::arcade_rec::is_arcade(&self.manifest)
    }

    /// The state at `frame` and where it came from: the trace, or replay.
    pub fn state(
        &self,
        frame: u64,
        rom: Option<&Path>,
        paths: &Paths,
    ) -> Result<(Vec<u8>, &'static str)> {
        if let Some(state) = gp_nes::recording::trace_state(&self.store, &self.manifest, frame)? {
            return Ok((state, "trace"));
        }
        if crate::arcade_rec::is_arcade(&self.manifest) {
            let set = self.romset(rom, paths)?;
            return Ok((
                crate::arcade_rec::replayed_state(&self.store, &self.manifest, &set, frame)?,
                "replay",
            ));
        }
        let rom = self.rom(rom, paths)?.bytes;
        Ok((
            gp_nes::recording::replayed_state(&self.store, &self.manifest, &rom, frame)?,
            "replay",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths_with_roms(dir: &Path) -> Paths {
        Paths {
            home: dir.to_path_buf(),
            store: dir.join("store"),
            roms: dir.join("roms"),
        }
    }

    #[test]
    fn finds_a_rom_by_hash_and_reads_the_folder_once() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_with_roms(dir.path());
        fs::create_dir_all(paths.roms.join("nested")).unwrap();
        let rom = gp_nes::testrom::build();
        let sha256 = identify_nes(&rom).unwrap().sha256;
        fs::write(paths.roms.join("nested/test.nes"), &rom).unwrap();
        fs::write(paths.roms.join("notes.txt"), "not a ROM").unwrap();

        let mut folder = RomFolder::new(&paths);
        assert_eq!(
            folder.find(&sha256).unwrap(),
            Some(paths.roms.join("nested/test.nes").as_path())
        );
        assert_eq!(folder.find(&"0".repeat(64)).unwrap(), None);
        // A file added after the first lookup isn't seen: one scan per folder.
        fs::write(paths.roms.join("late.nes"), synthetic()).unwrap();
        let late = identify_nes(&synthetic()).unwrap().sha256;
        assert_eq!(folder.find(&late).unwrap(), None);
        assert!(RomFolder::new(&paths).find(&late).unwrap().is_some());

        assert_eq!(rom_by_hash(&sha256, &paths).unwrap().bytes, rom);
        let missing = rom_by_hash(&"0".repeat(64), &paths).err().unwrap();
        assert!(missing.to_string().contains("no ROM in"), "{missing}");
    }

    #[test]
    fn a_missing_rom_folder_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_with_roms(dir.path());
        assert!(RomFolder::new(&paths).find(&"0".repeat(64)).is_err());
        let e = rom_by_hash(&"0".repeat(64), &paths).err().unwrap();
        assert!(e.to_string().contains("no ROM folder at"), "{e}");
    }

    /// A one-bank NROM image different from the test ROM.
    fn synthetic() -> Vec<u8> {
        let mut rom = b"NES\x1a\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00".to_vec();
        rom.extend(std::iter::repeat_n(0xea, 16 * 1024));
        rom
    }
}
