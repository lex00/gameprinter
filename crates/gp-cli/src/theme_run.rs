//! A model run as the theme commands drive it.
//!
//! game-printer's theme commands ran a model in the built-in interpreter
//! with its game's hooks compiled into gp (`interp::load_interp`). gameprinter
//! links no game's hooks: a model with hooks runs through its game's module
//! ([`crate::hosted`]). This is the one place that changes: a run opens
//! through the module at `$GP_HOME/work/modules/GAME.wasm` (GAME is the
//! template's `game`, else its folder's name), or `GP_THEME_MODULE` when
//! set, and falls back to the built-in interpreter on the model's
//! components alone when there is none, saying so. Each step gives the
//! simulation's events, as the interpreter's `advance` did, and the APU
//! writes of the game's sound driver, as `gp_sim::ext::hook_apu` did.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, anyhow};
use gp_evidence::Paths;
use gp_interp::{Bundle, Lifted};
use gp_model::Model;
use gp_runtime::InputFrame;
use gp_sim::{Spec, World};

use crate::hosted::{self, Source};

/// A model and its lifted assets as a bundle, as game-printer's
/// `interp::load_bundle` read them.
pub(crate) fn load_bundle(model: &Path, assets: &Path) -> Result<Bundle> {
    let file = if model.is_dir() {
        model.join(gp_model::MODEL_FILE)
    } else {
        model.to_owned()
    };
    let text =
        std::fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
    let m = Model::parse(&text).with_context(|| file.display().to_string())?;
    let lifted = Lifted::read_dir(assets, &m).map_err(|e| {
        anyhow!("{e}\n(lift the model's assets with gp assets lift, or name them with --assets)")
    })?;
    Ok(Bundle::new(&m, lifted))
}

/// The game a model folder is of: template.toml's `game`, else the
/// folder's name.
pub(crate) fn game_of(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("template.toml"))
        .ok()
        .and_then(|t| t.parse::<toml::Table>().ok())
        .and_then(|t| t.get("template")?.get("game")?.as_str().map(str::to_owned))
        .or_else(|| {
            std::fs::canonicalize(dir)
                .ok()?
                .file_name()?
                .to_str()
                .map(str::to_owned)
        })
        .unwrap_or_default()
}

/// The module a model folder's game runs through, if there is one.
pub(crate) fn module_for(dir: &Path, paths: &Paths) -> Option<PathBuf> {
    if let Some(m) = std::env::var_os("GP_THEME_MODULE").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(m));
    }
    let f = paths
        .home
        .join("work")
        .join("modules")
        .join(format!("{}.wasm", game_of(dir)));
    f.is_file().then_some(f)
}

/// What one frame gave: the simulation's events and the sound driver's
/// APU writes.
pub(crate) type Stepped = (Vec<gp_sim::Event>, Vec<(u8, u8)>);

/// One run of a model, from power-on.
pub(crate) struct Run {
    rt: hosted::Model,
    /// The hooks that don't run (no module was found): empty through a
    /// module.
    pub(crate) not_run: Vec<String>,
}

impl Run {
    /// Opens a run of `bundle`, the model in `dir`.
    pub(crate) fn open(bundle: &Bundle, dir: &Path, paths: &Paths) -> Result<Self> {
        let module = module_for(dir, paths);
        let source = Source::new_allowing(bundle.clone(), module.as_deref(), true)?;
        let rt = source.open()?;
        let not_run = match &rt {
            hosted::Model::Native(i) => i.ext_hooks().iter().map(|h| (*h).to_owned()).collect(),
            hosted::Model::Module(_) => Vec::new(),
        };
        let rt = rt.with_apu_events()?;
        Ok(Self { rt, not_run })
    }

    pub(crate) fn spec(&self) -> &Spec {
        self.rt.spec()
    }

    pub(crate) fn world(&self) -> &World {
        self.rt.world()
    }

    /// One frame: the simulation's events, and the APU writes the game's
    /// sound driver made on it.
    pub(crate) fn advance(&mut self, f: &InputFrame) -> Result<Stepped, String> {
        // A frame no player waits on: a call's deadline, not a frame's, so a
        // busy machine doesn't stop the module mid-review.
        let events = self
            .rt
            .step_unhurried_events(std::slice::from_ref(f))
            .map_err(|e| format!("{e:#}"))?;
        let apu = gp_interp::lockstep::apu_writes(&events);
        let names = &self.rt.spec().events;
        let sim = events
            .iter()
            .filter_map(|e| {
                let k = names.iter().position(|n| n == &e.kind)?;
                Some(gp_sim::Event {
                    frame: e.frame,
                    kind: gp_sim::spec::EventKind(k),
                    entity: e.entity.and_then(|i| usize::try_from(i).ok()),
                })
            })
            .collect();
        Ok((sim, apu))
    }

    /// The world's state hash as a frame hash, as `gp interp run` hashes it.
    pub(crate) fn frame_hash(&self) -> gp_evidence::frame_hashes::FrameHash {
        let h = gp_evidence::Hash::from_bytes(self.rt.world().hash());
        gp_evidence::frame_hashes::FrameHash { state: h, ram: h }
    }
}
