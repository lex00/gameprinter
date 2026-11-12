//! `gp reach`: run a ROM under clips and say what its code did, from
//! gp-events' levels L0 (footprints) and L1 (edges)
//! (docs/research/rom-event-model.md).
//!
//! Each clip of a clip set (docs/formats/clips.md) runs from power-on: its
//! start's presses from power-on (a `.presses` file, empty for power-on
//! itself) or the first frames of a recording's input, then its own
//! presses. The clips' levels are merged per ROM. The output names each
//! address as the static harvest does (`C123`, `03:8123`), holds no time
//! and no ROM byte, and is the same for the same ROM and clips.
//!
//! Each run's levels are kept in gp-events' store (`$GP_HOME/work/events`)
//! by the ROM and the input's hash, so a clip run once isn't run again.
//! `--frames` adds L2 (frame events, with `--watch` addresses) per clip,
//! and `--detail CLIP:A..B` prints L3, the instructions of frames A to B
//! of one clip, made again by running it.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::Args;
use gp_events::names::Namer;
use gp_events::store::Store as EventStore;
use gp_events::{COLLECTOR_VERSION, EdgeKind, Levels, NO_ROM};
use gp_evidence::input_log::InputFrame;
use gp_evidence::manifest::PowerOn;
use gp_evidence::{InputLog, Paths, Store};
use serde_json::{Value, json};
use sha2::Digest;

use crate::Outcome;
use crate::clipset;

/// The shape of `gp reach`'s output. A change to it bumps this.
pub const SCHEMA: &str = "gp-events reach 0.1.0";

#[derive(Args, Debug, Clone)]
pub struct ReachArgs {
    /// The iNES ROM to run.
    pub rom: PathBuf,
    /// Clip sets (clips.toml) whose clips to run after stage 0's own
    /// starts, each from power-on.
    #[arg(long = "clip")]
    pub clips: Vec<PathBuf>,
    #[command(flatten)]
    pub starts: Starts,
    /// Write reach.json (the whole output) into this directory, and with
    /// --frames each clip's L2 as CLIP.frames.ndjson.
    #[arg(long)]
    pub write: Option<PathBuf>,
    /// Also keep L2, each frame's events: the console's event log, code and
    /// edges first seen, RAM bytes changed, writes to --watch addresses.
    #[arg(long)]
    pub frames: bool,
    /// Addresses whose writes L2 lists with their writer (`$F0`, `0x6F0`).
    #[arg(long = "watch", value_name = "ADDR")]
    pub watch: Vec<String>,
    /// Print L3 instead: every instruction of frames A to B of one clip
    /// (`CLIP:A..B` without B, `CLIP:A..=B` with it), as `gp trace
    /// instructions` prints them.
    #[arg(long, value_name = "CLIP:A..B")]
    pub detail: Option<String>,
    /// The store folder (default $GP_HOME/work/events).
    #[arg(long)]
    pub store: Option<PathBuf>,
    /// Run every clip again, reading nothing from the store.
    #[arg(long)]
    pub no_store: bool,
    /// Only these clips of the sets: an ID, or a prefix ending in `*`.
    /// Repeat for more.
    #[arg(long, value_name = "ID")]
    pub only: Vec<String>,
    /// Report by routine of the code map over frames A to B of each clip
    /// (`A..B`, `A..=B`, `A..`, or `..` for whole clips): the instructions
    /// each ran and the ways each branch went, counted instruction by
    /// instruction (docs/code-reading.md). Stage 0's starts run only when
    /// no clip set is given.
    #[arg(long, value_name = "A..B")]
    pub window: Option<String>,
    /// Compare with other clip sets (side B): the routines, instructions
    /// and branch ways one side ran and the other didn't. B's clips are
    /// these sets' (whole clips unless --diff-window), else A's sets with
    /// --diff-only and --diff-window.
    #[arg(long = "diff-clip", value_name = "SET")]
    pub diff_clips: Vec<PathBuf>,
    /// Side B's clips: an ID, or a prefix ending in `*`.
    #[arg(long, value_name = "ID")]
    pub diff_only: Vec<String>,
    /// Side B's window (`A..B`, `A..=B`, `A..`, `..`).
    #[arg(long, value_name = "A..B")]
    pub diff_window: Option<String>,
    /// Side B is a `gp reach --write` output (reach.json): its instruction
    /// starts and branch edges, without counts.
    #[arg(long, value_name = "REACH_JSON", conflicts_with_all = ["diff_clips", "diff_only", "diff_window"])]
    pub diff_reach: Option<PathBuf>,
    /// Count, for each routine, the addresses these templates cite (`gp
    /// template cites`).
    #[arg(long = "cites", value_name = "TEMPLATE")]
    pub cites: Vec<PathBuf>,
}

/// One clip's input from power-on.
#[derive(Clone)]
pub(crate) struct ClipInput {
    /// The clip set's content hash (its file's BLAKE3), so the output
    /// doesn't depend on how its path was typed.
    pub set: String,
    pub id: String,
    /// The console's power-on: a recording start's own, else the default.
    pub power_on: PowerOn,
    pub inputs: Vec<InputFrame>,
}

impl ClipInput {
    /// What a run of this clip is: its power-on and every frame's input,
    /// all controllers and the reset button, hashed. The store keys runs by
    /// it, so two clips share a run only when they are the same run.
    pub fn key(&self) -> Result<String> {
        let v = json!({
            "power_on": serde_json::to_value(self.power_on)?,
            "frames": self.inputs.iter().map(|f| json!([f.pads, f.reset])).collect::<Vec<_>>(),
        });
        Ok(gp_evidence::Hash::of(&serde_json::to_vec(&v)?).to_string())
    }
}

/// Stage 0's own starts (`gp_explore::presses::start_set`): runs from
/// power-on made from a seed, so a command that runs a ROM needs no clip
/// file. Clip sets given as well run after them.
#[derive(Args, Debug, Clone)]
pub struct Starts {
    /// How many seeded runs of random presses to make, besides power-on
    /// idle and Start presses (each 3600 frames).
    #[arg(long, default_value_t = 4)]
    pub runs: usize,
    /// The seed of the random presses.
    #[arg(long, default_value_t = 1)]
    pub seed: u64,
    /// How many frames each start runs.
    #[arg(long, default_value_t = gp_explore::presses::START_FRAMES)]
    pub start_frames: usize,
    /// Make none of these starts: run only the clip sets given.
    #[arg(long)]
    pub no_starts: bool,
}

impl Starts {
    /// The starts as clips from power-on, with the clip sets' clips after.
    pub(crate) fn with(&self, sets: &[PathBuf], paths: &Paths) -> Result<Vec<ClipInput>> {
        self.with_platform(sets, paths, false)
    }

    /// [`Starts::with`] for an arcade romset when `arcade`: the starts are
    /// gp-explore's arcade palette's (a coin and Start, then the stick and
    /// tap), and `--start-frames` keeps its NES default of 3600, two
    /// minutes of the board's 30 Hz frames.
    pub(crate) fn with_platform(
        &self,
        sets: &[PathBuf],
        paths: &Paths,
        arcade: bool,
    ) -> Result<Vec<ClipInput>> {
        let mut out = Vec::new();
        if !self.no_starts {
            let set = format!(
                "starts {}{} {} {}",
                if arcade { "arcade " } else { "" },
                self.runs,
                self.seed,
                self.start_frames
            );
            let starts = if arcade {
                gp_explore::arcade::start_set(self.runs, self.seed, self.start_frames)
            } else {
                gp_explore::presses::start_set(self.runs, self.seed, self.start_frames)
            };
            for (id, pads) in starts {
                out.push(ClipInput {
                    set: set.clone(),
                    id,
                    power_on: PowerOn::default(),
                    inputs: gp_explore::presses::log_frames(&pads),
                });
            }
        }
        out.extend(clip_inputs(sets, paths)?);
        if out.is_empty() {
            bail!("no clip to run: --no-starts needs a clip set");
        }
        Ok(out)
    }
}

/// The input of every clip in `sets`, from power-on.
pub(crate) fn clip_inputs(sets: &[PathBuf], paths: &Paths) -> Result<Vec<ClipInput>> {
    let mut out = Vec::new();
    let mut store: Option<Store> = None;
    for file in sets {
        let set = clipset::read(file)?;
        let set_hash = gp_evidence::Hash::of(&fs::read(file)?).to_string();
        let dir = file.parent().unwrap_or(Path::new("."));
        let read = |p: &str| -> Result<(u64, Vec<String>)> {
            let text = fs::read_to_string(dir.join(p))
                .with_context(|| format!("reading {}", dir.join(p).display()))?;
            clipset::parse_presses(&text).map(|p| (p.frames, p.spans))
        };
        for c in &set.clips {
            // An arcade clip's presses name the board's buttons; its log
            // comes as the corpus makes it.
            if let Some((log, _)) = clipset::recipe_log(dir, c)?
                && log.arcade.is_some()
            {
                out.push(ClipInput {
                    set: set_hash.clone(),
                    id: c.id.clone(),
                    power_on: PowerOn::default(),
                    inputs: log.frames,
                });
                continue;
            }
            let mut power_on = PowerOn::default();
            let mut inputs: Vec<InputFrame> =
                match (&c.start.presses_from_power_on, &c.start.recording) {
                    (Some(p), _) => {
                        let (frames, spans) = read(p)?;
                        log_frames(frames, &spans)?
                    }
                    (None, Some(rec)) => {
                        if store.is_none() {
                            store = Some(Store::open(&paths.store)?);
                        }
                        let s = store.as_ref().expect("opened");
                        let e = gp_evidence::recordings::resolve(s, rec)?;
                        let m = gp_evidence::recordings::load_manifest(s, &e.id)?;
                        let log = InputLog::parse(
                            &String::from_utf8(s.get(&m.input_log)?)
                                .context("the recording's input log isn't text")?,
                        )?;
                        power_on = m.power_on;
                        let n = usize::try_from(c.start.frame)?;
                        if n > log.frames.len() {
                            bail!(
                                "{}: clip {} starts at frame {n} of recording {rec}, which has {}",
                                file.display(),
                                c.id,
                                log.frames.len()
                            );
                        }
                        log.frames[..n].to_vec()
                    }
                    (None, None) => bail!("{}: clip {} has no start", file.display(), c.id),
                };
            let (frames, spans) = match &c.presses_file {
                Some(p) => {
                    let (_, spans) = read(p)?;
                    (c.frames, spans)
                }
                None => (c.frames, c.presses.clone()),
            };
            inputs.extend(log_frames(frames, &spans)?);
            out.push(ClipInput {
                set: set_hash.clone(),
                id: c.id.clone(),
                power_on,
                inputs,
            });
        }
    }
    Ok(out)
}

fn log_frames(frames: u64, spans: &[String]) -> Result<Vec<InputFrame>> {
    let refs: Vec<&str> = spans.iter().map(String::as_str).collect();
    Ok(gp_evidence::input_log::generate(usize::try_from(frames)?, &refs)?.frames)
}

/// The clips' runs, merged, with the clip each edge was first taken in.
pub(crate) struct Reached {
    pub levels: Levels,
    /// For each edge, the clip (index into the clips) that first took it.
    pub first_clip: BTreeMap<(EdgeKind, u32, u32), usize>,
    pub clips: Vec<(String, String, u64, String)>,
    /// Clips run now; the others came from the store.
    pub ran: usize,
    pub seconds: f64,
}

/// What `run_clips` keeps besides L0 and L1.
#[derive(Default)]
pub(crate) struct Keep<'a> {
    pub store: Option<&'a EventStore>,
    /// L2 with these watched addresses, into the store (and `out`).
    pub frames: Option<&'a std::collections::BTreeSet<u16>>,
    pub out: Option<&'a Path>,
}

/// The hash a store keys a watch set by.
fn watch_key(w: &std::collections::BTreeSet<u16>) -> String {
    let text: String = w.iter().map(|a| format!("{a:04X},")).collect();
    gp_evidence::Hash::of(text.as_bytes()).to_string()[..16].to_owned()
}

/// Runs each clip from power-on with a collector of L0 and L1 (and L2 when
/// asked), merged in the clips' order; a clip whose levels the store has
/// isn't run. Clips run on the free helper threads (`--jobs`), each
/// distinct input once, so the output is the same for any number of jobs.
pub(crate) fn run_clips(rom: &[u8], clips: &[ClipInput], keep: &Keep) -> Result<Reached> {
    let t0 = Instant::now();
    let rom_sha = hex::encode(sha2::Sha256::digest(rom));
    let wkey = keep.frames.map(watch_key);
    let keys = clips
        .iter()
        .map(ClipInput::key)
        .collect::<Result<Vec<_>>>()?;
    // The first clip of each distinct input runs it; the rest share its run.
    let mut first_of: BTreeMap<&str, usize> = BTreeMap::new();
    for (i, k) in keys.iter().enumerate() {
        first_of.entry(k.as_str()).or_insert(i);
    }
    let runs: Vec<usize> = first_of.values().copied().collect();
    let done = crate::pool::map(
        &runs,
        || (),
        |_, _, &i| -> std::result::Result<(Levels, bool), String> {
            run_one(rom, &rom_sha, &clips[i], &keys[i], wkey.as_deref(), keep)
                .map_err(|e| format!("{e:#}"))
        },
        &mut |_| {},
    );
    let mut by_key: BTreeMap<&str, Levels> = BTreeMap::new();
    let mut ran = 0;
    for (&i, r) in runs.iter().zip(done) {
        let (l, fresh) = r.map_err(anyhow::Error::msg)?;
        ran += usize::from(fresh);
        by_key.insert(keys[i].as_str(), l);
    }
    let mut merged: Option<Levels> = None;
    let mut first_clip = BTreeMap::new();
    let mut listed = Vec::new();
    for (i, c) in clips.iter().enumerate() {
        let input = &keys[i];
        let levels = &by_key[input.as_str()];
        if let (Some(st), Some(w), Some(out)) = (keep.store, &wkey, keep.out) {
            fs::create_dir_all(out)?;
            let name = format!("{}.frames.ndjson", clipset::file_key(&c.id));
            fs::copy(st.frames_file(&rom_sha, input, w), out.join(name))?;
        }
        listed.push((c.set.clone(), c.id.clone(), levels.frames, input.clone()));
        for r in &levels.edges {
            first_clip.entry((r.kind, r.from, r.to)).or_insert(i);
        }
        match &mut merged {
            None => merged = Some(levels.clone()),
            Some(m) => m.merge(levels).map_err(anyhow::Error::msg)?,
        }
    }
    Ok(Reached {
        levels: merged.context("no clip to run")?,
        first_clip,
        clips: listed,
        ran,
        seconds: t0.elapsed().as_secs_f64(),
    })
}

/// One clip's levels: the store's, or a run's (true) kept in the store.
fn run_one(
    rom: &[u8],
    rom_sha: &str,
    c: &ClipInput,
    input: &str,
    wkey: Option<&str>,
    keep: &Keep,
) -> Result<(Levels, bool)> {
    let kept = keep.store.and_then(|st| {
        let l = st.levels(rom_sha, input)?;
        match wkey {
            Some(w) if !st.has_frames(rom_sha, input, w) => None,
            _ => Some(l),
        }
    });
    if let Some(l) = kept {
        return Ok((l, false));
    }
    let (run, frames) = match keep.frames {
        Some(w) => {
            let (r, f) = gp_events::run_frames(rom, c.power_on, &c.inputs, w)?;
            (r, Some(f))
        }
        None => (gp_events::run(rom, c.power_on, &c.inputs, true)?, None),
    };
    let l = Levels::of(&run.collector, run.frames);
    // With no store, L2 goes straight to the output folder.
    if let (None, Some(f), Some(out)) = (keep.store, &frames, keep.out) {
        fs::create_dir_all(out)?;
        let mut text = Vec::new();
        for e in f {
            serde_json::to_writer(&mut text, e)?;
            text.push(b'\n');
        }
        let name = format!("{}.frames.ndjson", clipset::file_key(&c.id));
        fs::write(out.join(name), text)?;
    }
    if let Some(st) = keep.store {
        st.put_levels(rom_sha, input, &l)?;
        if let (Some(f), Some(w)) = (&frames, wkey) {
            st.put_frames(rom_sha, input, w, f)?;
        }
    }
    Ok((l, true))
}

/// The output, as JSON: the clips, L0's counts and sets, and L1's edges
/// by kind and in full, with a hash over all of it.
pub(crate) fn report(rom: &[u8], r: &Reached) -> Result<Value> {
    let (prg, mapper) = gp_nes::harvest::ines_prg(rom).context("not an iNES ROM")?;
    let mut board = gp_nes::codemap::Board::of(prg, mapper);
    board.switched = Namer::observed_switched(&board, &r.levels.footprint.windows);
    let namer = Namer::of_board(board);
    let f = &r.levels.footprint;
    let mut by_kind: BTreeMap<String, usize> = BTreeMap::new();
    let mut list = Vec::new();
    for e in &r.levels.edges {
        let (k, s) = ((e.kind, e.from, e.to), &e.stat);
        let kind = serde_json::to_value(k.0)?
            .as_str()
            .unwrap_or_default()
            .to_owned();
        *by_kind.entry(kind.clone()).or_default() += 1;
        let first = r.first_clip.get(&k).map(|&i| &r.clips[i].1);
        let mut v = json!({
            "kind": kind,
            "from": namer.name(k.1, s.from_cpu),
            "to": namer.name(k.2, s.to_cpu),
            "from_offset": (k.1 != NO_ROM).then_some(k.1),
            "to_offset": (k.2 != NO_ROM).then_some(k.2),
            "count": s.count,
            "first": {"clip": first, "frame": s.first_frame},
        });
        if !s.pointers.is_empty() {
            v["pointers"] = json!(
                s.pointers
                    .iter()
                    .map(|(p, val)| format!("${p:04X}=${val:04X}"))
                    .collect::<Vec<_>>()
            );
        }
        list.push(v);
    }
    let entries: Vec<String> = r
        .levels
        .edges
        .iter()
        .filter(|e| e.kind.enters() && e.to != NO_ROM)
        .map(|e| (e.to, namer.name(e.to, e.stat.to_cpu)))
        .collect::<BTreeMap<_, _>>()
        .into_values()
        .collect();
    let mut doc = json!({
        "schema": SCHEMA,
        "collector_version": COLLECTOR_VERSION,
        "rom_sha256": hex::encode(sha2::Sha256::digest(rom)),
        "prg_bytes": f.executed.len(),
        "clips": r.clips.iter().map(|(set, id, frames, input)| json!({
            "set": set, "id": id, "frames": frames, "input": input,
        })).collect::<Vec<_>>(),
        "footprint": {
            "instructions": f.instructions,
            "outside_rom": f.outside_rom,
            "instruction_starts": f.starts.count(),
            "executed_bytes": f.executed.count(),
            "ram_written_bytes": f.ram_written.count(),
            "prg_ram_written_bytes": f.prg_ram_written.count(),
            "executed": f.executed,
            "starts": f.starts,
        },
        "edges": {
            "count": r.levels.edges.len(),
            "by_kind": by_kind,
            "entries": entries,
            "list": list,
        },
    });
    let hash = gp_evidence::Hash::of(&serde_json::to_vec(&doc)?).to_string();
    doc["hash"] = json!(hash);
    Ok(doc)
}

/// An address as `--watch` takes it: `$F0`, `0x6F0` or decimal.
pub(crate) fn parse_addr(s: &str) -> Result<u16> {
    let t = s.trim();
    let v = if let Some(h) = t.strip_prefix('$').or_else(|| t.strip_prefix("0x")) {
        u16::from_str_radix(h, 16)
    } else {
        t.parse()
    };
    v.with_context(|| format!("{s:?} isn't an address"))
}

/// Frames `A..B` (without B) or `A..=B` (with it), as `gp trace
/// instructions` reads a range: the first and last frame.
pub(crate) fn frame_range(range: &str) -> Result<(u64, u64)> {
    let (a, b) = range.split_once("..").context("a range is A..B or A..=B")?;
    let a: u64 = a.parse()?;
    let last: u64 = match b.strip_prefix('=') {
        Some(b) => b.parse()?,
        None => b.parse::<u64>()?.checked_sub(1).context("an empty range")?,
    };
    if last < a {
        bail!("the range {range} is empty");
    }
    Ok((a, last))
}

/// L3 for frames `a` to `last` of a recipe, as `gp reach --detail` prints
/// it: the output's JSON (with `clip`) and its text.
pub(crate) fn detail_of(rom: &[u8], c: &ClipInput, a: u64, last: u64) -> Result<(Value, String)> {
    let mut out = Vec::new();
    let t = gp_events::detail::window(rom, c.power_on, &c.inputs, a, last, &mut out)?;
    let text = String::from_utf8(out)?;
    Ok((
        json!({"clip": c.id, "start": t.start, "end": t.end, "instructions": t.instructions, "interrupts": t.interrupts, "trace": text}),
        text.trim_end().to_owned(),
    ))
}

/// `--detail CLIP:A..B`: L3 for one clip's frames.
fn detail(rom: &[u8], clips: &[ClipInput], spec: &str) -> Result<Outcome> {
    let (id, range) = spec.rsplit_once(':').context("--detail takes CLIP:A..B")?;
    let (a, last) = frame_range(range).with_context(|| format!("--detail {spec}"))?;
    let c = clips
        .iter()
        .find(|c| c.id == id)
        .with_context(|| format!("no clip {id} in the sets given"))?;
    let (v, text) = detail_of(rom, c, a, last)?;
    Ok(Outcome::ok(v, text))
}

pub(crate) fn run(a: &ReachArgs, paths: &Paths) -> Result<Outcome> {
    if crate::reach_window::wanted(a) {
        if a.detail.is_some() || a.frames {
            bail!(
                "--window and --diff-* report by routine; --detail and --frames are other reports"
            );
        }
        return crate::reach_window::run(a, paths);
    }
    let rom = fs::read(&a.rom).with_context(|| format!("reading {}", a.rom.display()))?;
    let arcade = crate::original::Platform::of_file(&rom) == crate::original::Platform::Arcade;
    let clips = crate::code::select(a.starts.with_platform(&a.clips, paths, arcade)?, &a.only)?;
    if crate::original::Platform::of_file(&rom) == crate::original::Platform::Arcade {
        // Both Z80s' levels, by chip (crate::code_z80); --detail, --frames
        // and --watch are the NES's reports still.
        if a.detail.is_some() || a.frames || !a.watch.is_empty() {
            bail!(
                "--detail, --frames and --watch read gp-events' NES levels; on an arcade romset gp reach reports both Z80s' code by chip (and `gp trace instructions` a window's instructions)"
            );
        }
        let set = gp_arcade::Romset::from_zip(&rom)?;
        let levels = crate::code_z80::run_clips(&set, &clips, (1, u64::MAX))?;
        let (text, doc) = crate::code_z80::reach(&set, &levels);
        if let Some(w) = &a.write {
            fs::write(w, serde_json::to_string_pretty(&doc)?)
                .with_context(|| format!("writing {}", w.display()))?;
        }
        return Ok(Outcome::ok(doc, text));
    }
    if let Some(spec) = &a.detail {
        return detail(&rom, &clips, spec);
    }
    let store = (!a.no_store).then(|| {
        EventStore::at(
            a.store
                .clone()
                .unwrap_or_else(|| paths.home.join("work").join("events")),
        )
    });
    let watch: std::collections::BTreeSet<u16> = a
        .watch
        .iter()
        .map(|w| parse_addr(w))
        .collect::<Result<_>>()?;
    if a.frames && store.is_none() && a.write.is_none() {
        bail!("--frames with --no-store needs --write, where L2 goes");
    }
    let store_ref = store.as_ref();
    let keep = Keep {
        store: store_ref,
        frames: a.frames.then_some(&watch),
        out: a.write.as_deref().filter(|_| a.frames),
    };
    let r = run_clips(&rom, &clips, &keep)?;
    let doc = report(&rom, &r)?;
    if let Some(d) = &a.write {
        fs::create_dir_all(d)?;
        fs::write(
            d.join("reach.json"),
            serde_json::to_string_pretty(&doc)? + "\n",
        )?;
    }
    let frames: u64 = r.clips.iter().map(|c| c.2).sum();
    let f = &doc["footprint"];
    let e = &doc["edges"];
    let text = format!(
        "{} clips ({} run, {} from the store), {frames} frames in {:.2} s: {} instruction starts, {} PRG bytes executed of {}; {} edges ({}), {} entries\nhash {}",
        r.clips.len(),
        r.ran,
        r.clips.len() - r.ran,
        r.seconds,
        f["instruction_starts"],
        f["executed_bytes"],
        doc["prg_bytes"],
        e["count"],
        e["by_kind"]
            .as_object()
            .map(|m| m
                .iter()
                .map(|(k, v)| format!("{k} {v}"))
                .collect::<Vec<_>>()
                .join(", "))
            .unwrap_or_default(),
        e["entries"].as_array().map_or(0, Vec::len),
        doc["hash"].as_str().unwrap_or_default(),
    );
    Ok(Outcome::ok(doc, text))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(pad2: u8, power_on: PowerOn) -> ClipInput {
        ClipInput {
            set: String::new(),
            id: "c".into(),
            power_on,
            inputs: vec![
                InputFrame {
                    polled: true,
                    pads: [0, pad2, 0, 0],
                    reset: false,
                };
                20
            ],
        }
    }

    /// Two runs share a store key only when they are the same run: the
    /// second controller and the power-on are part of it.
    #[test]
    fn the_key_holds_every_controller_and_the_power_on() {
        let base = clip(0, PowerOn::default());
        assert_eq!(
            base.key().unwrap(),
            clip(0, PowerOn::default()).key().unwrap()
        );
        assert_ne!(
            base.key().unwrap(),
            clip(1, PowerOn::default()).key().unwrap()
        );
        let fceux = PowerOn {
            ram_init: gp_evidence::manifest::RamInit::Fceux,
            ..PowerOn::default()
        };
        assert_ne!(base.key().unwrap(), clip(0, fceux).key().unwrap());
        let mut reset = clip(0, PowerOn::default());
        reset.inputs[5].reset = true;
        assert_ne!(base.key().unwrap(), reset.key().unwrap());
    }
}
