//! `gp testroms`: the accuracy baseline. Downloads the standard NES test
//! ROMs at pinned hashes and runs them on gp-nes (docs/test-roms.md).
//!
//! The suites, the source commit and every file's SHA-256 are in
//! test-roms/suites.toml, compiled in so `gp` needs no repo to run them.
//!
//! Ported from game-printer's testroms.rs; the run recorder is the
//! process's (crate::runrec), and the command's arguments moved here from
//! main.rs. These are the NES's (gp-nes's CPU, PPU, APU and mappers). An
//! arcade board's counterpart is the MAME oracle (`gp oracle diff` against
//! a capture, docs/oracle.md); a Z80 instruction suite (ZEXDOC and ZEXALL,
//! CP/M programs) needs a CP/M harness on gp-arcade's Z80 first (the
//! program at 0100, BDOS calls 2 and 9 at 0005 printing its result) and a
//! pinned download, and isn't here yet.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use gp_evidence::Paths;
use gp_evidence::manifest::{Mmc3Revision, PowerOn, RamInit, Region};
use gp_nes::accuracy::{self, Verdict};

use crate::Outcome;

#[derive(clap::Subcommand)]
pub enum TestromsCmd {
    /// Download every test ROM that's missing into $GP_HOME/work/test-roms,
    /// checking each against its pinned SHA-256. Exits 1 if a file's hash
    /// differs from its pin, 2 if a download failed.
    Fetch {
        /// Download every file again, even those already at their pinned hash.
        #[arg(long)]
        redownload: bool,
    },
    /// Run the suites and report each ROM. Downloads missing ROMs first and
    /// skips, with a notice, any it can't get. Records each suite's pass
    /// count as `testroms.<suite>.passed` and `.total`. Exits 1 on a failure
    /// not listed as known in the manifest, a known failure that now passes,
    /// or a download whose SHA-256 isn't its pin. The suites are the NES's;
    /// an arcade board is held to MAME with `gp oracle diff`.
    Run {
        /// Suites to run, by name. By default, all of them.
        suites: Vec<String>,
        /// Don't download; run only what's already in $GP_HOME/work/test-roms.
        #[arg(long)]
        offline: bool,
        /// Power on with RAM all ones instead of zeros, to find results that
        /// depend on it. Records `testroms.ram_ones.*` instead of `testroms.*`.
        #[arg(long)]
        ram_ones: bool,
    },
    /// List our own test programs, which are built rather than downloaded:
    /// each one's path under $GP_HOME/work/test-roms and the source in the
    /// repo it's built from, tab-separated. `just build-testroms` reads it.
    Built,
}

/// `gp testroms`.
pub(crate) fn command(cmd: &TestromsCmd, paths: &Paths) -> Result<Outcome> {
    let m = Manifest::load(None)?;
    match cmd {
        TestromsCmd::Fetch { redownload } => fetch_all(&m, *redownload, paths),
        TestromsCmd::Run {
            suites,
            offline,
            ram_ones,
        } => {
            let ram = if *ram_ones {
                RamInit::Ones
            } else {
                RamInit::Zeros
            };
            run(&m, suites, *offline, ram, paths)
        }
        TestromsCmd::Built => Ok(built(&m)),
    }
}

/// The committed manifest.
const SUITES: &str = include_str!("../../../test-roms/suites.toml");

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    source: Source,
    #[serde(rename = "suite")]
    suites: Vec<Suite>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    /// `owner/name` on GitHub.
    repo: String,
    /// The full commit SHA every file is downloaded at.
    commit: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Check {
    Nestest,
    Blargg,
    #[serde(rename = "blargg-2005")]
    Blargg2005,
    /// blargg's 2005 PPU and APU tests: the result code printed as `$01`.
    #[serde(rename = "blargg-2005-screen")]
    Blargg2005Screen,
    Screen,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Suite {
    name: String,
    check: Check,
    about: String,
    /// The most frames one ROM may run.
    frames: Option<u64>,
    /// The region to power on in, `ntsc` (the default) or `pal`.
    region: Option<String>,
    /// Set when what the suite is compared with was made with RAM powered
    /// on as zeros (nestest.log), so a `--ram-ones` run skips it.
    #[serde(default)]
    needs_zeroed_ram: bool,
    /// nestest's expected CPU log.
    log: Option<File>,
    #[serde(rename = "rom")]
    roms: Vec<Rom>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    path: String,
    sha256: String,
    /// Set for our own programs, which are built, never downloaded.
    #[serde(skip)]
    built_from: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Rom {
    path: String,
    sha256: String,
    /// The MMC3 revision to power on with, for a ROM that tests one.
    mmc3_revision: Option<String>,
    /// For a test program of our own: its source in the repo. It's
    /// assembled into the work directory by `just build-testroms`, never
    /// downloaded.
    built_from: Option<String>,
    /// Why this ROM fails today: the open question that tracks it.
    known_failure: Option<String>,
    /// How its result starts when it fails in the known way, such as
    /// `code 2:`. Required with `known_failure`: any other failure, a
    /// timeout included, is a new failure.
    known_result: Option<String>,
}

impl Manifest {
    pub fn load(path: Option<&Path>) -> Result<Self> {
        let text = match path {
            Some(p) => fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?,
            None => SUITES.to_owned(),
        };
        let m: Manifest = toml::from_str(&text).context("reading the test ROM manifest")?;
        let hex = |s: &str, len: usize| {
            s.len() == len
                && s.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        };
        if !hex(&m.source.commit, 40) {
            bail!("the manifest's commit must be a full 40-character SHA");
        }
        let safe = |s: &str| {
            !s.is_empty()
                && !s
                    .split('/')
                    .any(|part| part.is_empty() || part == "." || part == "..")
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"/._-".contains(&b))
        };
        if !safe(&m.source.repo) || m.source.repo.matches('/').count() != 1 {
            bail!("the manifest's repo must be owner/name on GitHub");
        }
        for s in &m.suites {
            for f in s.files() {
                if !safe(&f.path) || !hex(&f.sha256, 64) {
                    bail!("{}: bad path or SHA-256 in the manifest", f.path);
                }
            }
            if let Some(r) = &s.region
                && !matches!(r.as_str(), "ntsc" | "pal")
            {
                bail!("{}: region is ntsc or pal, not {r:?}", s.name);
            }
            if (s.check == Check::Nestest) != s.log.is_some() {
                bail!("{}: a nestest suite has a log, and only it", s.name);
            }
            for r in &s.roms {
                if let Some(source) = &r.built_from
                    && (!safe(source) || !source.starts_with("test-roms/"))
                {
                    bail!(
                        "{}: built_from must be a source path under test-roms/, not {source:?}",
                        r.path
                    );
                }
                // `just build-testroms` builds `x.s` with the `x.cfg` beside
                // it into gp/x.nes, so the path follows from the source.
                if let Some(source) = &r.built_from {
                    let stem = source.rsplit('/').next().and_then(|f| f.strip_suffix(".s"));
                    if stem.is_none_or(|stem| r.path != format!("gp/{stem}.nes")) {
                        bail!(
                            "{}: a program built from {source} must be an assembly source x.s, built to gp/x.nes",
                            r.path
                        );
                    }
                }
                if r.known_failure.is_some() != r.known_result.is_some() {
                    bail!(
                        "{}: known_failure and known_result go together, so only the known failure is excused",
                        r.path
                    );
                }
            }
        }
        Ok(m)
    }

    fn url(&self, path: &str) -> String {
        format!(
            "https://raw.githubusercontent.com/{}/{}/{path}",
            self.source.repo, self.source.commit
        )
    }
}

impl Suite {
    /// Every file the suite needs.
    fn files(&self) -> Vec<File> {
        self.roms
            .iter()
            .map(|r| File {
                path: r.path.clone(),
                sha256: r.sha256.clone(),
                built_from: r.built_from.clone(),
            })
            .chain(self.log.clone())
            .collect()
    }
}

fn work_dir(paths: &Paths) -> PathBuf {
    paths.home.join("work").join("test-roms")
}

fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Why a suite with `needs_zeroed_ram` is skipped when RAM powers on as ones.
const ZEROED_RAM_ONLY: &str = "skipped: its expected output was made with RAM powered on as zeros";

/// What fetching one file came to.
enum Fetched {
    /// Present with the pinned hash, as bytes.
    Ready(Vec<u8>),
    /// Couldn't be had; the reason. The ROM is skipped.
    Missing(String),
    /// Downloaded, but with another hash. Never used.
    WrongHash(String),
}

/// The file at its pinned hash: from the work directory, or downloaded into
/// it unless `offline`. A cached file with another hash is downloaded again.
fn fetch(m: &Manifest, file: &File, dir: &Path, offline: bool) -> Result<Fetched> {
    let dest = dir.join(&file.path);
    let cached = fs::read(&dest).ok();
    if let Some(bytes) = &cached
        && sha256(bytes) == file.sha256
    {
        return Ok(Fetched::Ready(bytes.clone()));
    }
    if let Some(source) = &file.built_from {
        // Our own program: built from source, so never downloaded. A build
        // with another hash means the source or the assembler changed.
        return Ok(match cached {
            Some(bytes) => Fetched::WrongHash(format!(
                "{} has SHA-256 {}, the manifest pins {}: rebuild it with `just build-testroms`, and if {source} changed, update the pin",
                dest.display(),
                sha256(&bytes),
                file.sha256
            )),
            None => Fetched::Missing(format!(
                "our own program, not built yet: `just build-testroms` assembles it from {source}"
            )),
        });
    }
    if offline {
        return Ok(match cached {
            // A cached file that changed is never used, and never quietly skipped.
            Some(bytes) => Fetched::WrongHash(format!(
                "{} has SHA-256 {}, the manifest pins {}",
                dest.display(),
                sha256(&bytes),
                file.sha256
            )),
            None => Fetched::Missing("not downloaded, and --offline".into()),
        });
    }
    let parent = dest.parent().context("a manifest path has no directory")?;
    fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    let partial = dest.with_extension("partial");
    let url = m.url(&file.path);
    let status = Command::new("curl")
        .args(["-sSfL", "--retry", "2", "--proto", "=https", "-o"])
        .arg(&partial)
        .arg(&url)
        .status();
    match status {
        Err(e) => return Ok(Fetched::Missing(format!("couldn't run curl: {e}"))),
        Ok(s) if !s.success() => {
            let _ = fs::remove_file(&partial);
            return Ok(Fetched::Missing(format!(
                "download failed (offline?): {url}"
            )));
        }
        Ok(_) => {}
    }
    let bytes = fs::read(&partial).with_context(|| format!("reading {}", partial.display()))?;
    let got = sha256(&bytes);
    if got != file.sha256 {
        let _ = fs::remove_file(&partial);
        return Ok(Fetched::WrongHash(format!(
            "{url} has SHA-256 {got}, the manifest pins {}",
            file.sha256
        )));
    }
    fs::rename(&partial, &dest).with_context(|| format!("writing {}", dest.display()))?;
    Ok(Fetched::Ready(bytes))
}

/// Our own test programs: each built ROM's path and its source, as
/// `gp testroms built` lists them for `just build-testroms`.
pub fn built(m: &Manifest) -> crate::Outcome {
    let built: Vec<_> = m
        .suites
        .iter()
        .flat_map(|s| &s.roms)
        .filter_map(|r| {
            r.built_from
                .as_ref()
                .map(|src| (r.path.as_str(), src.as_str()))
        })
        .collect();
    let text = built
        .iter()
        .map(|(path, src)| format!("{path}\t{src}"))
        .collect::<Vec<_>>()
        .join("\n");
    let json = built
        .iter()
        .map(|(path, src)| json!({ "path": path, "built_from": src }))
        .collect::<Vec<_>>();
    crate::Outcome::ok(json!({ "built": json }), text)
}

/// `gp testroms fetch`: downloads every file that's missing or has the
/// wrong hash, or every file with `redownload`. Exits 1 if a download has a hash other than the pin, 2 if
/// some couldn't be downloaded.
pub fn fetch_all(m: &Manifest, redownload: bool, paths: &Paths) -> Result<Outcome> {
    let dir = work_dir(paths);
    if redownload {
        for f in m
            .suites
            .iter()
            .flat_map(Suite::files)
            .filter(|f| f.built_from.is_none())
        {
            match fs::remove_file(dir.join(&f.path)) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                    return Err(e).with_context(|| format!("removing {}", f.path));
                }
                _ => {}
            }
        }
    }
    let (mut ready, mut missing, mut wrong) = (0u64, Vec::new(), Vec::new());
    for s in &m.suites {
        // Our own programs are built by `just build-testroms`, not fetched.
        for f in s.files().into_iter().filter(|f| f.built_from.is_none()) {
            match fetch(m, &f, &dir, false)? {
                Fetched::Ready(_) => ready += 1,
                Fetched::Missing(why) => missing.push(format!("{}: {why}", f.path)),
                Fetched::WrongHash(why) => wrong.push(why),
            }
        }
    }
    crate::runrec::metric("testroms.fetch.ready", ready as f64, "count");
    let mut lines = vec![format!(
        "{ready} files at their pinned hashes in {}",
        dir.display()
    )];
    lines.extend(missing.iter().map(|w| format!("missing   {w}")));
    lines.extend(wrong.iter().map(|w| format!("WRONG     {w}")));
    if !wrong.is_empty() {
        lines.push(
            "The pinned source changed upstream, or the download was corrupted: stop and review before changing a pin.".into(),
        );
    }
    let exit = if !wrong.is_empty() {
        1
    } else {
        2 * u8::from(!missing.is_empty())
    };
    Ok(Outcome {
        json: json!({ "dir": dir, "ready": ready, "missing": missing, "wrong_hash": wrong }),
        text: lines.join("\n"),
        exit,
    })
}

/// How one ROM's result compares with what the manifest expects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Passed,
    /// Failed, and expected to (a known failure).
    Known,
    /// Failed or timed out, and not expected to.
    Failed,
    /// Passed, though listed as a known failure.
    Fixed,
    Skipped,
}

impl Status {
    fn mark(self) -> &'static str {
        match self {
            Self::Passed => "ok",
            Self::Known => "known",
            Self::Failed => "FAILED",
            Self::Fixed => "FIXED",
            Self::Skipped => "skip",
        }
    }
}

/// One line of the results table.
struct Row {
    suite: String,
    name: String,
    status: Status,
    detail: String,
    frames: Option<u64>,
}

/// One check's result, before it's compared with the manifest.
struct Checked {
    name: String,
    passed: bool,
    detail: String,
    frames: Option<u64>,
}

/// Runs one ROM by its suite's check and says whether it passed, with
/// details. nestest gives two rows: official opcodes, then all of them.
fn run_rom(
    s: &Suite,
    rom: &Rom,
    bytes: &[u8],
    log: Option<&str>,
    ram_init: RamInit,
) -> Result<Vec<Checked>> {
    let name = Path::new(&rom.path)
        .file_stem()
        .map_or_else(|| rom.path.clone(), |n| n.to_string_lossy().into_owned());
    let mut power_on = PowerOn {
        ram_init,
        ..PowerOn::default()
    };
    if s.region.as_deref() == Some("pal") {
        power_on.region = Region::Pal;
    }
    if let Some(rev) = &rom.mmc3_revision {
        power_on.mapper_revisions.mmc3 = match rev.as_str() {
            "a" => Mmc3Revision::A,
            "bc" => Mmc3Revision::BC,
            "acclaim" => Mmc3Revision::Acclaim,
            other => bail!("{}: unknown mmc3_revision {other:?}", rom.path),
        };
    }
    let frames = s.frames.unwrap_or(3000);
    let verdict = |r: accuracy::TestResult| {
        // The ROM's own text, without blank lines. A pass shows only the
        // last line ("Passed"); the rest is diagnostics.
        let text: Vec<&str> = r
            .message
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        let detail = match r.verdict {
            Verdict::TimedOut => {
                format!("timed out after {} frames: {}", r.frames, text.join(" / "))
            }
            Verdict::Passed => format!(
                "{} in {} frames",
                text.last().copied().unwrap_or("passed"),
                r.frames
            ),
            Verdict::Failed => match r.code {
                Some(code) => format!("code {code}: {}", text.join(" / ")),
                None => text.join(" / "),
            },
        };
        vec![Checked {
            name: name.clone(),
            passed: r.verdict == Verdict::Passed,
            detail,
            frames: Some(r.frames),
        }]
    };
    Ok(match s.check {
        Check::Blargg => verdict(accuracy::run_blargg(bytes, power_on, frames)?),
        Check::Blargg2005 => verdict(accuracy::run_blargg_2005(bytes, power_on, frames)?),
        Check::Blargg2005Screen => verdict(accuracy::run_screen_code(bytes, power_on, frames)?),
        Check::Screen => verdict(accuracy::run_screen(bytes, power_on, frames)?),
        Check::Nestest => {
            let log = log.context("nestest needs its log")?;
            let r = accuracy::run_nestest(bytes, log, power_on)?;
            let mismatch = r.mismatch.as_ref().map(|(line, want, got)| {
                format!("first mismatch at log line {line}: expected {want}, got {got}")
            });
            let official = if r.official_passed() {
                format!("{} log lines match", r.official_lines)
            } else {
                mismatch.clone().unwrap_or_else(|| {
                    format!(
                        "log matches, but nestest reports code {:02X}h at $02",
                        r.codes[0]
                    )
                })
            };
            let all = if r.all_passed() {
                format!("{} log lines match", r.total_lines)
            } else {
                mismatch.unwrap_or_else(|| {
                    format!(
                        "log matches, but nestest reports {:02X}h at $02 and {:02X}h at $03",
                        r.codes[0], r.codes[1]
                    )
                })
            };
            vec![
                Checked {
                    name: "official opcodes".into(),
                    passed: r.official_passed(),
                    detail: official,
                    frames: None,
                },
                Checked {
                    name: "all opcodes".into(),
                    passed: r.all_passed(),
                    detail: all,
                    frames: None,
                },
            ]
        }
    })
}

/// `gp testroms run`: runs the named suites (all by default) and reports
/// every ROM. Exits 1 on a failure that isn't a known one, a known failure
/// that now passes, or a download whose hash isn't its pin; a ROM that
/// can't be downloaded is skipped.
pub fn run(
    m: &Manifest,
    only: &[String],
    offline: bool,
    ram_init: RamInit,
    paths: &Paths,
) -> Result<Outcome> {
    for name in only {
        if !m.suites.iter().any(|s| &s.name == name) {
            bail!(
                "no suite named {name}; the suites are {}",
                m.suites
                    .iter()
                    .map(|s| s.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    }
    let dir = work_dir(paths);
    let mut rows = Vec::new();
    let mut wrong_hash = Vec::new();
    // Our own programs that aren't built: unlike a download, that's never
    // skipped quietly, since `just build-testroms` is all it takes.
    let mut not_built = Vec::new();
    for s in m
        .suites
        .iter()
        .filter(|s| only.is_empty() || only.contains(&s.name))
    {
        if ram_init != RamInit::Zeros && s.needs_zeroed_ram {
            rows.extend(s.roms.iter().map(|rom| Row {
                suite: s.name.clone(),
                name: rom.path.clone(),
                status: Status::Skipped,
                detail: ZEROED_RAM_ONLY.into(),
                frames: None,
            }));
            continue;
        }
        let log = match &s.log {
            Some(f) => match fetch(m, f, &dir, offline)? {
                Fetched::Ready(b) => Some(Ok(String::from_utf8_lossy(&b).into_owned())),
                Fetched::Missing(why) => Some(Err(why)),
                Fetched::WrongHash(why) => {
                    wrong_hash.push(why.clone());
                    Some(Err(why))
                }
            },
            None => None,
        };
        for rom in &s.roms {
            let fetched = match &log {
                Some(Err(why)) => Fetched::Missing(format!("its log: {why}")),
                _ => fetch(
                    m,
                    &File {
                        path: rom.path.clone(),
                        sha256: rom.sha256.clone(),
                        built_from: rom.built_from.clone(),
                    },
                    &dir,
                    offline,
                )?,
            };
            let skip = match fetched {
                Fetched::Ready(b) => Ok(b),
                Fetched::Missing(why) => {
                    if rom.built_from.is_some() {
                        not_built.push(rom.path.clone());
                    }
                    Err(why)
                }
                Fetched::WrongHash(why) => {
                    wrong_hash.push(why.clone());
                    Err(why)
                }
            };
            let bytes = match skip {
                Ok(b) => b,
                Err(why) => {
                    rows.push(Row {
                        suite: s.name.clone(),
                        name: rom.path.clone(),
                        status: Status::Skipped,
                        detail: why,
                        frames: None,
                    });
                    continue;
                }
            };
            let log = log
                .as_ref()
                .and_then(|l| l.as_ref().ok())
                .map(String::as_str);
            let results = {
                let _s = tracing::info_span!("testroms.rom").entered();
                run_rom(s, rom, &bytes, log, ram_init)?
            };
            for Checked {
                name,
                passed,
                detail,
                frames,
            } in results
            {
                let known_way = rom
                    .known_result
                    .as_deref()
                    .is_some_and(|start| detail.starts_with(start));
                let status = match (passed, rom.known_failure.is_some()) {
                    (true, false) => Status::Passed,
                    (true, true) => Status::Fixed,
                    (false, true) if known_way => Status::Known,
                    (false, _) => Status::Failed,
                };
                let detail = match (&rom.known_failure, status) {
                    (Some(why), Status::Failed) => format!(
                        "{detail}; it has a known failure ({why}), but this isn't it: expected a result starting {:?}",
                        rom.known_result.as_deref().unwrap_or("")
                    ),
                    (Some(why), Status::Known) => format!("{detail} [{why}]"),
                    (Some(_), Status::Fixed) => format!(
                        "{detail}; it passes now, so remove its known_failure from test-roms/suites.toml"
                    ),
                    _ => detail,
                };
                rows.push(Row {
                    suite: s.name.clone(),
                    name,
                    status,
                    detail,
                    frames,
                });
            }
        }
    }
    Ok(report(m, rows, wrong_hash, &not_built, ram_init))
}

fn report(
    m: &Manifest,
    rows: Vec<Row>,
    wrong_hash: Vec<String>,
    not_built: &[String],
    ram_init: RamInit,
) -> Outcome {
    // A run with RAM powered on as all ones keeps its own history, apart
    // from the baseline's.
    let (prefix, ram) = match ram_init {
        RamInit::Ones => ("testroms.ram_ones", ", RAM powered on as all ones"),
        _ => ("testroms", ""),
    };
    let mut lines = vec![format!(
        "test ROMs from {} at {}{ram}",
        m.source.repo,
        &m.source.commit[..12]
    )];
    let mut suites = Vec::new();
    let count =
        |rows: &[&Row], st: &[Status]| rows.iter().filter(|r| st.contains(&r.status)).count();
    let all: Vec<&Row> = rows.iter().collect();
    for s in &m.suites {
        let mine: Vec<&Row> = rows.iter().filter(|r| r.suite == s.name).collect();
        if mine.is_empty() {
            continue;
        }
        let passed = count(&mine, &[Status::Passed, Status::Fixed]);
        let skipped = count(&mine, &[Status::Skipped]);
        let total = mine.len() - skipped;
        lines.push(format!(
            "{}: {passed} of {total} passed{}",
            s.name,
            if skipped > 0 {
                format!(", {skipped} skipped")
            } else {
                String::new()
            }
        ));
        for r in &mine {
            lines.push(format!(
                "  {:<7}{:<24} {}",
                r.status.mark(),
                r.name,
                r.detail
            ));
        }
        // Only suites that ran record a pass rate, so a skipped (offline)
        // suite leaves a gap in the history instead of a false zero.
        if total > 0 {
            crate::runrec::metric(
                format!("{prefix}.{}.passed", s.name),
                passed as f64,
                "count",
            );
            crate::runrec::metric(format!("{prefix}.{}.total", s.name), total as f64, "count");
        }
        suites.push(json!({
            "name": s.name,
            "about": s.about,
            "passed": passed,
            "total": total,
            "skipped": skipped,
            "roms": mine.iter().map(|r| json!({
                "name": r.name,
                "result": format!("{:?}", r.status).to_lowercase(),
                "detail": r.detail,
                "frames": r.frames,
            })).collect::<Vec<Value>>(),
        }));
    }
    let passed = count(&all, &[Status::Passed, Status::Fixed]);
    let known = count(&all, &[Status::Known]);
    let failed = count(&all, &[Status::Failed]);
    let fixed = count(&all, &[Status::Fixed]);
    let skipped = count(&all, &[Status::Skipped]);
    let total = rows.len() - skipped;
    if total > 0 {
        crate::runrec::metric(format!("{prefix}.passed"), passed as f64, "count");
        crate::runrec::metric(format!("{prefix}.total"), total as f64, "count");
    }
    crate::runrec::metric(
        format!("{prefix}.unexpected"),
        (failed + fixed) as f64,
        "count",
    );
    crate::runrec::metric(format!("{prefix}.skipped"), skipped as f64, "count");
    lines.push(format!(
        "{passed} of {total} passed, {known} known failures, {failed} unexpected failures, {fixed} fixed, {skipped} skipped"
    ));
    let not_downloaded = all
        .iter()
        .filter(|r| r.status == Status::Skipped && r.detail != ZEROED_RAM_ONLY)
        .count();
    if not_downloaded > 0 && wrong_hash.is_empty() {
        lines.push(format!(
            "{not_downloaded} skipped: not downloaded or built. `gp testroms fetch` downloads them, and `just build-testroms` builds our own (docs/test-roms.md)."
        ));
    }
    for w in &wrong_hash {
        lines.push(format!("WRONG HASH {w}"));
    }
    // Nothing ran at all (every ROM missing) is not a pass.
    let exit = if total == 0 {
        lines.push(
            "No test ROM ran, so nothing was checked. `gp testroms fetch` downloads them.".into(),
        );
        2
    } else if failed + fixed > 0 || !wrong_hash.is_empty() {
        1
    } else if !not_built.is_empty() {
        lines.push(format!(
            "Not built: {}. Run `just build-testroms`, then run again.",
            not_built.join(", ")
        ));
        2
    } else {
        0
    };
    Outcome {
        json: json!({
            "repo": m.source.repo,
            "commit": m.source.commit,
            "ram_init": if ram.is_empty() { "zeros" } else { "ones" },
            "suites": suites,
            "passed": passed,
            "total": total,
            "known_failures": known,
            "unexpected_failures": failed,
            "fixed": fixed,
            "skipped": skipped,
            "wrong_hash": wrong_hash,
            "not_built": not_built,
        }),
        text: lines.join("\n"),
        exit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_committed_manifest_is_valid() {
        let m = Manifest::load(None).unwrap();
        assert!(m.suites.iter().any(|s| s.check == Check::Nestest));
        let mut names: Vec<_> = m.suites.iter().map(|s| &s.name).collect();
        names.dedup();
        assert_eq!(names.len(), m.suites.len(), "suite names are unique");
        for s in &m.suites {
            assert!(!s.roms.is_empty(), "{}", s.name);
        }
        assert!(
            m.url("other/nestest.nes")
                .starts_with("https://raw.githubusercontent.com/christopherpow/nes-test-roms/")
        );
    }

    #[test]
    fn refuses_unsafe_paths_and_floating_pins() {
        let base = |commit: &str, path: &str| {
            format!(
                "[source]\nrepo = \"a/b\"\ncommit = \"{commit}\"\n[[suite]]\nname = \"s\"\ncheck = \"blargg\"\nabout = \"x\"\n[[suite.rom]]\npath = \"{path}\"\nsha256 = \"{}\"\n",
                "0".repeat(64)
            )
        };
        let load = |text: String| {
            let dir = tempfile::tempdir().unwrap();
            let p = dir.path().join("m.toml");
            fs::write(&p, text).unwrap();
            Manifest::load(Some(&p))
        };
        let sha = "0".repeat(40);
        assert!(load(base(&sha, "dir/x.nes")).is_ok());
        assert!(load(base("master", "dir/x.nes")).is_err());
        assert!(load(base(&sha, "../x.nes")).is_err());
        assert!(load(base(&sha, "/x.nes")).is_err());
        assert!(load(base(&sha, "a//x.nes")).is_err());
        // Our own programs name their source under test-roms/.
        let built = |source: &str| format!("{}built_from = \"{source}\"\n", base(&sha, "gp/x.nes"));
        assert!(load(built("test-roms/x/x.s")).is_ok());
        assert!(load(built("../x.s")).is_err());
        assert!(load(built("crates/x.s")).is_err());
        // A known failure must say which result it excuses.
        let known = |extra: &str| format!("{}{extra}", base(&sha, "dir/x.nes"));
        assert!(load(known("known_failure = \"why\"\n")).is_err());
        assert!(load(known("known_result = \"code 2:\"\n")).is_err());
        assert!(
            load(known(
                "known_failure = \"why\"\nknown_result = \"code 2:\"\n"
            ))
            .is_ok()
        );
    }

    #[test]
    fn our_own_programs_have_their_source_in_the_repo() {
        let m = Manifest::load(None).unwrap();
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let built: Vec<_> = m
            .suites
            .iter()
            .flat_map(|s| &s.roms)
            .filter_map(|r| r.built_from.as_deref())
            .collect();
        assert!(!built.is_empty());
        for source in built {
            assert!(repo.join(source).is_file(), "{source}");
            let cfg = source.strip_suffix(".s").unwrap().to_owned() + ".cfg";
            assert!(repo.join(&cfg).is_file(), "{cfg}");
        }
    }

    #[test]
    fn checks_region_names() {
        let manifest = |region: &str| {
            format!(
                "[source]\nrepo = \"a/b\"\ncommit = \"{}\"\n[[suite]]\nname = \"s\"\ncheck = \"blargg-2005\"\nabout = \"x\"\nregion = \"{region}\"\n[[suite.rom]]\npath = \"x.nes\"\nsha256 = \"{}\"\n",
                "0".repeat(40),
                "0".repeat(64)
            )
        };
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("m.toml");
        fs::write(&p, manifest("pal")).unwrap();
        assert!(Manifest::load(Some(&p)).is_ok());
        fs::write(&p, manifest("secam")).unwrap();
        assert!(Manifest::load(Some(&p)).is_err());
    }

    /// Our own programs are never downloaded: missing, they're reported as
    /// not built; built with another hash, they're never used.
    #[test]
    fn a_built_program_is_never_downloaded() {
        let m = Manifest::load(None).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let file = File {
            path: "gp/x.nes".into(),
            sha256: sha256(b"right"),
            built_from: Some("test-roms/x/x.s".into()),
        };
        match fetch(&m, &file, dir.path(), false).unwrap() {
            Fetched::Missing(why) => assert!(why.contains("just build-testroms"), "{why}"),
            _ => panic!("a program that isn't built is missing"),
        }
        fs::create_dir_all(dir.path().join("gp")).unwrap();
        fs::write(dir.path().join("gp/x.nes"), b"wrong").unwrap();
        assert!(matches!(
            fetch(&m, &file, dir.path(), false).unwrap(),
            Fetched::WrongHash(_)
        ));
        fs::write(dir.path().join("gp/x.nes"), b"right").unwrap();
        assert!(matches!(
            fetch(&m, &file, dir.path(), false).unwrap(),
            Fetched::Ready(b) if b == b"right"
        ));
    }

    /// A missing download is skipped, but our own program missing means
    /// `just build-testroms` didn't run: the run couldn't check it, exit 2.
    #[test]
    fn a_missing_built_program_exits_2() {
        let m = Manifest::load(None).unwrap();
        let home = tempfile::tempdir().unwrap();
        let paths = Paths {
            home: home.path().to_owned(),
            store: home.path().join("store"),
            roms: home.path().join("roms"),
        };
        let out = run(&m, &["mmc1_banking".into()], true, RamInit::Zeros, &paths).unwrap();
        assert_eq!(out.exit, 2);
        assert!(out.text.contains("just build-testroms"), "{}", out.text);
    }

    /// nestest's log was made with RAM zeroed, so a run with RAM all ones
    /// skips it, says why, and doesn't call it a missing download.
    #[test]
    fn a_ram_ones_run_skips_what_needs_zeroed_ram() {
        let m = Manifest::load(None).unwrap();
        let home = tempfile::tempdir().unwrap();
        let paths = Paths {
            home: home.path().to_owned(),
            store: home.path().join("store"),
            roms: home.path().join("roms"),
        };
        let out = run(&m, &["nestest".into()], true, RamInit::Ones, &paths).unwrap();
        assert!(out.text.contains(ZEROED_RAM_ONLY), "{}", out.text);
        assert!(!out.text.contains("not downloaded"), "{}", out.text);
        assert_eq!(out.exit, 2, "nothing ran");
    }

    /// nestest through the instruction tracer that `gp trace instructions`
    /// uses, line by line against nestest.log: the address, bytes and
    /// disassembly, then the registers, PPU position and cycle count. It
    /// needs the downloaded test ROMs (`gp testroms fetch`, docs/test-roms.md)
    /// and reads them from `$GP_HOME/work/test-roms` without downloading;
    /// without them it skips with a notice, like `gp testroms run`.
    #[test]
    fn the_instruction_tracer_matches_nestest_log() {
        let m = Manifest::load(None).unwrap();
        let suite = m.suites.iter().find(|s| s.check == Check::Nestest).unwrap();
        let dir = work_dir(&Paths::from_env().unwrap());
        let mut files = Vec::new();
        for file in suite.files() {
            match fetch(&m, &file, &dir, true).unwrap() {
                Fetched::Ready(bytes) => files.push(bytes),
                Fetched::Missing(why) | Fetched::WrongHash(why) => {
                    eprintln!(
                        "skipping the nestest trace comparison: {} ({why}); `gp testroms fetch` downloads it",
                        file.path
                    );
                    return;
                }
            }
        }
        let [rom, log] = &files[..] else {
            panic!("nestest is one ROM and its log");
        };
        let log = std::str::from_utf8(log).unwrap();
        let r = accuracy::run_nestest(rom, log, PowerOn::default()).unwrap();
        assert!(r.all_passed(), "{:?}", r.mismatch);
        assert_eq!(r.matched, r.total_lines);
    }
}
