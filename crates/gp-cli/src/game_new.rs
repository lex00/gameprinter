//! `gp game new GAME --rom ROM --genre G`: a new game's package folder in a
//! games checkout, everything it needs before its first clip. Brought over
//! from game-printer's command of the same name, which wrote into the tool
//! repo's templates/ and sources/; a game is a package in the games repo
//! now, so this writes the package's own files instead.
//!
//! | Written | What |
//! |---|---|
//! | `package.toml` | The package: its name, version 0.1.0, the ROM entry with its pins |
//! | `roms.toml` | The ROM's `[[source]]`: its hashes and header facts, unverified |
//! | `template.toml` | The template manifest, pinned to the ROM, the asset map and the lift |
//! | `assets/SOURCE.toml` | An asset map of what the iNES header says: the CHR ROM as tiles, when there is one. For an arcade game, the board's: every tile and sprite by region and index (gp_assets::arcade) |
//! | `model.toml` | The genre's header, the pad, a player and an empty level, each a placeholder its evidence says is one |
//! | `mapping.toml`, `observe.toml`, `probes.toml`, `profiles.toml`, `fidelity.toml` | Empty or default, each saying what goes in it |
//! | `corpus.toml` | The corpus plan with no clips |
//! | `$GP_HOME/mechanics/GAME/` | The quest log seeded from the genre's checklist (mechanic/checklists.toml), and a mechanic session for each row |
//!
//! Nothing is written over: a folder that exists is refused, and the quest
//! log only gains the rows it lacks. No ROM byte is written; the lift's
//! digest is a hash of what the ROM gives, made here. `--source` names the
//! ROM's source ID (default: the game's id), and the genres are game-
//! printer's: platformer, topdown, gridhop, events, and other.
//!
//! On an NES ROM the state mapping gets the draw sites `gp interp draws
//! --find` finds on an idle run from power-on, with `score_draws`, for
//! review (`--no-draws` leaves the search out), as game-printer's did. It
//! writes the level kit's stub rules and the kit (docs/level-kit.md). Left
//! for later steps (~/.gameprinter/notes/bring-PLAN.md): the theme kit;
//! `--branch` and the loop.toml it wrote went with `gp loop`.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use gp_evidence::Paths;
use gp_evidence::source::NesImage;
use gp_model::model::Device;
use serde_json::json;

use crate::Outcome;
use crate::template::sha256_hex;

/// What `gp game new` was asked.
pub struct New<'a> {
    pub game: &'a str,
    pub rom: &'a Path,
    pub genre: &'a str,
    pub title: Option<&'a str>,
    /// The package's namespace (`NS/GAME`).
    pub namespace: Option<&'a str>,
    /// The games checkout.
    pub games: Option<&'a Path>,
    /// The source ID the ROM is registered under (default: the game's id).
    pub source: Option<&'a str>,
    /// Today, for the evidence lines.
    pub date: &'a str,
    /// Search an idle run for draw sites (an NES ROM's).
    pub draws: bool,
}

/// How many frames of no input the draw search runs from power-on: a
/// minute, long enough for most attract demos to start.
const IDLE_FRAMES: usize = 3600;

/// The draw sites an idle run of an NES image finds, as `[[draws]]`
/// entries, and a line saying what was found.
fn idle_draws(bytes: &[u8]) -> (Option<String>, usize, String) {
    let idle = vec![gp_evidence::input_log::InputFrame::default(); IDLE_FRAMES];
    let found = if gp_arcade::romset::looks_like_zip(bytes) {
        gp_arcade::Romset::from_zip(bytes)
            .map_err(anyhow::Error::from)
            .and_then(|set| crate::draws_find_z80::find(&set, &idle, 0))
    } else {
        crate::draws_find::find(bytes, &idle, 0)
    };
    let draws = match found {
        Ok(f) if f.proposed().next().is_some() => Some(crate::draws_find::proposal(&f, &[])),
        Ok(_) => None,
        Err(e) => return (None, 0, format!("the idle run failed: {e}")),
    };
    let n = draws
        .as_deref()
        .map_or(0, |d| d.matches("[[draws]]").count());
    let line = if n > 0 {
        format!(
            "{n} draw sites found on {IDLE_FRAMES} idle frames, in mapping.toml with score_draws for review"
        )
    } else {
        format!(
            "none found on {IDLE_FRAMES} idle frames: run `gp interp draws --find ROM --input LOG` on the first clips"
        )
    };
    (draws, n, line)
}

/// The genres a skeleton can start from.
pub const GENRES: &[&str] = &["platformer", "topdown", "gridhop", "events", "other"];

/// What a genre gives the model's header.
struct Shape {
    view: &'static str,
    extension: Option<&'static gp_model::platformer::Extension>,
}

fn shape(genre: &str) -> Shape {
    let ext = |n: &str| gp_model::validate::EXTENSIONS.iter().find(|e| e.name == n);
    match genre {
        "platformer" => Shape {
            view: "side",
            extension: ext("platformer"),
        },
        "topdown" => Shape {
            view: "top-down",
            extension: ext("topdown"),
        },
        // A hop on a grid of cells (Q*bert): no extension yet, and the
        // model's view has no `isometric`, so top-down.
        "gridhop" => Shape {
            view: "top-down",
            extension: None,
        },
        _ => Shape {
            view: "side",
            extension: None,
        },
    }
}

fn name_ok(game: &str) -> bool {
    !game.is_empty()
        && game.len() <= 64
        && game
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !game.starts_with('-')
        && !game.ends_with('-')
}

/// The model's name: the game's id with `_` for `-`.
fn model_name(game: &str) -> String {
    game.replace('-', "_")
}

/// A TOML string, quoted.
fn q(s: &str) -> String {
    toml::Value::String(s.to_owned()).to_string()
}

/// The namespace the other packages in the checkout share, if they agree.
fn shared_namespace(games: &Path) -> Option<String> {
    let mut seen = std::collections::BTreeSet::new();
    for e in fs::read_dir(games).ok()?.filter_map(Result::ok) {
        let Ok(text) = fs::read_to_string(e.path().join("package.toml")) else {
            continue;
        };
        let Ok(t) = text.parse::<toml::Table>() else {
            continue;
        };
        if t.get("package")
            .and_then(|p| p.get("kind"))
            .and_then(toml::Value::as_str)
            != Some("game")
        {
            continue;
        }
        if let Some((ns, _)) = t
            .get("package")
            .and_then(|p| p.get("name"))
            .and_then(toml::Value::as_str)
            .and_then(|n| n.split_once('/'))
        {
            seen.insert(ns.to_owned());
        }
    }
    (seen.len() == 1).then(|| seen.into_iter().next().unwrap_or_default())
}

pub fn run(a: &New<'_>, paths: &Paths) -> Result<Outcome> {
    let game = a.game;
    if !name_ok(game) {
        bail!("a game's id is lower-case letters, digits and dashes: {game}");
    }
    if !GENRES.contains(&a.genre) {
        bail!("no genre {}: one of {}", a.genre, GENRES.join(", "));
    }
    let games: PathBuf = crate::roms_cmd::games_dir(a.games)
        .context("no games checkout: pass --games or set GP_GAMES_DIR")?;
    let dir = games.join(game);
    if dir.exists() {
        bail!(
            "{} exists already: gp game new writes over nothing",
            dir.display()
        );
    }
    let namespace = match a.namespace {
        Some(n) => n.to_owned(),
        None => shared_namespace(&games)
            .context("the games here don't share one namespace: pass --namespace")?,
    };

    // The ROM, by its hash, in $GP_ROM_DIR, and named by no game yet.
    let bytes = fs::read(a.rom).with_context(|| format!("reading {}", a.rom.display()))?;
    let rom = crate::roms::Rom::of_bytes(bytes)
        .map_err(|e| anyhow::anyhow!("{}: {e:#}", a.rom.display()))?;
    let sha = rom.sha256.clone();
    let found = crate::roms::find(std::slice::from_ref(&sha), &[], &paths.roms)?;
    if !found.contains_key(&sha) {
        bail!(
            "{} isn't in $GP_ROM_DIR ({}): add it with `gp roms add {}` first",
            a.rom.display(),
            paths.roms.display(),
            a.rom.display()
        );
    }
    let (reg, _) = crate::roms_cmd::sources(Some(&games))?;
    if let Some(k) = reg.by_sha256(&sha) {
        bail!(
            "the game whose roms.toml names {} has this ROM already: work in that package",
            k.id
        );
    }
    let source = a.source.unwrap_or(game);
    if !name_ok(source) {
        bail!("a source ID is lower-case letters, digits and dashes: {source}");
    }
    if reg.by_id(source).is_some() {
        bail!(
            "a roms.toml in {} has a source named {source} already: give --source",
            games.display()
        );
    }
    let title = a.title.unwrap_or(game).to_owned();
    let shape = shape(a.genre);
    let name = model_name(game);
    let (files, about) = match rom.platform {
        crate::roms::Platform::Nes => {
            let image = gp_evidence::source::identify_nes(&rom.bytes)
                .map_err(|e| anyhow::anyhow!("{}: {e}", a.rom.display()))?;
            nes_files(a, &namespace, &title, &shape, &name, &image, &rom.bytes)?
        }
        crate::roms::Platform::Arcade => arcade_files(a, &namespace, &title, &shape, &name, &rom)?,
    };
    write_package(&games, &dir, game, &files)?;
    // The level kit (docs/level-kit.md): a rules file whose parts are all
    // open, and the kit from what the model knows.
    let ldir = dir.join(crate::level_kit::DIR);
    fs::create_dir_all(&ldir)?;
    fs::write(
        ldir.join(crate::level_kit::RULES_FILE),
        crate::level_kit::stub_rules(game),
    )?;
    let (m, _) = crate::theme::read_model(&dir)?;
    crate::level_kit::write(&dir, &m, &ldir)?;
    let (qdir, rows, added) = crate::mechanic::seed_quest_log(paths, game, a.genre, &sha)?;
    let mut text = vec![
        format!(
            "{namespace}/{game}: a skeleton for the genre {} in {}",
            a.genre,
            dir.display()
        ),
        format!(
            "  model    {} ({}), view {}: {}, a player and an empty level, each a placeholder",
            name,
            shape.extension.map_or_else(
                || "core only".to_owned(),
                |e| format!("{} {}", e.name, e.version)
            ),
            shape.view,
            about.input
        ),
        format!(
            "  rom      {source} ({}), {}, in roms.toml{}",
            &sha[..16],
            known_entry(&sha)
                .and_then(|b| b.lines().find_map(|l| l.strip_prefix("status = ")))
                .map_or("unverified", |v| v.trim_matches('"')),
            if known_entry(&sha).is_some() {
                " (its entry from roms/known.toml)"
            } else {
                ""
            }
        ),
        format!("  assets   {}", about.assets),
        format!("  draws    {}", about.draws_line),
        "  corpus   no clips yet".into(),
        format!(
            "  quest    {} rows from the {} checklist ({added} new), a session for each, in {}",
            rows.len(),
            a.genre,
            qdir.display()
        ),
        String::new(),
        "Wrote:".into(),
    ];
    text.extend(files.iter().map(|(f, _)| format!("  {game}/{f}")));
    text.push(format!("  {game}/{}/", crate::level_kit::DIR));
    text.extend([
        String::new(),
        format!(
            "Next: gp model check {game}; gp template check {game} --rom ROM; gp package scan {game}. Then find the player's bytes and map them in mapping.toml before the first clip. Then record the quest log's rows with gp mechanic start {game}."
        ),
    ]);
    Ok(Outcome::ok(
        json!({
            "game": game, "package": format!("{namespace}/{game}"), "genre": a.genre,
            "platform": rom.platform.name(), "system": rom.system,
            "rom": sha, "dir": dir, "wrote": files.iter().map(|(f, _)| f).collect::<Vec<_>>(),
            "lifted_sha256": about.lifted, "draws": about.draws,
            "quest_log": rows.iter().map(|r| r.row.clone()).collect::<Vec<_>>(),
        }),
        text.join("\n"),
    ))
}

/// What `run` says of the files a platform's skeleton has.
struct About {
    input: &'static str,
    assets: String,
    lifted: Option<String>,
    /// The draw sites the idle run found, and what to say of them.
    draws: usize,
    draws_line: String,
}

/// An NES game's files: the package with its pins, the asset map of what
/// the iNES header says, and the rest.
fn nes_files(
    a: &New<'_>,
    namespace: &str,
    title: &str,
    shape: &Shape,
    name: &str,
    image: &NesImage,
    bytes: &[u8],
) -> Result<(Vec<(String, String)>, About)> {
    let game = a.game;
    let source = a.source.unwrap_or(game);
    let sha = image.sha256.clone();
    let title = title.to_owned();
    let namespace = namespace.to_owned();
    // The asset map and what it lifts, for the pins.
    let map_rel = format!("assets/{source}.toml");
    let map_text = asset_map(source, &title, image);
    let map = gp_assets::AssetMap::parse(&map_text).context("the skeleton's asset map")?;
    let (lifted, _) =
        gp_assets::lift(bytes, &map, &map_text).context("lifting with the skeleton's asset map")?;
    let lifted: BTreeMap<String, Vec<u8>> = lifted.into_iter().map(|f| (f.name, f.bytes)).collect();
    let lifted_sha = crate::template::lifted_digest(&lifted);
    let map_sha = sha256_hex(map_text.as_bytes());
    let recipes = crate::plan::recipe_set_hash(&BTreeMap::new());
    let (draws, n_draws, draws_line) = if a.draws {
        idle_draws(bytes)
    } else {
        (None, 0, "not searched (--no-draws)".to_owned())
    };

    let files: Vec<(String, String)> = vec![
        (
            "package.toml".into(),
            package_toml(
                &namespace,
                (game, source),
                &title,
                image,
                &map_rel,
                &map_sha,
                &lifted_sha,
                &recipes,
            ),
        ),
        (
            "roms.toml".into(),
            roms_toml(source, &title, image, a.date)?,
        ),
        (
            "template.toml".into(),
            template_toml(
                (game, source),
                &title,
                &sha,
                &map_rel,
                &map_sha,
                &lifted_sha,
            ),
        ),
        (map_rel.clone(), map_text.clone()),
        (
            "model.toml".into(),
            model_toml(name, &title, source, a.genre, shape, Device::NesPad),
        ),
        (
            "mapping.toml".into(),
            mapping_toml(name, source, Device::NesPad, Some(draws.as_deref())),
        ),
        ("fidelity.toml".into(), fidelity_toml(name, &title)),
        ("observe.toml".into(), observe_toml(name, &title)),
        ("probes.toml".into(), probes_toml(game, &title)),
        ("profiles.toml".into(), profiles_toml(&title)),
        (
            "corpus.toml".into(),
            corpus_toml(source, &title, &sha, &recipes),
        ),
    ];
    let about = About {
        input: "a pad",
        assets: format!(
            "{map_rel}: {}",
            if image.header.chr_rom_bytes > 0 {
                format!("{} tiles of CHR ROM", image.header.chr_rom_bytes / 16)
            } else {
                "no CHR ROM: the game's tiles are for the tools to find".into()
            }
        ),
        lifted: Some(lifted_sha),
        draws: n_draws,
        draws_line,
    };
    Ok((files, about))
}

/// An arcade game's files: the package entry and roms.toml source name
/// the romset's set and identity; the asset map is the board's, every
/// tile and sprite by region and index, pinned with what it lifts from
/// this set; the model reads the arcade device.
fn arcade_files(
    a: &New<'_>,
    namespace: &str,
    title: &str,
    shape: &Shape,
    name: &str,
    rom: &crate::roms::Rom,
) -> Result<(Vec<(String, String)>, About)> {
    let game = a.game;
    let source = a.source.unwrap_or(game);
    let system = rom.system.clone().unwrap_or_default();
    let set = gp_arcade::romset::set(&system).with_context(|| format!("no arcade set {system}"))?;
    let recipes = crate::plan::recipe_set_hash(&BTreeMap::new());
    let map_rel = format!("assets/{game}.toml");
    let sets: Vec<&str> = gp_arcade::romset::SETS.iter().map(|s| s.name).collect();
    let map_text = gp_assets::arcade::mcr_91490_map(game, title, &sets);
    let (lifted, _) = crate::assets::lift_files(&rom.bytes, &map_text)
        .context("lifting with the skeleton's asset map")?;
    let lifted: BTreeMap<String, Vec<u8>> = lifted.into_iter().map(|f| (f.name, f.bytes)).collect();
    let lifted_sha = crate::template::lifted_digest(&lifted);
    let map_sha = sha256_hex(map_text.as_bytes());
    let (draws, n_draws, draws_line) = if a.draws {
        idle_draws(&rom.bytes)
    } else {
        (None, 0, "not searched (--no-draws)".to_owned())
    };
    let files: Vec<(String, String)> = vec![
        (
            "package.toml".into(),
            format!(
                "# {namespace}/{game}: written by gp game new. A commercial game: the package holds\n\
                 # no game data, only facts about the romset and the model made from it.\n\
                 [package]\nname = {}\nversion = \"0.1.0\"\nkind = \"game\"\ntitle = {}\ntier = \"commercial\"\n\n\
                 [rom]\nhow_to_get = \"\"\"\nSay here which romset this game reads and how a player gets their own copy.\n\"\"\"\n\n\
                 # The romset's identity: the SHA-256 of its chips' SHA-1s, sorted\n\
                 # (gp_arcade::romset). The asset map names pictures by region and\n\
                 # index; lifted_sha256 is what it lifts from this set.\n\
                 [[rom.entry]]\nid = {}\nrole = \"scored\"\ntitle = {}\nsha256 = {}\nplatform = \"arcade\"\nsystem = {}\n\
                 board_status = \"unverified\"\n\
                 asset_map = {}\nasset_map_sha256 = {}\nlifted_sha256 = {}\n\n\
                 [corpus]\nname = {}\nversion = \"0.1.0\"\nrecipes = {}\n",
                q(&format!("{namespace}/{game}")),
                q(title),
                q(source),
                q(set.title),
                q(&rom.sha256),
                q(&system),
                q(&map_rel),
                q(&map_sha),
                q(&lifted_sha),
                q(game),
                q(&recipes),
            ),
        ),
        (
            "roms.toml".into(),
            arcade_roms_toml(source, title, rom, set, a.date)?,
        ),
        (
            "template.toml".into(),
            format!(
                "# Template manifest for {title}, a commercial game, written by gp game new:\n\
                 # it ships with no assets and no facts.\n\
                 [template]\nformat = \"0.3.1\"\nid = {}\ntitle = {}\ntier = \"commercial\"\nmodel = \"model.toml\"\n\
                 contract = \"fidelity.toml\"\nclips = \"corpus.toml\"\ngame = {}\n\n\
                 [source]\nid = {}\nplatform = \"arcade\"\nsystem = {}\nhow = \"rom\"\naccepts = [{}]\n\n\
                 [recipe]\nasset_map = {}\nasset_map_sha256 = {}\nlifted_sha256 = {}\n",
                q(game),
                q(title),
                q(game),
                q(source),
                q(&system),
                q(&rom.sha256),
                q(&map_rel),
                q(&map_sha),
                q(&lifted_sha),
            ),
        ),
        (map_rel.clone(), map_text.clone()),
        (
            "model.toml".into(),
            model_toml(name, title, source, a.genre, shape, Device::Arcade),
        ),
        (
            "mapping.toml".into(),
            mapping_toml(name, source, Device::Arcade, Some(draws.as_deref())),
        ),
        ("fidelity.toml".into(), fidelity_toml(name, title)),
        ("observe.toml".into(), observe_toml(name, title)),
        ("probes.toml".into(), probes_toml(game, title)),
        ("profiles.toml".into(), profiles_toml(title)),
        (
            "corpus.toml".into(),
            corpus_toml(game, title, &rom.sha256, &recipes),
        ),
    ];
    let about = About {
        input: "the arcade controls (coin, start1, a joystick and button1)",
        assets: format!("{map_rel}: 1024 tiles (gfx1) and 256 sprites (gfx2) of the {system} set"),
        lifted: Some(lifted_sha),
        draws: n_draws,
        draws_line,
    };
    Ok((files, about))
}

fn arcade_roms_toml(
    id: &str,
    title: &str,
    rom: &crate::roms::Rom,
    set: &gp_arcade::romset::SetInfo,
    date: &str,
) -> Result<String> {
    let mut s = String::from("# Hashes and facts only: never the game itself.\n[[source]]\n");
    let _ = writeln!(s, "id = {}", q(id));
    let _ = writeln!(s, "title = {}", q(title));
    s += "platform = \"arcade\"\nregion = \"us\"\n";
    let _ = writeln!(s, "revision = {}", q(set.title));
    let _ = writeln!(s, "system = {}", q(set.name));
    let _ = writeln!(s, "sha256 = {}", q(&rom.sha256));
    s += "status = \"unverified\"\n";
    let _ = writeln!(
        s,
        "evidence = {}",
        q(&format!(
            "Measured {date} by gp game new on the owner's romset: every chip of MAME 0.276's {} list matched by SHA-1. The sha256 is the set's identity, the SHA-256 of its chips' SHA-1s sorted (gp_arcade::romset).",
            set.name
        ))
    );
    let _ = writeln!(
        s,
        "notes = {}",
        q(&format!(
            "MAME's {} ({}, {}), Midway MCR boards 91490 and 90913: a 512x480 screen at 30 frames a second. The region is a guess.",
            set.name, set.title, set.year
        ))
    );
    gp_evidence::registry::Registry::from_toml(&s, Path::new("roms.toml"))
        .map_err(|e| anyhow::anyhow!("the new roms.toml doesn't read back: {e}"))?;
    Ok(s)
}

/// Writes the package's files into a folder beside the target first,
/// renamed into place once every file is written, so a failure leaves
/// nothing half made.
fn write_package(games: &Path, dir: &Path, game: &str, files: &[(String, String)]) -> Result<()> {
    let tmp = games.join(format!(".{game}.new"));
    if tmp.exists() {
        bail!(
            "{} is left from an earlier run; move it away",
            tmp.display()
        );
    }
    let write_all = || -> Result<()> {
        for (f, body) in files {
            let p = tmp.join(f);
            if let Some(parent) = p.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&p, body).with_context(|| format!("writing {}", p.display()))?;
        }
        fs::create_dir_all(tmp.join("inputs"))?;
        Ok(())
    };
    if let Err(e) = write_all() {
        let _ = fs::remove_dir_all(&tmp);
        return Err(e);
    }
    fs::rename(&tmp, dir).with_context(|| format!("writing {}", dir.display()))?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn package_toml(
    ns: &str,
    (game, source): (&str, &str),
    title: &str,
    image: &NesImage,
    map: &str,
    map_sha: &str,
    lifted_sha: &str,
    recipes: &str,
) -> String {
    let h = &image.header;
    format!(
        "# {ns}/{game}: written by gp game new. A commercial game: the package holds\n\
         # no game data, only facts about the ROM and the model made from it.\n\
         [package]\nname = {}\nversion = \"0.1.0\"\nkind = \"game\"\ntitle = {}\ntier = \"commercial\"\n\n\
         [rom]\nhow_to_get = \"\"\"\nSay here which release this game reads and how a player gets their own copy.\n\"\"\"\n\n\
         [[rom.entry]]\nid = {}\nrole = \"scored\"\ntitle = {}\nsha256 = {}\nplatform = \"nes\"\nmapper = {}\n\
         prg_rom_bytes = {}\nchr_rom_bytes = {}\nboard_status = \"unverified\"\n\
         asset_map = {}\nasset_map_sha256 = {}\nlifted_sha256 = {}\n\n\
         [corpus]\nname = {}\nversion = \"0.1.0\"\nrecipes = {}\n",
        q(&format!("{ns}/{game}")),
        q(title),
        q(source),
        q(title),
        q(&image.sha256),
        h.mapper,
        h.prg_rom_bytes,
        h.chr_rom_bytes,
        q(map),
        q(map_sha),
        q(lifted_sha),
        q(game),
        q(recipes),
    )
}

/// The NES ROM entries game-printer registered for games with no folder
/// here yet (roms/known.toml).
const KNOWN: &str = include_str!("../../../roms/known.toml");

/// The entry of roms/known.toml with this headerless SHA-256, as its
/// text, from its `id` line to its last.
fn known_entry(sha256: &str) -> Option<&'static str> {
    let line = format!("sha256 = {}", q(sha256));
    KNOWN
        .split("\n[[source]]\n")
        .skip(1)
        .find(|b| b.lines().any(|l| l == line))
}

fn roms_toml(id: &str, title: &str, image: &NesImage, date: &str) -> Result<String> {
    if let Some(block) = known_entry(&image.sha256) {
        // The known entry, under the new game's source id.
        let mut s = String::from(
            "# Hashes and facts only: never the game itself. Copied by gp game new from gameprinter's roms/known.toml, which game-printer's sources/nes.toml registered.\n[[source]]\n",
        );
        for l in block.trim_end().lines() {
            if l.starts_with("id = ") {
                let _ = writeln!(s, "id = {}", q(id));
            } else {
                s.push_str(l);
                s.push('\n');
            }
        }
        gp_evidence::registry::Registry::from_toml(&s, Path::new("roms.toml"))
            .map_err(|e| anyhow::anyhow!("the new roms.toml doesn't read back: {e}"))?;
        return Ok(s);
    }
    let h = &image.header;
    let k = |n: u64| n / 1024;
    let mut s = String::from("# Hashes and facts only: never the game itself.\n[[source]]\n");
    let _ = writeln!(s, "id = {}", q(id));
    let _ = writeln!(s, "title = {}", q(title));
    s += "platform = \"nes\"\nregion = \"ntsc\"\n";
    let _ = writeln!(s, "sha256 = {}", q(&image.sha256));
    let _ = writeln!(s, "mapper = {}", h.mapper);
    s += "status = \"unverified\"\n";
    let _ = writeln!(
        s,
        "evidence = {}",
        q(&format!(
            "Measured {date} by gp game new on the owner's dump. Not yet checked against a published checksum or a cartridge database."
        ))
    );
    let _ = writeln!(
        s,
        "notes = {}",
        q(&format!(
            "Its header says mapper {}, {}K PRG and {}K CHR. The region is a guess; no board facts until a cartridge database confirms them.",
            h.mapper,
            k(h.prg_rom_bytes),
            k(h.chr_rom_bytes)
        ))
    );
    gp_evidence::registry::Registry::from_toml(&s, Path::new("roms.toml"))
        .map_err(|e| anyhow::anyhow!("the new roms.toml doesn't read back: {e}"))?;
    Ok(s)
}

fn asset_map(source: &str, title: &str, image: &NesImage) -> String {
    let chr = image.header.chr_rom_bytes;
    let mut s = format!(
        "# Asset map for {title} ({source}), written by gp game new: only what the\n\
         # iNES header says. Where the game keeps its levels, palettes and sprites\n\
         # is for the tools to find.\n\
         schema = 1\nsource = {}\ntitle = {}\nsha256 = {}\n",
        q(source),
        q(title),
        q(&image.sha256)
    );
    if chr > 0 {
        let _ = write!(
            s,
            "\n# {}KB of CHR ROM: {} tiles.\n[[asset]]\nname = \"tiles\"\nformat = \"chr-2bpp\"\nregion = \"chr\"\noffset = 0x0000\ntiles = {}\ntiles_per_row = 16\n",
            chr / 1024,
            chr / 16,
            chr / 16
        );
    } else {
        s += "\n# No CHR ROM: the game draws its tiles into CHR RAM from PRG, so the\n# tiles are for the tools to find.\nasset = []\n";
    }
    s
}

fn template_toml(
    (game, source): (&str, &str),
    title: &str,
    sha: &str,
    map: &str,
    map_sha: &str,
    lifted: &str,
) -> String {
    format!(
        "# Template manifest for {title}, a commercial game, written by gp game new:\n\
         # it ships with no assets and no facts.\n\
         [template]\nformat = \"0.3.1\"\nid = {}\ntitle = {}\ntier = \"commercial\"\nmodel = \"model.toml\"\n\
         contract = \"fidelity.toml\"\nclips = \"corpus.toml\"\ngame = {}\n\n\
         [source]\nid = {}\nplatform = \"nes\"\nhow = \"rom\"\naccepts = [{}]\n\n\
         [recipe]\nasset_map = {}\nasset_map_sha256 = {}\nlifted_sha256 = {}\n",
        q(game),
        q(title),
        q(game),
        q(source),
        q(sha),
        q(map),
        q(map_sha),
        q(lifted)
    )
}

const BUTTONS: &[&str] = &["left", "right", "up", "down", "a", "b", "start", "select"];

/// The arcade skeleton's actions: each named for its button but the
/// button, which is the game's to name (Tapper's tap).
const ARCADE_ACTIONS: &[(&str, &str)] = &[
    ("coin", "coin"),
    ("start1", "start1"),
    ("left", "left"),
    ("right", "right"),
    ("up", "up"),
    ("down", "down"),
    ("button1", "button1"),
];

fn model_toml(
    name: &str,
    title: &str,
    source: &str,
    genre: &str,
    shape: &Shape,
    device: Device,
) -> String {
    let mut systems: Vec<&str> = vec!["rules", "entity_states", "counters", "spawn"];
    if let Some(e) = shape.extension {
        systems.extend(e.systems);
    }
    systems.extend(["exits", "camera"]);
    let systems = systems.iter().map(|s| q(s)).collect::<Vec<_>>().join(", ");
    let says = format!(
        "{title} ({source}), a skeleton for the {genre} genre written by gp game new. The system order is a placeholder until it's measured from the ROM's frame. The pad, the player and the empty level are placeholders too, so the model runs and its clips can be scored from the first one: their evidence says so, and each is replaced as it's measured"
    );
    let mut s = format!(
        "[model]\nschema = {}\nname = {}\ntitle = {}\nsource = {}\nview = {}\nframe_rate = {}\nsystems = [{systems}]\ndescription = {}\n",
        q(gp_model::CORE_VERSION),
        q(name),
        q(title),
        q(source),
        q(shape.view),
        q(match device {
            Device::NesPad => "39375000/655171",
            Device::Arcade => "30",
        }),
        q(&says)
    );
    if let Some(e) = shape.extension {
        let _ = write!(s, "\n[model.extensions]\n{} = {}\n", e.name, q(e.version));
    }
    match device {
        Device::NesPad => {
            s += "\n[input]\ndevice = \"nes-pad\"\n\n[input.actions]\n";
            for b in BUTTONS {
                let _ = writeln!(s, "{b} = {}", q(b));
            }
        }
        Device::Arcade => {
            s += "\n[input]\ndevice = \"arcade\"\n\n[input.actions]\n";
            for (a, b) in ARCADE_ACTIONS {
                let _ = writeln!(s, "{a} = {}", q(b));
            }
        }
    }
    s += "\n[entities.player]\ndescription = \"The player, not measured yet\"\nrole = \"player\"\nsize = [16, 16]\nhitbox = { x = 0, y = 0, w = 16, h = 16 }\n";
    // A screen of 32 by 30 tiles: 8 pixels on the NES (256 by 240), 16 on
    // Midway's MCR (512 by 480).
    let tile = match device {
        Device::NesPad => 8,
        Device::Arcade => 16,
    };
    let _ = write!(
        s,
        "\n[levels.main]\ndescription = \"The first level, not lifted yet: no tiles\"\nempty = true\nsize = [32, 30]\ntile_size = {tile}\n\n[[levels.main.spawn]]\nentity = \"player\"\nx = 0\ny = 0\n"
    );
    if device == Device::Arcade {
        s += "\n[present.screen]\nsize = [512, 480]\n";
    }
    if device == Device::Arcade {
        s += "\n[evidence.\"present.screen\"]\nsource = \"inferred\"\nconfidence = 90\nnote = \"From the board, not this game: MAME 0.276's mcr.cpp: Midway MCR's screen is 512 by 480 at 30 frames a second, and gp-arcade's frames match MAME's\"\n";
    }
    for part in ["input", "entities.player", "levels.main"] {
        let _ = write!(
            s,
            "\n[evidence.{}]\nsource = \"inferred\"\nconfidence = 0\nnote = \"A placeholder from gp game new: nothing here is measured yet\"\n",
            q(part)
        );
    }
    gp_model::Model::parse(&s)
        .map(|m| m.to_toml())
        .expect("the skeleton's model parses")
}

/// The state mapping. `draws` is `None` where no draw search ran (an
/// arcade original, which has no draw search yet); `Some(sites)` the
/// search's `[[draws]]` entries, if it found any.
fn mapping_toml(name: &str, source: &str, device: Device, draws: Option<Option<&str>>) -> String {
    let (platform, example) = match device {
        Device::NesPad => ("", "ram[0x0000]"),
        Device::Arcade => (
            "# An arcade original: ram[...] reads the main CPU's addresses (work RAM\n\
             # at 0xE000 on Midway MCR).\nplatform = \"arcade\"\n",
            "ram[0xE000]",
        ),
    };
    let text = format!(
        "# The state mapping, written by gp game new: where the game's RAM holds\n\
         # each part of the model's state. Nothing is mapped yet. Add the player's\n\
         # place first, before the first clip: a clip scored over no fields agrees\n\
         # on every frame, and the gate would then hold every clip to it.\n\
         schema = {}\nmodel = {}\nsource = {}\ndescription = \"Nothing measured yet\"\nalign = \"polled\"\n\
         # No fields yet: replace this line with the first [[field]] tables.\nfield = []\n\n\
         {}\
         # Where a run leaves what the template holds: scoring stops at the first\n\
         # frame out of scope. Set it once the level or stage byte is known, as:\n\
         # [scope]\n\
         # leaves = [\"{example}\"]\n",
        gp_interp::mapping::SCHEMA,
        q(name),
        q(source),
        draws_text(draws),
    );
    format!("{platform}{text}")
}

/// The mapping's draws section: the sites an idle run found, or how to
/// find them.
fn draws_text(draws: Option<Option<&str>>) -> String {
    let Some(found) = draws else {
        return String::new();
    };
    let mut s = "# A clip's score takes the original's random draws at the [[draws]] sites\n\
                 # (feature draws.score, docs/formats/draws.md), so the rebuild makes the\n\
                 # original's random choices from the first clip.\n"
        .to_owned();
    match found {
        Some(d) => {
            let _ = write!(
                s,
                "# The sites below were found by `gp interp draws --find` on {IDLE_FRAMES} frames\n# of no input from power-on.\n{d}"
            );
        }
        None => {
            s += "# None were found on an idle run from power-on. Run `gp interp draws --find\n\
                  # ROM --input LOG` on the first clips (`gp clip log GAME CLIP`) and\n\
                  # paste what it proposes here, with:\n\
                  # score_draws = true\n";
        }
    }
    s + "\n"
}

fn fidelity_toml(name: &str, title: &str) -> String {
    let text = format!(
        "[contract]\nschema = {}\nmodel = {}\ndefault = \"exact\"\ndescription = {}\n\n\
         [corpus]\ntogether = \"100%\"\nlongest_stretch = \"10s\"\n\
         note = \"Every clip at 100% of its frames together straight through from its start, and each long run (over 600 frames) with a longest exact stretch of 10 s or more in resync mode\"\n",
        q(gp_contract::VERSION),
        q(name),
        q(&format!(
            "{title}: the original's play frame for frame through the state mapping, its random choices given. The corpus bars are what `gp corpus score` and `gp model gate` hold every clip to: the owner's bar for Donkey Kong (2026-11-02), every game's default until its own is set (written by gp game new)"
        ))
    );
    gp_contract::Contract::parse(&text)
        .map(|c| c.to_toml())
        .expect("the skeleton's contract parses")
}

fn observe_toml(name: &str, title: &str) -> String {
    format!(
        "# What the tools read from {title} beyond its state mapping, written by\n\
         # gp game new. Add the level or stage byte once it's found, with the\n\
         # source of each fact, as:\n\
         # [level]\n\
         # ram = [0x0000]\n\
         # when = \"ram[0x0000] >= 1\"\n\
         # names = [[\"0x01\", \"first\"]]\n\
         schema = 0\nmodel = {}\n",
        q(name)
    )
}

fn probes_toml(game: &str, title: &str) -> String {
    format!(
        "# Probe recipes for {title} ({game}), written empty by gp game new. Each is\n\
         # a start and presses that reproduce an evidence clip, and the\n\
         # measurements its evidence quotes with the value each must have.\n\
         \n\
         # RAM addresses by name, each from the model's evidence or the mapping.\n\
         [watch]\n"
    )
}

fn profiles_toml(title: &str) -> String {
    format!(
        "# Profiles for {title}, written by gp game new. The model as written is\n\
         # \"retro\", every switch off, and every comparison and score uses it.\n\
         #\n\
         # \"normal\" is the game with no lag and none of the console's quirks. It\n\
         # turns nothing on yet. When the model gains an option for a quirk of the\n\
         # original, add a switch that turns it off, in the group \"timing\" for\n\
         # frame timing and lag, else in a group such as \"fixes\", and list it in\n\
         # \"normal\".\n\
         \n\
         format = \"{}\"\n\
         \n\
         [presets.normal]\n\
         description = \"No lag and no console quirks\"\n\
         switches = []\n",
        gp_model::profile::FORMAT
    )
}

fn corpus_toml(game: &str, title: &str, sha: &str, recipes: &str) -> String {
    format!(
        "# The corpus plan (format {}): the recipes that make this game's clips.\n\
         # Written empty by gp game new. Each clip is presses from power-on and\n\
         # presses from there, as canonical `.presses` files in inputs/, with a\n\
         # frame count. Curves are never kept here: `gp game score` derives them\n\
         # from your own ROM.\n\
         \n\
         [corpus]\nformat = {}\nname = {}\nversion = \"0.1.0\"\ntitle = {}\nrecipes = {}\n\n\
         [target]\ngame = {}\nrom = {}\n",
        crate::plan::FORMAT,
        q(crate::plan::FORMAT),
        q(game),
        q(title),
        q(recipes),
        q(game),
        q(sha),
    )
}

/// Today's date in UTC, `YYYY-MM-DD`, for the evidence lines.
pub fn today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    date_of_days((secs / 86_400) as i64)
}

/// The date `days` after 1970-01-01, `YYYY-MM-DD`.
pub fn date_of_days(days: i64) -> String {
    // Howard Hinnant's days-to-civil.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_date_agrees_with_the_truncating_form_sealed_and_corpus_score_used() {
        // The civil-date arithmetic `gp sealed`'s ledger time and
        // `gp corpus score`'s date carried before they called this, with
        // `/` where this has `div_euclid`: equal for every day from 1970.
        let old = |days: i64| {
            let z = days + 719_468;
            let era = z / 146_097;
            let doe = z - era * 146_097;
            let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
            let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
            let mp = (5 * doy + 2) / 153;
            let d = doy - (153 * mp + 2) / 5 + 1;
            let m = if mp < 10 { mp + 3 } else { mp - 9 };
            let y = yoe + era * 400 + i64::from(m <= 2);
            format!("{y:04}-{m:02}-{d:02}")
        };
        for days in (0..1_000_000)
            .step_by(7)
            .chain([0, 59, 60, 365, 11_016, 20_770])
        {
            assert_eq!(date_of_days(days), old(days), "day {days}");
        }
        assert_eq!(date_of_days(0), "1970-01-01");
        assert_eq!(date_of_days(11_016), "2000-02-29");
        assert_eq!(date_of_days(20_770), "2026-11-13");
    }

    /// A made-up NROM image: a header and filler, never a real game's bytes.
    fn image() -> Vec<u8> {
        let mut rom = b"NES\x1a\x02\x01\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00".to_vec();
        rom.extend((0..32768 + 8192).map(|i| (i * 7 % 251) as u8));
        rom
    }

    #[test]
    fn a_new_game_is_a_package_the_tools_accept_and_nothing_is_written_over() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("home");
        let paths = Paths {
            home: home.clone(),
            store: home.join("store"),
            roms: home.join("roms"),
        };
        fs::create_dir_all(paths.roms.join("nes")).unwrap();
        let rom = paths.roms.join("nes/test.nes");
        fs::write(&rom, image()).unwrap();
        let games = d.path().join("games");
        fs::create_dir_all(&games).unwrap();
        let a = New {
            game: "test-hop",
            rom: &rom,
            genre: "platformer",
            title: Some("Test Hop"),
            namespace: Some("someone"),
            games: Some(&games),
            source: None,
            date: "2026-11-11",
            draws: true,
        };
        let out = run(&a, &paths).unwrap();
        assert_eq!(out.json["package"], "someone/test-hop");
        let dir = games.join("test-hop");
        // The model and contract read, the package loads, the template's pins
        // hold against the ROM, the roms.toml names the ROM.
        crate::model::check(&dir, None).unwrap();
        let pkg = crate::package::Package::load(&dir).unwrap();
        assert_eq!(pkg.entries.len(), 1);
        assert!(pkg.roms_toml_disagreements().unwrap().is_empty());
        let t = crate::template::check(&dir, &rom).unwrap();
        assert_eq!(t.exit, 0, "{}", t.text);
        let (reg, _) = crate::roms_cmd::sources(Some(&games)).unwrap();
        assert_eq!(reg.by_id("test-hop").unwrap().sha256, out.json["rom"]);
        // No ROM byte in any file written.
        let rom_bytes = image();
        for f in out.json["wrote"].as_array().unwrap() {
            let text = fs::read(dir.join(f.as_str().unwrap())).unwrap();
            assert!(
                !text
                    .windows(32)
                    .any(|w| rom_bytes[16..].windows(32).any(|r| r == w))
            );
        }
        // The quest log: the genre's rows, each next with a session.
        let q: Vec<serde_json::Value> = serde_json::from_str(
            &fs::read_to_string(home.join("mechanics/test-hop/queue.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(q.len(), out.json["quest_log"].as_array().unwrap().len());
        assert_eq!(q[1]["mechanic"], "walk");
        assert_eq!(q[1]["status"], "next");
        let s: toml::Table = toml::from_str(
            &fs::read_to_string(home.join("mechanics/test-hop/walk/session.toml")).unwrap(),
        )
        .unwrap();
        assert_eq!(s["rom"].as_str(), out.json["rom"].as_str());
        // Again: refused, and nothing changed.
        assert!(run(&a, &paths).is_err());
        // A ROM a game names already is refused.
        let b = New {
            game: "test-hop-two",
            ..a
        };
        assert!(
            run(&b, &paths)
                .err()
                .unwrap()
                .to_string()
                .contains("has this ROM already")
        );
    }

    /// An arcade game from the owner's rbtapper romset, when it's on this
    /// machine: the package names the set by its identity, the model reads
    /// the arcade device at 30 frames a second, and no chip's bytes are in
    /// any file written. Skipped without the romset.
    #[test]
    fn a_new_arcade_game_names_its_romset_and_reads_the_arcade_controls() {
        let Some(zip) = gp_arcade::romset::local_romset("rbtapper") else {
            eprintln!("skipped: no rbtapper romset");
            return;
        };
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("home");
        let paths = Paths {
            home: home.clone(),
            store: home.join("store"),
            roms: home.join("roms"),
        };
        fs::create_dir_all(paths.roms.join("arcade")).unwrap();
        let rom = paths.roms.join("arcade/rbtapper.zip");
        fs::copy(&zip, &rom).unwrap();
        let games = d.path().join("games");
        fs::create_dir_all(&games).unwrap();
        let a = New {
            game: "tapper",
            rom: &rom,
            genre: "other",
            title: Some("Tapper"),
            namespace: Some("someone"),
            games: Some(&games),
            source: None,
            date: "2026-11-11",
            draws: true,
        };
        let out = run(&a, &paths).unwrap();
        assert_eq!(out.json["platform"], "arcade");
        assert_eq!(out.json["system"], "rbtapper");
        assert_eq!(
            out.json["rom"],
            gp_arcade::romset::set("rbtapper").unwrap().identity()
        );
        let dir = games.join("tapper");
        crate::model::check(&dir, None).unwrap();
        let m =
            gp_model::Model::parse(&fs::read_to_string(dir.join("model.toml")).unwrap()).unwrap();
        assert_eq!(m.model.frame_rate.to_string(), "30");
        assert_eq!(m.input.as_ref().unwrap().device, Device::Arcade);
        let pkg = crate::package::Package::load(&dir).unwrap();
        assert_eq!(pkg.platform().unwrap(), crate::roms::Platform::Arcade);
        let mapping = gp_interp::mapping::Mapping::parse(
            &fs::read_to_string(dir.join("mapping.toml")).unwrap(),
        )
        .unwrap();
        assert_eq!(mapping.platform.as_deref(), Some("arcade"));
        assert_eq!(pkg.entries[0].system.as_deref(), Some("rbtapper"));
        assert!(pkg.roms_toml_disagreements().unwrap().is_empty());
        let t = crate::template::check(&dir, &rom).unwrap();
        assert_eq!(t.exit, 0, "{}", t.text);
        let chips = crate::roms::Rom::read(&rom).unwrap();
        for f in out.json["wrote"].as_array().unwrap() {
            let text = fs::read(dir.join(f.as_str().unwrap())).unwrap();
            assert!(
                !text
                    .windows(32)
                    .any(|w| chips.data().windows(32).any(|r| r == w))
            );
        }
    }

    /// A ROM game-printer registered gets its entry from roms/known.toml.
    #[test]
    fn the_known_entries_load_and_one_is_copied_under_the_new_games_id() {
        let reg = gp_evidence::registry::Registry::from_toml(KNOWN, Path::new("roms/known.toml"))
            .unwrap();
        assert_eq!(reg.sources.len(), 6);
        let kirby = "a6b81fec11c24a33fd763db5c28005e760a1614e70c1bb5ccde0bd4242431000";
        let block = known_entry(kirby).unwrap();
        assert!(block.starts_with("id = \"kirby-usa-prg0\""));
        assert!(known_entry(&"0".repeat(64)).is_none());
    }

    /// The state mapping parses with the draw search's sites, without them,
    /// and with no search (an arcade original), as game-printer's test had.
    #[test]
    fn the_mapping_parses_with_or_without_draw_sites() {
        let found = "score_draws = true\n\n[[draws]]\nsite = \"s\"\npc = 0xE2DD\nram = 0x19\n";
        for (device, d) in [
            (Device::NesPad, Some(Some(found))),
            (Device::NesPad, Some(None)),
            (Device::Arcade, None),
        ] {
            let m = gp_interp::mapping::Mapping::parse(&mapping_toml(
                "new_game", "new-game", device, d,
            ))
            .unwrap();
            assert_eq!(m.draws.len(), usize::from(d == Some(Some(found))));
            assert_eq!(m.score_draws, d == Some(Some(found)));
        }
    }

    #[test]
    fn a_rom_not_in_the_rom_folder_is_refused() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("home");
        let paths = Paths {
            home: home.clone(),
            store: home.join("store"),
            roms: home.join("roms"),
        };
        let rom = d.path().join("elsewhere.nes");
        fs::write(&rom, image()).unwrap();
        fs::create_dir_all(d.path().join("games")).unwrap();
        let games = d.path().join("games");
        let a = New {
            game: "test-hop",
            rom: &rom,
            genre: "platformer",
            title: None,
            namespace: Some("someone"),
            games: Some(&games),
            source: None,
            date: "2026-11-11",
            draws: true,
        };
        let e = run(&a, &paths).err().unwrap();
        assert!(e.to_string().contains("gp roms add"), "{e}");
    }
}
