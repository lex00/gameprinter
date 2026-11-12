//! `gp validate`: a model held to its fidelity contract against recordings
//! of the original (Milestone 2, step 5; docs/validation.md). The checks
//! are compiled and run in `gp-validate`; this module finds the model, the
//! recordings and their ROMs, runs the original and the subject, keeps
//! results in the store and prints the report.
//!
//! Ported from game-printer's validate.rs. What gameprinter's layout
//! changed: the model runs through its game's module (`--module`) or the
//! built-in interpreter ([`crate::hosted::Source`]), and the original
//! through [`OriginalRuntime`], the NES's for an NES recording or
//! baseline. The Bevy subject (`--runtime bevy`, `--project`,
//! `--target-dir`) is gone with the Bevy generator, and with it the
//! conformance check. `--allow-missing-ext` is gone: a model naming hooks
//! runs only through its module. The run recorder's metrics (`--metrics`)
//! are gone with the recorder.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Args, ValueEnum};
use serde_json::json;

use gp_contract::{CONTRACT_FILE, Contract};
use gp_evidence::baseline::{self, Baseline};
use gp_evidence::manifest::{PowerOn, SourceRef};
use gp_evidence::recordings;
use gp_evidence::source::identify_nes;
use gp_evidence::{Hash, InputLog, Paths, Store};
use gp_interp::mapping::{MAPPING_FILE, Mapping};
use gp_model::{MODEL_FILE, Model};
use gp_nes::protocol::NesRuntime;
use gp_runtime::{InputFrame, Runtime as _, Value};
use gp_validate::cache::{self, KeyParts};
use gp_validate::compile::{self, Plan};
use gp_validate::evaluate::{Against, Status};
use gp_validate::original::{self, OBSERVE_FILE, ObserveMap, Subject};
use gp_validate::{RecordingResult, Summary};

use crate::Outcome;
use crate::hosted::{self, Source};
use crate::open::{RomFolder, now_rfc3339};
use crate::original::OriginalRuntime;

#[derive(Args)]
pub struct ValidateArgs {
    /// The model directory, holding model.toml and fidelity.toml.
    pub model: PathBuf,
    /// Recordings of the original to validate against: recording IDs (or
    /// unique prefixes of at least 4 characters), baseline files, or
    /// directories of baselines.
    #[arg(long, num_args = 1.., required_unless_present = "store")]
    pub against: Vec<String>,
    /// Validate against all of the store's recordings of the model's source
    /// ROM (the registry's, or `--rom`'s).
    #[arg(long, conflicts_with = "against")]
    pub store: bool,
    /// The runtime held against the original: `interp` runs the model
    /// (through its game's module with `--module`, else in the built-in
    /// interpreter); `nes` runs the original itself, which tests the
    /// engine; `none` holds only the original against the model's numbers.
    #[arg(long, value_enum, default_value_t = RuntimeArg::Interp)]
    pub runtime: RuntimeArg,
    /// The game's built module, which runs the model's hooks. Needed for a
    /// model with hooks.
    #[arg(long)]
    pub module: Option<PathBuf>,
    /// The lifted assets the interpreter reads (default:
    /// $GP_HOME/work/assets/<model.source>).
    #[arg(long)]
    pub assets: Option<PathBuf>,
    /// A ROM file to use for every recording that names its hash, rather
    /// than looking each one up in the ROM folder.
    #[arg(long)]
    pub rom: Option<PathBuf>,
    /// The original's state mapping, instead of the model directory's
    /// mapping.toml (docs/formats/state-mapping.md).
    #[arg(long)]
    pub mapping: Option<PathBuf>,
    /// Neither read earlier results nor keep new ones, so nothing is written.
    #[arg(long)]
    pub no_save: bool,
    /// The subject runs from its own power-on state. Without it, it takes
    /// the original's state at the first compared frame, translated
    /// through the mapping, and the original's random draws when the
    /// mapping sets `score_draws`, as a clip's score does (gp clip score).
    #[arg(long)]
    pub power_on: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum RuntimeArg {
    Interp,
    Nes,
    None,
}

/// One run of the original to validate against.
struct Run {
    name: String,
    /// A recording's ID, or a baseline's path.
    id: String,
    /// What identifies the run's content, for the result's key.
    content: String,
    sha256: String,
    power_on: PowerOn,
    inputs: Vec<InputFrame>,
    /// The input log's frames, for recording the original's draws.
    log: Vec<gp_evidence::input_log::InputFrame>,
}

fn inputs_of(log: &InputLog) -> Vec<InputFrame> {
    log.frames
        .iter()
        .map(|f| InputFrame {
            buttons: f.buttons_word(),
            axes: Vec::new(),
            reset: f.reset,
        })
        .collect()
}

fn read(path: &Path) -> Result<String> {
    fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))
}

pub(crate) fn run(args: &ValidateArgs, paths: &Paths) -> Result<Outcome> {
    let dir = &args.model;
    if !dir.is_dir() {
        bail!("{} isn't a model directory", dir.display());
    }
    let model_text = read(&dir.join(MODEL_FILE))?;
    let model =
        Model::parse(&model_text).with_context(|| dir.join(MODEL_FILE).display().to_string())?;
    let contract_text = read(&dir.join(CONTRACT_FILE))?;
    let contract = Contract::parse(&contract_text)
        .with_context(|| dir.join(CONTRACT_FILE).display().to_string())?;
    let problems = contract.check(&model);
    if !problems.is_empty() {
        let list: Vec<String> = problems.iter().map(ToString::to_string).collect();
        bail!(
            "{} has problems; `gp model check` lists them:\n{}",
            dir.join(CONTRACT_FILE).display(),
            list.join("\n")
        );
    }
    let mapping_file = args
        .mapping
        .clone()
        .unwrap_or_else(|| dir.join(MAPPING_FILE));
    let mapping_text = read(&mapping_file)?;
    let mapping = Mapping::parse(&mapping_text)
        .map_err(anyhow::Error::msg)
        .with_context(|| mapping_file.display().to_string())?;
    // What the mapping can't say, such as the level and entities in slots.
    let observe_file = dir.join(OBSERVE_FILE);
    let observe = match fs::read_to_string(&observe_file) {
        Ok(text) => Some(
            ObserveMap::parse(&text)
                .map_err(anyhow::Error::msg)
                .with_context(|| observe_file.display().to_string())?,
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e).with_context(|| observe_file.display().to_string()),
    };
    if let Some(o) = &observe
        && o.model != model.model.name
    {
        bail!(
            "{} is for {}, and the model is {}",
            observe_file.display(),
            o.model,
            model.model.name
        );
    }
    let mapping_text = match &observe {
        Some(o) => format!("{mapping_text}\n{}", o.text),
        None => mapping_text,
    };
    if mapping.model != model.model.name {
        bail!(
            "the state mapping is for {}, and the model is {}",
            mapping.model,
            model.model.name
        );
    }
    // Enemies in the original's reused slots are measured slot life by
    // slot life (gp_validate::slots), in place of their walk checks.
    let slotted: std::collections::BTreeSet<String> = mapping
        .slots
        .iter()
        .flat_map(|s| s.kinds.iter().map(|k| k.entity.clone()))
        .collect();
    let plan = compile::compile_with_slots(&model, &contract, &slotted)?;
    let observe = match (observe, &mapping.slots) {
        (Some(mut o), Some(slots)) => {
            o.take_slots(slots);
            Some(o)
        }
        (o, _) => o,
    };

    let save = !args.no_save;
    // Baselines alone, with nothing to save, need no store at all.
    let needs_store = args.store || args.against.iter().any(|a| !Path::new(a).exists());
    let store = if save {
        Some(Store::open(&paths.store)?)
    } else if needs_store {
        Some(Store::open_existing(&paths.store)?)
    } else {
        None
    };
    let given = match &args.rom {
        Some(p) => {
            let bytes = fs::read(p).with_context(|| format!("reading {}", p.display()))?;
            if crate::original::Platform::of_file(&bytes) == crate::original::Platform::Arcade {
                bail!(
                    "{} is an arcade romset; gp validate runs NES recordings and baselines, and an arcade recording's source (a romset, wave 6 of the port) isn't read here yet",
                    p.display()
                );
            }
            let sha = identify_nes(&bytes)?.sha256;
            Some((sha, bytes))
        }
        None => None,
    };
    let (runs, sealed) = runs(
        args,
        store.as_ref(),
        &model,
        given.as_ref().map(|g| g.0.as_str()),
    )?;

    let original_id = gp_nes::recording::describe_runtime(&gp_nes::runtime_ref());
    // The model as the tools run it: through its game's module, or the
    // built-in interpreter for a model with no hooks (crate::hosted).
    let subject: Option<Source> = match args.runtime {
        RuntimeArg::Interp => {
            Some(Source::open_dir(dir, args.assets.as_deref(), paths, args.module.as_deref())?.0)
        }
        _ => None,
    };
    // The counters a hook may write: the checks against the model leave
    // their unexplained changes to the hook.
    let mut plan = plan;
    if let Some(src) = &subject {
        plan.kept_by_hooks = kept_by_hooks(&model, &src.exts);
    }
    // The interpreter's version, with the hooks' label, and the model and
    // assets are the subject results are keyed by, so a new interpreter or
    // other hooks don't reuse stored results.
    let subject_id = match (args.runtime, &subject) {
        (RuntimeArg::Interp, Some(src)) => Some(format!(
            "{} (model and assets blake3:{})",
            gp_nes::recording::describe_runtime(&gp_interp::recording::runtime_ref_with_hooks(
                &src.hooks
            )),
            &Hash::of(src.bundle_bytes()).to_hex()[..12]
        )),
        (RuntimeArg::Nes, _) => Some(original_id.clone()),
        _ => None,
    };
    // A subject that takes the original's state at its start is another
    // run than one from its own power-on, so their results are kept apart.
    let subject_id = match subject_id {
        Some(s) if subject.is_some() && !args.power_on => Some(format!(
            "{s}, started from the original's state{}",
            if mapping.score_draws {
                " with its draws"
            } else {
                ""
            }
        )),
        s => s,
    };
    let model_canon = model.to_toml();
    let contract_canon = contract.to_toml();
    // What each run needs, found one after another: a stored result, or
    // its ROM, read once a hash.
    let mut roms = RomFolder::new(paths);
    let mut read: std::collections::HashMap<String, Option<std::sync::Arc<Vec<u8>>>> =
        Default::default();
    enum Todo {
        Stored(RecordingResult),
        Skip,
        Run(Hash, std::sync::Arc<Vec<u8>>),
    }
    let mut todo = Vec::with_capacity(runs.len());
    for r in &runs {
        let key = cache::key(&KeyParts {
            model: &model_canon,
            contract: &contract_canon,
            mapping: &mapping_text,
            recording: &r.content,
            original: &original_id,
            subject: subject_id.as_deref(),
        });
        if let Some(store) = store.as_ref().filter(|_| save)
            && let Some(found) = cache::lookup(store, key)?
        {
            todo.push(Todo::Stored(found));
            continue;
        }
        let rom = match &given {
            Some((sha, bytes)) if *sha == r.sha256 => Some(std::sync::Arc::new(bytes.clone())),
            Some((sha, _)) if r.id.ends_with(".toml") => bail!(
                "{} is for the ROM with SHA-256 {}, and --rom has {sha}; if the game's source changed, record the baseline again",
                r.id,
                r.sha256
            ),
            _ => match read.get(&r.sha256) {
                Some(rom) => rom.clone(),
                None => {
                    let rom = roms.read(&r.sha256)?.map(std::sync::Arc::new);
                    read.insert(r.sha256.clone(), rom.clone());
                    rom
                }
            },
        };
        todo.push(match rom {
            Some(rom) => Todo::Run(key, rom),
            None => Todo::Skip,
        });
    }
    // The runs to evaluate, on as many threads as `--jobs` gives
    // (crate::pool); the results come back in the runs' order.
    let one = |r: &Run, key: Hash, rom: &[u8]| -> Result<(RecordingResult, String)> {
        let runs = {
            let load = || {
                NesRuntime::load(rom, r.power_on)
                    .map(OriginalRuntime::Nes)
                    .map_err(|e| e.to_string())
            };
            match &subject {
                Some(src) => {
                    // A clip's score takes the original's draws over the
                    // run (the mapping's `score_draws`); they start at the
                    // first compared frame, where the model takes the
                    // original's state.
                    let draws = if mapping.score_draws && !args.power_on {
                        let nes =
                            gp_nes::Nes::power_on(rom, r.power_on).map_err(anyhow::Error::msg)?;
                        Some(crate::lockstep::record_draws_on(nes, &mapping, &r.log, 0)?)
                    } else {
                        None
                    };
                    let make = || -> Result<hosted::Model, String> {
                        let m = src.open().map_err(|e| format!("{e:#}"))?;
                        match &draws {
                            Some(d) => m.with_draws(d.clone()).map_err(|e| format!("{e:#}")),
                            None => Ok(m),
                        }
                    };
                    let spec = src.spec.clone();
                    let mut take = |o: &OriginalRuntime, m: &mut hosted::Model| {
                        if draws.is_some() {
                            m.set(gp_interp::runtime::DRAWS_BASE, Value::Int(o.frame() as i64))?;
                        }
                        let mut base = gp_sim::World::power_on(&spec);
                        base.frame = m.frame();
                        gp_interp::lockstep::load(o, m, &spec, &base, &mapping, false).map(|_| ())
                    };
                    original::run(
                        &load,
                        Subject::Model {
                            make: &make,
                            start: (!args.power_on).then_some(
                                &mut take
                                    as &mut dyn FnMut(
                                        &OriginalRuntime,
                                        &mut hosted::Model,
                                    )
                                        -> Result<(), String>,
                            ),
                        },
                        &mapping,
                        observe.as_ref(),
                        &r.inputs,
                    )
                }
                None => original::run::<OriginalRuntime, hosted::Model>(
                    &load,
                    Subject::Original,
                    &mapping,
                    observe.as_ref(),
                    &r.inputs,
                ),
            }
            .map_err(anyhow::Error::msg)?
        };
        let subject_model = &subject;
        let subject = (args.runtime != RuntimeArg::None).then_some(&runs.subject);
        let mut outcomes = gp_validate::evaluate::evaluate(&plan, &model, &runs.original, subject);
        if subject.is_some()
            && let Some(src) = &subject_model
        {
            outcomes.extend(slot_outcomes(&plan, &model, &mapping, rom, r, src)?);
        }
        let result = RecordingResult {
            validate_version: gp_validate::VALIDATE_VERSION,
            key: key.to_hex(),
            recording: r.id.clone(),
            name: r.name.clone(),
            subject: subject_id.clone(),
            frames: runs.original.len().saturating_sub(1) as u64,
            outcomes,
        };
        let scope = runs.scope_end.map_or_else(String::new, |f| {
            format!("; out of the mapping's scope from the original's frame {f}")
        });
        let line = format!(
            "evaluated {}  ({} model frames; {} frames of the original before the model starts, {} lag frames{scope})",
            r.name, result.frames, runs.before_start, runs.lag_frames
        );
        Ok((result, line))
    };
    let work: Vec<(usize, Hash, std::sync::Arc<Vec<u8>>)> = todo
        .iter()
        .enumerate()
        .filter_map(|(i, t)| match t {
            Todo::Run(k, rom) => Some((i, *k, rom.clone())),
            _ => None,
        })
        .collect();
    let mut done = crate::pool::map(
        &work,
        || (),
        |_, _, (i, key, rom)| one(&runs[*i], *key, rom),
        &mut |_| {},
    )
    .into_iter();
    let (mut results, mut lines) = (Vec::new(), Vec::new());
    for (r, t) in runs.iter().zip(todo) {
        match t {
            Todo::Stored(found) => {
                lines.push(format!("stored    {}", r.name));
                results.push(found);
            }
            Todo::Skip => {
                lines.push(format!(
                    "skip      {}  (no ROM with its hash in {})",
                    r.name,
                    paths.roms.display()
                ));
            }
            Todo::Run(..) => {
                let (result, line) = done.next().expect("a result for every run")?;
                if let Some(store) = store.as_ref().filter(|_| save) {
                    cache::save(store, &result, &now_rfc3339())?;
                }
                lines.push(line);
                results.push(result);
            }
        }
    }

    // Reachability is a check of the model alone (gp_validate::reach), run
    // once beside the recordings' results.
    if !results.is_empty()
        && gp_validate::reach::wanted(&plan)
        && let Some(src) = &subject
    {
        let outcomes = gp_validate::reach::outcomes(&plan, &src.spec, src);
        lines.push(format!(
            "searched  reachability of {} levels by the player's moves",
            outcomes.len()
        ));
        results.push(RecordingResult {
            validate_version: gp_validate::VALIDATE_VERSION,
            key: String::new(),
            recording: "model".into(),
            name: "reachability".into(),
            subject: subject_id.clone(),
            frames: 0,
            outcomes,
        });
    }
    let summary = gp_validate::summarize(&results);
    let count = |s: Status| summary.iter().filter(|c| c.status == s).count();
    let (passed, failed, unexercised) = (
        count(Status::Pass),
        count(Status::Fail),
        count(Status::Unexercised),
    );
    let exit = if results.is_empty() {
        2
    } else {
        u8::from(failed > 0)
    };
    let mut text = render(
        &model,
        dir,
        &plan,
        &summary,
        subject_id.as_deref(),
        results.len(),
    );
    text.push_str("\nrecordings:\n  ");
    text.push_str(&lines.join("\n  "));
    if sealed > 0 {
        text.push_str(&format!(
            "\n  left out {sealed} recording{} that play{} a sealed run (the coordinator's, docs/contributing-mechanics.md)",
            if sealed == 1 { "" } else { "s" },
            if sealed == 1 { "s" } else { "" },
        ));
    }
    if results.is_empty() {
        text.push_str("\nNo recording could be run, so nothing was validated.");
    }
    let unchecked_json: serde_json::Map<String, serde_json::Value> = plan
        .unchecked
        .iter()
        .map(|(l, facts)| (l.name().to_owned(), json!(facts)))
        .collect();
    Ok(Outcome {
        json: json!({
            "model": model.model.name,
            "subject": subject_id,
            "original": original_id,
            "ok": exit == 0,
            "passed": passed,
            "failed": failed,
            "unexercised": unexercised,
            "checks": summary,
            "unchecked": unchecked_json,
            "recordings": results,
            "sealed_left_out": sealed,
        }),
        text,
        exit,
    })
}

/// The counters each of the model's `ext` hooks may write, by counter: a
/// hook's description names them (`counters` for all, `counters.<c>` for
/// one), and a hook that describes nothing may write any.
fn kept_by_hooks(
    model: &Model,
    exts: &gp_sim::ext::Exts,
) -> std::collections::BTreeMap<String, Vec<String>> {
    let mut out: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    let hooks: std::collections::BTreeSet<&String> = model
        .entities
        .values()
        .filter_map(|e| e.core.ext.as_ref())
        .collect();
    for hook in hooks {
        let records = exts.get(hook).map(|h| h.describe()).unwrap_or_default();
        let writes: Vec<&str> = records
            .iter()
            .flat_map(|r| r.writes.iter().copied())
            .collect();
        let all = records.is_empty() || writes.contains(&"counters");
        for name in model.counters.keys() {
            let one = format!("counters.{name}");
            if all || writes.contains(&one.as_str()) {
                out.entry(name.clone()).or_default().push(hook.clone());
            }
        }
    }
    out
}

/// The runs `--against` or `--store` name, and how many recordings
/// `--store` left out because they play a sealed run (the store's guard
/// refuses them; `--against` names one, and the refusal stops the run).
fn runs(
    args: &ValidateArgs,
    store: Option<&Store>,
    model: &Model,
    rom_sha: Option<&str>,
) -> Result<(Vec<Run>, usize)> {
    let mut out: Vec<Run> = Vec::new();
    let mut sealed = 0;
    let need = || store.context("recordings need a store");
    if args.store {
        let store = need()?;
        let want = rom_sha.map(str::to_owned).or_else(|| {
            crate::open::load_registry()
                .ok()?
                .by_id(model.model.source.as_deref()?)
                .map(|s| s.sha256.clone())
        });
        let listing = recordings::list_checked(store)?;
        let retired = recordings::retired(&listing.marks);
        for e in recordings::unique(listing.entries) {
            if retired.contains(&e.id) {
                continue;
            }
            let m = match recordings::load_manifest(store, &e.id) {
                Err(e) if crate::sealed::is_refusal(&e) => {
                    sealed += 1;
                    continue;
                }
                m => m?,
            };
            // Model recordings aren't recordings of the original.
            let SourceRef::NesRom { sha256, .. } = &m.source else {
                continue;
            };
            if want.as_deref().is_some_and(|w| w != sha256) {
                continue;
            }
            out.push(recording_run(store, e, m)?);
        }
        return Ok((out, sealed));
    }
    for a in &args.against {
        let p = Path::new(a);
        if p.is_dir() {
            for f in baseline::list(p)? {
                out.push(baseline_run(&f)?);
            }
        } else if p.extension().is_some_and(|x| x == "toml") {
            out.push(baseline_run(p)?);
        } else {
            let store = need()?;
            let e = recordings::resolve(store, a)?;
            if out.iter().any(|r| r.id == e.id.to_hex()) {
                continue;
            }
            let m = recordings::load_manifest(store, &e.id)?;
            out.push(recording_run(store, e, m)?);
        }
    }
    Ok((out, sealed))
}

fn baseline_run(file: &Path) -> Result<Run> {
    let b = Baseline::load(file)?;
    let log = crate::sealed::read_baseline_input(file)?;
    let text = read(file)?;
    Ok(Run {
        name: file
            .file_stem()
            .map_or_else(|| "baseline".into(), |s| s.to_string_lossy().into_owned()),
        id: file.display().to_string(),
        content: format!("baseline blake3:{}", Hash::of(text.as_bytes())),
        sha256: b.sha256,
        power_on: PowerOn::default(),
        inputs: inputs_of(&log),
        log: log.frames.clone(),
    })
}

fn recording_run(
    store: &Store,
    e: recordings::IndexEntry,
    m: gp_evidence::RecordingManifest,
) -> Result<Run> {
    let text = String::from_utf8(store.get(&m.input_log)?).context("the input log isn't text")?;
    let log = InputLog::parse(&text)?;
    let SourceRef::NesRom { sha256, .. } = m.source else {
        bail!(
            "{} isn't a recording of an NES ROM (a model's run, or an arcade romset's, which gp validate doesn't read yet)",
            e.name
        );
    };
    Ok(Run {
        name: e.name,
        id: e.id.to_hex(),
        content: format!("recording blake3:{}", e.id),
        sha256,
        power_on: m.power_on,
        inputs: inputs_of(&log),
        log: log.frames.clone(),
    })
}

fn render(
    model: &Model,
    dir: &Path,
    plan: &Plan,
    summary: &[Summary],
    subject: Option<&str>,
    recordings: usize,
) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{} ({}) against {recordings} recordings of the original; subject: {}",
        model.model.title,
        dir.display(),
        subject.unwrap_or("none")
    );
    let count = |s: Status| summary.iter().filter(|c| c.status == s).count();
    let facts: Vec<String> = plan.facts.iter().map(|(l, n)| format!("{l} {n}")).collect();
    let unchecked: Vec<String> = plan
        .unchecked
        .iter()
        .map(|(l, f)| format!("{l} {}", f.len()))
        .collect();
    let _ = writeln!(
        out,
        "checks: {} passed, {} failed, {} unexercised; facts by level: {}; facts no check measures: {}",
        count(Status::Pass),
        count(Status::Fail),
        count(Status::Unexercised),
        facts.join(", "),
        if unchecked.is_empty() {
            "none".to_owned()
        } else {
            unchecked.join(", ")
        }
    );
    for s in summary {
        let word = match s.status {
            Status::Pass => "pass",
            Status::Fail => "FAIL",
            Status::Unexercised => "none",
        };
        let what = match &s.case {
            Some(c) => format!("{} {} ({c})", s.part, s.quantity),
            None => format!("{} {}", s.part, s.quantity),
        };
        let vs = match s.against {
            Against::Model => "vs model",
            Against::Original => "vs original",
        };
        let e = &s.example;
        let mut detail = match (&e.expected, &e.actual) {
            (Some(x), Some(a)) if s.status == Status::Fail => format!("expected {x}, got {a}"),
            (Some(x), Some(a)) if x == a => x.clone(),
            (Some(x), Some(a)) => format!("{a} for {x}"),
            _ => String::new(),
        };
        if let Some(t) = &e.tolerance {
            let _ = write!(detail, " (within {t})");
        }
        if s.status == Status::Fail {
            let _ = write!(detail, " in {}", s.from);
            if let Some(f) = e.frame {
                let _ = write!(detail, " at frame {f}");
            }
        } else if s.status == Status::Pass {
            let _ = write!(
                detail,
                "; {} samples in {} recordings",
                s.samples, s.recordings
            );
        }
        if let Some(d) = &e.detail {
            let _ = write!(detail, "; {d}");
        }
        let _ = writeln!(
            out,
            "  {word}  {:<10} {what:<34} {vs:<11} {}",
            s.level,
            detail.trim_start_matches("; ").trim()
        );
    }
    out.trim_end().to_owned()
}

/// The slot checks on one recording: the original run again, each slot
/// life beside one body of its entity (gp_interp::compare::compare_slots).
/// The bodies are stepped by the hooks gp has for the model; a model whose
/// hooks run only in its module leaves its slot checks unmeasured, with
/// the reason.
fn slot_outcomes(
    plan: &Plan,
    model: &gp_model::Model,
    mapping: &Mapping,
    rom: &[u8],
    r: &Run,
    src: &Source,
) -> Result<Vec<gp_validate::evaluate::Outcome>> {
    if mapping.slots.is_none() || !plan.checks.iter().any(|c| c.kind == compile::Kind::Slots) {
        return Ok(Vec::new());
    }
    let mut nes = NesRuntime::load(rom, r.power_on).map_err(anyhow::Error::msg)?;
    let mut rows = gp_validate::slots::Rows::default();
    let reports = match src.native_exts() {
        Some(exts) => gp_interp::compare::compare_slots(
            &mut nes,
            mapping,
            &r.inputs,
            &src.spec,
            exts,
            &mut |row| rows.push(row),
        )
        .map_err(anyhow::Error::msg)?,
        None => {
            return Ok(gp_validate::slots::unmeasured(
                plan,
                model,
                "the slot checks step the bodies by the model's hooks in gp, and this model's hooks run only in its module",
            ));
        }
    };
    Ok(gp_validate::slots::evaluate(plan, model, &reports, &rows))
}
