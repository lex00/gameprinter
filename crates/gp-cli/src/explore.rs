//! `gp explore`: search for presses that run code no run has run yet, with
//! no model (gp-explore, docs/research/rom-event-model.md, section 8).
//!
//! The start is a clip from power-on (its whole input is the prefix). The
//! start clips' runs seed the static harvest's code map (as `gp model
//! static --clips` does), and the targets are the routines it has that no
//! run reached, or the routines named. The search ranks each candidate by
//! the fewest calls from a routine it ran to a target, then by the code it
//! ran first; with no target, by new code alone. What it keeps is written
//! as clips from power-on, so `gp reach` and `gp model static --clips` run
//! them as any other.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::Args;
use gp_evidence::Paths;
use gp_explore::presses::spans;
use gp_explore::{CallGraph, Found, NewCode, Opts, Progress, Score, Start, Toward, search_watched};
use serde_json::json;

use crate::Outcome;
use crate::clipset;
use crate::reach::{ClipInput, Keep, Starts, run_clips};

#[derive(Args, Debug, Clone)]
pub struct ExploreArgs {
    /// The iNES ROM, or an arcade romset's zip (searched on gp-arcade with
    /// the board's stick and tap, crate::explore_z80).
    pub rom: PathBuf,
    /// A clip set (clips.toml) whose clips are run first, after stage 0's
    /// own starts; the search starts at the end of each (or of the `--from`
    /// ones), each candidate from one.
    #[arg(long)]
    pub start: Option<PathBuf>,
    /// How many seeded runs of random presses stage 0's starts make
    /// (`gp model static --runs`).
    #[arg(long, default_value_t = 4)]
    pub start_runs: usize,
    /// Their seed (`gp model static --seed`).
    #[arg(long, default_value_t = 1)]
    pub start_seed: u64,
    /// How many frames each runs (`gp model static --start-frames`).
    #[arg(long, default_value_t = gp_explore::presses::START_FRAMES)]
    pub start_frames: usize,
    /// Start only from `--start`'s clips, not stage 0's own starts.
    #[arg(long, requires = "start")]
    pub no_starts: bool,
    /// Clips to start from, by ID (default: every clip of the set).
    #[arg(long)]
    pub from: Vec<String>,
    /// Routines to aim at, by the harvest's names (`sub_01_8000`; on an
    /// arcade romset the main Z80's, `sub_5FAC`). On an arcade romset also
    /// an address inside a routine (`0BC3`, a branch way), reached when the
    /// main CPU runs the instruction there. Default: as `--aim` says.
    #[arg(long = "target")]
    pub targets: Vec<String>,
    /// What to aim at with no `--target`: `unrun`, the routines the code map
    /// has that no start clip ran; `unread`, those no path of the static
    /// reader read (the unread report's routines); or `stops`, the target
    /// of each place the reader stopped, in each bank the board can map at
    /// that address.
    #[arg(long, default_value = "unrun")]
    pub aim: String,
    /// Aim at new code only, with no target.
    #[arg(long, conflicts_with = "targets")]
    pub new_code: bool,
    /// Candidates to run, at most.
    #[arg(long, default_value_t = 300)]
    pub budget: usize,
    #[arg(long, default_value_t = 1)]
    pub seed: u64,
    #[arg(long, default_value_t = 60)]
    pub min_frames: usize,
    #[arg(long, default_value_t = 180)]
    pub max_frames: usize,
    /// Clips to keep, best first.
    #[arg(long, default_value_t = 8)]
    pub keep: usize,
    /// Write clips.toml (the kept clips, from power-on) and each start's
    /// explore-start-K.presses
    /// into this directory.
    #[arg(long)]
    pub write: Option<PathBuf>,
    /// The store of runs (default $GP_HOME/work/events).
    #[arg(long)]
    pub store: Option<PathBuf>,
}

impl ExploreArgs {
    /// Stage 0's starts as these flags set them.
    pub(crate) fn starts(&self) -> Starts {
        Starts {
            runs: self.start_runs,
            seed: self.start_seed,
            start_frames: self.start_frames,
            no_starts: self.no_starts,
        }
    }
}

/// What a search found, for `gp explore` and `gp watch` to write.
pub(crate) struct Explored {
    pub start_clips: Vec<ClipInput>,
    pub targets: Vec<String>,
    /// The targets the kept clips reach.
    pub reached: Vec<String>,
    /// Kept, best first, at most `--keep`.
    pub kept: Vec<Found>,
}

/// The routines a stop's target names: the target address in each bank the
/// board can map there, those the code map has (all of one, unswitched).
pub(crate) fn stop_targets(
    board: &gp_nes::codemap::Board,
    graph: &CallGraph,
    target: &str,
) -> Vec<usize> {
    let Ok(t) = u16::from_str_radix(target, 16) else {
        return Vec::new();
    };
    let names: Vec<String> = match board.region(t) {
        Some((_, size)) => (0..board.prg.len() / size as usize)
            .map(|b| format!("sub_{b:02X}_{t:04X}"))
            .collect(),
        None => vec![format!("sub_{t:04X}")],
    };
    names.iter().filter_map(|n| graph.index(n)).collect()
}

/// The search `a` asks for, from `clips` whose merged runs are `known`,
/// telling `on` as it goes.
pub(crate) fn explore_with(
    a: &ExploreArgs,
    rom_path: &std::path::Path,
    rom: &[u8],
    clips: Vec<ClipInput>,
    known: &gp_events::Levels,
    on: &mut dyn FnMut(Progress),
) -> Result<Explored> {
    let start_clips: Vec<ClipInput> = if a.from.is_empty() {
        clips
    } else {
        let mut v = Vec::new();
        for id in &a.from {
            let i = clips
                .iter()
                .position(|c| &c.id == id)
                .with_context(|| format!("no start clip {id}"))?;
            v.push(clips[i].clone());
        }
        v
    };
    anyhow::ensure!(!start_clips.is_empty(), "the start set has no clip");
    let (h, unread) = crate::model_static::harvest_seeded(rom_path, None, true, Some(known))?;
    let unread_names: BTreeSet<String> = unread
        .as_ref()
        .map(|u| u.unread_routines.iter().cloned().collect())
        .unwrap_or_default();
    let (prg, mapper) = gp_nes::harvest::ines_prg(rom).context("not an iNES file")?;
    let mut board = gp_nes::codemap::Board::of(prg, mapper);
    board.switched = gp_events::names::Namer::observed_switched(&board, &known.footprint.windows);
    let graph = CallGraph::of(&board, &h);
    anyhow::ensure!(
        matches!(a.aim.as_str(), "unrun" | "unread" | "stops"),
        "--aim is unrun, unread or stops"
    );
    let targets: BTreeSet<usize> = if a.new_code {
        BTreeSet::new()
    } else if !a.targets.is_empty() {
        a.targets
            .iter()
            .map(|t| {
                graph
                    .index(t)
                    .with_context(|| format!("the code map has no routine {t}"))
            })
            .collect::<Result<_>>()?
    } else if a.aim == "stops" {
        unread
            .iter()
            .flat_map(|u| u.stops.iter())
            .map(|st| st.target.as_str())
            .filter(|t| !t.is_empty())
            .flat_map(|t| stop_targets(&board, &graph, t))
            .filter(|&i| {
                let (_, o, n) = &graph.routines[i];
                !(*o..o + n).any(|x| known.footprint.starts.contains(x))
            })
            .collect()
    } else {
        (0..graph.routines.len())
            .filter(|&i| {
                let (name, o, n) = &graph.routines[i];
                // A routine named by the power-on view in a switched region
                // (`sub_8000` on a banked board) is the harvest's guess at
                // a bank it didn't know: which bytes it means is unknown.
                let ambiguous = gp_nes::codemap::va_parse(name.trim_start_matches("sub_"))
                    .is_some_and(|v| v >> 16 == 0 && board.region(v as u16).is_some());
                let aimed = match a.aim.as_str() {
                    "unread" => unread_names.contains(name),
                    _ => !(*o..o + n).any(|x| known.footprint.starts.contains(x)),
                };
                !ambiguous && aimed
            })
            .collect()
    };
    let target_names: Vec<String> = targets
        .iter()
        .map(|&i| graph.routines[i].0.clone())
        .collect();
    let starts: Vec<Start> = start_clips
        .iter()
        .map(|c| Start::of(rom, c.power_on, &c.inputs))
        .collect::<Result<_, _>>()?;
    let opts = Opts {
        seed: a.seed,
        budget: a.budget,
        min_frames: a.min_frames,
        max_frames: a.max_frames,
    };
    let mut toward;
    let mut new_code;
    let score: &mut dyn Score = if targets.is_empty() {
        new_code = NewCode::new(known);
        &mut new_code
    } else {
        toward = Toward::new(graph.clone(), targets.clone(), known);
        &mut toward
    };
    let mut found = search_watched(rom, &starts, score, &opts, on)?;
    found.truncate(a.keep);
    // What the kept clips reach, not every candidate the search ran.
    let reached: Vec<String> = {
        let mut r = BTreeSet::new();
        for f in &found {
            for &t in &targets {
                let (_, o, n) = &graph.routines[t];
                if (*o..o + n).any(|x| f.levels.footprint.starts.contains(x)) {
                    r.insert(graph.routines[t].0.clone());
                }
            }
        }
        r.into_iter().collect()
    };
    Ok(Explored {
        start_clips,
        targets: target_names,
        reached,
        kept: found,
    })
}

/// Why a kept clip was kept, in words.
pub(crate) fn why(f: &Found) -> String {
    if f.rank.0 == u32::MAX {
        format!("runs {} instruction starts no run had", -f.rank.1)
    } else if f.rank.0 == 0 {
        "runs a target routine".to_owned()
    } else {
        format!(
            "{} calls from a target, {} new instruction starts",
            f.rank.0, -f.rank.1
        )
    }
}

pub(crate) fn run(a: &ExploreArgs, paths: &Paths) -> Result<Outcome> {
    let t0 = Instant::now();
    let rom = fs::read(&a.rom).with_context(|| format!("reading {}", a.rom.display()))?;
    if crate::roms::Platform::of_file(&rom) == crate::roms::Platform::Arcade {
        return run_arcade(a, &rom, paths, t0);
    }
    let sets: Vec<PathBuf> = a.start.iter().cloned().collect();
    let clips = a.starts().with(&sets, paths)?;
    let store = gp_events::store::Store::at(
        a.store
            .clone()
            .unwrap_or_else(|| paths.home.join("work").join("events")),
    );
    let known = run_clips(
        &rom,
        &clips,
        &Keep {
            store: Some(&store),
            ..Default::default()
        },
    )?
    .levels;
    let e = explore_with(a, &a.rom, &rom, clips, &known, &mut |_| {})?;
    let (target_names, reached) = (&e.targets, &e.reached);
    let kept: Vec<&Found> = e.kept.iter().collect();
    if let Some(d) = &a.write {
        write_clips(d, a, &e)?;
    }
    let secs = t0.elapsed().as_secs_f64();
    let out = json!({
        "targets": target_names,
        "reached": reached,
        "kept": kept.iter().map(|f| json!({
            "presses": spans(&f.pads),
            "frames": f.pads.len(),
            "distance": (f.rank.0 != u32::MAX).then_some(f.rank.0),
            "new_starts": -f.rank.1,
        })).collect::<Vec<_>>(),
        "budget": a.budget,
        "seed": a.seed,
    });
    let text = format!(
        "{} targets ({}); reached {} ({}); kept {} clips; {:.1} s",
        target_names.len(),
        target_names
            .iter()
            .take(6)
            .cloned()
            .collect::<Vec<_>>()
            .join(", "),
        reached.len(),
        reached.join(", "),
        kept.len(),
        secs,
    );
    Ok(Outcome::ok(out, text))
}

/// `gp explore` on an arcade romset (crate::explore_z80): the same flags,
/// output and clips.toml, the presses in the board's buttons.
fn run_arcade(a: &ExploreArgs, rom: &[u8], paths: &Paths, t0: Instant) -> Result<Outcome> {
    let set = gp_arcade::Romset::from_zip(rom)?;
    let sets: Vec<PathBuf> = a.start.iter().cloned().collect();
    let clips = a.starts().with_platform(&sets, paths, true)?;
    let known = crate::explore_z80::known(&set, &clips)?;
    let e = crate::explore_z80::explore_with(a, &set, clips, &known, &mut |_| {})?;
    if let Some(d) = &a.write {
        crate::explore_z80::write_clips(d, a, &set, &e)?;
    }
    let secs = t0.elapsed().as_secs_f64();
    let out = json!({
        "set": set.name,
        "targets": e.targets,
        "reached": e.reached,
        "kept": e.kept.iter().map(|f| json!({
            "presses": gp_explore::arcade::spans(&f.pads),
            "frames": f.pads.len(),
            "distance": (f.rank.0 != u32::MAX).then_some(f.rank.0),
            "new_starts": -f.rank.1,
            "why": crate::explore_z80::why(f),
        })).collect::<Vec<_>>(),
        "budget": a.budget,
        "seed": a.seed,
    });
    let text = format!(
        "{}: {} targets ({}); reached {} ({}); kept {} clips; {:.1} s",
        set.name,
        e.targets.len(),
        e.targets
            .iter()
            .take(6)
            .cloned()
            .collect::<Vec<_>>()
            .join(", "),
        e.reached.len(),
        e.reached.join(", "),
        e.kept.len(),
        secs,
    );
    Ok(Outcome::ok(out, text))
}

/// Writes what `e` kept into `d` as a clip set from power-on: clips.toml and
/// each start's explore-start-K.presses.
pub(crate) fn write_clips(d: &std::path::Path, a: &ExploreArgs, e: &Explored) -> Result<()> {
    let (start_clips, kept) = (&e.start_clips, &e.kept);
    fs::create_dir_all(d)?;
    let out = d.join("clips.toml");
    if out.exists()
        && let Some(st) = &a.start
        && fs::canonicalize(&out)? == fs::canonicalize(st)?
    {
        bail!(
            "--write {} would replace the start set; write into another folder",
            d.display()
        );
    }
    for (k, c) in start_clips.iter().enumerate() {
        if c.inputs.iter().any(|i| i.reset || i.pads[1] != 0) {
            bail!(
                "start clip {}'s presses reset or use a second controller, which a .presses file can't say",
                c.id
            );
        }
        let pads: Vec<u8> = c.inputs.iter().map(|i| i.pads[0]).collect();
        fs::write(
            d.join(format!("explore-start-{k}.presses")),
            clipset::presses_text(&crate::clipset::Presses {
                frames: pads.len() as u64,
                spans: spans(&pads),
                arcade: None,
            }),
        )?;
    }
    let mut text = String::from("[clipset]\nformat = \"0.3.0\"\ngenerator = \"gp explore\"\n");
    for (i, f) in kept.iter().enumerate() {
        let why = why(f);
        let presses = spans(&f.pads)
            .iter()
            .map(|s| format!("\"{s}\""))
            .collect::<Vec<_>>()
            .join(", ");
        text.push_str(&format!(
            "\n[[clips]]\nid = \"explore.s{}.{i}\"\nname = \"Explored from {}: {}\"\nsource = \"explore\"\npresses = [{presses}]\nframes = {}\nwhy = \"{why}\"\n\n[clips.start]\npresses_from_power_on = \"explore-start-{}.presses\"\nframe = {}\n",
            a.seed,
            start_clips[f.start].id,
            why,
            f.pads.len(),
            f.start,
            start_clips[f.start].inputs.len(),
        ));
    }
    fs::write(d.join("clips.toml"), text)?;
    Ok(())
}
