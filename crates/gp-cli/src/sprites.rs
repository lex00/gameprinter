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
//! `sprites schema` prints the image format config's schema
//! (docs/formats/image-config.md), from `gp-imagefmt`.

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
