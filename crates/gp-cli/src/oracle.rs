//! `gp oracle`: run recordings in Mesen 2 and compare them with `gp-nes`
//! (Milestone 1, step 5; docs/oracle.md).
//!
//! Mesen runs in its own Docker image (docker/mesen/Dockerfile, built by
//! `just mesen-build`), in its test runner mode, with
//! scripts/oracle/capture.lua. This module writes the script, the input log
//! and a config file into a work directory, mounts the ROM read-only, and
//! turns what the script writes into a recording ([`gp_oracle`]).
//!
//! Ported from game-printer's oracle.rs. What gameprinter's layout changed:
//! the run recorder is the process's (crate::runrec); the triage file is
//! gameprinter's oracle/triage.toml, cut to gp-testrom's records (the other
//! games' are gone with them); and an arcade recording goes to the MAME
//! oracle instead of Mesen (crate::oracle_mame): `record` runs it in MAME's
//! image and `diff` compares it with a MAME capture frame by frame.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use serde_json::json;

use gp_evidence::manifest::{RecordingManifest, RuntimeRef, SourceRef};
use gp_evidence::recordings::IndexEntry;
use gp_evidence::source::identify_nes;
use gp_evidence::{Hash, InputLog, Paths, Store};
use gp_oracle::capture::Capture;
use gp_oracle::diff::{self, Report};
use gp_oracle::mesen;
use gp_oracle::states::{self, States};
use gp_oracle::triage::{self, Record, Triage, Verdict};

use crate::Outcome;
use crate::open::{RomFolder, now_rfc3339};

fn resolve_recording(store: &Store, prefix: &str) -> Result<IndexEntry> {
    Ok(gp_evidence::recordings::resolve(store, prefix)?)
}

/// The capture script, built in so `gp` needs no checkout to run it.
const CAPTURE_LUA: &str = include_str!("../../../scripts/oracle/capture.lua");

#[derive(Subcommand)]
pub enum OracleCmd {
    /// Run a recording's input log from power-on in Mesen 2, in its Docker
    /// image, and store what Mesen did as a recording of its own (runtime
    /// `mesen2`). Needs `just mesen-build` first. An arcade recording runs
    /// in MAME instead (`just mame-build`), its capture kept under
    /// $GP_HOME/work/oracle/ (docs/oracle-mame.md).
    Record {
        /// The gp-nes (or gp-arcade) recording whose ROM, power-on state
        /// and input to run.
        id: String,
        /// linux/arm64 or linux/amd64 (default: this machine's).
        #[arg(long)]
        platform: Option<String>,
        /// A ROM file to mount, when the recording's isn't in $GP_ROM_DIR.
        #[arg(long)]
        rom: Option<PathBuf>,
        /// Don't capture events (NMIs, controller and register accesses).
        #[arg(long)]
        no_events: bool,
    },
    /// Compare two recordings field by field. Each divergence is reported
    /// with the fields that differ and its classes, then the diff resyncs
    /// and looks for the next. A gp-nes recording as A runs live, so the
    /// resync copies B's values into it. Stores the report in the store.
    /// Exits 1 if anything differs. An arcade recording as A is played on
    /// gp-arcade and compared frame by frame with a MAME capture as B (its
    /// RAM, input ports, picture hash and AY writes).
    Diff {
        /// Recording A, normally the gp-nes one.
        a: String,
        /// Recording B, normally the Mesen one; for an arcade A, a MAME
        /// capture folder under $GP_HOME/work/oracle/ or its path.
        b: String,
        /// A's ROM file, instead of finding it in $GP_ROM_DIR by hash.
        #[arg(long)]
        rom: Option<PathBuf>,
        /// Compare the two traces only, without running A (no resync by
        /// copying).
        #[arg(long)]
        trace_only: bool,
        /// Stop after this many divergences.
        #[arg(long, default_value_t = 40)]
        max: usize,
        /// Which triage to read (default: oracle/triage.toml in the checkout).
        #[arg(long)]
        triage: Option<PathBuf>,
    },
    /// Set the triage of a divergence class in oracle/triage.toml.
    Triage {
        /// The class, named as the diff reports it (a field, frame-shift,
        /// lag or events.<kind>).
        class: String,
        /// fork-bug, mesen-difference or accepted-configuration.
        #[arg(value_parser = ["fork-bug", "mesen-difference", "accepted-configuration"])]
        verdict: String,
        /// The evidence: what differs and why.
        #[arg(long)]
        note: String,
        /// A source (registry ID, such as smb1-ntsc-prg0) the record applies
        /// to; repeat for more. Without it, any source.
        #[arg(long = "source")]
        sources: Vec<String>,
        /// The frames it applies to: `N` or `A..=B`. Without it, any frame.
        #[arg(long)]
        frames: Option<String>,
        /// The file to write, which may be new (default: the existing
        /// oracle/triage.toml in the checkout).
        #[arg(long)]
        file: Option<PathBuf>,
    },
    /// Mark a gp-nes recording golden after diffing it against its Mesen
    /// recording. Every class must be triaged and none a fork bug; then the
    /// verdict is stored with the triage records it relied on, and the
    /// recordings log gets a golden entry. Exits 1 if it can't be golden.
    Golden {
        /// The gp-nes recording.
        id: String,
        /// The Mesen recording of the same run.
        #[arg(long)]
        against: String,
        /// The gp-nes recording's ROM, if it isn't in $GP_ROM_DIR.
        #[arg(long)]
        rom: Option<PathBuf>,
        /// Where the triage is kept (default: oracle/triage.toml in the checkout).
        #[arg(long)]
        triage: Option<PathBuf>,
    },
    /// Record baselines in gp-nes and in Mesen (on each platform), diff
    /// them and check every class is triaged, all in a temporary store.
    /// `just check-mesen` runs it. Known fork bugs are listed but don't
    /// fail it. Baselines without their ROM in $GP_ROM_DIR are skipped. Exits
    /// 1 if a class isn't triaged or two platforms' Mesen runs differ.
    Check {
        /// Baseline files or directories.
        #[arg(required = true)]
        baselines: Vec<PathBuf>,
        /// linux/arm64 or linux/amd64; repeat for both (default: this
        /// machine's). Every platform must give the same Mesen recording.
        #[arg(long = "platform")]
        platforms: Vec<String>,
        /// The triage to check classes against (default: oracle/triage.toml in the checkout).
        #[arg(long)]
        triage: Option<PathBuf>,
    },
}

pub fn run(cmd: &OracleCmd, paths: &Paths) -> Result<Outcome> {
    match cmd {
        OracleCmd::Record {
            id,
            platform,
            rom,
            no_events,
        } => record(id, platform.as_deref(), rom.as_deref(), !no_events, paths),
        OracleCmd::Diff {
            a,
            b,
            rom,
            trace_only,
            max,
            triage,
        } => diff_cmd(
            a,
            b,
            rom.as_deref(),
            *trace_only,
            *max,
            triage.as_deref(),
            paths,
        ),
        OracleCmd::Triage {
            class,
            verdict,
            note,
            sources,
            frames,
            file,
        } => set_triage(
            class,
            verdict,
            note,
            sources,
            frames.as_deref(),
            file.as_deref(),
        ),
        OracleCmd::Golden {
            id,
            against,
            rom,
            triage,
        } => golden(id, against, rom.as_deref(), triage.as_deref(), paths),
        OracleCmd::Check {
            baselines,
            platforms,
            triage,
        } => check(baselines, platforms, triage.as_deref(), paths),
    }
}

/// `arm64` or `amd64`, from `linux/<arch>` or this machine.
fn arch(platform: Option<&str>) -> Result<&'static str> {
    match platform {
        None => Ok(if cfg!(target_arch = "aarch64") {
            "arm64"
        } else {
            "amd64"
        }),
        Some("linux/arm64") => Ok("arm64"),
        Some("linux/amd64") => Ok("amd64"),
        Some(other) => bail!("platform {other:?} isn't linux/arm64 or linux/amd64"),
    }
}

/// The triage file: the one given, or the nearest oracle/triage.toml in
/// the current directory or above it. A file that isn't there is an error
/// (exit 2), never a new file in some other directory.
fn triage_path(given: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = given {
        return Ok(p.to_path_buf());
    }
    let cwd = std::env::current_dir()?;
    cwd.ancestors()
        .map(|dir| dir.join(triage::FILE))
        .find(|p| p.is_file())
        .with_context(|| {
            format!(
                "no {} in {} or above it; give --triage (or --file)",
                triage::FILE,
                cwd.display()
            )
        })
}

/// A recording's ROM: its bytes, and the file they came from (named in
/// messages and given to the diff). Our test ROM is built in; its file is
/// written into `work`.
struct Rom {
    bytes: Vec<u8>,
    path: PathBuf,
}

fn find_rom(sha256: &str, given: Option<&Path>, paths: &Paths, work: &Path) -> Result<Option<Rom>> {
    let testrom = gp_nes::testrom::build();
    if identify_nes(&testrom)?.sha256 == sha256 {
        let path = work.join("gp-testrom.nes");
        fs::write(&path, &testrom)?;
        return Ok(Some(Rom {
            bytes: testrom,
            path,
        }));
    }
    let path = match given {
        Some(p) => p.to_path_buf(),
        // The file, found by gp-cli's one lookup by hash.
        None => match RomFolder::new(paths).find(sha256)? {
            Some(p) => p.to_path_buf(),
            None => return Ok(None),
        },
    };
    let bytes = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    if identify_nes(&bytes)?.sha256 != sha256 {
        bail!(
            "{} isn't the ROM the recording was made from (SHA-256 {}...)",
            path.display(),
            &sha256[..16.min(sha256.len())]
        );
    }
    Ok(Some(Rom {
        bytes,
        path: fs::canonicalize(&path)?,
    }))
}

fn work_dir(paths: &Paths) -> Result<tempfile::TempDir> {
    let parent = paths.home.join("work").join("oracle");
    fs::create_dir_all(&parent)?;
    tempfile::Builder::new()
        .prefix("run-")
        .tempdir_in(&parent)
        .with_context(|| format!("creating a work directory in {}", parent.display()))
}

/// A model recording's is empty: it has no ROM, so no ROM is found for it.
fn sha_of(manifest: &RecordingManifest) -> &str {
    manifest.source.nes_sha256().unwrap_or_default()
}

/// `--user uid:gid` of the work directory's owner, on Unix.
fn user(work: &Path) -> Result<Vec<String>> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let m = fs::metadata(work)?;
        Ok(vec!["--user".into(), format!("{}:{}", m.uid(), m.gid())])
    }
    #[cfg(not(unix))]
    {
        let _ = work;
        Ok(Vec::new())
    }
}

/// What one Mesen run stored.
struct MesenRun {
    id: Hash,
    manifest: RecordingManifest,
    notes: Vec<String>,
    seconds: f64,
}

/// Runs `input` from power-on in Mesen and stores the result in `store`.
#[allow(clippy::too_many_arguments)]
fn run_mesen(
    store: &Store,
    gp: &RecordingManifest,
    gp_name: &str,
    source_id: Option<String>,
    input: &InputLog,
    rom: &Rom,
    arch: &str,
    events: bool,
    paths: &Paths,
) -> Result<MesenRun> {
    // A fresh directory for each run, so one run's capture can never be
    // read as another's.
    let work_dir = work_dir(paths)?;
    let work = work_dir.path();
    let image = mesen::image(arch);
    let inspect = Command::new("docker")
        .args(["image", "inspect", "--format", "{{.Id}}", &image])
        .output()
        .context("running docker; is it installed and running?")?;
    if !inspect.status.success() {
        bail!(
            "the Mesen image {image} isn't built; run `just mesen-build linux/{arch}` (docs/oracle.md)"
        );
    }
    let switches = mesen::switches(&gp.power_on)
        .map_err(|e| anyhow::anyhow!("Mesen can't run this recording's power-on state: {e}"))?;
    let (input_dir, out_dir) = (work.join("in"), work.join("out"));
    fs::create_dir_all(&input_dir)?;
    fs::create_dir_all(&out_dir)?;
    fs::write(input_dir.join("input.log"), input.to_text())?;
    fs::write(
        input_dir.join("config"),
        format!(
            "frames {}\nevents {}\nram_fill {}\n",
            input.frames.len(),
            u8::from(events),
            if gp.power_on.ram_init == gp_evidence::manifest::RamInit::Fceux {
                "fceux"
            } else {
                "none"
            }
        ),
    )?;
    fs::write(input_dir.join("capture.lua"), CAPTURE_LUA)?;
    // Mesen gets the image gp-nes boots (`board::runnable`: junk around the
    // game data dropped, a registered source's header from its board facts),
    // so a junk or nonstandard header can't make the two boot differently.
    let runnable = gp_nes::board::runnable(&rom.bytes).map_err(|e| {
        anyhow::anyhow!("can't make a runnable image of {}: {e}", rom.path.display())
    })?;
    fs::write(input_dir.join("rom.nes"), &runnable)?;
    // About 100 frames a second natively; give emulation plenty.
    let timeout = 120 + input.frames.len();
    let mount = |from: &Path, to: &str, ro: bool| {
        format!(
            "type=bind,source={},target={to}{}",
            from.display(),
            if ro { ",readonly" } else { "" }
        )
    };
    let start = Instant::now();
    let output = Command::new("docker")
        .args(["run", "--rm", "--platform", &format!("linux/{arch}")])
        // Mesen needs no network, and runs as the caller so what it writes
        // into the work directory is the caller's.
        .args(["--network", "none"])
        .args(user(work)?)
        .args(["--mount", &mount(&input_dir, "/gp/in", true)])
        .args(["--mount", &mount(&out_dir, "/gp/out", false)])
        .arg(&image)
        .arg("--testRunner")
        .arg(format!("--timeout={timeout}"))
        .arg("--debug.scriptWindow.allowIoOsAccess=true")
        .args(&switches)
        .args(["/gp/in/capture.lua", "/gp/in/rom.nes"])
        .output()
        .context("running docker")?;
    let seconds = start.elapsed().as_secs_f64();
    let code = output.status.code();
    if code != Some(0) {
        let reason = fs::read_to_string(out_dir.join("error.txt")).unwrap_or_default();
        let stderr = String::from_utf8_lossy(&output.stderr);
        let tail: Vec<&str> = stderr.lines().rev().take(5).collect();
        bail!(
            "Mesen exited with {} after {seconds:.0} s{}{}",
            code.map_or_else(|| "a signal".into(), |c| format!("code {c}")),
            if reason.trim().is_empty() {
                String::new()
            } else {
                format!(": {}", reason.trim())
            },
            if tail.is_empty() {
                String::new()
            } else {
                format!(
                    "\n{}",
                    tail.into_iter().rev().collect::<Vec<_>>().join("\n")
                )
            }
        );
    }
    let text = fs::read_to_string(out_dir.join("capture.txt"))
        .context("reading the capture Mesen wrote")?;
    let capture = Capture::parse(&text, input.frames.len() as u64)?;
    let region = gp.power_on.region;
    let want = match region {
        gp_evidence::manifest::Region::Ntsc => "Ntsc",
        gp_evidence::manifest::Region::Pal => "Pal",
    };
    if capture.region != want {
        bail!("Mesen ran the {} region, not {want}", capture.region);
    }
    let mut states_list = Vec::with_capacity(capture.frames.len());
    let mut events_list = Vec::with_capacity(capture.frames.len());
    for frame in &capture.frames {
        states_list.push(mesen::encode(frame, region)?);
        events_list.push(mesen::events(frame, region).map_err(anyhow::Error::msg)?);
    }
    let mut observed = input.clone();
    for (f, frame) in observed.frames.iter_mut().zip(&capture.frames[1..]) {
        f.polled = frame.polled;
    }
    let notes = mesen::notes(&gp.power_on);
    let (id, manifest) = states::store(
        store,
        states::Made {
            source: gp.source.clone(),
            source_id,
            runtime: RuntimeRef {
                name: mesen::RUNTIME.into(),
                version: mesen::VERSION.into(),
                commit: Some(mesen::COMMIT.into()),
            },
            power_on: gp.power_on,
            input: &observed,
            provenance: gp.provenance.clone(),
            name: format!("{gp_name} (mesen)"),
            created_at: now_rfc3339(),
            notes: (!notes.is_empty()).then(|| notes.join("; ")),
        },
        &States {
            table: mesen::field_table(),
            states: states_list,
        },
        events.then_some(&events_list[..]),
    )?;
    Ok(MesenRun {
        id,
        manifest,
        notes,
        seconds,
    })
}

struct GpRecording {
    entry: IndexEntry,
    manifest: RecordingManifest,
}

fn open(store: &Store, prefix: &str) -> Result<GpRecording> {
    let entry = resolve_recording(store, prefix)?;
    let manifest = gp_evidence::recordings::load_manifest(store, &entry.id)?;
    if let SourceRef::Model { name, .. } = &manifest.source {
        bail!(
            "{} is a run of the model `{name}` in {}, which has no ROM; the oracle compares ROM recordings",
            &entry.id.to_hex()[..12],
            manifest.runtime.name
        );
    }
    Ok(GpRecording { entry, manifest })
}

fn record(
    id: &str,
    platform: Option<&str>,
    rom: Option<&Path>,
    events: bool,
    paths: &Paths,
) -> Result<Outcome> {
    let arch = arch(platform)?;
    let store = Store::open(&paths.store)?;
    let gp = open(&store, id)?;
    if crate::oracle_mame::is_arcade(&gp.manifest) {
        return crate::oracle_mame::record(&store, &gp.entry, &gp.manifest, arch, rom, paths);
    }
    if gp.manifest.runtime.name == mesen::RUNTIME {
        bail!("{id} is a Mesen recording already; give the gp-nes recording it came from");
    }
    let input = states::input_log(&store, &gp.manifest)?;
    let work = work_dir(paths)?;
    let Some(rom) = find_rom(sha_of(&gp.manifest), rom, paths, work.path())? else {
        bail!(
            "no ROM with SHA-256 {} in {}; give --rom",
            sha_of(&gp.manifest),
            paths.roms.display()
        );
    };
    let run = run_mesen(
        &store,
        &gp.manifest,
        &gp.entry.name,
        gp.entry.source.clone(),
        &input,
        &rom,
        arch,
        events,
        paths,
    )?;
    crate::runrec::output("recording", format!("blake3:{}", run.id), None);
    crate::runrec::metric("oracle.record.frames", run.manifest.frames as f64, "count");
    crate::runrec::metric("oracle.record.seconds", run.seconds, "s");
    let mut text = format!(
        "ran {} frames in Mesen {} ({}, linux/{arch}) in {:.0} s\nid: {}",
        run.manifest.frames,
        mesen::VERSION,
        &mesen::COMMIT[..12],
        run.seconds,
        run.id
    );
    for n in &run.notes {
        text.push_str(&format!("\nnote: {n}"));
    }
    text.push_str(&format!(
        "\ncompare: gp oracle diff {} {}",
        &gp.entry.id.to_hex()[..12],
        &run.id.to_hex()[..12]
    ));
    Ok(Outcome::ok(
        json!({ "id": run.id, "from": gp.entry.id, "platform": format!("linux/{arch}"), "frames": run.manifest.frames, "seconds": run.seconds, "notes": run.notes }),
        text,
    ))
}

/// Diffs two stored recordings, live when A is this runtime's and its ROM
/// is found.
fn diff_recordings(
    store: &Store,
    a: &GpRecording,
    b: &GpRecording,
    rom: Option<&Path>,
    trace_only: bool,
    max: usize,
    paths: &Paths,
) -> Result<(Report, Option<String>)> {
    let sa = states::load(store, &a.manifest)?;
    let sb = states::load(store, &b.manifest)?;
    let ia = states::input_log(store, &a.manifest)?;
    let ib = states::input_log(store, &b.manifest)?;
    let mut note = None;
    let work = work_dir(paths)?;
    let rom = if trace_only {
        None
    } else if !gp_nes::recording::made_by_this_runtime(&a.manifest) {
        note = Some("A wasn't made by this gp-nes, so the diff reads its trace and resyncs by setting fields aside".to_owned());
        None
    } else {
        let found = find_rom(sha_of(&a.manifest), rom, paths, work.path())?;
        if found.is_none() {
            note = Some(format!(
                "A's ROM isn't in {}, so the diff reads A's trace and resyncs by setting fields aside",
                paths.roms.display()
            ));
        }
        found
    };
    let live = rom.as_ref().map(|r| diff::LiveA {
        rom: &r.bytes,
        power_on: a.manifest.power_on,
        input: &ia,
    });
    let mut report = diff::diff(&sa, &sb, live, max)?;
    diff::add_lag(&mut report, &ia, &ib);
    if a.manifest.events.is_some() && b.manifest.events.is_some() {
        let ea = gp_nes::recording::events_by_frame(store, &a.manifest)?;
        let eb = gp_nes::recording::events_by_frame(store, &b.manifest)?;
        diff::add_events(&mut report, &ea, &eb);
    }
    Ok((report, note))
}

fn describe_runtime(r: &RuntimeRef) -> String {
    format!("{} {}", r.name, r.version)
}

fn report_text(
    a: &GpRecording,
    b: &GpRecording,
    report: &Report,
    check: &triage::Check,
    triage: &Triage,
) -> String {
    let mut out = vec![
        format!(
            "a: {} ({}), {}",
            a.entry.name,
            &a.entry.id.to_hex()[..12],
            describe_runtime(&a.manifest.runtime)
        ),
        format!(
            "b: {} ({}), {}",
            b.entry.name,
            &b.entry.id.to_hex()[..12],
            describe_runtime(&b.manifest.runtime)
        ),
        format!(
            "compared frames 0 to {} ({}), {} fields; not compared: {} only in a, {} only in b",
            report.frames,
            if report.mode == "live" {
                "a rerun in gp-nes"
            } else {
                "from both traces"
            },
            report.compared.len(),
            report.only_a.len(),
            report.only_b.len()
        ),
    ];
    if !report.only_a.is_empty() {
        out.push(format!("  only in a: {}", report.only_a.join(", ")));
    }
    if !report.only_b.is_empty() {
        out.push(format!("  only in b: {}", report.only_b.join(", ")));
    }
    match report.first() {
        None => out.push("no divergence: every compared field matches on every frame".into()),
        Some(d) => out.push(format!(
            "first divergence at frame {}: {}",
            d.frame,
            d.classes.join(", ")
        )),
    }
    for d in &report.divergences {
        let at = if d.frame == d.a_frame {
            format!("frame {}", d.frame)
        } else {
            format!("frame {} (a's {})", d.frame, d.a_frame)
        };
        out.push(format!("  {at}: {}", d.classes.join(", ")));
        for f in d.fields.iter().take(8) {
            out.push(format!("      {}: a {}  b {}", f.field, f.a, f.b));
        }
        if d.fields.len() > 8 {
            out.push(format!("      and {} more fields", d.fields.len() - 8));
        }
        if !d.resync.is_empty() {
            out.push(format!("      resync: {}", d.resync));
        }
    }
    if report.truncated {
        out.push(format!(
            "  stopped after {} divergences (--max)",
            report.divergences.len()
        ));
    }
    if !report.lag_frames.is_empty() {
        let shown: Vec<String> = report
            .lag_frames
            .iter()
            .take(8)
            .map(u64::to_string)
            .collect();
        out.push(format!(
            "lag: {} frames differ: {}{}",
            report.lag_frames.len(),
            shown.join(" "),
            if report.lag_frames.len() > 8 {
                " ..."
            } else {
                ""
            }
        ));
    }
    if !report.events.is_empty() {
        out.push(
            "events (each kind over the whole run; offsets are b's position minus a's, in dots):"
                .into(),
        );
        for e in &report.events {
            let offsets: Vec<String> = e
                .offsets
                .iter()
                .map(|(d, n)| format!("{d:+} x{n}"))
                .collect();
            out.push(format!(
                "  {:<17} a {:>5}  b {:>5}  {}  {} in another frame  offsets {}",
                e.kind,
                e.a_events,
                e.b_events,
                e.first_frame
                    .map_or_else(|| "same".to_owned(), |f| format!("differ from frame {f}")),
                e.other_frame,
                if offsets.is_empty() {
                    "none".into()
                } else {
                    offsets.join(", ")
                }
            ));
        }
    }
    if !report.classes.is_empty() {
        out.push("classes:".into());
        for c in &report.classes {
            let t = if check.untriaged.contains(&c.class) {
                "NOT TRIAGED".to_owned()
            } else {
                triage
                    .verdict_of(&c.class)
                    .unwrap_or_else(|| "NOT TRIAGED".into())
            };
            out.push(format!(
                "  {:<32} first at frame {:<5} {:<3} divergences  {t}",
                c.class, c.first_frame, c.divergences
            ));
        }
        out.push(format!(
            "{} classes: {} accepted, {} not triaged, {} fork bugs",
            report.classes.len(),
            check.accepted.len(),
            check.untriaged.len(),
            check.fork_bugs.len()
        ));
        for u in &check.uncovered {
            out.push(format!("  not triaged: {u}"));
        }
    }
    out.join("\n")
}

#[allow(clippy::too_many_arguments)]
fn diff_cmd(
    a: &str,
    b: &str,
    rom: Option<&Path>,
    trace_only: bool,
    max: usize,
    triage_file: Option<&Path>,
    paths: &Paths,
) -> Result<Outcome> {
    let store = Store::open(&paths.store)?;
    let a = open(&store, a)?;
    if crate::oracle_mame::is_arcade(&a.manifest) {
        let cap = crate::oracle_mame::capture_dir(b, paths).with_context(|| {
            format!(
                "{b} isn't a MAME capture: give a folder under {} (gp oracle record names it) or its path",
                paths.home.join("work").join("oracle").display()
            )
        })?;
        return crate::oracle_mame::diff(&store, &a.entry, &a.manifest, &cap, rom, max, paths);
    }
    let b = open(&store, b)?;
    let (report, note) = diff_recordings(&store, &a, &b, rom, trace_only, max, paths)?;
    let triage = Triage::load(&triage_path(triage_file)?)?;
    let check = triage.check(a.entry.source.as_deref(), class_frames(&report));
    let json = json!({
        "a": a.entry.id,
        "b": b.entry.id,
        "report": report,
        "triage": check,
        "note": note,
    });
    let stored = store.put_bytes(serde_json::to_string_pretty(&json)?.as_bytes())?;
    crate::runrec::output("diff report", format!("blake3:{stored}"), None);
    crate::runrec::metric(
        "oracle.diff.divergences",
        report.divergences.len() as f64,
        "count",
    );
    let mut text = report_text(&a, &b, &report, &check, &triage);
    if let Some(n) = &note {
        text.push_str(&format!("\nnote: {n}"));
    }
    text.push_str(&format!("\nreport: blake3:{stored} (in the store)"));
    let mut json = json;
    json["report_object"] = json!(stored);
    Ok(Outcome {
        json,
        text,
        exit: u8::from(!report.classes.is_empty()),
    })
}

/// Each class of a report with the frames it was found at.
fn class_frames(report: &Report) -> impl Iterator<Item = (&str, &[u64])> {
    report
        .classes
        .iter()
        .map(|c| (c.class.as_str(), c.frames.as_slice()))
}

fn today() -> String {
    now_rfc3339()[..10].to_owned()
}

/// `N` or `A..=B` as an inclusive frame range.
fn frame_range(text: &str) -> Result<[u64; 2]> {
    let num = |t: &str| {
        t.trim()
            .parse::<u64>()
            .with_context(|| format!("{t:?} in --frames {text} isn't a frame number"))
    };
    let range = match text.split_once("..=") {
        Some((a, b)) => [num(a)?, num(b)?],
        None => {
            let n = num(text)?;
            [n, n]
        }
    };
    if range[0] > range[1] {
        bail!("--frames {text} runs backwards");
    }
    Ok(range)
}

fn set_triage(
    class: &str,
    verdict: &str,
    note: &str,
    sources: &[String],
    frames: Option<&str>,
    file: Option<&Path>,
) -> Result<Outcome> {
    let (path, mut t) = match file {
        Some(p) => (p.to_path_buf(), Triage::load_or_new(p)?),
        None => {
            let p = triage_path(None)?;
            let t = Triage::load(&p)?;
            (p, t)
        }
    };
    let verdict = Verdict::from_name(verdict).expect("clap checks the verdict");
    let record = Record {
        class: class.to_owned(),
        verdict,
        sources: sources.to_vec(),
        frames: frames.map(frame_range).transpose()?,
        note: note.to_owned(),
        date: today(),
    };
    let scope = record.scope();
    let before = t
        .records
        .iter()
        .find(|r| r.class == record.class && r.scope() == scope)
        .map(|r| r.verdict);
    t.set(record);
    t.save(&path)?;
    let text = match before {
        Some(b) => format!(
            "{class} ({scope}): {} -> {} in {}",
            b.name(),
            verdict.name(),
            path.display()
        ),
        None => format!(
            "{class} ({scope}): {} in {}",
            verdict.name(),
            path.display()
        ),
    };
    Ok(Outcome::ok(
        json!({ "class": class, "verdict": verdict.name(), "scope": scope, "file": path }),
        text,
    ))
}

fn golden(
    id: &str,
    against: &str,
    rom: Option<&Path>,
    triage_file: Option<&Path>,
    paths: &Paths,
) -> Result<Outcome> {
    let store = Store::open(&paths.store)?;
    let a = open(&store, id)?;
    if crate::oracle_mame::is_arcade(&a.manifest) {
        bail!(
            "golden is the NES's: a gp-nes recording against its Mesen recording, every class triaged. An arcade recording is held to MAME with gp oracle diff ID CAPTURE; a MAME capture isn't a store recording, so no verdict is stored for it yet"
        );
    }
    let b = open(&store, against)?;
    if a.manifest.runtime.name == mesen::RUNTIME || b.manifest.runtime.name != mesen::RUNTIME {
        bail!("give the gp-nes recording first and its Mesen recording with --against");
    }
    // The input logs differ only in the polled flags each runtime saw.
    let pads = |log: InputLog| -> Vec<[u8; 4]> { log.frames.iter().map(|f| f.pads).collect() };
    if pads(states::input_log(&store, &a.manifest)?)
        != pads(states::input_log(&store, &b.manifest)?)
    {
        bail!("the two recordings didn't run the same input");
    }
    if a.manifest.source != b.manifest.source || a.manifest.power_on != b.manifest.power_on {
        bail!("the two recordings didn't run the same ROM and power-on state");
    }
    let (report, _) = diff_recordings(&store, &a, &b, rom, false, usize::MAX, paths)?;
    let path = triage_path(triage_file)?;
    let triage = Triage::load(&path)?;
    let names = report.class_names();
    let check = triage.check(a.entry.source.as_deref(), class_frames(&report));
    let text = report_text(&a, &b, &report, &check, &triage);
    if !check.passed() {
        return Ok(Outcome {
            json: json!({ "golden": false, "triage": check }),
            text: format!(
                "{text}\nnot golden: {}",
                [
                    (!check.untriaged.is_empty())
                        .then(|| format!("not triaged: {}", check.untriaged.join(", "))),
                    (!check.fork_bugs.is_empty())
                        .then(|| format!("fork bugs: {}", check.fork_bugs.join(", "))),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join("; ")
            ),
            exit: 1,
        });
    }
    let used: Vec<&Record> = names.iter().flat_map(|c| triage.of(c)).collect();
    let verdict = json!({
        "kind": "gp-oracle-verdict",
        "schema": 0,
        "recording": a.entry.id,
        "oracle": b.entry.id,
        "oracle_runtime": b.manifest.runtime,
        "report": report,
        "triage": used,
        "date": now_rfc3339(),
    });
    let verdict_id = store.put_bytes(serde_json::to_string_pretty(&verdict)?.as_bytes())?;
    let added = gp_evidence::recordings::append(
        &store,
        &IndexEntry {
            golden: true,
            notes: Some(format!(
                "golden: oracle verdict blake3:{verdict_id} (diff against {}, every class triaged)",
                &b.entry.id.to_hex()[..12]
            )),
            created_at: now_rfc3339(),
            ..a.entry.clone()
        },
    )?;
    crate::runrec::output("verdict", format!("blake3:{verdict_id}"), None);
    Ok(Outcome::ok(
        json!({ "golden": true, "verdict": verdict_id, "added": added }),
        format!(
            "{text}\ngolden: {} ({}), verdict blake3:{verdict_id}",
            a.entry.name,
            &a.entry.id.to_hex()[..12]
        ),
    ))
}

fn check(
    files: &[PathBuf],
    platforms: &[String],
    triage_file: Option<&Path>,
    paths: &Paths,
) -> Result<Outcome> {
    use gp_evidence::baseline::{self, Baseline};
    let mut list = Vec::new();
    for f in files {
        if f.is_dir() {
            list.extend(baseline::list(f)?);
        } else {
            list.push(f.clone());
        }
    }
    let archs: Vec<&str> = if platforms.is_empty() {
        vec![arch(None)?]
    } else {
        platforms
            .iter()
            .map(|p| arch(Some(p)))
            .collect::<Result<_>>()?
    };
    let triage_path = triage_path(triage_file)?;
    let triage = Triage::load(&triage_path)?;
    let tmp = tempfile::tempdir().context("creating a temporary store")?;
    let store = Store::open(tmp.path())?;
    let (mut lines, mut results) = (Vec::new(), Vec::new());
    let (mut passed, mut failed, mut skipped) = (0u64, 0u64, 0u64);
    for file in &list {
        let b = Baseline::load(file)?;
        let name = file
            .file_stem()
            .map_or_else(|| "baseline".into(), |s| s.to_string_lossy().into_owned());
        let work = work_dir(paths)?;
        let Some(rom) = find_rom(&b.sha256, None, paths, work.path())? else {
            skipped += 1;
            lines.push(format!(
                "skip   {}  (no ROM with its hash in {})",
                file.display(),
                paths.roms.display()
            ));
            continue;
        };
        let input = b.input()?;
        let image = identify_nes(&rom.bytes)?;
        let recorded = gp_nes::recording::record_with(
            &store,
            gp_nes::recording::RecordRequest {
                rom: &rom.bytes,
                source: SourceRef::NesRom {
                    sha256: image.sha256.clone(),
                    mapper: image.header.mapper,
                },
                source_id: b.source.clone(),
                power_on: gp_evidence::manifest::PowerOn::default(),
                input: &input,
                checkpoint_interval: 60,
                provenance: gp_evidence::manifest::Provenance::Scripted,
                name: name.clone(),
                created_at: now_rfc3339(),
                notes: None,
            },
            gp_nes::recording::Capture::ALL,
        )?;
        let gp = open(&store, &recorded.id.to_hex())?;
        let observed = states::input_log(&store, &gp.manifest)?;
        let mut mesen_ids = Vec::new();
        let mut mesen_rec = None;
        for arch in &archs {
            let run = run_mesen(
                &store,
                &gp.manifest,
                &name,
                b.source.clone(),
                &observed,
                &rom,
                arch,
                true,
                paths,
            )?;
            crate::runrec::metric(
                format!("oracle.check.{name}.{arch}.seconds"),
                run.seconds,
                "s",
            );
            lines.push(format!(
                "       {name}: Mesen on linux/{arch}: {} frames in {:.0} s, {}",
                run.manifest.frames,
                run.seconds,
                &run.id.to_hex()[..12]
            ));
            mesen_ids.push(run.id);
            mesen_rec.get_or_insert(run.id);
        }
        let same_everywhere = mesen_ids.windows(2).all(|w| w[0] == w[1]);
        let mesen = open(&store, &mesen_rec.expect("one platform at least").to_hex())?;
        let (report, _) = diff_recordings(
            &store,
            &gp,
            &mesen,
            Some(&rom.path),
            false,
            usize::MAX,
            paths,
        )?;
        let names = report.class_names();
        let c = triage.check(b.source.as_deref(), class_frames(&report));
        // Fork bugs are known and filed; what fails the check is a class
        // nobody has looked at, or Mesen disagreeing with itself.
        let ok = c.untriaged.is_empty() && same_everywhere;
        if ok {
            passed += 1;
        } else {
            failed += 1;
        }
        lines.push(format!(
            "{} {}  {} divergences, {} classes ({} accepted, {} not triaged, {} fork bugs){}",
            if ok { "ok    " } else { "FAILED" },
            file.display(),
            report.divergences.len(),
            names.len(),
            c.accepted.len(),
            c.untriaged.len(),
            c.fork_bugs.len(),
            if same_everywhere {
                String::new()
            } else {
                ", and the platforms' Mesen recordings differ".into()
            }
        ));
        for u in &c.uncovered {
            lines.push(format!("         not triaged: {u}"));
        }
        for u in &c.fork_bugs {
            lines.push(format!("         known fork bug: {u}"));
        }
        results.push(json!({
            "file": file,
            "ok": ok,
            "gp": recorded.id,
            "mesen": mesen_ids,
            "report": report,
            "triage": c,
        }));
    }
    crate::runrec::metric("oracle.check.passed", passed as f64, "count");
    crate::runrec::metric("oracle.check.failed", failed as f64, "count");
    lines.push(format!(
        "{passed} baselines passed, {failed} failed, {skipped} skipped (triage: {})",
        triage_path.display()
    ));
    Ok(Outcome {
        json: json!({ "results": results, "passed": passed, "failed": failed, "skipped": skipped }),
        text: lines.join("\n"),
        exit: u8::from(failed > 0),
    })
}
