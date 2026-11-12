//! A model as the tools run it: through its game's module (`--module`), or
//! the built-in interpreter for a model with no hooks. gp links no game's
//! hook crate, so a model that names hooks runs only through its module.
//!
//! `Spec` is compiled here on the host, where no hook runs. The module's own
//! compile of the same bytes must agree with it (`spec_digest`), or the
//! scores of two tool versions wouldn't mean the same thing. The `World`
//! that the screen measure and the other presentation readers need is
//! decoded from the module's canonical state, once per frame it is asked
//! for.

use std::cell::OnceCell;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context as _, Result, anyhow, bail};
use gp_evidence::Paths;
use gp_interp::variant::{self, Edit, Sim, Stepper, Variant};
use gp_interp::{Bundle, Interp, Lifted};
use gp_module::{DrawsReport, ModuleCode, ModuleRuntime};
use gp_runtime::{
    Capabilities, Entry, Event, InputFrame, LoadNote, PatchResult, Rect, Runtime, Until, Value,
};
use gp_sim::ext::Exts;
use gp_sim::{Spec, World};

/// What every run of one model shares: the compiled spec, and how to make
/// a run of it.
pub(crate) struct Source {
    bundle: Bundle,
    bytes: Vec<u8>,
    pub spec: Spec,
    /// The hooks' descriptions: from the module, or the hooks in `gp` itself
    /// for a game with none of its own.
    pub exts: Arc<Exts>,
    /// Set when the model's hooks run in a game's module.
    module: Option<ModuleCode>,
    /// What the hooks are, for caches: the module file's hash, else
    /// `gp`'s own label for them.
    pub hooks: String,
}

impl Source {
    /// A model read from `dir` with its lifted assets in `assets`, its
    /// `$GP_HOME/work/assets/<source>` when `assets` is `None`. A model with
    /// hooks needs `module`; the module's spec is checked against this one.
    pub(crate) fn open_dir(
        dir: &Path,
        assets: Option<&Path>,
        paths: &Paths,
        module: Option<&Path>,
    ) -> Result<(Self, std::path::PathBuf)> {
        let file = dir.join(gp_model::MODEL_FILE);
        let text =
            fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
        let model = gp_model::Model::parse(&text).with_context(|| file.display().to_string())?;
        let assets = match assets {
            Some(a) => a.to_owned(),
            None => {
                let Some(source) = &model.model.source else {
                    bail!("the model names no source, so give its lifted assets with --assets");
                };
                paths.home.join("work").join("assets").join(source)
            }
        };
        let lifted = Lifted::read_dir(&assets, &model).map_err(|e| {
            anyhow!(
                "{e}\n(lift the model's assets with gp assets lift, or name them with --assets)"
            )
        })?;
        let bundle = Bundle::new(&model, lifted);
        let source = Self::new(bundle, module)?;
        Ok((source, assets))
    }

    pub(crate) fn new(bundle: Bundle, module: Option<&Path>) -> Result<Self> {
        Self::new_allowing(bundle, module, false)
    }

    /// [`Self::new`], and with `allow_missing_ext` a model that names hooks
    /// runs with no module in the built-in interpreter, its entities doing
    /// only what their components say (`gp interp run --allow-missing-ext`).
    pub(crate) fn new_allowing(
        bundle: Bundle,
        module: Option<&Path>,
        allow_missing_ext: bool,
    ) -> Result<Self> {
        let interp = Interp::from_bundle(&bundle).map_err(|e| anyhow!("{e}"))?;
        let spec = interp.spec().clone();
        let (exts, label) = gp_exts::registry(&spec.name);
        let exts = Arc::new(exts);
        let missing: Vec<String> = interp
            .with_exts(exts.clone())
            .ext_hooks()
            .iter()
            .map(|h| (*h).to_owned())
            .collect();
        let bytes = bundle.to_bytes();
        let mut module_hash = String::new();
        let mut described = None;
        let module = match module {
            Some(p) => {
                let wasm = fs::read(p).with_context(|| p.display().to_string())?;
                module_hash = blake3::hash(&wasm).to_hex().to_string();
                let code = ModuleCode::compile(&wasm).map_err(|e| anyhow!("{e}"))?;
                let first = code.load(&bytes).map_err(|e| anyhow!("{e}"))?;
                described = Some(
                    gp_interp::describe::read_exts(&first.describe().map_err(|e| anyhow!("{e}"))?)
                        .map_err(|e| anyhow!("{e}"))?,
                );
                let theirs = first.spec_digest().map_err(|e| anyhow!("{e}"))?;
                let ours = gp_sim::save::spec_digest(&spec);
                if theirs != ours {
                    bail!(
                        "the module compiles this model to a different spec: digest {theirs}, here {ours}. Rebuild it from this tool's kit"
                    );
                }
                Some(code)
            }
            None if missing.is_empty() || allow_missing_ext => None,
            None => bail!(
                "the model `{}` names hooks, which run only in its game's module (--module): {}",
                spec.name,
                missing.join(", ")
            ),
        };
        let hooks = match module {
            Some(_) => module_hash,
            None => label,
        };
        let exts = described.map_or(exts, Arc::new);
        Ok(Self {
            bundle,
            bytes,
            spec,
            exts,
            module,
            hooks,
        })
    }

    /// A run of this source's model edited to `m`, started at power-on:
    /// a [`Source`] compiled from it, checked against the module as usual.
    pub(crate) fn with_model(&self, m: &gp_model::Model, module: Option<&Path>) -> Result<Self> {
        Self::new(Bundle::new(m, self.bundle.assets.clone()), module)
    }

    /// This source with its model edited to `m` and the same assets, run
    /// through the module already compiled: the refinement loop's trials
    /// make one for each value they try. The module's spec is checked
    /// against the edited one, as [`Self::new`] does.
    pub(crate) fn edited(&self, m: &gp_model::Model) -> Result<Self> {
        let bundle = Bundle::new(m, self.bundle.assets.clone());
        let interp = Interp::from_bundle(&bundle).map_err(|e| anyhow!("{e}"))?;
        let spec = interp.spec().clone();
        let bytes = bundle.to_bytes();
        if let Some(code) = &self.module {
            let rt = code.load(&bytes).map_err(|e| anyhow!("{e}"))?;
            let theirs = rt.spec_digest().map_err(|e| anyhow!("{e}"))?;
            let ours = gp_sim::save::spec_digest(&spec);
            if theirs != ours {
                bail!(
                    "the module compiles the edited model to a different spec: digest {theirs}, here {ours}"
                );
            }
        }
        Ok(Self {
            bundle,
            bytes,
            spec,
            exts: self.exts.clone(),
            module: self.module.clone(),
            hooks: self.hooks.clone(),
        })
    }

    /// The model and assets as one run keeps them, for keys.
    pub(crate) fn bundle_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The hooks, when they run in gp itself: none for a model whose hooks
    /// run in its module, which `exts` only describes.
    pub(crate) fn native_exts(&self) -> Option<&Exts> {
        self.module.is_none().then_some(&*self.exts)
    }

    pub(crate) fn parse_model(&self) -> Result<gp_model::Model> {
        self.bundle.parse_model().map_err(|e| anyhow!("{e}"))
    }

    /// A new run of the model at power-on.
    pub(crate) fn open(&self) -> Result<Model> {
        match &self.module {
            Some(code) => {
                let rt = code.load(&self.bytes).map_err(|e| anyhow!("{e}"))?;
                Ok(Model::Module(Box::new(Hosted::new(
                    rt,
                    Arc::new(self.spec.clone()),
                ))))
            }
            None => Ok(Model::Native(Box::new(
                Interp::from_bundle(&self.bundle)
                    .map_err(|e| anyhow!("{e}"))?
                    .with_exts(self.exts.clone()),
            ))),
        }
    }
}

/// Opens the searches' spec variants: the module's own session, loaded from
/// the edited model text ([`gp_interp::variant`]), or the built-in
/// interpreter with this model's hooks when it has no module.
impl Sim for Source {
    fn open(&self, base: &Spec, edit: &Edit) -> Result<Variant<'_>, String> {
        let spec = edit.spec(base)?;
        let Some(code) = &self.module else {
            return variant::native(&self.exts, base, edit);
        };
        let model = self.bundle.parse_model()?;
        let text = edit.model(&model, base)?;
        let bundle = Bundle::new(&text, self.bundle.assets.clone());
        let mut rt = code.load(&bundle.to_bytes())?;
        if let Edit::AloneFlat(level) = edit {
            rt.flat_level(*level as u32)?;
        }
        let theirs = rt.spec_digest()?;
        let ours = gp_sim::save::spec_digest(&spec);
        if theirs != ours {
            return Err(format!(
                "the model text of {edit:?} doesn't compile to the edited spec: the module has {theirs}, here {ours}"
            ));
        }
        let events = spec
            .events
            .iter()
            .enumerate()
            .map(|(i, name)| (name.clone(), gp_sim::spec::EventKind(i)))
            .collect();
        Ok(Variant::new(
            spec,
            Box::new(ModuleStepper {
                rt,
                events,
                last: Vec::new(),
            }),
        ))
    }
}

/// Steps a world the host holds through a session of a game's module:
/// `load-state`, `step`, `canonical-state` for each frame, and no load when
/// the world is still the state the module just gave back.
struct ModuleStepper {
    rt: ModuleRuntime,
    events: HashMap<String, gp_sim::spec::EventKind>,
    /// The canonical state the module holds.
    last: Vec<u8>,
}

impl Stepper for ModuleStepper {
    fn step(&mut self, spec: &Spec, w: &mut World, buttons: u8) -> Vec<gp_sim::Event> {
        let state = w.canonical();
        if state != self.last {
            self.rt
                .load_state(&state)
                .unwrap_or_else(|e| panic!("load-state into the module failed: {e}"));
        }
        let input = InputFrame {
            buttons: u32::from(buttons),
            axes: Vec::new(),
            reset: false,
        };
        let events = self
            .rt
            .step(std::slice::from_ref(&input))
            .unwrap_or_else(|e| panic!("a step in the module failed: {e}"));
        self.last = self.rt.canonical_state();
        *w = World::from_canonical(spec, &self.last)
            .unwrap_or_else(|e| panic!("the module's canonical state doesn't decode: {e}"));
        events
            .into_iter()
            .filter_map(|e| {
                let kind = self.events.get(&e.kind).copied()?;
                Some(gp_sim::Event {
                    kind,
                    frame: e.frame,
                    entity: e.entity.map(|i| i as usize),
                })
            })
            .collect()
    }
}

/// A run through a game's module, with the world it stands at decoded on
/// the host when something asks for it.
pub(crate) struct Hosted {
    rt: ModuleRuntime,
    spec: Arc<Spec>,
    world: OnceCell<World>,
}

impl Hosted {
    fn new(rt: ModuleRuntime, spec: Arc<Spec>) -> Self {
        Self {
            rt,
            spec,
            world: OnceCell::new(),
        }
    }

    fn world(&self) -> &World {
        self.world.get_or_init(|| {
            World::from_canonical(&self.spec, &self.rt.canonical_state())
                .unwrap_or_else(|e| panic!("the module's canonical state doesn't decode: {e}"))
        })
    }
}

/// A run of a model, as the tools drive it: through its module or the
/// built-in interpreter.
pub(crate) enum Model {
    Native(Box<Interp>),
    Module(Box<Hosted>),
}

impl Model {
    pub(crate) fn spec(&self) -> &Spec {
        match self {
            Self::Native(i) => i.spec(),
            Self::Module(h) => &h.spec,
        }
    }

    pub(crate) fn world(&self) -> &World {
        match self {
            Self::Native(i) => i.world(),
            Self::Module(h) => h.world(),
        }
    }

    /// Makes `step` return the hooks' sound-driver writes as events.
    pub(crate) fn with_apu_events(self) -> Result<Self> {
        match self {
            Self::Native(i) => Ok(Self::Native(Box::new(i.with_apu_events()))),
            Self::Module(mut h) => {
                h.rt.set_apu_events().map_err(|e| anyhow!("{e}"))?;
                Ok(Self::Module(h))
            }
        }
    }

    /// Gives the run its script of random draws.
    pub(crate) fn with_draws(self, d: gp_sim::DrawScript) -> Result<Self> {
        match self {
            Self::Native(i) => Ok(Self::Native(Box::new(i.with_draws(d)))),
            Self::Module(mut h) => {
                h.rt.set_draws(&d.text(), d.base)
                    .map_err(|e| anyhow!("{e}"))?;
                Ok(Self::Module(h))
            }
        }
    }

    /// Starts noting what the run reads inside its module. A native run's
    /// reads are the host thread's, noted there.
    pub(crate) fn start_reads(&mut self) -> Result<()> {
        match self {
            Self::Native(_) => Ok(()),
            Self::Module(h) => h.rt.start_reads().map_err(|e| anyhow!("{e}")),
        }
    }

    /// What the run read inside its module since [`Self::start_reads`].
    pub(crate) fn take_reads(&mut self) -> Result<Option<gp_sim::reads::Reads>> {
        match self {
            Self::Native(_) => Ok(None),
            Self::Module(h) => {
                let b = h.rt.take_reads().map_err(|e| anyhow!("{e}"))?;
                gp_interp::reads::read_reads(&b)
                    .map(Some)
                    .map_err(|e| anyhow!("{e}"))
            }
        }
    }

    /// How the run took its script of random draws; None when it has none.
    pub(crate) fn draws_report(&self) -> Result<Option<DrawsReport>> {
        match self {
            Self::Native(i) => Ok(i.draws().map(|d| DrawsReport {
                taken: d.next as u32,
                script: d.draws.len() as u32,
                desync: d.desync.clone(),
                desyncs: d.desyncs.clone(),
            })),
            Self::Module(h) => h.rt.report_draws().map_err(|e| anyhow!("{e}")),
        }
    }
}

impl gp_interp::baseline::Runs for Model {
    fn run_spec(&self) -> &Spec {
        self.spec()
    }
    fn run_world(&self) -> World {
        self.world().clone()
    }
}

impl Runtime for Model {
    fn capabilities(&self) -> Capabilities {
        match self {
            Self::Native(i) => i.capabilities(),
            Self::Module(h) => h.rt.capabilities(),
        }
    }

    fn step(&mut self, inputs: &[InputFrame]) -> Result<Vec<Event>, String> {
        match self {
            Self::Native(i) => i.step(inputs),
            Self::Module(h) => {
                h.world = OnceCell::new();
                h.rt.step(inputs)
            }
        }
    }

    fn run_until(
        &mut self,
        condition: &str,
        inputs: &[InputFrame],
        max_frames: u32,
    ) -> Result<Until, String> {
        match self {
            Self::Native(i) => i.run_until(condition, inputs, max_frames),
            Self::Module(h) => {
                h.world = OnceCell::new();
                h.rt.run_until(condition, inputs, max_frames)
            }
        }
    }

    fn frame(&self) -> u64 {
        match self {
            Self::Native(i) => i.frame(),
            Self::Module(h) => h.rt.frame(),
        }
    }

    fn snapshot(&self) -> Vec<u8> {
        match self {
            Self::Native(i) => i.snapshot(),
            Self::Module(h) => h.rt.snapshot(),
        }
    }

    fn restore(&mut self, snap: &[u8]) -> Result<(), String> {
        match self {
            Self::Native(i) => i.restore(snap),
            Self::Module(h) => {
                h.world = OnceCell::new();
                h.rt.restore(snap)
            }
        }
    }

    fn fork(&self) -> Self {
        match self {
            Self::Native(i) => Self::Native(Box::new(i.fork())),
            Self::Module(h) => Self::Module(Box::new(Hosted::new(h.rt.fork(), h.spec.clone()))),
        }
    }

    fn canonical_state(&self) -> Vec<u8> {
        match self {
            Self::Native(i) => i.canonical_state(),
            Self::Module(h) => h.rt.canonical_state(),
        }
    }

    fn state_hash(&self) -> u64 {
        match self {
            Self::Native(i) => i.state_hash(),
            Self::Module(h) => h.rt.state_hash(),
        }
    }

    fn load_state(&mut self, state: &[u8]) -> Result<Vec<LoadNote>, String> {
        match self {
            Self::Native(i) => i.load_state(state),
            Self::Module(h) => {
                h.world = OnceCell::new();
                h.rt.load_state(state)
            }
        }
    }

    fn query(&self, selector: &str) -> Result<Vec<Entry>, String> {
        match self {
            Self::Native(i) => i.query(selector),
            Self::Module(h) => h.rt.query(selector),
        }
    }

    fn sample(
        &mut self,
        selectors: &[String],
        inputs: &[InputFrame],
        frames: u32,
    ) -> Result<Vec<Vec<Entry>>, String> {
        match self {
            Self::Native(i) => i.sample(selectors, inputs, frames),
            Self::Module(h) => {
                h.world = OnceCell::new();
                h.rt.sample(selectors, inputs, frames)
            }
        }
    }

    fn entities_at(&self, area: Rect) -> Vec<u64> {
        match self {
            Self::Native(i) => i.entities_at(area),
            Self::Module(h) => h.rt.entities_at(area),
        }
    }

    fn patch(&mut self, model_diff: &str) -> PatchResult {
        match self {
            Self::Native(i) => i.patch(model_diff),
            Self::Module(h) => h.rt.patch(model_diff),
        }
    }

    fn set(&mut self, p: &str, v: Value) -> Result<(), String> {
        match self {
            Self::Native(i) => i.set(p, v),
            Self::Module(h) => {
                h.world = OnceCell::new();
                h.rt.set(p, v)
            }
        }
    }

    fn explain(&self, p: &str) -> Vec<String> {
        match self {
            Self::Native(i) => i.explain(p),
            Self::Module(h) => h.rt.explain(p),
        }
    }
}

/// What `gp interp odds` and `gp interp slots` ask of a run, in this
/// process or in its module (the module's `set-draw-script`, `draw-calls`
/// and `step-hooks` calls).
impl Model {
    /// Gives the run a draws script whole: its origin, whether it is
    /// lenient, and whether it keeps the run's draw calls, which
    /// [`Self::with_draws`]'s text form leaves out.
    pub(crate) fn with_draw_script(self, d: gp_sim::DrawScript) -> Result<Self> {
        match self {
            Self::Native(i) => Ok(Self::Native(Box::new(i.with_draws(d)))),
            Self::Module(mut h) => {
                h.rt.set_draw_script(&gp_module::DrawScriptCall {
                    text: &d.text(),
                    base: d.base,
                    origin: d.origin,
                    lenient: d.lenient,
                    keep_calls: d.calls.is_some(),
                })
                .map_err(|e| anyhow!("{e}"))?;
                Ok(Self::Module(h))
            }
        }
    }

    /// [`Runtime::step`] with a call's deadline in a module, not a frame's
    /// (`gp_module::ModuleRuntime::step_unhurried`).
    pub(crate) fn step_unhurried(&mut self, inputs: &[InputFrame]) -> Result<()> {
        match self {
            Self::Native(i) => i.step(inputs).map(|_| ()).map_err(|e| anyhow!("{e}")),
            Self::Module(h) => {
                h.world = OnceCell::new();
                h.rt.step_unhurried(inputs)
                    .map(|_| ())
                    .map_err(|e| anyhow!("{e}"))
            }
        }
    }

    /// The draw calls the run's script has kept, frame and site each, in
    /// order; empty when it keeps none.
    pub(crate) fn draw_calls(&self) -> Result<Vec<(u64, String)>> {
        match self {
            Self::Native(i) => Ok(i.draws().and_then(|d| d.calls.clone()).unwrap_or_default()),
            Self::Module(h) => Ok(h
                .rt
                .draw_calls()
                .map_err(|e| anyhow!("{e}"))?
                .unwrap_or_default()),
        }
    }

    /// The hash of the world the run stands at, as [`World::hash`] gives it.
    pub(crate) fn world_hash(&self) -> Result<[u8; 32]> {
        match self {
            Self::Native(i) => Ok(i.world().hash()),
            Self::Module(h) => h.rt.world_hash().map_err(|e| anyhow!("{e}")),
        }
    }

    /// Steps every body of `w` but the player once with this run's spec and
    /// hooks, as the enemies system does
    /// ([`gp_interp::compare::step_hooks`]): whether a hook drew.
    pub(crate) fn step_hooks(&self, w: &mut World) -> Result<bool> {
        match self {
            Self::Native(i) => Ok(gp_interp::compare::step_hooks(i.spec(), w, i.exts())),
            Self::Module(h) => {
                let (drew, state) =
                    h.rt.step_hooks(&w.canonical())
                        .map_err(|e| anyhow!("{e}"))?;
                *w = World::from_canonical(&h.spec, &state).map_err(|e| anyhow!("{e}"))?;
                Ok(drew)
            }
        }
    }
}
