//! The runs stage 0 makes of a ROM, from gp-events' levels L0
//! (footprints) and L1 (edges) (docs/research/rom-event-model.md): clips
//! from power-on, stage 0's own seeded starts, each clip's run kept in
//! gp-events' store (`$GP_HOME/work/events`) by the ROM and the input's
//! hash, so a clip run once isn't run again.
//!
//! The helpers of game-printer's `gp reach` that `gp explore` and `gp
//! watch` run on, under their old names; wave 3b brings `gp reach` itself
//! (its report, `--window` and `--diff-*`) on top of them.
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::Args;
use gp_events::store::Store as EventStore;
use gp_events::{EdgeKind, Levels};
use gp_evidence::input_log::InputFrame;
use gp_evidence::manifest::PowerOn;
use gp_evidence::{InputLog, Paths, Store};
use serde_json::{Value, json};
use sha2::Digest;

use crate::clipset;

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
        let mut out = Vec::new();
        if !self.no_starts {
            let set = format!("starts {} {} {}", self.runs, self.seed, self.start_frames);
            for (id, pads) in
                gp_explore::presses::start_set(self.runs, self.seed, self.start_frames)
            {
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
            clipset::parse_presses(&text)
        };
        for c in &set.clips {
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
#[allow(dead_code, reason = "`gp reach`'s report (wave 3b) reads the rest")]
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
