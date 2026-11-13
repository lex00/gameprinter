//! `gp theme`: themes on a model (docs/formats/theme.md). Checking a stack
//! (each theme, the readability check, the Style checks), the formats'
//! schemas, painting placeholder restyled art from roles, and a themed
//! video of the interpreter. The theme options here are shared with
//! `gp gen bevy` and `gp gen conform`.

use std::collections::BTreeMap;
use std::fs;
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use gp_evidence::Paths;
use gp_evidence::frame_hashes::{self, FrameHash};
use gp_evidence::hash::Hash;
use gp_look::present::{Art, art_themed};
use gp_model::Model;
use gp_model::path::Step;
use gp_theme::color::Rgb;
use gp_theme::compose::{Placed, Scene};
use gp_theme::format::{Restyle, StyleGuide, Theme, sha256_hex};
use gp_theme::paint::{Scheme, Shades};
use gp_theme::pix::Rgba;
use gp_theme::readability::{self, Report, Verdict};
use gp_theme::stack::{self, Places};
use gp_theme::style::{Deterministic, Reference, StyleMeasure};
use gp_theme::{Resolved, Scope, Stack};
use serde_json::json;

use crate::Outcome;
use crate::interp::InputArgs;

/// game-printer's commands wrote a run record of their inputs and outputs
/// (`gp_metrics::RunRecorder`); gameprinter keeps none, so the theme
/// commands' calls to it do nothing.
pub(crate) struct RunRecorder;

impl RunRecorder {
    pub(crate) fn output(
        &mut self,
        _name: impl Into<String>,
        _hash: impl Into<String>,
        _bytes: Option<u64>,
    ) {
    }
}

/// Which themes to draw with. With none of these, nothing is themed.
#[derive(clap::Args, Debug, Default, Clone)]
pub struct ThemeArgs {
    /// A stack file (stack.toml, docs/formats/theme.md): baked layers from
    /// the template's themes/, personal ones from $GP_HOME/themes/.
    #[arg(long, value_name = "FILE")]
    pub stack: Option<PathBuf>,
    /// The model's own stack: the template's themes/stack.toml, then the
    /// player's $GP_HOME/themes/stacks/<model>.toml.
    #[arg(long)]
    pub themes: bool,
    /// A theme directory to draw on top, as a personal layer; repeat for
    /// more, bottom first.
    #[arg(long = "theme", value_name = "DIR")]
    pub theme_dirs: Vec<PathBuf>,
    /// Go on when the readability check refuses the stack.
    #[arg(long)]
    pub allow_unreadable: bool,
}

impl ThemeArgs {
    pub fn any(&self) -> bool {
        self.stack.is_some() || self.themes || !self.theme_dirs.is_empty()
    }
}

#[derive(Subcommand)]
pub enum ThemeCmd {
    /// Check a model's theme stack: each theme's file, its restyles' slot names
    /// against every model in its `for` list (read from the templates of the
    /// same game), then the readability check and the Style checks on the
    /// themed art. Exits 1 when a theme is invalid or a restyle names a slot or
    /// entry no model has. It also exits 1 when the readability check refuses
    /// or a Style check fails. Without theme options it checks the model's own
    /// stack (`--themes`).
    Check {
        /// A model directory (holding model.toml) or a model.toml file.
        model: PathBuf,
        #[command(flatten)]
        theme: ThemeArgs,
        /// The model's lifted or stub assets (default:
        /// $GP_HOME/work/assets/<model.source>).
        #[arg(long, value_name = "DIR")]
        assets: Option<PathBuf>,
        /// The originality check (docs/theme-kit.md): each themed art slot
        /// against the original's from the lift, each sound replacement's
        /// melody against the original cue, and every theme file for runs
        /// of the ROM's bytes. Refuses a theme too close to the original.
        #[arg(long)]
        original: bool,
        /// The ROM for the originality check's byte runs (default: the
        /// first file under $GP_ROM_DIR whose hash the template accepts).
        #[arg(long)]
        rom: Option<PathBuf>,
    },
    /// Print the theme format's JSON Schema with `theme`, or the stack
    /// format's with `stack`.
    Schema {
        #[arg(value_parser = ["theme", "stack"])]
        which: String,
    },
    /// Paint a restyled-assets theme for a model from its roles: a sprite
    /// for each entity role and a cell for each tileset item role, in a
    /// paper-cutout style, as .pix files with a theme.toml naming them
    /// (CC0-1.0, origin generated). Writes OUT; refuses an OUT that exists
    /// unless `--force`.
    Paint {
        /// A model directory (holding model.toml) or a model.toml file.
        model: PathBuf,
        /// The theme directory to write.
        #[arg(short, long, value_name = "DIR")]
        out: PathBuf,
        /// The theme's id.
        #[arg(long, default_value = "paper")]
        id: String,
        /// The theme's title.
        #[arg(long, default_value = "Paper")]
        title: String,
        /// Picks the grain; the same seed paints the same pixels.
        #[arg(long, default_value_t = 1)]
        seed: u32,
        /// The game's title as the theme shows it on a title screen, in
        /// place of the original's (letters, digits and spaces).
        #[arg(long, default_value = "PAPER PLAY")]
        logo: String,
        /// A file whose first line is the logo, in place of --logo: the
        /// one place a game's name is kept (Rivet Rodeo's is
        /// scripts/rivet-rodeo/title.txt).
        #[arg(long, value_name = "FILE", conflicts_with = "logo")]
        logo_file: Option<PathBuf>,
        /// The theme's credit line, on a title screen in place of the
        /// original's own words (a maker's or a country's line).
        #[arg(long, default_value = "MADE WITH GAME PRINTER")]
        credit: String,
        /// Another model the theme is for, by `model.name`, listed in its
        /// `for` after the painted one (repeatable). Paint the model with
        /// the most slots, and name the others of the same game with
        /// `--for`: a model passes over the restyles of slots it lacks.
        #[arg(long = "for", value_name = "MODEL")]
        also_for: Vec<String>,
        #[arg(long)]
        force: bool,
    },
    /// Write a game's theme kit (docs/theme-kit.md) as kit.toml in the
    /// template's themes/kit/ (or OUT). For each art slot the kit gives its
    /// role and size. It also gives the slot's frames with their timing, and
    /// its colors and anchor. Its description of each slot names no character.
    /// For each sound role it gives the cue, length and loop. With --rom
    /// (checked against the template's accepted hashes) it also writes the
    /// local part, the tiles each frame is cut into and a reference sheet,
    /// under $GP_HOME/work/theme-kits/GAME/, never into the kit.
    Kit {
        /// A template directory, or a template id under ./templates.
        game: PathBuf,
        /// Your ROM of the game, for the local part.
        #[arg(long)]
        rom: Option<PathBuf>,
        /// Where to write kit.toml (default: the template's themes/kit/).
        #[arg(short, long, value_name = "DIR")]
        out: Option<PathBuf>,
        /// The lifted assets (default: $GP_HOME/work/assets/<source>).
        #[arg(long, value_name = "DIR")]
        assets: Option<PathBuf>,
    },
    /// Fit art to a game's slots (docs/theme-kit.md, "Fitting"). Each sprite
    /// slot's frames come from `ART/<slot id>/<frame>.png` (any size, any
    /// colors) or from the filled grids of `gp theme sheet` (--sheets, read
    /// through sheets.json). Each frame has its background cleared (alpha, the
    /// key color or a flood from the edges) and is cropped to the figure. One
    /// scale serves the whole slot. The frame is placed by the anchor,
    /// downscaled by area averaging, and cut into the game's tiles by one
    /// lightness split for the slot. Each picture slot (`ART/<slot id>.png`, or
    /// the sheets' picture) is kept whole. Needs the kit's local part (`gp
    /// theme kit --rom`). Merges into OUT's theme.toml when it has one.
    Fit {
        /// A template directory, or a template id under ./templates.
        game: PathBuf,
        /// The folder of art, by slot id.
        #[arg(long, value_name = "DIR")]
        art: Option<PathBuf>,
        /// A folder `gp theme sheet` wrote, with the filled grids.
        #[arg(long, value_name = "DIR")]
        sheets: Option<PathBuf>,
        /// The theme directory to write.
        #[arg(short, long, value_name = "DIR")]
        out: PathBuf,
        /// Also write a contact sheet: each slot's fitted frames at 1x and
        /// 4x.
        #[arg(long, value_name = "FILE")]
        contact: Option<PathBuf>,
    },
    /// Write starting grids and prompts for an image model from a game's kit
    /// and a style card (docs/theme-kit.md, "Sheets"). Each grid holds a
    /// subject or a group of small slots. Its cells are sized by slot, and each
    /// has a label, a box and a line, with a neutral pose figure. The command
    /// writes a prompt file per grid for gpt_image_2 and for gen4_image (1000
    /// characters at most); the style board and turnaround prompts to make
    /// first; a prompt per whole picture; and sheets.json, which `gp theme fit
    /// --sheets` reads back. OUT must be outside the repo.
    Sheet {
        /// A template directory, or a template id under ./templates.
        game: PathBuf,
        /// A style card by name (personal, then themes/styles/) or file.
        #[arg(long)]
        style: Option<String>,
        /// The folder to write.
        #[arg(short, long, value_name = "DIR")]
        out: PathBuf,
        /// The theme's own title, the only words a title screen gets.
        #[arg(long, default_value = "MY GAME")]
        logo: String,
        #[arg(long)]
        force: bool,
        /// Also write stage guides made from your ROM (the lifted level's
        /// platforms and ladders), marked rom_derived in sheets.json. Off by
        /// default: they are the original's layout, never to be sent to a
        /// service unless you choose to. An original template's guides are
        /// written without it.
        #[arg(long)]
        rom_guides: bool,
        /// Write retry prompts for these cells of OUT's filled grids
        /// (`A03,D07`) instead of new sheets: each asks the model to paint
        /// only those cells again on the filled grid.
        #[arg(long, value_delimiter = ',', value_name = "CELLS")]
        retry: Vec<String>,
    },
    /// Style cards (docs/theme-kit.md, "Style cards"): how a theme looks,
    /// apart from any game.
    Style {
        #[command(subcommand)]
        cmd: crate::theme_style::StyleCmd,
    },
    /// Make a theme's art with Runway's API (docs/theme-kit.md, "Generating
    /// with Runway"). It makes the style board and turnarounds on gen4_image
    /// first. Then it makes each sheet of `gp theme sheet` on gpt_image_2, and
    /// every picture on gen4_image with those as references. Then it runs `gp
    /// theme fit` and `gp theme check --original`, makes single-cell calls for
    /// refused slots, and draws a contact sheet. Every call is in the run
    /// folder's ledger, so running it again continues without paying twice.
    /// `--dry-run` prints the calls and their cost and makes none; a paid run
    /// asks on the terminal or needs `--yes`, and stops before passing
    /// `--budget`. The key comes from RUNWAY_API_KEY, the keychain (service
    /// game-printer-runway) or $GP_HOME/secrets/runway (mode 600).
    Generate(crate::theme_generate::GenerateArgs),
    /// Run a model in the interpreter on an input and write its frames, drawn
    /// with a theme stack, as an animated PNG. Prints the digest of the
    /// per-frame state hashes, which is `gp interp run`'s for the same input
    /// with any theme: the presentation reads the world and never writes it.
    /// Publish a theme version for a game (docs/theme-publishing.md). It
    /// refuses an id a theme the game ships with already has, runs every check
    /// (readability and Style, originality against the game's ROM when it is
    /// here, geometry), and refuses on any failure. Then it makes the package,
    /// scans it for game data and signs its manifest with a maintainer's SSH
    /// key. It keeps the package in the local store, copies it to the mirrors
    /// in mirrors.toml and adds it to the game's themes index,
    /// `themes/index/GAME.toml`, through the gate (GAME is the template's
    /// `game`, else its id). A game with no ROM art gets a look package (the
    /// theme, the look gp drew, the check results); a game lifted from a ROM
    /// gets a source package (the theme's own files and the check results,
    /// nothing drawn from the ROM's art). --dry-run stops before the store and
    /// prints what would be published.
    Publish(crate::theme_publish::PublishArgs),
    /// Check a published ROM theme (a source package) on a model of its
    /// `for` with a ROM on this machine, and add the result to the game's
    /// themes index: for a model the publisher had no ROM for, or to
    /// confirm one. Reads the package by its id from the local store or a
    /// mirror (every hash and the signature checked), runs the same checks
    /// as publish for the model, compares the look drawn here with the one
    /// recorded, and lands the index line with the model's result, slot
    /// fingerprint and look digest through the gate. A look that differs
    /// from the recorded one is reported and not added. The package and
    /// its signature are left as they are. --dry-run writes nothing.
    Recheck(crate::theme_publish::RecheckArgs),
    /// Fetch a published theme by the game's index: the manifest by its
    /// hash, its signature, and each file by its hash, from the local
    /// store or the first mirror that has it, every hash checked, into
    /// OUT (default $GP_HOME/themes/packages/GAME/THEME).
    Fetch {
        game: String,
        theme: String,
        /// The index file (default: the repo's themes/index/GAME.toml).
        #[arg(long, value_name = "FILE")]
        index: Option<PathBuf>,
        #[arg(short, long, value_name = "DIR")]
        out: Option<PathBuf>,
    },
    /// List the themes published for a game. Each entry gives a theme's title,
    /// author and licence. Its size and package are given too, with the check
    /// results and whether the local store has the theme.
    List {
        game: String,
        /// The index file (default: the repo's themes/index/GAME.toml).
        #[arg(long, value_name = "FILE")]
        index: Option<PathBuf>,
        /// Read the index copy on the first mirror that has one instead.
        #[arg(long)]
        mirror: bool,
    },
    Video {
        /// A model directory (holding model.toml) or a model.toml file.
        model: PathBuf,
        #[command(flatten)]
        input: InputArgs,
        #[command(flatten)]
        theme: ThemeArgs,
        /// The model's lifted or stub assets.
        #[arg(long, value_name = "DIR")]
        assets: Option<PathBuf>,
        /// The video, outside any git work tree.
        #[arg(short, long, value_name = "FILE")]
        out: PathBuf,
        /// Each pixel repeated this many times both ways, 1 to 4.
        #[arg(long, default_value_t = 2, value_parser = clap::value_parser!(u8).range(1..=4))]
        scale: u8,
        /// Replace an existing file.
        #[arg(long)]
        force: bool,
        /// Also play the stack's sound per role on its cues, as the Bevy
        /// build and the web preview play it (gp_present::sound::RolePlayer),
        /// into this 16-bit WAV file at 44,100 Hz, and list each cue's frame.
        #[arg(long, value_name = "FILE")]
        wav: Option<PathBuf>,
    },
}

/// A model's file, directory and parsed model.
pub(crate) fn read_model(arg: &Path) -> Result<(Model, PathBuf)> {
    let (file, dir) = if arg.is_dir() {
        (arg.join(gp_model::MODEL_FILE), arg.to_owned())
    } else {
        (
            arg.to_owned(),
            arg.parent().map(Path::to_owned).unwrap_or_default(),
        )
    };
    let text = fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
    let m = Model::parse(&text).with_context(|| format!("{}", file.display()))?;
    Ok((m, dir))
}

/// The theme stack the options name for a model, resolved. Empty without
/// options.
pub(crate) fn resolve(
    args: &ThemeArgs,
    model_dir: &Path,
    model: &str,
    paths: &Paths,
) -> Result<Resolved> {
    let places = Places::for_model(model_dir, Some(&paths.home));
    let mut stacked = Stack {
        format: gp_theme::FORMAT_VERSION.into(),
        model: None,
        layer: Vec::new(),
    };
    if args.themes {
        stacked.layer.extend(
            places
                .default_stack(model)
                .map_err(anyhow::Error::msg)?
                .layer,
        );
    }
    if let Some(f) = &args.stack {
        let text = fs::read_to_string(f).with_context(|| format!("reading {}", f.display()))?;
        let s = Stack::parse(&text).map_err(|e| anyhow::anyhow!("{}: {e}", f.display()))?;
        if let Some(m) = &s.model
            && m != model
        {
            bail!("{}: the stack is for `{m}`, not `{model}`", f.display());
        }
        stacked.layer.extend(s.layer);
    }
    let mut r = stack::resolve(&stacked, &places, model).map_err(anyhow::Error::msg)?;
    for d in &args.theme_dirs {
        let l = stack::load(d, Scope::Personal).map_err(anyhow::Error::msg)?;
        if r.layers.iter().any(|x| x.theme.id == l.theme.id) {
            bail!("theme `{}` is in the stack twice", l.theme.id);
        }
        if !l.theme.for_models.is_empty() && !l.theme.for_models.iter().any(|m| m == model) {
            bail!(
                "theme `{}` is for {}, not `{model}`",
                l.theme.id,
                l.theme.for_models.join(", ")
            );
        }
        r.layers.push(l);
    }
    Ok(r)
}

/// The readability check on themed art. With a stack on, a refusal stops
/// the command unless `allow`.
pub(crate) fn readable(art: &Art, theme: &Resolved, allow: bool) -> Result<Option<Report>> {
    if theme.is_empty() {
        return Ok(None);
    }
    let r = readability::check(&art.theme.samples, theme);
    if r.verdict == Verdict::Refuse && !allow {
        bail!(
            "{}the theme stack makes the game hard to read; change it, or pass --allow-unreadable",
            r.text()
        );
    }
    Ok(Some(r))
}

/// The assets directory: the given one, or the model source's lift.
fn assets_dir(m: &Model, assets: Option<&Path>, paths: &Paths) -> Result<PathBuf> {
    if let Some(a) = assets {
        return Ok(a.to_owned());
    }
    let Some(source) = &m.model.source else {
        bail!("the model names no source, so give its assets with --assets");
    };
    Ok(paths.home.join("work").join("assets").join(source))
}

pub(crate) fn run(cmd: &ThemeCmd, paths: &Paths, rec: &mut RunRecorder) -> Result<Outcome> {
    match cmd {
        ThemeCmd::Schema { which } => Ok(Outcome::ok(
            json!({ "which": which }),
            match which.as_str() {
                "theme" => gp_theme::format::theme_schema(),
                _ => gp_theme::format::stack_schema(),
            }
            .trim_end()
            .to_owned(),
        )),
        ThemeCmd::Check {
            model,
            theme,
            assets,
            original,
            rom,
        } => {
            let mut out = check(model, theme, assets.as_deref(), paths, rec)?;
            if *original {
                originality(
                    model,
                    theme,
                    assets.as_deref(),
                    rom.as_deref(),
                    paths,
                    rec,
                    &mut out,
                )?;
            }
            Ok(out)
        }
        ThemeCmd::Paint {
            model,
            out,
            id,
            title,
            seed,
            logo,
            logo_file,
            credit,
            also_for,
            force,
        } => {
            let logo = match logo_file {
                Some(f) => fs::read_to_string(f)
                    .with_context(|| format!("reading {}", f.display()))?
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_owned(),
                None => logo.clone(),
            };
            paint(
                model, out, id, title, *seed, &logo, credit, also_for, *force, paths, rec,
            )
        }
        ThemeCmd::Kit {
            game,
            rom,
            out,
            assets,
        } => crate::theme_kit::kit(
            game,
            rom.as_deref(),
            out.as_deref(),
            assets.as_deref(),
            paths,
            rec,
        ),
        ThemeCmd::Fit {
            game,
            art,
            sheets,
            out,
            contact,
        } => crate::theme_fit::fit_cmd(
            game,
            art.as_deref(),
            sheets.as_deref(),
            out,
            contact.as_deref(),
            paths,
        ),
        ThemeCmd::Sheet {
            game,
            style,
            out,
            logo,
            force,
            rom_guides,
            retry,
        } => {
            if retry.is_empty() {
                crate::theme_sheet::sheet_cmd(
                    game,
                    style.as_deref(),
                    out,
                    logo,
                    *force,
                    *rom_guides,
                    paths,
                )
            } else {
                crate::theme_sheet::retry_cmd(out, retry)
            }
        }
        ThemeCmd::Style { cmd } => crate::theme_style::run(cmd, paths),
        ThemeCmd::Generate(a) => crate::theme_generate::run(a, paths),
        ThemeCmd::Publish(a) => crate::theme_publish::publish(a, paths, rec),
        ThemeCmd::Recheck(a) => crate::theme_publish::recheck(a, paths, rec),
        ThemeCmd::Fetch {
            game,
            theme,
            index,
            out,
        } => crate::theme_publish::fetch(game, theme, index.as_deref(), out.as_deref(), paths),
        ThemeCmd::List {
            game,
            index,
            mirror,
        } => crate::theme_publish::list(game, index.as_deref(), *mirror, paths),
        ThemeCmd::Video {
            model,
            input,
            theme,
            assets,
            out,
            scale,
            force,
            wav,
        } => video(
            model,
            input,
            theme,
            assets.as_deref(),
            out,
            *scale,
            *force,
            wav.as_deref(),
            paths,
            rec,
        ),
    }
}

/// The themed art for a model: the stack, the compiled spec and pictures.
pub(crate) fn themed(
    model: &Path,
    args: &ThemeArgs,
    assets: Option<&Path>,
    paths: &Paths,
    _rec: &mut RunRecorder,
) -> Result<(Model, Resolved, Art, gp_interp::lifted::Bundle)> {
    let (m, dir) = read_model(model)?;
    let theme = resolve(args, &dir, &m.model.name, paths)?;
    let adir = assets_dir(&m, assets, paths)?;
    let bundle = crate::theme_run::load_bundle(model, &adir)?;
    let spec = gp_interp::compile::compile(&m, &bundle.assets).map_err(anyhow::Error::msg)?;
    let art = art_themed(&m, &spec, &adir, &theme).map_err(anyhow::Error::msg)?;
    Ok((m, theme, art, bundle))
}

pub(crate) fn check(
    model: &Path,
    args: &ThemeArgs,
    assets: Option<&Path>,
    paths: &Paths,
    rec: &mut RunRecorder,
) -> Result<Outcome> {
    let mut args = args.clone();
    if !args.any() {
        args.themes = true;
    }
    // Slot names first: a restyle no model has would be passed over with
    // no word, so a typo is refused before anything is drawn.
    let (m0, dir) = read_model(model)?;
    let stack = resolve(&args, &dir, &m0.model.name, paths)?;
    let layers: Vec<&Theme> = stack.layers.iter().map(|l| &l.theme).collect();
    let slots = crate::theme_slots::check(&dir, &m0, &layers);
    if slots.refused {
        return Ok(Outcome {
            json: json!({"model": m0.model.name, "slots": slots.json}),
            text: slots.text.trim_end().to_owned(),
            exit: 1,
        });
    }
    let (m, theme, art, bundle) = themed(model, &args, assets, paths, rec)?;
    let report = readability::check(&art.theme.samples, &theme);
    let geometry = geometry(&m, &dir, &theme, &bundle, assets, paths);
    let contract_file = dir.join(gp_contract::CONTRACT_FILE);
    let contract = fs::read_to_string(&contract_file)
        .ok()
        .and_then(|t| gp_contract::Contract::parse(&t).ok());
    // Style checks cover the sprites the contract holds to Style.
    let subjects: Vec<_> = art
        .theme
        .subjects
        .iter()
        .filter(|s| match &contract {
            Some(c) => {
                let steps: Vec<Step> = s.part.split('.').map(|k| Step::Key(k.into())).collect();
                c.bind(&m).resolve(&steps, None).level == gp_contract::Level::Style
            }
            None => true,
        })
        .cloned()
        .collect();
    let reference = Reference::of(&theme);
    let measures = reference
        .as_ref()
        .map(|r| Deterministic.measure(&subjects, r))
        .unwrap_or_default();
    let failed = measures.iter().filter(|x| !x.pass).count();
    let mut text = String::new();
    if theme.is_empty() {
        text.push_str("no themes in the stack: the model's own art\n");
    }
    for l in &theme.layers {
        text.push_str(&format!(
            "layer {} ({:?}): {}, {}\n",
            l.theme.id, l.scope, l.theme.title, l.theme.license
        ));
    }
    text.push_str(&slots.text);
    text.push_str(&report.text().to_lowercase());
    match &reference {
        None => text.push_str(
            "style: no layer has a style guide, so Style checks measure against the original art\n",
        ),
        Some(r) => {
            text.push_str(&format!(
                "style against `{}` ({}): {} measures, {failed} failed\n",
                r.theme,
                Deterministic.name(),
                measures.len()
            ));
            for x in measures.iter().filter(|x| !x.pass) {
                text.push_str(&format!(
                    "  {} {}: {:.3}, wants {}\n",
                    x.part, x.metric, x.value, x.bound
                ));
            }
        }
    }
    let (geo_json, geo_refused) = match &geometry {
        Ok((flags, checked)) => {
            let refuse = flags.iter().filter(|f| f.verdict == "refuse").count();
            text.push_str(&format!(
                "geometry: {checked} slots and stages checked, {} flags, {refuse} refused (tolerances {} px of place, {} of size, {} at the feet and the floor)\n",
                flags.len(),
                crate::theme_geometry::EDGE,
                crate::theme_geometry::SIZE,
                crate::theme_geometry::FEET
            ));
            // The same flag on several frames is one line.
            let mut lines: Vec<(String, String, Vec<String>)> = Vec::new();
            for f in flags {
                let what = format!(
                    "{}{}",
                    f.what,
                    if f.verdict == "refuse" {
                        " REFUSED"
                    } else {
                        ""
                    }
                );
                match lines.iter_mut().find(|l| l.0 == f.slot && l.1 == what) {
                    Some(l) => l.2.push(f.at.clone()),
                    None => lines.push((f.slot.clone(), what, vec![f.at.clone()])),
                }
            }
            for (slot, what, at) in lines {
                let at = if at.len() > 4 {
                    format!("{} frames ({}, ...)", at.len(), at[..3].join(", "))
                } else {
                    at.join(", ")
                };
                text.push_str(&format!("  {slot} {at}: {what}\n"));
            }
            (
                json!({"checked": checked, "flags": flags}),
                refuse > 0 && !args.allow_unreadable,
            )
        }
        Err(e) => {
            text.push_str(&format!("geometry: not checked ({e:#})\n"));
            (json!({"checked": 0, "why": format!("{e:#}")}), false)
        }
    };
    let refused = geo_refused || report.verdict == Verdict::Refuse && !args.allow_unreadable;
    Ok(Outcome {
        json: json!({
            "model": m.model.name,
            "layers": theme.describe(),
            "slots": slots.json,
            "readability": report,
            "geometry": geo_json,
            "style": {
                "reference": reference.as_ref().map(|r| r.theme.clone()),
                "measure": Deterministic.name(),
                "measures": measures,
                "failed": failed,
            },
        }),
        text: text.trim_end().to_owned(),
        exit: u8::from(refused || failed > 0),
    })
}

/// The geometry check (docs/theme-kit.md, "Gameplay geometry"), with the
/// lifted assets: the flags and the slots and stages checked.
fn geometry(
    m: &Model,
    dir: &Path,
    theme: &Resolved,
    bundle: &gp_interp::lifted::Bundle,
    assets: Option<&Path>,
    paths: &Paths,
) -> Result<(Vec<crate::theme_geometry::Flag>, usize)> {
    let adir = assets_dir(m, assets, paths)?;
    let (id, _, map_file) = crate::theme_kit::manifest(dir);
    let map_text = fs::read_to_string(map_file.unwrap_or_else(|| adir.join("assets.toml")))
        .context("no lifted assets; run gp template extract with your ROM")?;
    let map = gp_assets::AssetMap::parse(&map_text).map_err(anyhow::Error::msg)?;
    let (kit, _) = crate::theme_kit::kit_for(m, &id, dir)?;
    let local = crate::theme_kit::local_of(m, &kit, &adir, &map)?;
    let spec = gp_interp::compile::compile(m, &bundle.assets).map_err(anyhow::Error::msg)?;
    crate::theme_geometry::check(m, &kit, &local, theme, &adir, &map, &spec)
}

/// The originality check, added to a theme check's outcome.
pub(crate) fn originality(
    model: &Path,
    args: &ThemeArgs,
    assets: Option<&Path>,
    rom: Option<&Path>,
    paths: &Paths,
    rec: &mut RunRecorder,
    out: &mut Outcome,
) -> Result<()> {
    use crate::theme_original as o;
    let mut args = args.clone();
    if !args.any() {
        args.themes = true;
    }
    let (m, theme, _, bundle) = themed(model, &args, assets, paths, rec)?;
    let (_, dir) = read_model(model)?;
    let adir = assets_dir(&m, assets, paths)?;
    let (id, accepts, map_file) = crate::theme_kit::manifest(&dir);
    let map_text = fs::read_to_string(map_file.unwrap_or_else(|| adir.join("assets.toml")))?;
    let map = gp_assets::AssetMap::parse(&map_text).map_err(anyhow::Error::msg)?;
    let kit = crate::theme_kit::kit_of(&m, &id);
    let local = crate::theme_kit::local_of(&m, &kit, &adir, &map)?;
    let authored = fs::read_to_string(dir.join("template.toml"))
        .ok()
        .and_then(|t| t.parse::<toml::Table>().ok())
        .and_then(|t| {
            t.get("source")?
                .get("how")?
                .as_str()
                .map(|h| h == "authored")
        })
        .unwrap_or(false);
    let mut scores = o::art_scores(&m, &kit, &local, &theme, &adir, &map, authored)?;
    // Inputs that start the sound roles: the template's clips from power-on.
    let mut inputs = Vec::new();
    // gameprinter's games keep them in inputs/, game-printer's templates
    // in clips/.
    let mut files: Vec<PathBuf> = ["clips", "inputs"]
        .iter()
        .filter_map(|d| fs::read_dir(dir.join(d)).ok())
        .flat_map(|d| {
            d.filter_map(|e| e.ok().map(|e| e.path()))
                .collect::<Vec<_>>()
        })
        .collect();
    files.retain(|p| p.extension().is_some_and(|e| e == "presses"));
    files.sort();
    for f in files.iter().take(60) {
        if let Ok(text) = fs::read_to_string(f) {
            inputs.push(presses_frames(&text));
        }
    }
    let cues = o::original_cues(&bundle, &m, &inputs, (&dir, paths))?;
    scores.extend(o::sound_scores(&theme, &cues)?);
    // ROM bytes in the theme folders.
    let rom_bytes = match rom {
        Some(r) => Some(fs::read(r)?),
        None => std::env::var_os("GP_ROM_DIR").and_then(|d| {
            walk_files(Path::new(&d)).into_iter().find_map(|p| {
                let b = fs::read(&p).ok()?;
                (accepts.contains(&sha256_hex(&b))
                    || accepts.contains(&sha256_hex(b.get(16..).unwrap_or_default())))
                .then_some(b)
            })
        }),
    };
    let mut rom_hits = Vec::new();
    if let Some(r) = &rom_bytes {
        for l in &theme.layers {
            rom_hits.extend(o::rom_runs(&l.dir, r.get(16..).unwrap_or_default())?);
        }
    }
    let refused: Vec<&o::Score> = scores.iter().filter(|s| s.refused).collect();
    let judged = scores.iter().filter(|s| s.judged).count();
    let worst_art = scores
        .iter()
        .filter(|s| s.judged && s.shape.is_some())
        .map(|s| s.value)
        .fold(0.0, f64::max);
    let worst_shape = scores
        .iter()
        .filter(|s| s.judged)
        .filter_map(|s| s.shape)
        .fold(0.0, f64::max);
    let names_read = scores
        .iter()
        .filter(|s| s.what.ends_with(" text") && s.refused)
        .count();
    let worst_sound = scores
        .iter()
        .filter(|s| s.judged && s.what.starts_with("sound"))
        .map(|s| s.value)
        .fold(0.0, f64::max);
    out.text.push_str(&format!(
        "\noriginality: {} of {} measures judged, {} refused; art agreement at most {:.3} (refused at {}), shape at most {:.3} (refused at {}), both the best of shifts to {} px and the mirror; names read in titles and score displays: {}; melody at most {:.3} (refused at {}); ROM runs of {} bytes: {}",
        judged,
        scores.len(),
        refused.len(),
        worst_art,
        o::ART_REFUSE,
        worst_shape,
        o::SHAPE_REFUSE,
        o::SHIFT,
        names_read,
        worst_sound,
        o::MELODY_REFUSE,
        o::ROM_RUN,
        if rom_bytes.is_none() { "not checked (no ROM)".to_owned() } else { rom_hits.len().to_string() }
    ));
    for s in &scores {
        out.text.push_str(&format!(
            "\n  {} {:.3}{}{}{}",
            s.what,
            s.value,
            s.shape
                .map(|v| format!(" shape {v:.3}"))
                .unwrap_or_default(),
            if s.how.is_empty() {
                String::new()
            } else {
                format!(" ({})", s.how.chars().take(100).collect::<String>())
            },
            if s.refused {
                " REFUSED"
            } else if !s.judged {
                " (not judged)"
            } else {
                ""
            }
        ));
    }
    for h in &rom_hits {
        out.text.push_str(&format!("\n  ROM bytes in {h}"));
    }
    out.json["originality"] = json!({
        "art_refuse": o::ART_REFUSE,
        "shape_refuse": o::SHAPE_REFUSE,
        "shift": o::SHIFT,
        "art_min_pixels": o::ART_MIN_PIXELS,
        "melody_refuse": o::MELODY_REFUSE,
        "melody_min_steps": o::MELODY_MIN_STEPS,
        "rom_run": o::ROM_RUN,
        "scores": scores,
        "rom_files": rom_hits,
    });
    if !refused.is_empty() || !rom_hits.is_empty() {
        out.exit = 1;
    }
    Ok(())
}

fn walk_files(d: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = fs::read_dir(d) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                out.extend(walk_files(&p));
            } else {
                out.push(p);
            }
        }
    }
    out
}

/// A `.presses` file's frames: spans of buttons held, from power-on.
pub(crate) fn presses_frames(text: &str) -> Vec<gp_runtime::InputFrame> {
    use gp_evidence::input_log::button as b;
    let mut frames = 0usize;
    let mut pads = Vec::new();
    let mut spans = Vec::new();
    for l in text.lines().skip(1) {
        let l = l.trim();
        if let Some(n) = l.strip_prefix("frames ") {
            frames = n.trim().parse().unwrap_or(0);
            continue;
        }
        let Some((btn, r)) = l.split_once('@') else {
            continue;
        };
        let Some((a, z)) = r.split_once('-') else {
            continue;
        };
        let mut bits = 0u8;
        for k in btn.split('+') {
            bits |= match k {
                "a" => b::A,
                "b" => b::B,
                "select" => b::SELECT,
                "start" => b::START,
                "up" => b::UP,
                "down" => b::DOWN,
                "left" => b::LEFT,
                "right" => b::RIGHT,
                _ => 0,
            };
        }
        spans.push((
            bits,
            a.parse::<usize>().unwrap_or(0),
            z.parse::<usize>().unwrap_or(0),
        ));
    }
    pads.resize(frames, 0u8);
    for (bits, a, z) in spans {
        for p in pads.iter_mut().take(z + 1).skip(a) {
            *p |= bits;
        }
    }
    gp_explore::presses::inputs_of(&pads)
}

/// The paper scheme: colors by role.
fn paper() -> Scheme {
    let sh = |b: [u8; 3], l: [u8; 3], d: [u8; 3]| Shades {
        base: Rgb(b),
        light: Rgb(l),
        dark: Rgb(d),
    };
    let roles = [
        (
            "player",
            sh([0x5c, 0xa8, 0x4e], [0x9c, 0xd6, 0x7e], [0x2f, 0x5e, 0x2a]),
        ),
        (
            "enemy",
            sh([0xd0, 0x5a, 0x3c], [0xee, 0x9a, 0x76], [0x7a, 0x2e, 0x1e]),
        ),
        (
            "boss",
            sh([0x9a, 0x3c, 0x8a], [0xc8, 0x7c, 0xb8], [0x52, 0x1e, 0x4a]),
        ),
        (
            "hazard",
            sh([0xe8, 0xa0, 0x20], [0xf6, 0xd0, 0x6a], [0x8a, 0x5a, 0x0e]),
        ),
        (
            "projectile",
            sh([0xe8, 0xa0, 0x20], [0xf6, 0xd0, 0x6a], [0x8a, 0x5a, 0x0e]),
        ),
        (
            "collectible",
            sh([0xf2, 0xc8, 0x3a], [0xfa, 0xe8, 0x96], [0x9a, 0x76, 0x14]),
        ),
        (
            "power-up",
            sh([0xe2, 0x4e, 0x86], [0xf2, 0x98, 0xb8], [0x86, 0x22, 0x4c]),
        ),
        (
            "sky",
            sh([0xe9, 0xe2, 0xd0], [0xf6, 0xf1, 0xe4], [0xc9, 0xc0, 0xaa]),
        ),
        (
            "background",
            sh([0xd8, 0xd0, 0xbc], [0xea, 0xe4, 0xd4], [0xb4, 0xaa, 0x92]),
        ),
        (
            "terrain",
            sh([0x8e, 0x6c, 0x4a], [0xb8, 0x9a, 0x6e], [0x5a, 0x42, 0x2c]),
        ),
        (
            "platform",
            sh([0x8e, 0x6c, 0x4a], [0xb8, 0x9a, 0x6e], [0x5a, 0x42, 0x2c]),
        ),
        (
            "water",
            sh([0x6a, 0x9e, 0xc2], [0xa4, 0xc8, 0xe0], [0x3a, 0x62, 0x82]),
        ),
        (
            "goal",
            sh([0x4a, 0x6e, 0xb8], [0x8a, 0xa6, 0xdc], [0x26, 0x3c, 0x6e]),
        ),
    ];
    Scheme {
        roles: roles.iter().map(|(r, s)| ((*r).to_owned(), *s)).collect(),
        default: sh([0xb0, 0xa8, 0x98], [0xd4, 0xce, 0xc2], [0x6e, 0x66, 0x5a]),
        ink: Rgb([0x2a, 0x22, 0x1c]),
    }
}

/// The template's paper theme with the defaults, as `gp game new` writes
/// it.
// game-printer's `gp game new` painted a new game's paper theme with this;
// gameprinter's `game new` doesn't yet.
#[allow(dead_code)]
pub(crate) fn paint_model(
    model: &Path,
    out: &Path,
    paths: &Paths,
    rec: &mut RunRecorder,
) -> Result<()> {
    paint(
        model,
        out,
        "paper",
        "Paper",
        1,
        "PAPER PLAY",
        "MADE WITH GAME PRINTER",
        &[],
        true,
        paths,
        rec,
    )
    .map(|_| ())
}

/// The paper theme of a game drawn from tiles (theme_paper): the kit and
/// its local part from the lifted assets, the levels' boxes from the
/// compiled spec.
#[allow(clippy::too_many_arguments)]
fn paint_tiled(
    m: &Model,
    model: &Path,
    out: &Path,
    generator: &str,
    seed: u32,
    logo: &str,
    credit: &str,
    paths: &Paths,
    _rec: &mut RunRecorder,
) -> Result<crate::theme_paper::Out> {
    let (_, dir) = read_model(model)?;
    let source = m
        .model
        .source
        .clone()
        .context("the model names no source")?;
    let adir = paths.home.join("work").join("assets").join(&source);
    if !adir.join("assets.toml").is_file() {
        bail!("no lifted assets in {}", adir.display());
    }
    let bundle = crate::theme_run::load_bundle(model, &adir)?;
    let spec = gp_interp::compile::compile(m, &bundle.assets).map_err(anyhow::Error::msg)?;
    let (id, _, map_file) = crate::theme_kit::manifest(&dir);
    let map_text = fs::read_to_string(map_file.unwrap_or_else(|| adir.join("assets.toml")))
        .context("reading the asset map")?;
    let map = gp_assets::AssetMap::parse(&map_text).map_err(anyhow::Error::msg)?;
    let kit = crate::theme_kit::kit_of(m, &id);
    let local = crate::theme_kit::local_of(m, &kit, &adir, &map)?;
    crate::theme_paper::paint_tiles(m, &kit, &local, &spec, out, generator, seed, logo, credit)
}

#[allow(clippy::too_many_arguments)]
fn paint(
    model: &Path,
    out: &Path,
    id: &str,
    title: &str,
    seed: u32,
    logo: &str,
    credit: &str,
    also_for: &[String],
    force: bool,
    paths: &Paths,
    rec: &mut RunRecorder,
) -> Result<Outcome> {
    let (m, _) = read_model(model)?;
    if !gp_theme::format::is_name(id) {
        bail!("`{id}` isn't a theme id: lowercase letters, digits, - and _");
    }
    let mut for_models = vec![m.model.name.clone()];
    for n in also_for {
        if !gp_model::path::is_name(n) {
            bail!("--for `{n}` isn't a model name (a model's `model.name`)");
        }
        if !for_models.contains(n) {
            for_models.push(n.clone());
        }
    }
    if out.exists() && !force {
        bail!("{} exists; pass --force to write over it", out.display());
    }
    fs::create_dir_all(out).with_context(|| format!("creating {}", out.display()))?;
    let scheme = paper();
    let generator = format!("gp theme paint --id {id} --seed {seed}");
    let mut restyles = Vec::new();
    let write = |file: &str, img: &Rgba, what: &str| -> Result<String> {
        let text = gp_theme::pix::write(img, &format!("{what}, painted by {generator} (CC0-1.0)"))
            .map_err(anyhow::Error::msg)?;
        fs::write(out.join(file), &text).with_context(|| format!("writing {file}"))?;
        Ok(sha256_hex(text.as_bytes()))
    };
    // A game drawn from tiles gets its tiles, layouts and sounds painted
    // from its kit; one that isn't (or whose lift isn't here) a sprite per
    // entity role.
    let tiled = m
        .assets
        .values()
        .any(|s| s.kind == gp_model::model::AssetKind::Tiles);
    let mut notes = Vec::new();
    let tile_out = if tiled {
        match paint_tiled(&m, model, out, &generator, seed, logo, credit, paths, rec) {
            Ok(o) => Some(o),
            Err(e) => {
                notes.push(format!("painted sprites by role instead of tiles: {e:#}"));
                None
            }
        }
    } else {
        None
    };
    // A sprite per entity role, the size of the largest drawn sprite of the
    // role (or the entity's size, for a sprite drawn from rows).
    let mut sizes: BTreeMap<String, [u32; 2]> = BTreeMap::new();
    for e in m
        .entities
        .values()
        .filter(|_| tile_out.as_ref().is_none_or(|o| o.objects))
    {
        let Some(s) = &e.core.sprite else { continue };
        let size = if s.frame.is_some() || s.tiles.is_empty() {
            e.core.size.map(|v| (v.raw().max(0) >> 16) as u32)
        } else {
            let cols = s.width.max(1);
            [cols * 8, (s.tiles.len() as u32).div_ceil(cols) * 8]
        };
        let slot = sizes.entry(e.core.role.clone()).or_insert(size);
        *slot = [slot[0].max(size[0]), slot[1].max(size[1])];
    }
    for (role, [w, h]) in &sizes {
        let file = format!("{role}.pix");
        let img = gp_theme::paint::sprite(role, *w, *h, &scheme, seed);
        let sha256 = write(&file, &img, &format!("{role} sprite, {w} by {h}"))?;
        restyles.push(Restyle {
            target: Some(role.clone()),
            slot: None,
            entry: None,
            file,
            origin: "generated".into(),
            license: "CC0-1.0".into(),
            sha256,
            generator: Some(generator.clone()),
            author: None,
            note: None,
        });
    }
    // A cell per item role of each tileset.
    for (slot, s) in &m.assets {
        if s.kind != gp_model::model::AssetKind::Metatiles {
            continue;
        }
        let mut roles: Vec<&String> = s.item_roles.iter().collect();
        roles.sort();
        roles.dedup();
        for role in roles {
            let file = format!("{slot}-{role}.pix");
            let img = gp_theme::paint::cell(role, &scheme, seed);
            let sha256 = write(&file, &img, &format!("{role} cell of {slot}"))?;
            restyles.push(Restyle {
                target: Some(role.clone()),
                slot: Some(slot.clone()),
                entry: None,
                file,
                origin: "generated".into(),
                license: "CC0-1.0".into(),
                sha256,
                generator: Some(generator.clone()),
                author: None,
                note: None,
            });
        }
    }
    let (palette, sound, style_palette) = match tile_out {
        Some(o) => {
            restyles.extend(o.restyles);
            notes.extend(o.notes);
            (o.palette, o.sound, crate::theme_paper::paper_palette())
        }
        None => (
            Vec::new(),
            Vec::new(),
            scheme.colors().iter().map(Rgb::to_string).collect(),
        ),
    };
    let theme = Theme {
        format: gp_theme::FORMAT_VERSION.into(),
        id: id.into(),
        title: title.into(),
        description: Some(format!(
            "Paper cutouts in place of {}'s art, painted from its roles",
            m.model.title
        )),
        author: None,
        license: "CC0-1.0".into(),
        for_models: for_models.clone(),
        palette,
        effect: Vec::new(),
        restyle: restyles,
        audio: Vec::new(),
        sound,
        style: Some(StyleGuide {
            palette: style_palette,
            max_palette_distance: Some(4.0),
            edge_density: Some([0.1, 0.8]),
            size_tolerance: Some(0.35),
        }),
    };
    let also: String = for_models[1..]
        .iter()
        .map(|n| format!(" --for {n}"))
        .collect();
    let text = format!(
        "# Painted by {generator}{also} from {}'s roles (docs/formats/theme.md).\n{}",
        m.model.name,
        theme.to_text()
    );
    fs::write(out.join(gp_theme::format::THEME_FILE), &text).context("writing theme.toml")?;
    let problems = theme.check(out);
    if !problems.is_empty() {
        bail!("the painted theme doesn't check: {problems:?}");
    }
    Ok(Outcome::ok(
        json!({ "dir": out, "theme": id, "restyles": theme.restyle.len(), "sounds": theme.sound.len(), "notes": notes }),
        format!(
            "painted {} restyles and {} sounds for {} into {}{}",
            theme.restyle.len(),
            theme.sound.len(),
            for_models.join(" and "),
            out.display(),
            notes
                .iter()
                .map(|n| format!("\nnote: {n}"))
                .collect::<String>()
        ),
    ))
}

/// A sprite picture and whether it mirrors.
type Pic = Option<(Rgba, bool)>;

/// Draws an interpreter's world with themed art, on the CPU. It reads the
/// world and never writes it.
pub(crate) struct Renderer {
    levels: BTreeMap<String, (Rgba, [u8; 3])>,
    kinds: Vec<(Pic, Vec<Pic>)>,
    overlay: Option<Rgba>,
    emitters: Vec<(Option<String>, gp_theme::particles::Emitter)>,
    roles: BTreeMap<String, String>,
    clear: [u8; 3],
    pub view: [u32; 2],
}

impl Renderer {
    pub(crate) fn new(m: &Model, art: &Art, spec: &gp_sim::Spec) -> Result<Self> {
        let decode =
            |p: &gp_look::present::Picture| Rgba::from_png(&p.png).map_err(anyhow::Error::msg);
        let mut levels = BTreeMap::new();
        for (n, (p, clear)) in &art.levels {
            levels.insert(n.clone(), (decode(p)?, *clear));
        }
        let pic = |p: &Option<gp_look::present::SpritePic>| -> Result<Pic> {
            p.as_ref().map(|(p, f)| Ok((decode(p)?, *f))).transpose()
        };
        let mut kinds = Vec::new();
        for k in &spec.kinds {
            let e = art.sprites.get(&k.name).cloned().unwrap_or_default();
            kinds.push((
                pic(&e.base)?,
                e.states.iter().map(&pic).collect::<Result<_>>()?,
            ));
        }
        Ok(Self {
            levels,
            kinds,
            overlay: art.theme.overlay.as_ref().map(decode).transpose()?,
            emitters: art.theme.emitters.clone(),
            roles: art.roles.clone(),
            clear: art.clear,
            view: m
                .cameras
                .values()
                .next()
                .map(|c| c.size)
                .unwrap_or([256, 240]),
        })
    }

    /// The frame the world shows now.
    pub(crate) fn frame(&self, spec: &gp_sim::Spec, w: &gp_sim::World) -> Rgba {
        let px = |raw: i64| raw >> 16;
        let view = self.view;
        let level = &spec.levels[w.level].name;
        let (img, clear) = match self.levels.get(level) {
            Some((i, c)) => (Some(i), *c),
            None => (None, self.clear),
        };
        let [cx, cy] = w.cameras.first().copied().unwrap_or_default();
        let camera = [px(cx.raw()), px(cy.raw())];
        let mut sprites = Vec::new();
        for b in w.bodies.iter().filter(|b| b.active()) {
            let Some((base, states)) = self.kinds.get(b.kind) else {
                continue;
            };
            let Some((image, flip)) = states
                .get(b.state)
                .and_then(Option::as_ref)
                .or(base.as_ref())
            else {
                continue;
            };
            // Sprites stand on the body's bottom edge, as the generated
            // game draws them.
            let h = px(spec.kinds[b.kind].size[1].raw());
            sprites.push(Placed {
                image,
                x: px(b.x.raw()),
                y: px(b.y.raw()) + h - i64::from(image.height),
                flip: b.facing_left && *flip,
            });
        }
        let dots = self.dots(spec, w);
        gp_theme::compose::frame(&Scene {
            view,
            clear,
            level: img,
            camera,
            sprites,
            dots,
            overlay: self.overlay.as_ref(),
        })
    }

    /// The theme's particles this frame.
    fn dots(&self, spec: &gp_sim::Spec, w: &gp_sim::World) -> Vec<gp_theme::particles::Dot> {
        let px = |raw: i64| raw >> 16;
        let view = self.view;
        let [cx, cy] = w.cameras.first().copied().unwrap_or_default();
        let camera = [px(cx.raw()), px(cy.raw())];
        let mut dots = Vec::new();
        for (role, e) in &self.emitters {
            let boxes: Vec<[f32; 4]> = w
                .bodies
                .iter()
                .filter(|b| {
                    b.active()
                        && role
                            .as_ref()
                            .is_some_and(|r| self.roles.get(&spec.kinds[b.kind].name) == Some(r))
                })
                .map(|b| {
                    let k = &spec.kinds[b.kind];
                    [
                        (px(b.x.raw()) - camera[0]) as f32,
                        (px(b.y.raw()) - camera[1]) as f32,
                        px(k.size[0].raw()) as f32,
                        px(k.size[1].raw()) as f32,
                    ]
                })
                .collect();
            dots.extend(gp_theme::particles::dots(
                e,
                w.frame,
                [view[0] as f32, view[1] as f32],
                &boxes,
            ));
        }
        dots
    }
}

/// The game as the generated game and the web preview draw it: the
/// presentation's animator and scene (gp_present) with the themed
/// pictures, so the title, the HUD, every clip and effect show, then the
/// theme's particles and overlay. It reads the world and never writes it.
pub(crate) struct Shown {
    present: gp_present::Present,
    pics: Vec<Option<gp_present::raster::Image>>,
    anim: gp_present::Animator,
}

impl Shown {
    pub(crate) fn new(art: &Art) -> Result<Self> {
        let by_file: BTreeMap<&str, &[u8]> = art
            .pictures
            .iter()
            .map(|p| (p.file.as_str(), p.png.as_slice()))
            .collect();
        let pics = art
            .present
            .pictures
            .iter()
            .map(|p| {
                by_file
                    .get(p.file.as_str())
                    .map(|b| {
                        Rgba::from_png(b).map(|i| gp_present::raster::Image {
                            w: i.width,
                            h: i.height,
                            px: i.pixels,
                        })
                    })
                    .transpose()
                    .map_err(anyhow::Error::msg)
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            present: art.present.clone(),
            pics,
            anim: gp_present::Animator::new(),
        })
    }

    /// The visible part's size.
    fn size(&self) -> [u32; 2] {
        let v = self.present.screen.visible();
        [v[2], v[3]]
    }

    pub(crate) fn step(&mut self, rt: &crate::theme_run::Run, events: &[gp_sim::Event]) {
        self.anim.step(&self.present, rt.spec(), rt.world());
        if !events.is_empty() {
            self.anim
                .fired(&self.present, rt.spec(), rt.world(), events);
        }
    }

    pub(crate) fn frame(&self, r: &Renderer, rt: &crate::theme_run::Run) -> Rgba {
        let (spec, w) = (rt.spec(), rt.world());
        let scene = self.anim.scene(&self.present, spec, w);
        let f = gp_present::raster::draw(&self.present, w, &self.pics, &scene, &[], &[]);
        let [vl, vt, vw, vh] = f.visible;
        let mut out = Rgba::new(vw, vh);
        for y in 0..vh {
            for x in 0..vw {
                let c = f.rgb[((y + vt) * f.w + x + vl) as usize];
                out.set(x, y, [c[0], c[1], c[2], 255]);
            }
        }
        // Particles and the overlay, as the theme draws them.
        if r.overlay.is_some() || !r.emitters.is_empty() {
            let dots = r.dots(spec, w);
            gp_theme::compose::frame(&Scene {
                view: [vw, vh],
                clear: [0, 0, 0],
                level: Some(&out),
                camera: [0, 0],
                sprites: Vec::new(),
                dots,
                overlay: r.overlay.as_ref(),
            })
        } else {
            out
        }
    }
}

/// A frame's pixels as RGB, each repeated `s` times both ways.
fn scaled_rgb(frame: &Rgba, s: usize, out: &mut Vec<u8>) {
    let (vw, vh) = (frame.width as usize, frame.height as usize);
    out.clear();
    for y in 0..vh {
        let start = out.len();
        for x in 0..vw {
            let p = frame.pixels[y * vw + x];
            for _ in 0..s {
                out.extend_from_slice(&p[..3]);
            }
        }
        let row = out[start..].to_vec();
        for _ in 1..s {
            out.extend_from_slice(&row);
        }
    }
}

/// A world's state hash as a frame hash, as `gp interp run` hashes it.
pub(crate) fn frame_hash(rt: &crate::theme_run::Run) -> FrameHash {
    rt.frame_hash()
}

/// The model's sound roles' cues and a player holding the stack's sound
/// for each (`gp_look::sound::role_files`, as the generator writes
/// them), keeping a log of each cue.
fn role_player(
    m: &Model,
    spec: &gp_sim::Spec,
    theme: &Resolved,
) -> Result<(gp_present::sound::Cues, gp_present::sound::RolePlayer)> {
    let mut notes = Vec::new();
    let snd = m.present.as_ref().and_then(|p| p.sound.as_ref());
    let cues = gp_present::sound::Cues {
        roles: snd
            .map(|s| gp_look::sound::role_cues(s, spec, &mut notes))
            .unwrap_or_default(),
        silent: snd
            .map(|s| gp_look::sound::silent_cues(s, spec, &mut notes))
            .unwrap_or_default(),
        ..Default::default()
    };
    let mut player = gp_present::sound::RolePlayer::new(&cues);
    player.keep_log = true;
    for (role, wav) in gp_look::sound::role_files(&cues, theme, &mut notes) {
        let s = gp_present::sound::read_wav(&wav).map_err(anyhow::Error::msg)?;
        player.set(&cues, &role, s);
    }
    for n in notes {
        eprintln!("warning: sound: {n}");
    }
    Ok((cues, player))
}

#[allow(clippy::too_many_arguments)]
fn video(
    model: &Path,
    input: &InputArgs,
    args: &ThemeArgs,
    assets: Option<&Path>,
    out: &Path,
    scale: u8,
    force: bool,
    wav: Option<&Path>,
    paths: &Paths,
    rec: &mut RunRecorder,
) -> Result<Outcome> {
    let out = std::path::absolute(out).with_context(|| format!("resolving {}", out.display()))?;
    if let Some(repo) = gp_evidence::paths::enclosing_git_work_tree(&out) {
        bail!(
            "{} is inside the git work tree at {}; videos stay outside any repo",
            out.display(),
            repo.display()
        );
    }
    if out.exists() && !force {
        bail!(
            "{} already exists; pass --force to replace it",
            out.display()
        );
    }
    let (m, theme, art, bundle) = themed(model, args, assets, paths, rec)?;
    let report = readable(&art, &theme, args.allow_unreadable)?;
    let log = input.read(paths)?;
    let mut rt = crate::theme_run::Run::open(&bundle, &read_model(model)?.1, paths)?;
    let renderer = Renderer::new(&m, &art, rt.spec())?;
    let mut shown = Shown::new(&art)?;
    let s = usize::from(scale);
    let [vw, vh] = shown.size().map(|v| v as usize);
    let frames = crate::run::frames(&log);
    if let Some(d) = out.parent() {
        fs::create_dir_all(d).with_context(|| format!("creating {}", d.display()))?;
    }
    let file = fs::File::create(&out).with_context(|| format!("writing {}", out.display()))?;
    let fps = m.model.frame_rate.num as f64 / m.model.frame_rate.den.max(1) as f64;
    let mut apng = crate::theme_apng::Apng::new(
        BufWriter::new(file),
        vw * s,
        vh * s,
        u32::try_from(frames.len()).context("too many frames")?,
        (1000, (fps * 1000.0).round() as u16),
    )?;
    let mut hashes = vec![frame_hash(&rt)];
    let mut rgb = Vec::with_capacity(vw * vh * 3 * s * s);
    // The stack's sound per role, played on its cues as every runtime
    // plays it.
    let mut sound = match wav {
        Some(_) => Some(role_player(&m, rt.spec(), &theme)?),
        None => None,
    };
    let mut samples: Vec<i16> = Vec::new();
    shown.step(&rt, &[]);
    for f in &frames {
        let (events, _) = rt.advance(f).map_err(anyhow::Error::msg)?;
        hashes.push(frame_hash(&rt));
        if let Some((cues, player)) = &mut sound {
            samples.extend(player.frame(cues, rt.world(), gp_present::sound::ROLE_FRAME));
        }
        shown.step(&rt, &events);
        let frame = shown.frame(&renderer, &rt);
        scaled_rgb(&frame, s, &mut rgb);
        apng.frame(&rgb)?;
    }
    apng.finish()?;
    let digest = Hash::of(&frame_hashes::encode(&hashes));
    let bytes = fs::metadata(&out)?.len();
    rec.output("video", out.display().to_string(), Some(bytes));
    let mut cue_lines = String::new();
    let mut cues_json = Vec::new();
    if let (Some(w), Some((_, player))) = (wav, &sound) {
        fs::write(
            w,
            gp_theme::tune::wav(&samples, gp_present::sound::ROLE_RATE),
        )
        .with_context(|| format!("writing {}", w.display()))?;
        rec.output("wav", w.display().to_string(), None);
        cue_lines = format!(
            "wrote {:.1} s of sound per role to {}; cues by frame:\n",
            samples.len() as f64 / f64::from(gp_present::sound::ROLE_RATE),
            w.display()
        );
        for (frame, role) in &player.log {
            cue_lines.push_str(&format!("  {frame} {role}\n"));
            cues_json.push(json!([frame, role]));
        }
    }
    let mut text = format!(
        "wrote {} frames of {} to {} with {} theme layers\ndigest {digest}\n",
        frames.len(),
        m.model.name,
        out.display(),
        theme.layers.len()
    );
    text.push_str(&cue_lines);
    if let Some(r) = &report {
        text.push_str(&r.text());
    }
    Ok(Outcome::ok(
        json!({
            "model": m.model.name,
            "out": out,
            "frames": frames.len(),
            "bytes": bytes,
            "digest": digest,
            "cues": cues_json,
            "layers": theme.describe(),
            "readability": report,
        }),
        text.trim_end().to_owned(),
    ))
}
