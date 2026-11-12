//! `gp element`: one game element's code, read from the original's run
//! rather than from the disassembly by hand (.claude/skills/element-extraction).
//!
//! `gp element inputs` is steps 2 and 3 of that skill for one clip: from
//! the clip's first wrong frame and measure (or a byte you name), it runs
//! the original to that frame, finds the instruction that last wrote the
//! byte and its routine by the disassembly's labels, and slices the frame
//! back from that write ([`gp_nes::slice`]): the bytes the routine read,
//! through indexes and pointers, the branches it took, and, one level on,
//! the routines that set what it read earlier in the frame. Each read gets
//! a role. The roles come from the labels' names and the trace, until the
//! RAM-role classifier (`gp_nes::ramrole`) is on main to give them.
//!
//! The output is the skill's step 3 table and its sentence ("MovePodoboo
//! wrote Enemy_Y_Speed+x from PseudoRandomBitReg+1+x"), and JSON for the
//! element dossier and `gp agent brief`. The same clip and frame give the
//! same output: the original is deterministic and the slice reads only its
//! trace. Labels and code text are never copied: only routine and label
//! names, addresses and values are printed.
//!
//! Changed from game-printer by gameprinter's layout: the old command read
//! a capture run's kept clip, its background score and its bench (helpers
//! of the old `crate::serve`). Here `--run` is the game's package folder
//! and `--clip` a clip of its corpus, as wave 3a rehoused the capture
//! commands. The clip's first wrong measures and the original's slot of
//! each body come from its lockstep straight through, run here as `gp clip
//! score` runs it (with the original's draws when the mapping scores them),
//! and only when the command needs them: `--frame` with `--byte`, or with
//! `--measure` and `--slot`, runs no model. The labels are the game's
//! `labels.toml` (or `--labels`); the file is read here until wave 1's `gp
//! labels` lands with its own reader. The original starts through gp-nes,
//! and an arcade romset is refused in words.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use anyhow::{Context as _, Result, bail};
use clap::{Args, Subcommand};
use gp_evidence::Paths;
use gp_interp::mapping::{Field, Mapping};
use gp_nes::slice::{self, FrameTrace, From, Gap, How, Loc, Recorder, Step};
use serde_json::{Value, json};

use crate::Outcome;
use crate::clip_score::{Ctx, Open};
use labelfile::LabelsFile;

/// The JSON shape of `gp element inputs`. A change to it bumps this.
pub const INPUTS_SCHEMA: &str = "gp element inputs 1";

#[derive(Subcommand)]
pub enum ElementCmd {
    /// The routine that wrote a measure's byte on a clip's first wrong
    /// frame, and every byte and branch condition its value depends on,
    /// each with its label and role: the element-extraction skill's steps
    /// 2 and 3, from the original's trace. Read-only.
    Inputs(InputsArgs),
}

#[derive(Args, Clone, Default)]
pub struct InputsArgs {
    /// The game's package folder (games/donkey-kong). In game-printer this
    /// was a capture run.
    #[arg(long)]
    pub run: PathBuf,
    /// A clip of the game's corpus by its ID (dk.hammer.018).
    #[arg(long)]
    pub clip: String,
    /// The clip's frame, counted from its start as its score counts it
    /// (default: the measure's first wrong frame).
    #[arg(long)]
    pub frame: Option<u64>,
    /// A RAM byte to slice instead of the measure's: a label
    /// (`Enemy_Y_Speed`, `Enemy_Y_Speed+2`) or an address (`$00A2`,
    /// `0xa2`).
    #[arg(long)]
    pub byte: Option<String>,
    /// The measure, as the score names it (`bodies.podoboo[1].vy`,
    /// `entities.mario[0].facing`). Default: the clip's first wrong one.
    #[arg(long)]
    pub measure: Option<String>,
    /// An element by its model entity (`goomba`): its first wrong measure
    /// in the clip, and the routines that wrote its slot's bytes on the
    /// frame.
    #[arg(long)]
    pub entity: Option<String>,
    /// The original's slot for a body measure, instead of the one the
    /// lockstep paired with it.
    #[arg(long)]
    pub slot: Option<u16>,
    /// How many levels of earlier writes in the frame to follow from the
    /// routine's inputs.
    #[arg(long, default_value_t = 1)]
    pub depth: u32,
    /// Don't go back across frames: by default each byte the slice tested
    /// or read that an earlier frame left is sliced on the frame that last
    /// wrote it (one step back).
    #[arg(long = "no-back", action = clap::ArgAction::SetFalse, default_value_t = true)]
    pub back: bool,
    /// The labels file, instead of the game's usual one.
    #[arg(long)]
    pub labels: Option<PathBuf>,
    /// The game's built module, for the lockstep of a model with hooks.
    #[arg(long)]
    pub module: Option<PathBuf>,
    /// The model folder (model.toml and mapping.toml); by default the game's
    /// own.
    #[arg(long)]
    pub model: Option<PathBuf>,
    /// The model's lifted assets (default: $GP_HOME/work/assets/SOURCE).
    #[arg(long)]
    pub assets: Option<PathBuf>,
    /// The ROM; by default the one in $GP_ROM_DIR the package names.
    #[arg(long)]
    pub rom: Option<PathBuf>,
}

pub(crate) fn run(cmd: &ElementCmd, paths: &Paths) -> Result<Outcome> {
    match cmd {
        ElementCmd::Inputs(a) => {
            let v = inputs(paths, a)?;
            let text = text_of(&v);
            Ok(Outcome::ok(v, text))
        }
    }
}

/// A byte to slice.
struct Target {
    addr: u16,
    /// Where it came from: the measure's ram byte, its fraction, or
    /// `--byte`.
    part: &'static str,
}

/// `gp element inputs` as JSON ([`INPUTS_SCHEMA`]).
pub fn inputs(paths: &Paths, a: &InputsArgs) -> Result<Value> {
    let game = &a.run;
    let clip = crate::clip_score::corpus_clip(game, &a.clip)
        .with_context(|| format!("no clip {} in {}'s corpus", a.clip, game.display()))?;
    let (log, start) = clip.log()?;
    let rom = crate::clip_score::rom_of(paths, game, a.rom.as_deref())?;
    crate::original::nes_only(
        crate::original::Platform::of_file(&rom),
        "an element's inputs (`gp element inputs`)",
    )?;
    let model_dir = a.model.clone().unwrap_or_else(|| game.clone());
    let mfile = model_dir.join(gp_interp::mapping::MAPPING_FILE);
    let mapping = Mapping::parse(
        &std::fs::read_to_string(&mfile).with_context(|| format!("reading {}", mfile.display()))?,
    )
    .map_err(|e| anyhow::anyhow!("{}: {e}", mfile.display()))?;
    let start_frame = start;
    let start = start as u64;
    // The clip's lockstep straight through, run once when needed.
    let lockstep = std::cell::OnceCell::new();
    let ls = || -> Result<&gp_interp::lockstep::Lockstep> {
        if let Some(l) = lockstep.get() {
            return Ok(l);
        }
        let ctx = Ctx::open(
            paths,
            game,
            &Open {
                module: a.module.as_deref(),
                model: a.model.as_deref(),
                assets: a.assets.as_deref(),
                rom: a.rom.as_deref(),
            },
        )?;
        let (l, _, _) = crate::clip_score::straight(&ctx, &log, start_frame, None, false)?;
        Ok(lockstep.get_or_init(|| l))
    };
    let wants_score =
        a.frame.is_none() || (a.byte.is_none() && a.measure.is_none()) || a.entity.is_some();
    let tracks: Vec<gp_interp::scenario::TrackFirst> = if wants_score {
        gp_interp::scenario::tracks(ls()?, start)
    } else {
        Vec::new()
    };
    let scored = wants_score;
    let clip_id = clip.recipe.id.clone();
    let clip_name = clip.recipe.name.clone();

    // The measure and frame.
    let track = match (&a.entity, &a.measure) {
        (_, Some(m)) => tracks.iter().find(|t| &t.path == m),
        (Some(e), None) => tracks.iter().find(|t| &t.track == e),
        (None, None) => tracks.first(),
    };
    if let (Some(e), None, None) = (&a.entity, &a.measure, track) {
        bail!(
            "the {e} isn't wrong in clip {}; the wrong tracks are {}. Give --measure and --frame for one that is right",
            clip_id,
            list_or_none(tracks.iter().map(|t| t.track.as_str()))
        );
    }
    let measure = a.measure.clone().or_else(|| track.map(|t| t.path.clone()));
    let frame = match (a.frame, track) {
        (Some(f), _) => f,
        (None, Some(t)) => t.original_frame.saturating_sub(start),
        (None, None) => bail!(
            "clip {} has no wrong measure{}; give --frame",
            clip_id,
            if scored { "" } else { " scored yet" }
        ),
    };
    anyhow::ensure!(
        frame >= 1,
        "frame {frame} is the clip's start, which no frame of the clip writes; give --frame 1 or later"
    );
    let of = start + frame;

    // The labels.
    let (names, labels_file, labels_missing) = names_for(game, a.labels.as_deref());

    // The bytes.
    let mut slot = a.slot;
    let mut targets: Vec<Target> = Vec::new();
    let mut fields_of_entity: Vec<u16> = Vec::new();
    if let Some(byte) = &a.byte {
        let addr = names
            .resolve(byte)
            .with_context(|| format!("no label or address {byte}"))?;
        targets.push(Target { addr, part: "byte" });
    } else {
        let m = measure
            .as_deref()
            .context("no measure to slice; give --measure or --byte")?;
        if m.starts_with(&format!("{}.", gp_interp::bodies::PREFIX)) {
            let (prefix, field) = m
                .rsplit_once('.')
                .context("a body measure is bodies.<kind>[n].<field>")?;
            let s = match slot {
                Some(s) => s,
                None => {
                    let slots = body_slots(ls()?);
                    *slots.get(prefix).with_context(|| {
                        format!(
                            "the lockstep pairs no slot of the original with {prefix}; give --slot"
                        )
                    })?
                }
            };
            slot = Some(s);
            let kind = prefix
                .trim_start_matches("bodies.")
                .split('[')
                .next()
                .unwrap_or("");
            for (addr, part) in body_bytes(&mapping, kind, field, s)
                .with_context(|| format!("the mapping's slots have no field {field} for {kind}"))?
            {
                targets.push(Target { addr, part });
            }
            fields_of_entity = entity_bytes(&mapping, kind, s);
        } else {
            let f = mapping
                .fields
                .iter()
                .find(|f| f.path == m)
                .with_context(|| format!("the mapping has no field {m}"))?;
            for (addr, part) in field_bytes(f, 0) {
                targets.push(Target { addr, part });
            }
        }
    }
    anyhow::ensure!(!targets.is_empty(), "the measure reads no RAM byte");

    // Run the original to the frame, keeping each target's last write.
    let inputs = &log.frames[start_frame.min(log.frames.len())..];
    let original = crate::code::start_state(
        &paths.home,
        &rom,
        gp_evidence::manifest::PowerOn::default(),
        &log.frames,
        start_frame,
    )?;
    anyhow::ensure!(
        frame as usize <= inputs.len(),
        "frame {frame} is past the clip's {} frames",
        inputs.len()
    );
    let mut last: Vec<Option<(u64, Rc<FrameTrace>)>> = vec![None; targets.len()];
    let mut entity_writers: BTreeSet<String> = BTreeSet::new();
    let ram = run_frames(original.clone(), inputs, start, frame, |f, t| {
        for (n, tg) in targets.iter().enumerate() {
            if t.last_write(tg.addr).is_some() {
                last[n] = Some((f, t.clone()));
            }
        }
        if f == of {
            for &addr in &fields_of_entity {
                for j in t.writes_of(addr) {
                    entity_writers.insert(names.routine(t.steps[j].pc));
                }
            }
        }
    })?;

    // Slice each byte's last write.
    let slices: Vec<Option<(u64, Rc<FrameTrace>, slice::Followed)>> = targets
        .iter()
        .zip(&last)
        .map(|(tg, l)| {
            l.as_ref().map(|(wf, t)| {
                let w = t.last_write(tg.addr).expect("written");
                let f = slice::follow(t, w, Loc::Mem(tg.addr), slice::Options::default(), a.depth);
                (*wf, t.clone(), f)
            })
        })
        .collect();

    // One step back across frames: each byte a slice tested or read whose
    // value an earlier frame left, sliced on the frame that last wrote it.
    let own: BTreeSet<u16> = targets.iter().map(|t| t.addr).collect();
    let mut wanted: BTreeMap<u16, u64> = BTreeMap::new();
    for (wf, _, f) in slices.iter().flatten() {
        for fol in std::iter::once(f).chain(f.feeds.iter().map(|(_, s)| s)) {
            for r in &fol.slice.reads {
                if r.from == From::Before
                    && matches!(r.how, How::Value | How::Branch)
                    && slice::space(r.addr) == slice::Space::Ram
                    && !(0x100..0x200).contains(&r.addr)
                    && !own.contains(&r.addr)
                {
                    let e = wanted.entry(r.addr).or_insert(*wf);
                    *e = (*e).min(*wf);
                }
            }
        }
    }
    // A write that only steps the byte from itself (a timer counting
    // down) is passed over for the one that set it, and counted.
    let mut earlier: BTreeMap<u16, (u64, Rc<FrameTrace>)> = BTreeMap::new();
    let mut stepped: BTreeMap<u16, (u64, String)> = BTreeMap::new();
    if a.back && !wanted.is_empty() {
        let upto = wanted.values().max().copied().unwrap_or(start + 1) - start - 1;
        run_frames(original, inputs, start, upto, |f, t| {
            for (&addr, &read_on) in &wanted {
                if f >= read_on {
                    continue;
                }
                let Some(w) = t.last_write(addr) else {
                    continue;
                };
                let sl = slice::slice(t, w, Loc::Mem(addr), slice::Options { branches: false });
                let only_self = sl
                    .reads
                    .iter()
                    .filter(|r| r.how == How::Value)
                    .all(|r| r.addr == addr);
                if only_self && sl.reads.iter().any(|r| r.addr == addr) {
                    let e = stepped
                        .entry(addr)
                        .or_insert((0, names.routine(t.steps[w].pc)));
                    e.0 += 1;
                } else {
                    earlier.insert(addr, (f, t.clone()));
                    stepped.remove(&addr);
                }
            }
        })?;
    }
    let earlier_json = |addr: u16| -> Value {
        let mut v = match earlier.get(&addr) {
            Some((f, t)) => {
                let w = t.last_write(addr).expect("written");
                let fol = slice::follow(t, w, Loc::Mem(addr), slice::Options::default(), 0);
                let mut v = slice_json(&names, t, &fol);
                v["frame_written"] = json!(f - start);
                v
            }
            None => json!({"with_start": true}),
        };
        v["through"] = json!(names.ram(addr));
        if let Some((n, by)) = stepped.get(&addr) {
            v["stepped"] = json!({"frames": n, "by": by});
        }
        v
    };

    let mut bytes = Vec::new();
    for (n, tg) in targets.iter().enumerate() {
        let value = ram.get(usize::from(tg.addr)).copied().unwrap_or(0);
        let mut v = match &last[n] {
            None => json!({
                "name": names.ram(tg.addr), "byte": names.ram(tg.addr), "addr": hex(tg.addr), "part": tg.part,
                "value": value, "written": false,
                "note": format!("not written from the clip's start (frame {start}) to frame {of}: its value came with the start, which no trace of this clip covers"),
            }),
            Some((wf, t)) => {
                let (_, _, f) = slices[n].as_ref().expect("sliced");
                let mut v = slice_json(&names, t, f);
                if a.back {
                    let mut back: Vec<u16> = Vec::new();
                    for fol in std::iter::once(f).chain(f.feeds.iter().map(|(_, s)| s)) {
                        for r in &fol.slice.reads {
                            if wanted.contains_key(&r.addr) && !back.contains(&r.addr) {
                                back.push(r.addr);
                            }
                        }
                    }
                    back.sort_unstable();
                    v["earlier"] = Value::Array(back.into_iter().map(earlier_json).collect());
                }
                v["addr"] = json!(hex(tg.addr));
                v["part"] = json!(tg.part);
                v["value"] = json!(value);
                v["written"] = json!(true);
                v["frame_written"] = json!(wf - start);
                v["original_frame_written"] = json!(wf);
                if *wf != of {
                    v["note"] = json!(format!(
                        "not written on frame {frame}; last written on frame {}",
                        wf - start
                    ));
                }
                v
            }
        };
        v["sentence"] = json!(sentence(&v));
        bytes.push(v);
    }
    let sentence = bytes
        .iter()
        .filter(|b| b["written"] == true)
        .map(|b| b["sentence"].as_str().unwrap_or("").to_owned())
        .collect::<Vec<_>>();
    let gaps: Vec<Value> = bytes
        .iter()
        .flat_map(|b| b["gaps"].as_array().cloned().unwrap_or_default())
        .collect();
    Ok(json!({
        "schema": INPUTS_SCHEMA,
        "run": a.run,
        "clip": clip_id,
        "clip_name": clip_name,
        "frame": frame,
        "original_frame": of,
        "start": start,
        "measure": if a.byte.is_some() { Value::Null } else { json!(measure) },
        "entity": a.entity,
        "slot": slot,
        "labels": labels_file,
        "labels_missing": labels_missing,
        "roles_from": "labels",
        "bytes": bytes,
        "entity_routines": entity_writers,
        "sentence": sentence.join(" "),
        "gaps": gaps,
    }))
}

/// Runs the original from the clip's start (frame `start`) through
/// `frames` of the clip's inputs, recording each frame and giving it to
/// `each` with the original's frame number. Returns the RAM after.
fn run_frames(
    original: gp_nes::Nes,
    inputs: &[gp_evidence::input_log::InputFrame],
    start: u64,
    frames: u64,
    mut each: impl FnMut(u64, &Rc<FrameTrace>),
) -> Result<Vec<u8>> {
    let mut nes = original;
    nes.set_events(false);
    for (k, i) in inputs.iter().take(frames as usize).enumerate() {
        let mut rec = Recorder::new();
        rec.start(nes.bus_mut_for_observers());
        let f = start + k as u64 + 1;
        nes.step_with(
            gp_evidence::input_log::InputFrame {
                polled: false,
                ..*i
            },
            |bus| rec.before(bus),
        )
        .map_err(|e| anyhow::anyhow!("the original, frame {f}: {e}"))?;
        each(f, &Rc::new(rec.finish()));
    }
    Ok(nes.wram().to_vec())
}

fn list_or_none<'a>(it: impl Iterator<Item = &'a str>) -> String {
    let v: Vec<&str> = it.collect();
    if v.is_empty() {
        "none".into()
    } else {
        v.join(", ")
    }
}

fn hex(a: u16) -> String {
    format!("${a:04X}")
}

/// A slot kind's field by its path, and the slot within its group: the
/// lockstep numbers slots on from one group to the next.
fn slot_field<'a>(m: &'a Mapping, kind: &str, field: &str, slot: u16) -> Option<(&'a Field, u16)> {
    let mut local = slot;
    for g in m.slot_groups() {
        if local >= g.count {
            local -= g.count;
            continue;
        }
        let k = g.kinds.iter().find(|k| k.entity == kind)?;
        return g
            .fields_of(k)
            .into_iter()
            .find(|f| f.path == field)
            .map(|f| (f, local));
    }
    None
}

/// The bytes a body measure reads in slot `slot`. The measures the
/// lockstep derives read these: `alive` the slot's flag, `moving` the
/// speed across (or the place), `moving_y` the speed up and down (or the
/// height), `phase` the place and height.
fn body_bytes(m: &Mapping, kind: &str, field: &str, slot: u16) -> Option<Vec<(u16, &'static str)>> {
    let of = |f: &str| slot_field(m, kind, f, slot).map(|(f, local)| field_bytes(f, local));
    match field {
        gp_interp::bodies::ALIVE => {
            let mut local = slot;
            for g in m.slot_groups() {
                if local < g.count {
                    return Some(vec![(g.active_at(local), "flag")]);
                }
                local -= g.count;
            }
            None
        }
        gp_interp::bodies::MOVING => of("vx").or_else(|| of("x")),
        gp_interp::bodies::MOVING_Y => of("vy").or_else(|| of("y")),
        gp_interp::bodies::PHASE => {
            let mut v = of("x").unwrap_or_default();
            v.extend(of("y").unwrap_or_default());
            (!v.is_empty()).then_some(v)
        }
        other => of(other),
    }
}

/// Every RAM byte of a slot kind's fields in one slot.
fn entity_bytes(m: &Mapping, kind: &str, slot: u16) -> Vec<u16> {
    let mut local = slot;
    for g in m.slot_groups() {
        if local >= g.count {
            local -= g.count;
            continue;
        }
        let Some(k) = g.kinds.iter().find(|k| k.entity == kind) else {
            return Vec::new();
        };
        let mut out: Vec<u16> = g
            .fields_of(k)
            .into_iter()
            .flat_map(|f| field_bytes(f, local).into_iter().map(|(a, _)| a))
            .collect();
        out.sort_unstable();
        out.dedup();
        return out;
    }
    Vec::new()
}

/// A field's bytes in slot `slot`: its whole bytes, then its fraction.
fn field_bytes(f: &Field, slot: u16) -> Vec<(u16, &'static str)> {
    let at = f.slot_addresses(slot);
    let names = f.ram.iter().map(|_| "ram").chain(f.frac.map(|_| "frac"));
    at.into_iter().zip(names).collect()
}

/// The names the labels file gives: routines by the address they cover,
/// RAM bytes by the label at or just below.
pub(crate) struct Names {
    routines: Vec<(u16, u32, String)>,
    ram: Vec<(u16, u16, String)>,
    data: Vec<(u16, u32, String)>,
}

/// How far below a byte a RAM label may sit and still name it
/// (`PseudoRandomBitReg+1`).
const NEAR: u16 = 16;

impl Names {
    pub(crate) fn from_file(f: &LabelsFile) -> Self {
        let mut routines: Vec<(u16, u32, String)> = f
            .routine
            .iter()
            .map(|r| (r.addr, r.size, r.name.clone()))
            .collect();
        routines.sort();
        // In address order, and at one address in the file's order: the
        // disassembly names the main use first (SprObject_Y_Speed before
        // Player_Y_Speed).
        let mut ram: Vec<(u16, u16, String)> = f
            .label
            .iter()
            .map(|l| (l.addr, l.size.max(1), l.name.clone()))
            .collect();
        ram.sort_by_key(|l| l.0);
        let mut data: Vec<(u16, u32, String)> = f
            .data
            .iter()
            .map(|r| (r.addr, r.size, r.name.clone()))
            .collect();
        data.sort();
        Self {
            routines,
            ram,
            data,
        }
    }

    fn empty() -> Self {
        Self {
            routines: Vec::new(),
            ram: Vec::new(),
            data: Vec::new(),
        }
    }

    /// The routine covering `pc`: the last routine label at or before it
    /// whose size reaches it. On a board with banks, the first bank's.
    pub(crate) fn routine(&self, pc: u16) -> String {
        self.routines
            .iter()
            .rev()
            .find(|(a, size, _)| *a <= pc && u32::from(pc) < u32::from(*a) + (*size).max(1))
            .map_or_else(|| hex(pc), |(_, _, n)| n.clone())
    }

    /// A RAM byte's name: a label at it, one whose size covers it, or one
    /// at most [`NEAR`] bytes below it, with the offset.
    pub(crate) fn ram(&self, addr: u16) -> String {
        self.ram_like(addr, None)
    }

    /// [`Names::ram`], choosing among the labels at one address the one
    /// that ends like `like` (`Enemy_X_Speed` for a byte the code reaches
    /// as `SprObject_X_Speed+x`), else the first listed.
    pub(crate) fn ram_like(&self, addr: u16, like: Option<&str>) -> String {
        if slice::space(addr) == slice::Space::Rom {
            return self
                .data
                .iter()
                .rev()
                .find(|(a, size, _)| *a <= addr && u32::from(addr) < u32::from(*a) + (*size).max(1))
                .map_or_else(|| hex(addr), |(a, _, n)| with_offset(n, addr - a));
        }
        // The highest label at or below it that covers it by its size, else
        // the highest within [`NEAR`]; at one address, the one sharing the
        // longest ending with `like`, then the first listed.
        let tail = |n: &str| -> usize {
            like.map_or(0, |w| {
                n.bytes()
                    .rev()
                    .zip(w.bytes().rev())
                    .take_while(|(a, b)| a == b)
                    .count()
            })
        };
        let best = |ok: &dyn Fn(u16, u16) -> bool| {
            let mut best: Option<&(u16, u16, String)> = None;
            for l in &self.ram {
                if l.0 > addr || !ok(addr - l.0, l.1) {
                    continue;
                }
                let better = match best {
                    None => true,
                    Some(b) => {
                        l.0 > b.0
                            || (l.0 == b.0
                                && (tail(&l.2), self.continues(l))
                                    > (tail(&b.2), self.continues(b)))
                    }
                };
                if better {
                    best = Some(l);
                }
            }
            best
        };
        match best(&|off, size| off < size).or_else(|| best(&|off, _| off < NEAR)) {
            Some((a, _, n)) => with_offset(n, addr - a),
            None => io_name(addr).map_or_else(|| hex(addr), str::to_owned),
        }
    }

    /// Whether a label continues an array the label just below starts:
    /// `Enemy_X_Speed` at $58 after `SprObject_X_Speed` at $57 share the
    /// ending after their first `_`. Of several names at one address, such
    /// a name is the table's, the others reuses of the byte.
    fn continues(&self, l: &(u16, u16, String)) -> bool {
        let Some((_, end)) = l.2.split_once('_') else {
            return false;
        };
        self.ram
            .iter()
            .any(|o| o.0 + 1 == l.0 && o.2.split_once('_').is_some_and(|(_, e)| e == end))
    }

    /// A label (`Name`, `Name+2`) or an address (`$00A2`, `0xa2`, `162`).
    pub(crate) fn resolve(&self, q: &str) -> Option<u16> {
        let q = q.trim();
        if let Some(h) = q.strip_prefix('$').or_else(|| q.strip_prefix("0x")) {
            return u16::from_str_radix(h, 16).ok();
        }
        if let Ok(n) = q.parse::<u16>() {
            return Some(n);
        }
        let (name, off) = match q.rsplit_once('+') {
            Some((n, o)) if o.parse::<u16>().is_ok() => (n, o.parse::<u16>().ok()?),
            _ => (q, 0),
        };
        self.ram
            .iter()
            .find(|(_, _, n)| n == name)
            .map(|(a, _, _)| a + off)
    }

    /// How an instruction's operand names the byte it reached: the base
    /// label with the index (`Enemy_Y_Speed+x`), the pointer
    /// (`(EnemyData)+y`), or the byte's own name.
    fn operand(&self, s: &Step) -> String {
        let reg = match s.index() {
            Some(Loc::X) => "x",
            Some(Loc::Y) => "y",
            _ => "",
        };
        match (s.base(), s.indirect()) {
            (Some(p), true) if reg == "y" => format!("({})+y", self.ram(p)),
            (Some(p), true) => format!("({}+x)", self.ram(p)),
            (Some(b), false) if !reg.is_empty() => format!("{}+{reg}", self.ram(b)),
            _ => s.addr.map_or_else(String::new, |a| self.ram(a)),
        }
    }

    /// The byte a step's operand reached, named like the code's base label.
    fn byte_of(&self, s: &Step, addr: u16) -> String {
        let like = s
            .base()
            .filter(|_| s.addr == Some(addr))
            .map(|b| self.ram(b));
        let like = like.as_deref().map(|n| n.split('+').next().unwrap_or(n));
        self.ram_like(addr, like)
    }

    /// A read's name: the operand's as the code names it when the read is
    /// the operand's byte, else the byte's own (a pointer byte, the stack).
    fn read(&self, s: &Step, addr: u16) -> String {
        if s.addr == Some(addr) {
            return self.operand(s);
        }
        if (0x100..0x200).contains(&addr) && s.ptr.is_none_or(|p| p != addr) {
            return "the stack".into();
        }
        self.ram(addr)
    }
}

fn with_offset(n: &str, off: u16) -> String {
    if off == 0 {
        n.to_owned()
    } else {
        format!("{n}+{off}")
    }
}

/// The NES registers' names.
fn io_name(addr: u16) -> Option<&'static str> {
    Some(match addr {
        0x2000 => "PPU_CTRL",
        0x2001 => "PPU_MASK",
        0x2002 => "PPU_STATUS",
        0x2004 => "OAM_DATA",
        0x2007 => "PPU_DATA",
        0x4015 => "APU_STATUS",
        0x4016 => "JOYPAD1",
        0x4017 => "JOYPAD2",
        _ => return None,
    })
}

/// A read's role, from the byte's own name and the trace, until the
/// RAM-role classifier gives them: what a model rule would read it as.
fn role(addr: u16, name: &str, how: How, from: From) -> &'static str {
    match slice::space(addr) {
        slice::Space::Io if addr == 0x4016 || addr == 0x4017 => return "controller",
        slice::Space::Io => return "register",
        slice::Space::Rom => return "table",
        slice::Space::Ram => {}
    }
    if (0x100..0x200).contains(&addr) && name == "the stack" {
        return "stack";
    }
    let up = name.to_ascii_uppercase();
    let has = |w: &[&str]| w.iter().any(|x| up.contains(x));
    if has(&["RANDOM", "RNG", "LFSR"]) {
        "random"
    } else if has(&["FRAMECOUNTER", "FRAME_COUNTER", "FRAMES"]) {
        "frame counter"
    } else if has(&["JOYPAD", "BUTTON", "PAD"]) {
        "controller"
    } else if has(&["BLOCK_BUFFER", "METATILE", "TILE", "LEVEL", "AREADATA"]) {
        "tile data"
    } else if how == How::Pointer {
        "pointer"
    } else if has(&["TIMER"]) {
        "timer"
    } else if has(&["PLAYER"]) {
        "player state"
    } else if name.starts_with('$') && matches!(from, From::Inside(_) | From::Outside(_)) {
        "scratch"
    } else {
        "state"
    }
}

fn where_from(names: &Names, t: &FrameTrace, from: From) -> String {
    match from {
        From::Before => "the frame before".into(),
        From::Inside(j) => format!("set in {}", names.routine(t.steps[j].pc)),
        From::Outside(j) => format!("set earlier by {}", names.routine(t.steps[j].pc)),
        From::Rom => "ROM".into(),
        From::Io => "hardware".into(),
    }
}

/// One slice and what it follows, as JSON.
fn slice_json(names: &Names, t: &FrameTrace, f: &slice::Followed) -> Value {
    let s = &f.slice;
    let w = &t.steps[s.writer];
    let call = &t.calls[w.call as usize];
    let all: Vec<Value> = s
        .reads
        .iter()
        .rev()
        .filter(|r| !matches!(r.from, From::Inside(_)))
        .map(|r| {
            let st = &t.steps[r.step];
            let name = names.read(st, r.addr);
            let byte = if name == "the stack" {
                name.clone()
            } else {
                names.byte_of(st, r.addr)
            };
            json!({
                "name": name, "byte": byte, "addr": hex(r.addr), "value": r.value,
                "role": role(r.addr, &byte, r.how, r.from),
                "how": r.how.name(),
                "by": names.routine(st.pc),
                "from": where_from(names, t, r.from),
            })
        })
        .collect();
    // The same byte read the same way by one routine from one place (a
    // loop over a register, a table read twice) is one row, counted.
    let mut reads: Vec<Value> = Vec::new();
    for r in all {
        let key = |v: &Value| {
            (
                v["by"].clone(),
                v["name"].clone(),
                v["addr"].clone(),
                v["how"].clone(),
                v["from"].clone(),
            )
        };
        match reads.iter_mut().find(|x| key(x) == key(&r)) {
            Some(x) => x["times"] = json!(x["times"].as_u64().unwrap_or(1) + 1),
            None => reads.push(r),
        }
    }
    let regs: Vec<Value> = s
        .regs_in
        .iter()
        .map(|r| {
            json!({
                "register": r.loc.name(), "how": r.how.name(),
                "from": r.from.map_or_else(|| "as the frame before left it".to_owned(), |j| format!("set earlier by {}", names.routine(t.steps[j].pc))),
            })
        })
        .collect();
    let branches: Vec<Value> = s
        .branches
        .iter()
        .rev()
        .map(|b| {
            let st = &t.steps[b.step];
            json!({"routine": names.routine(st.pc), "pc": hex(st.pc), "taken": b.taken})
        })
        .collect();
    let gaps: Vec<Value> = s
        .gaps
        .iter()
        .map(|g| match *g {
            Gap::CodeInRam { step } => json!({
                "kind": "code in RAM", "pc": hex(t.steps[step].pc),
                "says": format!("code at {} ran from RAM, where it may have been written: the slice takes its bytes as they were", hex(t.steps[step].pc)),
            }),
            Gap::Unmodelled { step } => json!({
                "kind": "unmodelled", "pc": hex(t.steps[step].pc),
                "says": format!("the instruction at {} (opcode {:02X}) isn't modelled: what it defines is left out", hex(t.steps[step].pc), t.steps[step].op),
            }),
            Gap::Register { step, addr } => json!({
                "kind": "register", "pc": hex(t.steps[step].pc),
                "says": format!("{} reads {} ({}): the slice stops at the hardware", names.routine(t.steps[step].pc), io_name(addr).unwrap_or("a register"), hex(addr)),
            }),
        })
        .collect();
    // A store whose value no byte or register feeds: the code's own.
    let constant = w.stored().is_some()
        && !s.reads.iter().any(|r| r.how == How::Value)
        && !s.regs_in.iter().any(|r| r.how == How::Value);
    let feeds: Vec<Value> = f
        .feeds
        .iter()
        .map(|(l, sub)| {
            let mut v = slice_json(names, t, sub);
            v["through"] = json!(match l {
                Loc::Mem(_) => v["byte"].as_str().unwrap_or("?").to_owned(),
                other => other.name(),
            });
            v
        })
        .collect();
    json!({
        "name": names.operand(w),
        "byte": w.addr.map(|a| names.byte_of(w, a)),
        "writer": {
            "routine": names.routine(w.pc), "pc": hex(w.pc),
            "call": if call.entry == 0 { Value::Null } else { json!(names.routine(call.entry)) },
            "in_interrupt": call.interrupt,
        },
        "constant": if constant { json!(format!("${:02X}", w.stored().unwrap_or(0))) } else { Value::Null },
        "steps": s.steps.len(),
        "reads": reads,
        "registers": regs,
        "branches": branches,
        "feeds": feeds,
        "gaps": gaps,
    })
}

/// The step 3 sentence for one byte: "MovePodoboo wrote Enemy_Y_Speed+x
/// from PseudoRandomBitReg+1+x; it tested EnemyIntervalTimer+x".
fn sentence(b: &Value) -> String {
    if b["written"] != true {
        return format!(
            "{} was not written in the clip before frame {}",
            b["name"].as_str().unwrap_or("?"),
            b["frame"].as_u64().map_or(String::new(), |f| f.to_string())
        );
    }
    let mut out = one(b);
    let feeds: Vec<String> = b["feeds"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|f| {
            let mut t = format!(
                "{} came from {}",
                f["through"].as_str().unwrap_or("?"),
                f["writer"]["routine"].as_str().unwrap_or("?")
            );
            t.push_str(&parts(f, ", "));
            t
        })
        .collect();
    if !feeds.is_empty() {
        out.push_str("; ");
        out.push_str(&feeds.join("; "));
    }
    out.push('.');
    let back: Vec<String> = b["earlier"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|e| {
            let through = e["through"].as_str().unwrap_or("?");
            let then = e["stepped"]["frames"].as_u64().map_or(String::new(), |n| {
                format!(
                    ", then stepped on {n} frame(s) by {}",
                    e["stepped"]["by"].as_str().unwrap_or("?")
                )
            });
            if e["with_start"] == true {
                return format!("{through} came with the clip's start{then}");
            }
            let values = names_of(e, "value");
            format!(
                "{through} was set on frame {} by {}{}{then}",
                e["frame_written"],
                e["writer"]["routine"].as_str().unwrap_or("?"),
                match (e["constant"].as_str(), values.is_empty()) {
                    (Some(c), _) => format!(" a constant ({c})"),
                    (None, false) => format!(" from {}", and_list(&values)),
                    (None, true) => String::new(),
                }
            )
        })
        .collect();
    if !back.is_empty() {
        out.push_str(&format!(" Before the frame: {}.", back.join("; ")));
    }
    out
}

/// "MovePodoboo wrote Enemy_Y_Speed+x from ..., it tested ...".
fn one(b: &Value) -> String {
    format!(
        "{} wrote {}{}",
        b["writer"]["routine"].as_str().unwrap_or("?"),
        b["name"].as_str().unwrap_or("?"),
        parts(b, "; ")
    )
}

/// A slice's value inputs (or its constant), the bytes that picked what
/// it read, and the branches' tests.
fn parts(b: &Value, sep: &str) -> String {
    let mut out = String::new();
    let values = names_of(b, "value");
    if let Some(c) = b["constant"].as_str() {
        out.push_str(&format!(" a constant ({c})"));
    } else if !values.is_empty() {
        out.push_str(&format!(" from {}", and_list(&values)));
    }
    let index = names_of(b, "index");
    if !index.is_empty() {
        out.push_str(&format!("{sep}indexed by {}", and_list(&index)));
    }
    let tests = names_of(b, "branch");
    if !tests.is_empty() {
        out.push_str(&format!("{sep}it tested {}", and_list(&tests)));
    }
    out
}

/// The names of a slice's reads that reach the write `how`, in program
/// order, once each, and the registers that came from the caller.
fn names_of(b: &Value, how: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for r in b["reads"].as_array().into_iter().flatten() {
        if r["how"] == how && r["role"] != "stack" {
            let n = r["name"].as_str().unwrap_or("?").to_owned();
            if !out.contains(&n) {
                out.push(n);
            }
        }
    }
    if how == "value" {
        for r in b["registers"].as_array().into_iter().flatten() {
            if r["how"] == "value" {
                out.push(format!(
                    "{} from the caller",
                    r["register"].as_str().unwrap_or("?")
                ));
            }
        }
    }
    out
}

/// The most names a sentence lists in one place; the table has them all.
const LIST: usize = 6;

fn and_list(v: &[String]) -> String {
    if v.len() > LIST {
        return format!("{} and {} more", v[..LIST].join(", "), v.len() - LIST);
    }
    match v {
        [] => String::new(),
        [a] => a.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// The original's slot of each body the lockstep pairs, by its measures'
/// prefix (`bodies.podoboo[1]`): which slot `bodies.<kind>[n]` names, since
/// n counts the run's bodies of that kind in both games.
fn body_slots(ls: &gp_interp::lockstep::Lockstep) -> BTreeMap<String, u16> {
    ls.bodies
        .iter()
        .filter_map(|body| {
            let slot = body.slot?;
            Some((
                format!("{}.{}[{}]", gp_interp::bodies::PREFIX, body.kind, body.nth),
                slot,
            ))
        })
        .collect()
}

/// The game's labels (its folder's `labels.toml`), or `file`; the file used
/// and why there are none.
fn names_for(game: &Path, file: Option<&Path>) -> (Names, Option<PathBuf>, Option<String>) {
    let p = match file {
        Some(f) => f.to_path_buf(),
        None => {
            let p = game.join(labelfile::FILE);
            if !p.is_file() {
                return (
                    Names::empty(),
                    None,
                    Some(format!("{} doesn't exist", p.display())),
                );
            }
            p
        }
    };
    match std::fs::read_to_string(&p)
        .map_err(anyhow::Error::from)
        .and_then(|t| labelfile::parse(&t))
    {
        Ok(f) => (Names::from_file(&f), Some(p), None),
        Err(e) => (
            Names::empty(),
            Some(p.clone()),
            Some(format!("{}: {e:#}", p.display())),
        ),
    }
}

/// A game's labels file (docs/formats/labels.md), read as game-printer's
/// serve/labels.rs read it, until wave 1's `gp labels` (labels.rs) lands
/// with the one reader.
mod labelfile {
    use anyhow::Result;
    use serde::Deserialize;

    /// A game's labels file, in its folder.
    pub(crate) const FILE: &str = "labels.toml";

    #[derive(Debug, Clone, Deserialize)]
    pub(crate) struct LabelsFile {
        pub schema: u32,
        #[allow(dead_code, reason = "read for its check, as the format has it")]
        pub game: String,
        #[serde(default)]
        pub label: Vec<Label>,
        #[serde(default)]
        pub routine: Vec<Rom>,
        #[serde(default)]
        pub data: Vec<Rom>,
    }

    #[derive(Debug, Clone, Deserialize)]
    pub(crate) struct Rom {
        pub name: String,
        pub addr: u16,
        pub size: u32,
    }

    #[derive(Debug, Clone, Deserialize)]
    pub(crate) struct Label {
        pub name: String,
        pub addr: u16,
        pub size: u16,
    }

    pub(crate) fn parse(text: &str) -> Result<LabelsFile> {
        let file: LabelsFile = toml::from_str(text)?;
        anyhow::ensure!(file.schema == 0, "labels schema {} isn't 0", file.schema);
        Ok(file)
    }
}

/// The output as text: per byte, the routine, the table, the sentence.
pub fn text_of(v: &Value) -> String {
    let mut out = format!(
        "{} ({}), frame {} (the original's {}){}{}",
        v["clip"].as_str().unwrap_or(""),
        v["clip_name"].as_str().unwrap_or(""),
        v["frame"],
        v["original_frame"],
        v["measure"]
            .as_str()
            .map_or(String::new(), |m| format!(": {m}")),
        v["slot"]
            .as_u64()
            .map_or(String::new(), |s| format!(", slot {s}")),
    );
    if let Some(m) = v["labels_missing"].as_str() {
        out.push_str(&format!("\nNo labels: {m}. Names are addresses."));
    }
    for b in v["bytes"].as_array().into_iter().flatten() {
        out.push_str("\n\n");
        out.push_str(&byte_text(b));
    }
    if let Some(r) = v["entity_routines"].as_array().filter(|r| !r.is_empty()) {
        let names: Vec<&str> = r.iter().filter_map(Value::as_str).collect();
        out.push_str(&format!(
            "\n\nRoutines that wrote the {}'s slot on the frame: {}",
            v["entity"].as_str().unwrap_or("element"),
            names.join(", ")
        ));
    }
    let gaps = v["gaps"].as_array().map_or(0, Vec::len);
    out.push_str(&if gaps == 0 {
        "\n\nThe slice is exact for every instruction it covers.".to_owned()
    } else {
        format!("\n\nIt gives up in {gaps} place(s), listed above.")
    });
    out
}

fn byte_text(b: &Value) -> String {
    let mut out = format!(
        "{} ({} {}, the measure's {}), {} after the frame",
        b["name"].as_str().unwrap_or("?"),
        b["addr"].as_str().unwrap_or(""),
        b["byte"].as_str().unwrap_or(""),
        b["part"].as_str().unwrap_or(""),
        b["value"]
    );
    if let Some(n) = b["note"].as_str() {
        out.push_str(&format!("\n{n}"));
    }
    if b["written"] != true {
        return out;
    }
    out.push_str(&slice_text(b, ""));
    for e in b["earlier"].as_array().into_iter().flatten() {
        let through = e["through"].as_str().unwrap_or("?");
        if e["with_start"] == true {
            out.push_str(&format!(
                "\n{through} came with the clip's start: no frame of the clip wrote it before"
            ));
        } else {
            out.push_str(&format!(
                "\n{through} was set on frame {}{}:",
                e["frame_written"],
                e["stepped"]["frames"]
                    .as_u64()
                    .map_or(String::new(), |n| format!(
                        ", then stepped on {n} frame(s) by {}",
                        e["stepped"]["by"].as_str().unwrap_or("?")
                    ))
            ));
            out.push_str(&slice_text(e, "  "));
        }
    }
    out.push_str(&format!("\n{}", b["sentence"].as_str().unwrap_or("")));
    out
}

fn slice_text(b: &Value, indent: &str) -> String {
    let w = &b["writer"];
    let mut out = format!(
        "\n{indent}Written by {} at {}{}",
        w["routine"].as_str().unwrap_or("?"),
        w["pc"].as_str().unwrap_or(""),
        w["call"]
            .as_str()
            .filter(|c| Some(*c) != w["routine"].as_str())
            .map_or(String::new(), |c| format!(", in the call to {c}")),
    );
    if let Some(c) = b["constant"].as_str() {
        out.push_str(&format!(", a constant ({c})"));
    }
    let reads = b["reads"].as_array().cloned().unwrap_or_default();
    if !reads.is_empty() {
        let rows: Vec<[String; 6]> = reads
            .iter()
            .map(|r| {
                [
                    r["by"].as_str().unwrap_or("").to_owned(),
                    r["name"].as_str().unwrap_or("").to_owned(),
                    r["role"].as_str().unwrap_or("").to_owned(),
                    r["how"].as_str().unwrap_or("").to_owned(),
                    format!(
                        "{}{} at {}{}",
                        r["value"],
                        r["times"]
                            .as_u64()
                            .map_or(String::new(), |n| format!(" (read {n} times)")),
                        r["addr"].as_str().unwrap_or(""),
                        r["byte"]
                            .as_str()
                            .filter(|b| Some(*b) != r["name"].as_str() && !b.starts_with('$'))
                            .map_or(String::new(), |b| format!(" ({b})"))
                    ),
                    r["from"].as_str().unwrap_or("").to_owned(),
                ]
            })
            .collect();
        out.push_str(&table(
            indent,
            ["Routine", "Read", "Role", "How", "Value", "From"],
            &rows,
        ));
    }
    for r in b["registers"].as_array().into_iter().flatten() {
        out.push_str(&format!(
            "\n{indent}Took {} ({}), {}",
            r["register"].as_str().unwrap_or(""),
            r["how"].as_str().unwrap_or(""),
            r["from"].as_str().unwrap_or("")
        ));
    }
    let br = b["branches"].as_array().map_or(0, Vec::len);
    if br > 0 {
        let taken = b["branches"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|x| x["taken"] == true)
            .count();
        out.push_str(&format!(
            "\n{indent}{br} branch(es) on the way, {taken} taken (their tests are the reads marked branch)"
        ));
    }
    for g in b["gaps"].as_array().into_iter().flatten() {
        out.push_str(&format!(
            "\n{indent}Gives up: {}",
            g["says"].as_str().unwrap_or("")
        ));
    }
    for f in b["feeds"].as_array().into_iter().flatten() {
        out.push_str(&format!(
            "\n{indent}{} was set earlier in the frame:",
            f["through"].as_str().unwrap_or("?")
        ));
        out.push_str(&slice_text(f, &format!("{indent}  ")));
    }
    out
}

/// A plain table, columns padded.
fn table(indent: &str, head: [&str; 6], rows: &[[String; 6]]) -> String {
    let mut width: [usize; 6] = head.map(str::len);
    for r in rows {
        for (i, c) in r.iter().enumerate() {
            width[i] = width[i].max(c.len());
        }
    }
    let line = |cells: [&str; 6]| -> String {
        let mut s = format!("\n{indent}");
        for (i, c) in cells.iter().enumerate() {
            s.push_str(&format!("{c:<w$}", w = width[i]));
            if i < 5 {
                s.push_str("  ");
            }
        }
        s.trim_end().to_owned()
    };
    let mut out = line(head);
    for r in rows {
        out.push_str(&line([&r[0], &r[1], &r[2], &r[3], &r[4], &r[5]]));
    }
    out
}

/// Names of every RAM label, for tests.
#[cfg(test)]
fn sample_names() -> Names {
    let f = labelfile::parse(
        r#"
schema = 0
game = "test"
[[label]]
name = "Random"
addr = 0x07A7
size = 1
[[label]]
name = "Enemy_Y_Speed"
addr = 0x00A0
size = 1
[[routine]]
name = "Mover"
bank = 0
addr = 0xC100
prg = 0x0100
size = 16
"#,
    )
    .unwrap();
    Names::from_file(&f)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_bytes_by_the_label_at_or_just_below() {
        let n = sample_names();
        assert_eq!(n.ram(0x07A7), "Random");
        assert_eq!(n.ram(0x07A8), "Random+1");
        assert_eq!(n.ram(0x00A3), "Enemy_Y_Speed+3");
        assert_eq!(n.ram(0x0010), "$0010");
        assert_eq!(n.ram(0x4016), "JOYPAD1");
        assert_eq!(n.routine(0xC105), "Mover");
        assert_eq!(n.routine(0xC110), "$C110");
        assert_eq!(n.resolve("Random+1"), Some(0x07A8));
        assert_eq!(n.resolve("$00a3"), Some(0xA3));
        assert_eq!(n.resolve("0xa3"), Some(0xA3));
        assert_eq!(n.resolve("Nope"), None);
    }

    #[test]
    fn roles_from_names_and_the_trace() {
        let r = |a: u16, name: &str, how, from| role(a, name, how, from);
        assert_eq!(r(0x07A8, "Random+1", How::Value, From::Before), "random");
        assert_eq!(r(0x4016, "JOYPAD1", How::Value, From::Io), "controller");
        assert_eq!(r(0xC000, "$C000", How::Value, From::Rom), "table");
        assert_eq!(r(0x0000, "$0000", How::Value, From::Outside(3)), "scratch");
        assert_eq!(
            r(0x00A0, "Enemy_Y_Speed+x", How::Value, From::Before),
            "state"
        );
    }

    #[test]
    fn the_sentence_names_values_then_tests_then_feeds() {
        let b = json!({
            "written": true, "name": "Enemy_Y_Speed+x", "constant": null,
            "writer": {"routine": "MovePodoboo"},
            "reads": [
                {"name": "PseudoRandomBitReg+1+x", "how": "value", "role": "random"},
                {"name": "EnemyIntervalTimer+x", "how": "branch", "role": "timer"},
                {"name": "Enemy_Y_HighPos+x", "how": "value", "role": "state"},
            ],
            "registers": [],
            "feeds": [{"through": "$0000", "writer": {"routine": "Caller"},
                       "reads": [{"name": "FrameCounter", "how": "value", "role": "frame counter"}],
                       "registers": []}],
        });
        assert_eq!(
            sentence(&b),
            "MovePodoboo wrote Enemy_Y_Speed+x from PseudoRandomBitReg+1+x and Enemy_Y_HighPos+x; it tested EnemyIntervalTimer+x; $0000 came from Caller from FrameCounter."
        );
        let c = json!({"written": true, "name": "Enemy_Y_Speed+x", "constant": "$F9",
                       "writer": {"routine": "MovePodoboo"}, "reads": [], "registers": [], "feeds": []});
        assert_eq!(
            sentence(&c),
            "MovePodoboo wrote Enemy_Y_Speed+x a constant ($F9)."
        );
    }
}
