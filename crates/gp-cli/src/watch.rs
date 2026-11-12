//! `gp watch`: stage 0 and its search on a ROM, written as they go to a
//! watch session that `gp watch events`, `gp serve` and `gp mcp` follow
//! (docs/watch.md; docs/research/watch-ui.md, section 6).
//!
//! A session is `$GP_HOME/watch/NAME/`: `session.json` (the ROM and the
//! options), `events.ndjson` (gp-events' watch format, one writer holding
//! `writer.lock`, `seq` with no gaps, each line one write so a reader never
//! sees half of one), the harvests a `snapshot` names (`static-0.json`,
//! `unread-0.json` from the ROM alone, `static.json` and `unread.json` with
//! the runs) and `clips/clips.toml`, the clips the search kept. The file
//! holds no wall time: the same ROM and options give the same file.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use gp_events::edges::EdgeKey;
use gp_events::names::Namer;
use gp_events::watch::{ExploreOpts, SCHEMA, StartClip, Stop, WatchBody, WatchEvent};
use gp_events::{Bits, Collector, FrameEvents, Levels};
use gp_evidence::Paths;
use gp_explore::Progress;
use gp_explore::presses::spans;
use serde_json::{Value, json};
use sha2::Digest;

use crate::Outcome;
use crate::explore::ExploreArgs;
use crate::reach::ClipInput;

#[derive(Subcommand)]
pub enum WatchCmd {
    /// Open a session and run stage 0 in it: read the ROM alone, run the
    /// start clips (stage 0's own starts, and `--start`'s), writing each
    /// frame's novelty (code, banks, edges, pointer values, stops passed)
    /// as it comes, read again with the runs, then search as `gp explore`
    /// does and write each clip it keeps with its novelty. The events go to
    /// $GP_HOME/watch/NAME/events.ndjson.
    Start(Box<WatchStartArgs>),
    /// Print a session's events after `--after` (a `seq`), one JSON line
    /// each. With `--follow`, keep printing new ones as the writer adds
    /// them, until it is done.
    Events {
        name: String,
        #[arg(long, default_value_t = 0)]
        after: u64,
        #[arg(long)]
        follow: bool,
        /// Only these kinds (`bank`, `stop.reached`; repeat).
        #[arg(long = "kind")]
        kinds: Vec<String>,
    },
    /// List the sessions, with their ROM and how many events each has.
    List,
    /// Search again from the session's start clips (as `gp explore`,
    /// aimed at the stops left by default), writing into the session.
    Explore(WatchExploreArgs),
    /// Run the session's recipes again watching addresses (`--watch $F0`)
    /// and hooking instructions (`--hook 01:8000`, `--hook sub_C1FB`),
    /// writing each write and call as `watch` and `hook` events (at most
    /// 200 for each in each recipe; the rest are counted).
    Rerun {
        name: String,
        #[arg(long = "watch", value_name = "ADDR")]
        watch: Vec<String>,
        #[arg(long = "hook", value_name = "AT")]
        hooks: Vec<String>,
    },
    /// L3 for frames A..B (or A..=B) of a recipe the session ran, as `gp
    /// reach --detail` gives it.
    Detail {
        name: String,
        #[arg(long)]
        recipe: String,
        #[arg(long)]
        frames: String,
    },
    /// What is under a box of the screen (`--rect X,Y,W,H` in picture
    /// pixels) at a frame of a recipe: the sprites there and the routine
    /// that last wrote each one's bytes in the sprite table.
    Point {
        name: String,
        #[arg(long)]
        recipe: String,
        #[arg(long)]
        frame: u64,
        #[arg(long)]
        rect: String,
    },
}

#[derive(Args, Debug, Clone)]
pub struct WatchStartArgs {
    /// The session's name: letters, digits, - and _.
    pub name: String,
    #[command(flatten)]
    pub explore: ExploreArgs,
    /// Replace a session of this name.
    #[arg(long)]
    pub replace: bool,
}

/// The folder sessions live in.
pub(crate) fn sessions_dir(home: &Path) -> PathBuf {
    home.join("watch")
}

/// A recipe a session kept (`recipes/HASH.presses`), as a clip from
/// power-on.
pub(crate) fn recipe(dir: &Path, hash: &str) -> Result<ClipInput> {
    if hash.len() != 64 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("a recipe is its 64-digit hash");
    }
    let text = fs::read_to_string(dir.join("recipes").join(format!("{hash}.presses")))
        .with_context(|| format!("the session has no recipe {hash}"))?;
    let crate::clipset::Presses { frames, spans, .. } = crate::clipset::parse_presses(&text)?;
    let refs: Vec<&str> = spans.iter().map(String::as_str).collect();
    let inputs = gp_evidence::input_log::generate(usize::try_from(frames)?, &refs)?.frames;
    Ok(ClipInput {
        set: "watch".into(),
        id: hash.to_owned(),
        power_on: Default::default(),
        inputs,
    })
}

/// A session's folder, refusing a name that could reach outside it.
pub(crate) fn session_dir(home: &Path, name: &str) -> Result<PathBuf> {
    if name.is_empty()
        || name.len() > 80
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        bail!("a session name is letters, digits, - and _ (got {name:?})");
    }
    Ok(sessions_dir(home).join(name))
}

pub(crate) fn run(cmd: &WatchCmd, paths: &Paths) -> Result<Outcome> {
    match cmd {
        WatchCmd::Start(a) => start(a, paths),
        WatchCmd::Events {
            name,
            after,
            follow,
            kinds,
        } => events(&session_dir(&paths.home, name)?, *after, *follow, kinds),
        WatchCmd::List => list(paths),
        WatchCmd::Explore(a) => explore_more(a, paths),
        WatchCmd::Rerun { name, watch, hooks } => rerun(paths, name, watch, hooks),
        WatchCmd::Detail {
            name,
            recipe,
            frames,
        } => detail(paths, name, recipe, frames),
        WatchCmd::Point {
            name,
            recipe,
            frame,
            rect,
        } => {
            let r: Vec<u16> = rect
                .split(',')
                .map(|x| x.trim().parse::<u16>())
                .collect::<std::result::Result<_, _>>()
                .context("--rect is X,Y,W,H")?;
            anyhow::ensure!(r.len() == 4, "--rect is X,Y,W,H");
            let v = point(
                &session_dir(&paths.home, name)?,
                recipe,
                *frame,
                (r[0], r[1], r[2], r[3]),
            )?;
            let text = v["words"]
                .as_array()
                .map(|w| {
                    w.iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default();
            Ok(Outcome::ok(v, text))
        }
    }
}

/// The one writer of a session's events.
struct Writer {
    dir: PathBuf,
    file: File,
    seq: u64,
    _lock: gp_capture::WriterLock,
}

impl Writer {
    /// Keeps `clip`'s input as `recipes/HASH.presses`, so a reader can run
    /// the recipe an event names again (`/detail`). A recipe with a reset
    /// or a second controller isn't kept: a presses file can't say it.
    fn recipe(&self, clip: &ClipInput, hash: &str) -> Result<()> {
        if clip.inputs.iter().any(|i| i.reset || i.pads[1] != 0) {
            return Ok(());
        }
        let d = self.dir.join("recipes");
        fs::create_dir_all(&d)?;
        let pads: Vec<u8> = clip.inputs.iter().map(|i| i.pads[0]).collect();
        fs::write(
            d.join(format!("{hash}.presses")),
            crate::clipset::presses_text(&crate::clipset::Presses {
                frames: pads.len() as u64,
                spans: spans(&pads),
                arcade: None,
            }),
        )?;
        Ok(())
    }

    fn emit(
        &mut self,
        run: Option<&str>,
        frame: Option<u64>,
        level: Option<u8>,
        body: WatchBody,
    ) -> Result<()> {
        let ev = WatchEvent {
            v: SCHEMA,
            seq: self.seq + 1,
            run: run.map(str::to_owned),
            frame,
            level,
            body,
        };
        let mut line = serde_json::to_string(&ev)?;
        line.push('\n');
        // One write a line, so a reader sees whole lines or none.
        self.file.write_all(line.as_bytes())?;
        self.file.flush()?;
        self.seq += 1;
        Ok(())
    }
}

/// What the session has seen so far, so only what is new is written.
struct Seen {
    namer: Namer,
    board: gp_nes::codemap::Board,
    code: Bits,
    edges: BTreeSet<EdgeKey>,
    banks: BTreeSet<u32>,
    pointers: BTreeSet<(u16, u16)>,
    /// Stops not passed yet: the stop and the CPU address it goes to.
    stops: Vec<(Stop, u16)>,
}

impl Seen {
    /// The CPU address PRG offset `o` ran at, from L0's windows.
    fn cpu_of(&self, o: u32, windows: &[u64]) -> u16 {
        let page = windows.get(o as usize / 0x400).copied().unwrap_or(0);
        let cp = if page == 0 {
            0
        } else {
            63 - page.leading_zeros()
        };
        ((cp << 10) | (o & 0x3FF)) as u16
    }

    /// Writes what frame `f` of the run `recipe` did that the session hadn't
    /// seen: new code, the first code in a bank, new edges, new pointer
    /// values, and the stops it passed.
    fn frame(
        &mut self,
        w: &mut Writer,
        recipe: &str,
        f: &FrameEvents,
        c: &Collector,
    ) -> Result<()> {
        let at = Some(f.frame);
        let mut ranges: Vec<(u32, u32)> = Vec::new();
        for &(a, b) in &f.new_code {
            for o in a..b {
                if self.code.contains(o as usize) {
                    continue;
                }
                self.code.set(o as usize);
                match ranges.last_mut() {
                    Some(r) if r.1 == o => r.1 += 1,
                    _ => ranges.push((o, o + 1)),
                }
            }
        }
        if !ranges.is_empty() {
            let windows = &c.footprint.windows;
            let names = ranges
                .iter()
                .map(|r| self.namer.name(r.0, self.cpu_of(r.0, windows)))
                .collect();
            w.emit(
                Some(recipe),
                at,
                Some(2),
                WatchBody::NewCode {
                    ranges: ranges.clone(),
                    at: names,
                },
            )?;
            for r in &ranges {
                let cpu = self.cpu_of(r.0, windows);
                if let Some((_, size)) = self.board.region(cpu) {
                    let bank = r.0 / size;
                    if self.banks.insert(bank) {
                        w.emit(
                            Some(recipe),
                            at,
                            Some(2),
                            WatchBody::Bank {
                                bank,
                                at: self.namer.name(r.0, cpu),
                            },
                        )?;
                    }
                }
            }
        }
        let Some(table) = c.edges.as_ref().map(|e| &e.table) else {
            return Ok(());
        };
        for k in &f.new_edges {
            if !self.edges.insert(*k) {
                continue;
            }
            let Some(st) = table.get(k) else { continue };
            let from = self.namer.name(k.1, st.from_cpu);
            let to = self.namer.name(k.2, st.to_cpu);
            w.emit(
                Some(recipe),
                at,
                Some(2),
                WatchBody::Edge {
                    edge: k.0,
                    from: from.clone(),
                    to: to.clone(),
                },
            )?;
            for &(ptr, value) in &st.pointers {
                if self.pointers.insert((ptr, value)) {
                    w.emit(
                        Some(recipe),
                        at,
                        Some(2),
                        WatchBody::Pointer {
                            ptr,
                            value,
                            to: to.clone(),
                        },
                    )?;
                }
            }
            // A stop is passed when its transfer is taken to where it goes.
            let mut passed = Vec::new();
            self.stops.retain(|(s, target)| {
                let hit = s.pc == from && st.to_cpu == *target;
                if hit {
                    passed.push(s.clone());
                }
                !hit
            });
            for s in passed {
                w.emit(
                    Some(recipe),
                    at,
                    Some(2),
                    WatchBody::StopReached {
                        stop: s.id,
                        routine: format!("sub_{}", to.replace(':', "_")),
                    },
                )?;
            }
        }
        Ok(())
    }

    /// Runs `clip` with L2 and writes its novelty; returns its levels.
    fn run(&mut self, w: &mut Writer, rom: &[u8], clip: &ClipInput) -> Result<Levels> {
        let recipe = clip.key()?;
        w.recipe(clip, &recipe)?;
        let mut err = None;
        let run = gp_events::run_frames_with(
            rom,
            clip.power_on,
            &clip.inputs,
            &BTreeSet::new(),
            &[],
            |f, c| {
                if err.is_none()
                    && let Err(e) = self.frame(w, &recipe, &f, c)
                {
                    err = Some(e);
                }
            },
        )?;
        if let Some(e) = err {
            return Err(e);
        }
        Ok(Levels::of(&run.collector, run.frames))
    }
}

/// The console's state hash after the last frame of `clip`'s recipe, as
/// gp-play gives it (`Nes::state_or_board_free_hash`), for the page's player
/// to check its replay against.
fn end_hash(rom: &[u8], clip: &ClipInput) -> Result<String> {
    let mut nes = gp_nes::Nes::power_on(rom, clip.power_on)?;
    for i in &clip.inputs {
        nes.step(*i)?;
    }
    Ok(nes.state_or_board_free_hash()?.to_hex())
}

/// A harvest written into the session as `static{tag}.json` and
/// `unread{tag}.json`, with each file's BLAKE3 hash.
fn write_harvest(
    dir: &Path,
    tag: &str,
    h: &gp_nes::harvest::Harvest,
    u: &gp_nes::harvest::unread::Unread,
) -> Result<BTreeMap<String, String>> {
    let mut files = BTreeMap::new();
    for (name, text) in [
        (
            format!("static{tag}.json"),
            serde_json::to_string_pretty(h)? + "\n",
        ),
        (
            format!("unread{tag}.json"),
            serde_json::to_string_pretty(u)? + "\n",
        ),
    ] {
        fs::write(dir.join(&name), &text)?;
        files.insert(name, gp_evidence::Hash::of(text.as_bytes()).to_string());
    }
    Ok(files)
}

fn stops_of(u: &gp_nes::harvest::unread::Unread) -> Vec<Stop> {
    u.stops
        .iter()
        .map(|s| Stop {
            id: s.id.clone(),
            pc: s.pc.clone(),
            target: (!s.target.is_empty()).then(|| s.target.clone()),
            behind: s.behind,
        })
        .collect()
}

/// A routine read now that wasn't before, as `fact` events.
fn facts(
    w: &mut Writer,
    before: &gp_nes::harvest::unread::Unread,
    after: &gp_nes::harvest::unread::Unread,
) -> Result<()> {
    let now: BTreeSet<&String> = after.unread_routines.iter().collect();
    for r in &before.unread_routines {
        if !now.contains(r) {
            w.emit(
                None,
                None,
                None,
                WatchBody::Fact {
                    what: "routine read".into(),
                    routine: r.clone(),
                },
            )?;
        }
    }
    Ok(())
}

fn start(a: &WatchStartArgs, paths: &Paths) -> Result<Outcome> {
    let t0 = Instant::now();
    let e = &a.explore;
    let dir = session_dir(&paths.home, &a.name)?;
    if dir.join("events.ndjson").exists() {
        if !a.replace {
            bail!(
                "a session {} exists; pass --replace to start it again",
                a.name
            );
        }
        if gp_capture::writer_busy(&dir) {
            bail!("session {} is being written now", a.name);
        }
        fs::remove_dir_all(&dir)?;
    }
    fs::create_dir_all(&dir)?;
    let lock = gp_capture::take_writer(&dir)?;
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("events.ndjson"))?;
    let mut w = Writer {
        dir: dir.clone(),
        file,
        seq: 0,
        _lock: lock,
    };
    let rom = fs::read(&e.rom).with_context(|| format!("reading {}", e.rom.display()))?;
    crate::original::nes_only(crate::roms::Platform::of_file(&rom), "gp watch's stage 0")?;
    let (prg, mapper) = gp_nes::harvest::ines_prg(&rom).context("not an iNES file")?;
    let board = gp_nes::codemap::Board::of(prg, mapper);
    let sets: Vec<PathBuf> = e.start.iter().cloned().collect();
    let clips = e.starts().with(&sets, paths)?;
    let aim = if e.new_code {
        "new-code"
    } else if !e.targets.is_empty() {
        "named"
    } else {
        e.aim.as_str()
    };
    let starts: Vec<StartClip> = clips
        .iter()
        .map(|c| {
            Ok(StartClip {
                id: c.id.clone(),
                recipe: c.key()?,
                frames: c.inputs.len() as u64,
            })
        })
        .collect::<Result<_>>()?;
    let rom_sha = hex::encode(sha2::Sha256::digest(&rom));
    fs::write(
        dir.join("session.json"),
        serde_json::to_string_pretty(&json!({
            "name": a.name,
            "rom": e.rom,
            "rom_sha256": rom_sha,
            "starts": starts,
            "explore": {"aim": aim, "targets": e.targets, "budget": e.budget, "seed": e.seed,
                        "start_runs": e.start_runs, "start_seed": e.start_seed},
        }))? + "\n",
    )?;
    w.emit(
        None,
        None,
        None,
        WatchBody::Start {
            rom_sha256: rom_sha,
            mapper,
            prg_bytes: prg.len(),
            starts,
            explore: ExploreOpts {
                aim: aim.to_owned(),
                targets: e.targets.clone(),
                budget: e.budget,
                seed: e.seed,
            },
        },
    )?;
    // What the ROM alone gives the reader.
    let (h0, u0) = crate::model_static::harvest_seeded(&e.rom, None, true, None)?;
    let u0 = u0.context("the reader gives an unread report")?;
    let files = write_harvest(&dir, "-0", &h0, &u0)?;
    w.emit(
        None,
        None,
        None,
        WatchBody::Snapshot {
            files,
            routines: u0.routines,
            read: u0.read,
            stops: stops_of(&u0),
        },
    )?;
    let mut seen = Seen {
        namer: Namer::of_board(board.clone()),
        board: board.clone(),
        code: Bits::new(prg.len()),
        edges: BTreeSet::new(),
        banks: BTreeSet::new(),
        pointers: BTreeSet::new(),
        stops: stops_of(&u0)
            .into_iter()
            .filter_map(|s| {
                let t = u16::from_str_radix(s.target.as_deref()?, 16).ok()?;
                Some((s, t))
            })
            .collect(),
    };
    // The start clips, frame by frame.
    let mut known: Option<Levels> = None;
    for c in &clips {
        let l = seen.run(&mut w, &rom, c)?;
        match &mut known {
            None => known = Some(l),
            Some(k) => k.merge(&l).map_err(anyhow::Error::msg)?,
        }
    }
    let mut known = known.context("no start clip")?;
    // Read again with the runs.
    let (h1, u1) = crate::model_static::harvest_seeded(&e.rom, None, true, Some(&known))?;
    let u1 = u1.context("the reader gives an unread report")?;
    facts(&mut w, &u0, &u1)?;
    let files = write_harvest(&dir, "", &h1, &u1)?;
    w.emit(
        None,
        None,
        None,
        WatchBody::Snapshot {
            files,
            routines: u1.routines,
            read: u1.read,
            stops: stops_of(&u1),
        },
    )?;
    let (explored, u2) = search_into(&mut w, &mut seen, e, &rom, &dir, clips, &mut known, &u1)?;
    let secs = t0.elapsed().as_secs_f64();
    Ok(Outcome::ok(
        json!({"name": a.name, "dir": dir, "events": w.seq, "kept": explored.kept.len(),
               "reached": explored.reached, "read": u2.read, "routines": u2.routines}),
        format!(
            "session {}: {} events in {}; {} of {} routines read; kept {} clips, reached {} ({}); {:.1} s",
            a.name,
            w.seq,
            dir.join("events.ndjson").display(),
            u2.read,
            u2.routines,
            explored.kept.len(),
            explored.reached.len(),
            explored.reached.join(", "),
            secs
        ),
    ))
}

/// Searches as `gp explore` does from `clips` (whose merged runs are
/// `known`), writing its progress and each kept clip with its novelty, then
/// the behaviour of the start and kept clips, what the reader reads now
/// that it didn't in `before`, and `watch.end`. The kept clips go to
/// `clips/` in the session.
#[allow(clippy::too_many_arguments)]
fn search_into(
    w: &mut Writer,
    seen: &mut Seen,
    e: &ExploreArgs,
    rom: &[u8],
    dir: &Path,
    clips: Vec<ClipInput>,
    known: &mut Levels,
    before: &gp_nes::harvest::unread::Unread,
) -> Result<(crate::explore::Explored, gp_nes::harvest::unread::Unread)> {
    // The search, its kept clips with their novelty as they are kept.
    let start_clips: Vec<ClipInput> = clips.clone();
    let mut err: Option<anyhow::Error> = None;
    let mut last_kept = 0usize;
    let mut kept_n = 0usize;
    let explored = {
        let w = &mut *w;
        let seen = &mut *seen;
        let mut on = |p: Progress| {
            if err.is_some() {
                return;
            }
            let r = (|| -> Result<()> {
                match p {
                    Progress::Ran {
                        ran,
                        budget,
                        best,
                        kept,
                    } => {
                        // Every tenth candidate, and each that kept one.
                        if ran % 10 == 0 || kept != last_kept || ran == budget {
                            last_kept = kept;
                            w.emit(
                                None,
                                None,
                                None,
                                WatchBody::ExploreProgress {
                                    ran,
                                    budget,
                                    kept,
                                    best,
                                },
                            )?;
                        }
                    }
                    Progress::Kept { found, .. } => {
                        let from = &start_clips[found.start];
                        let mut clip = from.clone();
                        clip.inputs
                            .extend(gp_explore::presses::log_frames(&found.pads));
                        clip.id = format!("explore.s{}.k{kept_n}", e.seed);
                        kept_n += 1;
                        let recipe = clip.key()?;
                        let end_hash = end_hash(rom, &clip)?;
                        w.emit(
                            Some(&recipe),
                            None,
                            None,
                            WatchBody::ClipKept {
                                id: clip.id.clone(),
                                start: from.id.clone(),
                                presses: spans(&found.pads),
                                frames: found.pads.len(),
                                recipe: recipe.clone(),
                                why: crate::explore::why(found),
                                end_hash,
                            },
                        )?;
                        seen.run(w, rom, &clip)?;
                    }
                }
                Ok(())
            })();
            if let Err(x) = r {
                err = Some(x);
            }
        };
        crate::explore::explore_with(e, &e.rom, rom, clips, known, &mut on)?
    };
    if let Some(x) = err {
        return Err(x);
    }
    for f in &explored.kept {
        known.merge(&f.levels).map_err(anyhow::Error::msg)?;
    }
    crate::explore::write_clips(&dir.join("clips"), e, &explored)?;
    // Behaviour, from the start and kept clips' runs.
    let mut all = explored.start_clips.clone();
    for (i, f) in explored.kept.iter().enumerate() {
        let mut c = explored.start_clips[f.start].clone();
        c.inputs.extend(gp_explore::presses::log_frames(&f.pads));
        c.id = format!("explore.s{}.{i}", e.seed);
        all.push(c);
    }
    let b = crate::behave::build(rom, &all)?;
    for o in crate::behave::open_facts(&b) {
        w.emit(
            None,
            None,
            None,
            WatchBody::Behaviour {
                what: o.what,
                routine: o.routine,
                why: o.why,
            },
        )?;
    }
    let (_, u2) = crate::model_static::harvest_seeded(&e.rom, None, true, Some(known))?;
    let u2 = u2.context("the reader gives an unread report")?;
    facts(w, before, &u2)?;
    w.emit(
        None,
        None,
        None,
        WatchBody::End {
            ran: e.budget,
            kept: explored.kept.len(),
            reached: explored.reached.clone(),
            read: u2.read,
            routines: u2.routines,
        },
    )?;
    Ok((explored, u2))
}

/// Whether event `v` is one of `kinds` (all, with none named).
fn wanted(v: &Value, kinds: &[String]) -> bool {
    kinds.is_empty()
        || v["kind"]
            .as_str()
            .is_some_and(|k| kinds.iter().any(|x| x == k))
}

fn events(dir: &Path, after: u64, follow: bool, kinds: &[String]) -> Result<Outcome> {
    let file = dir.join("events.ndjson");
    if !follow {
        if !file.is_file() {
            bail!("no watch session at {}", dir.display());
        }
        let evs: Vec<Value> = gp_capture::events_after(dir, after, usize::MAX)?
            .into_iter()
            .filter(|v| wanted(v, kinds))
            .collect();
        let text = evs
            .iter()
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        return Ok(Outcome::ok(json!({"events": evs}), text));
    }
    // Follow: read only the bytes added since the last look, print each
    // whole line after `after`, and stop once the writer is gone and the
    // file has no more.
    if !dir.is_dir() {
        bail!("no watch session at {}", dir.display());
    }
    let mut tail = gp_capture::Tail::default();
    let mut printed = 0u64;
    let out = std::io::stdout();
    loop {
        let busy = gp_capture::writer_busy(dir);
        for (v, line) in tail.read(&file)? {
            if v["seq"].as_u64().is_some_and(|s| s > after) && wanted(&v, kinds) {
                let mut o = out.lock();
                writeln!(o, "{line}")?;
                o.flush()?;
                printed += 1;
            }
        }
        // Once the writer lets go, read what it wrote last, then stop.
        if !busy && file.is_file() && fs::metadata(&file)?.len() == tail.offset() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(Outcome::ok(json!({"printed": printed}), String::new()))
}

/// Every session, by name: its ROM, its events and whether a writer has it.
pub(crate) fn sessions(paths: &Paths) -> Result<Vec<Value>> {
    let mut rows = Vec::new();
    if let Ok(rd) = fs::read_dir(sessions_dir(&paths.home)) {
        let mut names: Vec<String> = rd
            .filter_map(|e| e.ok())
            .filter(|e| e.path().join("session.json").is_file())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        for n in names {
            let d = sessions_dir(&paths.home).join(&n);
            let s: Value = serde_json::from_str(&fs::read_to_string(d.join("session.json"))?)?;
            rows.push(json!({
                "name": n,
                "rom": s["rom"],
                "events": gp_capture::last_seq(&d)?,
                "writing": gp_capture::writer_busy(&d),
            }));
        }
    }
    Ok(rows)
}

fn list(paths: &Paths) -> Result<Outcome> {
    let rows = sessions(paths)?;
    let text = rows
        .iter()
        .map(|r| {
            format!(
                "{}  {} events{}  {}",
                r["name"].as_str().unwrap_or(""),
                r["events"],
                if r["writing"].as_bool() == Some(true) {
                    " (writing)"
                } else {
                    ""
                },
                r["rom"].as_str().unwrap_or("")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    Ok(Outcome::ok(json!({"sessions": rows}), text))
}

impl Writer {
    /// Reopens a session to append to its events, taking its writer lock.
    fn append(dir: &Path) -> Result<Writer> {
        if !dir.join("session.json").is_file() {
            bail!("no watch session at {}", dir.display());
        }
        let lock = gp_capture::take_writer(dir)?;
        let seq = gp_capture::last_seq(dir)?;
        let file = OpenOptions::new()
            .append(true)
            .open(dir.join("events.ndjson"))?;
        Ok(Writer {
            dir: dir.to_owned(),
            file,
            seq,
            _lock: lock,
        })
    }
}

/// A session as its files hold it: the ROM, its board, the start clips and
/// the clips its searches kept, each a recipe the session ran.
pub(crate) struct Session {
    pub rom_path: PathBuf,
    pub rom: Vec<u8>,
    pub board: gp_nes::codemap::Board,
    pub starts: Vec<ClipInput>,
    pub kept: Vec<ClipInput>,
}

pub(crate) fn load_session(dir: &Path) -> Result<Session> {
    let s: Value = serde_json::from_str(
        &fs::read_to_string(dir.join("session.json"))
            .with_context(|| format!("no watch session at {}", dir.display()))?,
    )?;
    let rom_path = PathBuf::from(s["rom"].as_str().context("the session names no ROM")?);
    let rom = fs::read(&rom_path).with_context(|| format!("reading {}", rom_path.display()))?;
    let (prg, mapper) = gp_nes::harvest::ines_prg(&rom).context("not an iNES file")?;
    let board = gp_nes::codemap::Board::of(prg, mapper);
    let named = |id: &str, hash: &str| -> Result<ClipInput> {
        let mut c = recipe(dir, hash)?;
        c.id = id.to_owned();
        Ok(c)
    };
    let mut starts = Vec::new();
    for st in s["starts"].as_array().into_iter().flatten() {
        starts.push(named(
            st["id"].as_str().unwrap_or(""),
            st["recipe"].as_str().unwrap_or(""),
        )?);
    }
    let mut kept = Vec::new();
    for e in gp_capture::events_after(dir, 0, usize::MAX)? {
        if e["kind"] == "clip.kept" {
            kept.push(named(
                e["data"]["id"].as_str().unwrap_or(""),
                e["data"]["recipe"].as_str().unwrap_or(""),
            )?);
        }
    }
    Ok(Session {
        rom_path,
        rom,
        board,
        starts,
        kept,
    })
}

impl Seen {
    /// What runs already showed: their code, edges, banks and pointer
    /// values, with the stops of `u` not passed yet.
    fn from_known(
        board: &gp_nes::codemap::Board,
        known: &Levels,
        u: &gp_nes::harvest::unread::Unread,
    ) -> Seen {
        let mut banks = BTreeSet::new();
        for (p, &cpus) in known.footprint.windows.iter().enumerate() {
            for cp in 0..64u32 {
                if cpus & (1u64 << cp) != 0
                    && let Some((_, size)) = board.region((cp << 10) as u16)
                {
                    banks.insert((p as u32 * 1024) / size);
                }
            }
        }
        Seen {
            namer: Namer::of_board(board.clone()),
            board: board.clone(),
            code: known.footprint.executed.clone(),
            edges: known.edges.iter().map(|r| (r.kind, r.from, r.to)).collect(),
            banks,
            pointers: known
                .edges
                .iter()
                .flat_map(|r| r.stat.pointers.iter().copied())
                .collect(),
            stops: stops_of(u)
                .into_iter()
                .filter_map(|s| {
                    let t = u16::from_str_radix(s.target.as_deref()?, 16).ok()?;
                    Some((s, t))
                })
                .collect(),
        }
    }
}

#[derive(Args, Debug, Clone)]
pub struct WatchExploreArgs {
    pub name: String,
    /// Routines to aim at, by the harvest's names (`sub_01_8000`).
    #[arg(long = "target")]
    pub targets: Vec<String>,
    /// `stops`, `unrun` or `unread`, as `gp explore --aim` takes it.
    #[arg(long, default_value = "stops")]
    pub aim: String,
    /// Aim at new code only.
    #[arg(long, conflicts_with = "targets")]
    pub new_code: bool,
    #[arg(long, default_value_t = 100)]
    pub budget: usize,
    #[arg(long, default_value_t = 2)]
    pub seed: u64,
    #[arg(long, default_value_t = 60)]
    pub min_frames: usize,
    #[arg(long, default_value_t = 180)]
    pub max_frames: usize,
    #[arg(long, default_value_t = 8)]
    pub keep: usize,
}

/// `gp watch explore`: another search from the session's start clips,
/// written into the same session.
fn explore_more(a: &WatchExploreArgs, paths: &Paths) -> Result<Outcome> {
    let dir = session_dir(&paths.home, &a.name)?;
    let sess = load_session(&dir)?;
    let mut w = Writer::append(&dir)?;
    let store = gp_events::store::Store::at(paths.home.join("work").join("events"));
    let mut all = sess.starts.clone();
    all.extend(sess.kept.iter().cloned());
    let mut known = crate::reach::run_clips(
        &sess.rom,
        &all,
        &crate::reach::Keep {
            store: Some(&store),
            ..Default::default()
        },
    )?
    .levels;
    let (_, u) = crate::model_static::harvest_seeded(&sess.rom_path, None, true, Some(&known))?;
    let u = u.context("the reader gives an unread report")?;
    let mut seen = Seen::from_known(&sess.board, &known, &u);
    let e = ExploreArgs {
        rom: sess.rom_path.clone(),
        start: None,
        start_runs: 0,
        start_seed: 0,
        start_frames: 0,
        no_starts: false,
        from: Vec::new(),
        targets: a.targets.clone(),
        aim: a.aim.clone(),
        new_code: a.new_code,
        budget: a.budget,
        seed: a.seed,
        min_frames: a.min_frames,
        max_frames: a.max_frames,
        keep: a.keep,
        write: None,
        store: None,
    };
    let (ex, u2) = search_into(
        &mut w,
        &mut seen,
        &e,
        &sess.rom,
        &dir,
        sess.starts.clone(),
        &mut known,
        &u,
    )?;
    Ok(Outcome::ok(
        json!({"name": a.name, "events": w.seq, "kept": ex.kept.len(), "targets": ex.targets,
               "reached": ex.reached, "read": u2.read, "routines": u2.routines}),
        format!(
            "session {}: search {} of {} targets reached ({}); kept {}; {} of {} routines read",
            a.name,
            ex.reached.len(),
            ex.targets.len(),
            ex.reached.join(", "),
            ex.kept.len(),
            u2.read,
            u2.routines
        ),
    ))
}

/// The PRG offset of an instruction named as the harvest names it:
/// `01:8000`, `C27D` or `sub_01_8000`.
fn parse_at(board: &gp_nes::codemap::Board, s: &str) -> Result<usize> {
    let t = s.trim().trim_start_matches("sub_");
    let t = match t.split_once('_') {
        Some((b, a)) if b.len() <= 2 => format!("{b}:{a}"),
        _ => t.to_owned(),
    };
    let v = gp_nes::codemap::va_parse(&t).with_context(|| format!("{s:?} isn't an address"))?;
    board
        .prg_offset(v)
        .with_context(|| format!("{s:?} isn't in PRG ROM on this board"))
}

/// The most `watch` or `hook` events a rerun writes for one address or
/// hook in one recipe; the rest are counted.
const RERUN_CAP: usize = 200;

/// `gp watch rerun`: the session's recipes run again, watching `watch`
/// and hooking `hooks`, the hits written as `watch` and `hook` events.
fn rerun(paths: &Paths, name: &str, watch: &[String], hooks: &[String]) -> Result<Outcome> {
    let dir = session_dir(&paths.home, name)?;
    let sess = load_session(&dir)?;
    let addrs: BTreeSet<u16> = watch
        .iter()
        .map(|a| crate::reach::parse_addr(a))
        .collect::<Result<_>>()?;
    let at: Vec<usize> = hooks
        .iter()
        .map(|h| parse_at(&sess.board, h))
        .collect::<Result<_>>()?;
    let mut w = Writer::append(&dir)?;
    let namer = Namer::of_board(sess.board.clone());
    let mut all = sess.starts.clone();
    all.extend(sess.kept.iter().cloned());
    w.emit(
        None,
        None,
        None,
        WatchBody::Rerun {
            watch: addrs.iter().map(|a| format!("{a:04X}")).collect(),
            hooks: hooks.to_vec(),
            recipes: all.len(),
        },
    )?;
    let (mut watch_hits, mut hook_hits, mut written) = (0usize, 0usize, 0usize);
    for c in &all {
        let recipe = c.key()?;
        let mut count: BTreeMap<(bool, u32), usize> = BTreeMap::new();
        let mut err = None;
        gp_events::run_frames_with(&sess.rom, c.power_on, &c.inputs, &addrs, &at, |f, _| {
            let mut go = || -> Result<()> {
                for h in &f.watch {
                    watch_hits += 1;
                    let n = count.entry((false, u32::from(h.addr))).or_insert(0);
                    *n += 1;
                    if *n <= RERUN_CAP {
                        written += 1;
                        w.emit(
                            Some(&recipe),
                            Some(f.frame),
                            Some(2),
                            WatchBody::Watch {
                                addr: h.addr,
                                value: h.value,
                                writer: namer.name(h.offset, h.cpu),
                            },
                        )?;
                    }
                }
                for h in &f.hooks {
                    hook_hits += 1;
                    let n = count.entry((true, h.offset)).or_insert(0);
                    *n += 1;
                    if *n <= RERUN_CAP {
                        written += 1;
                        w.emit(
                            Some(&recipe),
                            Some(f.frame),
                            Some(2),
                            WatchBody::Hook {
                                at: namer.name(h.offset, h.cpu),
                                a: h.a,
                                x: h.x,
                                y: h.y,
                                s: h.s,
                                p: h.p,
                            },
                        )?;
                    }
                }
                Ok(())
            };
            if err.is_none()
                && let Err(e) = go()
            {
                err = Some(e);
            }
        })?;
        if let Some(e) = err {
            return Err(e);
        }
    }
    w.emit(
        None,
        None,
        None,
        WatchBody::RerunEnd {
            watch_hits,
            hook_hits,
            written,
        },
    )?;
    Ok(Outcome::ok(
        json!({"name": name, "recipes": all.len(), "watch_hits": watch_hits, "hook_hits": hook_hits, "written": written, "events": w.seq}),
        format!(
            "session {name}: {} recipes run again; {watch_hits} watched writes, {hook_hits} hook calls, {written} written as events",
            all.len()
        ),
    ))
}

/// `gp watch detail`: L3 for a window of a recipe the session ran.
fn detail(paths: &Paths, name: &str, hash: &str, frames: &str) -> Result<Outcome> {
    let dir = session_dir(&paths.home, name)?;
    let sess = load_session(&dir)?;
    let clip = recipe(&dir, hash)?;
    let (a, last) = crate::reach::frame_range(frames)?;
    let (v, text) = crate::reach::detail_of(&sess.rom, &clip, a, last)?;
    Ok(Outcome::ok(v, text))
}

/// What is under a box of the screen at a frame of a recipe: the sprites
/// there (from OAM as the frame ended) and, for each, the instruction that
/// last wrote its bytes in the sprite table the code copies to OAM by DMA
/// (the page the last write to $4014 named), with the routine it is in.
pub(crate) fn point(
    dir: &Path,
    hash: &str,
    frame: u64,
    rect: (u16, u16, u16, u16),
) -> Result<Value> {
    let sess = load_session(dir)?;
    let clip = recipe(dir, hash)?;
    let n = usize::try_from(frame)?.min(clip.inputs.len());
    let inputs = &clip.inputs[..n];
    let mut nes = gp_nes::Nes::power_on(&sess.rom, clip.power_on)?;
    for i in inputs {
        nes.step(*i)?;
    }
    let oam = nes.oam();
    let height: u16 = if nes.tall_sprites() { 16 } else { 8 };
    let (bx, by, bw, bh) = rect;
    let under: Vec<usize> = (0..64)
        .filter(|&i| {
            let (y, x) = (u16::from(oam[4 * i]) + 1, u16::from(oam[4 * i + 3]));
            y < 240 && x < bx + bw && bx < x + 8 && y < by + bh && by < y + height
        })
        .collect();
    // The page the code copies to OAM: the last value written to $4014.
    let mut page = None;
    let mut writers: BTreeMap<u16, (String, u64, u8)> = BTreeMap::new();
    let namer = Namer::of_board(sess.board.clone());
    gp_events::run_frames_with(
        &sess.rom,
        clip.power_on,
        inputs,
        &BTreeSet::from([0x4014]),
        &[],
        |f, _| {
            for h in &f.watch {
                page = Some(h.value);
            }
        },
    )?;
    if let Some(p) = page {
        let watch: BTreeSet<u16> = under
            .iter()
            .flat_map(|&i| (0..4).map(move |k| u16::from(p) << 8 | (4 * i + k) as u16))
            .collect();
        gp_events::run_frames_with(&sess.rom, clip.power_on, inputs, &watch, &[], |f, _| {
            for h in &f.watch {
                writers.insert(h.addr, (namer.name(h.offset, h.cpu), f.frame, h.value));
            }
        })?;
    }
    let routines: Vec<(String, u32, u32)> = serde_json::from_str::<Value>(
        &fs::read_to_string(dir.join("static.json")).unwrap_or_else(|_| "{}".into()),
    )
    .ok()
    .and_then(|v| v["routines"].as_array().cloned())
    .unwrap_or_default()
    .iter()
    .filter_map(|r| {
        let v = gp_nes::codemap::va_parse(r["addr"].as_str()?)?;
        Some((
            r["name"].as_str()?.to_owned(),
            v,
            r["size"].as_u64()? as u32,
        ))
    })
    .collect();
    let routine_of = |at: &str| -> Option<String> {
        let v = gp_nes::codemap::va_parse(at)?;
        routines
            .iter()
            .filter(|(_, a, n)| v >> 16 == a >> 16 && (*a..a + n).contains(&v))
            .min_by_key(|(_, _, n)| *n)
            .map(|(name, _, _)| name.clone())
    };
    let sprites: Vec<Value> = under
        .iter()
        .map(|&i| {
            let bytes: Vec<Value> = (0..4u16)
                .filter_map(|k| {
                    let addr = (u16::from(page?) << 8) | (4 * i as u16 + k);
                    let (writer, at, value) = writers.get(&addr)?.clone();
                    Some(json!({"addr": addr, "value": value, "writer": writer,
                                "routine": routine_of(&writer), "frame": at}))
                })
                .collect();
            json!({"index": i, "y": oam[4 * i], "tile": oam[4 * i + 1], "attr": oam[4 * i + 2],
                   "x": oam[4 * i + 3], "bytes": bytes})
        })
        .collect();
    // In words: each sprite and the routine that wrote its position.
    let words: Vec<String> = sprites
        .iter()
        .map(|s| {
            let b = &s["bytes"];
            let by = b
                .as_array()
                .and_then(|v| v.first())
                .map(|w| {
                    format!(
                        "written by {} ({}) at frame {}",
                        w["routine"]
                            .as_str()
                            .unwrap_or("a routine the reader hasn't named"),
                        w["writer"].as_str().unwrap_or(""),
                        w["frame"]
                    )
                })
                .unwrap_or_else(|| "with no write seen".into());
            format!("sprite {}, {by}", s["index"])
        })
        .collect();
    Ok(json!({
        "recipe": hash, "frame": n, "rect": [bx, by, bw, bh],
        "oam_page": page, "sprites": sprites, "words": words,
    }))
}

/// Appends an input to a session's `inputs.ndjson` in one write; returns
/// how many the session has.
pub(crate) fn append_input(dir: &Path, input: &gp_events::watch::WatchInput) -> Result<usize> {
    input.check().map_err(anyhow::Error::msg)?;
    if !dir.join("session.json").is_file() {
        bail!("no watch session at {}", dir.display());
    }
    let mut line = serde_json::to_string(input)?;
    line.push('\n');
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("inputs.ndjson"))?
        .write_all(line.as_bytes())?;
    Ok(read_inputs(dir)?.len())
}

/// A session's inputs, each whole line that parses.
pub(crate) fn read_inputs(dir: &Path) -> Result<Vec<gp_events::watch::WatchInput>> {
    let text = fs::read_to_string(dir.join("inputs.ndjson")).unwrap_or_default();
    Ok(text
        .split_inclusive('\n')
        .filter(|l| l.ends_with('\n'))
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect())
}
