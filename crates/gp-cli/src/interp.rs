//! `gp interp run` and `gp interp compare` with game-printer's inputs and
//! modes back: a model played on an input log, a baseline's holds (or, once
//! a recording in the store), in a profile, at a level or from a
//! state, with its hashes, frames file and APU log written; and the
//! comparison with the original through the state mapping, with the
//! lockstep's report, in either mode, from a frame, with a draws script.
//!
//! Changed from game-printer's interp.rs by gameprinter's layout: the model
//! runs through [`crate::hosted::Source`], so a model with hooks needs its
//! game's module (`--module`); without one, `--allow-missing-ext` runs it in
//! the built-in interpreter on its components alone, as before. The
//! original starts through [`crate::original::OriginalRuntime`]. A
//! recording's input log is read from the store wave 3a+ brought back.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context as _, Result, anyhow, bail};
use gp_evidence::frame_hashes::{self, FrameHash};
use gp_evidence::{Hash, InputLog, Paths};
use gp_interp::lockstep::Mode;
use gp_interp::mapping::{MAPPING_FILE, Mapping, show};
use gp_interp::{Bundle, Lifted};
use gp_model::Model;
use gp_runtime::Runtime;
use serde_json::json;

use crate::Outcome;
use crate::hosted::{self, Source};
use crate::mechanics::model_file;

#[derive(clap::Args)]
#[group(required = true, multiple = false)]
pub struct InputArgs {
    /// The input log to play.
    #[arg(long)]
    pub input: Option<PathBuf>,
    /// A baseline whose holds give the input.
    #[arg(long)]
    pub baseline: Option<PathBuf>,
    /// A recording in the store (an ID prefix) whose input log to play.
    #[arg(long, value_name = "ID")]
    pub recording: Option<String>,
}

impl InputArgs {
    pub(crate) fn read(&self, paths: &Paths) -> Result<InputLog> {
        match (&self.input, &self.baseline, &self.recording) {
            (Some(p), _, _) => InputLog::parse(
                &fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?,
            )
            .map_err(|e| anyhow!("{}: {e}", p.display())),
            (None, Some(b), _) => gp_evidence::baseline::Baseline::load(b)
                .with_context(|| format!("reading {}", b.display()))?
                .input()
                .map_err(|e| anyhow!("{}: {e}", b.display())),
            (None, None, Some(id)) => {
                let o = crate::open::Opened::new(paths, id)?;
                let bytes = o.store.get(&o.manifest.input_log)?;
                InputLog::parse(&String::from_utf8_lossy(&bytes))
                    .map_err(|e| anyhow!("recording {}: {e}", o.short()))
            }
            (None, None, None) => bail!("give --input, --baseline or --recording"),
        }
    }
}

#[derive(clap::Args)]
pub struct RunArgs {
    /// A model directory (holding model.toml) or a model.toml file.
    pub model: PathBuf,
    #[command(flatten)]
    pub input: InputArgs,
    /// The game's built module. Needed for a model with hooks.
    #[arg(long)]
    pub module: Option<PathBuf>,
    /// The model's lifted assets, as gp assets lift writes them
    /// (default: $GP_HOME/work/assets/<model.source>).
    #[arg(long, value_name = "DIR")]
    pub assets: Option<PathBuf>,
    /// Run a model that names hooks without its module, in the built-in
    /// interpreter: those entities then do only what their components say.
    #[arg(long)]
    pub allow_missing_ext: bool,
    /// Run the model in this profile from the profiles.toml beside it: a
    /// preset such as `normal`, a switch, or switches and presets joined by
    /// `+`. The default, `retro`, is the model as written. The state hashes
    /// include the switches on.
    #[arg(long, value_name = "NAME")]
    pub profile: Option<String>,
    /// Write the state hash at power-on and after every frame to this file,
    /// one in hex a line, as the web build's `gp_run_frames` writes them, to
    /// compare another runtime with frame by frame.
    #[arg(long, value_name = "FILE")]
    pub hashes: Option<PathBuf>,
    /// Write the input played as a frames file (`gp-frames 1`), for another
    /// runtime to play the same frames.
    #[arg(long, value_name = "FILE")]
    pub frames: Option<PathBuf>,
    /// Write the APU registers the model's hooks write (a game's sound
    /// driver kept as state) as an untimed APU log (docs/formats/apu-log.md).
    #[arg(long, value_name = "FILE")]
    pub apu_log: Option<PathBuf>,
    /// Start the model at this level instead of its `start_level`.
    #[arg(long, value_name = "NAME")]
    pub level: Option<String>,
    /// Start from this canonical model state (as `gp interp translate --out`
    /// writes one) instead of power-on, through the runtime protocol's
    /// load-state. The hashes then start with its hash.
    #[arg(long, value_name = "FILE")]
    pub state: Option<PathBuf>,
}

#[derive(clap::Args)]
pub struct CompareArgs {
    /// A model directory holding model.toml and mapping.toml.
    pub model: PathBuf,
    /// The original game's ROM.
    #[arg(long)]
    pub rom: PathBuf,
    #[command(flatten)]
    pub input: InputArgs,
    /// The game's built module. Needed for a model with hooks.
    #[arg(long)]
    pub module: Option<PathBuf>,
    /// The model's lifted assets (default:
    /// $GP_HOME/work/assets/<model.source>).
    #[arg(long, value_name = "DIR")]
    pub assets: Option<PathBuf>,
    /// The state mapping, instead of the model directory's mapping.toml.
    #[arg(long, value_name = "FILE")]
    pub mapping: Option<PathBuf>,
    /// Run a model that names hooks without its module, in the built-in
    /// interpreter: those entities then do only what their components say.
    #[arg(long)]
    pub allow_missing_ext: bool,
    /// The lockstep's mode: `resync` puts the original's mapped state back
    /// into the model at every split (a path past its tolerance) and goes
    /// on; `free` never does, and shows the drift.
    #[arg(long, default_value = "resync", value_name = "MODE")]
    pub mode: String,
    /// Start the lockstep at this frame of the original: its state there is
    /// translated into the model's through the mapping and both go on from
    /// it.
    #[arg(long, value_name = "FRAME")]
    pub from: Option<u64>,
    /// A random draws script (`gp interp draws`): the lockstep's model takes
    /// the original's random draws from it in place of its own generator's,
    /// and reports where it first draws at another point or another number
    /// of times (docs/formats/draws.md).
    #[arg(long, value_name = "FILE")]
    pub draws: Option<PathBuf>,
    /// A path whose values on each compared frame, the original's and the
    /// model's, are printed where either changes (and all of them in the
    /// JSON's `values`): a mapped path, or one the run doesn't compare (a
    /// presentation field, or a hook's word as `entities.flow[0].ext[3]`),
    /// so a hook's own state can be seen on a run from power-on.
    /// Repeatable.
    #[arg(long = "values", value_name = "PATH")]
    pub values: Vec<String>,
}

fn model_dir(model: &Path) -> PathBuf {
    if model.is_dir() {
        model.to_owned()
    } else {
        model
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
            .to_owned()
    }
}

/// The model in a profile from the `profiles.toml` beside it; the model as
/// written for none or the base.
fn in_profile(dir: &Path, m: Model, profile: Option<&str>) -> Result<Model> {
    match profile {
        None | Some(gp_model::profile::BASE) => Ok(m),
        Some(name) => {
            let profiles =
                gp_model::profile::Profiles::read_dir(dir).map_err(|e| anyhow!("{e}"))?;
            profiles.apply(&m, name).map_err(|e| anyhow!("{e}"))
        }
    }
}

/// What the commands load: the model, edited by a profile and a start
/// level, with its lifted assets, as the tools run it.
struct Loaded {
    source: Source,
    assets: PathBuf,
    /// The hooks the model names that this run doesn't have: none through
    /// a module.
    not_run: Vec<String>,
}

fn load(
    model: &Path,
    assets: Option<&Path>,
    module: Option<&Path>,
    (profile, level): (Option<&str>, Option<&str>),
    allow_missing_ext: bool,
    paths: &Paths,
) -> Result<Loaded> {
    let file = model_file(model);
    let text = fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
    let m = Model::parse(&text).with_context(|| file.display().to_string())?;
    let mut m = in_profile(&model_dir(model), m, profile)?;
    if let Some(level) = level {
        m.model.start_level = Some(level.to_owned());
    }
    let assets = match assets {
        Some(a) => a.to_owned(),
        None => {
            let Some(source) = &m.model.source else {
                bail!("the model names no source, so give its lifted assets with --assets");
            };
            paths.home.join("work").join("assets").join(source)
        }
    };
    let lifted = Lifted::read_dir(&assets, &m).map_err(|e| {
        anyhow!("{e}\n(lift the model's assets with gp assets lift, or name them with --assets)")
    })?;
    let source = Source::new_allowing(Bundle::new(&m, lifted), module, allow_missing_ext)?;
    let not_run = match source.open()? {
        hosted::Model::Native(i) => i.ext_hooks().iter().map(|h| (*h).to_owned()).collect(),
        hosted::Model::Module(_) => Vec::new(),
    };
    Ok(Loaded {
        source,
        assets,
        not_run,
    })
}

fn not_run_line(not_run: &[String]) -> Option<String> {
    (!not_run.is_empty()).then(|| {
        format!(
            "ext hooks not run (no module given; those entities run on their components alone): {}",
            not_run.join(", ")
        )
    })
}

/// `gp interp run`.
pub(crate) fn run(a: &RunArgs, paths: &Paths) -> Result<Outcome> {
    let input = a.input.read(paths)?;
    let l = load(
        &a.model,
        a.assets.as_deref(),
        a.module.as_deref(),
        (a.profile.as_deref(), a.level.as_deref()),
        a.allow_missing_ext,
        paths,
    )?;
    let mut rt = l.source.open()?;
    if a.apu_log.is_some() {
        rt = rt.with_apu_events()?;
    }
    if let Some(s) = &a.state {
        let bytes = fs::read(s).with_context(|| format!("reading {}", s.display()))?;
        rt.load_state(&bytes)
            .map_err(|e| anyhow!("{}: {e}", s.display()))?;
    }
    let spec_digest = gp_sim::save::spec_digest(rt.spec());
    let hash = |rt: &hosted::Model| {
        let h = Hash::from_bytes(rt.world().hash());
        FrameHash { state: h, ram: h }
    };
    let mut hashes = vec![hash(&rt)];
    let start = Instant::now();
    let mut apu = Vec::new();
    let inputs = crate::run::frames(&input);
    for (k, f) in inputs.iter().enumerate() {
        let events = rt
            .step(std::slice::from_ref(f))
            .map_err(|e| anyhow!("{e}"))?;
        if a.apu_log.is_some() {
            for (reg, value) in gp_interp::lockstep::apu_writes(&events) {
                apu.push(gp_sound::log::Write {
                    frame: k as u64 + 1,
                    cycle: 0,
                    reg,
                    value,
                });
            }
        }
        hashes.push(hash(&rt));
    }
    let fps = input.frames.len() as f64 / start.elapsed().as_secs_f64().max(1e-9);
    if let Some(out) = &a.apu_log {
        let log = gp_sound::log::ApuLog {
            first: 1,
            end: input.frames.len() as u64 + 1,
            writes: apu,
            ..Default::default()
        };
        fs::write(out, log.to_text()).with_context(|| format!("writing {}", out.display()))?;
    }
    let digest = Hash::of(&frame_hashes::encode(&hashes));
    if let Some(out) = &a.hashes {
        let text: String = hashes.iter().map(|h| format!("{}\n", h.state)).collect();
        fs::write(out, text).with_context(|| format!("writing {}", out.display()))?;
    }
    if let Some(out) = &a.frames {
        fs::write(out, frames_file(&inputs)?)
            .with_context(|| format!("writing {}", out.display()))?;
    }
    let leaves = gp_interp::state::leaves(rt.spec(), rt.world());
    let state: Vec<(String, String)> = leaves
        .iter()
        .map(|(p, v)| (gp_model::path::display(p), show(v)))
        .collect();
    let profile = rt
        .spec()
        .profile
        .clone()
        .unwrap_or_else(|| gp_model::profile::BASE.to_owned());
    let mut text = format!(
        "ran {} frames of {} ({profile} physics) at {fps:.0} frames/s\ndigest {digest}\n",
        input.frames.len(),
        rt.spec().name
    );
    if let Some(line) = not_run_line(&l.not_run) {
        text.push_str(&line);
        text.push('\n');
    }
    if !l.source.hooks.is_empty() {
        text.push_str(&format!("ext hooks: {}\n", l.source.hooks));
    }
    for (p, v) in &state {
        text.push_str(&format!("{p} = {v}\n"));
    }
    Ok(Outcome::ok(
        json!({
            "model": rt.spec().name,
            "profile": profile,
            "frames": input.frames.len(),
            "digest": digest,
            "spec_digest": spec_digest,
            "fps": fps,
            "ext_not_run": l.not_run,
            "ext_hooks": l.source.hooks,
            "state": state.iter().map(|(p, v)| json!({"path": p, "value": v})).collect::<Vec<_>>(),
        }),
        text.trim_end().to_owned(),
    ))
}

/// A frames file (`gp-frames 1`): a line a frame, controller 1's bits in
/// hex, then controller 2's when any is down, then `r` on a reset.
fn frames_file(inputs: &[gp_runtime::InputFrame]) -> Result<String> {
    let mut s = format!("{}\n", gp_look::FRAMES_HEADER);
    for f in inputs {
        let b = u16::try_from(f.buttons)
            .map_err(|_| anyhow!("a frames file holds controllers 1 and 2 only"))?;
        let [b1, b2] = b.to_le_bytes();
        s.push_str(&format!("{b1:02x}"));
        if b2 != 0 {
            s.push_str(&format!(" {b2:02x}"));
        }
        s.push_str(if f.reset { " r\n" } else { "\n" });
    }
    Ok(s)
}

/// The state mapping: the given file, or the model directory's.
fn load_mapping(model: &Path, mapping: Option<&Path>) -> Result<(PathBuf, Mapping)> {
    let file = match mapping {
        Some(f) => f.to_owned(),
        None => model_dir(model).join(MAPPING_FILE),
    };
    let mapping = Mapping::parse(
        &fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?,
    )
    .map_err(|e| anyhow!("{}: {e}", file.display()))?;
    Ok((file, mapping))
}

/// `gp interp compare`.
pub(crate) fn compare(a: &CompareArgs, paths: &Paths) -> Result<Outcome> {
    let input = a.input.read(paths)?;
    let mode = Mode::parse(&a.mode).map_err(|e| anyhow!("{e}"))?;
    let draws = match &a.draws {
        Some(p) => Some(
            gp_sim::DrawScript::parse(
                &fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?,
            )
            .map_err(|e| anyhow!("{}: {e}", p.display()))?,
        ),
        None => None,
    };
    let l = load(
        &a.model,
        a.assets.as_deref(),
        a.module.as_deref(),
        (None, None),
        a.allow_missing_ext,
        paths,
    )?;
    let (mapping_file, mapping) = load_mapping(&a.model, a.mapping.as_deref())?;
    let rom = fs::read(&a.rom).with_context(|| format!("reading {}", a.rom.display()))?;
    let interp = l.source.open()?;
    if mapping.model != interp.spec().name {
        bail!(
            "{} is the state mapping of `{}`, not of `{}`",
            mapping_file.display(),
            mapping.model,
            interp.spec().name
        );
    }
    let inputs = crate::run::frames(&input);
    // A mapping with `score_draws` scores with the original's draws: with
    // no script given, the run records one from the input. From the
    // model's own power-on (no --from) it takes them from its first
    // compared frame (crate::lockstep::run_resumed_on).
    let draws = match draws {
        None if mapping.score_draws => Some(crate::lockstep::record_draws(
            &rom,
            &mapping,
            &input.frames,
            a.from.unwrap_or(0),
        )?),
        d => d,
    };
    // One run: the agreement by path is the lockstep's own, in its mode,
    // from its start and with its draws, so the lines below and its
    // summary say the same thing.
    let mut original = crate::original::OriginalRuntime::of_rom(&rom)?;
    let (ls, mut lockstep, ls_lines) = crate::lockstep::run_resumed_on(
        &mut original,
        &l.source,
        &model_dir(&a.model),
        &mapping,
        &rom,
        &inputs,
        mode,
        a.from,
        !a.values.is_empty(),
        Some(&l.assets),
        draws,
        None,
        None,
        &a.values,
    )?;
    let (values_json, values_lines) =
        values_of(&ls.paths, &ls.values, &ls.watched, &ls.frames, &a.values)?;
    let fields = agreement(&ls.paths, &ls.splits, &ls.frames);
    let mut lines = vec![format!(
        "compared {} model frames with {} frames of the original ({} before the model starts, {} lag frames skipped), {} mode",
        ls.frames.len(),
        ls.original_frames + 1,
        ls.before_start,
        ls.lag_frames,
        a.mode
    )];
    let width = fields.iter().map(|f| f.path.len()).max().unwrap_or(0);
    let mut fields_json = Vec::new();
    for f in &fields {
        let mut j = json!({
            "path": f.path,
            "compared": f.compared,
            "agreed": f.agreed,
            "agrees": f.first.is_none(),
        });
        match &f.first {
            None => lines.push(format!(
                "agrees   {:width$}  {} of {} frames",
                f.path, f.agreed, f.compared
            )),
            Some((frame, original_frame, theirs, ours)) => {
                lines.push(format!(
                    "DIFFERS  {:width$}  {} of {} frames; first at model frame {frame} (original frame {original_frame}): original {theirs}, model {ours}",
                    f.path, f.agreed, f.compared,
                ));
                j["first"] = json!({
                    "frame": frame,
                    "original_frame": original_frame,
                    "original": theirs,
                    "model": ours,
                });
            }
        }
        fields_json.push(j);
    }
    let agreeing = fields.iter().filter(|f| f.first.is_none()).count();
    lines.extend(ls_lines);
    lines.extend(values_lines);
    if !a.values.is_empty() {
        lockstep["values"] = values_json;
    }
    if let Some(line) = not_run_line(&l.not_run) {
        lines.push(line);
    }
    if !l.source.hooks.is_empty() {
        lines.push(format!("ext hooks: {}", l.source.hooks));
    }
    // Last, which scripts read last.
    lines.push(format!(
        "{agreeing} of {} mapped paths agree on every frame",
        fields.len()
    ));
    let agrees = agreeing == fields.len();
    Ok(Outcome {
        json: json!({
            "lockstep": lockstep,
            "model": interp.spec().name,
            "model_frames": ls.frames.len(),
            "original_frames": ls.original_frames + 1,
            "before_start": ls.before_start,
            "lag_frames": ls.lag_frames,
            "agrees": agrees,
            "ext_not_run": l.not_run,
            "ext_hooks": l.source.hooks,
            "fields": fields_json,
        }),
        text: lines.join("\n"),
        exit: u8::from(!agrees),
    })
}

/// `--values`: each path's two values on every compared frame, as
/// `gp clip score --values` reads them (a compared path from the run's
/// values, any other from its watched values), in JSON as
/// `{path: [[original, model], ...]}` beside the frames; as text, the
/// frames where either value changes.
fn values_of(
    paths: &[gp_interp::lockstep::PathSeries],
    values: &[(Vec<gp_runtime::Value>, Vec<gp_runtime::Value>)],
    watched: &[Vec<Option<gp_interp::lockstep::Watched>>],
    frames: &[(u64, u64)],
    want: &[String],
) -> Result<(serde_json::Value, Vec<String>)> {
    let all = |xs: &[gp_runtime::Value]| xs.iter().map(show).collect::<Vec<_>>().join(" ");
    let mut by = serde_json::Map::new();
    let mut lines = Vec::new();
    for (w, path) in want.iter().enumerate() {
        let rows: Vec<(String, String)> = match paths.iter().position(|x| &x.path == path) {
            Some(p) => values
                .iter()
                .map(|(o, m)| {
                    let one = |v: &[gp_runtime::Value]| v.get(p).map_or_else(String::new, show);
                    (one(o), one(m))
                })
                .collect(),
            None if watched
                .iter()
                .any(|row| row.get(w).is_some_and(Option::is_some)) =>
            {
                watched
                    .iter()
                    .map(|row| match row.get(w).and_then(Option::as_ref) {
                        Some((o, m)) => (all(o), all(m)),
                        None => (String::new(), String::new()),
                    })
                    .collect()
            }
            None => bail!(
                "--values {path}: the run compares no such path and the mapping maps no field there (a mapped field's path, or a slot field as entities.KIND[N].FIELD)"
            ),
        };
        let mut last: Option<&(String, String)> = None;
        for (i, r) in rows.iter().enumerate() {
            if last != Some(r) {
                let (m, o) = frames.get(i).copied().unwrap_or_default();
                lines.push(format!(
                    "values {path} at model frame {m} (original frame {o}): original {}, model {}",
                    r.0, r.1
                ));
            }
            last = Some(r);
        }
        by.insert(
            path.clone(),
            serde_json::Value::Array(rows.iter().map(|(o, m)| json!([o, m])).collect()),
        );
    }
    Ok((json!({"frames": frames, "values": by}), lines))
}

/// One mapped path's agreement in a lockstep: the frames compared, those
/// within its tolerance, and its first split (the model's and the
/// original's frame, and both values there, as they stood before any
/// resync).
struct Agreement {
    path: String,
    compared: usize,
    agreed: u64,
    first: Option<(u64, u64, String, String)>,
}

/// Each path's [`Agreement`], read from a lockstep's own series, splits
/// and compared frames.
fn agreement(
    paths: &[gp_interp::lockstep::PathSeries],
    splits: &[gp_interp::lockstep::Split],
    frames: &[(u64, u64)],
) -> Vec<Agreement> {
    paths
        .iter()
        .map(|p| {
            let first = p.first_split.and_then(|at| {
                let split = splits.iter().find(|s| s.at == at)?;
                let sp = split.paths.iter().find(|x| x.path == p.path)?;
                Some((
                    split.frame,
                    split.original_frame,
                    show(&sp.original),
                    show(&sp.model),
                ))
            });
            // A split this path's series has but the split list doesn't
            // name (it never should) still differs, at its frame.
            let first = first.or_else(|| {
                p.first_split.map(|at| {
                    let (m, o) = frames.get(at).copied().unwrap_or_default();
                    (m, o, "?".to_owned(), "?".to_owned())
                })
            });
            Agreement {
                path: p.path.clone(),
                compared: p.distance.len(),
                agreed: p.within,
                first,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    /// `--values` on a made-up run of three frames: a compared path read
    /// from the run's values, a hook's word from its watched values, each
    /// printed where it changes; a path neither has is refused in words.
    #[test]
    fn values_are_printed_where_they_change() {
        use gp_runtime::Value;
        let series = gp_interp::lockstep::PathSeries {
            path: "counters.fare".into(),
            tolerance: gp_sim::Fx::ZERO,
            distance: vec![gp_sim::Fx::ZERO; 3],
            within: 3,
            max: gp_sim::Fx::ZERO,
            first_split: None,
        };
        let int = |n: i64| vec![Value::Int(n)];
        let values = vec![(int(1), int(1)), (int(1), int(1)), (int(2), int(2))];
        let watched = vec![
            vec![None, Some((int(7), int(7)))],
            vec![None, Some((int(8), int(7)))],
            vec![None, Some((int(8), int(7)))],
        ];
        let frames = [(0, 94), (1, 95), (2, 96)];
        let want = [
            "counters.fare".to_owned(),
            "entities.cab[0].ext[3]".to_owned(),
        ];
        let (j, lines) = super::values_of(
            std::slice::from_ref(&series),
            &values,
            &watched,
            &frames,
            &want,
        )
        .unwrap();
        assert_eq!(
            lines,
            vec![
                "values counters.fare at model frame 0 (original frame 94): original 1, model 1",
                "values counters.fare at model frame 2 (original frame 96): original 2, model 2",
                "values entities.cab[0].ext[3] at model frame 0 (original frame 94): original 7, model 7",
                "values entities.cab[0].ext[3] at model frame 1 (original frame 95): original 8, model 7",
            ]
        );
        assert_eq!(j["values"]["entities.cab[0].ext[3]"][2][0], "8");
        let e = super::values_of(&[series], &values, &watched, &frames, &["nope".to_owned()])
            .unwrap_err()
            .to_string();
        assert!(e.contains("--values nope"), "{e}");
    }

    /// A made-up lockstep of three frames: a path that stays within its
    /// tolerance agrees, and one that splits on the second frame differs
    /// there with the split's own values, so the lines and the lockstep's
    /// splits say the same thing.
    #[test]
    fn the_agreement_is_the_lockstep_s_own() {
        use gp_interp::lockstep::{PathSeries, Split, SplitPath};
        use gp_runtime::Value;
        let series = |path: &str, within: u64, first_split: Option<usize>| PathSeries {
            path: path.into(),
            tolerance: gp_sim::Fx::ZERO,
            distance: vec![gp_sim::Fx::ZERO; 3],
            within,
            max: gp_sim::Fx::ZERO,
            first_split,
        };
        let paths = [
            series("entities.cab[0].x", 3, None),
            series("counters.fare", 2, Some(1)),
        ];
        let splits = [Split {
            at: 1,
            frame: 11,
            original_frame: 105,
            paths: vec![SplitPath {
                path: "counters.fare".into(),
                original: Value::Int(4),
                model: Value::Int(5),
                distance: gp_sim::Fx::from_int(1),
            }],
        }];
        let frames = [(10, 104), (11, 105), (12, 106)];
        let a = super::agreement(&paths, &splits, &frames);
        assert_eq!(
            (a[0].compared, a[0].agreed, a[0].first.is_none()),
            (3, 3, true)
        );
        assert_eq!(a[1].agreed, 2);
        assert_eq!(a[1].first, Some((11, 105, "4".to_owned(), "5".to_owned())));
    }

    #[test]
    fn a_frames_file_has_a_line_a_frame_and_marks_resets() {
        let f = |buttons, reset| gp_runtime::InputFrame {
            buttons,
            axes: Vec::new(),
            reset,
        };
        let text = super::frames_file(&[f(0x01, false), f(0x0880, true)]).unwrap();
        assert_eq!(text, "gp-frames 1\n01\n80 08 r\n");
        assert!(super::frames_file(&[f(0x1_0000, false)]).is_err());
    }
}
