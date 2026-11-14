//! The heavy part of `gp progress` (docs/progress.md): `--measure` plays a
//! full playthrough (a TAS or a person's run through the game) on the
//! original and keeps what the ladder's stages 2, 3, 6 and 9 count:
//!
//! | Measure | How |
//! |---|---|
//! | RAM the game changes in play | Each byte that differs from the frame before, on frames the player controls (`observe.toml`'s `control`, else the mapping's `start`, else every polled frame). On the NES the stack page and the sprite page are left out; on an arcade board the 2 KB of work RAM at E000, by its index from there |
//! | Each RAM byte's role | On the NES, from a trace of every instruction of the pass ([`gp_nes::ramrole`]): input, a counter, scratch, sound, a pointer, or game state. Not on an arcade board yet |
//! | Object kinds | Each slot table's ID byte in every live slot, on the same frames |
//! | Code | The CPU addresses an instruction runs at, on the same frames: gp-nes's covered step, or the main Z80's from gp-arcade's trace ([`crate::covered`]) |
//! | The clips' code | Each of the game's corpus clips run on the original from its start |
//! | The whole game | The rebuilt game started from the original's state at the playthrough's first frame in play (the start translation, with every loaded object carried), then both on the playthrough's input in lockstep, free mode: the first frame any measure parts, and the share of frames before it. Then again in resync mode, put back after each part, for the frames that agree |
//! | Level by level | The same from each level's first frame in play, to the next level's: the frames before the level's first part |
//!
//! The lockstep runs (the two whole-game passes and one a level) run side
//! by side on the pool, each from a copy of the original taken in the one
//! pass, so none replays the frames before its start.
//!
//! The result goes to `$GP_HOME/work/progress/<game>/<run>.json`, keyed by
//! the model's hash, so the ladder says when it is older than the model.
//! Nothing in the run or the store is written.
//!
//! Ported from game-printer's progress_measure.rs. What gameprinter's
//! layout changed: a run is a game of the games checkout (gp serve's), its
//! model the run's working copy or the game's folder, its ROM the one its
//! package names; the playthrough defaults to the longest store recording
//! of the game's ROMs; the clips are the game's corpus clips; the start and
//! the lockstep from it are the sweep's ([`crate::sweep::measure`]) rather
//! than a copy; and the original is [`OriginalRuntime`], so an arcade
//! game (Tapper) is measured through its own mapping on gp-arcade.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use gp_interp::lockstep::Mode;
use gp_interp::mapping::Mapping;
use gp_nes::coverage::CodeMap;
use gp_nes::ramrole::{Classified, Tracer};
use gp_runtime::Runtime;
use gp_runtime::select::Condition;

use crate::original::OriginalRuntime;
use crate::sweep::measure::{ModelFrame, StartHere, holds};

/// What the measure keeps.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Measured {
    pub version: String,
    /// The playthrough's recording ID and its name in the store.
    pub playthrough: String,
    pub name: String,
    pub frames: u64,
    /// Frames the player controls.
    pub play_frames: u64,
    /// The RAM bytes that change in play.
    pub ram: Vec<u16>,
    /// The (slot table, ID) pairs seen in a live slot in play.
    pub kinds: Vec<(u8, u8)>,
    /// CPU addresses run in play.
    pub code: u64,
    /// The levels the playthrough reaches, in order, by the mapping's
    /// slot levels.
    pub levels: Vec<String>,
    pub clips: Option<ClipCode>,
    pub whole: Option<Whole>,
    /// Each RAM byte's role, from the trace.
    #[serde(default)]
    pub roles: Classified,
    /// The model's hash it was measured on.
    pub model: String,
    pub seconds: f64,
}

/// The run's clips on the original.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct ClipCode {
    pub clips: u64,
    /// CPU addresses the clips run.
    pub code: u64,
    /// Of those, the ones the playthrough runs in play too.
    pub shared: u64,
    /// The ones it doesn't.
    pub only: u64,
}

/// The whole playthrough on the rebuilt game.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Whole {
    /// The original's frame the rebuilt game starts from, its state
    /// translated: the playthrough's first frame in play.
    pub from: u64,
    /// Frames compared, from there to the end.
    pub compared: u64,
    /// The first frame a measure parts, in free mode.
    pub first: Option<FirstSplit>,
    /// The first frame one of the player's own measures parts (its place,
    /// speed, standing), in free mode.
    #[serde(default)]
    pub player: Option<FirstSplit>,
    /// In resync mode: frames on which every measure agrees, of those
    /// compared.
    pub resync: Option<(u64, u64)>,
    /// Each level from its own start, in the order the playthrough plays
    /// them.
    #[serde(default)]
    pub levels: Vec<LevelRun>,
}

/// One level of the playthrough, the rebuilt game started from the
/// original's state at the level's first frame in play.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct LevelRun {
    pub level: String,
    /// The original's frame it starts from.
    pub from: u64,
    /// Frames compared, to the next level's start.
    pub compared: u64,
    /// The first part, if any.
    pub first: Option<FirstSplit>,
    /// Why the rebuilt game couldn't start here, when it couldn't.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cant: Option<String>,
}

impl LevelRun {
    /// Frames together before the first part.
    pub fn together(&self) -> u64 {
        self.first.as_ref().map_or(self.compared, |s| s.at)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct FirstSplit {
    /// The compared frame's index: the frames together before it.
    pub at: u64,
    pub frame: u64,
    pub original_frame: u64,
    /// The level the original is in there.
    pub level: String,
    pub paths: Vec<String>,
}

pub(crate) struct Opts<'a> {
    pub playthrough: Option<String>,
    pub clips: bool,
    pub whole: bool,
    /// The game's built module, for a model with hooks.
    pub module: Option<&'a Path>,
}

/// The model's hash in a model folder, as the queue keys its scores.
pub(crate) fn model_key(mdir: &Path) -> String {
    std::fs::read_to_string(mdir.join(gp_model::MODEL_FILE))
        .map(|t| crate::scenario::model_hash(&t))
        .unwrap_or_default()
}

fn file_of(paths: &gp_evidence::Paths, tdir: &Path, run: &str) -> PathBuf {
    let t = tdir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("template");
    paths
        .home
        .join("work")
        .join("progress")
        .join(t)
        .join(format!("{run}.json"))
}

/// The kept measure for `run`, if any.
pub(crate) fn load(paths: &gp_evidence::Paths, tdir: &Path, run: Option<&str>) -> Option<Measured> {
    let f = file_of(paths, tdir, run?);
    let m: Measured = serde_json::from_slice(&std::fs::read(f).ok()?).ok()?;
    (m.version == crate::progress::VERSION).then_some(m)
}

/// The bytes left out of the NES's RAM count: the stack and the sprite
/// page.
fn skipped(a: u16) -> bool {
    (0x100..0x300).contains(&a)
}

/// The arcade board's work RAM, at E000 in the main CPU's space.
const ARCADE_RAM: u16 = 0xE000;

/// The 2 KB of work RAM the counts read, by index: the NES's $0000-$07FF,
/// the arcade board's E000-E7FF.
fn ram(o: &OriginalRuntime) -> Vec<u8> {
    let base = match o {
        OriginalRuntime::Nes(_) => 0,
        OriginalRuntime::Arcade(_) => ARCADE_RAM,
    };
    (0..0x800u16).map(|a| peek(o, base + a)).collect()
}

/// A byte as the CPU reads it, without side effects.
fn peek(o: &OriginalRuntime, a: u16) -> u8 {
    match o {
        OriginalRuntime::Nes(n) => n.nes().peek(a),
        OriginalRuntime::Arcade(r) => r.tapper().peek(a),
    }
}

/// The condition for "the player controls": `observe.toml`'s `control`,
/// else the mapping's `start`.
fn play_condition(mdir: &Path, mapping: &Mapping) -> Result<Option<Condition>> {
    let control = std::fs::read_to_string(mdir.join("observe.toml"))
        .ok()
        .and_then(|t| toml::from_str::<toml::Value>(&t).ok())
        .and_then(|v| v.get("control")?.as_str().map(str::to_owned));
    control
        .or_else(|| mapping.start.clone())
        .map(|c| Condition::parse(&c).map_err(|e| anyhow::anyhow!("the play condition {c:?}: {e}")))
        .transpose()
}

/// A start of a level: where the rebuilt game can begin, and the level.
#[derive(Clone)]
pub(crate) struct LevelStart {
    pub here: StartHere,
    pub level: String,
}

/// What one pass of the playthrough on the original gives.
pub(crate) struct Pass {
    pub frames: u64,
    pub play_frames: u64,
    pub ram: BTreeSet<u16>,
    pub kinds: BTreeSet<(u8, u8)>,
    pub code: CodeMap,
    /// Each level change: the frame and the level.
    pub levels: Vec<(u64, String)>,
    /// Each level's first frame in play the rebuilt game can start from.
    pub starts: Vec<LevelStart>,
    pub roles: Classified,
}

/// One frame of the playthrough: the NES traced for the RAM roles and its
/// code, an arcade board with its code from the instruction trace.
fn step(
    o: &mut OriginalRuntime,
    input: &gp_runtime::InputFrame,
    frame: &mut CodeMap,
    tracer: &mut Tracer,
) -> Result<bool> {
    match o {
        OriginalRuntime::Nes(n) => n
            .step_traced(input, frame, tracer)
            .map_err(anyhow::Error::msg),
        OriginalRuntime::Arcade(_) => {
            crate::covered::step_covered(o, input, frame).map_err(anyhow::Error::msg)
        }
    }
}

/// Plays `inputs` on the original, counting what the game does in play
/// and, on the NES, tracing each instruction for the RAM bytes' roles.
pub(crate) fn playthrough(
    rom: &[u8],
    mapping: &Mapping,
    play: Option<&Condition>,
    inputs: &[gp_runtime::InputFrame],
) -> Result<Pass> {
    let mut o = OriginalRuntime::of_rom(rom)?;
    let nes = o.nes().is_some();
    let mut tables = Vec::new();
    if let Some(s) = &mapping.slots {
        tables.push(s);
        tables.extend(&s.groups);
    }
    let levels: Vec<(String, Condition)> = mapping
        .slots
        .iter()
        .flat_map(|s| &s.levels)
        .filter_map(|l| Some((l.level.clone(), Condition::parse(&l.when).ok()?)))
        .collect();
    // The model's frame: from the mapping's start (else the frame before
    // the first polled one), one more each polled frame, as the lockstep
    // counts it.
    let mut model_frame = ModelFrame::new(mapping, &o)?;
    let mut prev = ram(&o);
    let mut out = Pass {
        frames: 0,
        play_frames: 0,
        ram: BTreeSet::new(),
        kinds: BTreeSet::new(),
        code: CodeMap::new(),
        levels: Vec::new(),
        starts: Vec::new(),
        roles: Classified::default(),
    };
    let mut tracer = Tracer::new();
    let mut frame = CodeMap::new();
    // The level whose start is wanted, and a copy of the original at the
    // frame before, when that frame is in play.
    let mut wanted: Option<String> = None;
    let mut here: Option<(OriginalRuntime, u64)> = None;
    for input in inputs {
        let before = here.take();
        frame.clear();
        let polled = step(&mut o, input, &mut frame, &mut tracer)?;
        out.frames += 1;
        model_frame.step(&o, polled);
        let now = ram(&o);
        let in_play = match play {
            Some(c) => holds(&o, c),
            None => polled,
        };
        if in_play {
            out.play_frames += 1;
            out.code.union(&frame);
            for a in 0..0x800u16 {
                if now[a as usize] != prev[a as usize] && !(nes && skipped(a)) {
                    out.ram.insert(a);
                }
            }
            for (t, s) in tables.iter().enumerate() {
                for i in 0..s.count {
                    if s.flag_of(i, |a| Ok(peek(&o, a))).unwrap_or(0) != 0 {
                        let id = s.id.map_or(0, |at| peek(&o, at + i));
                        out.kinds.insert((t as u8, id));
                    }
                }
            }
        }
        if let Some((l, _)) = levels.iter().find(|(_, c)| holds(&o, c))
            && out.levels.last().is_none_or(|(_, last)| last != l)
        {
            out.levels.push((o.frame(), l.clone()));
            wanted = Some(l.clone());
        }
        // A game without slot levels has one start: its first in play.
        if levels.is_empty() && out.starts.is_empty() && wanted.is_none() {
            wanted = Some(String::new());
        }
        // The frame before this polled one was in play: start there.
        if polled && let (Some(l), Some((there, mf))) = (&wanted, before) {
            out.starts.push(LevelStart {
                here: StartHere {
                    frame: there.frame(),
                    model_frame: mf,
                    original: there,
                },
                level: l.clone(),
            });
            wanted = None;
        }
        if wanted.is_some()
            && in_play
            && let Some(mf) = model_frame.frame
        {
            here = Some((o.clone(), mf));
        }
        prev = now;
    }
    if nes {
        out.roles = tracer.trace.classify();
    }
    Ok(out)
}

/// The level the original is in at `frame`, from the level changes.
fn level_at(levels: &[(u64, String)], frame: u64) -> String {
    levels
        .iter()
        .take_while(|(f, _)| *f <= frame)
        .last()
        .map(|(_, l)| l.clone())
        .unwrap_or_default()
}

/// CPU addresses in a map.
fn code_set(m: &CodeMap) -> BTreeSet<u32> {
    (0..0x1_0000u32).filter(|&a| m.get(a)).collect()
}

/// One lockstep run of the measure: the rebuilt game from the original's
/// state at a start, on `inputs` from there.
struct Job<'a> {
    start: &'a StartHere,
    inputs: &'a [gp_runtime::InputFrame],
    mode: Mode,
}

/// The playthrough: `id` (a recording, or a baseline file), else the
/// longest store recording of the game's ROMs. Its ID (a baseline's is
/// `baseline:<file>`), its name and its input.
fn pick_playthrough(
    paths: &gp_evidence::Paths,
    game: &Path,
    id: Option<&str>,
) -> Result<(String, String, gp_evidence::InputLog)> {
    let store = || gp_evidence::Store::open(&paths.store);
    let recording = |i: &str| -> Result<(String, String, gp_evidence::InputLog)> {
        let store = store()?;
        let entry = gp_evidence::recordings::resolve(&store, i)?;
        let manifest = gp_evidence::recordings::load_manifest(&store, &entry.id)?;
        let log = gp_evidence::InputLog::parse(
            &String::from_utf8(store.get(&manifest.input_log)?)
                .context("the input log isn't text")?,
        )?;
        Ok((entry.id.to_string(), entry.name.clone(), log))
    };
    let baseline = |p: &Path| -> Result<(String, String, gp_evidence::InputLog)> {
        let b = gp_evidence::baseline::Baseline::load(p)?;
        let stem = p.file_stem().and_then(|n| n.to_str()).unwrap_or("baseline");
        let name = b.description.clone().unwrap_or_else(|| stem.to_owned());
        Ok((format!("baseline:{stem}"), name, b.input()?))
    };
    if let Some(i) = id {
        let p = Path::new(i);
        return if p.is_file() {
            baseline(p)
        } else {
            recording(i)
        };
    }
    // The store's recordings of the game's ROMs, the longest; the newest
    // of those as long.
    let shas: Vec<String> = std::fs::read_to_string(game.join("roms.toml"))
        .ok()
        .and_then(|t| gp_evidence::registry::Registry::from_toml(&t, game).ok())
        .map(|r| r.sources.into_iter().map(|k| k.sha256).collect())
        .unwrap_or_default();
    let mut best: Option<(u64, String)> = None;
    if let Ok(store) = gp_evidence::Store::open_existing(&paths.store)
        && let Ok(listing) = gp_evidence::recordings::list_checked(&store)
    {
        for e in listing.entries.iter().rev() {
            let Ok(m) = gp_evidence::recordings::load_manifest(&store, &e.id) else {
                continue;
            };
            let sha = match &m.source {
                gp_evidence::manifest::SourceRef::NesRom { sha256, .. } => sha256.as_str(),
                gp_evidence::manifest::SourceRef::Romset { identity, .. } => identity.as_str(),
                gp_evidence::manifest::SourceRef::Model { .. } => continue,
            };
            if shas.iter().any(|s| s == sha) && best.as_ref().is_none_or(|b| m.frames > b.0) {
                best = Some((m.frames, e.id.to_hex()));
            }
        }
    }
    let (_, id) = best.context(
        "the store has no recording of the game's ROM: give a full playthrough with --playthrough",
    )?;
    recording(&id)
}

/// The game's corpus clips run on the original from their starts: the
/// code they run.
fn clips_code(rom: &[u8], game: &Path) -> Result<(u64, CodeMap)> {
    let set = crate::clipset::read_game(game)?;
    let logs: Vec<(gp_evidence::InputLog, usize)> = set
        .clips
        .iter()
        .filter_map(|r| crate::clipset::recipe_log(game, r).ok().flatten())
        .collect();
    let maps = crate::pool::map(
        &logs,
        || (),
        |_, _, (log, at)| -> Option<CodeMap> {
            let inputs = crate::run::frames(log);
            let mut o = OriginalRuntime::of_rom(rom).ok()?;
            o.step(&inputs[..(*at).min(inputs.len())]).ok()?;
            let mut map = CodeMap::new();
            for i in &inputs[(*at).min(inputs.len())..] {
                crate::covered::step_covered(&mut o, i, &mut map).ok()?;
            }
            Some(map)
        },
        &mut |_| {},
    );
    let mut all = CodeMap::new();
    let mut n = 0;
    for m in maps.iter().flatten() {
        all.union(m);
        n += 1;
    }
    Ok((n, all))
}

/// `gp progress --measure`: measures and keeps the result. `game` is the
/// game's folder, `mdir` the model the run plays (its working copy, or
/// the game's folder).
pub(crate) fn measure(
    paths: &gp_evidence::Paths,
    game: &Path,
    mdir: &Path,
    run: &str,
    o: &Opts,
) -> Result<Measured> {
    let t0 = std::time::Instant::now();
    let rom = crate::clip_score::rom_of(paths, game, None)?;
    let mapping = Mapping::parse(&std::fs::read_to_string(mdir.join("mapping.toml"))?)
        .map_err(anyhow::Error::msg)?;
    let (pid, pname, log) = pick_playthrough(paths, game, o.playthrough.as_deref())?;
    let inputs = crate::run::frames(&log);
    let play = play_condition(mdir, &mapping)?;
    eprintln!(
        "progress: playing {} ({} frames) on the original",
        pname,
        inputs.len()
    );
    let pass = playthrough(&rom, &mapping, play.as_ref(), &inputs)?;
    let code = code_set(&pass.code);

    // A part left out keeps what the last measure of this playthrough had.
    let before = load(paths, game, Some(run)).filter(|m| m.playthrough == pid);
    let clips = if o.clips {
        eprintln!("progress: the game's corpus clips on the original");
        let (n, all) = clips_code(&rom, game)?;
        let theirs = code_set(&all);
        Some(ClipCode {
            clips: n,
            code: theirs.len() as u64,
            shared: theirs.intersection(&code).count() as u64,
            only: theirs.difference(&code).count() as u64,
        })
    } else {
        before.as_ref().and_then(|b| b.clips.clone())
    };

    let whole = if o.whole && !pass.starts.is_empty() {
        let source = crate::hosted::Source::open_dir(mdir, None, paths, o.module)
            .map(|(s, _)| s)
            .or_else(|_| {
                crate::hosted::Source::open_dir(mdir, Some(&mdir.join("assets")), paths, o.module)
                    .map(|(s, _)| s)
            })?;
        let model = source.parse_model()?;
        let mine = gp_interp::scenario::player(&model).map(|p| format!("entities.{p}["));
        let first = &pass.starts[0].here;
        let from = |s: &StartHere| {
            usize::try_from(s.frame)
                .unwrap_or(usize::MAX)
                .min(inputs.len())
        };
        // The whole game twice, then each level to the next one's start.
        let mut jobs = vec![
            Job {
                start: first,
                inputs: &inputs[from(first)..],
                mode: Mode::Free,
            },
            Job {
                start: first,
                inputs: &inputs[from(first)..],
                mode: Mode::Resync,
            },
        ];
        for (i, s) in pass.starts.iter().enumerate() {
            let end = pass
                .starts
                .get(i + 1)
                .map_or(inputs.len(), |n| from(&n.here));
            jobs.push(Job {
                start: &s.here,
                inputs: &inputs[from(&s.here)..end.max(from(&s.here))],
                mode: Mode::Free,
            });
        }
        eprintln!(
            "progress: the rebuilt game from the original's state at frame {}: the whole playthrough twice (free and resync) and {} levels",
            first.frame,
            pass.starts.len()
        );
        let split_of = |s: &gp_interp::lockstep::Split, only: Option<&str>| {
            let paths: Vec<String> = s
                .paths
                .iter()
                .map(|p| p.path.clone())
                .filter(|p| only.is_none_or(|o| p.starts_with(o)))
                .collect();
            FirstSplit {
                at: s.at as u64,
                frame: s.frame,
                original_frame: s.original_frame,
                level: level_at(&pass.levels, s.original_frame),
                paths,
            }
        };
        // What each run keeps: its frames, its first part, the player's
        // first part, and its parts.
        type Kept = (u64, Option<FirstSplit>, Option<FirstSplit>, u64);
        let runs: Vec<std::result::Result<Kept, String>> = jobs
            .iter()
            .map(|job| {
                let ls = crate::sweep::measure::lockstep_at(
                    mdir, &source, &mapping, job.start, job.inputs, job.mode,
                )
                .map_err(|e| format!("{e:#}"))?;
                let first = ls.splits.first().map(|s| split_of(s, None));
                let player = mine.as_deref().and_then(|p| {
                    ls.splits
                        .iter()
                        .find(|s| s.paths.iter().any(|x| x.path.starts_with(p)))
                        .map(|s| split_of(s, Some(p)))
                });
                Ok((
                    ls.frames.len() as u64,
                    first,
                    player,
                    ls.splits.len() as u64,
                ))
            })
            .collect();
        let mut runs = runs.into_iter();
        let (compared, first_split, player, _) = runs
            .next()
            .context("the whole-game run")?
            .map_err(|e| anyhow::anyhow!("the whole game from frame {}: {e}", first.frame))?;
        let resync = runs
            .next()
            .context("the resync run")?
            .ok()
            .map(|(n, _, _, apart)| (n.saturating_sub(apart), n));
        let levels = pass
            .starts
            .iter()
            .zip(runs)
            .map(|(s, r)| match r {
                Ok((n, first, _, _)) => LevelRun {
                    level: s.level.clone(),
                    from: s.here.frame,
                    compared: n,
                    first,
                    cant: None,
                },
                Err(e) => LevelRun {
                    level: s.level.clone(),
                    from: s.here.frame,
                    cant: Some(e),
                    ..LevelRun::default()
                },
            })
            .collect();
        Some(Whole {
            from: first.frame,
            compared,
            first: first_split,
            player,
            resync,
            levels,
        })
    } else if o.whole {
        anyhow::bail!(
            "the playthrough never reaches a frame in play the rebuilt game can start from"
        );
    } else {
        before.as_ref().and_then(|b| b.whole.clone())
    };

    let mut seen_levels: Vec<String> = Vec::new();
    for (_, l) in &pass.levels {
        if !seen_levels.contains(l) {
            seen_levels.push(l.clone());
        }
    }
    let m = Measured {
        version: crate::progress::VERSION.into(),
        playthrough: pid,
        name: pname,
        frames: pass.frames,
        play_frames: pass.play_frames,
        ram: pass.ram.into_iter().collect(),
        kinds: pass.kinds.into_iter().collect(),
        code: code.len() as u64,
        levels: seen_levels,
        clips,
        whole,
        roles: pass.roles,
        model: model_key(mdir),
        seconds: t0.elapsed().as_secs_f64(),
    };
    let file = file_of(paths, game, run);
    std::fs::create_dir_all(file.parent().unwrap_or(Path::new(".")))?;
    std::fs::write(&file, serde_json::to_vec_pretty(&m)?)
        .with_context(|| format!("writing {}", file.display()))?;
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_level_at_a_frame_is_the_last_change_before_it() {
        let l = vec![(10, "w1_1".to_owned()), (500, "w1_2".to_owned())];
        assert_eq!(level_at(&l, 5), "");
        assert_eq!(level_at(&l, 10), "w1_1");
        assert_eq!(level_at(&l, 499), "w1_1");
        assert_eq!(level_at(&l, 900), "w1_2");
    }

    #[test]
    fn the_stack_and_sprite_pages_are_left_out() {
        assert!(skipped(0x100) && skipped(0x1FF) && skipped(0x200) && skipped(0x2FF));
        assert!(!skipped(0xFF) && !skipped(0x300));
    }

    #[test]
    fn a_measure_of_another_version_is_not_read() {
        let dir = tempfile::tempdir().unwrap();
        let paths = gp_evidence::Paths {
            home: dir.path().to_owned(),
            store: dir.path().join("store"),
            roms: dir.path().join("roms"),
        };
        let t = dir.path().join("mosshop");
        let f = file_of(&paths, &t, "hop");
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        let mut m = Measured {
            version: crate::progress::VERSION.into(),
            ..Measured::default()
        };
        std::fs::write(&f, serde_json::to_vec(&m).unwrap()).unwrap();
        assert_eq!(load(&paths, &t, Some("hop")), Some(m.clone()));
        m.version = "progress 0".into();
        std::fs::write(&f, serde_json::to_vec(&m).unwrap()).unwrap();
        assert_eq!(load(&paths, &t, Some("hop")), None);
        assert_eq!(load(&paths, &t, None), None);
    }
}
