//! `gp sprites capture`: full sprite capture into a model (docs/capture.md,
//! "Sprites"). The capture is `gp-sprites`; this module reads recordings'
//! traces and writes the files.
//!
//! On the NES it reads the sprite table, CHR banks and palette. On an
//! arcade game (port wave T, Midway MCR as in Tapper) it reads each
//! frame's sprite RAM through gp-arcade's sprite list
//! ([`gp_arcade::video::sprite_list`]) and finds each sprite's picture by
//! its number among the arcade lift's `mcr-sprites` (gfx2, 32 by 32):
//! a frame's piece is a whole sprite, 16 by 16 in the game's units, with
//! its colour bank as palette 4 to 7. A position the state mapping reads
//! is placed as the board places a sprite with those bytes (gp_sprites::
//! observe::Screen::Mcr). The sprite-set goes in the arcade map as a
//! `sprite-set` entry, as in an NES map. The board's colours are in its
//! palette RAM, not NES colour numbers, so no palettes are captured, and
//! `--apply` is refused: no model slot takes an arcade lift's pictures yet
//! (gp_interp::Lifted), so the model is only proposed.
//!
//! `sprites export`, `import` and `schema` are game-printer's, ported as
//! they were: sprite sheets out to image editors and back in
//! (docs/formats/image-config.md), written and read by `gp-imagefmt`. The
//! export's colours are the NES palette from gp-present (game-printer took
//! them from gp-gen-bevy, which is retired), and the image config's layers
//! are read by assets_open.rs's `image_layers`. On an arcade game both are
//! refused in words: a sheet holds four palettes of four colours with
//! colour 0 clear, as NES sprites are, and a Midway MCR sprite is one
//! 32 by 32 picture of sixteen values (clear where `value & 7` is 0).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use serde_json::json;

use gp_assets::map::AssetMap;
use gp_evidence::Paths;
use gp_model::Model;
use gp_sprites::chr::{Banks, TileBank};
use gp_sprites::config::Config;
use gp_sprites::observe::{ListSprite, Observer, Screen, Shot};
use gp_sprites::tables::Tables;

use crate::Outcome;
use crate::open::Opened;
use crate::recordings::parse_frames;

#[derive(Subcommand)]
pub enum SpritesCmd {
    /// Capture every sprite frame of every entity: the frames the game's
    /// sprite tables define (sprites.toml says where and in what shape) and
    /// every distinct frame the recordings show on each entity, merged into
    /// one sprite-set with each entity's clips by model state, their timing,
    /// palette variants and coverage. The set is written to the lifted
    /// assets; the model with its sprites is written beside it as a
    /// proposal, or over the template's with `--apply`
    Capture(CaptureArgs),
    /// Write an entity's frames (or every entity's) as image files in the
    /// layout a format config says: a sheet or a file per frame, as indexed or
    /// RGBA PNG with a palette file and a JSON sidecar, or as a Pixelorama
    /// project
    Export(ExportArgs),
    /// Read frames edited outside back in: checked against each frame's
    /// slot (palette indices, colors per tile, the tile budget, flips) and
    /// written as restyled art a theme can place
    Import(ImportArgs),
    /// Print the schema of the image format config as JSON
    /// (`schema/image-config.json`)
    Schema,
}

#[derive(Args)]
pub struct CaptureArgs {
    /// A template directory: model.toml, and mapping.toml and sprites.toml
    /// when the game has them.
    pub template: PathBuf,
    /// Store recordings (IDs with frame traces) whose sprites to capture;
    /// repeat for more. Without any, only the tables are read.
    #[arg(long = "recording", value_name = "ID")]
    pub recordings: Vec<String>,
    /// Which frames of each recording: `N`, `A..B`, `A..=B` or `A..`.
    #[arg(long, default_value = "1..")]
    pub frames: String,
    /// The model's lifted assets (default: $GP_HOME/work/assets/<source>).
    /// The sprite-set is written here.
    #[arg(long)]
    pub assets: Option<PathBuf>,
    /// Rewrite the template's model.toml with the captured sprites, instead
    /// of proposing it in $GP_HOME/work/sprites/<template>/model.toml.
    #[arg(long)]
    pub apply: bool,
}

pub(crate) fn run(cmd: &SpritesCmd, paths: &Paths) -> Result<Outcome> {
    match cmd {
        SpritesCmd::Capture(a) => capture(a, paths),
        SpritesCmd::Export(a) => export(a, paths),
        SpritesCmd::Import(a) => import(a, paths),
        SpritesCmd::Schema => {
            let s = gp_imagefmt::config::schema();
            Ok(Outcome::ok(
                serde_json::from_str(&s)?,
                s.trim_end().to_owned(),
            ))
        }
    }
}

/// The template's folder name, as the scorecard keys it.
fn template_id(dir: &Path) -> String {
    std::fs::canonicalize(dir)
        .unwrap_or_else(|_| dir.to_path_buf())
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn assets_dir(model: &Model, assets: Option<&Path>, paths: &Paths) -> Result<PathBuf> {
    if let Some(a) = assets {
        return Ok(a.to_path_buf());
    }
    let Some(source) = &model.model.source else {
        bail!("the model names no source, so give its lifted assets with --assets");
    };
    Ok(paths.home.join("work").join("assets").join(source))
}

/// Feeds a recording's frames to the observer: the RAM of frame `f` with
/// the sprites, palettes and banks of `f + lag`, which are what the
/// console drew from it.
fn observe_recording(
    obs: &mut Observer<'_>,
    opened: &Opened,
    range: std::ops::Range<u64>,
    lag: u64,
) -> Result<()> {
    let manifest = &opened.manifest;
    let table = gp_nes::recording::field_table(&opened.store, manifest)?;
    let last = manifest.frames;
    let end = (range.end.saturating_sub(1) + lag).min(last);
    let mut held: std::collections::VecDeque<(u64, Vec<u8>)> = Default::default();
    let mut failed = None;
    let ok = gp_nes::recording::scan_trace(
        &opened.store,
        manifest,
        range.start..=end,
        |frame, state| {
            let fields = match table.split(state) {
                Ok(f) => f,
                Err(e) => {
                    failed = Some(e.to_string());
                    return false;
                }
            };
            let get = |name: &str| fields.iter().find(|(n, _)| *n == name).map(|(_, b)| *b);
            let byte = |name: &str| get(name).and_then(|b| b.first().copied()).unwrap_or(0);
            let (Some(ram), Some(oam)) = (get("ram"), get("oam")) else {
                failed = Some("the trace has no `ram` and `oam` fields".into());
                return false;
            };
            held.push_back((frame, ram.to_vec()));
            if frame < range.start + lag {
                return true;
            }
            let (f, ram) = held.pop_front().expect("pushed above");
            if f >= range.end {
                return false;
            }
            let chr_ram = get("chr_ram").filter(|b| b.len() >= 0x2000);
            let mapper = get("mapper.number").map_or(0, |b| {
                u16::from_le_bytes([b[0], b.get(1).copied().unwrap_or(0)])
            });
            let mut mmc3 = [0u8; 9];
            mmc3[0] = byte("mmc3.bank_select");
            for (i, slot) in mmc3[1..].iter_mut().enumerate() {
                *slot = byte(&format!("mmc3.r{i}"));
            }
            let banks = Banks {
                chr_ram,
                mapper,
                mmc1: [byte("mmc1.control"), byte("mmc1.chr0"), byte("mmc1.chr1")],
                mmc3,
                cnrom: byte("cnrom.chr_bank"),
            };
            let palette = get("palette").unwrap_or(&[]);
            obs.shot(&Shot {
                frame: f,
                ram: &ram,
                oam,
                palette,
                ctrl: byte("ppu.ctrl"),
                banks,
            });
            true
        },
    )?;
    if let Some(e) = failed {
        bail!(e);
    }
    if !ok {
        bail!("{} has no frame trace", opened.short());
    }
    Ok(())
}

/// Feeds an arcade recording's frames to the observer: each frame's work
/// RAM with the sprites drawn from the same frame's sprite RAM (the board
/// has no lag between them).
fn observe_arcade(
    obs: &mut Observer<'_>,
    opened: &Opened,
    range: std::ops::Range<u64>,
) -> Result<()> {
    let manifest = &opened.manifest;
    let table = crate::arcade_rec::field_table_of(&opened.store, manifest)?;
    let mut failed = None;
    let ok = gp_nes::recording::scan_trace(
        &opened.store,
        manifest,
        range.start..=range.end.saturating_sub(1).min(manifest.frames),
        |frame, state| {
            let fields = match table.split(state) {
                Ok(f) => f,
                Err(e) => {
                    failed = Some(e.to_string());
                    return false;
                }
            };
            let get = |name: &str| fields.iter().find(|(n, _)| *n == name).map(|(_, b)| *b);
            let (Some(ram), Some(sprite_ram)) = (get("ram"), get("sprite_ram")) else {
                failed = Some("the trace has no `ram` and `sprite_ram` fields".into());
                return false;
            };
            // The mapping reads the main CPU's addresses: work RAM at E000.
            let mut space = vec![0u8; 0xe000 + ram.len()];
            space[0xe000..].copy_from_slice(ram);
            let list: Vec<ListSprite> = gp_arcade::video::sprite_list(sprite_ram)
                .into_iter()
                .map(|s| ListSprite {
                    x: i32::from(s.x),
                    y: i32::from(s.y),
                    code: u32::from(s.code),
                    color: s.color,
                    flip_x: s.flip_x,
                    flip_y: s.flip_y,
                })
                .collect();
            obs.shot_list(frame, &space, &list);
            true
        },
    )?;
    if let Some(e) = failed {
        bail!(e);
    }
    if !ok {
        bail!("{} has no frame trace", opened.short());
    }
    Ok(())
}

/// Whether the template is a package for an arcade board.
fn arcade_package(dir: &Path) -> Result<bool> {
    Ok(dir.join("package.toml").is_file()
        && crate::package::Package::load(dir)?.platform()? == crate::roms::Platform::Arcade)
}

fn capture(args: &CaptureArgs, paths: &Paths) -> Result<Outcome> {
    let dir = &args.template;
    let arcade = arcade_package(dir)?;
    if arcade && args.apply {
        bail!(
            "{} is an arcade game: no model slot takes an arcade lift's pictures yet (gp_interp::Lifted refuses them), so capture only proposes the model; leave out --apply",
            dir.display()
        );
    }
    let id = template_id(dir);
    let model_path = dir.join(gp_model::MODEL_FILE);
    let model = crate::gaps::load_model(dir)?;
    // Where the template's own files are, as its evidence cites them: a
    // template in this repo's templates/, or a package in the games
    // checkout by its folder.
    let at = if dir.join("package.toml").is_file() {
        id.clone()
    } else {
        format!("templates/{id}")
    };
    let source = model.model.source.clone().unwrap_or_default();
    let lifted = assets_dir(&model, args.assets.as_deref(), paths)?;
    let map_file = lifted.join("assets.toml");
    let map_text = std::fs::read_to_string(&map_file).with_context(|| {
        format!(
            "reading {} (lift the game's assets first: gp assets lift)",
            map_file.display()
        )
    })?;
    let arcade_map = if gp_assets::arcade::is_arcade(&map_text) {
        if !arcade {
            bail!(
                "{} is an arcade lift, and {} is an NES game",
                map_file.display(),
                dir.display()
            );
        }
        Some(
            gp_assets::arcade::ArcadeMap::parse(&map_text)
                .map_err(|e| anyhow::anyhow!("{}: {e}", map_file.display()))?,
        )
    } else {
        if arcade {
            bail!(
                "{} is an NES asset map, and {} is an arcade game",
                map_file.display(),
                dir.display()
            );
        }
        None
    };
    let map = match &arcade_map {
        Some(_) => None,
        None => Some(
            AssetMap::parse(&map_text)
                .map_err(|e| anyhow::anyhow!("{}: {e}", map_file.display()))?,
        ),
    };
    let config_file = dir.join("sprites.toml");
    let config = match std::fs::read_to_string(&config_file) {
        Ok(t) => {
            Config::parse(&t).map_err(|e| anyhow::anyhow!("{}: {e}", config_file.display()))?
        }
        Err(_) => Config::parse("schema = 1").expect("the empty config parses"),
    };
    let has_set = match (&map, &arcade_map) {
        (Some(m), _) => m
            .assets
            .iter()
            .any(|a| a.name() == config.asset && a.format() == "sprite-set"),
        (_, Some(m)) => m
            .assets
            .iter()
            .any(|a| a.name == config.asset && a.format == gp_assets::arcade::Format::SpriteSet),
        _ => false,
    };
    if !has_set {
        bail!(
            "the asset map has no sprite-set asset named `{}`; add `[[asset]] name = \"{}\" format = \"sprite-set\"` to assets/{source}.toml and lift again{}",
            config.asset,
            config.asset,
            if arcade_map.is_some() {
                " (an arcade map's pictures are already named `sprites`: give the set another name, and name it in sprites.toml as `asset` and `slot`)"
            } else {
                ""
            }
        );
    }
    let mapping = match std::fs::read_to_string(dir.join(gp_interp::mapping::MAPPING_FILE)) {
        Ok(t) => Some(gp_interp::mapping::Mapping::parse(&t).map_err(anyhow::Error::msg)?),
        Err(_) => None,
    };
    let bank = match (&map, &arcade_map) {
        (Some(m), _) => TileBank::load(&lifted, m, &model),
        (_, Some(m)) => TileBank::load_arcade(&lifted, m, &model),
        _ => unreachable!("one map or the other"),
    }
    .map_err(anyhow::Error::msg)?;

    // Entity IDs: the state mapping's slot kinds, then the config's.
    let mut ids: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    if let Some(s) = mapping.as_ref().and_then(|m| m.slots.as_ref()) {
        for k in &s.kinds {
            ids.entry(k.entity.clone()).or_default().extend(&k.ids);
        }
    }
    for (name, e) in &config.entities {
        ids.entry(name.clone()).or_default().extend(&e.ids);
    }
    let tables = Tables::read(&lifted, &config, &ids).map_err(anyhow::Error::msg)?;

    let mut obs =
        Observer::new(&model, mapping.as_ref(), &config, &bank).map_err(anyhow::Error::msg)?;
    if arcade {
        obs.set_screen(Screen::Mcr);
    }
    let mut refs = Vec::new();
    for r in &args.recordings {
        let opened = Opened::open(paths, r)?;
        if opened.is_arcade() != arcade {
            bail!(
                "{} is {}, and {} is a recording of {}: capture reads a game's own original",
                dir.display(),
                if arcade {
                    "an arcade game"
                } else {
                    "an NES game"
                },
                opened.short(),
                if opened.is_arcade() {
                    "an arcade romset"
                } else {
                    "an NES ROM"
                }
            );
        }
        if !arcade {
            crate::track::nes_recording(&opened, "gp sprites capture")?;
        }
        let range = parse_frames(&args.frames, opened.manifest.frames)?;
        let range = range.start.max(1)..range.end;
        if arcade {
            observe_arcade(&mut obs, &opened, range)
        } else {
            observe_recording(&mut obs, &opened, range, config.oam_lag)
        }
        .with_context(|| format!("reading {}", opened.short()))?;
        refs.push(gp_model::model::Ref {
            at: format!("recording {}", opened.short()),
            within: None,
            quote: None,
        });
    }
    if config_file.exists() {
        refs.push(gp_model::model::Ref {
            at: format!("{at}/sprites.toml:1"),
            within: None,
            quote: None,
        });
    }
    // GP_SPRITES_DEBUG=<entity> prints what was seen of one entity.
    if let Ok(e) = std::env::var("GP_SPRITES_DEBUG") {
        let mut counts: BTreeMap<(String, u32, [i32; 2]), usize> = BTreeMap::new();
        for o in obs.obs.iter().filter(|o| o.entity == e) {
            *counts
                .entry((o.state.clone(), o.layout, o.anchor))
                .or_default() += 1;
        }
        for ((st, l, a), n) in counts {
            let p: Vec<String> = obs.layouts[l as usize]
                .iter()
                .map(|p| {
                    format!(
                        "{}@{},{}p{}{}{}",
                        p.tile,
                        p.x,
                        p.y,
                        p.palette,
                        if p.flip_x { "h" } else { "" },
                        if p.flip_y { "v" } else { "" }
                    )
                })
                .collect();
            eprintln!("{e} {st:?} layout {l} at {a:?}: {n} [{}]", p.join(" "));
        }
    }
    let pattern = gp_sprites::pattern_map(&obs.pattern);
    let mut cap = gp_sprites::build::build(
        &model,
        &config,
        &tables,
        &bank,
        &obs.layouts,
        &obs.obs,
        &obs.level_colors,
        &pattern,
    );
    let mut out = model.clone();
    let rows = gp_sprites::emit::rows_reader(&model, &lifted);
    let written = gp_sprites::emit::write(
        &mut out,
        &mut cap,
        &config.slot,
        &source,
        &rows,
        &bank,
        &refs,
        &lifted,
    );
    // The set goes with the lifted assets; the model as asked.
    let set_file = lifted.join(format!("{}.toml", config.asset));
    std::fs::write(&set_file, cap.set.to_text())
        .with_context(|| format!("writing {}", set_file.display()))?;
    let model_out = if args.apply {
        model_path.clone()
    } else {
        let d = paths.home.join("work").join("sprites").join(&id);
        std::fs::create_dir_all(&d)?;
        d.join("model.toml")
    };
    let mut text_out = out.to_toml();
    if args.apply {
        // Evidence that quotes a line of this model file follows the line.
        let old_text = std::fs::read_to_string(&model_path)?;
        let rel = format!("{at}/model.toml:");
        if follow_self_refs(&mut out, &old_text, &text_out, &rel) {
            text_out = out.to_toml();
        }
    }
    std::fs::write(&model_out, text_out)
        .with_context(|| format!("writing {}", model_out.display()))?;

    // Coverage, per entity and in all.
    let mut frames = 0usize;
    let (mut states, mut covered, mut unobserved, mut observed_only) =
        (0usize, 0usize, 0usize, 0usize);
    let mut text = String::new();
    let mut per = serde_json::Map::new();
    for (name, es) in &cap.entities {
        let c = &es.coverage;
        frames += c.frames;
        states += c.states.len();
        covered += c.covered.len();
        unobserved += c.unobserved.len();
        observed_only += c.observed_only;
        text.push_str(&format!(
            "{name}: {} frames, states {}/{} ({}), {} observations{}{}\n",
            c.frames,
            c.covered.len(),
            c.states.len(),
            if c.covered.is_empty() {
                "none".into()
            } else {
                c.covered.join(", ")
            },
            c.observations,
            if c.unobserved.is_empty() {
                String::new()
            } else {
                format!(", never seen: {}", c.unobserved.join(", "))
            },
            if c.observed_only > 0 {
                format!(", {} seen in no table", c.observed_only)
            } else {
                String::new()
            },
        ));
        for t in &c.timings {
            text.push_str(&format!("  {t}\n"));
        }
        for v in &c.variants {
            text.push_str(&format!("  {v}\n"));
        }
        if !c.unmodelled.is_empty() {
            text.push_str(&format!(
                "  states the model doesn't have: {}\n",
                c.unmodelled.join(", ")
            ));
        }
        per.insert(name.clone(), serde_json::to_value(c)?);
    }
    let no_frames: Vec<&String> = model
        .entities
        .keys()
        .filter(|n| !cap.entities.get(*n).is_some_and(|e| e.coverage.frames > 0))
        .collect();
    text.push_str(&format!(
        "\n{frames} frames for {} entities; states with frames {covered}/{states}; {unobserved} table frames never seen; {observed_only} frames seen in no table; {} sprites with a tile no lifted asset holds; {} table pieces unmatched; {} entity frames left out (a state that froze the world, or a skip); {} frames in no level of the model\n",
        cap.entities.len(),
        obs.unresolved,
        cap.unresolved_pieces,
        obs.skipped,
        obs.outside
    ));
    if !no_frames.is_empty() {
        text.push_str(&format!(
            "no frames for: {}\n",
            no_frames
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    text.push_str(&format!(
        "sprite-set: {}\nmodel: {}{}",
        set_file.display(),
        model_out.display(),
        if args.apply {
            ""
        } else {
            " (a proposal; --apply writes the template's)"
        }
    ));
    Ok(Outcome::ok(
        json!({
            "template": id,
            "sprite_set": set_file,
            "model": model_out,
            "applied": args.apply,
            "frames": frames,
            "states": states,
            "states_covered": covered,
            "unobserved": unobserved,
            "observed_only": observed_only,
            "unresolved_sprites": obs.unresolved,
            "skipped": obs.skipped,
            "outside": obs.outside,
            "entities": per,
            "entities_without_frames": no_frames,
            "written": written,
        }),
        text,
    ))
}

/// Moves evidence refs `<rel><line>` with a quote to the line of the new
/// text that holds the quote under the same table header as before.
fn follow_self_refs(model: &mut Model, old: &str, new: &str, rel: &str) -> bool {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    let header = |lines: &[&str], n: usize| -> Option<String> {
        lines[..n.min(lines.len())]
            .iter()
            .rev()
            .find(|l| l.starts_with('['))
            .map(|l| (*l).to_owned())
    };
    let mut changed = false;
    for ev in model.evidence.values_mut() {
        for r in &mut ev.refs {
            let (Some(line), Some(q)) = (r.at.strip_prefix(rel), r.quote.clone()) else {
                continue;
            };
            let Ok(n) = line.parse::<usize>() else {
                continue;
            };
            if new_lines.get(n - 1).is_some_and(|l| l.contains(&q)) {
                continue;
            }
            let Some(h) = header(&old_lines, n) else {
                continue;
            };
            let Some(start) = new_lines.iter().position(|l| **l == h) else {
                continue;
            };
            if let Some(k) = new_lines[start..]
                .iter()
                .take_while(|l| **l == h || !l.starts_with('['))
                .position(|l| l.contains(&q))
            {
                r.at = format!("{rel}{}", start + k + 1);
                changed = true;
            }
        }
    }
    changed
}

#[derive(Args)]
pub struct ExportArgs {
    /// A template directory holding model.toml.
    pub template: PathBuf,
    /// Only this entity's frames.
    #[arg(long)]
    pub entity: Option<String>,
    /// A format config (docs/formats/image-config.md); without it the
    /// project's, then $GP_HOME/config/image.toml, then the defaults.
    #[arg(long)]
    pub config: Option<PathBuf>,
    /// The model's lifted assets (default: $GP_HOME/work/assets/<source>).
    #[arg(long)]
    pub assets: Option<PathBuf>,
    /// Where to write (default: $GP_HOME/work/sprites/<template>/export).
    #[arg(short, long)]
    pub out: Option<PathBuf>,
}

#[derive(Args)]
pub struct ImportArgs {
    /// A template directory holding model.toml.
    pub template: PathBuf,
    /// The directory `gp sprites export` wrote, edited.
    #[arg(long = "from")]
    pub from: PathBuf,
    /// Only this entity's frames.
    #[arg(long)]
    pub entity: Option<String>,
    /// The format config the files were written with (else the sidecar's).
    #[arg(long)]
    pub config: Option<PathBuf>,
    /// The model's lifted assets (default: $GP_HOME/work/assets/<source>).
    #[arg(long)]
    pub assets: Option<PathBuf>,
    /// Where to write the checked art as a theme (default:
    /// $GP_HOME/themes/<template>-sprites).
    #[arg(short, long)]
    pub out: Option<PathBuf>,
    /// The edited art's license, an SPDX expression.
    #[arg(long, default_value = "CC0-1.0")]
    pub license: String,
}

/// The model's sprite-set slot and its file in the lifted assets.
fn sprite_set(model: &Model, lifted: &Path) -> Result<(String, gp_assets::sprite_set::SpriteSet)> {
    // The model's slot, else the set capture wrote under its default name.
    let (slot, asset) = match model
        .assets
        .iter()
        .find(|(_, s)| s.kind == gp_model::model::AssetKind::SpriteSet)
    {
        Some((slot, s)) => (
            slot.clone(),
            s.from.as_ref().map_or(slot.clone(), |f| f.asset.clone()),
        ),
        None => ("sprites".to_owned(), "sprites".to_owned()),
    };
    let file = lifted.join(format!("{asset}.toml"));
    let text = std::fs::read_to_string(&file)
        .with_context(|| format!("reading {} (gp sprites capture writes it)", file.display()))?;
    let set = gp_assets::sprite_set::SpriteSet::parse(&text)
        .map_err(|e| anyhow::anyhow!("{}: {e}", file.display()))?;
    Ok((slot, set))
}

/// The 16 sprite palette colors frames are exported in: the set's
/// default palettes.
fn export_palette(set: &gp_assets::sprite_set::SpriteSet) -> [[u8; 3]; 16] {
    let rgb = gp_present::palette::NES;
    let colors: Vec<u8> = set
        .sets
        .iter()
        .find(|s| s.levels.is_empty())
        .or(set.sets.first())
        .map(|s| s.colors.clone())
        .unwrap_or_else(|| [0x0F, 0x00, 0x10, 0x30].repeat(4));
    std::array::from_fn(|i| rgb[usize::from(colors.get(i).copied().unwrap_or(0x0F) & 0x3F)])
}

fn export(args: &ExportArgs, paths: &Paths) -> Result<Outcome> {
    let dir = &args.template;
    let id = template_id(dir);
    let model = crate::mechanics::load_model(dir)?;
    refuse_arcade(dir)?;
    let lifted = assets_dir(&model, args.assets.as_deref(), paths)?;
    let map_text = std::fs::read_to_string(lifted.join("assets.toml"))
        .with_context(|| format!("reading {}", lifted.join("assets.toml").display()))?;
    let map = AssetMap::parse(&map_text).map_err(|e| anyhow::anyhow!("{e}"))?;
    let bank = TileBank::load(&lifted, &map, &model).map_err(anyhow::Error::msg)?;
    let (_, set) = sprite_set(&model, &lifted)?;
    let layers = crate::assets_open::image_layers(paths, args.config.as_deref())?;
    let cfg = gp_imagefmt::config::resolve(&layers, Some(&id), args.entity.as_deref())
        .map_err(anyhow::Error::msg)?;
    let palette = export_palette(&set);
    let mut frames = Vec::new();
    let mut skipped = 0;
    for (i, f) in set.frames.iter().enumerate() {
        if args.entity.as_ref().is_some_and(|e| *e != f.entity) || f.pieces.is_empty() {
            continue;
        }
        let [l, t, r, b] = f.bounds();
        let (w, h) = ((r - l) as u32, (b - t) as u32);
        let mut px = vec![0u8; (w * h) as usize];
        let mut tiles = Vec::new();
        let mut ok = true;
        for p in &f.pieces {
            let Some(asset) = bank.assets.iter().find(|a| {
                p.slot
                    .as_deref()
                    .is_some_and(|s| a.slot.as_deref() == Some(s) || a.name == s)
            }) else {
                ok = false;
                continue;
            };
            let Some(tile) = asset.tiles.get(p.tile as usize) else {
                ok = false;
                continue;
            };
            let (ox, oy) = ((p.x - l) as u32, (p.y - t) as u32);
            let pal = p.palette.saturating_sub(4) & 3;
            for y in 0..8u32 {
                for x in 0..8u32 {
                    let sx = if p.flip_x { 7 - x } else { x };
                    let sy = if p.flip_y { 7 - y } else { y };
                    let c = tile[(sy * 8 + sx) as usize];
                    if c != 0 {
                        px[((oy + y) * w + ox + x) as usize] = pal * 4 + c;
                    }
                }
            }
            tiles.push(gp_imagefmt::sheet::Tile {
                x: ox,
                y: oy,
                palette: pal,
                flip_x: p.flip_x,
                flip_y: p.flip_y,
            });
        }
        if !ok {
            skipped += 1;
        }
        frames.push(gp_imagefmt::sheet::Frame {
            name: cfg.name(&f.entity, &f.state, i as u32),
            entity: f.entity.clone(),
            state: f.state.clone(),
            set_index: i as u32,
            width: w,
            height: h,
            origin: [l, t],
            tiles,
            px,
        });
    }
    if frames.is_empty() {
        bail!(
            "no frames to export{}",
            args.entity
                .as_ref()
                .map(|e| format!(" for `{e}`"))
                .unwrap_or_default()
        );
    }
    let out = args.out.clone().unwrap_or_else(|| {
        paths
            .home
            .join("work")
            .join("sprites")
            .join(&id)
            .join("export")
    });
    std::fs::create_dir_all(&out)?;
    let files = gp_imagefmt::sheet::write(&cfg, &id, &frames, &palette);
    for (name, bytes) in &files {
        std::fs::write(out.join(name), bytes).with_context(|| format!("writing {name}"))?;
    }
    let names: Vec<&String> = files.iter().map(|(n, _)| n).collect();
    Ok(Outcome::ok(
        json!({ "out": out, "frames": frames.len(), "files": names, "config": cfg, "missing_tiles": skipped }),
        format!(
            "{} frames into {} ({} files: {}){}",
            frames.len(),
            out.display(),
            files.len(),
            names
                .iter()
                .take(6)
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            if skipped > 0 {
                format!("; {skipped} frames name tiles no lifted asset holds")
            } else {
                String::new()
            }
        ),
    ))
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn import(args: &ImportArgs, paths: &Paths) -> Result<Outcome> {
    let dir = &args.template;
    let id = template_id(dir);
    let model = crate::mechanics::load_model(dir)?;
    refuse_arcade(dir)?;
    let from = &args.from;
    let budget = match &args.config {
        Some(_) => {
            let layers = crate::assets_open::image_layers(paths, args.config.as_deref())?;
            Some(
                gp_imagefmt::config::resolve(&layers, Some(&id), args.entity.as_deref())
                    .map_err(anyhow::Error::msg)?
                    .tile_budget,
            )
        }
        None => None,
    };
    let got = gp_imagefmt::sheet::read(&|n| std::fs::read(from.join(n)).ok(), budget)
        .map_err(anyhow::Error::msg)?;
    let side: gp_imagefmt::sheet::Sidecar =
        serde_json::from_slice(&std::fs::read(from.join(gp_imagefmt::sheet::SIDECAR))?)?;
    let palette: [[u8; 3]; 16] = std::array::from_fn(|i| {
        side.palette
            .get(i)
            .and_then(|h| gp_imagefmt::palette::parse_hex(h))
            .unwrap_or([0, 0, 0])
    });
    let frames: Vec<&gp_imagefmt::sheet::Frame> = got
        .frames
        .iter()
        .filter(|f| args.entity.as_ref().is_none_or(|e| *e == f.entity))
        .collect();
    let mut text = format!(
        "{} frames read; {} distinct tiles ({} saved by flips)\n",
        frames.len(),
        got.tiles,
        got.saved_by_flips
    );
    for p in &got.problems {
        text.push_str(&format!(
            "  {}: {}\n",
            if p.frame.is_empty() { "all" } else { &p.frame },
            p.message
        ));
    }
    if !got.problems.is_empty() {
        let mut o = Outcome::ok(
            json!({ "problems": got.problems, "tiles": got.tiles }),
            text + "refused: fix the problems and import again",
        );
        o.exit = 1;
        return Ok(o);
    }
    // A theme layer restyling each entity's role with its own frame (the
    // frame its sprite shows when no clip holds), every frame beside it.
    let theme_id =
        format!("{id}-sprites").replace(|c: char| !c.is_ascii_alphanumeric() && c != '-', "-");
    let out = args
        .out
        .clone()
        .unwrap_or_else(|| paths.home.join("themes").join(&theme_id));
    std::fs::create_dir_all(&out)?;
    let mut toml_text = format!(
        "# Written by gp sprites import from {} (docs/formats/image-config.md).\nformat = \"0.1.0\"\nid = \"{theme_id}\"\ntitle = \"{id} sprites\"\ndescription = \"Sprites edited outside and imported\"\nlicense = \"{}\"\nfor = [\"{}\"]\n",
        from.display(),
        args.license,
        model.model.name
    );
    let mut restyled = Vec::new();
    for f in &frames {
        std::fs::write(
            out.join(format!("{}.png", f.name)),
            gp_imagefmt::sheet::rgba_png(f, &palette),
        )?;
    }
    let mut entities: Vec<&str> = frames.iter().map(|f| f.entity.as_str()).collect();
    entities.sort();
    entities.dedup();
    let mut roles_done = std::collections::BTreeSet::new();
    for e in entities {
        let Some(ent) = model.entities.get(e) else {
            continue;
        };
        let base = ent
            .core
            .sprite
            .as_ref()
            .and_then(|s| s.frame.as_ref())
            .map(|f| f.row);
        let pick = frames
            .iter()
            .find(|f| f.entity == e && Some(f.set_index) == base)
            .or_else(|| frames.iter().find(|f| f.entity == e));
        let Some(f) = pick else { continue };
        if !roles_done.insert(ent.core.role.clone()) {
            continue;
        }
        let file = format!("{}.png", f.name);
        let sha = sha256_hex(&std::fs::read(out.join(&file))?);
        toml_text.push_str(&format!(
            "\n[[restyle]]\ntarget = \"{}\"\nfile = \"{file}\"\norigin = \"drawn\"\nlicense = \"{}\"\nsha256 = \"{sha}\"\ngenerator = \"gp sprites import\"\nnote = \"{e}'s own frame; a restyle by state and frame is an open theme row, so its other frames are beside it\"\n",
            ent.core.role, args.license
        ));
        restyled.push(format!("{} ({e})", ent.core.role));
    }
    std::fs::write(out.join("theme.toml"), toml_text)?;
    text.push_str(&format!(
        "theme {} in {}: restyles {}",
        theme_id,
        out.display(),
        if restyled.is_empty() {
            "nothing".into()
        } else {
            restyled.join(", ")
        }
    ));
    Ok(Outcome::ok(
        json!({ "theme": out, "frames": frames.len(), "tiles": got.tiles, "saved_by_flips": got.saved_by_flips, "restyled": restyled }),
        text,
    ))
}

/// A sheet is NES-shaped (four palettes of four colours, colour 0 clear);
/// an arcade package's sprites are refused in words until it has a
/// 16-value form.
fn refuse_arcade(dir: &Path) -> Result<()> {
    if arcade_package(dir)? {
        bail!(
            "{} is an arcade game: a sprite sheet holds four palettes of four colours with colour 0 clear, as NES sprites are, and a Midway MCR sprite is a 32 by 32 picture of sixteen values, so its sprites aren't exported or imported yet",
            dir.display()
        );
    }
    Ok(())
}
