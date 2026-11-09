//! The scoring of a corpus clip: each scenario's lockstep of the original
//! in gp-nes and the model through its module (or the built-in interpreter
//! for a hookless model), from the original's state at one frame of the
//! clip's own input, with a tape of the original's side kept
//! (`crate::tape`) and each score cached by everything it reads.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use base64::Engine as _;
use serde::{Deserialize, Serialize};

use gp_evidence::Hash;
use gp_evidence::manifest::PowerOn;
use gp_interp::Lifted;
use gp_interp::lockstep::{self, Start};
use gp_interp::mapping::{MAPPING_FILE, Mapping};
use gp_interp::reads::{ReadSet, SpecNames};
use gp_interp::scenario::{self, Outcome as Ran, Quiet, Scenario, StartAt};
use gp_model::{MODEL_FILE, Model};
use gp_nes::protocol::NesRuntime;
use gp_runtime::{InputFrame, LoadHow, Runtime};
use gp_sim::Fx;

use crate::hosted::Source;

/// Everything a ladder reads but the model: the working copy, the
/// mapping, the assets, the ROM and the input.
#[derive(Clone)]
pub(crate) struct Bench {
    /// The model directory (a working copy for `--refine` and `--auto`).
    pub dir: PathBuf,
    pub mapping_text: String,
    pub mapping: Mapping,
    pub assets: PathBuf,
    pub lifted_bytes: Vec<u8>,
    /// The model as it compiles and runs: the module or the interpreter.
    pub hosted: Arc<Source>,
    pub rom: Vec<u8>,
    pub log_text: String,
    pub inputs: Vec<InputFrame>,
    /// The input as a start names it: a recording's ID, a baseline file
    /// or a capture run's recording name.
    pub input: String,
    /// Where results are cached.
    pub cache: PathBuf,
    /// Record what each clip's run reads of the spec and which hooks it
    /// calls (`Outcome::reads`): the host's own reads and each module run's.
    pub reads: bool,
    /// The clips to stop at their first split: clips the scorer knows the
    /// model played exactly before, so a split decides it.
    pub stop: BTreeSet<String>,
}

/// A start taken: the frame asked for or found, the start, and why it is
/// quiet when it was found.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Started {
    pub asked: Option<u64>,
    pub original_frame: u64,
    pub frame: u64,
    pub state: String,
    pub loaded: usize,
    pub approximated: usize,
    pub missing: usize,
    pub quiet: Option<Quiet>,
    /// What taking the start read of the spec, when the bench records reads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reads: Option<ReadSet>,
}

impl Started {
    /// No start of the bench's own: for a list whose scenarios all start
    /// from their own input (free play clips), which never read it.
    pub fn none() -> Self {
        Self {
            asked: None,
            original_frame: u64::MAX,
            frame: 0,
            state: String::new(),
            loaded: 0,
            approximated: 0,
            missing: 0,
            quiet: None,
            reads: None,
        }
    }

    fn start(&self) -> Result<Start> {
        Ok(Start {
            frame: self.frame,
            original_frame: self.original_frame,
            notes: Vec::new(),
            state: base64::engine::general_purpose::STANDARD.decode(&self.state)?,
        })
    }
}

/// `f`, with what it reads of the spec noted when `on`.
fn noted<R>(on: bool, f: impl FnOnce() -> R) -> (R, gp_sim::reads::Reads) {
    if on {
        gp_sim::reads::recording(f)
    } else {
        (f(), gp_sim::reads::Reads::default())
    }
}

/// A module run's reads by name.
fn run_reads(m: &mut crate::hosted::Model, names: &SpecNames) -> Result<Option<ReadSet>> {
    Ok(m.take_reads()?.map(|r| ReadSet::of(&r, names)))
}

fn key(parts: &[&[u8]]) -> String {
    let mut k = Vec::new();
    for p in parts {
        k.extend_from_slice(&(p.len() as u64).to_le_bytes());
        k.extend_from_slice(p);
    }
    let h = Hash::of(&k).to_string();
    h[h.len() - 16..].to_owned()
}

fn read_cache<T: serde::de::DeserializeOwned>(file: &Path) -> Option<T> {
    fs::read(file)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
}

fn write_cache<T: Serialize>(file: &Path, v: &T) {
    if let Some(p) = file.parent() {
        let _ = fs::create_dir_all(p);
    }
    // A cache that can't be written is only slower next time. Each
    // thread writes its own temporary file, so two threads scoring the
    // same scenario never mix their bytes; the rename is atomic.
    let tmp = file.with_extension(
        format!("json.tmp-{:?}", std::thread::current().id()).replace(['(', ')'], ""),
    );
    if fs::write(&tmp, serde_json::to_vec(v).unwrap_or_default()).is_ok() {
        let _ = fs::rename(&tmp, file);
    }
}

impl Bench {
    /// A bench on a model directory, an input and a ROM.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        dir: &Path,
        assets: &Path,
        rom: Vec<u8>,
        log_text: String,
        input: &str,
        cache: &Path,
        hosted: Arc<Source>,
    ) -> Result<Self> {
        let mapping_text = fs::read_to_string(dir.join(MAPPING_FILE)).with_context(|| {
            format!(
                "{} has no state mapping, so the two games can't be measured apart",
                dir.display()
            )
        })?;
        let mapping = Mapping::parse(&mapping_text).map_err(anyhow::Error::msg)?;
        let log = gp_evidence::InputLog::parse(&log_text)?;
        let inputs = crate::run::frames(&log);
        let mut b = Self {
            dir: dir.to_owned(),
            mapping_text,
            mapping,
            assets: assets.to_owned(),
            lifted_bytes: Vec::new(),
            hosted,
            rom,
            log_text,
            inputs,
            input: input.to_owned(),
            cache: cache.to_owned(),
            reads: false,
            stop: BTreeSet::new(),
        };
        let (m, _) = b.model()?;
        b.lifted_bytes = gp_interp::Bundle::new(&m, b.lifted(&m)?).to_bytes();
        Ok(b)
    }

    /// The model as it stands in the directory, and its text.
    pub fn model(&self) -> Result<(Model, String)> {
        let f = self.dir.join(MODEL_FILE);
        let text = fs::read_to_string(&f).with_context(|| format!("reading {}", f.display()))?;
        Ok((
            Model::parse(&text).with_context(|| f.display().to_string())?,
            text,
        ))
    }

    pub(crate) fn lifted(&self, m: &Model) -> Result<Lifted> {
        Lifted::read_dir(&self.assets, m).map_err(|e| {
            anyhow::anyhow!(
                "{e}\n(lift the model's assets with gp assets lift, or name them with --assets)"
            )
        })
    }

    fn contract(&self) -> Vec<u8> {
        fs::read(self.dir.join(gp_contract::CONTRACT_FILE)).unwrap_or_default()
    }

    /// The key of everything but the model a result reads.
    fn base_key(&self) -> Vec<u8> {
        let mut k = Vec::new();
        for p in [
            self.mapping_text.as_bytes(),
            &self.contract(),
            &self.rom,
            self.log_text.as_bytes(),
            env!("CARGO_PKG_VERSION").as_bytes(),
            scenario::SCENARIO_VERSION.as_bytes(),
            self.hosted.hooks.as_bytes(),
            // A result with its reads is kept apart from one without.
            if self.reads { b"reads 1" } else { b"" },
        ] {
            k.extend_from_slice(&(p.len() as u64).to_le_bytes());
            k.extend_from_slice(p);
        }
        k
    }

    /// The bench on a scenario's own input, when its start names an input
    /// log file (a free play save point's): the same model, ROM and
    /// cache, with that log's frames played to the start.
    pub fn for_start(&self, start: Option<&StartAt>) -> Result<Option<Bench>> {
        let Some((st, path)) = start.and_then(|s| s.log.as_deref().map(|l| (s, l))) else {
            return Ok(None);
        };
        let text = fs::read_to_string(path)
            .with_context(|| format!("reading the start's input log {path}"))?;
        let log = gp_evidence::InputLog::parse(&text)?;
        let mut b = self.clone();
        b.inputs = crate::run::frames(&log);
        b.log_text = text;
        b.input.clone_from(&st.input);
        Ok(Some(b))
    }

    /// The original standing at `frame` of the input.
    /// The original at its frame `frame` of the input, with video on from
    /// its last frame, so the picture it shows there is drawn for the screen
    /// measure.
    fn original_at(&self, frame: u64) -> Result<NesRuntime> {
        let mut o = NesRuntime::load(&self.rom, PowerOn::default()).map_err(anyhow::Error::msg)?;
        let n = usize::try_from(frame)
            .unwrap_or(usize::MAX)
            .min(self.inputs.len());
        for (k, i) in self.inputs[..n].iter().enumerate() {
            if k + 1 == n {
                o.set_video(true);
            }
            o.step(std::slice::from_ref(i))
                .map_err(anyhow::Error::msg)?;
        }
        o.set_video(true);
        Ok(o)
    }

    /// The original's snapshot a start names, when it has one at its own
    /// frame made by this gp-nes: the original standing there without
    /// playing the input to it.
    fn snapshot_of(&self, start: &StartAt) -> Option<NesRuntime> {
        let path = start.snapshot.as_deref()?;
        let bytes = fs::read(path).ok()?;
        let mut o = NesRuntime::load(&self.rom, PowerOn::default()).ok()?;
        o.restore(&bytes).ok()?;
        (o.frame() == start.frame).then_some(o)
    }

    /// The original standing at `frame`: from the start's snapshot when
    /// it has one there, else the input played to it.
    fn original_from(&self, start: Option<&StartAt>, frame: u64) -> Result<NesRuntime> {
        if let Some(st) = start.filter(|st| st.frame == frame)
            && let Some(o) = self.snapshot_of(st)
        {
            return Ok(o);
        }
        self.original_at(frame)
    }

    /// The start a scenario carries: from its snapshot and the model's
    /// frame there when it names both (a generated clip's), which is the
    /// start [`Bench::started`] takes by playing the input to the frame;
    /// else that.
    pub fn started_at(&self, model: &Model, text: &str, own: &StartAt) -> Result<Started> {
        let (Some(mf), Some(snap)) = (own.model_frame, own.snapshot.as_deref()) else {
            return self.started(model, text, Some(own.frame));
        };
        let snap_bytes = fs::read(snap).unwrap_or_default();
        let k = key(&[
            &self.base_key(),
            text.as_bytes(),
            &self.lifted_bytes,
            b"snapshot",
            &snap_bytes,
            mf.to_string().as_bytes(),
        ]);
        let file = self.cache.join(format!("start-{k}.json"));
        if let Some(s) = read_cache::<Started>(&file)
            && (!self.reads || s.reads.is_some())
        {
            return Ok(s);
        }
        let Some(o) = self.snapshot_of(own) else {
            return self.started(model, text, Some(own.frame));
        };
        let (s, got) = noted(self.reads, || self.started_from(mf, own, &o));
        let s = self.with_host_reads(s?, &got);
        write_cache(&file, &s);
        Ok(s)
    }

    /// `s`, its reads the module's it was given and the host's noted here.
    fn with_host_reads(&self, mut s: Started, got: &gp_sim::reads::Reads) -> Started {
        if self.reads {
            let mut all = ReadSet::of(got, &SpecNames::of(&self.hosted.spec));
            if let Some(m) = s.reads.take() {
                all.union(&m);
            }
            s.reads = Some(all);
        }
        s
    }

    /// A new run of the model, its module reads noted when the bench
    /// records them.
    fn open(&self) -> Result<crate::hosted::Model> {
        let mut m = self.hosted.open()?;
        if self.reads {
            m.start_reads()?;
        }
        Ok(m)
    }

    /// The start from the original's snapshot `o` at `own`'s frame, the
    /// model's frame there `mf`.
    fn started_from(&self, mf: u64, own: &StartAt, o: &NesRuntime) -> Result<Started> {
        let mut interp = self.open()?;
        let spec = interp.spec().clone();
        let mut base = gp_sim::World::power_on(&spec);
        base.frame = mf;
        let t = gp_interp::translate::from_original(o, &spec, &base, &self.mapping)
            .map_err(anyhow::Error::msg)?;
        let mut notes = t.notes.clone();
        notes.extend(
            interp
                .load_state(&t.state())
                .map_err(|e| anyhow::anyhow!("the model: {e}"))?,
        );
        let count = |h: LoadHow| notes.iter().filter(|n| n.how == h).count();
        let names = SpecNames::of(&spec);
        Ok(Started {
            reads: if self.reads {
                run_reads(&mut interp, &names)?
            } else {
                None
            },
            asked: Some(own.frame),
            original_frame: own.frame,
            frame: interp.frame(),
            state: base64::engine::general_purpose::STANDARD.encode(interp.canonical_state()),
            loaded: count(LoadHow::Loaded),
            approximated: count(LoadHow::Approximated),
            missing: count(LoadHow::Missing),
            quiet: None,
        })
    }

    /// The start at the original's frame `at`, or at a quiet frame found
    /// in the input, translated through the model as it stands. The
    /// start's state depends on the model only through its spec, so it is
    /// keyed by the model's text too.
    pub fn started(&self, model: &Model, text: &str, at: Option<u64>) -> Result<Started> {
        let k = key(&[
            &self.base_key(),
            text.as_bytes(),
            &self.lifted_bytes,
            at.map(|a| a.to_string()).unwrap_or_default().as_bytes(),
        ]);
        let file = self.cache.join(format!("start-{k}.json"));
        if let Some(s) = read_cache::<Started>(&file)
            && (!self.reads || s.reads.is_some())
        {
            return Ok(s);
        }
        let (s, got) = noted(self.reads, || self.started_fresh(model, at));
        let s = self.with_host_reads(s?, &got);
        write_cache(&file, &s);
        Ok(s)
    }

    /// [`Bench::started`] taken again.
    fn started_fresh(&self, model: &Model, at: Option<u64>) -> Result<Started> {
        let mut interp = self.open()?;
        let spec = interp.spec().clone();
        let power = NesRuntime::load(&self.rom, PowerOn::default()).map_err(anyhow::Error::msg)?;
        let quiet = match at {
            Some(_) => None,
            None => {
                let player = scenario::player(model).context(
                    "the model has no player (an entity with role `player`), so it has no scenarios",
                )?;
                let rate = (model.model.frame_rate.num, model.model.frame_rate.den);
                Some(
                    scenario::quiet_start(
                        &power,
                        &spec,
                        &self.mapping,
                        &self.inputs,
                        &player,
                        0,
                        rate,
                    )
                    .map_err(anyhow::Error::msg)?
                    .context(
                        "no frame of the input has the player on the ground with nothing near; give one with --at",
                    )?,
                )
            }
        };
        let frame = at.or(quiet.as_ref().map(|q| q.frame)).unwrap_or(0);
        let (_, start) = lockstep::start_at(
            &power,
            &mut interp,
            &spec,
            &self.mapping,
            &self.inputs,
            frame,
        )
        .map_err(anyhow::Error::msg)?;
        let count = |h: LoadHow| start.notes.iter().filter(|n| n.how == h).count();
        let names = SpecNames::of(&spec);
        Ok(Started {
            reads: if self.reads {
                run_reads(&mut interp, &names)?
            } else {
                None
            },
            asked: at,
            original_frame: start.original_frame,
            frame: start.frame,
            state: base64::engine::general_purpose::STANDARD.encode(&start.state),
            loaded: count(LoadHow::Loaded),
            approximated: count(LoadHow::Approximated),
            missing: count(LoadHow::Missing),
            quiet,
        })
    }
}

/// What every scenario a score reads but its own: the catalog, the
/// tolerances, the key of the rest and the model's pictures. Read once a
/// list, shared by its threads.
struct Shared {
    catalog: gp_interp::mechanics::Catalog,
    tolerance: Vec<Fx>,
    base: String,
    /// The model's pictures for the screen measure, decoded by the first
    /// scenario not cached while the others wait.
    art: std::sync::OnceLock<Option<Arc<crate::screen::Art>>>,
    /// The spec's names, for each clip's reads.
    names: SpecNames,
}

/// What a thread keeps between its scenarios: the original at each start
/// it took.
#[derive(Default)]
struct Mine {
    originals: BTreeMap<(String, u64), (NesRuntime, Start)>,
}

/// Scores scenarios against `model`, each its own result, cached unless
/// `cache` is false. The scenarios run on the helper threads that are free
/// (crate::pool), and the results are in the list's order, so they are the
/// same for any number of threads. Each thread takes the original at a
/// start once, when a scenario from it isn't cached. `progress` hears each
/// scenario as it starts: its index, the count, and it.
pub(crate) fn score_each(
    b: &Bench,
    model: &Model,
    text: &str,
    started: &Started,
    list: &[Scenario],
    cache: bool,
    progress: &mut dyn FnMut(usize, usize, &Scenario),
) -> Result<Vec<Result<Ran>>> {
    let (tolerance, _): (Vec<Fx>, _) = crate::lockstep::tolerances(&b.dir, model, &b.mapping);
    let shared = Shared {
        catalog: gp_interp::mechanics::catalog(model, &b.hosted.exts),
        tolerance,
        // The screen measure's version is in the key: a result from before
        // it has no screen score.
        base: key(&[
            &b.base_key(),
            text.as_bytes(),
            &b.lifted_bytes,
            b"screen 1",
            b"stretches 2",
        ]),
        art: std::sync::OnceLock::new(),
        names: if b.reads {
            SpecNames::of(&b.hosted.spec)
        } else {
            SpecNames::default()
        },
    };
    // The costliest first, so no thread is left with a long one at the
    // end: a start with no snapshot plays the input to it from power-on.
    let cost = |s: &Scenario| {
        let lead = s
            .start
            .as_ref()
            .filter(|st| st.snapshot.is_none())
            .map_or(0, |st| st.frame);
        lead + s.frames
    };
    let mut order: Vec<usize> = (0..list.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(cost(&list[i])));
    let ran = crate::pool::map(
        &order,
        Mine::default,
        |mine, _, &i| score_one(b, model, text, started, &shared, mine, &list[i], cache),
        &mut |k| progress(order[k], list.len(), &list[order[k]]),
    );
    // Back in the list's order.
    let mut out: Vec<Option<Result<Ran>>> = Vec::new();
    out.resize_with(list.len(), || None);
    for (i, r) in order.into_iter().zip(ran) {
        out[i] = Some(r);
    }
    Ok(out
        .into_iter()
        .map(|r| r.expect("every scenario ran"))
        .collect())
}

#[allow(clippy::too_many_arguments)]
fn score_one(
    b: &Bench,
    model: &Model,
    text: &str,
    started: &Started,
    shared: &Shared,
    mine: &mut Mine,
    s: &Scenario,
    cache: bool,
) -> Result<Ran> {
    // A clip runs on its save point's input; the rest on the ladder's.
    let own_input = b.for_start(s.start.as_ref())?;
    let sb = own_input.as_ref().unwrap_or(b);
    let st = match &s.start {
        Some(own) if own_input.is_some() || own.frame != started.original_frame => {
            // A played scenario from its own frame of its input.
            sb.started_at(model, text, own)?
        }
        _ => started.clone(),
    };
    let sj = serde_json::to_vec(s)?;
    let own_log = own_input
        .as_ref()
        .map_or(&[][..], |o| o.log_text.as_bytes());
    let file = b.cache.join(format!(
        "{}.json",
        key(&[
            shared.base.as_bytes(),
            st.original_frame.to_string().as_bytes(),
            &sj,
            own_log,
        ])
    ));
    if cache
        && let Some(r) = read_cache::<Ran>(&file)
        && (!b.reads || r.reads.is_some())
    {
        return Ok(r);
    }
    let stop = b.stop.contains(&s.id);
    let module = std::cell::RefCell::new(ReadSet::default());
    let (r, got) = noted(b.reads, || {
        taped_one(b, sb, &st, model, shared, mine, s, cache, stop, &module)
    });
    let mut r = r?;
    if b.reads {
        // The clip's reads, those of each of its module runs, and its
        // start's; a start with none recorded leaves the clip's unknown,
        // so any change scores it again.
        r.reads = st.reads.as_ref().map(|from| {
            let mut all = ReadSet::of(&got, &shared.names);
            all.union(&module.borrow());
            all.union(from);
            all
        });
    }
    // A run stopped at its split covers part of the clip: not the score.
    if cache && !r.stopped {
        write_cache(&file, &r);
    }
    Ok(r)
}

/// The file of a scenario's tape: the original's side of its lockstep
/// (`crate::tape`), keyed by the ROM, the input played to its start, the
/// scenario (its start, snapshot and presses), the original's frame there
/// and the emulator's sources.
fn tape_file(b: &Bench, sb: &Bench, st: &Started, s: &Scenario) -> Result<PathBuf> {
    let snap = s
        .start
        .as_ref()
        .and_then(|x| x.snapshot.as_deref())
        .and_then(|p| fs::read(p).ok())
        .unwrap_or_default();
    Ok(b.cache.join(format!(
        "tape-{}.lz4",
        key(&[
            &crate::tape::key_parts(),
            &b.rom,
            sb.log_text.as_bytes(),
            &serde_json::to_vec(s)?,
            &snap,
            st.original_frame.to_string().as_bytes(),
        ])
    )))
}

/// The original's random draws over a scenario, from its start (mapping
/// feature `draws.score`): what the rebuild takes at its draw sites in
/// place of its own generator's, so a clip makes the original's random
/// choices. Kept beside the scenario's tape, by the same key.
fn draws_of(
    b: &Bench,
    sb: &Bench,
    st: &Started,
    s: &Scenario,
    mine: &mut Mine,
    tape: &Path,
    cache: bool,
) -> Result<gp_sim::DrawScript> {
    // The draws depend on the mapping's draw sites as well as the tape's
    // run: a site added later (Donkey Kong's fires' `fire_way`) needs a
    // script that has it.
    let sites = key(&[format!("{:?}", b.mapping.draws).as_bytes()]);
    let file = tape.with_extension(format!("{sites}.draws"));
    if cache
        && let Ok(t) = fs::read_to_string(&file)
        && let Ok(d) = gp_sim::DrawScript::parse(&t)
    {
        return Ok(with_base(d, st.original_frame));
    }
    let at = (sb.input.clone(), st.original_frame);
    if let std::collections::btree_map::Entry::Vacant(e) = mine.originals.entry(at.clone()) {
        e.insert((
            sb.original_from(s.start.as_ref(), st.original_frame)?,
            st.start()?,
        ));
    }
    let nes = mine.originals[&at].0.fork().into_nes();
    let inputs: Vec<gp_evidence::input_log::InputFrame> = s
        .inputs()
        .map_err(anyhow::Error::msg)?
        .iter()
        .map(|f| gp_evidence::input_log::InputFrame {
            polled: false,
            pads: f.buttons.to_le_bytes(),
            reset: f.reset,
        })
        .collect();
    let d = crate::lockstep::record_draws_on(nes, &b.mapping, &inputs, st.original_frame)?;
    if cache {
        let _ = fs::write(&file, d.text());
    }
    Ok(with_base(d, st.original_frame))
}

fn with_base(mut d: gp_sim::DrawScript, base: u64) -> gp_sim::DrawScript {
    d.base = base;
    d
}

/// One scenario's lockstep for its score: the original's side from its
/// tape when it has one, so only the rebuilt game runs; else from the
/// emulator, writing the tape (when `cache`). A tape that misses anything
/// the run asks is put aside and the emulator runs.
#[allow(clippy::too_many_arguments)]
fn taped_one(
    b: &Bench,
    sb: &Bench,
    st: &Started,
    model: &Model,
    shared: &Shared,
    mine: &mut Mine,
    s: &Scenario,
    cache: bool,
    stop: bool,
    module: &std::cell::RefCell<ReadSet>,
) -> Result<Ran> {
    use crate::tape::Original;
    let file = tape_file(b, sb, st, s)?;
    let draws = if b.mapping.score_draws {
        Some(draws_of(b, sb, st, s, mine, &file, cache)?)
    } else {
        None
    };
    let note = |m: &mut crate::hosted::Model| -> Result<()> {
        if b.reads
            && let Some(r) = run_reads(m, &shared.names)?
        {
            module.borrow_mut().union(&r);
        }
        Ok(())
    };
    let run = |o: &Original, start: &Start| -> Result<Ran> {
        let mut interp = b.open()?;
        if let Some(d) = &draws {
            interp = interp.with_draws(d.clone())?;
        }
        let spec = interp.spec().clone();
        // The pictures are the whole model's, decoded by whichever clip
        // comes first: not that clip's reads (the scorer compares them
        // whole).
        let art = gp_sim::reads::paused(|| {
            shared
                .art
                .get_or_init(|| crate::screen::art(model, &spec, &b.assets).ok())
                .clone()
        });
        let mut probe = art.map(|a| crate::screen::Probe::new(a, crate::screen::nes_options()));
        let (mut r, _) = scenario::run_stopping(
            o,
            &mut interp,
            &spec,
            &b.mapping,
            start,
            s,
            &shared.tolerance,
            &shared.catalog,
            probe
                .as_mut()
                .map(|p| p as &mut dyn gp_interp::lockstep::Screens<_, _>),
            stop,
        )
        .map_err(anyhow::Error::msg)?;
        note(&mut interp)?;
        // A long run is scored by its exact stretches too: the same start
        // and presses in resync mode. A run stopped at its split has none.
        if s.frames > lockstep::LONG_RUN && !r.stopped {
            let mut interp = b.open()?;
            if let Some(d) = &draws {
                interp = interp.with_draws(d.clone())?;
            }
            let opts = lockstep::Options {
                mode: lockstep::Mode::Resync,
                tolerance: shared.tolerance.clone(),
                ..lockstep::Options::default()
            };
            let ls = lockstep::lockstep_from(
                &mut o.fork(),
                &mut interp,
                &spec,
                &b.mapping,
                start,
                &s.inputs().map_err(anyhow::Error::msg)?,
                &opts,
            )
            .map_err(anyhow::Error::msg)?;
            note(&mut interp)?;
            r.stretches = Some(lockstep::stretches(&ls));
        }
        Ok(r)
    };
    if cache && let Some(t) = crate::tape::load(&file) {
        let o = Original::replay(t);
        let r = run(&o, &st.start()?);
        if !o.missed() {
            return r;
        }
    }
    let at = (sb.input.clone(), st.original_frame);
    if let std::collections::btree_map::Entry::Vacant(e) = mine.originals.entry(at.clone()) {
        e.insert((
            sb.original_from(s.start.as_ref(), st.original_frame)?,
            st.start()?,
        ));
    }
    let (nes, start) = &mine.originals[&at];
    let o = Original::recording(nes.fork());
    let r = run(&o, start);
    if cache
        && r.is_ok()
        && !o.missed()
        && let Some(t) = o.into_tape()
    {
        crate::tape::save(&file, &t);
    }
    r
}

pub(crate) fn model_hash(text: &str) -> String {
    let h = Hash::of(text.as_bytes()).to_string();
    h[h.len() - 12..].to_owned()
}

pub(crate) fn resolve_logs(list: &mut [Scenario], home: &Path) {
    for s in list {
        let Some(st) = s.start.as_mut() else { continue };
        for l in [st.log.as_mut(), st.snapshot.as_mut()]
            .into_iter()
            .flatten()
        {
            if Path::new(l.as_str()).is_relative() {
                *l = home.join(&*l).to_string_lossy().into_owned();
            }
        }
    }
}

/// A clip's curve hash: the BLAKE3 of the measures on every frame, the
/// original's or the model's.
pub(crate) fn curve_hash(o: &Ran, model: bool) -> String {
    let mut s = String::new();
    for m in &o.measures {
        s.push_str(&m.path);
        s.push('=');
        let v = if model { &m.model } else { &m.original };
        s.push_str(&v.join(","));
        s.push('\n');
    }
    gp_evidence::Hash::of(s.as_bytes()).to_string()
}
