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
use std::fs;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context as _, Result, anyhow, bail};
use gp_evidence::Paths;
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
    /// The hooks in `gp` itself: only a game with none of its own.
    pub exts: Arc<Exts>,
    /// Set when the model's hooks run in a game's module.
    module: Option<ModuleCode>,
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
        let interp = Interp::from_bundle(&bundle).map_err(|e| anyhow!("{e}"))?;
        let spec = interp.spec().clone();
        let (exts, _) = gp_exts::registry(&spec.name);
        let exts = Arc::new(exts);
        let missing: Vec<String> = interp
            .with_exts(exts.clone())
            .ext_hooks()
            .iter()
            .map(|h| (*h).to_owned())
            .collect();
        let bytes = bundle.to_bytes();
        let module = match module {
            Some(p) => {
                let wasm = fs::read(p).with_context(|| p.display().to_string())?;
                let code = ModuleCode::compile(&wasm).map_err(|e| anyhow!("{e}"))?;
                let first = code.load(&bytes).map_err(|e| anyhow!("{e}"))?;
                let theirs = first.spec_digest().map_err(|e| anyhow!("{e}"))?;
                let ours = gp_sim::save::spec_digest(&spec);
                if theirs != ours {
                    bail!(
                        "the module compiles this model to a different spec: digest {theirs}, here {ours}. Rebuild it from this tool's kit"
                    );
                }
                Some(code)
            }
            None if missing.is_empty() => None,
            None => bail!(
                "the model `{}` names hooks, which run only in its game's module (--module): {}",
                spec.name,
                missing.join(", ")
            ),
        };
        Ok(Self {
            bundle,
            bytes,
            spec,
            exts,
            module,
        })
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
