//! `gp movie`: importing FCEUX `.fm2` and BizHawk `.bk2` movies as input logs
//! (docs/movies.md).
//!
//! An import keeps only the buttons. Every frame of the log it writes is marked
//! polled; `gp record` replaces those flags with what the game read, so lag
//! frames come from our runtime and not from the movie's emulator. A movie is
//! accepted only if the ROM hash it names is the target source's, and refused
//! (exit 2) with the source it does name otherwise.

use std::collections::HashMap;
use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use base64::Engine as _;
use clap::Subcommand;
use md5::Digest as _;
use serde_json::json;

use gp_evidence::Hash;
use gp_evidence::input_log::{InputFrame, InputLog, button};
use gp_evidence::registry::{KnownSource, Registry};
use gp_evidence::source::nes_banks;

use crate::Outcome;
use crate::recordings::IndexLabel;

/// The most frames an import reads: over nine hours at 60 frames a second,
/// longer than any published NES run, and small enough that a crafted file
/// can't make the import allocate gigabytes.
pub const MAX_FRAMES: usize = 2_000_000;

#[derive(Subcommand)]
pub enum MovieCmd {
    /// Turn an FCEUX `.fm2` or BizHawk `.bk2` movie into an input log. The
    /// movie must name the target ROM by hash; one made on another revision
    /// is refused with exit 2 and the revision it names. It imports buttons
    /// only, and `gp record` (or `--record`) derives the lag frames.
    Import {
        /// The movie file, `.fm2` or `.bk2`.
        file: PathBuf,
        /// The ROM entry (its ID in a game's roms.toml) the movie must be
        /// made on.
        #[arg(long, conflicts_with = "rom", required_unless_present = "rom")]
        source: Option<String>,
        /// A ROM file the movie must be made on, instead of a registered source.
        #[arg(long)]
        rom: Option<PathBuf>,
        /// Write the input log here (default: stdout, unless `--record`).
        #[arg(long)]
        out: Option<PathBuf>,
        /// Also record the log, as `gp record` would, with the recording's
        /// provenance set to a TAS import.
        #[arg(long)]
        record: bool,
        /// The movie's publication page (e.g. its TASVideos URL), kept in the
        /// recording's provenance. Only with `--record`; without it the
        /// provenance's URL is empty.
        #[arg(long, requires = "record")]
        url: Option<String>,
        /// A short name for the recording (default: the movie file's name
        /// without its extension). Only with `--record`.
        #[arg(long, requires = "record")]
        name: Option<String>,
        /// Move every input later by this many frames (empty frames are added
        /// at the start), or earlier with a negative count (the first frames
        /// are dropped), to match how many frames the game took to boot in
        /// the movie's emulator. Found per movie with docs/desync.md; the
        /// value used goes in the recording's notes.
        #[arg(long, default_value_t = 0, allow_negative_numbers = true)]
        shift: i64,
        /// Power-on RAM for `--record`: zeros, ones, or fceux, FCEUX's default
        /// fill (00 00 00 00 FF FF FF FF repeated), which movies made in
        /// FCEUX were made with.
        #[arg(long, default_value = "zeros", value_parser = ["zeros", "ones", "fceux"], requires = "record")]
        ram_init: String,
    },
    /// Print the hashes movies name a ROM by (FCEUX's MD5, BizHawk's SHA-1),
    /// for a game's roms.toml, and check them against the
    /// registry.
    Hashes {
        /// The ROM file.
        rom: PathBuf,
    },
    /// Find where a recorded movie stops fitting the game: input changes the
    /// game never read (made on lag frames and gone by the next read) and
    /// changes inside lag runs. A TAS wastes no input, so the first one marks
    /// where the replay left the author's path (docs/movies.md).
    Diagnose {
        /// The recording's ID, or a unique prefix of at least 4 characters.
        id: String,
    },
}

/// `gp record`, which `--record` calls with the log it wrote.
pub type RecordFn<'a> = dyn FnMut(
        Option<&str>,
        Option<&Path>,
        &Path,
        IndexLabel<'_>,
        gp_evidence::manifest::Provenance,
        &str,
    ) -> Result<Outcome>
    + 'a;

pub fn run(cmd: &MovieCmd, registry: &Registry, record: &mut RecordFn<'_>) -> Result<Outcome> {
    match cmd {
        MovieCmd::Hashes { rom } => hashes(rom, registry),
        MovieCmd::Diagnose { .. } => bail!("gp movie diagnose reads the store; main runs it"),
        MovieCmd::Import {
            file,
            source,
            rom,
            out,
            record: also_record,
            url,
            name,
            shift,
            ram_init,
        } => {
            let target = match rom {
                Some(path) => {
                    let bytes =
                        fs::read(path).with_context(|| format!("reading {}", path.display()))?;
                    Target::from_rom(&bytes, registry)?
                }
                None => Target::from_source(
                    source.as_deref().expect("clap requires --source or --rom"),
                    registry,
                )?,
            };
            let bytes = fs::read(file).with_context(|| format!("reading {}", file.display()))?;
            let mut movie = parse_movie(file, &bytes)?;
            check_identity(&movie, &target, registry)?;
            apply_shift(&mut movie.log, *shift)?;
            let text = movie.log.to_text();
            let frames = movie.log.frames.len();
            let mut summary = json!({
                "format": movie.format,
                "frames": frames,
                "controllers": movie.log.controllers,
                "source": target.label(),
                "rom_hash": movie.rom_hash.as_ref().map(|h| h.to_string()),
                "emulator": movie.emulator,
                "rerecords": movie.rerecords,
                "movie_blake3": Hash::of(&bytes).to_string(),
                "shift": shift,
            });
            let mut lines = vec![format!(
                "imported {frames} frames ({} controller{}) from {} movie {}, made on {}",
                movie.log.controllers,
                if movie.log.controllers == 1 { "" } else { "s" },
                movie.format,
                file.display(),
                target.label()
            )];

            // Where the log goes: --out, a scratch file for --record, or stdout.
            let scratch;
            let log_path = match (out, also_record) {
                (Some(path), _) => Some(path.clone()),
                (None, true) => {
                    scratch = tempfile::tempdir()?;
                    Some(scratch.path().join("movie.input"))
                }
                (None, false) => None,
            };
            if let Some(path) = &log_path {
                fs::write(path, &text).with_context(|| format!("writing {}", path.display()))?;
                if out.is_some() {
                    summary["out"] = json!(path);
                    lines.push(format!("wrote {}", path.display()));
                }
            }
            if !*also_record {
                if log_path.is_none() {
                    // The log is the output; with --json it's the `log` field.
                    summary["log"] = json!(text);
                    return Ok(Outcome::ok(summary, text.trim_end().to_owned()));
                }
                lines.push(format!(
                    "record it with `gp record --{} --input {}`",
                    match &target.source {
                        Some(id) if rom.is_none() => format!("source {id}"),
                        _ => "rom ROM".into(),
                    },
                    log_path.as_ref().expect("written").display()
                ));
                return Ok(Outcome::ok(summary, lines.join("\n")));
            }

            let default_name = file
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "movie".into());
            let notes = format!(
                "imported from {} movie {} (blake3 {}){}",
                movie.format,
                file.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                Hash::of(&bytes),
                movie
                    .emulator
                    .as_deref()
                    .map(|e| format!(", made with {e}"))
                    .unwrap_or_default()
            ) + &if *shift != 0 {
                format!(", inputs shifted {shift:+} frames")
            } else {
                String::new()
            };
            let recorded = record(
                rom.is_none()
                    .then_some(source.as_deref().unwrap_or_default()),
                rom.as_deref(),
                log_path.as_deref().expect("written for --record"),
                IndexLabel {
                    name: Some(name.as_deref().unwrap_or(&default_name)),
                    notes: Some(&notes),
                },
                gp_evidence::manifest::Provenance::TasImport {
                    url: url.clone().unwrap_or_default(),
                },
                ram_init,
            )?;
            summary["recording"] = recorded.json;
            lines.push(recorded.text);
            Ok(Outcome {
                json: summary,
                text: lines.join("\n"),
                exit: recorded.exit,
            })
        }
    }
}

/// Moves every input `shift` frames later (empty, polled frames added at the
/// start) or earlier (the first frames dropped).
fn apply_shift(log: &mut InputLog, shift: i64) -> Result<()> {
    let k = usize::try_from(shift.unsigned_abs()).context("shift too large")?;
    if shift >= 0 {
        if log.frames.len() + k > MAX_FRAMES {
            bail!("the shifted movie is longer than {MAX_FRAMES} frames");
        }
        let empty = InputFrame {
            polled: true,
            pads: [0; 4],
            reset: false,
        };
        log.frames.splice(0..0, std::iter::repeat_n(empty, k));
    } else {
        if k >= log.frames.len() {
            bail!("a shift of {shift} drops every frame of the movie");
        }
        if log.frames[..k].iter().any(|f| f.pads != [0; 4] || f.reset) {
            bail!("a shift of {shift} would drop frames that hold input");
        }
        log.frames.drain(..k);
        // A reset on what is now the first entry would come before frame 1
        // with nothing to reset from; refuse it rather than move it.
        if log.frames[0].reset {
            bail!("a shift of {shift} would put a reset on the first frame");
        }
    }
    Ok(())
}

/// A ROM hash a movie names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RomHash {
    /// FCEUX `romChecksum`: MD5 of PRG and CHR.
    Md5(String),
    /// BizHawk `SHA1`: of the whole file with its header, or of PRG and
    /// CHR, depending on BizHawk's game database.
    Sha1(String),
}

impl std::fmt::Display for RomHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RomHash::Md5(h) => write!(f, "MD5 {h}"),
            RomHash::Sha1(h) => write!(f, "SHA-1 {h}"),
        }
    }
}

/// A parsed movie: buttons and what it says about where it was made.
#[derive(Debug)]
pub struct Movie {
    pub format: &'static str,
    pub log: InputLog,
    pub rom_hash: Option<RomHash>,
    /// The ROM's file name as the movie records it.
    pub rom_name: Option<String>,
    pub emulator: Option<String>,
    pub rerecords: Option<u64>,
}

/// The ROM a movie must name, as the hashes movies use.
struct Target {
    source: Option<String>,
    md5: Vec<String>,
    sha1: Vec<String>,
}

impl Target {
    fn from_source(id: &str, registry: &Registry) -> Result<Self> {
        let known = registry.by_id(id).with_context(|| {
            format!(
                "{id} isn't a ROM entry of any game's roms.toml; `gp roms list` shows the known IDs"
            )
        })?;
        if known.md5.is_none() && known.sha1.is_none() && known.ines_sha1.is_none() {
            bail!(
                "{id} has no movie hashes in the registry; add md5, sha1 and ines_sha1 from `gp movie hashes ROM` (docs/movies.md)"
            );
        }
        Ok(Self {
            source: Some(id.to_owned()),
            md5: known.md5.iter().cloned().collect(),
            sha1: [&known.sha1, &known.ines_sha1]
                .into_iter()
                .flatten()
                .cloned()
                .collect(),
        })
    }

    fn from_rom(bytes: &[u8], registry: &Registry) -> Result<Self> {
        let h = RomHashes::of(bytes)?;
        Ok(Self {
            source: registry.by_sha256(&h.sha256).map(|k| k.id.clone()),
            md5: vec![h.md5],
            sha1: vec![h.sha1, h.ines_sha1, h.file_sha1],
        })
    }

    fn label(&self) -> String {
        self.source
            .clone()
            .unwrap_or_else(|| "the given ROM (not in the registry)".into())
    }

    fn accepts(&self, hash: &RomHash) -> bool {
        match hash {
            RomHash::Md5(h) => self.md5.contains(h),
            RomHash::Sha1(h) => self.sha1.contains(h),
        }
    }
}

/// The hashes of one ROM file that movies or the registry use.
pub(crate) struct RomHashes {
    pub(crate) sha256: String,
    pub(crate) md5: String,
    pub(crate) sha1: String,
    pub(crate) ines_sha1: String,
    pub(crate) file_sha1: String,
}

impl RomHashes {
    pub(crate) fn of(bytes: &[u8]) -> Result<Self> {
        let (image, prg, chr) = nes_banks(bytes)?;
        let md5 = hex::encode(
            md5::Md5::new()
                .chain_update(prg)
                .chain_update(chr)
                .finalize(),
        );
        let sha1 = hex::encode(
            sha1::Sha1::new()
                .chain_update(prg)
                .chain_update(chr)
                .finalize(),
        );
        // A plain iNES 1.0 header: the file's first seven bytes with the
        // trainer bit cleared, the mapper's high nibble, then zeros.
        let mut header = [0u8; 16];
        header[..7].copy_from_slice(&bytes[..7]);
        header[6] &= !0x04;
        header[7] = bytes[7] & 0xf0;
        let ines_sha1 = hex::encode(
            sha1::Sha1::new()
                .chain_update(header)
                .chain_update(prg)
                .chain_update(chr)
                .finalize(),
        );
        let file_sha1 = hex::encode(sha1::Sha1::digest(bytes));
        Ok(Self {
            sha256: image.sha256,
            md5,
            sha1,
            ines_sha1,
            file_sha1,
        })
    }
}

fn hashes(rom: &Path, registry: &Registry) -> Result<Outcome> {
    let bytes = fs::read(rom).with_context(|| format!("reading {}", rom.display()))?;
    let h = RomHashes::of(&bytes)?;
    let known = registry.by_sha256(&h.sha256);
    let mut mismatches = Vec::new();
    if let Some(k) = known {
        for (field, registered, actual) in [
            ("md5", &k.md5, &h.md5),
            ("sha1", &k.sha1, &h.sha1),
            ("ines_sha1", &k.ines_sha1, &h.ines_sha1),
        ] {
            if let Some(r) = registered
                && r != actual
            {
                mismatches.push(format!(
                    "{field}: registry has {r}, this ROM gives {actual}"
                ));
            }
        }
    }
    let mut text = format!(
        "md5 = \"{}\"\nsha1 = \"{}\"\nines_sha1 = \"{}\"\n(file SHA-1 {})",
        h.md5, h.sha1, h.ines_sha1, h.file_sha1
    );
    match known {
        Some(k) if mismatches.is_empty() => {
            let missing = [&k.md5, &k.sha1, &k.ines_sha1].iter().any(|f| f.is_none());
            text.push_str(&format!(
                "\n{}: {}",
                k.id,
                if missing {
                    "registered, some movie hashes not yet in the registry"
                } else {
                    "matches the registry"
                }
            ));
        }
        Some(k) => text.push_str(&format!("\n{}: {}", k.id, mismatches.join("; "))),
        None => text.push_str("\nnot in the source registry"),
    }
    Ok(Outcome {
        json: json!({
            "source": known.map(|k| &k.id),
            "sha256": h.sha256,
            "md5": h.md5,
            "sha1": h.sha1,
            "ines_sha1": h.ines_sha1,
            "file_sha1": h.file_sha1,
            "mismatches": mismatches,
        }),
        text,
        exit: if mismatches.is_empty() { 0 } else { 1 },
    })
}

/// Refuses a movie that doesn't name the target ROM, naming what it was made on.
fn check_identity(movie: &Movie, target: &Target, registry: &Registry) -> Result<()> {
    let Some(hash) = &movie.rom_hash else {
        bail!(
            "the movie doesn't name its ROM by hash, so it can't be checked against {}",
            target.label()
        );
    };
    if target.accepts(hash) {
        return Ok(());
    }
    let named = registry.sources.iter().find(|k| names(k, hash));
    let file = movie
        .rom_name
        .as_deref()
        .map(|n| format!(", ROM file {n:?}"))
        .unwrap_or_default();
    match named {
        Some(k) => bail!(
            "the movie was made on {} ({} {}{}), not {}; recordings of one revision say nothing about another",
            k.id,
            k.title,
            k.revision.as_deref().unwrap_or(&k.region),
            file,
            target.label()
        ),
        None => bail!(
            "the movie was made on a ROM that isn't in the source registry ({hash}{file}), not {}",
            target.label()
        ),
    }
}

fn names(k: &KnownSource, hash: &RomHash) -> bool {
    match hash {
        RomHash::Md5(h) => k.md5.as_ref() == Some(h),
        RomHash::Sha1(h) => k.sha1.as_ref() == Some(h) || k.ines_sha1.as_ref() == Some(h),
    }
}

pub fn parse_movie(path: &Path, bytes: &[u8]) -> Result<Movie> {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase());
    match ext.as_deref() {
        Some("fm2") => {
            let text = std::str::from_utf8(bytes).context("the .fm2 file isn't UTF-8 text")?;
            parse_fm2(text)
        }
        Some("bk2") => parse_bk2(bytes),
        _ => bail!(
            "{} isn't a .fm2 or .bk2 movie (by its extension)",
            path.display()
        ),
    }
}

const FM2_ORDER: [u8; 8] = [
    button::RIGHT,
    button::LEFT,
    button::DOWN,
    button::UP,
    button::START,
    button::SELECT,
    button::B,
    button::A,
];

/// Parses FCEUX's text format (fceux.com/web/FM2.html): `key value` header
/// lines, then one `|commands|port0|port1|port2|` line per frame.
pub fn parse_fm2(text: &str) -> Result<Movie> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = text.lines().enumerate().peekable();

    // The header runs up to the first frame line.
    let mut header: HashMap<&str, &str> = HashMap::new();
    while let Some((_, raw)) = lines.next_if(|(_, l)| !l.starts_with('|')) {
        let line = raw.trim_end_matches('\r').trim();
        match line.split_once(' ') {
            Some((key, value)) => header.entry(key).or_insert(value.trim()),
            None if !line.is_empty() => header.entry(line).or_insert(""),
            None => continue,
        };
    }

    match header.get("version") {
        Some(&"3") => {}
        Some(v) => bail!(".fm2 version {v} isn't supported (FCEUX writes 3)"),
        None => bail!("not an .fm2 movie: no `version` line"),
    }
    let flag = |k: &str| header.get(k).is_some_and(|v| *v != "0" && !v.is_empty());
    if flag("binary") {
        bail!("binary .fm2 input isn't supported; save the movie as text in FCEUX");
    }
    if header.contains_key("savestate") {
        bail!("the movie starts from a savestate; only movies from power-on can be imported");
    }
    if flag("palFlag") {
        bail!("the movie runs in PAL mode; only NTSC is recorded");
    }
    if flag("FDS") {
        bail!("Famicom Disk System movies aren't supported");
    }
    if flag("fourscore") {
        bail!("Four Score movies (four controllers) aren't supported yet");
    }
    // Port devices: 0 nothing, 1 gamepad, 2 Zapper; port2 is the expansion
    // port. port0 is required, since a missing one would drop controller 1.
    let Some(p0) = header.get("port0") else {
        bail!("the .fm2 header has no port0 line, so controller 1's device is unknown");
    };
    let p1 = header.get("port1").copied().unwrap_or("0");
    let p2 = header.get("port2").copied().unwrap_or("0");
    for (key, v, allowed) in [
        ("port0", *p0, &["0", "1"][..]),
        ("port1", p1, &["0", "1"][..]),
        ("port2", p2, &["0"][..]),
    ] {
        if !allowed.contains(&v) {
            bail!("{key} {v}: only gamepads (1) or nothing (0) can be imported");
        }
    }
    let (p0, p1) = (u32::from(*p0 == "1"), u32::from(p1 == "1"));
    // `length`, when present, is the movie's frame count; FCEUX ignores
    // records past it, and so does the import.
    let length: Option<usize> = header
        .get("length")
        .map(|v| {
            v.parse()
                .with_context(|| format!(".fm2 length {v:?} isn't a frame count"))
        })
        .transpose()?;
    let limit = length.unwrap_or(usize::MAX);

    let mut frames = Vec::new();
    let mut movie_frames = 0;
    let mut pending_reset = false;
    for (i, raw) in lines {
        let n = i + 1;
        let line = raw.trim_end_matches('\r');
        if let Some(rest) = line.strip_prefix('|') {
            if movie_frames >= limit {
                break;
            }
            if movie_frames >= MAX_FRAMES {
                bail!("the movie is longer than {MAX_FRAMES} frames, the import's limit");
            }
            if let Some(mut frame) = fm2_frame(n, rest, movie_frames, p0, p1)? {
                // FCEUX acts on a line's reset command after that line's
                // frame, so the reset is pressed before the next one: the
                // flag moves one frame on (TASVideos 3226M syncs this way,
                // docs/movies.md).
                let reset = std::mem::replace(&mut frame.reset, pending_reset);
                pending_reset = reset;
                frames.push(frame);
            }
            movie_frames += 1;
        } else if !line.trim().is_empty() {
            bail!(".fm2 line {n}: a header line after the first frame");
        }
    }
    if frames.is_empty() {
        bail!("the movie has no frames");
    }
    if pending_reset {
        bail!(".fm2: a soft reset on the last line has no frame after it to press it before");
    }

    let rom_hash = header
        .get("romChecksum")
        .map(|v| fm2_checksum(v))
        .transpose()?;
    let controllers = if p1 == 1 { 2 } else { 1 };
    let mut log = InputLog::new(controllers);
    log.frames = frames;
    Ok(Movie {
        format: "fm2",
        log,
        rom_hash,
        rom_name: header.get("romFilename").map(|s| s.to_string()),
        emulator: header.get("emuVersion").map(|v| format!("FCEUX {v}")),
        rerecords: header.get("rerecordCount").and_then(|v| v.parse().ok()),
    })
}

fn fm2_checksum(v: &str) -> Result<RomHash> {
    // FCEUX writes `base64:...`; its reader also takes `0x` followed by hex.
    let digest = if let Some(b) = v.strip_prefix("base64:") {
        base64::engine::general_purpose::STANDARD
            .decode(b.trim())
            .with_context(|| format!("romChecksum {v:?} isn't base64"))?
    } else if let Some(h) = v.strip_prefix("0x").or_else(|| v.strip_prefix("0X")) {
        hex::decode(h.trim()).with_context(|| format!("romChecksum {v:?} isn't hex"))?
    } else {
        bail!("romChecksum {v:?} is neither `base64:` nor `0x` hex");
    };
    if digest.len() != 16 {
        bail!("romChecksum {v:?} isn't an MD5 (16 bytes)");
    }
    Ok(RomHash::Md5(hex::encode(digest)))
}

/// One frame line, or None for the movie's first line, which is never one of
/// our frames.
fn fm2_frame(n: usize, rest: &str, index: usize, p0: u32, p1: u32) -> Result<Option<InputFrame>> {
    let fields: Vec<&str> = rest.split('|').collect();
    // `commands|port0|port1|port2|` splits into five, the last empty.
    if fields.len() < 4 {
        bail!(".fm2 line {n}: expected |commands|port0|port1|port2|");
    }
    let commands: u32 = fields[0]
        .trim()
        .parse()
        .with_context(|| format!(".fm2 line {n}: commands {:?} isn't a number", fields[0]))?;
    // Bit 1 is a soft reset, bit 2 a power cycle; higher bits are disk and
    // coin commands. FCEUX's first movie frame comes before our frame 0,
    // whatever its reset or power command: none, a power cycle (the power-on
    // every recording starts from), a soft reset or both. So the
    // first line is dropped and the second is our frame 0. Published runs
    // sync to their endings this way, with and without a first-line reset,
    // and BizHawk's conversion of 1715M holds the same inputs one frame
    // earlier (docs/movies.md). A first line holding buttons is refused,
    // since dropping it would lose input. A soft reset on a later line is
    // kept (below); a power cycle later has no input log form.
    if index == 0 {
        if commands & !3 != 0 {
            bail!(
                ".fm2 line {n} (movie frame 0): command {commands} (disk or coin) can't be imported"
            );
        }
        let held = [(fields[1], p0), (fields[2], p1)]
            .iter()
            .any(|(f, kind)| *kind != 0 && f.bytes().any(|c| c != b'.' && c != b' '));
        if held {
            bail!(
                ".fm2 line {n} (movie frame 0): the first line holds buttons, but it comes before our frame 0 and would be dropped"
            );
        }
        return Ok(None);
    }
    // A soft reset later in the movie is kept in the input log (format 1);
    // parse_fm2 moves it one frame on, since FCEUX acts on it after this
    // line's frame. A power cycle, or a disk or coin command, has no input
    // log form.
    if commands != 0 && commands != 1 {
        bail!(
            ".fm2 line {n} (movie frame {index}): command {commands} (power, disk or coin) can't be imported"
        );
    }
    let reset = commands == 1;
    let mut pads = [0u8; 4];
    for (port, (field, kind)) in [(fields[1], p0), (fields[2], p1)].into_iter().enumerate() {
        if kind == 0 {
            continue;
        }
        if field.len() != 8 {
            bail!(".fm2 line {n}: port{port} {field:?} isn't 8 buttons (RLDUTSBA)");
        }
        for (i, c) in field.bytes().enumerate() {
            if c != b'.' && c != b' ' {
                pads[port] |= FM2_ORDER[i];
            }
        }
    }
    Ok(Some(InputFrame {
        polled: true,
        pads,
        reset,
    }))
}

/// Parses a BizHawk movie: a zip with `Header.txt` (`Key Value` lines) and
/// `Input Log.txt`, whose `LogKey` names each column.
pub fn parse_bk2(bytes: &[u8]) -> Result<Movie> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .context("the .bk2 file isn't a zip archive")?;
    let mut read = |name: &str| -> Result<Option<String>> {
        let Ok(mut entry) = zip.by_name(name) else {
            return Ok(None);
        };
        // A movie's text parts are small; refuse anything absurd before
        // reading it into memory.
        // 64 MiB holds MAX_FRAMES lines of two gamepads with room to spare.
        const LIMIT: u64 = 64 << 20;
        if entry.size() > LIMIT {
            bail!("{name} in the .bk2 is over 64 MiB");
        }
        let mut s = String::new();
        // The declared size can lie; read one byte past the limit to notice.
        entry
            .by_ref()
            .take(LIMIT + 1)
            .read_to_string(&mut s)
            .with_context(|| format!("reading {name} from the .bk2"))?;
        if s.len() as u64 > LIMIT {
            bail!("{name} in the .bk2 is over 64 MiB");
        }
        Ok(Some(s))
    };
    let header_text = read("Header.txt")?.context("the .bk2 has no Header.txt")?;
    let input_text = read("Input Log.txt")?.context("the .bk2 has no Input Log.txt")?;

    let mut header: HashMap<String, String> = HashMap::new();
    for line in header_text.lines() {
        let line = line.trim_start_matches('\u{feff}').trim_end_matches('\r');
        if let Some((k, v)) = line.split_once(' ') {
            header
                .entry(k.to_owned())
                .or_insert_with(|| v.trim().to_owned());
        }
    }
    match header.get("Platform").map(String::as_str) {
        Some("NES") => {}
        Some(p) => bail!("the .bk2 is for platform {p}, not NES"),
        None => bail!("the .bk2 header has no Platform"),
    }
    let truthy = |k: &str| {
        header
            .get(k)
            .is_some_and(|v| v.eq_ignore_ascii_case("true"))
    };
    if truthy("StartsFromSavestate") || truthy("StartsFromSaveRam") {
        bail!(
            "the movie starts from a savestate or save RAM; only movies from power-on can be imported"
        );
    }
    if truthy("PAL") {
        bail!("the movie runs in PAL mode; only NTSC is recorded");
    }
    // BizHawk names the ROM by `SHA1`, or for some games by `MD5`, the
    // headerless MD5 (TASVideos 6366M, Super Mario Bros. 2).
    let rom_hash = header.get("SHA1").map(|v| bk2_hash(v)).or_else(|| {
        header
            .get("MD5")
            .map(|v| RomHash::Md5(v.to_ascii_lowercase()))
    });

    let (controllers, frames) = bk2_input(&input_text)?;
    let mut log = InputLog::new(controllers);
    log.frames = frames;
    Ok(Movie {
        format: "bk2",
        log,
        rom_hash,
        rom_name: header.get("GameName").cloned(),
        emulator: Some(
            [header.get("MovieVersion"), header.get("Core")]
                .into_iter()
                .flatten()
                .cloned()
                .collect::<Vec<_>>()
                .join(", "),
        )
        .filter(|s| !s.is_empty()),
        rerecords: header.get("rerecordCount").and_then(|v| v.parse().ok()),
    })
}

/// The `SHA1` header value. BizHawk writes its game database's hash for the
/// ROM there, so it may carry a `SHA1:` or `MD5:` prefix, and a database
/// entry keyed by MD5 gives 32 hex digits instead of 40.
fn bk2_hash(value: &str) -> RomHash {
    let v = value.trim().to_ascii_lowercase();
    if let Some(md5) = v.strip_prefix("md5:") {
        return RomHash::Md5(md5.to_owned());
    }
    let v = v.strip_prefix("sha1:").unwrap_or(&v);
    if v.len() == 32 {
        RomHash::Md5(v.to_owned())
    } else {
        RomHash::Sha1(v.to_owned())
    }
}

/// A column of a BizHawk input log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Column {
    Pad { port: usize, bit: u8 },
    Reset,
    Power,
}

fn bk2_column(name: &str) -> Option<Column> {
    match name {
        "Reset" => return Some(Column::Reset),
        "Power" => return Some(Column::Power),
        _ => {}
    }
    let rest = name.strip_prefix('P')?;
    let (port, button_name) = rest.split_once(' ')?;
    let port: usize = port.parse().ok()?;
    if !(1..=4).contains(&port) {
        return None;
    }
    let bit = match button_name {
        "Up" => button::UP,
        "Down" => button::DOWN,
        "Left" => button::LEFT,
        "Right" => button::RIGHT,
        "Start" => button::START,
        "Select" => button::SELECT,
        "B" => button::B,
        "A" => button::A,
        _ => return None,
    };
    Some(Column::Pad {
        port: port - 1,
        bit,
    })
}

fn bk2_input(text: &str) -> Result<(u8, Vec<InputFrame>)> {
    let mut columns: Option<Vec<Column>> = None;
    let mut frames = Vec::new();
    let mut ports = 0usize;
    for (i, raw) in text.lines().enumerate() {
        let n = i + 1;
        let line = raw.trim_start_matches('\u{feff}').trim_end_matches('\r');
        if let Some(key) = line.strip_prefix("LogKey:") {
            let mut cols = Vec::new();
            for name in key.split(['#', '|']).filter(|s| !s.is_empty()) {
                let col = bk2_column(name).with_context(|| {
                    format!(
                        "Input Log.txt: column {name:?} isn't a NES gamepad button, Reset or Power"
                    )
                })?;
                if let Column::Pad { port, .. } = col {
                    ports = ports.max(port + 1);
                }
                cols.push(col);
            }
            columns = Some(cols);
        } else if let Some(rest) = line.strip_prefix('|') {
            let cols = columns
                .as_ref()
                .context("Input Log.txt: a frame line before the LogKey line")?;
            if frames.len() >= MAX_FRAMES {
                bail!("the movie is longer than {MAX_FRAMES} frames, the import's limit");
            }
            let marks: Vec<u8> = rest.bytes().filter(|&b| b != b'|').collect();
            if marks.len() != cols.len() {
                bail!(
                    "Input Log.txt line {n}: {} buttons where the LogKey names {}",
                    marks.len(),
                    cols.len()
                );
            }
            let index = frames.len();
            let mut pads = [0u8; 4];
            let mut reset = false;
            for (col, mark) in cols.iter().zip(marks) {
                if mark == b'.' || mark == b' ' {
                    continue;
                }
                match col {
                    Column::Pad { port, bit } => pads[*port] |= bit,
                    Column::Power if index == 0 => {}
                    // A reset after the first frame is pressed at the start
                    // of its frame (input log format 1). One on the first
                    // frame, or a power cycle later, is refused.
                    Column::Reset if index > 0 => reset = true,
                    Column::Reset => bail!(
                        "Input Log.txt line {n} (frame 0): a reset on the first frame can't be imported"
                    ),
                    Column::Power => bail!(
                        "Input Log.txt line {n} (frame {index}): a power cycle can't be imported"
                    ),
                }
            }
            frames.push(InputFrame {
                polled: true,
                pads,
                reset,
            });
        }
    }
    if columns.is_none() {
        bail!("Input Log.txt has no LogKey line");
    }
    if frames.is_empty() {
        bail!("the movie has no frames");
    }
    match ports {
        1 | 2 => Ok((ports as u8, frames)),
        0 => bail!("the movie has no gamepad columns"),
        _ => bail!("movies with more than two gamepads aren't supported yet"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FM2: &str = "version 3\nemuVersion 22020\nrerecordCount 5\npalFlag 0\nromFilename smb\nromChecksum base64:jjYwGG411HcjG/j9UOVM3Q==\nguid 00000000-0000-0000-0000-000000000000\nfourscore 0\nmicrophone 0\nport0 1\nport1 1\nport2 0\nFDS 0\nNewPPU 0\n|2|........|........||\n|0|....T...|........||\n|0|R......A|.L......||\n";

    #[test]
    fn fm2_buttons_and_checksum() {
        let m = parse_fm2(FM2).unwrap();
        assert_eq!(m.log.controllers, 2);
        // The first line (here a power cycle) comes before our frame 0.
        assert_eq!(m.log.frames.len(), 2);
        assert_eq!(m.log.frames[0].pads[0], button::START);
        assert_eq!(m.log.frames[1].pads[0], button::RIGHT | button::A);
        assert_eq!(m.log.frames[1].pads[1], button::LEFT);
        assert!(m.log.frames.iter().all(|f| f.polled));
        assert_eq!(
            m.rom_hash,
            Some(RomHash::Md5("8e3630186e35d477231bf8fd50e54cdd".into()))
        );
        assert_eq!(m.rerecords, Some(5));
    }

    #[test]
    fn fm2_refusals() {
        for (from, to, what) in [
            ("port1 1", "port1 2", "port1 2"),
            ("palFlag 0", "palFlag 1", "PAL"),
            ("fourscore 0", "fourscore 1", "Four Score"),
            ("|0|....T...", "|3|....T...", "command 3"),
            ("version 3", "version 2", "version 2"),
            ("FDS 0", "FDS 1", "Famicom Disk System"),
            ("NewPPU 0", "binary 1", "binary"),
            ("port0 1\n", "", "no port0"),
            ("port0 1", "port0 2", "only gamepads"),
            ("port1 1", "port1 3", "only gamepads"),
            ("port2 0", "port2 1", "only gamepads"),
            ("|0|R......A", "|2|R......A", "command 2"),
            ("romChecksum base64:", "romChecksum ", "neither"),
            ("NewPPU 0", "length lots", "isn't a frame count"),
        ] {
            let text = FM2.replacen(from, to, 1);
            let e = parse_fm2(&text).unwrap_err().to_string();
            assert!(e.contains(what), "{from} -> {to}: {e}");
        }
        let e = parse_fm2(&format!("{FM2}savestate base64:AAAA\n"))
            .unwrap_err()
            .to_string();
        assert!(e.contains("header line after"), "{e}");
        // A soft reset after the first line is pressed before the frame
        // after its line's, and one on the last line is refused.
        let m = parse_fm2(&FM2.replacen("|0|....T...", "|1|....T...", 1)).unwrap();
        assert!(!m.log.frames[0].reset && m.log.frames[1].reset);
        assert_eq!(m.log.frames[0].pads[0], button::START);
        let e = parse_fm2(&FM2.replacen("|0|R......A", "|1|R......A", 1))
            .unwrap_err()
            .to_string();
        assert!(e.contains("last line"), "{e}");
        let e = parse_fm2(&FM2.replacen("NewPPU 0\n", "NewPPU 0\nsavestate base64:AA==\n", 1))
            .unwrap_err()
            .to_string();
        assert!(e.contains("savestate"), "{e}");
    }

    #[test]
    fn a_shift_adds_or_drops_frames_at_the_start() {
        let mut m = parse_fm2(FM2).unwrap();
        let original = m.log.frames.clone();
        apply_shift(&mut m.log, 2).unwrap();
        assert_eq!(m.log.frames.len(), original.len() + 2);
        assert_eq!(m.log.frames[0].pads, [0; 4]);
        assert_eq!(m.log.frames[2..], original[..]);
        apply_shift(&mut m.log, -2).unwrap();
        assert_eq!(m.log.frames, original);
        // Dropping a frame that holds input is refused.
        let e = apply_shift(&mut m.log, -1).unwrap_err().to_string();
        assert!(e.contains("hold input"), "{e}");
    }

    #[test]
    fn the_first_line_is_dropped_whatever_its_command() {
        // HappyLee's 1715M starts with |1|, 4313M with |0|; both sync with
        // their first line dropped, so the second line is frame 0.
        for first in ["|1|........", "|0|........", "|2|........"] {
            let m = parse_fm2(&FM2.replacen("|2|........", first, 1)).unwrap();
            assert_eq!(m.log.frames.len(), 2, "{first}");
            assert_eq!(m.log.frames[0].pads[0], button::START, "{first}");
        }
        // Buttons on the first line would be lost, so it's refused.
        let e = parse_fm2(&FM2.replacen("|2|........", "|0|.......A", 1))
            .unwrap_err()
            .to_string();
        assert!(e.contains("first line holds buttons"), "{e}");
        let e = parse_fm2(&FM2.replacen("|2|........", "|4|........", 1))
            .unwrap_err()
            .to_string();
        assert!(e.contains("disk or coin"), "{e}");
        // Controller 2's buttons count when port1 is a gamepad.
        let e = parse_fm2(&FM2.replacen("|2|........|........|", "|2|........|...U....|", 1))
            .unwrap_err()
            .to_string();
        assert!(e.contains("first line holds buttons"), "{e}");
        // With no gamepad in port1, its field is ignored.
        let one = FM2.replacen("port1 1", "port1 0", 1).replacen(
            "|2|........|........|",
            "|2|........|...U....|",
            1,
        );
        assert_eq!(parse_fm2(&one).unwrap().log.frames.len(), 2);
        // A movie that is only its first line has no frames.
        let only = FM2.split_inclusive('\n').take(15).collect::<String>();
        assert!(only.ends_with("|2|........|........||\n"), "{only}");
        let e = parse_fm2(&only).unwrap_err().to_string();
        assert!(e.contains("no frames"), "{e}");
    }

    #[test]
    fn fm2_length_and_hex_checksum() {
        // `length` cuts the frames short; records past it are ignored.
        // `length` counts the dropped first line too.
        let m = parse_fm2(&FM2.replacen("NewPPU 0", "length 2", 1)).unwrap();
        assert_eq!(m.log.frames.len(), 1);
        let m = parse_fm2(&FM2.replacen("NewPPU 0", "length 99", 1)).unwrap();
        assert_eq!(m.log.frames.len(), 2);
        let m = parse_fm2(&FM2.replacen(
            "base64:jjYwGG411HcjG/j9UOVM3Q==",
            "0x8e3630186e35d477231bf8fd50e54cdd",
            1,
        ))
        .unwrap();
        assert_eq!(
            m.rom_hash,
            Some(RomHash::Md5("8e3630186e35d477231bf8fd50e54cdd".into()))
        );
    }

    fn zip_of(files: &[(&str, &str)]) -> Vec<u8> {
        use std::io::Write as _;
        let mut out = std::io::Cursor::new(Vec::new());
        let mut zip = zip::ZipWriter::new(&mut out);
        for (name, text) in files {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(text.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
        out.into_inner()
    }

    const BK2_HEADER: &str = "MovieVersion BizHawk v2.0.0\nPlatform NES\nGameName x\nSHA1 EA343F4E445A9050D4B4FBAC2C77D0693B1D0922\nCore NesHawk\nStartsFromSavestate False\n";
    const BK2_INPUT: &str = "[Input]\nLogKey:#Reset|Power|#P1 Up|P1 Down|P1 Left|P1 Right|P1 Start|P1 Select|P1 B|P1 A|\n|.P|........|\n|..|...R....|\n[/Input]\n";

    #[test]
    fn bk2_parses_and_refuses() {
        let m = parse_bk2(&zip_of(&[
            ("Header.txt", BK2_HEADER),
            ("Input Log.txt", BK2_INPUT),
        ]))
        .unwrap();
        assert_eq!(m.log.frames.len(), 2);
        assert_eq!(m.log.frames[1].pads[0], button::RIGHT);

        for (from, to, what) in [
            (
                "StartsFromSavestate False",
                "StartsFromSavestate True",
                "savestate",
            ),
            (
                "StartsFromSavestate False",
                "StartsFromSaveRam True",
                "save RAM",
            ),
            ("Core NesHawk", "PAL True", "PAL"),
            ("Platform NES", "Platform SNES", "platform SNES"),
            ("Platform NES\n", "", "no Platform"),
        ] {
            let header = BK2_HEADER.replacen(from, to, 1);
            let e = parse_bk2(&zip_of(&[
                ("Header.txt", &header),
                ("Input Log.txt", BK2_INPUT),
            ]))
            .unwrap_err()
            .to_string();
            assert!(e.contains(what), "{from} -> {to}: {e}");
        }
        let e = parse_bk2(&zip_of(&[("Header.txt", BK2_HEADER)]))
            .unwrap_err()
            .to_string();
        assert!(e.contains("Input Log.txt"), "{e}");

        // No SHA1: it parses, and the identity check refuses it.
        let header = BK2_HEADER.replacen("SHA1 EA343F4E445A9050D4B4FBAC2C77D0693B1D0922\n", "", 1);
        let m = parse_bk2(&zip_of(&[
            ("Header.txt", &header),
            ("Input Log.txt", BK2_INPUT),
        ]))
        .unwrap();
        assert_eq!(m.rom_hash, None);
        let target = Target {
            source: Some("smb1-ntsc-prg0".into()),
            md5: vec![],
            sha1: vec!["ea343f4e445a9050d4b4fbac2c77d0693b1d0922".into()],
        };
        let e = check_identity(&m, &target, &Registry::default())
            .unwrap_err()
            .to_string();
        assert!(e.contains("doesn't name its ROM"), "{e}");
    }

    #[test]
    fn bk2_input_refusals() {
        // Power after the first frame, and more than two gamepads.
        let e = bk2_input(&BK2_INPUT.replacen("|..|...R....|", "|.P|...R....|", 1))
            .unwrap_err()
            .to_string();
        assert!(e.contains("power cycle"), "{e}");
        let three = "LogKey:#P1 A|#P2 A|#P3 A|\n|.|.|.|\n";
        let e = bk2_input(three).unwrap_err().to_string();
        assert!(e.contains("more than two"), "{e}");
        let e = bk2_input("|..|........|\n").unwrap_err().to_string();
        assert!(e.contains("before the LogKey"), "{e}");
    }

    #[test]
    fn imports_stop_at_the_frame_limit() {
        let mut text = String::from("LogKey:#P1 A|\n");
        text.push_str(&"|.|\n".repeat(MAX_FRAMES + 1));
        let e = bk2_input(&text).unwrap_err().to_string();
        assert!(e.contains("longer than"), "{e}");
        let mut fm2 = FM2.to_owned();
        fm2.push_str(&"|0|........|........||\n".repeat(MAX_FRAMES));
        let e = parse_fm2(&fm2).unwrap_err().to_string();
        assert!(e.contains("longer than"), "{e}");
    }

    #[test]
    fn bk2_hash_forms() {
        let sha1 = "ea343f4e445a9050d4b4fbac2c77d0693b1d0922";
        for v in [
            "EA343F4E445A9050D4B4FBAC2C77D0693B1D0922",
            "SHA1:EA343F4E445A9050D4B4FBAC2C77D0693B1D0922",
            "sha1:ea343f4e445a9050d4b4fbac2c77d0693b1d0922",
        ] {
            assert_eq!(bk2_hash(v), RomHash::Sha1(sha1.into()), "{v}");
        }
        let md5 = "8e3630186e35d477231bf8fd50e54cdd";
        assert_eq!(bk2_hash(md5), RomHash::Md5(md5.into()));
        assert_eq!(
            bk2_hash("MD5:8E3630186E35D477231BF8FD50E54CDD"),
            RomHash::Md5(md5.into())
        );
    }

    #[test]
    fn a_bk2_may_name_its_rom_by_md5() {
        let header = BK2_HEADER.replace(
            "SHA1 EA343F4E445A9050D4B4FBAC2C77D0693B1D0922",
            "MD5 8E3630186E35D477231BF8FD50E54CDD",
        );
        let m = parse_bk2(&zip_of(&[
            ("Header.txt", &header),
            ("Input Log.txt", BK2_INPUT),
        ]))
        .unwrap();
        assert_eq!(
            m.rom_hash,
            Some(RomHash::Md5("8e3630186e35d477231bf8fd50e54cdd".into()))
        );
    }

    #[test]
    fn bk2_columns_follow_the_log_key() {
        // Columns in an unusual order still land on the right buttons.
        let text = "[Input]\nLogKey:#Reset|Power|#P1 A|P1 B|P1 Up|P1 Down|P1 Left|P1 Right|P1 Start|P1 Select|\n|.P|........|\n|..|A......S|\n|..|.B...R..|\n[/Input]\n";
        let (controllers, frames) = bk2_input(text).unwrap();
        assert_eq!(controllers, 1);
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[1].pads[0], button::A | button::SELECT);
        assert_eq!(frames[2].pads[0], button::B | button::RIGHT);
        // A reset after the first frame is kept; on the first it's refused.
        let (_, reset) = bk2_input(&text.replacen("|..|A......S|", "|r.|A......S|", 1)).unwrap();
        assert!(reset[1].reset && !reset[2].reset);
        let e = bk2_input(&text.replacen("|.P|........|", "|r.|........|", 1))
            .unwrap_err()
            .to_string();
        assert!(e.contains("reset on the first frame"), "{e}");
        let e = bk2_input(&text.replacen("P1 A|", "P1 Trigger|", 1))
            .unwrap_err()
            .to_string();
        assert!(e.contains("Trigger"), "{e}");
    }
}
