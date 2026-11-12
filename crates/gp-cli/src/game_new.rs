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
//! | `assets/SOURCE.toml` | An asset map of what the iNES header says: the CHR ROM as tiles, when there is one |
//! | `model.toml` | The genre's header, the pad, a player and an empty level, each a placeholder its evidence says is one |
//! | `mapping.toml`, `observe.toml`, `probes.toml`, `profiles.toml`, `fidelity.toml` | Empty or default, each saying what goes in it |
//! | `corpus.toml` | The corpus plan with no clips |
//! | `$GP_HOME/mechanics/GAME/` | The quest log seeded from the genre's checklist (mechanic/checklists.toml), and a mechanic session for each row |
//!
//! Nothing is written over: a folder that exists is refused, and the quest
//! log only gains the rows it lacks. No ROM byte is written; the lift's
//! digest is a hash of what the ROM gives, made here. Left for later steps
//! (~/.game-printer/work/bring/PLAN.md): the draw-site search, the theme kit
//! and the level kit, whose tools haven't moved yet.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use gp_evidence::Paths;
use gp_evidence::source::NesImage;
use gp_model::model::Device;
use serde_json::json;
use sha2::Digest as _;

use crate::Outcome;

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
    /// Today, for the evidence lines.
    pub date: &'a str,
}

/// The genres a skeleton can start from.
pub const GENRES: &[&str] = &["platformer", "topdown", "other"];

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

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(sha2::Sha256::digest(bytes))
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
    if reg.by_id(game).is_some() {
        bail!(
            "a roms.toml in {} has a source named {game} already",
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
            "  rom      {game} ({}), unverified, in roms.toml",
            &sha[..16]
        ),
        format!("  assets   {}", about.assets),
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
            "lifted_sha256": about.lifted,
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
    let sha = image.sha256.clone();
    let title = title.to_owned();
    let namespace = namespace.to_owned();
    // The asset map and what it lifts, for the pins.
    let map_rel = format!("assets/{game}.toml");
    let map_text = asset_map(game, &title, image);
    let map = gp_assets::AssetMap::parse(&map_text).context("the skeleton's asset map")?;
    let (lifted, _) =
        gp_assets::lift(bytes, &map, &map_text).context("lifting with the skeleton's asset map")?;
    let lifted: BTreeMap<String, Vec<u8>> = lifted.into_iter().map(|f| (f.name, f.bytes)).collect();
    let lifted_sha = crate::template::lifted_digest(&lifted);
    let map_sha = sha256_hex(map_text.as_bytes());
    let recipes = crate::plan::recipe_set_hash(&BTreeMap::new());

    let files: Vec<(String, String)> = vec![
        (
            "package.toml".into(),
            package_toml(
                &namespace,
                game,
                &title,
                image,
                &map_rel,
                &map_sha,
                &lifted_sha,
                &recipes,
            ),
        ),
        ("roms.toml".into(), roms_toml(game, &title, image, a.date)?),
        (
            "template.toml".into(),
            template_toml(game, &title, &sha, &map_rel, &map_sha, &lifted_sha),
        ),
        (map_rel.clone(), map_text.clone()),
        (
            "model.toml".into(),
            model_toml(name, &title, game, a.genre, shape, Device::NesPad),
        ),
        (
            "mapping.toml".into(),
            mapping_toml(name, game, Device::NesPad),
        ),
        ("fidelity.toml".into(), fidelity_toml(name, &title)),
        ("observe.toml".into(), observe_toml(name, &title)),
        ("probes.toml".into(), probes_toml(game, &title)),
        ("profiles.toml".into(), profiles_toml(&title)),
        (
            "corpus.toml".into(),
            corpus_toml(game, &title, &sha, &recipes),
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
    };
    Ok((files, about))
}

/// An arcade game's files: the package entry and roms.toml source name
/// the romset's set and identity; there is no asset map, since the tools
/// lift no arcade assets yet; the model reads the arcade device.
fn arcade_files(
    a: &New<'_>,
    namespace: &str,
    title: &str,
    shape: &Shape,
    name: &str,
    rom: &crate::roms::Rom,
) -> Result<(Vec<(String, String)>, About)> {
    let game = a.game;
    let system = rom.system.clone().unwrap_or_default();
    let set = gp_arcade::romset::set(&system).with_context(|| format!("no arcade set {system}"))?;
    let recipes = crate::plan::recipe_set_hash(&BTreeMap::new());
    let files: Vec<(String, String)> = vec![
        (
            "package.toml".into(),
            format!(
                "# {namespace}/{game}: written by gp game new. A commercial game: the package holds\n\
                 # no game data, only facts about the romset and the model made from it.\n\
                 [package]\nname = {}\nversion = \"0.1.0\"\nkind = \"game\"\ntitle = {}\ntier = \"commercial\"\n\n\
                 [rom]\nhow_to_get = \"\"\"\nSay here which romset this game reads and how a player gets their own copy.\n\"\"\"\n\n\
                 # The romset's identity: the SHA-256 of its chips' SHA-1s, sorted\n\
                 # (gp_arcade::romset). No asset map: the tools lift no arcade assets yet.\n\
                 [[rom.entry]]\nid = {}\nrole = \"scored\"\ntitle = {}\nsha256 = {}\nplatform = \"arcade\"\nsystem = {}\n\
                 board_status = \"unverified\"\n\n\
                 [corpus]\nname = {}\nversion = \"0.1.0\"\nrecipes = {}\n",
                q(&format!("{namespace}/{game}")),
                q(title),
                q(game),
                q(set.title),
                q(&rom.sha256),
                q(&system),
                q(game),
                q(&recipes),
            ),
        ),
        (
            "roms.toml".into(),
            arcade_roms_toml(game, title, rom, set, a.date)?,
        ),
        (
            "template.toml".into(),
            format!(
                "# Template manifest for {title}, a commercial game, written by gp game new:\n\
                 # it ships with no assets and no facts. An arcade romset has no asset\n\
                 # map yet, so there is no [recipe].\n\
                 [template]\nformat = \"0.3.1\"\nid = {}\ntitle = {}\ntier = \"commercial\"\nmodel = \"model.toml\"\n\
                 contract = \"fidelity.toml\"\nclips = \"corpus.toml\"\ngame = {}\n\n\
                 [source]\nid = {}\nplatform = \"arcade\"\nsystem = {}\nhow = \"rom\"\naccepts = [{}]\n",
                q(game),
                q(title),
                q(game),
                q(game),
                q(&system),
                q(&rom.sha256),
            ),
        ),
        (
            "model.toml".into(),
            model_toml(name, title, game, a.genre, shape, Device::Arcade),
        ),
        (
            "mapping.toml".into(),
            mapping_toml(name, game, Device::Arcade),
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
        assets: "none: the tools lift no arcade assets yet".into(),
        lifted: None,
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
    game: &str,
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
        q(game),
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

fn roms_toml(id: &str, title: &str, image: &NesImage, date: &str) -> Result<String> {
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
    game: &str,
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
        q(game),
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

fn mapping_toml(name: &str, source: &str, device: Device) -> String {
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
         # Where a run leaves what the template holds: scoring stops at the first\n\
         # frame out of scope. Set it once the level or stage byte is known, as:\n\
         # [scope]\n\
         # leaves = [\"{example}\"]\n",
        gp_interp::mapping::SCHEMA,
        q(name),
        q(source)
    );
    format!("{platform}{text}")
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
    let days = (secs / 86_400) as i64;
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
            date: "2026-11-11",
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
            date: "2026-11-11",
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
            date: "2026-11-11",
        };
        let e = run(&a, &paths).err().unwrap();
        assert!(e.to_string().contains("gp roms add"), "{e}");
    }
}
