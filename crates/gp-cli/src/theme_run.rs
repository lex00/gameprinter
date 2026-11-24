//! A model run as the theme commands drive it.
//!
//! game-printer's theme commands ran a model in the built-in interpreter
//! with its game's hooks compiled into gp (`interp::load_interp`). gameprinter
//! links no game's hooks: a model with hooks runs through its game's module
//! ([`crate::hosted`]). This is the one place that changes: a run opens
//! through the module at `$GP_HOME/work/modules/GAME.wasm` (GAME is the
//! template's `game`, else its folder's name), or `GP_THEME_MODULE` when
//! set, else a game package's module built from this gp's kit as every
//! tool that opens a model builds it ([`crate::module::built_for`]), and
//! falls back to the built-in interpreter on the model's components alone
//! when there is none, naming the hooks that don't run ([`Run::not_run`]). Each step gives the
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

/// The module a model folder's game runs through, if there is one:
/// `GP_THEME_MODULE`; else, for a game package with hook code of its own,
/// its module built from this gp's kit (or reused), so a stale
/// `$GP_HOME/work/modules/GAME.wasm` (one built before the model took a
/// field, which its own model reader then refuses) is never run, as
/// gp serve's scoreboard picks it; else the game's module by name in
/// `$GP_HOME/work/modules`. A package whose module fails to build has
/// none here, and its hooks are named as not run.
pub(crate) fn module_for(dir: &Path, paths: &Paths) -> Option<PathBuf> {
    if let Some(m) = std::env::var_os("GP_THEME_MODULE").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(m));
    }
    if !own_hooks(dir) {
        return named_module(dir, &paths.home);
    }
    match crate::module::built_for(&paths.home, dir) {
        Ok(found) => found.map(|m| m.0),
        Err(e) => {
            eprintln!("warning: the game's module didn't build: {e:#}");
            None
        }
    }
}

/// Whether `dir` is a game package with hook code of its own, whose
/// module is built from it.
fn own_hooks(dir: &Path) -> bool {
    dir.join("package.toml").is_file() && matches!(crate::package::hook_code(dir), Ok(Some(_)))
}

/// The game's module by name, `home/work/modules/GAME.wasm`, when it is
/// there.
fn named_module(dir: &Path, home: &Path) -> Option<PathBuf> {
    let f = home
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
        Self::open_with(bundle, dir, paths, None)
    }

    /// [`Self::open`] through `module` when one is named, else the one
    /// [`module_for`] finds.
    pub(crate) fn open_with(
        bundle: &Bundle,
        dir: &Path,
        paths: &Paths,
        module: Option<&Path>,
    ) -> Result<Self> {
        let module = match module {
            Some(m) => Some(m.to_owned()),
            None => module_for(dir, paths),
        };
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

    /// Gives the run the original's random draws, so a game whose play
    /// turns on them (who comes in, where) follows the recorded play.
    /// A script only starts on a load, so the run's power-on state is
    /// loaded again once it has the script, as clip scoring does: the
    /// model then takes the original's draws from its first frame.
    pub(crate) fn with_draws(self, mut d: gp_sim::DrawScript) -> Result<Self> {
        use gp_runtime::Runtime as _;
        let power_on = self.rt.canonical_state();
        d.base = 0;
        let mut rt = self.rt.with_draws(d)?;
        rt.load_state(&power_on).map_err(|e| anyhow!("{e}"))?;
        Ok(Self {
            rt,
            not_run: self.not_run,
        })
    }

    pub(crate) fn world(&self) -> &World {
        self.rt.world()
    }

    /// The model as the tools run it, for drawing it as the screen
    /// measure draws it (crate::screen::model_frame).
    pub(crate) fn model(&self) -> &hosted::Model {
        &self.rt
    }

    /// One frame: the simulation's events, and the APU writes the game's
    /// sound driver made on it.
    pub(crate) fn advance(&mut self, f: &InputFrame) -> Result<Stepped, String> {
        let events = gp_runtime::Runtime::step(&mut self.rt, std::slice::from_ref(f))?;
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A made-up model with a hooked entity, in a folder that is no game
    /// package and with no module anywhere: the run names the hook as not
    /// run, which gp theme video refuses rather than draw another game.
    #[test]
    fn a_hook_with_no_module_is_named_as_not_run() {
        let home = tempfile::tempdir().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let text = "[model]\nschema = \"0.4\"\nname = \"cab\"\ntitle = \"Cab\"\n\
                    description = \"A made-up game\"\nview = \"side\"\nframe_rate = \"30\"\n\
                    systems = [\"rules\", \"entity_states\", \"counters\", \"spawn\", \"platformer.player\", \"platformer.enemies\", \"platformer.contact\", \"exits\", \"camera\"]\n\
                    start_level = \"main\"\n\n[model.extensions]\nplatformer = \"0.4\"\n\n\
                    [entities.player]\nrole = \"player\"\nsize = [16, 16]\n\
                    hitbox = { x = 0, y = 0, w = 16, h = 16 }\n\n\
                    [entities.bird]\nrole = \"npc\"\next = \"bird\"\nsize = [16, 16]\n\
                    hitbox = { x = 0, y = 0, w = 16, h = 16 }\n\n\
                    [levels.main]\nempty = true\nsize = [32, 30]\ntile_size = 16\n";
        let m = Model::parse(text).unwrap();
        let mut lifted = Lifted::default();
        lifted.files.insert(
            "assets.toml".into(),
            "schema = 1\nplatform = \"arcade\"\nsource = \"cab\"\nsets = [\"cab\"]\n\n\
             [[asset]]\nname = \"tiles\"\nformat = \"mcr-tiles\"\nregion = \"gfx1\"\ncount = 1\nper_row = 1\n"
                .into(),
        );
        let bundle = Bundle::new(&m, lifted);
        let paths = crate::test_paths(home.path());
        if std::env::var_os("GP_THEME_MODULE").is_some() {
            return;
        }
        assert_eq!(module_for(dir.path(), &paths), None);
        let run = Run::open_with(&bundle, dir.path(), &paths, None).unwrap();
        assert_eq!(run.not_run, vec!["bird".to_owned()]);
    }

    /// A game package with hook code of its own runs on its module built
    /// from its hooks, never on `$GP_HOME/work/modules/GAME.wasm`: a
    /// GAME.wasm built before the model took `board_sprite.gone_next`
    /// refuses the model with its own reader (unknown field gone_next), so
    /// gp theme look sheet stopped. A folder with no hook code still gets
    /// the module kept by name.
    #[test]
    fn a_game_with_hooks_never_runs_a_module_kept_by_name() {
        let home = tempfile::tempdir().unwrap();
        let games = tempfile::tempdir().unwrap();
        let g = games.path().join("rooftop");
        std::fs::create_dir_all(g.join("ext/src")).unwrap();
        std::fs::write(g.join("package.toml"), "[package]\nname = \"rooftop\"\n").unwrap();
        let named = home.path().join("work/modules/rooftop.wasm");
        std::fs::create_dir_all(named.parent().unwrap()).unwrap();
        std::fs::write(&named, "an old module").unwrap();
        // No hook code yet: the module kept by name.
        assert!(!own_hooks(&g));
        assert_eq!(named_module(&g, home.path()), Some(named.clone()));
        // Hook code of its own: built from it, so the old one isn't run.
        std::fs::write(g.join("ext/src/lib.rs"), "// hooks\n").unwrap();
        assert!(own_hooks(&g));
    }
}
