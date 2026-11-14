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

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use gp_evidence::Paths;
use gp_model::Model;
use gp_present::art::{ArtRules, Sized};
use gp_present::looks::Look;
use gp_theme::pix::Rgba;
use serde_json::{Value, json};

use crate::Outcome;
use crate::theme_kit::{ArtSlot, Kit};

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
}

pub(crate) fn run(cmd: &LookCmd, paths: &Paths) -> Result<Outcome> {
    match cmd {
        LookCmd::Check { dir, game, rom } => check(dir, game, rom.as_deref(), paths),
        LookCmd::New {
            dir,
            game,
            name,
            title,
            license,
        } => new(dir, game, name, title, license),
    }
}

// ---------------------------------------------------------------------
// Findings.

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Group {
    Files,
    Kit,
    Fit,
    Words,
    Rom,
}

impl Group {
    fn title(self) -> &'static str {
        match self {
            Group::Files => "Files",
            Group::Kit => "What the game needs drawn",
            Group::Fit => "Fit (a first pass from the files alone: look at it in the game too)",
            Group::Words => "Names",
            Group::Rom => "ROM data",
        }
    }
    fn key(self) -> &'static str {
        match self {
            Group::Files => "files",
            Group::Kit => "kit",
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
            !l.starts_with('#') && l.split_whitespace().any(|w| w == word)
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
    /// Its package name (lex00/donkey-kong), for the page's address.
    package: Option<String>,
    kit: Option<Kit>,
    deny: Vec<String>,
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
    Ok(Game {
        dir,
        title,
        package: field("name"),
        kit,
        deny,
    })
}

/// The game's pictures as the page builds them from the art lifted from
/// the person's ROM: the same model, spec and presentation.
struct Built {
    model: Model,
    art: gp_look::present::Art,
    /// The spec's kind names, in the order the presentation keeps them.
    kinds: Vec<String>,
    _lifted: Option<tempfile::TempDir>,
}

impl Built {
    fn names(&self) -> BTreeSet<&str> {
        self.art
            .present
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
    let mut kinds = Vec::new();
    for k in &spec.kinds {
        kinds.push(k.name.clone());
    }
    Ok(Built {
        model,
        art,
        kinds,
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
        let looks: Vec<&gp_present::Look> = p.kinds.get(k)?.iter().flatten().collect();
        let first = looks.first()?;
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
    report(dir, &game, built, Some((rom, paths)))
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
        if let Some(st) = &rules.stage {
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
    }

    // Coverage against the kit, then the fit.
    let ruled = rules.as_ref().map(ruled_names).unwrap_or_default();
    match &game.kit {
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

/// The game's pictures a rule draws: `pic` and `timed` pictures.
fn ruled_names(rules: &ArtRules) -> BTreeSet<String> {
    let mut s: BTreeSet<String> = rules.pics.iter().map(|p| p.build.clone()).collect();
    s.extend(rules.timed.iter().map(|t| t.build.clone()));
    if let Some(st) = &rules.stage {
        s.extend(st.levels.iter().map(|l| l.level.clone()));
    }
    s
}

/// Every name of the game's pictures or levels the rules give, against
/// the game's build: one it hasn't got, the page passes over. And the
/// page's own resolution of the rules.
fn names_check(rules: &ArtRules, text: &str, art_rel: &str, b: &Built, r: &mut Report) {
    let have = b.names();
    let mut named: Vec<&str> = Vec::new();
    named.extend(rules.pics.iter().map(|p| p.build.as_str()));
    for t in &rules.timed {
        named.push(&t.build);
        if let Some(a) = &t.after {
            named.push(a);
        }
    }
    named.extend(rules.held.iter().map(String::as_str));
    named.extend(rules.face.iter().map(String::as_str));
    if let Some(n) = &rules.near {
        named.extend(n.when.iter().map(String::as_str));
        named.extend(n.other.iter().map(String::as_str));
    }
    for e in &rules.elsewhere {
        named.push(&e.level);
        named.extend(e.builds.iter().map(String::as_str));
    }
    for p in &rules.props {
        named.push(&p.level);
        named.extend(p.over.iter().map(String::as_str));
        named.extend(p.burst.iter().map(String::as_str));
    }
    if let Some(d) = &rules.drift {
        named.push(&d.level);
    }
    if let Some(st) = &rules.stage {
        named.extend(st.levels.iter().map(|l| l.level.as_str()));
    }
    let mut unknown: BTreeMap<usize, Vec<&str>> = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for n in named {
        if !have.contains(n) && seen.insert(n) {
            unknown
                .entry(line_of(text, n).unwrap_or(0))
                .or_default()
                .push(n);
        }
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
        r.add(
            Group::Kit,
            Level::Problem,
            Some(art_rel),
            (*line > 0).then_some(*line),
            format!(
                "names {} this game doesn't have ({}), and the page passes over {}. Use the game's own picture names (frame-N-K.png, glyph-..., level-NAME.png, as the lines for the kit's slots give them)",
                if names.len() == 1 { "a picture" } else { "pictures" },
                said.join("; "),
                if names.len() == 1 { "it" } else { "them" },
            ),
        );
    }
    if let Err(e) = gp_present::art::Art::new(rules.clone(), &b.art.present) {
        r.add(Group::Kit, Level::Problem, Some(art_rel), None, format!("the page refuses these rules for this game: {e}. Check the picture names against the game's"));
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
    if !kit.sound.is_empty() {
        r.add(
            Group::Kit,
            Level::Note,
            None,
            None,
            format!("a large-art theme has no sound lines yet, so the game keeps its own sound ({} sound roles in the kit)", kit.sound.len()),
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

/// The road to see a theme in the game today, in commands.
fn play_road(folder: &Folder, game: &Game, name: Option<&str>) -> Vec<String> {
    let gp_games = game
        .dir
        .parent()
        .map_or_else(|| ".".into(), |p| p.display().to_string());
    let pkg = game.package.as_deref().unwrap_or("NAMESPACE/GAME");
    vec![
        "in the gameprinter checkout, with your ROM in $GP_ROM_DIR:".into(),
        format!(
            "  scripts/web/local-registry.sh {gp_games} {}",
            folder.root.display()
        ),
        "  target/debug/gp registry serve \"$GP_HOME/registry\" --page target/page-local".into(),
        format!(
            "  then open http://127.0.0.1:8377/p?g={pkg}&c={}",
            name.unwrap_or("NAMESPACE/NAME")
        ),
    ]
}

fn finish(folder: &Folder, game: &Game, pkg: Option<&Pkg>, r: Report) -> Outcome {
    let problems = r.count(Level::Problem);
    let warnings = r.count(Level::Warning);
    let name = pkg.and_then(|p| p.name.as_deref());
    let road = play_road(folder, game, name);
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

fn new(dir: &Path, game_arg: &Path, name: &str, title: &str, license: &str) -> Result<Outcome> {
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
    if !valid_package_name(name) {
        bail!(
            "`{name}` isn't NAMESPACE/NAME in lowercase letters, digits, - and _ (like lex00/turnips): your GitHub name, a slash and the theme's"
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
    let short = name.split_once('/').map_or(name, |x| x.1);
    let id: String = short
        .chars()
        .map(|c| if c == '_' { '-' } else { c })
        .collect();
    let id = if gp_present::looks::valid_id(&id) {
        id
    } else {
        format!("t-{id}")
    };
    let folder = Folder {
        root: dir.to_owned(),
        theme: dir.join("theme"),
    };
    let scale = 6u32;
    let mut art = format!(
        "gp-art 1\n# {title}: the rules for a large-art theme of {} (site docs, themes.md, lists every rule).\n# Each picture the game's theme kit lists is below, marked TO DRAW. Draw it into art/,\n# then write its `pic` lines (`gp theme look check` names the game's pictures for each pose).\nscale {scale}\nlimit off\n",
        game.title
    );
    let mut slots = 0;
    match &game.kit {
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
    let road = play_road(&folder, &game, Some(name));
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
    fn picture(w: u32, h: u32, ink: [u32; 4]) -> Rgba {
        let mut img = Rgba::new(w, h);
        for y in ink[1]..ink[3] {
            for x in ink[0]..ink[2] {
                img.set(x, y, [200, 80, 40, 255]);
            }
        }
        img
    }

    fn put(dir: &Path, rel: &str, bytes: &[u8]) {
        let p = dir.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, bytes).unwrap();
    }

    const ENTITY: &str = "blob";

    /// A made-up game: a model with one body, a kit with its slot (two
    /// poses, a touch box, a feet row) and a words file denying `zorp`.
    fn game(dir: &Path) -> Game {
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
            sound: Vec::new(),
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
    fn built() -> Built {
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
            _lifted: None,
        }
    }

    fn theme(dir: &Path, package: &str, look: &str, art: &str) {
        put(dir, "package.toml", package.as_bytes());
        put(dir, "theme/look.txt", look.as_bytes());
        put(dir, "theme/art.txt", art.as_bytes());
    }

    const GOOD_PACKAGE: &str = "[package]\nname = \"someone/bright\"\nversion = \"0.1.0\"\nkind = \"content\"\ntitle = \"Bright\"\nlicense = \"CC-BY-4.0\"\n";
    const GOOD_LOOK: &str = "gp-look 1\nid bright\ntitle Bright\nart art.txt\n";

    fn levels(o: &Outcome, level: &str) -> Vec<String> {
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
        new(&dir, g.path(), "someone/bright", "Bright", "CC-BY-4.0").unwrap();
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
        assert!(new(&dir, g.path(), "someone/other", "Other", "CC0-1.0").is_err());
        assert!(
            new(
                &t.path().join("z"),
                g.path(),
                "someone/zorp",
                "Z",
                "CC0-1.0"
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
}
