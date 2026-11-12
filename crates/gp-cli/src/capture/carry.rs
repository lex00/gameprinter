//! `gp capture carry`: what the game's clip starts can't carry into the
//! rebuilt game (`gp_interp::carry`). For every start of the game's clips,
//! and with `--every` for a sample of frames through the starts' presses
//! from power-on, it lists each loaded object the start can't carry and
//! why, groups them by cause, and counts the starts each gap alone holds
//! back: the starts that adding its mapping rows would open.
//!
//! Brought from game-printer's serve/carry.rs, whose tally and report are
//! kept. It read a capture run's starts and recordings; it reads a game's
//! package now ([`super::Inputs`]), so a start has no reason from when it
//! was made (`before`), the sample runs through the starts' presses files
//! where it ran through the run's recordings, and `--write` (into a capture
//! run's starts.json) waits for wave 7.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

use anyhow::Context as _;
use clap::Args;
use gp_interp::carry::{Cause, Gap};
use gp_nes::protocol::NesRuntime;
use gp_runtime::{Runtime, Value};
use serde::Serialize;
use serde_json::json;

#[derive(Args, Debug, Clone)]
pub struct CarryArgs {
    /// The game's package folder (games/donkey-kong). In game-printer this
    /// was a capture run.
    pub run: PathBuf,
    /// Also sample the starts' presses from power-on every this many
    /// polled frames in play with the player on the ground; 0 for the
    /// starts only.
    #[arg(long, default_value_t = 0)]
    pub every: u64,
    /// Write each start's reason now into a capture run's starts.json.
    /// Needs a capture run: wave 7.
    #[arg(long)]
    pub write: bool,
    /// The ROM file, instead of finding it in $GP_ROM_DIR by hash.
    #[arg(long)]
    pub rom: Option<PathBuf>,
    /// The model's lifted assets (default: $GP_HOME/work/assets/<model.source>).
    #[arg(long, value_name = "DIR")]
    pub assets: Option<PathBuf>,
    /// The game's built module, for a model with hooks (default:
    /// $GP_HOME/work/modules/GAME.wasm when there is one).
    #[arg(long)]
    pub module: Option<PathBuf>,
}

/// One place looked at: a start, or a sampled frame of a recording.
#[derive(Debug, Clone, Serialize)]
struct Point {
    /// The start's ID, or `<recording>@<frame>`.
    id: String,
    level: String,
    /// Why the start was refused when it was made, as the run has it.
    #[serde(skip_serializing_if = "Option::is_none")]
    before: Option<String>,
    /// Why it is refused now, or none.
    #[serde(skip_serializing_if = "Option::is_none")]
    cant: Option<String>,
    gaps: Vec<Gap>,
}

/// A gap by what fixes it: the entity (or ID), the cause and the field.
type Key = (String, Cause, String);

fn key(g: &Gap) -> Key {
    (g.what(), g.cause, g.field.clone().unwrap_or_default())
}

/// The counts for one set of points.
#[derive(Debug, Default, Serialize)]
struct Tally {
    points: usize,
    usable_before: usize,
    usable: usize,
    /// Points held back by loaded objects alone.
    by_objects: usize,
    /// Cause -> (objects, points with one).
    causes: BTreeMap<String, (usize, usize)>,
    /// What fixes it -> (points with it, points it alone holds back).
    fixes: Vec<(String, String, String, usize, usize)>,
    /// By kind: (points with one, points held back by that kind alone).
    kinds: Vec<(String, usize, usize)>,
}

fn tally(points: &[Point], with_before: bool) -> Tally {
    let mut t = Tally {
        points: points.len(),
        ..Tally::default()
    };
    let mut fixes: BTreeMap<Key, (usize, usize)> = BTreeMap::new();
    let mut kinds: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for p in points {
        if with_before && p.before.is_none() {
            t.usable_before += 1;
        }
        if p.cant.is_none() {
            t.usable += 1;
        }
        if p.gaps.is_empty() {
            continue;
        }
        t.by_objects += 1;
        let mut causes: BTreeSet<Cause> = BTreeSet::new();
        for g in &p.gaps {
            t.causes.entry(g.cause.words().to_owned()).or_default().0 += 1;
            causes.insert(g.cause);
        }
        for c in causes {
            t.causes.entry(c.words().to_owned()).or_default().1 += 1;
        }
        let keys: BTreeSet<Key> = p.gaps.iter().map(key).collect();
        let whats: BTreeSet<String> = p.gaps.iter().map(Gap::what).collect();
        let by_gaps = p.cant.as_deref().is_some_and(|c| c.contains("can't carry"));
        let lone = whats.len() == 1 && by_gaps;
        for w in whats {
            let e = kinds.entry(w).or_default();
            e.0 += 1;
            if lone {
                e.1 += 1;
            }
        }
        // Held back by nothing but loaded objects: fixing every key it has
        // opens it, and a lone key opens it alone.
        let only = keys.len() == 1 && by_gaps;
        for k in keys {
            let e = fixes.entry(k).or_default();
            e.0 += 1;
            if only {
                e.1 += 1;
            }
        }
    }
    let mut f: Vec<_> = fixes
        .into_iter()
        .map(|((what, cause, field), (n, alone))| (what, cause.words().to_owned(), field, n, alone))
        .collect();
    f.sort_by(|a, b| b.3.cmp(&a.3).then(b.4.cmp(&a.4)).then(a.cmp(b)));
    t.fixes = f;
    let mut k: Vec<_> = kinds.into_iter().map(|(w, (n, a))| (w, n, a)).collect();
    k.sort_by(|a, b| b.1.cmp(&a.1).then(a.cmp(b)));
    t.kinds = k;
    t
}

pub(crate) fn cli(paths: &gp_evidence::Paths, o: &CarryArgs) -> anyhow::Result<crate::Outcome> {
    if o.write {
        anyhow::bail!(
            "--write puts each start's reason into a capture run's starts.json, which gameprinter doesn't have until the capture runs come back (wave 7 of the port)"
        );
    }
    let inp = super::Inputs::of_game(paths, &o.run, o.rom.as_deref())?;
    let module = o.module.clone().or_else(|| {
        let name = o.run.file_name()?.to_string_lossy().into_owned();
        Some(
            paths
                .home
                .join("work")
                .join("modules")
                .join(format!("{name}.wasm")),
        )
        .filter(|p| p.is_file())
    });
    let (source, _) =
        crate::hosted::Source::open_dir(&o.run, o.assets.as_deref(), paths, module.as_deref())?;
    let spec = source.spec.clone();
    let mfile = o.run.join(gp_model::MODEL_FILE);
    let model = gp_model::Model::parse(
        &fs::read_to_string(&mfile).with_context(|| format!("reading {}", mfile.display()))?,
    )
    .with_context(|| mfile.display().to_string())?;
    let player = gp_interp::scenario::player(&model)
        .context("the model has no player, so nothing says where a start is")?;
    let mapping = &inp.mapping;
    let rom = inp.rom.clone();
    let load = |snap: &[u8]| -> anyhow::Result<NesRuntime> {
        let mut or = NesRuntime::load(&rom, gp_evidence::manifest::PowerOn::default())
            .map_err(|e| anyhow::anyhow!("the ROM: {e}"))?;
        or.restore(snap).map_err(anyhow::Error::msg)?;
        Ok(or)
    };
    let look = |or: &NesRuntime| -> anyhow::Result<(Option<String>, Vec<Gap>)> {
        let gaps = gp_interp::carry::gaps(or, &spec, mapping).map_err(anyhow::Error::msg)?;
        let cant = gp_interp::starts::cant_start_here(or, &spec, mapping, &player)
            .map_err(anyhow::Error::msg)?;
        Ok((cant, gaps))
    };
    let level_at = |or: &NesRuntime| -> String {
        gp_interp::translate::read_original(or, mapping)
            .ok()
            .and_then(|vals| {
                gp_interp::translate::level_of(or, mapping, &vals)
                    .ok()
                    .flatten()
            })
            .map(|(l, _)| l)
            .unwrap_or_default()
    };

    let mut starts: Vec<Point> = Vec::new();
    for s in &inp.starts {
        let or = load(&s.snapshot)?;
        let (cant, gaps) = look(&or)?;
        starts.push(Point {
            id: s.id.clone(),
            level: level_at(&or),
            before: None,
            cant,
            gaps,
        });
    }

    // A sample through each start's presses from power-on.
    let mut sampled: Vec<Point> = Vec::new();
    if o.every > 0 {
        let files: BTreeSet<&str> = inp.starts.iter().map(|s| s.presses_file.as_str()).collect();
        let cond = mapping
            .start
            .as_deref()
            .map(gp_runtime::select::Condition::parse)
            .transpose()
            .map_err(anyhow::Error::msg)?;
        let gi = mapping
            .fields
            .iter()
            .position(|f| f.path == format!("entities.{player}[0].on_ground"));
        for rec in files {
            let (n, spans) =
                crate::clipset::parse_presses(&fs::read_to_string(inp.dir.join(rec))?)?;
            let log = gp_evidence::InputLog::parse(&crate::clipset::log_of(n, &spans)?)?;
            let inputs: Vec<gp_runtime::InputFrame> = log
                .frames
                .iter()
                .map(|f| gp_runtime::InputFrame {
                    reset: f.reset,
                    ..gp_runtime::InputFrame::pads(f.pads)
                })
                .collect();
            let mut or = NesRuntime::load(&rom, gp_evidence::manifest::PowerOn::default())
                .map_err(|e| anyhow::anyhow!("the ROM: {e}"))?;
            let (mut started, mut polled_n) = (false, 0u64);
            for input in &inputs {
                let events = or
                    .step(std::slice::from_ref(input))
                    .map_err(anyhow::Error::msg)?;
                let polled = gp_interp::lockstep::polled(&events);
                let holds = |c: &gp_runtime::select::Condition| {
                    gp_interp::starts::holds(&or, c).unwrap_or(false)
                };
                if !started {
                    started = match &cond {
                        Some(c) => holds(c),
                        None => polled,
                    };
                    continue;
                }
                if !polled {
                    continue;
                }
                polled_n += 1;
                if !polled_n.is_multiple_of(o.every) || !cond.as_ref().is_none_or(holds) {
                    continue;
                }
                let vals = gp_interp::translate::read_original(&or, mapping)
                    .map_err(anyhow::Error::msg)?;
                if !gi.is_some_and(|i| matches!(vals[i], Value::Flag(true))) {
                    continue;
                }
                let level = gp_interp::translate::level_of(&or, mapping, &vals)
                    .ok()
                    .flatten()
                    .map(|(l, _)| l)
                    .unwrap_or_default();
                let (cant, gaps) = look(&or)?;
                sampled.push(Point {
                    id: format!("{rec}@{}", or.frame()),
                    level,
                    before: None,
                    cant,
                    gaps,
                });
            }
        }
    }

    let ts = tally(&starts, false);
    let tf = tally(&sampled, false);
    let mut lines = Vec::new();
    // A game's starts have no reason from when they were made, as a
    // capture run's had.
    lines.push(format!(
        "{} starts: {} usable now; {} held back by loaded objects the start can't carry",
        ts.points, ts.usable, ts.by_objects
    ));
    if o.every > 0 {
        lines.push(format!(
            "{} frames sampled every {} polled frames on the ground: {} usable; {} held back by loaded objects",
            tf.points, o.every, tf.usable, tf.by_objects
        ));
    }
    for (what, t) in [("starts", &ts), ("sampled frames", &tf)] {
        if t.points == 0 || t.causes.is_empty() {
            continue;
        }
        lines.push(String::new());
        lines.push(format!(
            "| Cause, in the {what} | Objects | {what} with one |"
        ));
        lines.push("|---|---|---|".into());
        for (c, (n, p)) in &t.causes {
            lines.push(format!("| {c} | {n} | {p} |"));
        }
        lines.push(String::new());
        lines.push(format!(
            "| Kind | {what} with one | {what} it alone holds back |"
        ));
        lines.push("|---|---|---|".into());
        for (w, n, alone) in &t.kinds {
            lines.push(format!("| {w} | {n} | {alone} |"));
        }
        lines.push(String::new());
        lines.push(format!(
            "| What | Cause | Field | {what} with it | {what} it alone holds back |"
        ));
        lines.push("|---|---|---|---|---|".into());
        for (w, c, f, n, alone) in &t.fixes {
            lines.push(format!("| {w} | {c} | {f} | {n} | {alone} |"));
        }
    }
    lines.push(String::new());
    for p in &starts {
        let now = match (&p.before, &p.cant) {
            (Some(_), None) => "opened",
            (None, Some(_)) => "refused",
            (None, None) => "usable",
            (Some(_), Some(_)) => "still refused",
        };
        lines.push(format!(
            "{:28} {:8} {}",
            p.id,
            now,
            p.cant.as_deref().unwrap_or("")
        ));
    }
    Ok(crate::Outcome::ok(
        json!({
            "starts": starts,
            "sampled": sampled,
            "tally": {"starts": ts, "sampled": tf},
        }),
        lines.join("\n"),
    ))
}
