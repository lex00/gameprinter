//! `gp interp moments` and the game view's moments strip
//! (docs/interpreter.md, "Moments"): the game's own moments in a run of
//! the original (jumps, landings, deaths, lives lost, counter changes,
//! level changes), each with what the model does there on the same
//! presses, from the original's state a little before it
//! (gp_interp::moments).
//!
//! Ported from game-printer: the original starts through
//! [`crate::original::OriginalRuntime`] and the model runs through
//! [`crate::hosted::Source`] (its module, or the built-in interpreter).

use std::path::Path;

use anyhow::Result;
use serde_json::{Value as Json, json};

use gp_interp::lockstep::{self, Lockstep, Mode, Options};
use gp_interp::mapping::Mapping;
use gp_interp::moments::{self, Answer, Moment, Side};
use gp_runtime::{InputFrame, Runtime as _};
use gp_sim::Fx;

use crate::hosted::Source;
use crate::original::OriginalRuntime;

fn px(f: Fx) -> f64 {
    // Floats only in presentation.
    f.raw() as f64 / 65536.0
}

/// The mapped path a kind of moment is read from, for the mechanic that
/// owns it: a jump's vertical speed (else its height), a landing's
/// standing, a death's being alive, a counter's or the level's own path.
fn owner_path(ls: &Lockstep, m: &Moment) -> String {
    let has = |f: &str| {
        let p = format!("{}.{f}", m.part);
        ls.paths.iter().any(|x| x.path == p).then_some(p)
    };
    match m.kind {
        "jump" => has("vy").or_else(|| has("y")).unwrap_or_default(),
        "landing" => has("on_ground").unwrap_or_default(),
        "death" => has("alive").unwrap_or_default(),
        _ => m.part.clone(),
    }
}

/// The mechanic that owns a moment's kind for its part, by the catalog.
pub(crate) fn mechanic_of(
    catalog: &gp_interp::mechanics::Catalog,
    ls: &Lockstep,
    m: &Moment,
) -> Option<&'static str> {
    let p = owner_path(ls, m);
    if p.is_empty() {
        return None;
    }
    lockstep::owners(catalog, &p).first().copied()
}

/// What the whole run's moments are read from: the original and the model
/// in lockstep from the start, with values.
pub(crate) fn whole(
    source: &Source,
    mapping: &Mapping,
    rom: &[u8],
    inputs: &[InputFrame],
) -> Result<Lockstep> {
    let mut original = OriginalRuntime::of_rom(rom)?;
    let mut interp = source.open()?;
    let spec = source.spec.clone();
    lockstep::lockstep(
        &mut original,
        &mut interp,
        &spec,
        mapping,
        inputs,
        &Options {
            mode: Mode::Resync,
            tolerance: Vec::new(),
            from: None,
            start_state: None,
            values: true,
            until_start: false,
            sound: false,
            stop_at_split: false,
            watch: Vec::new(),
        },
    )
    .map_err(anyhow::Error::msg)
}

/// The model's side of `found`, each from the original's state
/// [`moments::LEAD`] frames before it.
pub(crate) fn answers(
    source: &Source,
    mapping: &Mapping,
    rom: &[u8],
    inputs: &[InputFrame],
    found: &[Moment],
    checks: &moments::Checks,
) -> Result<Vec<Answer>> {
    let mut original = OriginalRuntime::of_rom(rom)?;
    let spec = source.spec.clone();
    // One run at power-on, forked for each check as the old tool cloned its
    // interpreter: a fork shares the module's instance.
    let base = source.open()?;
    let mut make = || -> Result<crate::hosted::Model, String> { Ok(base.fork()) };
    moments::check(
        &mut original,
        &mut make,
        &spec,
        mapping,
        inputs,
        found,
        checks,
    )
    .map_err(anyhow::Error::msg)
}

/// A moment's JSON: its kind, number, frames and seconds, what it did,
/// the stretch a focus on it covers, and the model's side.
pub(crate) fn moment_json(
    m: &Moment,
    a: Option<&Answer>,
    mechanic: Option<&str>,
    secs: &dyn Fn(u64) -> f64,
    first: u64,
) -> Json {
    let (from, to) = moments::span(m, moments::LEAD, first);
    let model = a.and_then(|a| a.model.as_ref()).map(|x| {
        json!({"from": x.from, "to": x.to, "offset": x.from as i64 - m.from as i64,
               "peak": x.peak, "height": x.height.map(px), "after": x.after})
    });
    json!({
        "kind": m.kind,
        "n": m.n,
        "part": m.part,
        "mechanic": mechanic,
        "from": m.from,
        "to": m.to,
        "seconds_from": secs(m.from),
        "seconds_to": secs(m.to),
        "peak": m.peak,
        "height": m.height.map(px),
        "before": m.before,
        "after": m.after,
        "focus": {"from": from.max(1), "to": to},
        "checked": a.is_some(),
        "start": a.map(|a| a.start),
        "model": model,
    })
}

/// A moment in a line, for the text report.
pub(crate) fn moment_line(m: &Moment, a: Option<&Answer>, secs: &dyn Fn(u64) -> f64) -> String {
    let what = match (m.kind, m.height) {
        ("jump", Some(h)) => format!(" rises {} px", px(h)),
        ("counter" | "level", _) => format!(
            " {} {} -> {}",
            m.part,
            m.before.as_deref().unwrap_or("?"),
            m.after.as_deref().unwrap_or("?")
        ),
        _ => String::new(),
    };
    let model = match a.and_then(|a| a.model.as_ref()) {
        Some(x) => {
            let d = x.from as i64 - m.from as i64;
            let when = match d {
                0 => "on the same frame".to_owned(),
                d if d > 0 => format!("{d} frames later"),
                d => format!("{} frames earlier", -d),
            };
            match x.height {
                Some(h) if m.kind == "jump" => format!("the model too, {when}, {} px", px(h)),
                _ => format!("the model too, {when}"),
            }
        }
        None if a.is_some() => "the model doesn't".to_owned(),
        None => "not checked".to_owned(),
    };
    format!(
        "{} {} at frames {}-{} ({:.1} s):{what}; {model}",
        m.kind,
        m.n,
        m.from,
        m.to,
        secs(m.from)
    )
}

/// The moments of a run and the model's side of each: the JSON and the
/// text lines. `kind` keeps one kind.
pub(crate) fn report(
    source: &Source,
    mapping: &Mapping,
    rom: &[u8],
    inputs: &[InputFrame],
    kind: Option<&str>,
) -> Result<(Json, Vec<String>)> {
    let model = source.parse_model()?;
    let tree = model.to_tree();
    let clocks = moments::clocks(&tree);
    let ls = whole(source, mapping, rom, inputs)?;
    let mut found = moments::detect(&ls, Side::Original, &clocks);
    if let Some(k) = kind {
        found.retain(|m| m.kind == k);
    }
    let first = ls.frames.first().map_or(0, |f| f.1);
    let checks = moments::Checks {
        lead: moments::LEAD,
        first,
        clocks: clocks.clone(),
        tolerance: Vec::new(),
    };
    let checked = answers(source, mapping, rom, inputs, &found, &checks)?;
    let catalog = gp_interp::mechanics::catalog(&model, &source.exts);
    let rate = (model.model.frame_rate.num, model.model.frame_rate.den);
    let secs = |f: u64| f as f64 * rate.1 as f64 / rate.0.max(1) as f64;
    let mut list = Vec::new();
    let mut lines = Vec::new();
    let mut kinds = serde_json::Map::new();
    for (m, a) in found.iter().zip(&checked) {
        let mech = mechanic_of(&catalog, &ls, m);
        list.push(moment_json(m, Some(a), mech, &secs, first));
        lines.push(moment_line(m, Some(a), &secs));
        let e = kinds
            .entry(m.kind)
            .or_insert_with(|| json!({"original": 0, "model": 0, "mechanic": mech}));
        e["original"] = json!(e["original"].as_u64().unwrap_or(0) + 1);
        if a.model.is_some() {
            e["model"] = json!(e["model"].as_u64().unwrap_or(0) + 1);
        }
    }
    let mut head = vec![format!(
        "{} moments in {} frames measured",
        found.len(),
        ls.frames.len()
    )];
    for (k, v) in &kinds {
        head.push(format!(
            "  {k}: {} in the original, the model does {} of them",
            v["original"], v["model"]
        ));
    }
    head.extend(lines);
    Ok((
        json!({"moments": list, "kinds": kinds, "first_frame": first, "measured": ls.frames.len()}),
        head,
    ))
}

/// `gp interp moments`.
pub(crate) fn cmd(
    model_dir: &Path,
    source: &Source,
    mapping: &Mapping,
    rom: &[u8],
    inputs: &[InputFrame],
    kind: Option<&str>,
) -> Result<crate::Outcome> {
    let _ = model_dir;
    if let Some(k) = kind
        && !moments::KINDS.contains(&k)
    {
        anyhow::bail!(
            "a kind of moment is one of {}, not `{k}`",
            moments::KINDS.join(", ")
        );
    }
    let (j, lines) = report(source, mapping, rom, inputs, kind)?;
    Ok(crate::Outcome::ok(j, lines.join("\n")))
}

/// `gp interp moments`'s arguments.
#[derive(clap::Args)]
pub struct MomentsArgs {
    /// A model directory holding model.toml and mapping.toml.
    pub model: std::path::PathBuf,
    /// The original game's ROM.
    #[arg(long)]
    pub rom: std::path::PathBuf,
    #[command(flatten)]
    pub input: crate::interp_args::InputArgs,
    /// The game's built module. Needed for a model with hooks.
    #[arg(long)]
    pub module: Option<std::path::PathBuf>,
    /// The model's lifted assets (default:
    /// $GP_HOME/work/assets/<model.source>).
    #[arg(long, value_name = "DIR")]
    pub assets: Option<std::path::PathBuf>,
    /// The state mapping, instead of the model directory's mapping.toml.
    #[arg(long, value_name = "FILE")]
    pub mapping: Option<std::path::PathBuf>,
    /// Only this kind: jump, landing, death, life_lost, counter or level.
    #[arg(long)]
    pub kind: Option<String>,
}

/// `gp interp moments` from its arguments.
pub(crate) fn run(a: &MomentsArgs, paths: &gp_evidence::Paths) -> Result<crate::Outcome> {
    use crate::interp_args as ia;
    let log = a.input.read()?;
    let (source, _) = ia::source(&a.model, a.assets.as_deref(), a.module.as_deref(), paths)?;
    let (_, mapping) = ia::load_mapping(&a.model, a.mapping.as_deref())?;
    let rom = ia::read_rom(&a.rom)?;
    cmd(
        &a.model,
        &source,
        &mapping,
        &rom,
        &crate::run::frames(&log),
        a.kind.as_deref(),
    )
}
