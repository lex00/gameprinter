//! `gp corpus reduce`: the fewest clips of a template's clip set that
//! still run every part of the ROM's code the set runs, as a plan
//! (ROADMAP.md, "Final phase: the smallest corpus", goals 1 to 4).
//!
//! Each clip's own frames (its start to its end) are counted as `gp code
//! listing` counts them (`gp_events::profile`), cut where the template's
//! stage byte changes (the mapping field the model's `[stages] counter`
//! names, `$53` on Donkey Kong), and kept in the events store. A clip's
//! features are the routines it ran and the branch ways it took, each
//! tagged with the stage it was on (system code is not, unless
//! `--stage-system`). With `--values`, after the cover, each sweep family
//! (clips with the same start whose own presses differ only in timing)
//! keeps the members whose compare classes near a boundary differ from
//! its kept members': one below, on or one above, at the flips of a
//! branch that decides at most twice in the window, in a routine every
//! member runs through the same branch ways, over the same frames.
//!
//! The cover is `gp capture prune`'s (docs/research/clip-set.md): greedy
//! weighted multi-cover, the clip with the most features not yet held per
//! frame first, then a reverse pass that takes out again, dearest first,
//! any clip whose features are all held enough without it. Guards are
//! kept whatever the cover: the first clip each fix commit of the
//! template's model, mapping and hooks names (git log), and the clips
//! `--keep` names. Clips whose run is the same as another's (the same
//! input from power-on and the same own frames) are exact duplicates: one
//! of each stands for all.
//!
//! The output is a plan only: `--write PLAN` writes the kept and dropped
//! clips with reasons to a file outside the repo. The clips (the game's
//! corpus plan, corpus.toml, or an older clips.toml) are never changed.
//!
//! Ported from game-printer, where it read a template's clips.toml; here it
//! reads a game package's clips as `gp corpus score` does (its corpus plan,
//! else clips.toml), and runs on an arcade romset too. There each run is
//! counted on gp-arcade with its levels watching both Z80s
//! ([`gp_arcade::levels`], as `gp reach --window` does on a romset): a
//! routine is an entry of the code map ([`crate::code_z80::CodeMap`]),
//! named `main:5FAC` or `sound:0276` with why it is one as its element, and a
//! branch way is a conditional's edge, taken or fallen. The stage byte is a
//! main CPU address; a game whose model has no `[stages] counter` (Tapper's
//! has none yet) is counted as one stage. Arcade windows are not kept in the
//! events store (it holds 6502 counts), and `--values` reads the NES's
//! compare classes, so it is refused on a romset.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context as _, Result, bail};
use gp_events::profile::{COMPARE_CLASSES, Segment, run_segments_on};
use gp_evidence::Paths;
use gp_nes::codemap::{Mode, Va, va_hex};
use serde_json::{Value, json};

use crate::Outcome;
use crate::clipset as cs;
use crate::code::{CodeMap, event_store};

#[derive(clap::Args, Debug, Clone)]
pub struct ReduceArgs {
    /// The game's package folder (games/donkey-kong) or a template folder:
    /// its corpus plan (corpus.toml) or clips.toml, model.toml and
    /// mapping.toml.
    pub template: PathBuf,
    /// The iNES ROM or arcade romset (default: the one in $GP_ROM_DIR the
    /// package accepts).
    #[arg(long)]
    pub rom: Option<PathBuf>,
    /// The RAM byte that says the stage (`$53`). Default: the first RAM
    /// byte of the mapping field the model's `[stages] counter` names
    /// (`counters.<counter>`). On a romset with neither, every frame is one
    /// stage.
    #[arg(long, value_name = "ADDR")]
    pub stage_byte: Option<String>,
    /// Keep this clip whatever the cover: an ID, or a prefix ending in
    /// `*`. Repeat for more.
    #[arg(long, value_name = "ID")]
    pub keep: Vec<String>,
    /// Take no guards from the fix commits (git log of the template's
    /// model.toml, mapping.toml and ext/).
    #[arg(long)]
    pub no_guards: bool,
    /// Hold every stage-tagged feature by two clips where two take it.
    #[arg(long)]
    pub twice: bool,
    /// Tag system code (the code map's `system` routines) with the stage
    /// too.
    #[arg(long)]
    pub stage_system: bool,
    /// After the cover, keep the sweep members that differ near a
    /// boundary. A sweep family is clips with the same start whose own
    /// presses differ only in timing. Each member is counted over the same
    /// frames: to a second after the last frame the presses differ. A
    /// compare's class at a branch's flips (register one below, on or one
    /// above the operand, in level code) counts when the branch is an
    /// event (its rarer way taken at most twice) in a routine every member
    /// runs through the same branch ways. A member is added when it shows
    /// such a class no kept member of its family shows.
    #[arg(long)]
    pub values: bool,
    /// Write the plan (kept and dropped clips with reasons, and the
    /// untaken branch ways) to this file, outside any git work tree.
    #[arg(long, value_name = "PLAN")]
    pub write: Option<PathBuf>,
    /// The store of counted windows (default $GP_HOME/work/events).
    #[arg(long)]
    pub store: Option<PathBuf>,
    /// Count every clip again, reading nothing from the store.
    #[arg(long)]
    pub no_store: bool,
}

/// The stage byte and the names of its values. `addr` is `None` only on a
/// romset whose model names no stage counter: one stage.
struct StageByte {
    addr: Option<u16>,
    names: BTreeMap<u8, String>,
}

impl StageByte {
    fn text(&self) -> String {
        self.addr
            .map_or_else(|| "none".to_owned(), |a| format!("${a:02X}"))
    }

    fn name(&self, v: u8) -> String {
        match self.names.get(&v) {
            Some(n) => n.clone(),
            None => format!("{v}"),
        }
    }
}

fn parse_addr(s: &str) -> Result<u16> {
    let t = s.trim();
    let h = t
        .strip_prefix('$')
        .or_else(|| t.strip_prefix("0x"))
        .unwrap_or(t);
    u16::from_str_radix(h, 16).with_context(|| format!("{s:?} isn't an address ($53)"))
}

/// The stage byte from `--stage-byte`, else from the model's `[stages]
/// counter` and the mapping's field for it. A value is named by the
/// model's `[stages] order` through the field's lookup; where two values
/// look up one stage, the largest takes the name.
fn stage_byte(t: &Path, given: Option<&str>, optional: bool) -> Result<StageByte> {
    let read = |f: &str| -> Option<toml::Value> {
        toml::from_str(&fs::read_to_string(t.join(f)).ok()?).ok()
    };
    let model = read("model.toml");
    let mapping = read("mapping.toml");
    let stages = model.as_ref().and_then(|m| m.get("stages"));
    let counter = stages
        .and_then(|s| s.get("counter"))
        .and_then(|c| c.as_str());
    let order: Vec<String> = stages
        .and_then(|s| s.get("order"))
        .and_then(|o| o.as_array())
        .map(|o| {
            o.iter()
                .filter_map(|x| x.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let fields: Vec<toml::Value> = mapping
        .as_ref()
        .and_then(|m| m.get("field"))
        .and_then(|f| f.as_array())
        .cloned()
        .unwrap_or_default();
    let ram0 = |f: &toml::Value| -> Option<u16> {
        u16::try_from(f.get("ram")?.as_array()?.first()?.as_integer()?).ok()
    };
    let field = counter.and_then(|c| {
        let path = format!("counters.{c}");
        fields
            .iter()
            .find(|f| f.get("path").and_then(|p| p.as_str()) == Some(path.as_str()))
    });
    let addr = match given {
        Some(s) => parse_addr(s)?,
        None => match field.and_then(ram0) {
            Some(a) => a,
            None if optional => {
                return Ok(StageByte {
                    addr: None,
                    names: [(0, "all".to_owned())].into(),
                });
            }
            None => bail!(
                "{}: no stage byte: the model's [stages] counter names no mapping field with a RAM byte; give --stage-byte",
                t.display()
            ),
        },
    };
    let mut names = BTreeMap::new();
    if let Some(f) = field.filter(|f| ram0(f) == Some(addr)) {
        let mut by_stage: BTreeMap<usize, u8> = BTreeMap::new();
        for pair in f
            .get("lookup")
            .and_then(|l| l.as_array())
            .into_iter()
            .flatten()
        {
            let p = pair.as_array();
            let v = p.and_then(|p| p.first()?.as_integer());
            let i = p.and_then(|p| p.get(1)?.as_integer());
            if let (Some(v), Some(i)) = (v, i)
                && let (Ok(v), Ok(i)) = (u8::try_from(v), usize::try_from(i))
            {
                let e = by_stage.entry(i).or_insert(v);
                *e = (*e).max(v);
            }
        }
        for (i, v) in by_stage {
            if let Some(n) = order.get(i) {
                names.insert(v, n.clone());
            }
        }
    }
    Ok(StageByte {
        addr: Some(addr),
        names,
    })
}

/// A choice of clips by greedy weighted multi-cover, as `gp capture
/// prune` makes it. `feats[i]` are clip i's features (indices below
/// `need.len()`), `need[f]` how many clips should hold feature f (capped
/// at how many hold it), `cost[i]` its price in frames, `forced` the clips
/// kept whatever. The clip with the most features still short per frame
/// is taken first (ties: the cheaper, then the first); then, dearest
/// first, a clip not forced whose every feature is held more than needed
/// is taken out again. Returns the kept clips in order.
pub(crate) fn cover(
    feats: &[Vec<usize>],
    need: &[usize],
    cost: &[u64],
    forced: &BTreeSet<usize>,
) -> BTreeSet<usize> {
    let mut avail = vec![0usize; need.len()];
    for fs in feats {
        for &f in fs {
            avail[f] += 1;
        }
    }
    let need: Vec<usize> = need.iter().zip(&avail).map(|(n, a)| *n.min(a)).collect();
    let mut held = vec![0usize; need.len()];
    let mut keep = BTreeSet::new();
    let take = |i: usize, keep: &mut BTreeSet<usize>, held: &mut Vec<usize>| {
        keep.insert(i);
        for &f in &feats[i] {
            held[f] += 1;
        }
    };
    for &i in forced {
        take(i, &mut keep, &mut held);
    }
    let price = |i: usize| 1.0 + cost[i] as f64 / 1000.0;
    loop {
        let mut best: Option<(f64, u64, usize)> = None;
        for i in 0..feats.len() {
            if keep.contains(&i) {
                continue;
            }
            let gain = feats[i].iter().filter(|&&f| held[f] < need[f]).count();
            if gain == 0 {
                continue;
            }
            let r = gain as f64 / price(i);
            let better = match best {
                None => true,
                Some((b, c, _)) => r > b || (r == b && cost[i] < c),
            };
            if better {
                best = Some((r, cost[i], i));
            }
        }
        let Some((_, _, i)) = best else { break };
        take(i, &mut keep, &mut held);
    }
    let mut by_cost: Vec<usize> = keep
        .iter()
        .copied()
        .filter(|i| !forced.contains(i))
        .collect();
    by_cost.sort_by(|a, b| cost[*b].cmp(&cost[*a]).then(b.cmp(a)));
    for i in by_cost {
        if feats[i].iter().all(|&f| held[f] > need[f]) {
            keep.remove(&i);
            for &f in &feats[i] {
                held[f] -= 1;
            }
        }
    }
    keep
}

/// Interns feature names as indices.
#[derive(Default)]
struct Names {
    of: BTreeMap<String, usize>,
    list: Vec<String>,
}

impl Names {
    fn id(&mut self, s: String) -> usize {
        if let Some(&i) = self.of.get(&s) {
            return i;
        }
        self.list.push(s.clone());
        self.of.insert(s, self.list.len() - 1);
        self.list.len() - 1
    }
}

/// One clip of the set, with its run.
struct Clip {
    id: String,
    family: String,
    frames: u64,
    /// Frames from power-on to its end.
    total: u64,
    /// The run it shares with others: the first clip with the same.
    rep: usize,
    run: usize,
}

/// The fix commits of the template: those changing its model, mapping or
/// hooks, newest first, each with the clips its message names in the
/// order it names them (`dk.m100.012`, or `m100 clip 12`).
fn fix_commits(t: &Path, ids: &[String]) -> Vec<(String, String, Vec<usize>)> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(t)
        .args([
            "log",
            "--no-merges",
            "--format=%H%x1f%s%x1f%B%x1e",
            "--",
            "model.toml",
            "mapping.toml",
            "ext",
        ])
        .output();
    let Ok(out) = out else { return Vec::new() };
    if !out.status.success() {
        return Vec::new();
    }
    let by_id: BTreeMap<&str, usize> = ids
        .iter()
        .enumerate()
        .map(|(i, s)| (s.as_str(), i))
        .collect();
    // `fam N` and `short-fam N` (the family without its game prefix).
    let mut by_num: BTreeMap<(String, u64), usize> = BTreeMap::new();
    for (i, id) in ids.iter().enumerate() {
        if let Some((fam, n)) = id.rsplit_once('.')
            && let Ok(n) = n.parse::<u64>()
        {
            by_num.insert((fam.to_owned(), n), i);
            if let Some((_, short)) = fam.split_once('.') {
                by_num.entry((short.to_owned(), n)).or_insert(i);
            }
        }
    }
    let clean = |w: &str| -> String {
        w.trim_matches(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '.' || c == '_'))
            .trim_end_matches('.')
            .to_owned()
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut commits = Vec::new();
    for rec in text.split('\x1e') {
        let parts: Vec<&str> = rec.trim_start_matches('\n').splitn(3, '\x1f').collect();
        if parts.len() < 3 {
            continue;
        }
        let words: Vec<String> = parts[2].split_whitespace().map(clean).collect();
        let mut named = Vec::new();
        for k in 0..words.len() {
            if let Some(&i) = by_id.get(words[k].as_str()) {
                named.push(i);
                continue;
            }
            if matches!(words.get(k + 1).map(String::as_str), Some("clip" | "clips")) {
                let fam = words[k].to_lowercase();
                // `m100 clips 28 and 36`, `m100 clips 28, 36`
                let mut j = k + 2;
                while let Some(n) = words.get(j).and_then(|w| w.parse::<u64>().ok()) {
                    if let Some(&i) = by_num.get(&(fam.clone(), n)) {
                        named.push(i);
                    }
                    j += 1;
                    if words.get(j).map(String::as_str) == Some("and") {
                        j += 1;
                    }
                }
            }
        }
        if !named.is_empty() {
            commits.push((parts[0].to_owned(), parts[1].to_owned(), named));
        }
    }
    commits
}

fn hit(pattern: &str, id: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(pre) => id.starts_with(pre),
        None => id == pattern,
    }
}

/// A window's value features: at each branch right after a compare in
/// the code map's level code (not system or unknown, nor outside it), the
/// compare's class at the branch's flips when the register was one below,
/// on or one above the operand, by way and stage: `V $E2E6 T +1
/// sub_E2B6@1`. The far classes and the frame loop's own compares part
/// every clip from every other, so they are left out.
fn near_classes(
    map: &CodeMap,
    segs: &[Segment],
    vn: &mut Names,
    v_at: &mut Vec<(Va, usize)>,
) -> BTreeSet<usize> {
    let mut out = BTreeSet::new();
    for s in segs {
        for (va, stat) in &map.by_address(&s.profile) {
            if stat.flips == 0 || !map.insn_at(*va).is_some_and(|i| i.mode == Mode::Rel) {
                continue;
            }
            let Some(ri) = map.routine_of(*va) else {
                continue;
            };
            let r = &map.routines[ri];
            if r.element == "system" || r.element == "unknown" {
                continue;
            }
            for bit in 0..10u16 {
                if stat.flips & (1 << bit) != 0 && matches!(bit % 5, 1..=3) {
                    let w = if bit < 5 { "T" } else { "F" };
                    let c = COMPARE_CLASSES[usize::from(bit % 5)];
                    let v = vn.id(format!("V {} {w} {c} {}@{}", va_hex(*va), r.name, s.value));
                    if v == v_at.len() {
                        v_at.push((*va, ri));
                    }
                    out.insert(v);
                }
            }
        }
    }
    out
}

/// Frames counted after the last frame where a sweep family's presses
/// differ: one second.
const AFTER_PRESSES: u64 = 60;

/// A decision is an event when its window takes the rarer way at most
/// this often (a take-off, a landing, an edge), not a test made every
/// frame or for every object.
const EVENT: u64 = 2;

/// A sweep member's counts over its family's window.
#[derive(Default)]
struct Window {
    /// Its value features ([`near_classes`]).
    v: BTreeSet<usize>,
    /// Each branch: times taken and fallen.
    ways: BTreeMap<Va, (u64, u64)>,
    /// The routines it ran.
    ran: BTreeSet<usize>,
}

/// What a clip starts from: its power-on and every frame before its own,
/// as a hash. Clips with the same are the same run until their own frames.
fn start_key(c: &crate::reach::ClipInput, first: u64) -> String {
    let n = usize::try_from(first.saturating_sub(1))
        .unwrap_or(usize::MAX)
        .min(c.inputs.len());
    let v = json!({
        "power_on": serde_json::to_value(c.power_on).unwrap_or_default(),
        "frames": c.inputs[..n].iter().map(|f| json!([f.pads, f.reset])).collect::<Vec<_>>(),
    });
    gp_evidence::Hash::of(&serde_json::to_vec(&v).unwrap_or_default()).to_string()
}

/// The shape of a clip's own presses: what was held, run by run, without
/// how long (the pads of each port and the reset, one number a run).
fn shape(frames: &[gp_evidence::input_log::InputFrame]) -> Vec<u64> {
    let mut out: Vec<u64> = Vec::new();
    for f in frames {
        let v = u64::from(u32::from_le_bytes(f.pads)) | (u64::from(f.reset) << 32);
        if out.last() != Some(&v) {
            out.push(v);
        }
    }
    out
}

/// Whether two shapes are one sweep's: the same, or one is the other with
/// one run held for no frames, of a set of buttons the clip holds again
/// later (a jump on the first frame against one after four frames of the
/// same walk; not standing still against walking left).
fn same_sweep(a: &[u64], b: &[u64]) -> bool {
    if a == b {
        return true;
    }
    let (long, short) = if a.len() > b.len() { (a, b) } else { (b, a) };
    long.len() == short.len() + 1
        && (0..long.len()).any(|r| {
            long[..r] == short[..r]
                && long[r + 1..] == short[r..]
                && long.iter().filter(|&&v| v == long[r]).count() > 1
        })
}

/// Sweep families: clips (here runs, by index) with the same start whose
/// own presses differ only in timing ([`same_sweep`], joined through each
/// other). Families of two or more, members in order, families by their
/// first member. Nothing is read from the clips' names.
pub(crate) fn sweep_families(starts: &[String], shapes: &[Vec<u64>]) -> Vec<Vec<usize>> {
    let mut by_start: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (k, s) in starts.iter().enumerate() {
        if !shapes[k].is_empty() {
            by_start.entry(s.as_str()).or_default().push(k);
        }
    }
    let mut root: Vec<usize> = (0..starts.len()).collect();
    fn find(root: &mut [usize], k: usize) -> usize {
        let mut r = k;
        while root[r] != r {
            r = root[r];
        }
        root[k] = r;
        r
    }
    for ks in by_start.values() {
        for (x, &i) in ks.iter().enumerate() {
            for &j in &ks[x + 1..] {
                if same_sweep(&shapes[i], &shapes[j]) {
                    let (ri, rj) = (find(&mut root, i), find(&mut root, j));
                    root[ri.max(rj)] = ri.min(rj);
                }
            }
        }
    }
    let mut fams: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for ks in by_start.values() {
        for &k in ks {
            let r = find(&mut root, k);
            fams.entry(r).or_default().push(k);
        }
    }
    let mut out: Vec<Vec<usize>> = fams.into_values().filter(|f| f.len() > 1).collect();
    for f in &mut out {
        f.sort_unstable();
    }
    out.sort();
    out
}

/// "Keep the members that differ", inside one family: `feats[m]` are
/// member m's value features, `kept` the members already kept. Features
/// every member shows part none of them and are left out. Then, while a
/// member shows a feature no kept member shows, the one showing the most
/// such is kept (ties: the cheaper, then the first). Returns the members
/// added, in order, each with the features that decided it.
pub(crate) fn keep_differing(
    feats: &[BTreeSet<usize>],
    cost: &[u64],
    kept: &BTreeSet<usize>,
) -> Vec<(usize, Vec<usize>)> {
    let Some(first) = feats.first() else {
        return Vec::new();
    };
    let common: BTreeSet<usize> = feats[1..].iter().fold(first.clone(), |acc, f| {
        acc.intersection(f).copied().collect()
    });
    let mut held: BTreeSet<usize> = kept
        .iter()
        .flat_map(|&m| feats[m].iter().copied())
        .filter(|v| !common.contains(v))
        .collect();
    let mut taken = kept.clone();
    let mut added = Vec::new();
    loop {
        let mut best: Option<(usize, u64, usize, Vec<usize>)> = None;
        for m in 0..feats.len() {
            if taken.contains(&m) {
                continue;
            }
            let new: Vec<usize> = feats[m]
                .iter()
                .copied()
                .filter(|v| !common.contains(v) && !held.contains(v))
                .collect();
            if new.is_empty() {
                continue;
            }
            let better = match &best {
                None => true,
                Some((n, c, _, _)) => new.len() > *n || (new.len() == *n && cost[m] < *c),
            };
            if better {
                best = Some((new.len(), cost[m], m, new));
            }
        }
        let Some((_, _, m, new)) = best else { break };
        held.extend(new.iter().copied());
        taken.insert(m);
        added.push((m, new));
    }
    added
}

/// The code a reduce reads its features through: the NES code map, or the
/// two Z80s' maps of a romset, their routines in one list by address
/// (`cpu << 16 | pc`).
enum Reader {
    Nes(CodeMap),
    Arcade {
        maps: Vec<crate::code_z80::CodeMap>,
        routines: Vec<crate::code::Routine>,
    },
}

impl Reader {
    fn arcade(set: &gp_arcade::Romset, levels: &gp_arcade::levels::Levels) -> Reader {
        let mut routines = Vec::new();
        let mut maps = Vec::new();
        for c in 0..2 {
            let m = crate::code_z80::CodeMap::of(set, c, Some(&levels.cpus[c]));
            let end = crate::code_z80::program(set, c).1;
            let entries: Vec<(u16, &str)> = m.routines.iter().map(|(a, w)| (*a, *w)).collect();
            for (k, (at, why)) in entries.iter().enumerate() {
                let next = entries.get(k + 1).map_or(end, |(n, _)| u32::from(*n));
                routines.push(crate::code::Routine {
                    name: z80_hex(z80_va(c, *at)),
                    at: z80_va(c, *at),
                    size: next - u32::from(*at),
                    element: (*why).to_owned(),
                });
            }
            maps.push(m);
        }
        Reader::Arcade { maps, routines }
    }

    fn routines(&self) -> &[crate::code::Routine] {
        match self {
            Reader::Nes(m) => &m.routines,
            Reader::Arcade { routines, .. } => routines,
        }
    }

    fn routine_of(&self, v: Va) -> Option<usize> {
        match self {
            Reader::Nes(m) => m.routine_of(v),
            Reader::Arcade { routines, .. } => {
                let i = routines.partition_point(|r| r.at <= v).checked_sub(1)?;
                let r = &routines[i];
                (v < r.at + r.size).then_some(i)
            }
        }
    }

    /// Whether the instruction at `v` is a conditional branch.
    fn is_branch(&self, v: Va) -> bool {
        match self {
            Reader::Nes(m) => m.insn_at(v).is_some_and(|i| i.mode == Mode::Rel),
            Reader::Arcade { maps, .. } => {
                let (c, pc) = z80_of(v);
                maps[c].insns.get(&pc).is_some_and(|i| i.flow.conditional())
            }
        }
    }

    /// Routine `i`'s conditional branches, in order.
    fn branches(&self, i: usize) -> Vec<Va> {
        match self {
            Reader::Nes(m) => m
                .body(i)
                .into_iter()
                .filter(|(_, ins)| ins.is_some_and(|x| x.mode == Mode::Rel))
                .map(|(p, _)| p)
                .collect(),
            Reader::Arcade { maps, routines } => {
                let r = &routines[i];
                let (c, pc) = z80_of(r.at);
                let end = u32::from(pc) + r.size;
                maps[c]
                    .insns
                    .range(pc..)
                    .take_while(|(p, _)| u32::from(**p) < end)
                    .filter(|(_, x)| x.flow.conditional())
                    .map(|(p, _)| z80_va(c, *p))
                    .collect()
            }
        }
    }

    fn hex(&self, v: Va) -> String {
        match self {
            Reader::Nes(_) => va_hex(v),
            Reader::Arcade { .. } => z80_hex(v),
        }
    }
}

fn z80_va(cpu: usize, pc: u16) -> Va {
    ((cpu as u32) << 16) | u32::from(pc)
}

fn z80_of(v: Va) -> (usize, u16) {
    ((v >> 16) as usize, (v & 0xffff) as u16)
}

fn z80_hex(v: Va) -> String {
    let (c, pc) = z80_of(v);
    format!("{}:{pc:04X}", crate::code_z80::CPUS[c])
}

/// One stretch of a run on one stage: its stage value, whether it ran
/// anything, and each instruction run with how often a branch there was
/// taken and fell.
struct Seg {
    value: u8,
    empty: bool,
    stats: BTreeMap<Va, (u64, u64)>,
}

/// A romset's runs counted on gp-arcade, each cut by the stage byte's value
/// as each frame begins (every frame one stage without a byte): per run, a
/// stretch per stage value with its levels, and all the runs' levels merged
/// for the code maps.
#[allow(clippy::type_complexity)]
fn count_arcade(
    set: &gp_arcade::Romset,
    inputs: &[crate::reach::ClipInput],
    runs: &[(String, u64, u64, usize)],
    stage: Option<u16>,
) -> Result<(
    Vec<Vec<(u8, gp_arcade::levels::Levels)>>,
    gp_arcade::levels::Levels,
)> {
    use gp_arcade::levels::{Levels, Watcher};
    let order: Vec<usize> = (0..runs.len()).collect();
    let done = crate::pool::map(
        &order,
        || (),
        |_, _, &k| -> std::result::Result<Vec<(u8, Levels)>, String> {
            let (_, first, last, i) = &runs[k];
            let c = &inputs[*i];
            let mut t = gp_arcade::Tapper::power_on(set);
            let mut by: BTreeMap<u8, Watcher> = BTreeMap::new();
            for (n, f) in c.inputs.iter().enumerate() {
                let frame = n as u64 + 1;
                if frame > *last {
                    break;
                }
                let bits = f.buttons_word();
                if bits > 0xff || f.reset {
                    return Err(format!(
                        "clip {}, frame {frame}: an arcade input frame holds bits 0 to 7 and no reset",
                        c.id
                    ));
                }
                let ctl = gp_arcade::Controls::from_bits(bits);
                if frame < *first {
                    t.step(ctl);
                } else {
                    let v = stage.map_or(0, |a| t.peek(a));
                    by.entry(v)
                        .or_insert_with(|| Watcher::new(false))
                        .step_quiet(&mut t, ctl);
                }
            }
            Ok(by.into_iter().map(|(v, w)| (v, w.finish())).collect())
        },
        &mut |_| {},
    );
    let mut all = Levels::default();
    let mut out = Vec::new();
    for r in done {
        let segs = r.map_err(anyhow::Error::msg)?;
        for (_, l) in &segs {
            crate::code_z80::merge(&mut all, l.clone());
        }
        out.push(segs);
    }
    Ok((out, all))
}

/// What a stretch of arcade levels ran: every instruction start, and each
/// conditional's edges as taken or fallen, by how often.
fn arcade_stats(levels: &gp_arcade::levels::Levels) -> BTreeMap<Va, (u64, u64)> {
    use gp_arcade::levels::EdgeKind;
    let mut out = BTreeMap::new();
    for (c, lv) in levels.cpus.iter().enumerate() {
        for pc in lv.starts.keys() {
            out.insert(z80_va(c, *pc), (0, 0));
        }
        for ((from, _, kind), seen) in &lv.edges {
            let e = out.entry(z80_va(c, *from)).or_insert((0, 0));
            match kind {
                EdgeKind::Fall => e.1 += seen.count,
                EdgeKind::Jump | EdgeKind::Loop | EdgeKind::Call | EdgeKind::Return => {
                    e.0 += seen.count;
                }
                _ => {}
            }
        }
    }
    out
}

/// The clips of a game or template folder: its corpus plan when
/// corpus.toml is one, else its clips.toml.
fn clips_file(t: &Path) -> Result<PathBuf> {
    if crate::plan::load(t)?.is_some() {
        return Ok(t.join(crate::corpus_score::FILE));
    }
    Ok(t.join(cs::FILE))
}

pub(crate) fn run(a: &ReduceArgs, paths: &Paths) -> Result<Outcome> {
    use std::fmt::Write as _;
    if let Some(w) = &a.write
        && let Some(repo) = gp_evidence::paths::enclosing_git_work_tree(w)
    {
        bail!(
            "--write {}: inside the git work tree {}; the plan stays outside the repo (under $GP_HOME/work)",
            w.display(),
            repo.display()
        );
    }
    let t0 = Instant::now();
    let t = &a.template;
    let clips_file = clips_file(t)?;
    let set = cs::read(&clips_file)?;
    let inputs = crate::reach::clip_inputs(std::slice::from_ref(&clips_file), paths)?;
    if inputs.len() != set.clips.len() {
        bail!("{}: clips and runs differ in number", clips_file.display());
    }
    let rom_path = match &a.rom {
        Some(r) => r.clone(),
        None => {
            let tt: toml::Value = toml::from_str(
                &fs::read_to_string(t.join("template.toml"))
                    .with_context(|| format!("reading {}", t.join("template.toml").display()))?,
            )?;
            crate::model_static::template_rom(t, &tt, paths)?
        }
    };
    let rom = fs::read(&rom_path).with_context(|| format!("reading {}", rom_path.display()))?;
    let rom_sha = crate::template::sha256_hex(&rom);
    let arcade = crate::original::Platform::of_file(&rom) == crate::original::Platform::Arcade;
    if arcade && a.values {
        bail!(
            "--values reads the 6502's compare classes (gp_events::profile); a romset's Z80 runs have no compare log yet, so give no --values on {}",
            rom_path.display()
        );
    }
    let sb = stage_byte(t, a.stage_byte.as_deref(), arcade)?;

    // Each clip's own window, and the runs: one for clips that are the
    // same run.
    let mut clips = Vec::new();
    let mut runs: Vec<(String, u64, u64, usize)> = Vec::new();
    let mut run_of: BTreeMap<(String, u64, u64), usize> = BTreeMap::new();
    for (i, (r, inp)) in set.clips.iter().zip(&inputs).enumerate() {
        let n = inp.inputs.len() as u64;
        let first = n.saturating_sub(r.frames) + 1;
        let key = (inp.key()?, first, n);
        let run = *run_of.entry(key.clone()).or_insert_with(|| {
            runs.push((key.0.clone(), first, n, i));
            runs.len() - 1
        });
        clips.push(Clip {
            id: r.id.clone(),
            family: r
                .id
                .rsplit_once('.')
                .map_or(r.id.clone(), |(f, _)| f.to_owned()),
            frames: r.frames,
            total: n,
            rep: runs[run].3,
            run,
        });
    }
    let store = if arcade {
        None
    } else {
        event_store(&a.store, a.no_store, paths)
    };
    let mut counted_now = 0;
    // NES: the 6502 segments, kept for the values.
    let mut segs: Vec<Vec<Segment>> = Vec::new();
    let (reader, run_segs): (Reader, Vec<Vec<Seg>>) = if arcade {
        let set = gp_arcade::Romset::from_zip(&rom)?;
        let (by_run, all) = count_arcade(&set, &inputs, &runs, sb.addr)?;
        counted_now = runs.len();
        let reader = Reader::arcade(&set, &all);
        let run_segs = by_run
            .iter()
            .map(|rs| {
                rs.iter()
                    .map(|(v, l)| Seg {
                        value: *v,
                        empty: l.cpus.iter().all(|c| c.starts.is_empty()),
                        stats: arcade_stats(l),
                    })
                    .collect()
            })
            .collect();
        (reader, run_segs)
    } else {
        let addr = sb.addr.context("an NES stage byte")?;
        let order: Vec<usize> = (0..runs.len()).collect();
        let done = crate::pool::map(
            &order,
            || (),
            |_, _, &k| -> std::result::Result<(Vec<Segment>, bool), String> {
                let (key, first, last, i) = &runs[k];
                if let Some(s) = store
                    .as_ref()
                    .and_then(|s| s.segments(&rom_sha, key, *first, *last, addr))
                {
                    return Ok((s, false));
                }
                let c = &inputs[*i];
                let (nes, at) = crate::code::console_before(&paths.home, &rom, c, *first)
                    .map_err(|e| format!("clip {}: {e:#}", c.id))?;
                let s = run_segments_on(nes, at, &c.inputs, *first, *last, addr)
                    .map_err(|e| format!("clip {}: {e}", c.id))?;
                if let Some(st) = &store {
                    st.put_segments(&rom_sha, key, *first, *last, addr, &s)
                        .map_err(|e| e.to_string())?;
                }
                Ok((s, true))
            },
            &mut |_| {},
        );
        for r in done {
            let (s, fresh) = r.map_err(anyhow::Error::msg)?;
            counted_now += usize::from(fresh);
            segs.push(s);
        }
        let profiles: Vec<&gp_events::profile::Profile> =
            segs.iter().flatten().map(|s| &s.profile).collect();
        let map = CodeMap::of(&rom, &profiles)?;
        let run_segs = segs
            .iter()
            .map(|rs| {
                rs.iter()
                    .map(|s| Seg {
                        value: s.value,
                        empty: s.profile.insns.is_empty(),
                        stats: map
                            .by_address(&s.profile)
                            .into_iter()
                            .map(|(v, st)| (v, (st.taken, st.fell)))
                            .collect(),
                    })
                    .collect()
            })
            .collect();
        (Reader::Nes(map), run_segs)
    };
    let routines = reader.routines();

    // Features of each run: by stage (q) and not (p).
    let mut qn = Names::default();
    let mut pn = Names::default();
    let mut vn = Names::default();
    // The branch and routine of each value feature.
    let mut v_at: Vec<(Va, usize)> = Vec::new();
    let mut run_q: Vec<Vec<usize>> = Vec::new();
    let mut run_p: Vec<Vec<usize>> = Vec::new();
    let mut run_v: Vec<BTreeSet<usize>> = Vec::new();
    let mut run_stages: Vec<BTreeSet<u8>> = Vec::new();
    // By stage: each branch's ways over the corpus, and the routines run.
    let mut ways: BTreeMap<u8, BTreeMap<Va, (u64, u64)>> = BTreeMap::new();
    let mut ran_in: BTreeMap<u8, BTreeSet<usize>> = BTreeMap::new();
    for (k, rs) in run_segs.iter().enumerate() {
        let (mut q, mut p) = (BTreeSet::new(), BTreeSet::new());
        let vs = match &reader {
            Reader::Nes(map) if a.values => near_classes(map, &segs[k], &mut vn, &mut v_at),
            _ => BTreeSet::new(),
        };
        let mut st = BTreeSet::new();
        for s in rs {
            if s.empty {
                continue;
            }
            st.insert(s.value);
            let mut ran = BTreeSet::new();
            for (va, &(taken, fell)) in &s.stats {
                let ri = reader.routine_of(*va);
                let (name, elem) = match ri {
                    Some(i) => (routines[i].name.as_str(), routines[i].element.as_str()),
                    None => ("outside", "outside"),
                };
                if let Some(i) = ri {
                    ran.insert(i);
                    ran_in.entry(s.value).or_default().insert(i);
                }
                let tag = if elem == "system" && !a.stage_system {
                    String::new()
                } else {
                    format!("@{}", s.value)
                };
                if !reader.is_branch(*va) {
                    continue;
                }
                let e = ways.entry(s.value).or_default().entry(*va).or_default();
                e.0 += taken;
                e.1 += fell;
                let at = reader.hex(*va);
                for (n, w) in [(taken, "T"), (fell, "F")] {
                    if n > 0 {
                        let f = format!("B {at} {w} {name}");
                        q.insert(qn.id(format!("{f}{tag}")));
                        p.insert(pn.id(f));
                    }
                }
            }
            for i in ran {
                let r = &routines[i];
                let tag = if r.element == "system" && !a.stage_system {
                    String::new()
                } else {
                    format!("@{}", s.value)
                };
                let f = format!("R {}", r.name);
                q.insert(qn.id(format!("{f}{tag}")));
                p.insert(pn.id(f));
            }
        }
        run_q.push(q.into_iter().collect());
        run_p.push(p.into_iter().collect());
        run_v.push(vs);
        run_stages.push(st);
    }
    let nq = qn.list.len();
    let n_feats = nq;
    let need_q: Vec<usize> = (0..n_feats)
        .map(|f| {
            if a.twice && qn.list[f].contains('@') {
                2
            } else {
                1
            }
        })
        .collect();
    let cost: Vec<u64> = runs.iter().map(|r| clips[r.3].frames).collect();

    // Guards: the first clip each fix commit names, and --keep.
    let ids: Vec<String> = clips.iter().map(|c| c.id.clone()).collect();
    for p in &a.keep {
        if !ids.iter().any(|id| hit(p, id)) {
            bail!(
                "--keep {p}: no clip of {} has that ID",
                clips_file.display()
            );
        }
    }
    let mut why_kept: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    let mut forced: BTreeSet<usize> = BTreeSet::new();
    let commits = if a.no_guards {
        Vec::new()
    } else {
        fix_commits(t, &ids)
    };
    let mut guards = 0;
    for (sha, subject, named) in &commits {
        let c = &clips[named[0]];
        forced.insert(c.run);
        guards += 1;
        why_kept.entry(c.run).or_default().push(format!(
            "guard: {} {} names {}",
            &sha[..8.min(sha.len())],
            subject,
            c.id
        ));
    }
    let n_guard_runs = forced.len();
    for (i, c) in clips.iter().enumerate() {
        if a.keep.iter().any(|p| hit(p, &c.id)) {
            forced.insert(c.run);
            why_kept
                .entry(c.run)
                .or_default()
                .push(format!("--keep {}", clips[i].id));
        }
    }
    // The covers.
    let none = BTreeSet::new();
    let plain_n = pn.list.len();
    let plain = cover(&run_p, &vec![1; plain_n], &cost, &none);
    let by_stage = cover(&run_q, &vec![1; nq], &cost, &none);
    let mut kept = cover(&run_q, &need_q, &cost, &forced);
    // Held by one run only: every cover needs that clip.
    let mut holders: Vec<Vec<usize>> = vec![Vec::new(); n_feats];
    for (k, fs) in run_q.iter().enumerate() {
        for &f in fs {
            holders[f].push(k);
        }
    }
    let mut sole: BTreeMap<usize, usize> = BTreeMap::new();
    for h in &holders {
        if let [k] = h.as_slice() {
            *sole.entry(*k).or_default() += 1;
        }
    }
    // Why each kept clip: what it alone holds among the kept.
    let mut held = vec![0usize; n_feats];
    for &k in &kept {
        for &f in &run_q[k] {
            held[f] += 1;
        }
    }
    let feat_name = |f: usize| -> String { qn.list[f].clone() };
    for &k in &kept {
        if let Some(n) = sole.get(&k) {
            why_kept
                .entry(k)
                .or_default()
                .push(format!("any cover: the only clip taking {n} features"));
        }
        let only: Vec<usize> = run_q[k].iter().copied().filter(|&f| held[f] == 1).collect();
        if !only.is_empty() {
            let ex: Vec<String> = only.iter().take(3).map(|&f| feat_name(f)).collect();
            why_kept.entry(k).or_default().push(format!(
                "cover: {} features no other kept clip takes ({}{})",
                only.len(),
                ex.join("; "),
                if only.len() > 3 { "; ..." } else { "" }
            ));
        }
        if why_kept.get(&k).is_none_or(Vec::is_empty) {
            why_kept
                .entry(k)
                .or_default()
                .push("cover: its features held twice only with it (--twice)".into());
        }
    }

    let kept_by_cover = kept.len();

    // Values: inside each sweep family only, the members whose
    // near-boundary compare classes differ from the kept members'.
    let mut fam_rows: Vec<Value> = Vec::new();
    let mut fam_text = String::new();
    if a.values
        && let Reader::Nes(map) = &reader
        && let Some(addr) = sb.addr
    {
        let is_rel = |v: Va| map.insn_at(v).is_some_and(|i| i.mode == Mode::Rel);
        let starts: Vec<String> = runs
            .iter()
            .map(|(_, first, _, i)| start_key(&inputs[*i], *first))
            .collect();
        let shapes: Vec<Vec<u64>> = runs
            .iter()
            .map(|(_, first, last, i)| {
                let f = usize::try_from(*first - 1).unwrap_or(usize::MAX);
                let l = usize::try_from(*last).unwrap_or(usize::MAX);
                shape(&inputs[*i].inputs[f.min(l)..l])
            })
            .collect();
        let families = sweep_families(&starts, &shapes);
        // Each member counted again over the same frames: from the common
        // start to a second after the last frame where the members'
        // presses differ (within the shortest member's length). Before
        // that frame they are one run; long after it a member differs
        // by what the enemies did, not by the presses.
        let mut wins: Vec<(usize, u64, u64)> = Vec::new();
        for fam in &families {
            let len = fam
                .iter()
                .map(|&k| runs[k].2 + 1 - runs[k].1)
                .min()
                .unwrap_or(0);
            let own = |k: usize, f: u64| {
                let (_, first, _, i) = &runs[k];
                let g = &inputs[*i].inputs[usize::try_from(first - 1 + f).unwrap_or(usize::MAX)];
                (g.pads, g.reset)
            };
            let Some(last_diff) = (0..len)
                .rev()
                .find(|&f| fam.iter().any(|&k| own(k, f) != own(fam[0], f)))
            else {
                continue;
            };
            let n = len.min(last_diff + 1 + AFTER_PRESSES);
            for &k in fam {
                wins.push((k, runs[k].1, runs[k].1 + n - 1));
            }
        }
        let order: Vec<usize> = (0..wins.len()).collect();
        let done = crate::pool::map(
            &order,
            || (),
            |_, _, &w| -> std::result::Result<(Vec<Segment>, bool), String> {
                let (k, first, last) = wins[w];
                let (key, _, _, i) = &runs[k];
                if let Some(s) = store
                    .as_ref()
                    .and_then(|s| s.segments(&rom_sha, key, first, last, addr))
                {
                    return Ok((s, false));
                }
                let c = &inputs[*i];
                let (nes, at) = crate::code::console_before(&paths.home, &rom, c, first)
                    .map_err(|e| format!("clip {}: {e:#}", c.id))?;
                let s = run_segments_on(nes, at, &c.inputs, first, last, addr)
                    .map_err(|e| format!("clip {}: {e}", c.id))?;
                if let Some(st) = &store {
                    st.put_segments(&rom_sha, key, first, last, addr, &s)
                        .map_err(|e| e.to_string())?;
                }
                Ok((s, true))
            },
            &mut |_| {},
        );
        let mut win: BTreeMap<usize, Window> = BTreeMap::new();
        for (w, r) in done.into_iter().enumerate() {
            let (s, fresh) = r.map_err(anyhow::Error::msg)?;
            counted_now += usize::from(fresh);
            let mut x = Window {
                v: near_classes(map, &s, &mut vn, &mut v_at),
                ..Window::default()
            };
            for sg in &s {
                for (va, st) in &map.by_address(&sg.profile) {
                    let Some(ri) = map.routine_of(*va) else {
                        continue;
                    };
                    x.ran.insert(ri);
                    if is_rel(*va) {
                        let e = x.ways.entry(*va).or_default();
                        e.0 += st.taken;
                        e.1 += st.fell;
                    }
                }
            }
            win.insert(wins[w].0, x);
        }
        for fam in &families {
            let empty = Window::default();
            let ws: Vec<&Window> = fam.iter().map(|&k| win.get(&k).unwrap_or(&empty)).collect();
            let decides = |v: usize| -> bool {
                let (at, ri) = v_at[v];
                let r = &map.routines[ri];
                let in_r = |x: &Window| -> BTreeSet<(Va, bool, bool)> {
                    x.ways
                        .range(r.at..r.at + r.size)
                        .map(|(&p, &(t, f))| (p, t > 0, f > 0))
                        .collect()
                };
                let first = in_r(ws[0]);
                ws.iter().all(|x| x.ran.contains(&ri) && in_r(x) == first)
                    && ws
                        .iter()
                        .all(|x| x.ways.get(&at).is_none_or(|&(t, f)| t.min(f) <= EVENT))
            };
            let feats: Vec<BTreeSet<usize>> = ws
                .iter()
                .map(|x| x.v.iter().copied().filter(|&v| decides(v)).collect())
                .collect();
            let fam_cost: Vec<u64> = fam.iter().map(|&k| cost[k]).collect();
            let fam_kept: BTreeSet<usize> =
                (0..fam.len()).filter(|&m| kept.contains(&fam[m])).collect();
            let added = keep_differing(&feats, &fam_cost, &fam_kept);
            let ids: Vec<&str> = fam.iter().map(|&k| clips[runs[k].3].id.as_str()).collect();
            let _ = writeln!(
                fam_text,
                "  {} ({} members, {} kept by the cover){}",
                ids.join(" "),
                fam.len(),
                fam_kept.len(),
                if added.is_empty() { ", none added" } else { "" }
            );
            let mut adds = Vec::new();
            for (m, deciding) in &added {
                let k = fam[*m];
                kept.insert(k);
                let ex: Vec<String> = deciding
                    .iter()
                    .take(3)
                    .map(|&v| vn.list[v].clone())
                    .collect();
                let why = format!(
                    "values: in the sweep of {}, {} class{} no kept member shows ({}{})",
                    ids[0],
                    deciding.len(),
                    if deciding.len() == 1 { "" } else { "es" },
                    ex.join("; "),
                    if deciding.len() > 3 { "; ..." } else { "" }
                );
                let _ = writeln!(fam_text, "    + {}: {}", ids[*m], why);
                why_kept.entry(k).or_default().push(why);
                adds.push(json!({"id": ids[*m], "classes": deciding.iter().map(|&v| vn.list[v].clone()).collect::<Vec<_>>()}));
            }
            fam_rows.push(json!({
                "members": ids,
                "kept_by_cover": fam_kept.iter().map(|&m| ids[m]).collect::<Vec<_>>(),
                "added": adds,
            }));
        }
    }

    // Stages of each clip, as names.
    let stage_text = |k: usize| -> String {
        let s: Vec<String> = run_stages[k].iter().map(|v| sb.name(*v)).collect();
        if s.is_empty() {
            "-".into()
        } else {
            s.join("/")
        }
    };
    let kept_clips: Vec<usize> = (0..clips.len())
        .filter(|&i| clips[i].rep == i && kept.contains(&clips[i].run))
        .collect();

    // Untaken branch ways by stage, in the routines that ran on it; the
    // code map's system and unknown routines and code outside it are
    // listed in the plan but not counted.
    let mut untaken: Vec<Value> = Vec::new();
    let mut untaken_count: BTreeMap<u8, (usize, BTreeSet<String>)> = BTreeMap::new();
    for (v, rs) in &ran_in {
        let w = ways.get(v);
        for &i in rs {
            let r = &routines[i];
            for p in reader.branches(i) {
                let (tk, fl) = w.and_then(|w| w.get(&p)).copied().unwrap_or((0, 0));
                let missing = match (tk > 0, fl > 0) {
                    (true, true) => continue,
                    (true, false) => "fell",
                    (false, true) => "taken",
                    (false, false) => "not run",
                };
                let level = r.element != "system" && r.element != "unknown";
                if level {
                    let e = untaken_count.entry(*v).or_default();
                    e.0 += 1;
                    e.1.insert(r.name.clone());
                }
                untaken.push(json!({
                    "stage": sb.name(*v), "at": reader.hex(p), "routine": r.name,
                    "element": r.element, "missing": missing, "counted": level,
                }));
            }
        }
    }

    // Duplicates.
    let mut dups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (i, c) in clips.iter().enumerate() {
        if c.rep != i {
            dups.entry(c.rep).or_default().push(i);
        }
    }

    // The text.
    let mut text = String::new();
    let all_frames: u64 = clips.iter().map(|c| c.frames).sum();
    let all_total: u64 = clips.iter().map(|c| c.total).sum();
    let kept_frames: u64 = kept_clips.iter().map(|&i| clips[i].frames).sum();
    let kept_total: u64 = kept_clips.iter().map(|&i| clips[i].total).sum();
    let _ = writeln!(
        text,
        "{}: {} clips, {} runs ({} counted now, {:.0} s), stage byte {}; {} features by stage ({} not), {} value features",
        t.display(),
        clips.len(),
        runs.len(),
        counted_now,
        t0.elapsed().as_secs_f64(),
        sb.text(),
        nq,
        plain_n,
        vn.list.len()
    );
    let sole_runs = sole.len();
    let _ = writeln!(text, "\n{:<52} {:>6}", "rule", "clips");
    let _ = writeln!(text, "{:<52} {:>6}", "all", clips.len());
    let _ = writeln!(text, "{:<52} {:>6}", "less exact duplicates", runs.len());
    let _ = writeln!(
        text,
        "{:<52} {:>6}",
        "branch-way cover, not by stage",
        plain.len()
    );
    let _ = writeln!(
        text,
        "{:<52} {:>6}",
        format!(
            "by stage ({}), {sole_runs} of them any cover needs",
            sb.text()
        ),
        by_stage.len()
    );
    let mut rule = String::from("kept: by stage");
    if !commits.is_empty() {
        let _ = write!(
            rule,
            ", {guards} fix commits' first clips ({n_guard_runs} clips)"
        );
    }
    if !a.keep.is_empty() {
        rule.push_str(", --keep");
    }
    if a.twice {
        rule.push_str(", twice");
    }
    let _ = writeln!(text, "{:<52} {:>6}", rule, kept_by_cover);
    if a.values {
        rule.push_str(", values in sweep families");
        let _ = writeln!(
            text,
            "{:<52} {:>6}",
            format!(
                "+ sweep members that differ near a boundary ({})",
                kept.len() - kept_by_cover
            ),
            kept.len()
        );
    }
    let _ = writeln!(
        text,
        "\nkept frames: {kept_frames} of {all_frames} own ({:.0}%), {kept_total} of {all_total} from power-on",
        100.0 * kept_frames as f64 / all_frames.max(1) as f64
    );
    // By stage and family.
    let mut by_st: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut by_fam: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for (i, c) in clips.iter().enumerate() {
        let k = c.run;
        let keep = c.rep == i && kept.contains(&k);
        let e = by_st.entry(stage_text(k)).or_default();
        e.1 += 1;
        e.0 += usize::from(keep);
        let e = by_fam.entry(c.family.as_str()).or_default();
        e.1 += 1;
        e.0 += usize::from(keep);
    }
    let _ = writeln!(text, "\nkept by stage (kept / had):");
    for (s, (k, h)) in &by_st {
        let _ = writeln!(text, "  {s:<24} {k:>4} / {h}");
    }
    let _ = writeln!(text, "\nkept by family (kept / had):");
    let mut fams: Vec<(&&str, &(usize, usize))> = by_fam.iter().collect();
    fams.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(b.1.1.cmp(&a.1.1)).then(a.0.cmp(b.0)));
    for (f, (k, h)) in fams {
        let _ = writeln!(text, "  {f:<32} {k:>4} / {h}");
    }
    let mut needed: Vec<&str> = sole.keys().map(|&k| clips[runs[k].3].id.as_str()).collect();
    needed.sort_unstable();
    let _ = writeln!(
        text,
        "\nclips any cover needs ({}): {}",
        needed.len(),
        needed.join(" ")
    );
    let _ = writeln!(
        text,
        "\nexact duplicates ({} clips):",
        dups.values().map(Vec::len).sum::<usize>()
    );
    for (r, ds) in &dups {
        let mut all = vec![clips[*r].id.as_str()];
        all.extend(ds.iter().map(|&d| clips[d].id.as_str()));
        let _ = writeln!(text, "  {}", all.join(" = "));
    }
    let _ = writeln!(
        text,
        "\nuntaken branch ways by stage (one way or neither, in routines run on that stage; not system or unknown code):"
    );
    for (v, (n, rs)) in &untaken_count {
        let _ = writeln!(
            text,
            "  {:<10} {n:>4} in {} routines",
            sb.name(*v),
            rs.len()
        );
    }
    if a.values {
        let _ = writeln!(
            text,
            "\nsweep families ({}: the same start, presses differing only in timing), and the members the values add (a class at an event branch, in a routine all members run the same ways):",
            fam_rows.len()
        );
        text.push_str(&fam_text);
    }

    // The plan.
    let short = |s: &str| s[..12.min(s.len())].to_owned();
    let mut keep_rows = Vec::new();
    let mut drop_rows = Vec::new();
    for (i, c) in clips.iter().enumerate() {
        let k = c.run;
        let row_stage = stage_text(k);
        if c.rep != i {
            drop_rows.push(json!({"id": c.id, "family": c.family, "stages": row_stage,
                "why": format!("exact duplicate of {}", clips[c.rep].id)}));
        } else if kept.contains(&k) {
            keep_rows.push(json!({"id": c.id, "family": c.family, "stages": row_stage,
                "frames": c.frames, "features": run_q[k].len(), "values": run_v[k].len(), "why": why_kept.get(&k).cloned().unwrap_or_default()}));
        } else {
            drop_rows.push(json!({"id": c.id, "family": c.family, "stages": row_stage,
                "frames": c.frames, "features": run_q[k].len(), "values": run_v[k].len(),
                "why": "every feature it takes is held by the kept clips"}));
        }
    }
    let doc = json!({
        "plan": {
            "template": t.display().to_string(),
            "clips_blake3": blake3::hash(&fs::read(&clips_file)?).to_hex().to_string(),
            "rom_sha256": rom_sha,
            "stage_byte": sb.text(),
            "rules": rule,
            "clips": clips.len(),
            "runs": runs.len(),
            "cover_not_by_stage": plain.len(),
            "cover_by_stage": by_stage.len(),
            "any_cover_needs": needed,
            "kept": kept.len(),
            "kept_frames": kept_frames,
            "all_frames": all_frames,
            "kept_from_power_on": kept_total,
            "all_from_power_on": all_total,
            "guards": commits.iter().map(|(h, s, n)| json!({"commit": short(h), "subject": s, "first": clips[n[0]].id})).collect::<Vec<_>>(),
            "note": "A plan only: the clips are unchanged. Removing clips needs the user's approval.",
        },
        "keep": keep_rows,
        "drop": drop_rows,
        "untaken": untaken,
        "families": fam_rows,
    });
    let mut summary = text.trim_end().to_owned();
    if let Some(w) = &a.write {
        if let Some(d) = w.parent() {
            fs::create_dir_all(d)?;
        }
        let body = toml::to_string(&doc).context("the plan as TOML")?;
        fs::write(
            w,
            format!("# gp corpus reduce: a plan, not applied. The clips are unchanged.\n{body}"),
        )?;
        let _ = write!(summary, "\n\nplan written to {}", w.display());
    }
    Ok(Outcome::ok(doc, summary))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cover_keeps_guards_takes_the_cheapest_and_drops_the_spare() {
        // Features 0..4. Clip 0 holds all but is dear; clips 1 and 2
        // hold 0,1 and 2,3 cheaply; clip 3 holds 4 alone; clip 4 is a
        // guard holding 0.
        let feats = vec![
            vec![0, 1, 2, 3, 4],
            vec![0, 1],
            vec![2, 3],
            vec![4],
            vec![0],
        ];
        let cost = vec![10_000, 100, 100, 100, 50];
        let need = vec![1; 5];
        let forced: BTreeSet<usize> = [4].into();
        let kept = cover(&feats, &need, &cost, &forced);
        assert_eq!(kept, [1, 2, 3, 4].into());
        // Twice: feature 4 is held by two clips (0 and 3), so both stay;
        // with the dear clip in, the cheap ones it doubles stay too.
        let kept2 = cover(&feats, &[2; 5], &cost, &BTreeSet::new());
        assert!(kept2.contains(&0) && kept2.contains(&3));
        // The same input, the same choice.
        assert_eq!(cover(&feats, &need, &cost, &forced), kept);
    }

    #[test]
    fn a_z80_address_keeps_its_cpu() {
        assert_eq!(z80_of(z80_va(1, 0x0276)), (1, 0x0276));
        assert_eq!(z80_hex(z80_va(0, 0x5fac)), "main:5FAC");
        assert_eq!(z80_hex(z80_va(1, 0x0276)), "sound:0276");
    }

    /// Tapper's romset from $GP_ROM_DIR, idle for 200 frames: one stretch on
    /// one stage, the main CPU's reset routine run, and branches read both
    /// ways. Skips without the romset.
    #[test]
    fn a_romset_run_is_counted_by_z80_routine() {
        let Some(dir) = std::env::var_os("GP_ROM_DIR") else {
            return;
        };
        let Ok(zip) = fs::read(Path::new(&dir).join("arcade/tapper.zip")) else {
            return;
        };
        let set = gp_arcade::Romset::from_zip(&zip).expect("romset");
        let clip = crate::reach::ClipInput {
            set: String::new(),
            id: "idle".into(),
            power_on: gp_evidence::manifest::PowerOn::default(),
            inputs: vec![gp_evidence::input_log::InputFrame::default(); 200],
        };
        let runs = vec![(String::new(), 101, 200, 0)];
        let (by_run, all) = count_arcade(&set, &[clip], &runs, None).expect("counted");
        assert_eq!(by_run.len(), 1);
        assert_eq!(by_run[0].len(), 1);
        assert_eq!(by_run[0][0].1.watched, 100);
        let r = Reader::arcade(&set, &all);
        let stats = arcade_stats(&by_run[0][0].1);
        assert!(r.routines().iter().any(|x| x.name == "main:0000"));
        assert!(stats.keys().any(|v| r.is_branch(*v)));
        assert!(stats.keys().all(|v| r.routine_of(*v).is_some()));
    }

    #[test]
    fn a_sweep_family_is_the_same_start_and_presses_moved_in_time() {
        // Shapes: R right, J right+jump, L left, 0 nothing.
        let (r, j, l, z) = (1u64, 2, 3, 0);
        let starts: Vec<String> = ["s", "s", "s", "s", "s", "t", "s"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let shapes = vec![
            vec![j, r, z],    // 0: jump on the first frame, then right
            vec![r, j, r, z], // 1: right 4 frames, jump, right
            vec![r, j, r, z], // 2: right 8 frames, jump, right
            vec![l, z],       // 3: left: another sweep's
            vec![z],          // 4: nothing: not left held for no frames
            vec![r, j, r, z], // 5: the same presses from another start
            vec![l, z],       // 6: left again, longer
        ];
        let fams = sweep_families(&starts, &shapes);
        assert_eq!(fams, vec![vec![0, 1, 2], vec![3, 6]]);
        // The same input, the same families.
        assert_eq!(sweep_families(&starts, &shapes), fams);

        // Inside the family: 1 kept by the cover. Feature 9 every member
        // shows; 0 shows 5 and 7 that 1 doesn't, 2 shows only what 1 does.
        let feats: Vec<BTreeSet<usize>> = vec![[5, 7, 9].into(), [6, 9].into(), [6, 9].into()];
        let added = keep_differing(&feats, &[60, 64, 68], &[1].into());
        assert_eq!(added, vec![(0, vec![5, 7])]);
        // Nothing kept yet: the member with the most differing classes
        // first, then any still differing; 9 alone decides nothing.
        let added = keep_differing(&feats, &[60, 64, 68], &BTreeSet::new());
        assert_eq!(added, vec![(0, vec![5, 7]), (1, vec![6])]);
    }
}
