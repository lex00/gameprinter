//! `gp track`: the objects of a recording found from its frame trace, with
//! no hand-written mapping (Milestone 2, step 7; docs/formats/tracks.md).
//! The tracker is `gp-track`; this module feeds it a recording's states and
//! prints what it finds.
//!
//! The tracker reads a frame's sprites as a list (`gp_track::sprites::Sprite`);
//! here they come from the NES's sprite table in a gp-nes recording's trace.
//! An arcade board's recording would need its own adapter from sprite RAM to
//! that list, so one is refused in words until gp-arcade has one.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::Args;
use serde_json::json;

use gp_evidence::trace::FieldTable;
use gp_track::{Frame, Options, Report, Source};

use crate::Outcome;
use crate::open::Opened;
use crate::recordings::parse_frames;

#[derive(Args)]
pub struct TrackArgs {
    /// The recording's ID, or a unique prefix of at least 4 characters.
    pub id: String,
    /// Which frames: `N`, `A..B` (B excluded), `A..=B` or `A..`.
    #[arg(long, default_value = "1..")]
    pub frames: String,
    /// Print the proposed `[slots]` section (docs/formats/state-mapping.md).
    #[arg(long)]
    pub propose_slots: bool,
    /// Score the proposed slots against this hand-written mapping.toml:
    /// precision and recall over the addresses it lists.
    #[arg(long, value_name = "MAPPING")]
    pub against: Option<PathBuf>,
    /// Frames the sprite table lags the game's RAM: 1 for a game that
    /// copies its sprites to the PPU at the start of the next frame (most
    /// do), 0 for one that writes them within the frame.
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u64).range(0..=1))]
    pub oam_lag: u64,
}

/// Frames of a recording's trace, shaped for the tracker: frame `f` pairs
/// the RAM of state `f` with the sprites and scroll of state `f + lag`,
/// which are what the console drew from it.
struct TraceSource<'a> {
    opened: &'a Opened,
    table: FieldTable,
    range: std::ops::Range<u64>,
    pads: Vec<u8>,
    lag: u64,
}

impl Source for TraceSource<'_> {
    fn scan(&mut self, each: &mut dyn FnMut(&Frame<'_>)) -> Result<(), String> {
        let last = self.opened.manifest.frames;
        let end = (self.range.end - 1 + self.lag).min(last);
        let mut held: std::collections::VecDeque<(u64, Vec<u8>)> = Default::default();
        let mut failed = None;
        let ok = gp_nes::recording::scan_trace(
            &self.opened.store,
            &self.opened.manifest,
            self.range.start..=end,
            |frame, state| {
                let fields = match self.table.split(state) {
                    Ok(f) => f,
                    Err(e) => {
                        failed = Some(e.to_string());
                        return false;
                    }
                };
                let get = |name: &str| fields.iter().find(|(n, _)| *n == name).map(|(_, b)| *b);
                let (Some(ram), Some(oam)) = (get("ram"), get("oam")) else {
                    failed = Some("the trace has no `ram` and `oam` fields".into());
                    return false;
                };
                held.push_back((frame, ram.to_vec()));
                if frame < self.range.start + self.lag {
                    return true;
                }
                let (f, ram) = held.pop_front().expect("pushed above");
                let t = get("ppu.t").map_or(0, |b| u16::from_le_bytes([b[0], b[1]]));
                let fine = get("ppu.x").map_or(0, |b| u16::from(b[0] & 7));
                let scroll_x = (t & 31) * 8 + fine + ((t >> 10) & 1) * 256;
                let tall = get("ppu.ctrl").is_some_and(|b| b[0] & 0x20 != 0);
                if f >= self.range.end {
                    return false;
                }
                each(&Frame {
                    frame: f,
                    ram: &ram,
                    oam,
                    tall,
                    sprites: None,
                    scroll_x,
                    pad: self.pads[(f - self.range.start) as usize],
                });
                true
            },
        )
        .map_err(|e| e.to_string())?;
        if let Some(e) = failed {
            return Err(e);
        }
        if !ok {
            return Err("the recording has no frame trace".into());
        }
        Ok(())
    }
}

/// A recording's objects as the tracker finds them, with what `gp model
/// propose` needs beside them: the frames tracked and each one's
/// controller.
pub(crate) struct Tracked {
    pub opened: Opened,
    pub range: std::ops::Range<u64>,
    /// The controller of each frame in `range`, in order.
    pub pads: Vec<u8>,
    pub report: Report,
}

/// Runs the tracker over the frames `frames` names of the recording `id`.
pub(crate) fn tracked(
    paths: &gp_evidence::Paths,
    id: &str,
    frames: &str,
    lag: u64,
) -> Result<Tracked> {
    let opened = Opened::open(paths, id)?;
    nes_recording(&opened, "gp track")?;
    let manifest = &opened.manifest;
    if manifest.trace.is_none() {
        bail!(
            "{} has no frame trace; gp track reads states from it (gp record stores one unless --no-trace)",
            opened.short()
        );
    }
    let range = parse_frames(frames, manifest.frames)?;
    // Frame 0 has no input behind it.
    let range = range.start.max(1)..range.end;
    if range.is_empty() {
        bail!("--frames {frames} leaves no frame to track (frame 0 is the power-on state)");
    }
    let log = gp_evidence::InputLog::parse(
        &String::from_utf8(opened.store.get(&manifest.input_log)?)
            .context("the recording's input log isn't text")?,
    )?;
    // State f ran with input log entry f - 1.
    let pads: Vec<u8> = range
        .clone()
        .map(|f| (log.frames[(f - 1) as usize].buttons_word() & 0xFF) as u8)
        .collect();
    let table = gp_nes::recording::field_table(&opened.store, manifest)?;
    let report = {
        let mut source = TraceSource {
            opened: &opened,
            table,
            range: range.clone(),
            pads: pads.clone(),
            lag,
        };
        gp_track::track(&mut source, &Options::default()).map_err(anyhow::Error::msg)?
    };
    Ok(Tracked {
        opened,
        range,
        pads,
        report,
    })
}

/// Refuses a recording that isn't gp-nes's in words: the detectors read the
/// NES's sprite table, nametables and APU from its trace and events.
pub(crate) fn nes_recording(opened: &Opened, what: &str) -> Result<()> {
    let name = &opened.manifest.runtime.name;
    if name != "gp-nes" {
        bail!(
            "{} is a {name} recording; {what} reads the NES's sprite table and video memory from a gp-nes trace, and gp has no adapter from another board's yet",
            opened.short()
        );
    }
    Ok(())
}

/// Refuses a game whose package names an arcade board, in words: the
/// detectors read the NES's sprite table, nametables and APU. A folder with
/// no package.toml (a template) is an NES game's.
pub(crate) fn nes_package(dir: &std::path::Path, what: &str) -> Result<()> {
    if !dir.join("package.toml").is_file() {
        return Ok(());
    }
    if crate::package::Package::load(dir)?.platform()? == crate::roms::Platform::Arcade {
        bail!(
            "{} is an arcade game; {what} reads the NES's sprite table, nametables and APU, and gp has no arcade counterpart yet (an adapter from the board's sprite RAM, tilemap and sound chip)",
            dir.display()
        );
    }
    Ok(())
}

pub(crate) fn run(args: &TrackArgs, paths: &gp_evidence::Paths) -> Result<Outcome> {
    let Tracked {
        opened,
        range,
        report,
        ..
    } = tracked(paths, &args.id, &args.frames, args.oam_lag)?;

    let mapping = args
        .against
        .as_ref()
        .map(|p| fs::read_to_string(p).with_context(|| format!("reading {}", p.display())))
        .transpose()?;
    let score = mapping
        .as_deref()
        .map(|m| gp_track::truth::score(&report, m))
        .transpose()
        .map_err(anyhow::Error::msg)?;
    let draft = gp_track::slots::draft(
        &report.groups,
        report.clock.as_ref(),
        report.player.as_ref(),
        &opened.entry.id.to_hex()[..8],
    );
    let text = if args.propose_slots {
        draft.clone()
    } else {
        text(&report, score.as_ref(), &opened.short())
    };
    let mut json = json!({
        "id": opened.entry.id,
        "frames": [range.start, range.end - 1],
        "report": report,
        "score": score,
    });
    if args.propose_slots {
        json["slots"] = json!(draft);
    }
    Ok(Outcome::ok(json, text))
}

fn text(r: &Report, score: Option<&gp_track::truth::Score>, name: &str) -> String {
    let mut s = String::new();
    let _ = writeln!(
        s,
        "{name}: frames {}..={}, {} tracks, {} slot groups",
        r.first,
        r.last,
        r.tracks.len(),
        r.groups.len()
    );
    let _ = writeln!(s, "scroll read against: {:?}", r.camera);
    match &r.player {
        Some(p) => {
            let _ = write!(
                s,
                "player: x 0x{:04X} ({:.2}), moves with the controller {:.2}",
                p.x.ram, p.x.score, p.input
            );
            if let Some(g) = &p.page {
                let _ = write!(s, ", page 0x{:04X}", g.ram);
            }
            if let Some(y) = &p.y {
                let _ = write!(s, ", y 0x{:04X}", y.ram);
            }
            s.push('\n');
        }
        None => s.push_str("player: none found\n"),
    }
    if let Some(c) = &r.clock {
        let _ = writeln!(s, "frame counter: 0x{:04X} ({:.2})", c.ram, c.score);
    }
    for (i, g) in r.groups.iter().enumerate() {
        let _ = write!(
            s,
            "group {i}: {} slots, stride {}, x 0x{:04X} ({:.2})",
            g.count, g.stride, g.x.ram, g.x.score
        );
        if let Some(p) = &g.x_page {
            let _ = write!(s, ", page 0x{:04X}", p.ram);
        }
        if let Some(y) = &g.y {
            let _ = write!(s, ", y 0x{:04X}", y.ram);
        }
        if let Some(a) = &g.active {
            let _ = write!(s, ", active 0x{:04X} ({:.2})", a.ram, a.score);
        }
        if let Some(id) = &g.id {
            let _ = write!(s, ", id 0x{:04X} ({:.2})", id.ram, id.score);
        }
        let _ = writeln!(s, "; {} tracks over {} frames", g.tracks, g.frames);
    }
    if !r.kinds.is_empty() {
        s.push_str("kinds (id, palette, tracks, frames, speed x, period):\n");
        for k in r.kinds.iter().take(24) {
            let _ = writeln!(
                s,
                "  {:>4} {} {:>4} {:>6} {:>6} {:>7}",
                k.id.map_or("-".into(), |i| format!("0x{i:02X}")),
                k.palette,
                k.tracks,
                k.frames,
                k.speed_x.map_or("-".into(), |v| format!("{v:.3}")),
                k.period.map_or("-".into(), |v| format!("{v:.1}")),
            );
        }
    }
    let long: Vec<_> = r.tracks.iter().filter(|t| t.seen >= 16).collect();
    let _ = writeln!(
        s,
        "{} tracks of 16 or more frames; the first 20:",
        long.len()
    );
    for t in long.iter().take(20) {
        let _ = writeln!(
            s,
            "  #{:<4} frames {}..={} ({} drawn){} {}",
            t.track,
            t.spawn,
            t.despawn,
            t.seen,
            match t.slot {
                Some((g, i)) => format!(" group {g} slot {i}"),
                None if t.player => " player".to_string(),
                None => String::new(),
            },
            match (t.motion.speed_x, t.motion.period) {
                (Some(v), Some(p)) => format!("speed {v:.3}, period {p:.1}"),
                (Some(v), None) => format!("speed {v:.3}"),
                _ => String::new(),
            },
        );
    }
    if let Some(sc) = score {
        let _ = writeln!(
            s,
            "against the mapping: precision {:.3}, recall {:.3} ({} of {} proposed, {} listed; {} other groups)",
            sc.precision, sc.recall, sc.correct, sc.proposed, sc.truth, sc.other_groups
        );
        if !sc.missed.is_empty() {
            let _ = writeln!(s, "  missed: {}", sc.missed.join(" "));
        }
        if !sc.wrong.is_empty() {
            let _ = writeln!(s, "  not in the mapping: {}", sc.wrong.join(" "));
        }
    }
    s.trim_end().to_string()
}

/// Where a game keeps its object slots, from a state mapping's `[slots]`:
/// slot i's x byte is `x + i` and its object ID `id + i`, while `active + i`
/// isn't zero.
#[allow(dead_code)] // gp-extract's feel and capture read it (port wave 5)
pub(crate) struct SlotIds {
    pub count: u16,
    pub x: u16,
    pub id: u16,
    pub active: u16,
}

#[allow(dead_code)]
impl SlotIds {
    /// From a mapping's `[slots]`, when it gives the slots' IDs and an `x`
    /// field one byte a slot.
    pub(crate) fn of(m: &gp_interp::mapping::Mapping) -> Option<Self> {
        let s = m.slots.as_ref()?;
        if s.active_stride.is_some() || s.free.is_some() {
            return None;
        }
        let x = s
            .fields
            .iter()
            .find(|f| f.path == "x" && f.stride.is_none())?
            .ram
            .first()
            .copied()?;
        Some(SlotIds {
            count: s.count,
            x,
            id: s.id?,
            active: s.active,
        })
    }
}

/// Fills in each enemy track's object ID from the mapping's slots where
/// the tracker found none: the track's x byte names its slot, and the
/// slot's ID byte, read from the trace on the track's frames while the
/// slot is active, gives the ID it holds most often. This is how capture
/// tells enemy kinds apart for any game whose mapping names its slots.
#[allow(dead_code)] // gp-extract's feel and capture read it (port wave 5)
pub(crate) fn ids_from_slots(t: &mut Tracked, slots: &SlotIds) -> Result<usize> {
    // The slot of each track that has none yet.
    let mut want: BTreeMap<usize, u16> = BTreeMap::new();
    for (i, tr) in t.report.tracks.iter().enumerate() {
        if tr.player || tr.id.is_some() {
            continue;
        }
        let Some(b) = &tr.x else { continue };
        if (slots.x..slots.x + slots.count).contains(&b.ram) {
            want.insert(i, b.ram - slots.x);
        }
    }
    if want.is_empty() {
        return Ok(0);
    }
    let frames: Vec<BTreeSet<u64>> = t
        .report
        .tracks
        .iter()
        .map(|tr| tr.positions.iter().map(|p| p[0] as u64).collect())
        .collect();
    let mut counts: BTreeMap<usize, BTreeMap<u8, usize>> = BTreeMap::new();
    let table = gp_nes::recording::field_table(&t.opened.store, &t.opened.manifest)?;
    let mut source = TraceSource {
        opened: &t.opened,
        table,
        range: t.range.clone(),
        pads: t.pads.clone(),
        lag: 0,
    };
    source
        .scan(&mut |f| {
            for (&track, &slot) in &want {
                if !frames[track].contains(&f.frame) {
                    continue;
                }
                let at = |a: u16| f.ram.get(usize::from(a + slot)).copied().unwrap_or(0);
                if at(slots.active) != 0 {
                    *counts
                        .entry(track)
                        .or_default()
                        .entry(at(slots.id))
                        .or_default() += 1;
                }
            }
        })
        .map_err(anyhow::Error::msg)?;
    let mut filled = 0;
    for (track, c) in counts {
        if let Some((&id, _)) = c.iter().max_by_key(|&(id, n)| (*n, std::cmp::Reverse(*id))) {
            t.report.tracks[track].id = Some(id);
            filled += 1;
        }
    }
    Ok(filled)
}

/// Where a mapping says the player's whole-pixel position is: the bytes of
/// `entities.<player>[0].x` and `.y`, low byte first.
#[allow(dead_code)] // gp-extract's feel and capture read it (port wave 5)
pub(crate) struct PlayerAt {
    pub x: Vec<u16>,
    pub y: Vec<u16>,
}

#[allow(dead_code)]
impl PlayerAt {
    pub(crate) fn of(m: &gp_interp::mapping::Mapping, player: &str) -> Option<Self> {
        let at = |axis: &str| {
            let path = format!("entities.{player}[0].{axis}");
            m.fields
                .iter()
                .find(|f| f.path == path && !f.flag && f.bits.is_none() && !f.digits)
                .map(|f| f.ram.clone())
                .filter(|r| !r.is_empty() && r.len() <= 2)
        };
        Some(PlayerAt {
            x: at("x")?,
            y: at("y")?,
        })
    }
}

/// When the tracker found no player, adds one from the mapping's player
/// fields: its position each frame read from RAM, so the feel of a game
/// whose player the sprite tracker can't pick out (one drawn in pieces
/// that change, or masked) is still measured. Returns whether it added one.
#[allow(dead_code)] // gp-extract's feel and capture read it (port wave 5)
pub(crate) fn player_from_mapping(t: &mut Tracked, at: &PlayerAt) -> Result<bool> {
    if t.report.tracks.iter().any(|tr| tr.player) {
        return Ok(false);
    }
    let table = gp_nes::recording::field_table(&t.opened.store, &t.opened.manifest)?;
    let mut positions = Vec::new();
    let mut source = TraceSource {
        opened: &t.opened,
        table,
        range: t.range.clone(),
        pads: t.pads.clone(),
        lag: 0,
    };
    let read = |ram: &[u8], bytes: &[u16]| -> i64 {
        bytes
            .iter()
            .enumerate()
            .map(|(i, &a)| i64::from(ram.get(usize::from(a)).copied().unwrap_or(0)) << (8 * i))
            .sum()
    };
    source
        .scan(&mut |f| {
            positions.push([f.frame as i64, read(f.ram, &at.x), read(f.ram, &at.y)]);
        })
        .map_err(anyhow::Error::msg)?;
    if positions.is_empty() {
        return Ok(false);
    }
    let track = t.report.tracks.len();
    t.report.tracks.push(gp_track::TrackReport {
        track,
        spawn: positions[0][0] as u64,
        despawn: positions[positions.len() - 1][0] as u64,
        seen: positions.len(),
        palette: 0,
        tiles: Vec::new(),
        x: None,
        y: None,
        slot: None,
        player: true,
        id: None,
        motion: gp_track::motion::measure(&positions),
        positions,
    });
    Ok(true)
}
