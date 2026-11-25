//! `gp theme look`: a large-art theme in a repo of its own (`package.toml`,
//! `theme/look.txt`, `theme/art.txt` and the pictures; lex00/turnips is the
//! pattern). `check` reads one the way the page does and reports what is
//! wrong, what is missing against the game's theme kit, how the pictures
//! sit on the game's boxes, names the kit denies, and the ROM-data check of
//! `gp template served`. `new` writes an empty one from the kit.
//!
//! The rules are read by gp-present's own reader (`gp_present::looks` and
//! `gp_present::art`), and the game's pictures are the ones the page builds
//! (`gp_look::present::art`, from the art lifted from the person's ROM), so
//! this check and the page can't disagree about what a rule names.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use clap::Subcommand;
use gp_evidence::Paths;
use gp_model::Model;
use gp_present::art::{ArtRules, Sized};
use gp_present::looks::Look;
use gp_theme::pix::Rgba;
use serde_json::{Value, json};

use crate::Outcome;
use crate::theme_kit::{ArtSlot, Kit};

mod sound;

#[derive(Subcommand)]
pub enum LookCmd {
    /// Check a large-art theme folder against a game, the way the page reads
    /// it: package.toml, look.txt and art.txt, every picture a rule names, the
    /// game's kit (every slot and pose drawn, nothing named the game lacks),
    /// how each picture sits on its body's box and feet row, names the kit
    /// denies, and ROM data (`gp template served`, when your ROM is here).
    /// Exits 0 with no problems (warnings allowed), else 1.
    Check {
        /// The theme's folder (holding package.toml and theme/), or its
        /// theme/ folder.
        dir: PathBuf,
        /// The game: its folder, or its name in the games checkout.
        #[arg(long)]
        game: PathBuf,
        /// Your ROM of the game (default: one under $GP_ROM_DIR the game
        /// accepts).
        #[arg(long)]
        rom: Option<PathBuf>,
    },
    /// An arcade room's cells as its `#+ room` rules see them: a grid of the
    /// screen's 32 by 30 cells from your own romset, each with its tile code
    /// in hex and its colour bank, and the class its rules put it in (the
    /// class's first letter, `.` for none), with the classes' letters
    /// listed. For writing `#+ room_cells` tests; printed only, never
    /// written to a repo.
    Cells {
        /// The theme's folder (holding package.toml and theme/), or its
        /// theme/ folder.
        dir: PathBuf,
        /// The game: its folder, or its name in the games checkout.
        #[arg(long)]
        game: PathBuf,
        /// The screen, by its slot (`gp theme look sheet` lists them).
        #[arg(long)]
        screen: String,
        /// Your ROM of the game (default: one under $GP_ROM_DIR the game
        /// accepts).
        #[arg(long)]
        rom: Option<PathBuf>,
    },
    /// Start a large-art theme for a game in a new folder: package.toml,
    /// theme/look.txt, theme/art.txt with one line per picture the game's kit
    /// lists (still to draw), a README and a .gitattributes. Writes nothing
    /// from a ROM, and refuses a folder that already holds files.
    New {
        /// The folder to write (new or empty).
        dir: PathBuf,
        /// The game: its folder, or its name in the games checkout.
        #[arg(long)]
        game: PathBuf,
        /// The theme's package name, NAMESPACE/NAME (your GitHub name and
        /// the theme's).
        #[arg(long)]
        name: String,
        /// The theme's title, as players see it.
        #[arg(long)]
        title: String,
        /// The licence of your pictures and rules.
        #[arg(long, default_value = "CC-BY-4.0")]
        license: String,
    },
    /// Play a large-art theme folder in the game without publishing it. It
    /// runs the check first (problems stop it unless --anyway; warnings are
    /// printed), then serves the game on 127.0.0.1 with your theme chosen.
    /// A game played from a ROM gets the page that plays any game by name,
    /// from a registry made in a temp folder: the game (read from the real
    /// registry, or built here when it isn't there) and its published
    /// themes, and your folder as a theme only this page sees. You load your
    /// own ROM in the page. A game with pictures of its own (a template, or
    /// a copy of one) is built for the browser with your theme added and
    /// needs no ROM. An arcade game plays from your romset zip on that
    /// page, your theme drawn over its board's pictures. With --shots DIR
    /// it plays the game's route in headless Chrome instead, on each ROM of
    /// the game that is here, and saves the
    /// game at named moments (the game's theme-shots.toml, else the page
    /// check's route, else the title and the first stage); with --against
    /// it does the same for a second theme and writes the pairs side by
    /// side. Nothing is published.
    ///
    /// Example: gp theme look play ~/my-theme --game rooftop --open
    ///
    /// Example: gp theme look play ~/my-theme --game rooftop --shots
    /// ~/theme-shots --against friend/night@0.9.2
    Play(crate::theme_play::PlayArgs),
    /// For an arcade game drawn by its board's art: the names a theme may
    /// give the board's pictures that the game shows, and a contact sheet
    /// of each entity's sprite codes, from your own romset and the game's
    /// recorded plays (its inputs/). A theme names a screen by its slot
    /// (screen_saloon), a sprite by its entity and code (sprite-mug-4d), a
    /// counter's digits as counter-score-7. Written under
    /// $GP_HOME/work/theme-kits/MODEL/board, or a folder you name outside
    /// any repo: they are pictures of your ROM's game, never to commit.
    /// `gp theme look check` then warns of a sprite code the game wasn't
    /// seen to show.
    ///
    /// Example: gp theme look sheet --game rooftop
    Sheet {
        /// The game: its folder, or its name in the games checkout.
        #[arg(long)]
        game: PathBuf,
        /// Your romset of the game (default: one under $GP_ROM_DIR the game
        /// accepts).
        #[arg(long)]
        rom: Option<PathBuf>,
        /// The folder to write (default
        /// $GP_HOME/work/theme-kits/MODEL/board). Refused inside a repo.
        #[arg(long, short)]
        out: Option<PathBuf>,
    },
    /// Bring named pictures from a theme folder (your copy, under
    /// $GP_HOME/work/themes) into a theme folder in a games checkout or a
    /// worktree of one (GAME/themes/NAME), for a change that lands them:
    /// each file under the paths named is written at the same place under
    /// the target's theme/ folder, put in the repo's Git LFS store when the
    /// repo keeps it in LFS, and marked to add, so `git diff` in the repo
    /// shows it as its LFS pointer. Nothing else in either folder changes.
    ///
    /// Example: gp theme look take ~/.gameprinter/work/themes/neon/repo
    /// --into ~/games/rooftop/themes/neon --only pieces/punk
    Take {
        /// The theme folder to take from (holding package.toml and theme/),
        /// or its theme/ folder.
        copy: PathBuf,
        /// The theme folder in a games checkout to write into
        /// (GAME/themes/NAME, or its theme/ folder).
        #[arg(long)]
        into: PathBuf,
        /// A file or folder under the copy's theme/ folder (pieces/punk);
        /// a folder brings every file under it. Repeatable.
        #[arg(long, required = true)]
        only: Vec<String>,
    },
}

pub(crate) fn run(cmd: &LookCmd, paths: &Paths) -> Result<Outcome> {
    match cmd {
        LookCmd::Check { dir, game, rom } => check(dir, game, rom.as_deref(), paths),
        LookCmd::Cells {
            dir,
            game,
            screen,
            rom,
        } => cells(dir, game, screen, rom.as_deref(), paths),
        LookCmd::New {
            dir,
            game,
            name,
            title,
            license,
        } => new(dir, game, name, title, license, &paths.home),
        LookCmd::Play(a) => crate::theme_play::run(a, paths),
        LookCmd::Sheet { game, rom, out } => {
            crate::theme_board::sheet(game, rom.as_deref(), out.as_deref(), paths)
        }
        LookCmd::Take { copy, into, only } => crate::theme_take::take(copy, into, only),
    }
}

// ---------------------------------------------------------------------
// Findings.

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Group {
    Files,
    Offered,
    Kit,
    Sound,
    Fit,
    Words,
    Rom,
}

impl Group {
    fn title(self) -> &'static str {
        match self {
            Group::Files => "Files",
            Group::Offered => "On the page (Settings, Theme)",
            Group::Kit => "What the game needs drawn",
            Group::Sound => "Sound (each role's files, as the page plays them)",
            Group::Fit => "Fit (a first pass from the files alone: look at it in the game too)",
            Group::Words => "Names",
            Group::Rom => "ROM data",
        }
    }
    fn key(self) -> &'static str {
        match self {
            Group::Files => "files",
            Group::Offered => "offered",
            Group::Kit => "kit",
            Group::Sound => "sound",
            Group::Fit => "fit",
            Group::Words => "names",
            Group::Rom => "rom_data",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Level {
    Problem,
    Warning,
    Note,
    Ok,
}

impl Level {
    fn word(self) -> &'static str {
        match self {
            Level::Problem => "problem",
            Level::Warning => "warning",
            Level::Note => "note",
            Level::Ok => "ok",
        }
    }
}

#[derive(Debug, Clone)]
struct Finding {
    group: Group,
    level: Level,
    file: Option<String>,
    line: Option<usize>,
    text: String,
}

#[derive(Default)]
struct Report {
    found: Vec<Finding>,
}

impl Report {
    fn add(
        &mut self,
        group: Group,
        level: Level,
        file: Option<&str>,
        line: Option<usize>,
        text: impl Into<String>,
    ) {
        self.found.push(Finding {
            group,
            level,
            file: file.map(str::to_owned),
            line,
            text: text.into(),
        });
    }
    fn count(&self, l: Level) -> usize {
        self.found.iter().filter(|f| f.level == l).count()
    }
}

// ---------------------------------------------------------------------
// The theme's folder.

const ART_FILE_DEFAULT: &str = "art.txt";
const PICTURE_EXTS: &[&str] = &["png", "jpg", "jpeg", "webp", "gif"];
/// A picture larger than this on a side some phones' browsers won't draw.
const MAX_SIDE: u32 = 4096;
/// Alpha at or above which a pixel counts as drawn.
const INK: u8 = 128;
/// How far a picture's shape may differ from its rule's box before it
/// reads as stretched: 15%.
const STRETCH: f64 = 0.15;

struct Folder {
    /// The repo: package.toml, README.
    root: PathBuf,
    /// Its theme/ folder: look.txt, art.txt, the pictures.
    theme: PathBuf,
}

impl Folder {
    fn of(dir: &Path) -> Folder {
        if dir.join(gp_present::looks::LOOK_FILE).is_file() && !dir.join("theme").is_dir() {
            Folder {
                root: dir.parent().map_or_else(|| dir.to_owned(), Path::to_owned),
                theme: dir.to_owned(),
            }
        } else {
            Folder {
                root: dir.to_owned(),
                theme: dir.join("theme"),
            }
        }
    }
    /// A path as the report names it: from the repo's folder.
    fn rel(&self, p: &Path) -> String {
        p.strip_prefix(&self.root)
            .unwrap_or(p)
            .to_string_lossy()
            .replace('\\', "/")
    }
    fn theme_rel(&self, file: &str) -> String {
        self.rel(&self.theme.join(file))
    }
}

/// Every file under `dir`, relative to it, without dot files and folders.
fn files_under(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_owned()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(r) = p.strip_prefix(dir) {
                out.push(r.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    out.sort();
    out
}

fn is_picture(f: &str) -> bool {
    Path::new(f)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| PICTURE_EXTS.contains(&e.to_ascii_lowercase().as_str()))
}

/// The art.txt line where a word first appears, by its number from 1.
fn line_of(text: &str, word: &str) -> Option<usize> {
    text.lines()
        .position(|l| {
            let l = l.trim();
            (!l.starts_with('#') || l.starts_with(gp_present::art::ADDED))
                && l.split_whitespace().any(|w| w == word)
        })
        .map(|i| i + 1)
}

/// The art.txt line of the `pic` rule drawing a game's picture, else the
/// first line naming it.
fn pic_line(text: &str, build: &str) -> Option<usize> {
    text.lines()
        .position(|l| {
            let mut w = l.split_whitespace();
            w.next() == Some("pic") && w.next() == Some(build)
        })
        .map(|i| i + 1)
        .or_else(|| line_of(text, build))
}

/// The drawn part of a picture: x0, y0, x1, y1 (the last two past it).
fn ink_box(img: &Rgba) -> Option<[u32; 4]> {
    let mut b: Option<[u32; 4]> = None;
    for y in 0..img.height {
        for x in 0..img.width {
            if img.get(x, y)[3] >= INK {
                b = Some(match b {
                    None => [x, y, x + 1, y + 1],
                    Some([x0, y0, x1, y1]) => [x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1)],
                });
            }
        }
    }
    b
}

// ---------------------------------------------------------------------
// The game: its folder, kit, words and the pictures the page builds.

struct Game {
    dir: PathBuf,
    title: String,
    kit: Option<Kit>,
    deny: Vec<String>,
    /// The regions of the stage the game's level kit names
    /// (`level-kit/kit.toml`'s `[[levels.regions]]`, docs/level-kit.md),
    /// which a `#+ in REGION` line may name; None for a game with no
    /// level kit.
    regions: Option<Vec<String>>,
}

fn game_of(arg: &Path) -> Result<Game> {
    let dir = crate::theme_kit::template_dir(arg)?;
    let dir = dir.canonicalize().unwrap_or(dir);
    let pkg: Option<toml::Table> = fs::read_to_string(dir.join("package.toml"))
        .ok()
        .and_then(|t| t.parse().ok());
    let field = |k: &str| {
        pkg.as_ref()
            .and_then(|p| p.get("package"))
            .and_then(|p| p.get(k))
            .and_then(|v| v.as_str())
            .map(str::to_owned)
    };
    let (id, _, _) = crate::theme_kit::manifest(&dir);
    let title = field("title").unwrap_or(id);
    let kit_file = dir
        .join("themes")
        .join("kit")
        .join(crate::theme_kit::KIT_FILE);
    let kit = match fs::read_to_string(&kit_file) {
        Ok(t) => Some(
            toml::from_str::<Kit>(&t).with_context(|| format!("reading {}", kit_file.display()))?,
        ),
        Err(_) => None,
    };
    let deny = crate::theme_words::load(&dir)?
        .map(|w| w.deny)
        .unwrap_or_default();
    let regions = if dir.join(gp_web_lift::LEVEL_KIT).is_file() {
        Some(
            gp_web_lift::regions(&dir)
                .map_err(|e| anyhow::anyhow!("{}: {e}", dir.display()))?
                .into_iter()
                .map(|r| r.name)
                .collect(),
        )
    } else {
        None
    };
    Ok(Game {
        dir,
        title,
        kit,
        deny,
        regions,
    })
}

/// The game's pictures as the page builds them from the art lifted from
/// the person's ROM: the same model, spec and presentation.
struct Built {
    model: Model,
    art: gp_look::present::Art,
    /// The spec's kind names, in the order the presentation keeps them.
    kinds: Vec<String>,
    /// Each kind's state a body starts in (its machine's initial state,
    /// 0 without one): the look a body is drawn with in play, by the
    /// presentation's per-state looks.
    initial: Vec<usize>,
    /// An arcade game drawn by its board's art: the names its rules give
    /// instead of the build's pictures (crate::theme_board).
    board: Option<crate::theme_board::BoardKit>,
    /// Its board's pictures, to measure a sprite's body in the original
    /// and to build a theme's rooms as the page does.
    board_art: Option<gp_look::board::BoardArt>,
    _lifted: Option<tempfile::TempDir>,
}

impl Built {
    /// How many levels the board's pictures may be drawn in.
    fn board_art_levels(&self) -> usize {
        self.model.levels.len().max(1)
    }
    /// The pictures a theme's rules name: the board's for a game its board
    /// draws, else the build's.
    fn present(&self) -> &gp_present::Present {
        self.board
            .as_ref()
            .map_or(&self.art.present, |b| &b.present)
    }
    fn names(&self) -> BTreeSet<&str> {
        self.present()
            .pictures
            .iter()
            .map(|p| p.file.as_str())
            .collect()
    }
    fn file(&self, pic: u32) -> &str {
        &self.art.present.pictures[pic as usize].file
    }
    fn blank(&self, pic: u32) -> bool {
        let p = &self.art.pictures[pic as usize];
        Rgba::from_png(&p.png)
            .ok()
            .is_some_and(|i| ink_box(&i).is_none())
    }
}

/// The page's build of the game, or why there is none.
fn build(game: &Game, rom: Option<&Path>, paths: &Paths) -> Result<Built, String> {
    let (model, _) = crate::theme::read_model(&game.dir).map_err(|e| format!("{e:#}"))?;
    let t = crate::template_extract::Template::load(&game.dir).map_err(|e| format!("{e:#}"))?;
    let lifted = crate::template_font::lifted_dir(&t, rom, paths).map_err(|e| format!("{e:#}"))?;
    let (dir, keep) = match lifted {
        Ok(tmp) => (tmp.path().to_owned(), Some(tmp)),
        Err(why) => {
            // The usual lifted folder, when the ROM was lifted before.
            let usual = model
                .model
                .source
                .as_ref()
                .map(|s| paths.home.join("work").join("assets").join(s));
            match usual {
                Some(u) if u.join("assets.toml").is_file() && rom.is_none() => (u, None),
                _ => return Err(why),
            }
        }
    };
    let bundle = gp_look::bundle(&model, &dir)?;
    let spec = gp_interp::compile::compile(&model, &bundle.assets)?;
    let art = gp_look::present::art(&model, &spec, &dir)?;
    let board = crate::theme_board::kit(&model, &spec, &dir, &paths.home)?;
    let board_art = if board.is_some() {
        gp_look::board::BoardArt::load(&model, &spec, &dir)?
    } else {
        None
    };
    let mut kinds = Vec::new();
    let mut initial = Vec::new();
    for k in &spec.kinds {
        kinds.push(k.name.clone());
        initial.push(k.machine.as_ref().map_or(0, |m| m.initial));
    }
    Ok(Built {
        model,
        art,
        kinds,
        initial,
        board,
        board_art,
        _lifted: keep,
    })
}

/// Each frame of a kit slot and the game's pictures it is drawn as, in
/// every palette set: None when the check can't tell which they are.
fn slot_pictures(kit: &Kit, slot: &ArtSlot, b: &Built) -> Option<Vec<Vec<u32>>> {
    let p = &b.art.present;
    let pics_of = |frame: u32| -> Vec<u32> {
        let mut v: Vec<u32> = p.frames.get(frame as usize).cloned().unwrap_or_default();
        v.dedup();
        v
    };
    let m = &b.model;
    let mp = m.present.clone().unwrap_or_default();
    let names = &slot.frame_names;
    if let Some(rest) = slot.id.strip_prefix("entity.") {
        let ids = crate::theme_kit::names_of(m);
        let name = m.entities.iter().find_map(|(n, e)| {
            (crate::theme_kit::entity_id(n, &e.core.role, &ids) == slot.id).then_some(n.clone())
        });
        let name = name.or_else(|| m.entities.contains_key(rest).then(|| rest.to_owned()))?;
        let k = b.kinds.iter().position(|x| *x == name)?;
        let states = p.kinds.get(k)?;
        let looks: Vec<&gp_present::Look> = states.iter().flatten().collect();
        // A pose is the look of the state a body starts in, as play draws
        // it: the first state's look is another state's (Moss Hop's hero:
        // dying sorts before normal).
        let first = b
            .initial
            .get(k)
            .and_then(|&i| states.get(i))
            .and_then(Option::as_ref)
            .or_else(|| looks.first().copied())?;
        let mut out = Vec::new();
        for f in names {
            let id = if f == "base" {
                Some(first.base.id)
            } else {
                let (c, j) = f.strip_prefix("clip")?.split_once('.')?;
                let (c, j): (usize, usize) = (c.parse().ok()?, j.parse().ok()?);
                looks.iter().find_map(|l| {
                    l.clips
                        .get(c)
                        .and_then(|cl| cl.frames.get(j))
                        .map(|fr| fr.id)
                })
            };
            out.push(id.map(pics_of).unwrap_or_default());
        }
        return Some(out);
    }
    if let Some(i) = slot.id.strip_prefix("effect.") {
        let fx = p.effects.get(i.parse::<usize>().ok()?)?;
        let all: Vec<u32> = fx
            .phases
            .iter()
            .flat_map(|ph| ph.frames.iter().map(|f| f.id))
            .collect();
        return Some(
            (0..names.len())
                .map(|n| all.get(n).map(|&id| pics_of(id)).unwrap_or_default())
                .collect(),
        );
    }
    if let Some(rest) = slot.id.strip_prefix("screen-sprite.") {
        let (i, k) = rest.split_once('.')?;
        if p.screens.len() != mp.screens.len() {
            return None;
        }
        let (i, k): (usize, usize) = (i.parse().ok()?, k.parse().ok()?);
        if let Some(s) = p.screens.get(i).and_then(|s| s.sprites.get(k)) {
            return Some(vec![vec![s.pic]]);
        }
        // A kit written before screens were added numbers them by the
        // screens then; its screen sprites keep their order.
        let nth = kit
            .art
            .iter()
            .filter(|s| s.id.starts_with("screen-sprite."))
            .position(|s| s.id == slot.id)?;
        let all: Vec<u32> = p
            .screens
            .iter()
            .flat_map(|s| s.sprites.iter().map(|x| x.pic))
            .collect();
        let kit_n = kit
            .art
            .iter()
            .filter(|s| s.id.starts_with("screen-sprite."))
            .count();
        return (all.len() == kit_n).then(|| vec![vec![all[nth]]]);
    }
    if slot.id == "font" {
        let hud = p.hud.as_ref()?;
        return Some(
            names
                .iter()
                .map(|d| {
                    let ch = d.chars().next();
                    let mut v: Vec<u32> = hud
                        .glyphs
                        .iter()
                        .filter(|g| Some(g.ch) == ch)
                        .map(|g| g.pic)
                        .collect();
                    v.sort_unstable();
                    v.dedup();
                    v
                })
                .collect(),
        );
    }
    if let Some(rest) = slot.id.strip_prefix("picture.") {
        let (s, e) = rest.split_once('.')?;
        let is = |r: &Option<gp_model::model::EntryRef>| {
            r.as_ref().is_some_and(|r| r.slot == s && r.entry == e)
        };
        let mut v = Vec::new();
        let by_name = |f: &str| {
            p.pictures
                .iter()
                .position(|x| x.file == f)
                .map(|i| i as u32)
        };
        for (name, l) in &m.levels {
            if is(&l.core.layout)
                && let Some(i) = by_name(&format!("level-{name}.png"))
            {
                v.push(i);
            }
        }
        if mp.hud.as_ref().is_some_and(|h| is(&h.layer)) {
            v.extend(p.levels.iter().filter_map(|l| l.hud_layer));
        }
        if p.screens.len() == mp.screens.len() {
            for (i, sc) in mp.screens.iter().enumerate() {
                if is(&sc.layout) {
                    v.push(p.screens[i].picture);
                }
            }
        }
        v.sort_unstable();
        v.dedup();
        if v.is_empty() {
            return None;
        }
        return Some(vec![v]);
    }
    None
}

/// Each kit slot's frames as the game's pictures they are drawn as, by
/// the names a large-art theme's rules give them, and each picture's size
/// in game pixels: for `gp theme sheet` and `gp theme fit --large`.
pub(crate) struct GamePictures {
    /// By slot id, per frame, the game's pictures (blank ones left out).
    pub frames: BTreeMap<String, Vec<Vec<String>>>,
}

/// [`GamePictures`] from the game's build with the person's lift, or why
/// there is none.
pub(crate) fn game_pictures(
    game_dir: &Path,
    kit: &Kit,
    rom: Option<&Path>,
    paths: &Paths,
) -> Result<GamePictures, String> {
    let game = game_of(game_dir).map_err(|e| format!("{e:#}"))?;
    let b = build(&game, rom, paths)?;
    let mut frames = BTreeMap::new();
    for slot in &kit.art {
        if crate::theme_board_kit::is_board(slot) {
            let v = slot
                .frame_names
                .iter()
                .map(|f| {
                    crate::theme_board_kit::picture(&b.model, slot, f)
                        .into_iter()
                        .collect()
                })
                .collect();
            frames.insert(slot.id.clone(), v);
            continue;
        }
        if let Some(fr) = slot_pictures(kit, slot, &b) {
            let v = fr
                .iter()
                .map(|pics| {
                    pics.iter()
                        .filter(|&&p| !b.blank(p))
                        .map(|&p| b.file(p).to_owned())
                        .collect()
                })
                .collect();
            frames.insert(slot.id.clone(), v);
        }
    }
    Ok(GamePictures { frames })
}

// ---------------------------------------------------------------------
// The check.

pub(crate) fn check(
    dir: &Path,
    game_arg: &Path,
    rom: Option<&Path>,
    paths: &Paths,
) -> Result<Outcome> {
    let game = game_of(game_arg)?;
    let built = build(&game, rom, paths);
    let out = report(dir, &game, built, Some((rom, paths)))?;
    // A theme copy's journey keeps its last check (crate::journeys).
    crate::journeys::note_check(&paths.home, dir, &out.json);
    Ok(out)
}

/// `gp theme look cells`: a screen's cells and the classes the theme's
/// room rules put them in.
fn cells(
    dir: &Path,
    game_arg: &Path,
    screen: &str,
    rom: Option<&Path>,
    paths: &Paths,
) -> Result<Outcome> {
    let game = game_of(game_arg)?;
    let built = build(&game, rom, paths).map_err(|e| anyhow!(e))?;
    let a = built.board_art.as_ref().ok_or_else(|| {
        anyhow!(
            "{} isn't drawn by an arcade board's art, so it has no rooms",
            game.title
        )
    })?;
    let cells = a
        .screen_cells(screen)
        .ok_or_else(|| anyhow!("{} shows no screen {screen}", game.title))?;
    let folder = Folder::of(dir);
    let text = fs::read_to_string(folder.theme.join(ART_FILE_DEFAULT)).unwrap_or_default();
    let rules = ArtRules::read(&text).map_err(|e| anyhow!("{}: {e}", ART_FILE_DEFAULT))?;
    let room = rules
        .rooms
        .iter()
        .find(|r| r.screens.iter().any(|s| s == screen));
    let classes = room.map(|r| gp_present::art::room::classify(r, &cells));
    let mut text = format!(
        "{screen}: {} by {} cells of {} board pixels; each cell is TILE.BANK then its class{}\n",
        cells.cols,
        cells.rows,
        cells.size,
        if room.is_none() {
            " (no `#+ room` line names this screen, so none)"
        } else {
            ""
        }
    );
    let letter = |i: usize| -> char {
        match (room, &classes) {
            (Some(r), Some(c)) => c
                .iter()
                .position(|g| g[i])
                .and_then(|k| r.classes[k].name.chars().next())
                .unwrap_or('.'),
            _ => '.',
        }
    };
    for row in 0..cells.rows {
        let line: Vec<String> = (0..cells.cols)
            .map(|col| {
                let i = row * cells.cols + col;
                let c = &cells.cells[i];
                format!("{:03x}.{}{}", c.tile, c.bank, letter(i))
            })
            .collect();
        let _ = writeln!(text, "{row:2} {}", line.join(" "));
    }
    if let (Some(r), Some(c)) = (room, &classes) {
        for (k, cl) in r.classes.iter().enumerate() {
            let n = c[k].iter().filter(|b| **b).count();
            let _ = writeln!(
                text,
                "{}: {} ({n} cells)",
                cl.name.chars().next().unwrap_or('?'),
                cl.name
            );
        }
    }
    Ok(Outcome::ok(json!({ "screen": screen }), text))
}

/// The check on a theme folder with the game's build (or why there is
/// none); the ROM-data check runs with `rom_check`.
fn report(
    dir: &Path,
    game: &Game,
    built: Result<Built, String>,
    rom_check: Option<(Option<&Path>, &Paths)>,
) -> Result<Outcome> {
    let dir = &dir.canonicalize().unwrap_or_else(|_| dir.to_owned());
    let folder = Folder::of(dir);
    if !folder.root.is_dir() {
        bail!("{} isn't a folder", dir.display());
    }
    let mut r = Report::default();

    // package.toml.
    let pkg = check_package(&folder, &mut r);

    // look.txt.
    let look_path = folder.theme.join(gp_present::looks::LOOK_FILE);
    let look_rel = folder.rel(&look_path);
    let look = match fs::read_to_string(&look_path) {
        Err(_) => {
            r.add(
                Group::Files,
                Level::Problem,
                Some(&look_rel),
                None,
                "there is no look.txt, so the page doesn't see a theme here. Make it with these lines: `gp-look 1`, `id NAME`, `title TITLE`, `art art.txt`",
            );
            None
        }
        Ok(t) => match Look::read(&t) {
            Err(e) => {
                let line = t
                    .lines()
                    .position(|l| e.contains(l.trim()) && !l.trim().is_empty())
                    .map(|i| i + 1);
                r.add(
                    Group::Files,
                    Level::Problem,
                    Some(&look_rel),
                    line,
                    format!("the page can't read it ({e}). It holds `gp-look 1`, then `id NAME` (lowercase letters, digits, - and _), `title TITLE` and `art art.txt`"),
                );
                None
            }
            Ok(l) => {
                if l.art.is_none() {
                    r.add(
                        Group::Files,
                        Level::Problem,
                        Some(&look_rel),
                        None,
                        "it names no rules file, so the page draws nothing of yours. Add the line `art art.txt`",
                    );
                }
                Some(l)
            }
        },
    };

    // art.txt, read by the page's own reader.
    let art_name = look
        .as_ref()
        .and_then(|l| l.art.clone())
        .unwrap_or_else(|| ART_FILE_DEFAULT.to_owned());
    let art_path = folder.theme.join(&art_name);
    let art_rel = folder.rel(&art_path);
    let art_text = fs::read_to_string(&art_path).ok();
    let rules = match &art_text {
        None => {
            r.add(
                Group::Files,
                Level::Problem,
                Some(&art_rel),
                None,
                format!("there is no {art_name}, the rules file look.txt names. Start one with the line `gp-art 1`"),
            );
            None
        }
        Some(t) => match ArtRules::read(t) {
            Ok(rules) => Some(rules),
            Err(e) => {
                let line = e
                    .strip_prefix("art rules line ")
                    .and_then(|s| s.split(':').next())
                    .and_then(|n| n.parse::<usize>().ok());
                let what = match line {
                    Some(_) => e.split_once(": ").map_or(e.as_str(), |x| x.1).to_owned(),
                    None => e.clone(),
                };
                r.add(
                    Group::Files,
                    Level::Problem,
                    Some(&art_rel),
                    line,
                    format!("the page can't read this line ({what}), and refuses the whole theme because of it. Fix the line (site docs, themes.md, lists each rule and its numbers) and check again"),
                );
                None
            }
        },
    };

    // The pictures: each a rule names exists and loads; unused ones.
    let on_disk = files_under(&folder.theme);
    let mut images: BTreeMap<String, Rgba> = BTreeMap::new();
    if let (Some(rules), Some(text)) = (&rules, &art_text) {
        for f in rules.files() {
            let p = folder.theme.join(&f);
            let line = line_of(text, &f);
            if !p.is_file() {
                r.add(
                    Group::Files,
                    Level::Problem,
                    Some(&art_rel),
                    line,
                    format!("names the picture {f}, which isn't in the theme folder, and the page refuses the theme when a picture is missing. Add {} or fix the name", folder.theme_rel(&f)),
                );
                continue;
            }
            if !is_picture(&f) {
                r.add(
                    Group::Files,
                    Level::Problem,
                    Some(&art_rel),
                    line,
                    format!("names {f}, which isn't a picture a browser loads. Save it as PNG"),
                );
                continue;
            }
            if !f.to_ascii_lowercase().ends_with(".png") {
                r.add(
                    Group::Files,
                    Level::Note,
                    Some(&folder.theme_rel(&f)),
                    None,
                    "not a PNG: a browser loads it, but this check can't measure it. PNG is what the other themes use",
                );
                continue;
            }
            match Rgba::load(&p) {
                Err(e) => r.add(
                    Group::Files,
                    Level::Problem,
                    Some(&folder.theme_rel(&f)),
                    None,
                    format!(
                        "can't be read as a PNG ({}). Save it again as PNG",
                        e.rsplit(": ").next().unwrap_or(&e)
                    ),
                ),
                Ok(img) => {
                    if img.width > MAX_SIDE || img.height > MAX_SIDE {
                        r.add(
                            Group::Files,
                            Level::Warning,
                            Some(&folder.theme_rel(&f)),
                            None,
                            format!("is {} by {} pixels; some phones don't draw a picture over {MAX_SIDE} on a side. Make it smaller", img.width, img.height),
                        );
                    }
                    images.insert(f, img);
                }
            }
        }
        let used: BTreeSet<String> = rules.files().into_iter().collect();
        let unused: Vec<&String> = on_disk
            .iter()
            .filter(|f| is_picture(f) && !used.contains(*f))
            .collect();
        if !unused.is_empty() {
            let shown: Vec<String> = unused.iter().take(8).map(|f| folder.theme_rel(f)).collect();
            r.add(
                Group::Files,
                Level::Note,
                None,
                None,
                format!(
                    "{} picture{} no rule uses (the page never loads {}): {}{}",
                    unused.len(),
                    if unused.len() == 1 { "" } else { "s" },
                    if unused.len() == 1 { "it" } else { "them" },
                    shown.join(", "),
                    if unused.len() > shown.len() {
                        ", ..."
                    } else {
                        ""
                    }
                ),
            );
        }
        // A stage only for an arcade game's rooms has no planks or rails.
        if let Some(st) = &rules.stage
            && (rules.rooms.is_empty() || !st.levels.is_empty())
        {
            stage_checks(st, text, &art_rel, &mut r);
        }
        twice(text, &art_rel, &mut r);
    }
    let files_ok = r
        .found
        .iter()
        .all(|f| f.group != Group::Files || f.level != Level::Problem);
    if files_ok {
        let n = rules.as_ref().map_or(0, |x| x.files().len());
        r.add(
            Group::Files,
            Level::Ok,
            None,
            None,
            format!("package.toml, look.txt and {art_name} read as the page reads them; {n} pictures named, all there"),
        );
    }

    // The game's pictures, as the page builds them.
    if let (Some(rules), Some(text), Ok(b)) = (&rules, &art_text, &built) {
        names_check(rules, text, &art_rel, b, &mut r);
        if let Some(k) = &b.board {
            board_drawn(rules, &art_rel, k, &mut r);
        }
        let tol = game
            .kit
            .as_ref()
            .map(|k| k.tolerance.clone())
            .unwrap_or_default();
        // A body's poses: the pictures the kit's bodies are drawn as.
        let poses: Option<BTreeSet<String>> = game.kit.as_ref().map(|k| {
            k.art
                .iter()
                .filter(|a| a.kind == "sprite" && a.id.starts_with("entity."))
                .filter_map(|a| slot_pictures(k, a, b))
                .flatten()
                .flatten()
                .map(|p| b.file(p).to_owned())
                .collect()
        });
        body_check(
            &tol,
            rules,
            text,
            &art_rel,
            &images,
            b,
            poses.as_ref(),
            &mut r,
        );
    }

    // Where each `pic` line's picture came from.
    if let (Some(rules), Some(text)) = (&rules, &art_text) {
        made_from(
            &folder,
            rules,
            text,
            &art_rel,
            rom_check.map(|(_, p)| p.home.as_path()),
            &mut r,
        );
    }

    // The lines added since gp-art 1 began (`#+`).
    if let (Some(rules), Some(text)) = (&rules, &art_text) {
        added_checks(rules, text, &art_rel, game, built.as_ref().ok(), &mut r);
    }

    // Whether the page would offer it, by the page's own rules.
    offered(&folder, pkg.as_ref(), game, built.as_ref().ok(), &mut r);

    // Coverage against the kit, then the fit.
    let ruled = rules.as_ref().map(ruled_names).unwrap_or_default();
    match &game.kit {
        // An arcade game drawn by its board: the board's names are what a
        // theme draws, and its sheet what the game shows (above).
        None if built.as_ref().is_ok_and(|b| b.board.is_some()) => {}
        None => r.add(
            Group::Kit,
            Level::Note,
            None,
            None,
            format!(
                "{} has no theme kit yet (themes/kit/kit.toml in its folder), so this check can't say which pictures the game needs or measure how they fit. Make one with `gp theme kit {}`, and run this check again",
                game.title,
                game.dir.display()
            ),
        ),
        Some(kit) => {
            coverage(kit, rules.as_ref(), &ruled, built.as_ref().ok(), &built, &mut r);
            if let (Some(rules), Some(text), Ok(b)) = (&rules, &art_text, &built) {
                fit(kit, rules, text, &art_rel, &images, b, &mut r);
            }
        }
    }
    if let (Some(rules), Some(text)) = (&rules, &art_text) {
        stretch(rules, text, &art_rel, &images, &mut r);
    }

    // The sound: each role's files, as the page plays them.
    if let Some(l) = &look {
        sound::check(&folder, l, &on_disk, game, &mut r);
    }

    // Names the kit denies.
    words(
        &folder,
        pkg.as_ref(),
        look.as_ref(),
        &on_disk,
        art_text.as_deref(),
        game,
        &mut r,
    );

    // ROM data, as `gp template served` checks it.
    if let Some((rom, paths)) = rom_check {
        rom_data(&folder, game, rom, paths, &mut r);
    }

    Ok(finish(&folder, game, pkg.as_ref(), r))
}

/// Whether the page would offer the theme in Settings, Theme, by the code
/// the page runs: its name by the theme list's rule
/// (`gp_web_registry::theme_id_problem`), and its folder as the lift reads
/// a finished look over the game's build (`gp_web_lift::ready_look_offer`).
/// What only the registry knows (that it is published for the game, its
/// namespace granted, no other theme of its name there) is
/// `gp registry check --for`'s.
fn offered(folder: &Folder, pkg: Option<&Pkg>, game: &Game, built: Option<&Built>, r: &mut Report) {
    let Some(name) = pkg.and_then(|p| p.name.as_deref()) else {
        r.add(
            Group::Offered,
            Level::Note,
            None,
            None,
            "package.toml has no name yet, so this check can't say whether the page would offer it: the page names a theme by its package name",
        );
        return;
    };
    let rel = folder.rel(&folder.root.join("package.toml"));
    let id = gp_web_registry::theme_id(name);
    if let Some((reason, detail)) = gp_web_registry::theme_id_problem(name) {
        r.add(
            Group::Offered,
            Level::Problem,
            Some(&rel),
            None,
            format!("the page would leave it out of Settings, Theme, because {reason}. {detail}"),
        );
        return;
    }
    if game.dir.join("themes").join(id).is_dir() {
        r.add(
            Group::Offered,
            Level::Warning,
            Some(&rel),
            None,
            format!("{} ships a theme in themes/{id}, and on the page yours, named `{id}` by its package name, takes that theme's place. Give it a name of its own unless it is meant to replace it", game.title),
        );
    }
    let Some(b) = built else {
        r.add(
            Group::Offered,
            Level::Note,
            None,
            None,
            format!("without the game's build (your ROM) this check can't say whether the page draws it; its name, `{id}`, is one the page can list"),
        );
        return;
    };
    match gp_web_lift::ready_look_offer(
        id,
        &folder.theme,
        &b.art.present_text(),
        b.board.as_ref().map(|k| &k.present),
        true,
    ) {
        Err((reason, detail)) => r.add(
            Group::Offered,
            Level::Problem,
            None,
            None,
            format!("the page would leave it out of Settings, Theme for {}, because {reason} ({detail})", game.title),
        ),
        Ok(lines) if !lines.is_empty() => r.add(
            Group::Offered,
            Level::Warning,
            None,
            None,
            format!("the page would offer it as `{id}`, drawn in part: {}", lines.join("; ")),
        ),
        Ok(_) => r.add(
            Group::Offered,
            Level::Ok,
            None,
            None,
            format!("the page would offer it in Settings, Theme for {} as `{id}`, once it is published for the game; after publishing, `gp registry check REGISTRY --for GAME --theme {name}` asks the registry", game.title),
        ),
    }
}

/// The theme's package fields, when package.toml reads.
struct Pkg {
    name: Option<String>,
    title: Option<String>,
}

fn valid_package_name(n: &str) -> bool {
    let part = |p: &str| {
        p.chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
            && p.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    };
    n.split_once('/').is_some_and(|(a, b)| part(a) && part(b))
}

/// A name `gp theme look new` gives a theme: one a registry takes
/// (`gp_package::registry::name_ok`) whose last part is a look id
/// (`gp_package::theme_id_ok`) with no `_`, so the look id is that part.
/// A copy made before with an `_` still reads ([`valid_package_name`]).
pub(crate) fn new_theme_name_ok(n: &str) -> bool {
    gp_package::registry::name_ok(n)
        && n.split_once('/')
            .is_some_and(|(_, last)| gp_package::theme_id_ok(last) && !last.contains('_'))
}

/// The nearest name [`new_theme_name_ok`] takes: lowercase, every other
/// character a `-`, `local/` when there is no namespace, and a last part
/// starting with a letter.
pub(crate) fn fixed_theme_name(n: &str) -> String {
    let fix = |s: &str| -> String {
        let s: String = s
            .chars()
            .map(|c| match c.to_ascii_lowercase() {
                c @ ('a'..='z' | '0'..='9' | '-') => c,
                _ => '-',
            })
            .collect();
        s.trim_matches('-').to_owned()
    };
    let (ns, last) = n.split_once('/').unwrap_or(("local", n));
    let ns = match fix(ns) {
        s if s.is_empty() => "local".to_owned(),
        s => s,
    };
    let last = fix(&last.replace('/', "-"));
    let last = if last.starts_with(|c: char| c.is_ascii_lowercase()) {
        last
    } else {
        format!("t-{last}").trim_end_matches('-').to_owned()
    };
    format!("{ns}/{last}")
}

fn valid_version(v: &str) -> bool {
    let core = v.split(['-', '+']).next().unwrap_or("");
    let parts: Vec<&str> = core.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

fn check_package(folder: &Folder, r: &mut Report) -> Option<Pkg> {
    let p = folder.root.join("package.toml");
    let rel = folder.rel(&p);
    let Ok(text) = fs::read_to_string(&p) else {
        r.add(
            Group::Files,
            Level::Problem,
            Some(&rel),
            None,
            "there is no package.toml beside theme/, so the theme can't be published. `gp theme look new` writes one; it holds [package] with name, version, kind = \"content\", title and license",
        );
        return None;
    };
    let t: toml::Table = match text.parse() {
        Ok(t) => t,
        Err(e) => {
            let line = e.span().map(|s| text[..s.start].lines().count().max(1));
            r.add(
                Group::Files,
                Level::Problem,
                Some(&rel),
                line,
                format!("isn't valid TOML ({}). Fix that line", e.message()),
            );
            return None;
        }
    };
    let line = |k: &str| {
        text.lines()
            .position(|l| {
                l.trim_start().starts_with(k)
                    && l[l.find(k).unwrap_or(0) + k.len()..]
                        .trim_start()
                        .starts_with('=')
            })
            .map(|i| i + 1)
    };
    let Some(pk) = t.get("package").and_then(|v| v.as_table()) else {
        r.add(
            Group::Files,
            Level::Problem,
            Some(&rel),
            None,
            "has no [package] table. Add one with name, version, kind = \"content\", title and license",
        );
        return None;
    };
    let get = |k: &str| pk.get(k).and_then(|v| v.as_str()).map(str::to_owned);
    let mut bad =
        |k: &str, what: String| r.add(Group::Files, Level::Problem, Some(&rel), line(k), what);
    let name = get("name");
    match &name {
        None => bad("name", "has no name. Add `name = \"NAMESPACE/NAME\"` (your GitHub name and the theme's, lowercase)".into()),
        Some(n) if !valid_package_name(n) => bad("name", format!("name `{n}` isn't NAMESPACE/NAME in lowercase letters, digits, - and _ (like lex00/turnips). Change it")),
        _ => {}
    }
    match get("version") {
        None => bad(
            "version",
            "has no version. Add `version = \"0.1.0\"`".into(),
        ),
        Some(v) if !valid_version(&v) => bad(
            "version",
            format!("version `{v}` isn't three numbers like 0.1.0. Change it"),
        ),
        _ => {}
    }
    match get("kind") {
        Some(k) if k == "content" => {}
        Some(k) => bad(
            "kind",
            format!("kind is `{k}`; a theme's is \"content\". Change it to `kind = \"content\"`"),
        ),
        None => bad("kind", "has no kind. Add `kind = \"content\"`".into()),
    }
    let title = get("title").filter(|t| !t.trim().is_empty());
    if title.is_none() {
        bad(
            "title",
            "has no title. Add `title = \"...\"`: the name players see".into(),
        );
    }
    if get("license")
        .or_else(|| get("licence"))
        .is_none_or(|l| l.trim().is_empty())
    {
        bad("license", "has no license. Add `license = \"CC-BY-4.0\"` (or the licence you choose for your pictures)".into());
    }
    Some(Pkg { name, title })
}

/// The stage rules: pieces the builder needs and `put`s of pieces that
/// aren't there.
fn stage_checks(
    st: &gp_present::art::stage::StageRules,
    text: &str,
    art_rel: &str,
    r: &mut Report,
) {
    let has = |n: &str| st.pieces.iter().any(|p| p.0 == n);
    for (piece, what) in [("plank", "platforms"), ("rail", "ladders")] {
        if !has(piece) {
            r.add(
                Group::Files,
                Level::Warning,
                Some(art_rel),
                line_of(text, "stage"),
                format!("the stages have no `stage_piece {piece}`, so their {what} aren't drawn. Add one"),
            );
        }
    }
    for l in &st.levels {
        if l.rope.is_some() && !has("rope") {
            r.add(Group::Files, Level::Warning, Some(art_rel), line_of(text, &l.level), format!("{} finds ropes, but there is no `stage_piece rope`, so they aren't drawn. Add one", l.level));
        }
        if !l.lumps.is_empty() && !has("lump") {
            r.add(Group::Files, Level::Warning, Some(art_rel), line_of(text, &l.level), format!("{} has `lump` lines, but there is no `stage_piece lump`, so they aren't drawn. Add one", l.level));
        }
        for (name, _) in &l.put {
            if !has(name) {
                r.add(Group::Files, Level::Problem, Some(art_rel), line_of(text, name), format!("`put {name}` names a piece no `stage_piece` line gives, and the page passes over it. Add `stage_piece {name} ART W H` or fix the name"));
            }
        }
    }
}

/// A game's picture two `pic` lines draw: the page keeps the last.
fn twice(text: &str, art_rel: &str, r: &mut Report) {
    let mut lines: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, l) in text.lines().enumerate() {
        let mut w = l.split_whitespace();
        if w.next() == Some("pic")
            && let Some(b) = w.next()
        {
            lines.entry(b).or_default().push(i + 1);
        }
    }
    let twice: Vec<(&str, Vec<usize>)> = lines.into_iter().filter(|(_, v)| v.len() > 1).collect();
    if twice.is_empty() {
        return;
    }
    let shown: Vec<String> = twice
        .iter()
        .take(5)
        .map(|(b, v)| {
            let v: Vec<String> = v.iter().map(usize::to_string).collect();
            format!("{b} (lines {})", v.join(", "))
        })
        .collect();
    r.add(
        Group::Files,
        Level::Warning,
        Some(art_rel),
        twice.first().and_then(|x| x.1.first().copied()),
        format!(
            "{} of the game's pictures have more than one `pic` line, and the page uses only the last of each: {}{}. Keep one line for each",
            twice.len(),
            shown.join("; "),
            if twice.len() > shown.len() { "; ..." } else { "" }
        ),
    );
}

/// The `#+` lines: one the page can't read (passed over, so the theme
/// draws without it), a region the game's level kit doesn't name, a
/// `hud_over` row with no HUD text in the background, and what a page
/// built before the lines does with them.
fn added_checks(
    rules: &ArtRules,
    text: &str,
    art_rel: &str,
    game: &Game,
    b: Option<&Built>,
    r: &mut Report,
) {
    for (line, said) in &rules.skipped {
        let word = said
            .strip_prefix(gp_present::art::ADDED)
            .and_then(|x| x.split_whitespace().next())
            .unwrap_or("");
        let why = match word {
            "hud_over" => "`#+ hud_over` takes no row or rows of the HUD, each a number from 0 to 63 (8 pixels each from the screen's top)".to_owned(),
            "in" => "`#+ in` takes a region's name (lowercase letters, digits, - and _) and then a whole `pic` or `timed` line".to_owned(),
            "on" => "`#+ on` takes one or more of the game's screens and then a whole `pic`, `timed`, `also` or `under` line".to_owned(),
            "under" => "`#+ under` comes after the `stage` line and takes an entity (the ENTITY of its bodies' pictures, sprite-ENTITY-CODE), its offset DX DY in screen pixels (from the middle of the body's box to the piece's middle, and from the box's bottom to the piece's top), then each piece of its cycle with the game frames it holds, PIECE:FRAMES (1 or more)".to_owned(),
            "room" => "`#+ room` takes one or more screens, after the `stage` line".to_owned(),
            "room_cells" => "`#+ room_cells` takes a new class's name (lowercase letters, digits, - and _) after a `#+ room` line, then its tests: tile=N or tile=LO-HI, bank=N, prio=N (0 to 3), share=P (1 to 100), !CLASS for a class given before, and colour tests such as r>g+40".to_owned(),
            "room_fill" | "room_box" => format!("`#+ {word}` takes a class its room gives and a piece"),
            "room_at" => "`#+ room_at` takes a class its room gives, a piece and its offset from each cell, DX DY in screen pixels".to_owned(),
            "room_run" => "`#+ room_run` takes a class its room gives and three pieces, left, middle and right, `-` for none".to_owned(),
            "room_react" => "`#+ room_react` takes a class its room gives, its offset DX DY in screen pixels, then each piece of its cycle with the game frames it holds, PIECE:FRAMES (1 or more), and no pick=".to_owned(),
            "room_react_when" => "`#+ room_react_when` comes after a `#+ room_react` and takes a piece and one or more tests, COUNTER CMP VALUE, CMP one of < <= == != >= >".to_owned(),
            "room_react_drift" => "`#+ room_react_drift` comes once after a `#+ room_react` and takes VX VY, the screen pixels it moves (whole numbers from -64 to 64, not both 0), and PER, the game frames each move takes (1 or more)".to_owned(),
            "room_react_stagger" => "`#+ room_react_stagger` comes once after a `#+ room_react` and takes N, the game frames (1 or more) each placement's cycle is further on than the one before it, then nothing or the word `rows`, for N frames further on for each row of cells down the screen instead".to_owned(),
            "room_scroll" => "`#+ room_scroll` takes one piece (no `a|b` list) after a `#+ room` line, the stage pixels it slides, DX (negative for left), and the game frames each slide takes, EVERY (1 or more), once for a piece in a room".to_owned(),
            "" => "it says nothing after `#+`".to_owned(),
            w => format!("no page knows `#+ {w}`; the lines are `#+ hud_over`, `#+ in`, `#+ on`, `#+ under` and the room lines (`#+ room`, `room_cells`, `room_fill`, `room_run`, `room_box`, `room_at`, `room_react`, `room_react_when`, `room_react_drift`, `room_react_stagger`, `room_scroll`, `room_front`, `room_mask`), and `#+ recolor`"),
        };
        r.add(
            Group::Files,
            Level::Problem,
            Some(art_rel),
            Some(*line),
            format!("the page can't read this line ({why}), and passes over it, so the theme is drawn without it. Fix the line and check again"),
        );
    }
    for region in rules.regions() {
        let line = line_of(text, region);
        match &game.regions {
            Some(named) if named.iter().any(|n| n == region) => {}
            Some(named) => r.add(
                Group::Kit,
                Level::Problem,
                Some(art_rel),
                line,
                format!(
                    "names the region `{region}`, which {}'s level kit doesn't name, so the page never draws these lines. The kit names {}",
                    game.title,
                    if named.is_empty() { "none".to_owned() } else { named.join(", ") }
                ),
            ),
            None => r.add(
                Group::Kit,
                Level::Problem,
                Some(art_rel),
                line,
                format!(
                    "names the region `{region}`, but {} has no level kit to name regions of its stage, so the page never draws these lines. Leave them out, or name the region in its level-kit/rules.toml and run gp level kit",
                    game.title
                ),
            ),
        }
    }
    if let (Some(over), Some(b)) = (&rules.hud_over, b) {
        let present = &b.art.present;
        let line = line_of(text, "hud_over");
        let layer = present.levels.iter().any(|l| l.hud_layer.is_some());
        let rows: BTreeSet<i64> = present
            .hud
            .iter()
            .flat_map(|h| h.fields.iter())
            .filter(|f| f.palette < 4)
            .map(|f| i64::from(f.at[1]))
            .collect();
        if !layer && rows.is_empty() {
            r.add(
                Group::Kit,
                Level::Warning,
                Some(art_rel),
                line,
                format!("{} has no HUD layer and no HUD text in the background (a HUD drawn as sprites is over the bodies already), so `#+ hud_over` changes nothing", game.title),
            );
        } else if !layer {
            let empty: Vec<String> = over
                .iter()
                .filter(|x| !rows.contains(x))
                .map(i64::to_string)
                .collect();
            if !empty.is_empty() {
                r.add(
                    Group::Kit,
                    Level::Warning,
                    Some(art_rel),
                    line,
                    format!(
                        "`#+ hud_over` names row {} where {} draws no HUD text in the background (its rows: {}), so it changes nothing there",
                        empty.join(", "),
                        game.title,
                        rows.iter().map(i64::to_string).collect::<Vec<_>>().join(", ")
                    ),
                );
            }
        }
    }
    let used = rules.hud_over.is_some()
        || !rules.regions().is_empty()
        || !rules.rooms.is_empty()
        || !rules.on_screen.is_empty()
        || !rules.under.is_empty();
    if used {
        r.add(
            Group::Files,
            Level::Note,
            Some(art_rel),
            line_of(text, gp_present::art::ADDED),
            "uses `#+` lines, which a page built before them takes for notes: there the theme is still offered, with the HUD's text under the bodies and each body's full picture everywhere, and the board's own screens where it builds rooms",
        );
    }
}

/// The game's pictures a rule draws: `pic` and `timed` pictures.
fn ruled_names(rules: &ArtRules) -> BTreeSet<String> {
    let mut s: BTreeSet<String> = rules.pics.iter().map(|p| p.build.clone()).collect();
    s.extend(rules.timed.iter().map(|t| t.build.clone()));
    if let Some(st) = &rules.stage {
        s.extend(st.levels.iter().map(|l| l.level.clone()));
    }
    // A screen a `#+ room` builds from pieces is drawn, as a `pic` line's.
    s.extend(rules.rooms.iter().flat_map(|r| r.screens.iter().cloned()));
    s
}

/// Every name of the game's pictures or levels the rules give, against
/// the game's build: one it hasn't got, the page passes over. And the
/// page's own resolution of the rules.
fn names_check(rules: &ArtRules, text: &str, art_rel: &str, b: &Built, r: &mut Report) {
    let have = b.names();
    // The page's own lookup: the names gp_present::art passes over.
    let pictures: Vec<&str> = have.iter().copied().collect();
    let mut unknown: BTreeMap<usize, Vec<&str>> = BTreeMap::new();
    for n in rules.passed_over(&pictures) {
        unknown
            .entry(line_of(text, n).unwrap_or(0))
            .or_default()
            .push(n);
    }
    let known: Vec<String> = have.iter().map(|s| (*s).to_owned()).collect();
    for (line, names) in &unknown {
        // Each name with the game's nearest, the likely typo's fix.
        let said: Vec<String> = names
            .iter()
            .map(|n| match gp_theme::slots::nearest(n, known.iter()) {
                Some(near) => format!("{n}, nearest {near}"),
                None => (*n).to_owned(),
            })
            .collect();
        let how = if b.board.is_some() {
            "Use the names of the pictures its board draws: a screen by its slot (as the game's model names it), a sprite as sprite-ENTITY-CODE with the picture code in hex, a counter's digits as counter-NAME-D and its repeated tile as counter-NAME (`gp theme look sheet` lists those the game shows)"
        } else {
            "Use the game's own picture names (frame-N-K.png, glyph-..., level-NAME.png, as the lines for the kit's slots give them)"
        };
        r.add(
            Group::Kit,
            Level::Problem,
            Some(art_rel),
            (*line > 0).then_some(*line),
            format!(
                "names {} this game doesn't have ({}), and the page passes over {}. {how}",
                if names.len() == 1 {
                    "a picture"
                } else {
                    "pictures"
                },
                said.join("; "),
                if names.len() == 1 { "it" } else { "them" },
            ),
        );
    }
    for (line, said) in under_problems(rules, &pictures) {
        r.add(Group::Kit, Level::Problem, Some(art_rel), Some(line), said);
    }
    if let Some(k) = &b.board {
        board_seen(rules, text, art_rel, k, r);
        if let Some(a) = &b.board_art {
            room_checks(rules, text, art_rel, a, k.seen.as_ref(), r);
        }
    }
    if let Err(e) = gp_present::art::Art::new(rules.clone(), b.present()) {
        r.add(Group::Kit, Level::Problem, Some(art_rel), None, format!("the page refuses these rules for this game: {e}. Check the picture names against the game's"));
    }
}

/// An arcade game's rooms (`#+ room`), built as the page builds them from
/// the screens in the person's lift: a rule whose class found no cell on
/// a screen the game shows (every screen of its looks, or with a sheet
/// those it showed) draws nothing there, and a piece no `stage_piece`
/// gives is passed over.
fn room_checks(
    rules: &ArtRules,
    text: &str,
    art_rel: &str,
    a: &gp_look::board::BoardArt,
    seen: Option<&BTreeMap<String, u64>>,
    r: &mut Report,
) {
    if rules.rooms.is_empty() {
        return;
    }
    let Ok(theme) = a.theme(text) else { return };
    let pieces: BTreeSet<&str> = rules
        .stage
        .iter()
        .flat_map(|st| st.pieces.iter().map(|p| p.0.as_str()))
        .collect();
    let shown = |screen: &str| seen.is_none_or(|s| s.contains_key(screen));
    for (line, class, screens) in theme.empty_rooms(&shown) {
        r.add(
            Group::Kit,
            Level::Problem,
            Some(art_rel),
            Some(line),
            format!(
                "its class `{class}` found no cell on {}, which the game shows, so it draws nothing there. Change the class's tests (its `#+ room_cells` line) to what that screen draws, by tile, bank or colour, or move the line to a room without that screen",
                screens.join(", ")
            ),
        );
    }
    for room in &rules.rooms {
        for place in &room.places {
            let missing: Vec<&str> = place
                .pieces()
                .into_iter()
                .filter(|p| !pieces.contains(p))
                .collect();
            if !missing.is_empty() {
                r.add(
                    Group::Kit,
                    Level::Problem,
                    Some(art_rel),
                    Some(place.line),
                    format!(
                        "names {} no `stage_piece` line gives, and the page passes over {}. Add `stage_piece NAME ART W H` or fix the name",
                        missing.join(", "),
                        if missing.len() == 1 { "it" } else { "them" }
                    ),
                );
            }
        }
    }
    for (line, said) in scroll_problems(rules) {
        r.add(Group::Kit, Level::Problem, Some(art_rel), Some(line), said);
    }
}

/// What is wrong with the `#+ under` lines, each with its line: a piece no
/// `stage_piece` line gives draws nothing on its frames, and an entity
/// none of the game's pictures (`pictures`) is a body of draws nothing at
/// all.
fn under_problems(rules: &ArtRules, pictures: &[&str]) -> Vec<(usize, String)> {
    let pieces: BTreeSet<&str> = rules
        .stage
        .iter()
        .flat_map(|st| st.pieces.iter().map(|p| p.0.as_str()))
        .collect();
    // The entities the game's bodies are of: sprite-ENTITY-CODE.
    let entities: BTreeSet<&str> = pictures
        .iter()
        .filter_map(|n| n.strip_prefix("sprite-")?.rsplit_once('-'))
        .filter(|(e, _)| pictures.iter().any(|n| gp_present::art::of_entity(n, e)))
        .map(|(e, _)| e)
        .collect();
    let mut out = Vec::new();
    for u in &rules.under {
        let mut missing: Vec<&str> = Vec::new();
        for (p, _) in &u.cycle {
            if !pieces.contains(p.as_str()) && !missing.contains(&p.as_str()) {
                missing.push(p);
            }
        }
        if !missing.is_empty() {
            out.push((
                u.line,
                format!(
                    "names {} no `stage_piece` line gives, and the page draws nothing on {} frames. Add `stage_piece NAME ART W H` or fix the name",
                    missing.join(", "),
                    if missing.len() == 1 { "its" } else { "their" }
                ),
            ));
        }
        if !entities.contains(u.entity.as_str()) {
            let have: Vec<&str> = entities.iter().copied().collect();
            out.push((
                u.line,
                format!(
                    "names the entity `{}`, which this game draws no body of (no picture named sprite-{}-CODE), so the page draws nothing under it. {}",
                    u.entity,
                    u.entity,
                    if have.is_empty() {
                        "The game's bodies aren't drawn by a board, so `#+ under` has nothing to follow".to_owned()
                    } else {
                        format!("Its entities are {}", have.join(", "))
                    }
                ),
            ));
        }
    }
    out
}

/// What is wrong with the rooms' `#+ room_scroll` lines, each with its
/// line: a piece no `stage_piece` line gives, or one no line of its room
/// lays, slides nothing.
fn scroll_problems(rules: &ArtRules) -> Vec<(usize, String)> {
    let pieces: BTreeSet<&str> = rules
        .stage
        .iter()
        .flat_map(|st| st.pieces.iter().map(|p| p.0.as_str()))
        .collect();
    let mut out = Vec::new();
    for room in &rules.rooms {
        for sc in &room.scrolls {
            let name = sc.piece.as_str();
            if !pieces.contains(name) {
                out.push((
                    sc.line,
                    format!(
                        "names {name} no `stage_piece` line gives, and the page passes over it. Add `stage_piece NAME ART W H` or fix the name"
                    ),
                ));
            } else if !room.places.iter().any(|p| p.pieces().contains(&name)) {
                out.push((
                    sc.line,
                    format!(
                        "slides {name}, which no line of its room lays, so nothing moves. Lay it with `#+ room_fill`, `#+ room_mask` or `#+ room_at` in the same room, or fix the name"
                    ),
                ));
            }
        }
    }
    out
}

/// An arcade game's board names against what the person's sheet saw the
/// game show (`gp theme look sheet`): a sprite code it never showed is
/// drawn only if the game shows it, so a warning; without a sheet, a note
/// saying how to make one.
fn board_seen(
    rules: &ArtRules,
    text: &str,
    art_rel: &str,
    k: &crate::theme_board::BoardKit,
    r: &mut Report,
) {
    let Some(seen) = &k.seen else {
        r.add(
            Group::Kit,
            Level::Note,
            None,
            None,
            format!(
                "the game is drawn by its board, whose {} pictures a theme may name; `gp theme look sheet --game GAME` plays its recorded inputs on your own romset and lists the sprite codes each entity shows, with a contact sheet ({}), and this check then says which of yours it never showed",
                k.names.len(),
                k.sheet.display()
            ),
        );
        return;
    };
    let have: BTreeSet<&str> = k.names.iter().map(|(n, _)| n.as_str()).collect();
    let mut never: BTreeMap<usize, Vec<&str>> = BTreeMap::new();
    let ruled = ruled_names(rules);
    for n in &ruled {
        if n.starts_with("sprite-") && have.contains(n.as_str()) && !seen.contains_key(n) {
            never
                .entry(line_of(text, n).unwrap_or(0))
                .or_default()
                .push(n);
        }
    }
    let total = never.values().map(Vec::len).sum::<usize>();
    let drawn = ruled.iter().filter(|n| seen.contains_key(*n)).count();
    if total > 0 {
        let first: Vec<&str> = never.values().flatten().take(8).copied().collect();
        r.add(
            Group::Kit,
            Level::Warning,
            Some(art_rel),
            never.keys().next().copied().filter(|l| *l > 0),
            format!(
                "{total} sprite name{} the game wasn't seen to show in its recorded plays ({}{}): each is drawn only if the game shows that code. The sheet in {} lists the codes it showed",
                if total == 1 { "" } else { "s" },
                first.join(", "),
                if total > first.len() { ", ..." } else { "" },
                k.sheet.display()
            ),
        );
    }
    r.add(
        Group::Kit,
        Level::Ok,
        None,
        None,
        format!(
            "{drawn} of the {} board pictures the game showed in its recorded plays are drawn by the theme",
            seen.len()
        ),
    );
}

/// An arcade game's board pictures with nothing drawn for them: each
/// screen its levels' looks show, and each name the person's sheet saw the
/// game show (`gp theme look sheet`'s names.toml), with no `pic` line (or,
/// for a screen, no `#+ room` building it). The
/// board's own picture shows for each, so a theme that leaves them is not
/// a skin of the whole game.
fn board_drawn(rules: &ArtRules, art_rel: &str, k: &crate::theme_board::BoardKit, r: &mut Report) {
    let ruled = ruled_names(rules);
    let pics: BTreeSet<&str> = ruled.iter().map(String::as_str).collect();
    let screen = |n: &str| !n.starts_with("sprite-") && !n.starts_with("counter-");
    let screens: Vec<&str> = k
        .names
        .iter()
        .map(|(n, _)| n.as_str())
        .filter(|n| screen(n) && !pics.contains(n))
        .filter(|n| k.seen.as_ref().is_none_or(|s| s.contains_key(*n)))
        .collect();
    if !screens.is_empty() {
        r.add(
            Group::Kit,
            Level::Problem,
            Some(art_rel),
            None,
            format!(
                "the game shows {} screen{} with nothing drawn for {}, so the board's own room shows behind your pictures: {}. Draw each from its sheet's picture (`gp theme sheet`, then `gp theme fit --large`), a `pic SLOT art/SLOT.png 512 480 0 0` line each",
                screens.len(),
                if screens.len() == 1 { "" } else { "s" },
                if screens.len() == 1 { "it" } else { "them" },
                screens.join(", ")
            ),
        );
    }
    let Some(seen) = &k.seen else { return };
    let left: Vec<&str> = seen
        .keys()
        .map(String::as_str)
        .filter(|n| !screen(n) && !pics.contains(n))
        .collect();
    if !left.is_empty() {
        let shown: Vec<&str> = left.iter().take(12).copied().collect();
        r.add(
            Group::Kit,
            Level::Problem,
            Some(art_rel),
            None,
            format!(
                "the game was seen to show {} picture{} in its recorded plays that no `pic` line draws, so the board's own picture shows for {}: {}{}. Each needs its own picture, from its cell of the sheets (`gp theme fit --large` writes the lines; {} lists what the game showed)",
                left.len(),
                if left.len() == 1 { "" } else { "s" },
                if left.len() == 1 { "it" } else { "them" },
                shown.join(", "),
                if left.len() > shown.len() { ", ..." } else { "" },
                k.sheet.join(crate::theme_board::SEEN_FILE).display()
            ),
        );
    }
}

/// The original's visible body of a game picture a `pic` line names, in
/// game pixels: a sprite frame's own drawn part (an NES build's
/// `frame-N-K.png`, a board's `sprite-ENTITY-CODE` as the board draws it
/// unflipped). None for a stage, a screen, a glyph or a counter, and for an
/// NES picture that isn't one of `poses` (a body's frames, by the kit).
fn original_body(b: &Built, name: &str, poses: Option<&BTreeSet<String>>) -> Option<[f64; 4]> {
    let ink = if let Some(rest) = name.strip_prefix("sprite-") {
        let art = b.board_art.as_ref()?;
        let code = u16::from_str_radix(rest.rsplit_once('-')?.1, 16).ok()?;
        let level = (0..b.board_art_levels()).find_map(|l| art.sprite_rgba(l, code, 0))?;
        let mut img = Rgba::new(32, 32);
        for (i, p) in level.iter().enumerate() {
            img.set(i as u32 % 32, i as u32 / 32, *p);
        }
        ink_box(&img)?
    } else if name.starts_with("frame-") && poses.is_none_or(|s| s.contains(name)) {
        let p = b.art.pictures.iter().find(|p| p.file == name)?;
        ink_box(&Rgba::from_png(&p.png).ok()?)?
    } else {
        return None;
    };
    Some(ink.map(f64::from))
}

/// Each sprite pose a `pic` line draws against the original's body in
/// that frame. Its bottom more than the kit's `feet` and `size` together
/// off is a problem, since the game stands the body on the original's
/// floor; more than `feet` alone, a warning. A picture wider or taller by
/// more than `size`, or its middle more than `edge` off, is a warning: large art draws its own figure, which needn't keep
/// the original's outline. A picture whose original is bigger than
/// [`BODY_MAX`] on a side is a scene, not a body, and isn't measured.
/// The largest original, in game pixels a side, the pose check takes for
/// a body.
const BODY_MAX: f64 = 64.0;

#[allow(clippy::too_many_arguments)]
fn body_check(
    tol: &crate::theme_geometry::Tolerance,
    rules: &ArtRules,
    text: &str,
    art_rel: &str,
    images: &BTreeMap<String, Rgba>,
    b: &Built,
    poses: Option<&BTreeSet<String>>,
    r: &mut Report,
) {
    let mut off: Vec<(f64, String)> = Vec::new();
    let mut shape: Vec<(f64, String)> = Vec::new();
    let mut measured = 0usize;
    for p in &rules.pics {
        let Some(own) = original_body(b, &p.build, poses) else {
            continue;
        };
        if own[2] - own[0] > BODY_MAX || own[3] - own[1] > BODY_MAX {
            continue;
        }
        let Some(img) = images.get(&p.art.art) else {
            continue;
        };
        let Some(ink) = ink_box(img) else { continue };
        let place = Place { img, rule: &p.art };
        let a = [
            place.to_game_x(ink[0]),
            place.to_game_y(ink[1]),
            place.to_game_x(ink[2]),
            place.to_game_y(ink[3]),
        ];
        measured += 1;
        let half = 0.5;
        let dw = (a[2] - a[0]) - (own[2] - own[0]);
        let dh = (a[3] - a[1]) - (own[3] - own[1]);
        let dx = (a[0] + a[2] - own[0] - own[2]) / 2.0;
        let dy = (a[1] + a[3] - own[1] - own[3]) / 2.0;
        let df = a[3] - own[3];
        let over = |v: f64, limit: i64, more: &str, less: &str, what: &str| {
            (v.abs() > limit as f64 + half).then(|| {
                (
                    v.abs() - limit as f64,
                    format!(
                        "{what} {:.1} px {}",
                        v.abs(),
                        if v > 0.0 { more } else { less }
                    )
                    .trim()
                    .to_owned(),
                )
            })
        };
        let line = pic_line(text, &p.build).unwrap_or(0);
        let said: Vec<(f64, String)> = [
            over(dw, tol.size, "wider", "narrower", ""),
            over(dh, tol.size, "taller", "shorter", ""),
            over(dx, tol.edge, "right", "left", "its middle"),
            over(dy, tol.edge, "low", "high", "its middle"),
        ]
        .into_iter()
        .flatten()
        .collect();
        if !said.is_empty() {
            let worst = said.iter().map(|s| s.0).fold(0f64, f64::max);
            let words: Vec<&str> = said.iter().map(|s| s.1.as_str()).collect();
            shape.push((
                worst,
                format!("{} (line {line}): {}", p.build, words.join(", ")),
            ));
        }
        // Off the floor by more than the feet and size tolerances together
        // is a problem (a body visibly standing in the air or sunk in
        // the floor); by less, a warning with the shape.
        if let Some((worst, said)) = over(df, tol.feet + tol.size, "low", "high", "its bottom") {
            off.push((worst, format!("{} (line {line}): {said}", p.build)));
        } else if let Some((worst, said)) = over(df, tol.feet, "low", "high", "its bottom") {
            shape.push((worst, format!("{} (line {line}): {said}", p.build)));
        }
    }
    if !shape.is_empty() {
        shape.sort_by(|x, y| y.0.total_cmp(&x.0));
        let n = shape.len();
        let shown: Vec<String> = shape.into_iter().take(5).map(|x| x.1).collect();
        r.add(
            Group::Fit,
            Level::Warning,
            Some(art_rel),
            None,
            format!(
                "{n} of {measured} poses differ from the original's body by more than the kit's tolerance (size {} px, middle {} px, bottom {} px). Large art may draw its own figure; look that each still reads as the body the game moves and stands where it stands; {}: {}",
                tol.size,
                tol.edge,
                tol.feet,
                if n > shown.len() { "the most" } else if n == 1 { "it is" } else { "they are" },
                shown.join("; ")
            ),
        );
    }
    if off.is_empty() {
        if measured > 0 {
            r.add(Group::Fit, Level::Ok, None, None, format!("{measured} poses stand within the kit's tolerance of the original's bottom ({} game pixels)", tol.feet));
        }
        return;
    }
    off.sort_by(|x, y| y.0.total_cmp(&x.0));
    let n = off.len();
    let shown: Vec<String> = off.into_iter().take(5).map(|x| x.1).collect();
    r.add(
        Group::Fit,
        Level::Problem,
        Some(art_rel),
        None,
        format!(
            "{n} of {measured} poses stand with their bottom off the original's by more than {} px (the kit's feet and size tolerances together), so the body is drawn off the floor the game stands it on. Fit them with `gp theme fit --large`, which stands each on the original's body, or move them with the rule's DY; {}: {}",
            tol.feet + tol.size,
            if n > shown.len() { "the worst" } else if n == 1 { "it is" } else { "they are" },
            shown.join("; ")
        ),
    );
}

/// Where each `pic` line's picture came from: theme/made-from.txt
/// (`gp theme fit --large` writes it) names, for each game picture, the
/// cell of the sheets it was fitted from (`cell SHEET CELL NAME...`), a
/// whole picture (`picture ID NAME...`), or that it was made another way
/// for that game picture (`placed NAME...`, or `hand`). Its `sheets` line names the
/// sheets.json; where that file is on this machine, each cell is held to
/// the game pictures sheets.json says it stands for.
fn made_from(
    folder: &Folder,
    rules: &ArtRules,
    text: &str,
    art_rel: &str,
    home: Option<&Path>,
    r: &mut Report,
) {
    use crate::theme_fit_large::{MADE_FROM, MADE_FROM_FORMAT};
    let mut pics: Vec<&str> = rules.pics.iter().map(|p| p.build.as_str()).collect();
    pics.sort_unstable();
    pics.dedup();
    if pics.is_empty() {
        return;
    }
    let path = folder.theme.join(MADE_FROM);
    let rel = folder.rel(&path);
    let Ok(t) = fs::read_to_string(&path) else {
        r.add(
            Group::Kit,
            Level::Problem,
            Some(&rel),
            None,
            format!(
                "{art_rel} has {} `pic` lines and nothing says where their pictures came from, so nothing ties each to the game's frame it stands for. `gp theme fit --large` fits the sheets' cells (sheets.json names the game's pictures each cell is) and writes {MADE_FROM}; for a picture made for a game picture some other way (generated and placed by your own script, edited in a paint program), add `placed NAME` to it (its first line `{MADE_FROM_FORMAT}`)",
                pics.len()
            ),
        );
        return;
    };
    #[derive(Clone)]
    enum From {
        Cell(String, String),
        Picture(String),
        Hand,
    }
    let mut from: BTreeMap<String, (usize, From)> = BTreeMap::new();
    let mut sheets_line: Option<String> = None;
    let mut lines = t.lines().enumerate();
    if lines.next().map(|(_, l)| l.trim()) != Some(MADE_FROM_FORMAT) {
        r.add(
            Group::Kit,
            Level::Problem,
            Some(&rel),
            Some(1),
            format!("its first line is `{MADE_FROM_FORMAT}`"),
        );
        return;
    }
    for (i, l) in lines {
        let w: Vec<&str> = l.split_whitespace().collect();
        let (names, src) = match w.as_slice() {
            [] => continue,
            [c, ..] if c.starts_with('#') => continue,
            ["sheets", p] => {
                sheets_line = Some((*p).to_owned());
                continue;
            }
            ["cell", sheet, cell, names @ ..] if !names.is_empty() => {
                (names, From::Cell((*sheet).to_owned(), (*cell).to_owned()))
            }
            ["picture", id, names @ ..] if !names.is_empty() => {
                (names, From::Picture((*id).to_owned()))
            }
            ["placed" | "hand", names @ ..] if !names.is_empty() => (names, From::Hand),
            _ => {
                r.add(Group::Kit, Level::Problem, Some(&rel), Some(i + 1), "a line it can't read: each is `sheets PATH`, `cell SHEET CELL NAME...`, `picture ID NAME...` or `placed NAME...`");
                continue;
            }
        };
        for n in names {
            if from.contains_key(*n) {
                r.add(
                    Group::Kit,
                    Level::Problem,
                    Some(&rel),
                    Some(i + 1),
                    format!("names {n} a second time: each game picture has one source"),
                );
                continue;
            }
            from.insert((*n).to_owned(), (i + 1, src.clone()));
        }
    }
    let unsourced: Vec<&str> = pics
        .iter()
        .copied()
        .filter(|n| !from.contains_key(*n))
        .collect();
    if !unsourced.is_empty() {
        let shown: Vec<&str> = unsourced.iter().take(8).copied().collect();
        r.add(
            Group::Kit,
            Level::Problem,
            Some(art_rel),
            pic_line(text, unsourced[0]),
            format!(
                "{} `pic` line{} whose picture {MADE_FROM} doesn't say came from a sheet's cell or was placed for that game picture: {}{}. Fit them with `gp theme fit --large`, or add `placed NAME` for one made for it another way",
                unsourced.len(),
                if unsourced.len() == 1 { "" } else { "s" },
                shown.join(", "),
                if unsourced.len() > shown.len() { ", ..." } else { "" }
            ),
        );
    }
    let cells = from
        .values()
        .filter(|(_, s)| !matches!(s, From::Hand))
        .count();
    let hand = from.len() - cells;
    if cells == 0 {
        return;
    }
    let Some(sl) = sheets_line else {
        r.add(
            Group::Kit,
            Level::Problem,
            Some(&rel),
            None,
            "it names cells but no `sheets PATH` line, the sheets.json the cells are in",
        );
        return;
    };
    let manifest = Some(match home {
        Some(h) => crate::theme_fit_large::sheets_path(&sl, h),
        None => PathBuf::from(&sl),
    })
    .filter(|p| p.is_file())
    .and_then(|p| crate::theme_sheet::Manifest::load(p.parent()?).ok());
    let Some(m) = manifest else {
        r.add(Group::Kit, Level::Note, Some(&rel), None, format!("{cells} pictures from the sheets' cells and {hand} placed by other means; the sheets.json it names ({sl}) isn't on this machine, so the check can't hold each cell to the game pictures it stands for there"));
        return;
    };
    let mut wrong: Vec<String> = Vec::new();
    for (name, (line, s)) in &from {
        let stands: Option<Vec<&String>> = match s {
            From::Cell(sheet, cell) => m
                .sheets
                .iter()
                .find(|x| x.id == *sheet)
                .and_then(|x| x.cells.iter().find(|c| c.cell == *cell))
                .map(|c| c.pictures.iter().flatten().collect()),
            From::Picture(id) => m
                .pictures
                .iter()
                .find(|p| p.id == *id)
                .map(|p| p.pictures.iter().collect()),
            From::Hand => continue,
        };
        match stands {
            None => wrong.push(format!("line {line}: {name}'s cell isn't in sheets.json")),
            // A cell the sheet couldn't name pictures for can't be held.
            Some(v) if v.is_empty() => {}
            Some(v) if !v.contains(&name) => wrong.push(format!(
                "line {line}: {name}, but that cell stands for {}",
                v.iter()
                    .take(4)
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
            Some(_) => {}
        }
    }
    if wrong.is_empty() {
        r.add(Group::Kit, Level::Ok, None, None, format!("{cells} pictures each fitted from the sheets' cell that stands for its game picture, and {hand} placed by other means"));
    } else {
        let n = wrong.len();
        let shown: Vec<String> = wrong.into_iter().take(6).collect();
        r.add(
            Group::Kit,
            Level::Problem,
            Some(&rel),
            None,
            format!("{n} game pictures given a cell of the sheets that doesn't stand for them (sheets.json ties each cell to the game's frames), so the picture shows another pose: {}. Fit again with `gp theme fit --large`", shown.join("; ")),
        );
    }
}

/// Every kit slot drawn: each pose of each body, the stages, the screens
/// and the digits.
fn coverage(
    kit: &Kit,
    rules: Option<&ArtRules>,
    ruled: &BTreeSet<String>,
    b: Option<&Built>,
    built: &Result<Built, String>,
    r: &mut Report,
) {
    let scale = rules.map_or(6, |x| x.scale.max(1));
    let empty_rules = ruled.is_empty();
    let mut drawn = 0usize;
    let mut unknown = Vec::new();
    for slot in &kit.art {
        let label = if slot.label.is_empty() {
            slot.id.clone()
        } else {
            slot.label.clone()
        };
        let pose = |i: usize| {
            slot.poses
                .get(i)
                .cloned()
                .unwrap_or_else(|| slot.frame_names.get(i).cloned().unwrap_or_default())
        };
        let level_slot = kit.surfaces.iter().any(|s| s.picture == slot.id);
        let how = |n: usize| -> String {
            if level_slot {
                "build it from pieces with a `stage` line, `stage_piece` lines and a `level` line for its level picture (a stage is never one picture)".into()
            } else {
                format!(
                    "{n} pose{}, {} by {} game pixels (draw at {} by {} for scale {scale})",
                    if n == 1 { "" } else { "s" },
                    slot.size[0],
                    slot.size[1],
                    slot.size[0] * scale,
                    slot.size[1] * scale
                )
            }
        };
        let frames = match b {
            Some(b) => slot_pictures(kit, slot, b),
            None => None,
        };
        let Some(frames) = frames else {
            if empty_rules {
                let poses: Vec<String> = (0..slot.frame_names.len()).map(pose).collect();
                r.add(
                    Group::Kit,
                    Level::Problem,
                    None,
                    None,
                    format!(
                        "still to draw: {label} ({}), {}: {}",
                        slot.id,
                        how(slot.frame_names.len()),
                        poses.join("; ")
                    ),
                );
            } else {
                unknown.push(format!("{label} ({})", slot.id));
            }
            continue;
        };
        let b = b.expect("frames come from a build");
        let mut missing = Vec::new();
        let mut partial = Vec::new();
        let mut needed = 0usize;
        for (i, pics) in frames.iter().enumerate() {
            let shown: Vec<u32> = pics.iter().copied().filter(|&p| !b.blank(p)).collect();
            if shown.is_empty() {
                continue;
            }
            needed += 1;
            let without: Vec<&str> = shown
                .iter()
                .map(|&p| b.file(p))
                .filter(|f| !ruled.contains(*f))
                .collect();
            if without.len() == shown.len() {
                if level_slot {
                    missing.push(format!("its level picture is {}", without[0]));
                } else {
                    missing.push(format!("{} ({})", pose(i), without[0]));
                }
            } else if !without.is_empty() {
                partial.push(format!("{} ({})", pose(i), without.join(", ")));
            }
        }
        if needed == 0 {
            continue;
        }
        if missing.len() == needed {
            r.add(
                Group::Kit,
                Level::Problem,
                None,
                None,
                format!(
                    "still to draw: {label} ({}), {}: {}",
                    slot.id,
                    how(needed),
                    missing.join("; ")
                ),
            );
        } else if !missing.is_empty() {
            r.add(
                Group::Kit,
                Level::Problem,
                None,
                None,
                format!("{label} ({}): {} of {needed} poses still to draw, so the game's own picture shows for them: {}", slot.id, missing.len(), missing.join("; ")),
            );
        }
        if !partial.is_empty() {
            r.add(
                Group::Kit,
                Level::Problem,
                None,
                None,
                format!("{label} ({}): drawn on some stages only. The game draws these poses in other colours on other stages, as the pictures named here, which have no rule yet, so the game's own picture shows there; add a rule for each: {}", slot.id, partial.join("; ")),
            );
        }
        if missing.is_empty() && partial.is_empty() {
            drawn += 1;
        }
    }
    if let Err(why) = built {
        r.add(
            Group::Kit,
            Level::Note,
            None,
            None,
            format!("without the game's pictures ({why}) the check can't tell which of them your rules name; put your ROM in $GP_ROM_DIR (or `gp roms add FILE`) or give it with --rom"),
        );
    }
    if !unknown.is_empty() {
        r.add(
            Group::Kit,
            Level::Note,
            None,
            None,
            format!("the check can't tell which of the game's pictures these are, so it can't say whether they're drawn; look at them in the game: {}", unknown.join(", ")),
        );
    }
    if drawn > 0 {
        r.add(
            Group::Kit,
            Level::Ok,
            None,
            None,
            format!(
                "{drawn} of the kit's {} art slots drawn in every pose",
                kit.art.len()
            ),
        );
    }
}

/// Where a rule draws a picture in the game's pixels: art pixel to game
/// pixel, and back.
struct Place<'a> {
    img: &'a Rgba,
    rule: &'a Sized,
}

impl Place<'_> {
    fn to_game_y(&self, ay: u32) -> f64 {
        self.rule.offset[1] as f64
            + f64::from(ay) * self.rule.size[1] as f64 / f64::from(self.img.height)
    }
    fn to_game_x(&self, ax: u32) -> f64 {
        self.rule.offset[0] as f64
            + f64::from(ax) * self.rule.size[0] as f64 / f64::from(self.img.width)
    }
    /// Whether any drawn pixel of the art falls in the game pixel (x, y).
    fn inked(&self, x: i64, y: i64) -> bool {
        let sx = f64::from(self.img.width) / self.rule.size[0] as f64;
        let sy = f64::from(self.img.height) / self.rule.size[1] as f64;
        let ax0 = ((x - self.rule.offset[0]) as f64 * sx).floor();
        let ay0 = ((y - self.rule.offset[1]) as f64 * sy).floor();
        let (ax1, ay1) = ((ax0 + sx).ceil(), (ay0 + sy).ceil());
        let clamp = |v: f64, max: u32| v.max(0.0).min(f64::from(max)) as u32;
        let (x0, x1) = (clamp(ax0, self.img.width), clamp(ax1, self.img.width));
        let (y0, y1) = (clamp(ay0, self.img.height), clamp(ay1, self.img.height));
        (y0..y1).any(|ay| (x0..x1).any(|ax| self.img.get(ax, ay)[3] >= INK))
    }
}

/// Each sprite pose's picture against its slot's boxes and feet row.
fn fit(
    kit: &Kit,
    rules: &ArtRules,
    text: &str,
    art_rel: &str,
    images: &BTreeMap<String, Rgba>,
    b: &Built,
    r: &mut Report,
) {
    let tol = &kit.tolerance;
    let rule_of = |build: &str| rules.pics.iter().find(|p| p.build == build).map(|p| &p.art);
    let mut checked = 0usize;
    for slot in &kit.art {
        let Some(geo) = &slot.geometry else { continue };
        let Some(frames) = slot_pictures(kit, slot, b) else {
            continue;
        };
        let label = if slot.label.is_empty() {
            slot.id.clone()
        } else {
            slot.label.clone()
        };
        let pose = |i: usize| {
            slot.poses
                .get(i)
                .cloned()
                .unwrap_or_else(|| slot.frame_names[i].clone())
        };
        let (mut feet, mut cover, mut small) = (Vec::new(), Vec::new(), Vec::new());
        let mut seen = BTreeSet::new();
        for (i, pics) in frames.iter().enumerate() {
            let Some(name) = slot.frame_names.get(i) else {
                continue;
            };
            let Some((pic, rule)) = pics
                .iter()
                .find_map(|&p| rule_of(b.file(p)).map(|r| (p, r)))
            else {
                continue;
            };
            // Two poses the game draws with one picture are measured once.
            if !seen.insert(pic) {
                continue;
            }
            let Some(img) = images.get(&rule.art) else {
                continue;
            };
            let Some(ink) = ink_box(img) else { continue };
            // The game's own picture, measured only: the theme is held to
            // what it does.
            let Some(own_img) = Rgba::from_png(&b.art.pictures[pic as usize].png).ok() else {
                continue;
            };
            let Some(own) = ink_box(&own_img) else {
                continue;
            };
            let place = Place { img, rule };
            let own_rule = Sized {
                art: String::new(),
                size: [i64::from(own_img.width), i64::from(own_img.height)],
                offset: [0, 0],
            };
            let own_place = Place {
                img: &own_img,
                rule: &own_rule,
            };
            let build = b.file(pic).to_owned();
            let at = format!(
                "{} ({build}, line {})",
                pose(i),
                pic_line(text, &build).unwrap_or(0)
            );
            checked += 1;
            // Feet: where the game's own picture stands on the feet row,
            // the art's bottom should too.
            if let Some(row) = geo.feet
                && (i64::from(own[3]) - row).abs() <= 1
            {
                let off = place.to_game_y(ink[3]) - row as f64;
                if off.abs() > tol.feet as f64 + 0.5 {
                    feet.push((
                        off.abs(),
                        format!(
                            "{at}: {:.1} px {}",
                            off.abs(),
                            if off < 0.0 { "above it" } else { "below it" }
                        ),
                    ));
                }
            }
            // Touch boxes in the frames that harm, where the game's own
            // picture covers them.
            if geo.danger.iter().any(|d| d == name) {
                for bx in geo.boxes.iter().filter(|x| {
                    x.kind == "touch" && (x.frames.is_empty() || x.frames.contains(name))
                }) {
                    let [x, y, w, h] = bx.rect;
                    let total = (w.max(0) * h.max(0)) as f64;
                    if total == 0.0 {
                        continue;
                    }
                    let part = |pl: &Place| {
                        let mut n = 0i64;
                        for gy in y..y + h {
                            for gx in x..x + w {
                                n += i64::from(pl.inked(gx, gy));
                            }
                        }
                        n as f64 / total
                    };
                    let (ours, theirs) = (part(&place), part(&own_place));
                    if ours < tol.cover && theirs >= tol.cover {
                        cover.push((
                            1.0 - ours,
                            format!(
                                "{at}: covers {:.0}% of the box at {x},{y}, {w} by {h} ({})",
                                ours * 100.0,
                                bx.what
                            ),
                        ));
                    }
                }
            }
            // Size: smaller than the game needs, where the game's own
            // picture isn't.
            let [mw, mh] = geo.min_size;
            let (w, h) = (
                place.to_game_x(ink[2]) - place.to_game_x(ink[0]),
                place.to_game_y(ink[3]) - place.to_game_y(ink[1]),
            );
            let (ow, oh) = (i64::from(own[2] - own[0]), i64::from(own[3] - own[1]));
            let short_w = w + 0.5 < mw as f64 && ow >= mw;
            let short_h = h + 0.5 < mh as f64 && oh >= mh;
            if short_w || short_h {
                small.push((0.0, format!("{at}: {w:.0} by {h:.0}")));
            }
        }
        let mut say = |mut list: Vec<(f64, String)>, what: String| {
            if list.is_empty() {
                return;
            }
            list.sort_by(|a, b| b.0.total_cmp(&a.0));
            let n = list.len();
            let shown: Vec<String> = list.into_iter().take(3).map(|x| x.1).collect();
            r.add(
                Group::Fit,
                Level::Warning,
                Some(art_rel),
                None,
                format!(
                    "{label} ({}), {n} pose{}: {what}. {}: {}",
                    slot.id,
                    if n == 1 { "" } else { "s" },
                    if n > 3 {
                        "The worst"
                    } else if n == 1 {
                        "It is"
                    } else {
                        "They are"
                    },
                    shown.join("; ")
                ),
            );
        };
        if let Some(row) = geo.feet {
            say(
                feet,
                format!(
                    "the feet stand more than {} px off the row the game stands this body on (row {row} of its {} by {} picture); move the picture with its rule's DY, or redraw the feet",
                    tol.feet, slot.size[0], slot.size[1]
                ),
            );
        }
        say(
            cover,
            format!(
                "the drawing covers less than {:.0}% of the box the game tests touches with, where the game's own picture covers it, so a touch counts where nothing is drawn; draw that part over the box, or move the picture with DX, DY",
                tol.cover * 100.0
            ),
        );
        say(
            small,
            format!(
                "drawn smaller than the {} by {} game pixels the game needs for this body to read; draw it larger, or give its rule a larger W and H",
                geo.min_size[0], geo.min_size[1]
            ),
        );
    }
    if checked > 0 {
        r.add(Group::Fit, Level::Note, None, None, format!("{checked} poses measured against the kit's boxes and feet rows; this can't see how a picture reads at game size, how poses flow, or a stage's ladders and floors: play it and look at every stage"));
    }
}

/// Pictures drawn at a shape other than their own.
fn stretch(
    rules: &ArtRules,
    text: &str,
    art_rel: &str,
    images: &BTreeMap<String, Rgba>,
    r: &mut Report,
) {
    let mut out: Vec<(usize, String)> = Vec::new();
    let mut seen = BTreeSet::new();
    let mut look = |art: &str, w: i64, h: i64, key: &str| {
        let Some(img) = images.get(art) else { return };
        if w <= 0 || h <= 0 || !seen.insert((art.to_owned(), w, h)) {
            return;
        }
        let own = f64::from(img.width) / f64::from(img.height);
        let drawn = w as f64 / h as f64;
        if (own / drawn - 1.0).abs() > STRETCH {
            out.push((
                line_of(text, key).unwrap_or(0),
                format!(
                    "{art} is {} by {} but drawn {w} by {h}",
                    img.width, img.height
                ),
            ));
        }
    };
    for p in &rules.pics {
        look(&p.art.art, p.art.size[0], p.art.size[1], &p.art.art);
    }
    for t in &rules.timed {
        look(&t.art.art, t.art.size[0], t.art.size[1], &t.art.art);
    }
    if let Some(st) = &rules.stage {
        for (_, a, s) in &st.pieces {
            look(a, s[0], s[1], a);
        }
    }
    if out.is_empty() {
        return;
    }
    out.sort();
    let n = out.len();
    let shown: Vec<String> = out
        .iter()
        .take(5)
        .map(|(l, s)| {
            if *l > 0 {
                format!("line {l}: {s}")
            } else {
                s.clone()
            }
        })
        .collect();
    r.add(
        Group::Fit,
        Level::Warning,
        Some(art_rel),
        out.first().map(|x| x.0).filter(|l| *l > 0),
        format!("{n} picture{} drawn stretched (its shape differs from its rule's width and height by more than {:.0}%){}: {}. Give the rule the picture's own shape, or crop the picture", if n == 1 { "" } else { "s" }, STRETCH * 100.0, if n > 5 { ", the first" } else { "" }, shown.join("; ")),
    );
}

/// The theme's names against the kit's `deny` list.
fn words(
    folder: &Folder,
    pkg: Option<&Pkg>,
    look: Option<&Look>,
    files: &[String],
    art_text: Option<&str>,
    game: &Game,
    r: &mut Report,
) {
    if game.deny.is_empty() {
        r.add(Group::Words, Level::Note, None, None, format!("{} has no list of names a theme may not use (themes/kit/words.toml), so names weren't checked", game.title));
        return;
    }
    let mut hits = 0;
    let mut say = |r: &mut Report, file: Option<&str>, what: &str, text: &str| {
        let d = crate::theme_words::denied(text, &game.deny);
        if !d.is_empty() {
            hits += 1;
            r.add(Group::Words, Level::Problem, file, None, format!("{what} `{text}` uses {}, from the kit's list of the original's names, and players see it. Choose words of your own", d.join(", ")));
        }
    };
    if let Some(p) = pkg {
        if let Some(n) = &p.name {
            say(r, Some("package.toml"), "the package name", n);
        }
        if let Some(t) = &p.title {
            say(r, Some("package.toml"), "the title", t);
        }
    }
    let look_rel = folder.rel(&folder.theme.join(gp_present::looks::LOOK_FILE));
    if let Some(l) = look {
        say(r, Some(&look_rel), "the theme id", &l.id);
        say(r, Some(&look_rel), "the theme title", &l.title);
    }
    // File names and art.txt's notes: plain words such as these are not
    // what a player sees, so a note once a word (the words file doesn't
    // tell the game's names from plain nouns).
    let mut by_word: BTreeMap<String, usize> = BTreeMap::new();
    for f in files {
        for d in crate::theme_words::denied(f, &game.deny) {
            *by_word.entry(d).or_default() += 1;
        }
    }
    for l in art_text
        .unwrap_or("")
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with('#'))
    {
        for d in crate::theme_words::denied(l, &game.deny) {
            *by_word.entry(d).or_default() += 1;
        }
    }
    if !by_word.is_empty() {
        let list: Vec<String> = by_word
            .iter()
            .map(|(w, n)| format!("`{w}` {n} time{}", if *n == 1 { "" } else { "s" }))
            .collect();
        r.add(
            Group::Words,
            Level::Note,
            None,
            None,
            format!(
                "file names or notes in art.txt use words from the kit's list of the original's names ({}). Players don't see these, so they're allowed; words of your own read better",
                list.join(", ")
            ),
        );
    }
    if hits == 0 {
        r.add(Group::Words, Level::Ok, None, None, format!("no name from the kit's list of {} of the original's names in what players see: the package's name and title, the look's id and title", game.deny.len()));
    }
}

/// `gp template served GAME --folder THEME/theme`.
fn rom_data(folder: &Folder, game: &Game, rom: Option<&Path>, paths: &Paths, r: &mut Report) {
    if !folder.theme.is_dir() {
        return;
    }
    let given: Vec<PathBuf> = rom.map(Path::to_owned).into_iter().collect();
    match crate::served::served(
        &game.dir,
        std::slice::from_ref(&folder.theme),
        &given,
        paths,
    ) {
        Err(e) => r.add(
            Group::Rom,
            Level::Note,
            None,
            None,
            format!("skipped: the ROM-data check couldn't run ({e:#})"),
        ),
        Ok(o) => {
            let problems = o.json["problems"].as_array().cloned().unwrap_or_default();
            for p in &problems {
                let file = p["file"].as_str().unwrap_or("");
                let file = file
                    .strip_prefix("theme/")
                    .map_or_else(|| file.to_owned(), |f| folder.theme_rel(f));
                r.add(
                    Group::Rom,
                    Level::Problem,
                    Some(&file),
                    None,
                    format!(
                        "{}. A theme may hold nothing from the ROM: replace it with your own work",
                        p["message"].as_str().unwrap_or("")
                    ),
                );
            }
            let missing: Vec<String> = o.json["not_compared"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            if !missing.is_empty() {
                r.add(
                    Group::Rom,
                    Level::Warning,
                    None,
                    None,
                    format!("skipped for {}: no ROM of it on this machine, so nothing here says your files are free of its data. Put your ROM in $GP_ROM_DIR (or `gp roms add FILE`) and check again", missing.join(", ")),
                );
            }
            if problems.is_empty() && missing.is_empty() {
                r.add(
                    Group::Rom,
                    Level::Ok,
                    None,
                    None,
                    format!(
                        "{} files compared with the ROM: none holds its data",
                        o.json["files"]
                    ),
                );
            }
        }
    }
}

/// The road to see a theme in the game, in commands.
fn play_road(folder: &Folder, game: &Game) -> Vec<String> {
    let g = game.dir.file_name().map_or_else(
        || game.dir.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    vec![
        format!(
            "gp theme look play {} --game {g} --open",
            folder.root.display()
        ),
        format!(
            "gp theme look play {} --game {g} --shots DIR   (pictures at the game's moments)",
            folder.root.display()
        ),
    ]
}

fn finish(folder: &Folder, game: &Game, pkg: Option<&Pkg>, r: Report) -> Outcome {
    let problems = r.count(Level::Problem);
    let warnings = r.count(Level::Warning);
    let name = pkg.and_then(|p| p.name.as_deref());
    let road = play_road(folder, game);
    let first = r.found.iter().find(|f| f.level == Level::Problem);
    let to_draw = r
        .found
        .iter()
        .filter(|f| {
            f.level == Level::Problem
                && f.group == Group::Kit
                && f.text.starts_with("still to draw")
        })
        .count();
    let next = match first {
        Some(f) if f.group == Group::Kit && to_draw > 0 && problems == to_draw => format!(
            "draw the {to_draw} slot{} still to draw, add a rule for each in art.txt, and check again",
            if to_draw == 1 { "" } else { "s" }
        ),
        Some(f) => format!(
            "fix the first problem{}, then check again",
            match (&f.file, f.line) {
                (Some(file), Some(l)) => format!(" ({file} line {l})"),
                (Some(file), None) => format!(" ({file})"),
                _ => format!(" ({})", f.group.title().split(' ').next().unwrap_or("")),
            }
        ),
        None => "play it in the game and look at every stage, every pose at game size, the deaths and the score display".into(),
    };
    let mut text = format!(
        "Checking {} against {} ({})\n",
        folder.root.display(),
        game.title,
        game.dir.display()
    );
    let mut groups: Vec<Group> = r.found.iter().map(|f| f.group).collect();
    groups.sort();
    groups.dedup();
    for g in &groups {
        let _ = writeln!(text, "\n{}", g.title());
        let mut fs: Vec<&Finding> = r.found.iter().filter(|f| f.group == *g).collect();
        fs.sort_by_key(|f| f.level);
        for f in fs {
            let at = match (&f.file, f.line) {
                (Some(file), Some(l)) => format!("{file} line {l}: "),
                (Some(file), None) => format!("{file}: "),
                _ => String::new(),
            };
            let _ = writeln!(text, "  {}: {at}{}", f.level.word(), f.text);
        }
    }
    let _ = writeln!(text, "\nTo see it in the game:");
    for l in &road {
        let _ = writeln!(text, "  {l}");
    }
    let _ = write!(
        text,
        "\n{problems} problem{}, {warnings} warning{}. Next: {next}.",
        if problems == 1 { "" } else { "s" },
        if warnings == 1 { "" } else { "s" },
    );
    let findings: Vec<Value> = r
        .found
        .iter()
        .map(|f| {
            json!({
                "group": f.group.key(),
                "level": f.level.word(),
                "file": f.file,
                "line": f.line,
                "message": f.text,
            })
        })
        .collect();
    Outcome {
        json: json!({
            "ok": problems == 0,
            "theme": folder.root.display().to_string(),
            "name": name,
            "game": game.dir.display().to_string(),
            "problems": problems,
            "warnings": warnings,
            "findings": findings,
            "next": next,
            "play": road,
        }),
        text,
        exit: u8::from(problems > 0),
    }
}

// ---------------------------------------------------------------------
// A new theme.

fn new(
    dir: &Path,
    game_arg: &Path,
    name: &str,
    title: &str,
    license: &str,
    home: &Path,
) -> Result<Outcome> {
    if dir.exists() {
        let busy = fs::read_dir(dir)
            .with_context(|| format!("reading {}", dir.display()))?
            .next()
            .is_some();
        if busy {
            bail!(
                "{} already holds files; give a new or empty folder, so nothing of yours is written over",
                dir.display()
            );
        }
    }
    if !new_theme_name_ok(name) {
        bail!(
            "`{name}` isn't a theme name a registry takes; use `{}`: NAMESPACE/NAME in lowercase letters, digits and -, the part after the slash starting with a letter (like lex00/turnips)",
            fixed_theme_name(name)
        );
    }
    let title = title.trim();
    if title.is_empty() || title.contains('\n') {
        bail!("give the theme a title on one line, with --title");
    }
    let game = game_of(game_arg)?;
    for (what, text) in [("name", name), ("title", title)] {
        let d = crate::theme_words::denied(text, &game.deny);
        if !d.is_empty() {
            bail!(
                "the {what} `{text}` uses {}, a name from the original game, which a theme may not use; choose another",
                d.join(", ")
            );
        }
    }
    // The look's id is the name's last part, as the page names a theme.
    let id = name.split_once('/').map_or(name, |x| x.1).to_owned();
    let folder = Folder {
        root: dir.to_owned(),
        theme: dir.join("theme"),
    };
    // An arcade game drawn by its board: the board's names, from its model
    // (and the codes the person's own sheet saw, when there is one). Its
    // picture is already 512 by 480, so the art is drawn at twice that.
    let board = crate::theme::read_model(&game.dir)
        .ok()
        .and_then(|(m, _)| Some((crate::theme_board::model_names(&m)?, m.model.name)));
    let scale = if board.is_some() { 2u32 } else { 6u32 };
    let mut art = format!(
        "gp-art 1\n# {title}: the rules for a large-art theme of {} (site docs, themes.md, lists every rule).\n# Each picture the game's theme kit lists is below, marked TO DRAW. Draw it into art/,\n# then write its `pic` lines (`gp theme look check` names the game's pictures for each pose).\nscale {scale}\nlimit off\n",
        game.title
    );
    let mut slots = 0;
    if let Some((names, model)) = &board {
        let seen = crate::theme_board::read_seen(&crate::theme_board::sheet_dir(home, model));
        let (lines, n) = crate::theme_board::new_lines(names, seen.as_ref(), &game.title);
        art.push_str(&lines);
        slots += n;
    }
    match &game.kit {
        None if board.is_some() => {}
        None => {
            let _ = writeln!(
                art,
                "# {} has no theme kit yet, so no list of what to draw; `gp theme kit` makes one.",
                game.title
            );
        }
        Some(kit) => {
            for s in &kit.art {
                slots += 1;
                let label = if s.label.is_empty() {
                    s.id.as_str()
                } else {
                    s.label.as_str()
                };
                let poses = if s.poses.is_empty() {
                    s.frame_names.join(", ")
                } else {
                    s.poses.join("; ")
                };
                let mut geo = String::new();
                if let Some(g) = &s.geometry {
                    for bx in &g.boxes {
                        let [x, y, w, h] = bx.rect;
                        let _ = write!(geo, "; {} box {x},{y} {w} by {h}", bx.kind);
                    }
                    if let Some(f) = g.feet {
                        let _ = write!(geo, "; feet on row {f}");
                    }
                }
                let _ = writeln!(
                    art,
                    "\n# TO DRAW {} ({}): {} by {} game pixels, draw at {} by {}{geo}. {} pose{}: {poses}",
                    label,
                    s.id,
                    s.size[0],
                    s.size[1],
                    s.size[0] * scale,
                    s.size[1] * scale,
                    s.frame_names.len(),
                    if s.frame_names.len() == 1 { "" } else { "s" },
                );
            }
        }
    }
    let package = format!(
        "# {name}: {title}, a large-art theme for {}.\n[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nkind = \"content\"\ntitle = \"{}\"\nlicense = \"{}\"\n",
        game.title,
        title.replace('\\', "\\\\").replace('"', "\\\""),
        license.replace('"', "")
    );
    let look = Look {
        id: id.clone(),
        title: title.to_owned(),
        art: Some(ART_FILE_DEFAULT.into()),
        ..Look::default()
    }
    .write();
    let deny = if game.deny.is_empty() {
        String::new()
    } else {
        format!(
            " The game's kit lists the names a theme may not use: {}.",
            game.deny.join(", ")
        )
    };
    let game_arg_s = game_arg.display();
    let road = play_road(&folder, &game);
    let readme = format!(
        "# {title}\n\nA theme for {} on Game Printer: new pictures, drawn by you, for every body, stage and screen. The game plays as it always does; every body keeps its place and its hit box, and only the pictures change. A player picks it under Settings, Theme.\n\n## What's here\n\n- `package.toml` names the package, `{name}`, and its licence ({license}).\n- `theme/look.txt` names the look and its rules file.\n- `theme/art.txt` holds the rules (`gp-art 1`): which of the game's pictures each of yours replaces, and at what size. Each line marked TO DRAW is a picture still to make.\n- `theme/art/` holds your pictures, as PNG, at {scale} times the game's pixel size.\n\n## The rules\n\n- Your own art only: no character's likeness, name or logo from the original, and nothing traced.\n- Nothing from the ROM: no picture, sound or data read from it. A stage is never a picture here; the rules build it from your pieces over the level picture the player's own ROM gives.\n- No names from the original.{deny}\n- If an AI tool made any of it, name the tool here, and make sure its terms let you share the result.\n\n## Check it\n\n```sh\ngp theme look check . --game {game_arg_s}\n```\n\nIt reads the theme the way the page does and lists what is wrong and where, what is still to draw, how each picture sits on its body's box and feet row, names the theme may not use, and whether any file holds data from your ROM. Fix the problems first; warnings are worth a look.\n\n## See it in the game\n\n{}\n\nLook at every stage, every pose at game size, the deaths and the score display.\n\nKeep the pictures in Git LFS (`.gitattributes` does that once you run `git lfs install`).\n",
        game.title,
        road.iter()
            .map(|l| format!("    {l}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    let write = |rel: &str, body: &str| -> Result<()> {
        let p = dir.join(rel);
        if let Some(d) = p.parent() {
            fs::create_dir_all(d).with_context(|| format!("creating {}", d.display()))?;
        }
        fs::write(&p, body).with_context(|| format!("writing {}", p.display()))
    };
    write("package.toml", &package)?;
    write("theme/look.txt", &look)?;
    write("theme/art.txt", &art)?;
    write("README.md", &readme)?;
    write(
        ".gitattributes",
        "*.png filter=lfs diff=lfs merge=lfs -text\n",
    )?;
    fs::create_dir_all(dir.join("theme").join("art"))
        .with_context(|| format!("creating {}", dir.display()))?;
    let text = format!(
        "Wrote {}: package.toml ({name}), theme/look.txt (id {id}), theme/art.txt ({slots} slots to draw), README.md and .gitattributes.\nNext: draw the pictures into theme/art/, add their rules, and check with `gp theme look check {} --game {game_arg_s}`.",
        dir.display(),
        dir.display()
    );
    Ok(Outcome::ok(
        json!({"dir": dir.display().to_string(), "name": name, "id": id, "slots": slots}),
        text,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme_geometry::{GeoBox, Geometry, Tolerance};

    /// A made-up picture: `w` by `h`, drawn (opaque) in the box x0, y0 to
    /// x1, y1 (past the last), clear elsewhere.
    pub(super) fn picture(w: u32, h: u32, ink: [u32; 4]) -> Rgba {
        let mut img = Rgba::new(w, h);
        for y in ink[1]..ink[3] {
            for x in ink[0]..ink[2] {
                img.set(x, y, [200, 80, 40, 255]);
            }
        }
        img
    }

    pub(super) fn put(dir: &Path, rel: &str, bytes: &[u8]) {
        let p = dir.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, bytes).unwrap();
    }

    const ENTITY: &str = "blob";

    /// A made-up game: a model with one body, a kit with its slot (two
    /// poses, a touch box, a feet row) and a words file denying `zorp`.
    fn game(dir: &Path) -> Game {
        game_kit(dir, Vec::new())
    }

    /// [`game`] with two sound roles in its kit: an effect `hop` and a
    /// looping tune `tune`.
    pub(super) fn game_with_sound(dir: &Path) -> Game {
        let role = |role: &str, kind: &str, loops: bool| crate::theme_kit::SoundSlot {
            role: role.into(),
            kind: kind.into(),
            cue: "made up".into(),
            frames: None,
            loops,
            description: "made up".into(),
        };
        game_kit(
            dir,
            vec![role("hop", "sfx", false), role("tune", "music", true)],
        )
    }

    fn game_kit(dir: &Path, sound: Vec<crate::theme_kit::SoundSlot>) -> Game {
        put(dir, "template.toml", b"[template]\nid = \"made-up\"\n");
        put(
            dir,
            "package.toml",
            b"[package]\nname = \"someone/made-up\"\ntitle = \"Made Up\"\n",
        );
        put(
            dir,
            "themes/kit/words.toml",
            b"format = \"0.1.0\"\ndeny = [\"zorp\"]\n",
        );
        let m = model();
        let id = crate::theme_kit::entity_id(ENTITY, "enemy", &crate::theme_kit::names_of(&m));
        let kit = Kit {
            format: "0.1.0".into(),
            game: "made-up".into(),
            model: "made_up".into(),
            screen: [256, 240],
            pixel_aspect: "1".into(),
            slots: String::new(),
            art: vec![ArtSlot {
                id,
                kind: "sprite".into(),
                role: "enemy".into(),
                size: [16, 16],
                frames: 2,
                frame_names: vec!["base".into(), "clip0.0".into()],
                timing: String::new(),
                colors: String::new(),
                anchor: String::new(),
                description: String::new(),
                label: "rolling thing".into(),
                poses: vec!["rolling".into(), "squashed".into()],
                geometry: Some(Geometry {
                    boxes: vec![GeoBox {
                        kind: "touch".into(),
                        what: "harms the hero where the hero's box meets it".into(),
                        rect: [4, 4, 8, 8],
                        frames: Vec::new(),
                    }],
                    danger: vec!["base".into(), "clip0.0".into()],
                    feet: Some(16),
                    min_size: [6, 6],
                    ..Geometry::default()
                }),
            }],
            sound,
            tolerance: Tolerance::default(),
            surfaces: Vec::new(),
        };
        put(
            dir,
            "themes/kit/kit.toml",
            toml::to_string(&kit).unwrap().as_bytes(),
        );
        game_of(dir).unwrap()
    }

    fn model() -> Model {
        Model::parse(&format!(
            "[model]\nschema = \"0.4\"\nname = \"made_up\"\ntitle = \"Made Up\"\nview = \"side\"\nframe_rate = \"60\"\nsystems = [\"rules\"]\n\n[entities.{ENTITY}]\nrole = \"enemy\"\nsize = [16, 16]\n\n[entities.{ENTITY}.hitbox]\nx = 0\ny = 0\nw = 16\nh = 16\n"
        ))
        .unwrap()
    }

    /// The game's build, made up: the body's two poses are pictures
    /// frame-0-0.png and frame-1-0.png, each filling its 16 by 16 picture
    /// down to the feet row.
    pub(super) fn built() -> Built {
        let mut art = gp_look::present::Art::default();
        for (i, f) in ["frame-0-0.png", "frame-1-0.png"].into_iter().enumerate() {
            let img = picture(16, 16, [2, 2, 14, 16]);
            art.present.pictures.push(gp_present::Picture {
                file: f.into(),
                size: [16, 16],
            });
            art.pictures.push(gp_look::present::Picture {
                file: f.into(),
                width: 16,
                height: 16,
                png: img.png(),
            });
            art.present.frames.push(vec![i as u32]);
        }
        let look = gp_present::Look {
            base: gp_present::Frame {
                id: 0,
                ..Default::default()
            },
            clips: vec![gp_present::Clip {
                frames: vec![gp_present::Frame {
                    id: 1,
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        art.present.kinds.push(vec![Some(look)]);
        Built {
            model: model(),
            art,
            kinds: vec![ENTITY.into()],
            initial: vec![0],
            board: None,
            board_art: None,
            _lifted: None,
        }
    }

    pub(super) fn theme(dir: &Path, package: &str, look: &str, art: &str) {
        put(dir, "package.toml", package.as_bytes());
        put(dir, "theme/look.txt", look.as_bytes());
        put(dir, "theme/art.txt", art.as_bytes());
        // Placed another way, each for its game picture.
        let names: Vec<&str> = art
            .lines()
            .filter_map(|l| l.trim().strip_prefix("pic "))
            .filter_map(|l| l.split_whitespace().next())
            .collect();
        let mut names = names;
        names.sort_unstable();
        names.dedup();
        put(
            dir,
            "theme/made-from.txt",
            format!("gp-made-from 1\nplaced {}\n", names.join(" ")).as_bytes(),
        );
    }

    pub(super) const GOOD_PACKAGE: &str = "[package]\nname = \"someone/bright\"\nversion = \"0.1.0\"\nkind = \"content\"\ntitle = \"Bright\"\nlicense = \"CC-BY-4.0\"\n";
    const GOOD_LOOK: &str = "gp-look 1\nid bright\ntitle Bright\nart art.txt\n";

    pub(super) fn levels(o: &Outcome, level: &str) -> Vec<String> {
        o.json["findings"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|f| f["level"] == level)
            .map(|f| {
                format!(
                    "{} {} {}",
                    f["file"].as_str().unwrap_or("-"),
                    f["line"],
                    f["message"].as_str().unwrap()
                )
            })
            .collect()
    }

    #[test]
    fn a_correct_theme_has_no_problems() {
        let g = tempfile::tempdir().unwrap();
        let game = game(g.path());
        let t = tempfile::tempdir().unwrap();
        theme(
            t.path(),
            GOOD_PACKAGE,
            GOOD_LOOK,
            "gp-art 1\nscale 4\npic frame-0-0.png art/roll.png 16 16 0 0\npic frame-1-0.png art/squash.png 16 16 0 0\n",
        );
        put(
            t.path(),
            "theme/art/roll.png",
            &picture(64, 64, [8, 8, 56, 64]).png(),
        );
        put(
            t.path(),
            "theme/art/squash.png",
            &picture(64, 64, [4, 16, 60, 64]).png(),
        );
        let o = report(t.path(), &game, Ok(built()), None).unwrap();
        assert_eq!(o.exit, 0, "{}", o.text);
        assert!(levels(&o, "problem").is_empty(), "{}", o.text);
        assert!(levels(&o, "warning").is_empty(), "{}", o.text);
        assert!(
            o.text
                .contains("1 of the kit's 1 art slots drawn in every pose"),
            "{}",
            o.text
        );
        assert!(o.text.ends_with("Next: play it in the game and look at every stage, every pose at game size, the deaths and the score display."), "{}", o.text);
    }

    /// The `#+` lines: read and drawn on the page, a page from before
    /// them offers the theme anyway, a line the page can't read is a
    /// problem (passed over), and a region must be one the game's level
    /// kit names (Moss Hop's names `start`).
    #[test]
    fn the_check_knows_the_added_lines_and_the_kits_regions() {
        let g = tempfile::tempdir().unwrap();
        let mut game = game(g.path());
        let t = tempfile::tempdir().unwrap();
        let art = "gp-art 1\nscale 4\npic frame-0-0.png art/roll.png 16 16 0 0\npic frame-1-0.png art/roll.png 16 16 0 0\n\
                   #+ hud_over\n#+ in top-ledge pic frame-0-0.png art/short.png 16 12 0 4\n";
        let check = |game: &Game, art: &str| {
            theme(t.path(), GOOD_PACKAGE, GOOD_LOOK, art);
            for f in ["roll", "short"] {
                put(
                    t.path(),
                    &format!("theme/art/{f}.png"),
                    &picture(64, 64, [8, 8, 56, 64]).png(),
                );
            }
            report(t.path(), game, Ok(built()), None).unwrap()
        };
        // No region in the kit: refused, by its line.
        let o = check(&game, art);
        let problems = levels(&o, "problem");
        assert_eq!(problems.len(), 1, "{}", o.text);
        assert!(
            problems[0].starts_with("theme/art.txt 6 names the region `top-ledge`, but Made Up has no level kit to name regions of its stage"),
            "{}",
            o.text
        );
        assert!(
            o.text
                .contains("uses `#+` lines, which a page built before them takes for notes"),
            "{}",
            o.text
        );
        // The game has no HUD to draw over the bodies: a warning.
        assert!(
            levels(&o, "warning")
                .iter()
                .any(|w| w.contains("so `#+ hud_over` changes nothing")),
            "{}",
            o.text
        );
        // A kit naming it: no problem. One naming others: refused, with
        // the names it has.
        game.regions = Some(vec!["top-ledge".into()]);
        let o = check(&game, art);
        assert!(levels(&o, "problem").is_empty(), "{}", o.text);
        game.regions = Some(vec!["bottom".into()]);
        let o = check(&game, art);
        assert!(
            levels(&o, "problem")[0].contains("which Made Up's level kit doesn't name, so the page never draws these lines. The kit names bottom"),
            "{}",
            o.text
        );
        // Moss Hop's kit, read as the check reads a game's: its region.
        let mosshop =
            game_of(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates/mosshop"))
                .unwrap();
        assert_eq!(mosshop.regions, Some(vec!["start".to_owned()]));
        // Lines the page can't read: each a problem, by line, and the rest
        // of the theme still read.
        game.regions = Some(vec!["top-ledge".into()]);
        let o = check(&game, &format!("{art}#+ hud_over x\n#+ sparkle 3\n"));
        let problems = levels(&o, "problem");
        assert_eq!(problems.len(), 2, "{}", o.text);
        assert!(
            problems[0].starts_with(
                "theme/art.txt 7 the page can't read this line (`#+ hud_over` takes no row"
            ),
            "{}",
            o.text
        );
        assert!(
            problems[1].starts_with(
                "theme/art.txt 8 the page can't read this line (no page knows `#+ sparkle`"
            ),
            "{}",
            o.text
        );
        // The page today reads every one of them (the old reader takes
        // them for notes, which the art module's own test shows).
        assert!(
            o.text
                .contains("read as the page reads them; 2 pictures named, all there"),
            "{}",
            o.text
        );
    }

    /// `#+ on SCREEN...`: a screen the game shows is read with no problem,
    /// one it hasn't got is refused by its line, and a line with no screen
    /// says what it takes.
    #[test]
    fn the_check_knows_an_on_screen_line_and_refuses_a_screen_the_game_hasnt_got() {
        let g = tempfile::tempdir().unwrap();
        let game = game(g.path());
        let t = tempfile::tempdir().unwrap();
        let art = "gp-art 1\nscale 4\npic frame-0-0.png art/roll.png 16 16 0 0\npic frame-1-0.png art/roll.png 16 16 0 0\n";
        let check = |art: &str| {
            theme(t.path(), GOOD_PACKAGE, GOOD_LOOK, art);
            for f in ["roll", "short"] {
                put(
                    t.path(),
                    &format!("theme/art/{f}.png"),
                    &picture(64, 64, [8, 8, 56, 64]).png(),
                );
            }
            report(t.path(), &game, Ok(built()), None).unwrap()
        };
        let o = check(&format!(
            "{art}#+ on frame-1-0.png pic frame-0-0.png art/short.png 16 12 0 4\n"
        ));
        assert!(levels(&o, "problem").is_empty(), "{}", o.text);
        assert!(
            o.text
                .contains("uses `#+` lines, which a page built before them takes for notes"),
            "{}",
            o.text
        );
        let o = check(&format!(
            "{art}#+ on screen_zz pic frame-0-0.png art/short.png 16 12 0 4\n"
        ));
        let problems = levels(&o, "problem");
        assert_eq!(problems.len(), 1, "{}", o.text);
        assert!(
            problems[0]
                .starts_with("theme/art.txt 5 names a picture this game doesn't have (screen_zz"),
            "{}",
            o.text
        );
        let o = check(&format!(
            "{art}#+ on pic frame-0-0.png art/short.png 16 12 0 4\n"
        ));
        let problems = levels(&o, "problem");
        assert_eq!(problems.len(), 1, "{}", o.text);
        assert!(
            problems[0].contains("(`#+ on` takes one or more of the game's screens"),
            "{}",
            o.text
        );
    }

    /// The check says whether the page would offer the theme, by the code
    /// the page runs: offered as its package name's last part; left out
    /// with why for a name the page can't list and for rules the page
    /// can't draw (a picture the theme lacks); a name the game's own theme
    /// has is a warning, since on the page it takes that theme's place.
    #[test]
    fn the_check_says_whether_the_page_would_offer_it() {
        let g = tempfile::tempdir().unwrap();
        let game = game(g.path());
        put(g.path(), "themes/paper/theme.toml", b"id = \"paper\"\n");
        let art = "gp-art 1\nscale 4\npic frame-0-0.png art/roll.png 16 16 0 0\npic frame-1-0.png art/roll.png 16 16 0 0\n";
        let offered = |package_name: &str, art: &str, with_picture: bool| {
            let t = tempfile::tempdir().unwrap();
            theme(
                t.path(),
                &GOOD_PACKAGE.replace("someone/bright", package_name),
                GOOD_LOOK,
                art,
            );
            if with_picture {
                put(
                    t.path(),
                    "theme/art/roll.png",
                    &picture(64, 64, [8, 8, 56, 64]).png(),
                );
            }
            let o = report(t.path(), &game, Ok(built()), None).unwrap();
            let lines: Vec<(String, String)> = o.json["findings"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|f| f["group"] == "offered")
                .map(|f| {
                    (
                        f["level"].as_str().unwrap().to_owned(),
                        f["message"].as_str().unwrap().to_owned(),
                    )
                })
                .collect();
            (o, lines)
        };
        let (o, lines) = offered("someone/bright", art, true);
        assert_eq!(lines.len(), 1, "{}", o.text);
        assert_eq!(lines[0].0, "ok");
        assert!(
            lines[0]
                .1
                .starts_with("the page would offer it in Settings, Theme for Made Up as `bright`"),
            "{}",
            o.text
        );
        assert!(
            o.text.contains("On the page (Settings, Theme)"),
            "{}",
            o.text
        );

        let (o, lines) = offered("someone/8bit", art, true);
        assert_eq!(o.exit, 1, "{}", o.text);
        assert_eq!(lines[0].0, "problem");
        assert!(
            lines[0].1.starts_with("the page would leave it out of Settings, Theme, because its name isn't one this page can list."),
            "{}",
            o.text
        );

        let (o, lines) = offered("someone/bright", art, false);
        assert_eq!(o.exit, 1, "{}", o.text);
        let left = lines.iter().find(|l| l.0 == "problem").unwrap();
        assert!(
            left.1.contains("because its files have a mistake in them")
                && left.1.contains("art/roll.png"),
            "{}",
            o.text
        );

        let (o, lines) = offered("someone/paper", art, true);
        assert!(
            lines
                .iter()
                .any(|l| l.0 == "warning" && l.1.contains("ships a theme in themes/paper")),
            "{}",
            o.text
        );
        assert!(lines.iter().any(|l| l.0 == "ok"), "{}", o.text);
    }

    #[test]
    fn each_kind_of_mistake_is_found_with_its_file_and_line() {
        let g = tempfile::tempdir().unwrap();
        let game = game(g.path());
        let t = tempfile::tempdir().unwrap();
        theme(
            t.path(),
            // A bad name, a paper kind, no licence.
            "[package]\nname = \"Some One/x\"\nversion = \"1\"\nkind = \"game\"\ntitle = \"Zorp Land\"\n",
            "gp-look 1\nid bright\ntitle Bright\nart art.txt\n",
            concat!(
                "gp-art 1\n",
                "scale 4\n",
                // Line 3: drawn too high, feet far above the row; stretched.
                "pic frame-0-0.png art/roll.png 16 16 0 0\n",
                // Line 4: a picture the theme hasn't got.
                "pic frame-0-0.png art/gone.png 16 16 0 0\n",
                // Line 5: a picture the game hasn't got.
                "pic frame-7-0.png art/roll.png 16 16 0 0\n",
                "stage 4 0 0 0 9 9 9\n",
                "level frame-1-0.png\n",
                // Line 8: a piece no stage_piece gives.
                "put nope 1 1 1 1\n",
                "# a note about zorp\n",
            ),
        );
        // Drawn in the top half only: feet 8 px above the row, the touch
        // box half bare. Its shape is 64 by 32, drawn 16 by 16.
        put(
            t.path(),
            "theme/art/roll.png",
            &picture(64, 32, [0, 0, 64, 8]).png(),
        );
        put(
            t.path(),
            "theme/art/unused.png",
            &picture(8, 8, [0, 0, 8, 8]).png(),
        );
        let o = report(t.path(), &game, Ok(built()), None).unwrap();
        assert_eq!(o.exit, 1);
        let problems = levels(&o, "problem").join("\n");
        for want in [
            "package.toml 2 name `Some One/x` isn't NAMESPACE/NAME",
            "package.toml 3 version `1` isn't three numbers",
            "package.toml 4 kind is `game`",
            "package.toml null has no license",
            "theme/art.txt 4 names the picture art/gone.png, which isn't in the theme folder",
            "theme/art.txt 5 names a picture this game doesn't have (frame-7-0.png, nearest frame-0-0.png)",
            "theme/art.txt 8 `put nope` names a piece",
            "package.toml null the title `Zorp Land` uses zorp",
        ] {
            assert!(problems.contains(want), "no `{want}` in:\n{problems}");
        }
        let warnings = levels(&o, "warning").join("\n");
        for want in [
            "pictures have more than one `pic` line",
            "the feet stand more than 1 px off the row",
            "art/roll.png is 64 by 32 but drawn 16 by 16",
            "no `stage_piece plank`",
        ] {
            assert!(warnings.contains(want), "no `{want}` in:\n{warnings}");
        }
        let notes = levels(&o, "note").join("\n");
        assert!(notes.contains("1 picture no rule uses"), "{notes}");
        assert!(notes.contains("theme/art/unused.png"), "{notes}");
        assert!(notes.contains("`zorp` 1 time"), "{notes}");
        // The page's reader refuses a line it can't read, by its number.
        let t2 = tempfile::tempdir().unwrap();
        theme(
            t2.path(),
            GOOD_PACKAGE,
            GOOD_LOOK,
            "gp-art 1\nscale 4\npic frame-0-0.png\n",
        );
        let o = report(t2.path(), &game, Ok(built()), None).unwrap();
        assert!(
            levels(&o, "problem")
                .iter()
                .any(|p| p.starts_with("theme/art.txt 3 the page can't read this line")),
            "{}",
            o.text
        );
    }

    /// A pose named by a picture the game hasn't got is said with the
    /// game's nearest name beside it, each on its own when one line names
    /// several (issue 23).
    #[test]
    fn an_unknown_pose_name_comes_with_the_nearest_the_game_has() {
        let g = tempfile::tempdir().unwrap();
        let game = game(g.path());
        let t = tempfile::tempdir().unwrap();
        theme(
            t.path(),
            GOOD_PACKAGE,
            GOOD_LOOK,
            concat!(
                "gp-art 1\n",
                "scale 4\n",
                "pic frame-0-0.png art/roll.png 16 16 0 0\n",
                // Line 4: a typo'd pose.
                "pic frame-1-O.png art/squash.png 16 16 0 0\n",
                // Line 5: two names on one line, one of them known.
                "held frame-0-0.png frame-00-0.png frame-1-0\n",
            ),
        );
        for f in ["theme/art/roll.png", "theme/art/squash.png"] {
            put(t.path(), f, &picture(64, 64, [8, 8, 56, 64]).png());
        }
        let o = report(t.path(), &game, Ok(built()), None).unwrap();
        let problems = levels(&o, "problem").join("\n");
        for want in [
            "theme/art.txt 4 names a picture this game doesn't have (frame-1-O.png, nearest frame-1-0.png)",
            "theme/art.txt 5 names pictures this game doesn't have (frame-00-0.png, nearest frame-0-0.png; frame-1-0, nearest frame-1-0.png)",
        ] {
            assert!(problems.contains(want), "no `{want}` in:\n{problems}");
        }
    }

    #[test]
    fn a_new_theme_checks_with_exactly_its_slots_still_to_draw() {
        let g = tempfile::tempdir().unwrap();
        let game = game(g.path());
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("bright");
        new(
            &dir,
            g.path(),
            "someone/bright",
            "Bright",
            "CC-BY-4.0",
            Path::new("/no-gp-home"),
        )
        .unwrap();
        for f in [
            "package.toml",
            "theme/look.txt",
            "theme/art.txt",
            "README.md",
            ".gitattributes",
        ] {
            assert!(dir.join(f).is_file(), "{f}");
        }
        let art = fs::read_to_string(dir.join("theme/art.txt")).unwrap();
        assert!(art.contains("# TO DRAW rolling thing"), "{art}");
        assert!(art.contains("feet on row 16"), "{art}");
        // A folder with files is refused, and a denied name.
        assert!(
            new(
                &dir,
                g.path(),
                "someone/other",
                "Other",
                "CC0-1.0",
                Path::new("/no-gp-home")
            )
            .is_err()
        );
        assert!(
            new(
                &t.path().join("z"),
                g.path(),
                "someone/zorp",
                "Z",
                "CC0-1.0",
                Path::new("/no-gp-home")
            )
            .is_err()
        );
        for b in [Ok(built()), Err("no ROM".to_owned())] {
            let o = report(&dir, &game, b, None).unwrap();
            let problems = levels(&o, "problem");
            assert_eq!(problems.len(), 1, "{}", o.text);
            assert!(
                problems[0].contains("still to draw: rolling thing"),
                "{}",
                o.text
            );
            assert!(
                problems[0].contains("rolling; squashed")
                    || problems[0].contains("rolling (frame-0-0.png); squashed (frame-1-0.png)"),
                "{}",
                o.text
            );
            assert!(
                o.text.contains("Next: draw the 1 slot still to draw"),
                "{}",
                o.text
            );
        }
    }

    /// `look new` gives a theme a name a registry takes, its look id the
    /// name's last part; a name that breaks the rule is refused in one
    /// sentence offering the fixed name. A copy made before with an `_`
    /// still checks.
    #[test]
    fn a_new_theme_gets_a_name_a_registry_takes() {
        let g = tempfile::tempdir().unwrap();
        game(g.path());
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("night");
        new(
            &dir,
            g.path(),
            "local/moss-night",
            "Night",
            "CC0-1.0",
            Path::new("/no-gp-home"),
        )
        .unwrap();
        let look = fs::read_to_string(dir.join("theme/look.txt")).unwrap();
        assert!(look.contains("\nid moss-night\n"), "{look}");
        let pkg = fs::read_to_string(dir.join("package.toml")).unwrap();
        assert!(pkg.contains("name = \"local/moss-night\""), "{pkg}");
        for (bad, fixed) in [
            ("local/moss_night", "local/moss-night"),
            ("Me/My_Theme", "me/my-theme"),
            ("someone/9lives", "someone/t-9lives"),
            ("solo", "local/solo"),
            ("a.b/c.d", "a-b/c-d"),
        ] {
            let e = new(
                &t.path().join("x"),
                g.path(),
                bad,
                "X",
                "CC0-1.0",
                Path::new("/no-gp-home"),
            )
            .err()
            .unwrap()
            .to_string();
            assert!(e.contains(&format!("use `{fixed}`")), "{bad}: {e}");
            assert!(!e.contains('\n'), "{e}");
            assert_eq!(fixed_theme_name(bad), fixed);
            assert!(new_theme_name_ok(fixed), "{fixed}");
        }
        assert!(valid_package_name("local/moss_night"));
    }

    #[test]
    fn a_game_with_no_kit_is_said_so() {
        let g = tempfile::tempdir().unwrap();
        put(g.path(), "template.toml", b"[template]\nid = \"made-up\"\n");
        let game = game_of(g.path()).unwrap();
        let t = tempfile::tempdir().unwrap();
        theme(t.path(), GOOD_PACKAGE, GOOD_LOOK, "gp-art 1\n");
        let o = report(t.path(), &game, Err("no ROM".into()), None).unwrap();
        assert_eq!(o.exit, 0, "{}", o.text);
        assert!(o.text.contains("has no theme kit yet"), "{}", o.text);
        assert!(o.text.contains("gp theme kit"), "{}", o.text);
    }

    fn problems_of(r: &Report) -> Vec<String> {
        r.found
            .iter()
            .filter(|f| f.level == Level::Problem)
            .map(|f| f.text.clone())
            .collect()
    }

    /// A theme whose `pic` lines don't say where their pictures came
    /// from is refused; `placed` lines (or `hand`) satisfy it; a cell is held to the
    /// game pictures sheets.json says it stands for.
    #[test]
    fn each_pic_line_says_which_cell_or_placing_it_came_from() {
        let t = tempfile::tempdir().unwrap();
        let art = "gp-art 1\nscale 4\npic frame-0-0.png art/a.png 16 16 0 0\npic frame-1-0.png art/b.png 16 16 0 0\n";
        put(t.path(), "theme/art.txt", art.as_bytes());
        let rules = ArtRules::read(art).unwrap();
        let folder = Folder::of(t.path());
        let check = |made: Option<&str>| {
            let f = t
                .path()
                .join("theme")
                .join(crate::theme_fit_large::MADE_FROM);
            match made {
                Some(m) => fs::write(&f, m).unwrap(),
                None => {
                    let _ = fs::remove_file(&f);
                }
            }
            let mut r = Report::default();
            made_from(&folder, &rules, art, "theme/art.txt", None, &mut r);
            r
        };
        let r = check(None);
        assert!(
            problems_of(&r)[0].contains("nothing says where their pictures came from"),
            "{:?}",
            problems_of(&r)
        );
        let r = check(Some("gp-made-from 1\nplaced frame-0-0.png frame-1-0.png\n"));
        assert!(problems_of(&r).is_empty(), "{:?}", problems_of(&r));
        let r = check(Some("gp-made-from 1\nhand frame-0-0.png\n"));
        assert!(
            problems_of(&r)[0].contains("frame-1-0.png"),
            "{:?}",
            problems_of(&r)
        );
        // Cells, held to sheets.json: B02 stands for frame-1-0.png only.
        let w = tempfile::tempdir().unwrap();
        let cell = |c: &str, pic: &str| {
            serde_json::json!({"cell": c, "slot": "entity.blob", "label": "blob", "frames": [0], "frame_names": ["base"],
                "rect": [0, 0, 10, 10], "body": [0, 0, 10, 10], "size": [16, 16], "anchor": "", "baseline": 10,
                "pose": "rolling", "pictures": [[pic]]})
        };
        let m = serde_json::json!({"format": crate::theme_sheet::SHEETS_FORMAT, "game": "made-up", "style": null, "key": "#FF00FF",
            "refs": [], "pictures": [], "code": [],
            "sheets": [{"id": "sheet-01", "image": "sheet-01.png", "size": [1024, 1024], "scale": 4, "model": "gpt_image_2",
                "prompts": {}, "out": "sheet-01.out.png", "refs": [], "cells": [cell("B01", "frame-0-0.png"), cell("B02", "frame-1-0.png")]}]});
        let sj = w.path().join(crate::theme_sheet::SHEETS_FILE);
        fs::write(&sj, m.to_string()).unwrap();
        let good = format!(
            "gp-made-from 1\nsheets {}\ncell sheet-01 B01 frame-0-0.png\ncell sheet-01 B02 frame-1-0.png\n",
            sj.display()
        );
        let r = check(Some(&good));
        assert!(problems_of(&r).is_empty(), "{:?}", problems_of(&r));
        // Paired in another order: each picture shows another frame's pose.
        let swapped = format!(
            "gp-made-from 1\nsheets {}\ncell sheet-01 B02 frame-0-0.png\ncell sheet-01 B01 frame-1-0.png\n",
            sj.display()
        );
        let r = check(Some(&swapped));
        assert!(
            problems_of(&r)[0].contains("doesn't stand for them"),
            "{:?}",
            problems_of(&r)
        );
        // Sheets elsewhere: a note, not a pass or a problem.
        let r = check(Some(
            "gp-made-from 1\nsheets $GP_HOME/none/sheets.json\ncell sheet-01 B01 frame-0-0.png frame-1-0.png\n",
        ));
        assert!(problems_of(&r).is_empty());
        assert!(
            r.found
                .iter()
                .any(|f| f.level == Level::Note && f.text.contains("isn't on this machine"))
        );
    }

    /// A pose drawn off the original's body in its frame, beyond the
    /// kit's tolerance, is a problem.
    #[test]
    fn a_pose_off_the_originals_bottom_is_refused_and_its_shape_warned() {
        let t = tempfile::tempdir().unwrap();
        // The original's body in frame-0-0.png is 2..14 by 2..16; this one
        // is 0..16 by 0..10: its middle 4 px high, its bottom 6 px high.
        let art = "gp-art 1\nscale 4\npic frame-0-0.png art/a.png 16 16 0 0\npic frame-1-0.png art/b.png 16 16 0 0\n";
        let rules = ArtRules::read(art).unwrap();
        let images: BTreeMap<String, Rgba> = [
            ("art/a.png".to_owned(), picture(64, 64, [0, 0, 64, 40])),
            ("art/b.png".to_owned(), picture(64, 64, [8, 8, 56, 64])),
        ]
        .into();
        let mut r = Report::default();
        body_check(
            &Tolerance::default(),
            &rules,
            art,
            "theme/art.txt",
            &images,
            &built(),
            None,
            &mut r,
        );
        let p = problems_of(&r);
        assert_eq!(p.len(), 1, "{p:?}");
        assert!(
            p[0].contains("1 of 2 poses") && p[0].contains("frame-0-0.png (line 3)"),
            "{p:?}"
        );
        assert!(p[0].contains("its bottom 6.0 px high"), "{p:?}");
        // Narrower and off its middle but standing on the original's
        // bottom: large art draws its own figure, so only a warning.
        let art2 = "gp-art 1\nscale 4\npic frame-0-0.png art/c.png 16 16 0 0\n";
        let rules2 = ArtRules::read(art2).unwrap();
        let images2: BTreeMap<String, Rgba> =
            [("art/c.png".to_owned(), picture(64, 64, [0, 16, 24, 64]))].into();
        let mut r2 = Report::default();
        body_check(
            &Tolerance::default(),
            &rules2,
            art2,
            "theme/art.txt",
            &images2,
            &built(),
            None,
            &mut r2,
        );
        assert!(problems_of(&r2).is_empty(), "{:?}", problems_of(&r2));
        assert!(
            r2.found
                .iter()
                .any(|f| f.level == Level::Warning && f.text.contains("1 of 1 poses differ")),
            "{:?}",
            r2.found
        );
        let _ = t;
    }

    /// An arcade game's screens and the names its plays showed, with no
    /// `pic` line, are problems.
    #[test]
    fn a_board_picture_the_game_shows_with_nothing_drawn_is_refused() {
        let k = crate::theme_board::BoardKit {
            names: vec![
                ("stage".into(), [512, 480]),
                ("stage2".into(), [512, 480]),
                ("sprite-cab-4d".into(), [32, 32]),
                ("sprite-cab-4e".into(), [32, 32]),
            ],
            present: gp_present::Present::default(),
            seen: Some(
                [
                    ("stage".to_owned(), 9),
                    ("sprite-cab-4d".to_owned(), 3),
                    ("sprite-cab-4e".to_owned(), 1),
                ]
                .into(),
            ),
            sheet: PathBuf::from("/made/up/board"),
        };
        let rules =
            ArtRules::read("gp-art 1\nscale 2\npic sprite-cab-4d art/a.png 32 32 0 0\n").unwrap();
        let mut r = Report::default();
        board_drawn(&rules, "theme/art.txt", &k, &mut r);
        let p = problems_of(&r);
        assert_eq!(p.len(), 2, "{p:?}");
        // The screen the plays showed; the one they didn't isn't asked for.
        assert!(
            p[0].contains("1 screen with nothing drawn for it") && p[0].contains("stage."),
            "{p:?}"
        );
        assert!(!p[0].contains("stage2"), "{p:?}");
        assert!(
            p[1].contains("sprite-cab-4e") && !p[1].contains("sprite-cab-4d"),
            "{p:?}"
        );
        // A screen a `#+ room` builds from pieces is drawn.
        let rules = ArtRules::read(
            "gp-art 1\nscale 2\npic sprite-cab-4d art/a.png 32 32 0 0\nstage 2 0 0 0 0 0 0\nstage_piece p art/p.png 16 16\n#+ room stage\n#+ room_cells all\n#+ room_fill all p\n",
        )
        .unwrap();
        assert_eq!(rules.rooms.len(), 1);
        let mut r = Report::default();
        board_drawn(&rules, "theme/art.txt", &k, &mut r);
        let p = problems_of(&r);
        assert!(
            p.iter().all(|t| !t.contains("screen with nothing drawn")),
            "{p:?}"
        );
    }

    /// The check reads a `#+ under` line and a `#+ room_react_stagger`
    /// line; it names an `under` piece no `stage_piece` line gives and an
    /// entity the game draws no body of; and of a line of either it can't
    /// read it says what the line takes.
    #[test]
    fn the_check_knows_the_under_and_stagger_lines_and_names_a_piece_the_look_hasnt_got() {
        let head = "gp-art 1\nscale 2\nstage 2 0 0 0 0 0 0\nstage_piece disc art/disc.png 48 16\nstage_piece lamp art/lamp.png 16 16\n#+ room rooftop\n#+ room_cells all\n#+ room_react all 0 0 lamp:4 disc:4\n";
        let read = |line: &str| ArtRules::read(&format!("{head}{line}\n")).unwrap();
        let have = [
            "rooftop",
            "sprite-cab-4d",
            "sprite-cab-4e",
            "sprite-tall-cab-1",
        ];
        let rules = read("#+ room_react_stagger 3\n#+ under cab 0 -2 disc:4 lamp:2");
        assert!(rules.skipped.is_empty(), "{:?}", rules.skipped);
        assert_eq!(rules.under.len(), 1);
        assert!(under_problems(&rules, &have).is_empty());
        let rules = read("#+ on rooftop under tall-cab 0 0 disc:1");
        assert!(rules.skipped.is_empty(), "{:?}", rules.skipped);
        assert!(under_problems(&rules, &have).is_empty());
        // A piece the look hasn't got, named once, by its line.
        let p = under_problems(&read("#+ under cab 0 0 disc:2 halo:2 halo:1"), &have);
        assert_eq!(p.len(), 1, "{p:?}");
        assert!(
            p[0].0 == 9 && p[0].1.starts_with("names halo no `stage_piece` line gives"),
            "{p:?}"
        );
        // An entity the game draws no body of, with those it does.
        let p = under_problems(&read("#+ under van 0 0 disc:2"), &have);
        assert_eq!(p.len(), 1, "{p:?}");
        assert!(
            p[0].0 == 9
                && p[0]
                    .1
                    .starts_with("names the entity `van`, which this game draws no body of")
                && p[0].1.ends_with("Its entities are cab, tall-cab"),
            "{p:?}"
        );
        // Lines the page can't read: the report says what each takes.
        let g = tempfile::tempdir().unwrap();
        let game = game(g.path());
        let t = tempfile::tempdir().unwrap();
        let art = "gp-art 1\nscale 4\npic frame-0-0.png art/roll.png 16 16 0 0\npic frame-1-0.png art/roll.png 16 16 0 0\n\
                   #+ under cab 0 0\n#+ room_react_stagger 0\n#+ room_react_stagger 2 cols\n";
        theme(t.path(), GOOD_PACKAGE, GOOD_LOOK, art);
        put(
            t.path(),
            "theme/art/roll.png",
            &picture(64, 64, [8, 8, 56, 64]).png(),
        );
        let o = report(t.path(), &game, Ok(built()), None).unwrap();
        let problems = levels(&o, "problem");
        assert_eq!(problems.len(), 3, "{}", o.text);
        assert!(
            problems[0].starts_with(
                "theme/art.txt 5 the page can't read this line (`#+ under` comes after the `stage` line and takes an entity"
            ),
            "{}",
            o.text
        );
        assert!(
            problems[1].starts_with(
                "theme/art.txt 6 the page can't read this line (`#+ room_react_stagger` comes once after a `#+ room_react` and takes N"
            ),
            "{}",
            o.text
        );
        assert!(
            problems[2].starts_with(
                "theme/art.txt 7 the page can't read this line (`#+ room_react_stagger`"
            ) && problems[2].contains("then nothing or the word `rows`"),
            "{}",
            o.text
        );
    }

    /// The check reads a `#+ room_scroll` line, and says so of one whose
    /// piece the look hasn't got or its room never lays; a line it can't
    /// read is passed over as any `#+` line is.
    #[test]
    fn the_check_knows_a_scroll_line_and_names_a_piece_the_look_hasnt_got() {
        let head = "gp-art 1\nscale 2\nstage 2 0 0 0 0 0 0\nstage_piece clouds art/clouds.png 64 32\nstage_piece hilltop art/hilltop.png 16 16\n#+ room rooftop\n#+ room_cells all\n#+ room_fill all clouds\n";
        let read = |line: &str| ArtRules::read(&format!("{head}{line}\n")).unwrap();
        let rules = read("#+ room_scroll clouds 1 4");
        assert!(rules.skipped.is_empty(), "{:?}", rules.skipped);
        assert_eq!(rules.rooms[0].scrolls.len(), 1);
        assert!(scroll_problems(&rules).is_empty());
        let p = scroll_problems(&read("#+ room_scroll airship -1 2"));
        assert_eq!(p.len(), 1);
        assert!(
            p[0].0 == 9
                && p[0]
                    .1
                    .starts_with("names airship no `stage_piece` line gives"),
            "{p:?}"
        );
        let p = scroll_problems(&read("#+ room_scroll hilltop -1 2"));
        assert!(
            p.len() == 1
                && p[0]
                    .1
                    .starts_with("slides hilltop, which no line of its room lays"),
            "{p:?}"
        );
        // No frames to a slide: passed over, and nothing slides.
        let rules = read("#+ room_scroll clouds 1 0");
        assert_eq!(rules.skipped.len(), 1, "{:?}", rules.skipped);
        assert!(rules.rooms[0].scrolls.is_empty());
    }
}
