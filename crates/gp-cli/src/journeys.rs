//! The person's copies, and the two journeys besides porting a game: making
//! a theme for a game, and making a new game from a template (issue #5;
//! docs/progress.md, "Copies and the other journeys").
//!
//! A copy is one of these, found where the tools make them:
//!
//! | Copy | Where | Made by |
//! |---|---|---|
//! | A theme | `$GP_HOME/work/themes/NAME/`, holding `theme/look.txt` (or `look.txt`) | `gp theme look new`, from the page's MAKE A THEME or the agent |
//! | A new game | `$GP_HOME/work/mcp/NAME/`, holding `model.toml` | `template_fork`, from the page's MAKE A NEW GAME or the agent |
//!
//! What the journeys keep of their own is under `$GP_HOME/work/journeys/
//! KIND/NAME/` (KIND `theme` or `new`), never in the copy:
//!
//! | File | What | Written by |
//! |---|---|---|
//! | `journey.json` | `{journey, name, from, from_kind, style?, made_ms, steps_done}`: the game or template it came from, the person's style words, and the steps the person said are done | The page, when it starts a copy or the person presses a step's DONE |
//! | `check.json` | `{at_ms, ok, problems, warnings, first, all_to_draw}`: the last `gp theme look check` of a theme copy | `gp theme look check` (and `play`, which checks first) |
//! | `play.json` | `{at_ms, links, pid, minutes}` or `{at_ms, shots}`: the theme last seen in the game | `gp theme look play` |
//! | `verify.json` | `{at_ms, ok, said}`: the last `gp level verify` of a new game's stage | `gp level verify` |
//! | `do-WHICH.json` | A page button's last run (`do-verify.json` for CHECK THE STAGE), as serve/jobs.rs keeps a game's | gp serve |
//!
//! The answer `next` gives for a copy is [`answer`]'s, from [`gather`]'s
//! facts alone, as a port's is crate::next's: it reads files and runs
//! nothing. The stages are the steps the page shows, T1 to T6 and N1 to N6.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use anyhow::{Result, bail};
use serde::Serialize;
use serde_json::{Value, json};

use crate::next::{Act, Answer, Running, StageId, Step, VERSION, Who, Yes, step};

/// The public page that plays a published game and its published themes
/// (docs/web.md: `/p?g=NS/NAME&c=NS/THEME`).
pub(crate) const PUBLIC_PAGE: &str = "https://nes-donkey-kong.intentius.workers.dev";
/// Where a theme is asked to be published today: an issue on the registry
/// the public page reads (site docs, themes.md, "Get it in the game").
pub(crate) const REGISTRY_ISSUES: &str = "https://github.com/lex00/gp-registry/issues/new";

const PICTURE_EXTS: &[&str] = &["png", "jpg", "jpeg", "webp", "gif"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Kind {
    Theme,
    New,
}

impl Kind {
    pub(crate) fn word(self) -> &'static str {
        match self {
            Kind::Theme => "theme",
            Kind::New => "new",
        }
    }
    pub(crate) fn parse(s: &str) -> Option<Kind> {
        match s {
            "theme" => Some(Kind::Theme),
            "new" => Some(Kind::New),
            _ => None,
        }
    }
}

/// What a copy came from: a game of the games checkout, a person's new
/// game (for a theme of it), or a template (for a new game).
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub(crate) struct Origin {
    pub name: String,
    pub title: String,
    /// `game` (the games checkout), `new` (a person's new game) or
    /// `template`.
    pub kind: &'static str,
    #[serde(skip)]
    pub dir: Option<PathBuf>,
}

impl Origin {
    /// The game as `--game` takes it: a game's name, else its folder.
    pub(crate) fn game_arg(&self) -> String {
        match (&self.dir, self.kind) {
            (Some(d), "new") => d.display().to_string(),
            _ => self.name.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Copy {
    pub kind: Kind,
    pub name: String,
    pub dir: PathBuf,
    pub from: Option<Origin>,
    /// The newest file in the copy, in ms since 1970.
    pub touched_ms: u64,
    pub record: Value,
}

/// A name a copy or a route may take: no separators, not hidden.
pub(crate) fn is_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && !s.starts_with('.')
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

pub(crate) fn themes_root(home: &Path) -> PathBuf {
    home.join("work").join("themes")
}

pub(crate) fn copies_root(home: &Path) -> PathBuf {
    home.join("work").join("mcp")
}

pub(crate) fn record_dir(home: &Path, kind: Kind, name: &str) -> PathBuf {
    home.join("work")
        .join("journeys")
        .join(kind.word())
        .join(name)
}

fn read_json(p: &Path) -> Value {
    fs::read(p)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Value::Null)
}

/// A file of the copy's journey folder (`journey.json`, `check.json` ...),
/// or null.
pub(crate) fn read(home: &Path, kind: Kind, name: &str, file: &str) -> Value {
    read_json(&record_dir(home, kind, name).join(file))
}

/// Writes a file of the copy's journey folder, whole.
pub(crate) fn write(home: &Path, kind: Kind, name: &str, file: &str, v: &Value) -> Result<()> {
    let d = record_dir(home, kind, name);
    fs::create_dir_all(&d)?;
    let tmp = d.join(format!(".{file}.tmp{}", std::process::id()));
    fs::write(&tmp, serde_json::to_vec_pretty(v)?)?;
    fs::rename(&tmp, d.join(file))?;
    Ok(())
}

pub(crate) fn now_ms() -> u64 {
    gp_mcp::asks::now_ms()
}

fn mtime_ms(p: &Path) -> u64 {
    fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as u64)
}

/// The newest file under a folder (a few levels down, no dot files, at
/// most a few thousand looked at), in ms.
pub(crate) fn newest(dir: &Path) -> u64 {
    fn walk(d: &Path, depth: u8, seen: &mut u32, best: &mut u64) {
        for e in fs::read_dir(d).into_iter().flatten().flatten() {
            if *seen > 4000 {
                return;
            }
            *seen += 1;
            let name = e.file_name();
            if name.to_string_lossy().starts_with('.') {
                continue;
            }
            let p = e.path();
            if p.is_dir() {
                if depth < 4 && name != "target" && name != "previews" {
                    walk(&p, depth + 1, seen, best);
                }
            } else {
                *best = (*best).max(mtime_ms(&p));
            }
        }
    }
    let mut best = 0;
    walk(dir, 0, &mut 0, &mut best);
    best
}

/// A theme folder's theme/ folder (where look.txt is), if it is one.
pub(crate) fn look_dir(dir: &Path) -> Option<PathBuf> {
    let look = gp_present::looks::LOOK_FILE;
    if dir.join("theme").join(look).is_file() {
        Some(dir.join("theme"))
    } else if dir.join(look).is_file() {
        Some(dir.to_owned())
    } else {
        None
    }
}

/// A template.toml's `[template] key`.
fn manifest_str(dir: &Path, table: &str, key: &str) -> Option<String> {
    let t: toml::Table = fs::read_to_string(dir.join("template.toml"))
        .ok()?
        .parse()
        .ok()?;
    t.get(table)?.get(key)?.as_str().map(str::to_owned)
}

/// A game's title: its template.toml's, else its folder's name.
pub(crate) fn title_of(dir: &Path) -> String {
    manifest_str(dir, "template", "title").unwrap_or_else(|| {
        dir.file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
    })
}

/// The games of the games checkout, by name, with their folders.
pub(crate) fn games(games: Option<&Path>) -> Vec<(String, PathBuf)> {
    let Some(g) = crate::roms_cmd::games_dir(games) else {
        return Vec::new();
    };
    let mut out: Vec<(String, PathBuf)> = fs::read_dir(&g)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("package.toml").is_file() || p.join(gp_model::MODEL_FILE).is_file())
        .filter_map(|p| Some((p.file_name()?.to_str()?.to_owned(), p)))
        .filter(|(n, _)| !n.starts_with('.'))
        .collect();
    out.sort();
    out
}

/// Where a theme came from: what the journey record says, else the game
/// its package.toml's first line names (`gp theme look new` writes "# NAME:
/// TITLE, a large-art theme for GAME.").
fn theme_from(home: &Path, gs: &[(String, PathBuf)], dir: &Path, record: &Value) -> Option<Origin> {
    let new_copy = |n: &str| {
        let d = copies_root(home).join(n);
        d.join(gp_model::MODEL_FILE).is_file().then(|| Origin {
            name: n.to_owned(),
            title: title_of(&d),
            kind: "new",
            dir: Some(d),
        })
    };
    let game = |n: &str| {
        gs.iter().find(|(g, _)| g == n).map(|(g, d)| Origin {
            name: g.clone(),
            title: title_of(d),
            kind: "game",
            dir: Some(d.clone()),
        })
    };
    if let Some(n) = record["from"].as_str() {
        return if record["from_kind"] == json!("new") {
            new_copy(n)
        } else {
            game(n)
        }
        .or_else(|| {
            Some(Origin {
                name: n.to_owned(),
                title: n.to_owned(),
                kind: "game",
                dir: None,
            })
        });
    }
    let first = fs::read_to_string(dir.join("package.toml")).ok()?;
    let line = first.lines().next()?;
    let t = line
        .split(", a large-art theme for ")
        .nth(1)?
        .trim_end_matches('.');
    gs.iter()
        .find(|(_, d)| title_of(d) == t)
        .and_then(|(g, _)| game(g))
}

/// Where a new game came from: the journey record, else its template.toml.
fn new_from(dir: &Path, record: &Value) -> Option<Origin> {
    let name = record["from"]
        .as_str()
        .map(str::to_owned)
        .or_else(|| manifest_str(dir, "template", "id"))?;
    Some(Origin {
        title: manifest_str(dir, "template", "title").unwrap_or_else(|| name.clone()),
        name,
        kind: "template",
        dir: None,
    })
}

/// Every copy the person has, the newest touched first.
pub(crate) fn list(home: &Path, games_dir: Option<&Path>) -> Vec<Copy> {
    let gs = games(games_dir);
    let mut out = Vec::new();
    let names = |root: PathBuf| -> Vec<(String, PathBuf)> {
        fs::read_dir(root)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| Some((e.file_name().to_str()?.to_owned(), e.path())))
            .filter(|(n, p)| is_name(n) && p.is_dir())
            .collect()
    };
    for (name, dir) in names(themes_root(home)) {
        if look_dir(&dir).is_none() {
            continue;
        }
        let record = read(home, Kind::Theme, &name, "journey.json");
        out.push(Copy {
            kind: Kind::Theme,
            from: theme_from(home, &gs, &dir, &record),
            // Its own files, or its journey's (a copy keeps the template's
            // file times).
            touched_ms: newest(&dir).max(newest(&record_dir(home, Kind::Theme, &name))),
            name,
            dir,
            record,
        });
    }
    for (name, dir) in names(copies_root(home)) {
        if !dir.join(gp_model::MODEL_FILE).is_file() {
            continue;
        }
        let record = read(home, Kind::New, &name, "journey.json");
        out.push(Copy {
            kind: Kind::New,
            from: new_from(&dir, &record),
            // Its own files, or its journey's (a copy keeps the template's
            // file times).
            touched_ms: newest(&dir).max(newest(&record_dir(home, Kind::New, &name))),
            name,
            dir,
            record,
        });
    }
    out.sort_by(|a, b| b.touched_ms.cmp(&a.touched_ms).then(a.name.cmp(&b.name)));
    out
}

/// A copy by its name, or `theme/NAME` or `new/NAME` where both kinds
/// have one of that name.
pub(crate) fn find(home: &Path, games_dir: Option<&Path>, name: &str) -> Result<Copy> {
    let (kind, n) = match name.split_once('/') {
        Some((k, n)) => match Kind::parse(k) {
            Some(k) => (Some(k), n),
            None => bail!("a copy is NAME, theme/NAME or new/NAME (got {name:?})"),
        },
        None => (None, name),
    };
    let all = list(home, games_dir);
    let found: Vec<Copy> = all
        .iter()
        .filter(|c| c.name == n && kind.is_none_or(|k| k == c.kind))
        .cloned()
        .collect();
    match found.len() {
        1 => Ok(found.into_iter().next().unwrap_or_else(|| unreachable!())),
        0 => bail!(
            "no copy {name}: your copies are {}",
            if all.is_empty() {
                "none yet".to_owned()
            } else {
                all.iter()
                    .map(|c| format!("{}/{}", c.kind.word(), c.name))
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        ),
        _ => bail!("{n} is both a theme and a new game: say theme/{n} or new/{n}"),
    }
}

/// The copy opened last on the page (`$GP_HOME/work/serve/last-copy`,
/// `KIND/NAME`), when it was opened after the last game.
pub(crate) fn last_copy(home: &Path) -> Option<String> {
    let serve = home.join("work").join("serve");
    let f = serve.join("last-copy");
    let c = fs::read_to_string(&f).ok()?.trim().to_owned();
    (mtime_ms(&f) >= mtime_ms(&serve.join("last-game")) && !c.is_empty()).then_some(c)
}

pub(crate) fn remember(home: &Path, kind: Kind, name: &str) {
    let d = home.join("work").join("serve");
    let _ = fs::create_dir_all(&d);
    let _ = fs::write(d.join("last-copy"), format!("{}/{name}\n", kind.word()));
}

// ---------------------------------------------------------------------
// What the commands leave for the journey.

/// The copy of `kind` a folder is, by its name, when it is one in this
/// home's own place for that kind.
fn copy_of(home: &Path, kind: Kind, dir: &Path) -> Option<String> {
    let root = match kind {
        Kind::Theme => themes_root(home),
        Kind::New => copies_root(home),
    };
    let root = root.canonicalize().ok()?;
    let dir = dir.canonicalize().ok()?;
    // A theme/ folder named for its theme stands for the theme.
    let dir = if kind == Kind::Theme && dir.file_name().is_some_and(|n| n == "theme") {
        dir.parent()?.to_owned()
    } else {
        dir
    };
    (dir.parent()? == root)
        .then(|| dir.file_name()?.to_str().map(str::to_owned))
        .flatten()
        .filter(|n| is_name(n))
}

/// `gp theme look check` on a theme copy: its result, for the journey.
pub(crate) fn note_check(home: &Path, dir: &Path, json: &Value) {
    let Some(name) = copy_of(home, Kind::Theme, dir) else {
        return;
    };
    let findings = json["findings"].as_array().cloned().unwrap_or_default();
    let problems: Vec<&Value> = findings
        .iter()
        .filter(|f| f["level"] == json!("problem"))
        .collect();
    let first = problems.first().map(|f| {
        let at = match (f["file"].as_str(), f["line"].as_u64()) {
            (Some(file), Some(l)) => format!(" ({file} line {l})"),
            (Some(file), None) => format!(" ({file})"),
            _ => String::new(),
        };
        format!("{}{at}", f["message"].as_str().unwrap_or(""))
    });
    let all_to_draw = !problems.is_empty()
        && problems.iter().all(|f| {
            f["group"] == json!("kit")
                && f["message"]
                    .as_str()
                    .is_some_and(|m| m.starts_with("still to draw"))
        });
    let _ = write(
        home,
        Kind::Theme,
        &name,
        "check.json",
        &json!({
            "at_ms": now_ms(),
            "ok": json["ok"],
            "problems": json["problems"],
            "warnings": json["warnings"],
            "first": first,
            "all_to_draw": all_to_draw,
            "findings": findings,
        }),
    );
}

/// `gp theme look play` on a theme copy: served or pictured.
pub(crate) fn note_play(home: &Path, dir: &Path, v: Value) {
    if let Some(name) = copy_of(home, Kind::Theme, dir) {
        let mut v = v;
        v["at_ms"] = json!(now_ms());
        let _ = write(home, Kind::Theme, &name, "play.json", &v);
    }
}

/// `gp level verify` on a new game: whether its stage passed.
pub(crate) fn note_verify(paths: &gp_evidence::Paths, game: &Path, r: &Result<crate::Outcome>) {
    let home = &paths.home;
    let dir = crate::level_edit::resolve(game, paths);
    let Some(name) = copy_of(home, Kind::New, &dir) else {
        return;
    };
    let v = match r {
        Ok(o) => {
            json!({"ok": o.exit == 0, "said": o.text.lines().take(12).collect::<Vec<_>>().join("\n")})
        }
        Err(e) => json!({"ok": false, "said": format!("{e:#}"), "refused": true}),
    };
    let mut v = v;
    v["at_ms"] = json!(now_ms());
    let _ = write(home, Kind::New, &name, "verify.json", &v);
}

// ---------------------------------------------------------------------
// The facts.

/// The last look check, as check.json keeps it.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Check {
    pub at_ms: u64,
    pub problems: u64,
    pub warnings: u64,
    pub first: Option<String>,
    pub all_to_draw: bool,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ThemeFacts {
    pub name: String,
    pub title: String,
    pub dir: PathBuf,
    pub from: Option<Origin>,
    pub style: Option<String>,
    /// Pictures in the theme's folder, and art.txt's lines still marked
    /// TO DRAW.
    pub pictures: u64,
    pub to_draw: u64,
    /// The newest file in the theme folder.
    pub changed_ms: u64,
    pub check: Option<Check>,
    /// When the theme was last served or pictured in the game.
    pub played_ms: Option<u64>,
    /// The steps the person said are done.
    pub done: Vec<String>,
    /// Pictures of the game can be taken: the person's ROM of it is here, or
    /// it plays from its own files.
    pub rom: bool,
    /// The theme's package name and the game's (for the share link).
    pub package: Option<String>,
    pub game_package: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct NewFacts {
    pub name: String,
    pub dir: PathBuf,
    pub from: Option<Origin>,
    /// Its pictures and levels are there (lifted or built).
    pub assets: bool,
    pub changed_ms: u64,
    /// When the copy was last built to play in a browser (build_publish).
    pub built_ms: Option<u64>,
    /// Stage edits made, and when the newest was.
    pub edits: u64,
    pub edited_ms: u64,
    /// The last `gp level verify`: when, whether it passed, and its words.
    pub verify: Option<(u64, bool, String)>,
    /// `gp level verify` refused the stage (it checks only some kinds of
    /// stage today): its words.
    pub no_check: Option<String>,
    pub done: Vec<String>,
    /// Theme copies made for this game.
    pub themes: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) enum Facts {
    Theme(ThemeFacts),
    New(NewFacts),
}

/// What every answer carries besides the journey's own facts.
#[derive(Debug, Clone, Default)]
pub(crate) struct Around {
    /// The agent's asks and the picture proposals, as crate::next reads them.
    pub asks: Vec<Value>,
    pub pictures: Vec<Value>,
    /// The sound changes the agent proposes for this copy
    /// (gp_mcp::sound_proposal::for_next).
    pub sounds: Vec<Value>,
    pub agent_runs: Vec<String>,
    pub page_runs: Vec<String>,
}

fn done_of(record: &Value) -> Vec<String> {
    record["steps_done"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| s.as_str().map(str::to_owned))
        .collect()
}

fn package_name(dir: &Path) -> Option<String> {
    let t: toml::Table = fs::read_to_string(dir.join("package.toml"))
        .ok()?
        .parse()
        .ok()?;
    t.get("package")?.get("name")?.as_str().map(str::to_owned)
}

fn theme_facts(home: &Path, roms: Option<&Path>, c: &Copy) -> ThemeFacts {
    let look = look_dir(&c.dir).unwrap_or_else(|| c.dir.join("theme"));
    let title = fs::read_to_string(look.join(gp_present::looks::LOOK_FILE))
        .ok()
        .and_then(|t| {
            t.lines()
                .find_map(|l| l.strip_prefix("title ").map(|s| s.trim().to_owned()))
        })
        .unwrap_or_else(|| c.name.clone());
    let mut pictures = 0;
    for d in [look.clone(), look.join("art")] {
        for e in fs::read_dir(d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_file()
                && p.extension()
                    .and_then(|x| x.to_str())
                    .is_some_and(|x| PICTURE_EXTS.contains(&x.to_ascii_lowercase().as_str()))
            {
                pictures += 1;
            }
        }
    }
    let art = fs::read_to_string(look.join("art.txt")).unwrap_or_default();
    let to_draw = art
        .lines()
        .filter(|l| l.trim_start().starts_with("# TO DRAW"))
        .count() as u64;
    let changed_ms = newest(&c.dir);
    let ck = read(home, Kind::Theme, &c.name, "check.json");
    let check = ck["at_ms"].as_u64().map(|at_ms| Check {
        at_ms,
        problems: ck["problems"].as_u64().unwrap_or(0),
        warnings: ck["warnings"].as_u64().unwrap_or(0),
        first: ck["first"].as_str().map(str::to_owned),
        all_to_draw: ck["all_to_draw"] == json!(true),
    });
    let played_ms = read(home, Kind::Theme, &c.name, "play.json")["at_ms"].as_u64();
    let from_dir = c.from.as_ref().and_then(|f| f.dir.clone());
    // A game with pictures of its own (a template, a copy of one) is
    // pictured with no ROM.
    let rom = from_dir.as_ref().zip(roms).is_some_and(|(d, roms)| {
        let (_, accepts, _) = crate::theme_kit::manifest(d);
        !accepts.is_empty() && crate::roms::find(&accepts, &[], roms).is_ok_and(|m| !m.is_empty())
    }) || from_dir
        .as_deref()
        .is_some_and(crate::theme_play::plays_without_rom);
    ThemeFacts {
        name: c.name.clone(),
        title,
        dir: c.dir.clone(),
        from: c.from.clone(),
        style: c.record["style"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned),
        pictures,
        to_draw,
        changed_ms,
        check,
        played_ms,
        done: done_of(&c.record),
        rom,
        package: package_name(&c.dir),
        game_package: from_dir.as_deref().and_then(package_name),
    }
}

fn new_facts(home: &Path, all: &[Copy], c: &Copy) -> NewFacts {
    let source = gp_model::Model::parse(
        &fs::read_to_string(c.dir.join(gp_model::MODEL_FILE)).unwrap_or_default(),
    )
    .ok()
    .and_then(|m| m.model.source);
    let lifted = |d: PathBuf| d.join(gp_assets::MAP_FILE).is_file();
    let assets = lifted(c.dir.join("assets"))
        || source.is_some_and(|s| lifted(home.join("work").join("assets").join(s)));
    let manifest = copies_root(home)
        .join(".publish")
        .join(&c.name)
        .join("web-preview")
        .join("manifest.json");
    let built_ms = manifest.is_file().then(|| mtime_ms(&manifest));
    let hist = c.dir.join(crate::level_edit::HISTORY_FILE);
    let edits = fs::read_to_string(&hist)
        .map(|t| t.lines().filter(|l| !l.trim().is_empty()).count() as u64)
        .unwrap_or(0);
    let v = read(home, Kind::New, &c.name, "verify.json");
    let no_check =
        (v["refused"] == json!(true)).then(|| v["said"].as_str().unwrap_or("").to_owned());
    let verify = v["at_ms"]
        .as_u64()
        .filter(|_| no_check.is_none())
        .map(|at| {
            (
                at,
                v["ok"] == json!(true),
                v["said"].as_str().unwrap_or("").to_owned(),
            )
        });
    NewFacts {
        name: c.name.clone(),
        dir: c.dir.clone(),
        from: c.from.clone(),
        assets,
        changed_ms: newest(&c.dir),
        built_ms,
        edits,
        edited_ms: if edits > 0 { mtime_ms(&hist) } else { 0 },
        verify,
        no_check,
        done: done_of(&c.record),
        themes: all
            .iter()
            .filter(|t| {
                t.kind == Kind::Theme
                    && t.from
                        .as_ref()
                        .is_some_and(|f| f.kind == "new" && f.name == c.name)
            })
            .map(|t| t.name.clone())
            .collect(),
    }
}

/// The facts of a copy, read from files only.
pub(crate) fn gather(paths: &gp_evidence::Paths, all: &[Copy], c: &Copy) -> Facts {
    gather_with(paths, all, c, true)
}

/// The same, looking for the person's ROM of the game only when `rom`
/// (it reads the ROM folder; a list of every copy leaves it out).
fn gather_with(paths: &gp_evidence::Paths, all: &[Copy], c: &Copy, rom: bool) -> Facts {
    match c.kind {
        Kind::Theme => Facts::Theme(theme_facts(
            &paths.home,
            rom.then_some(paths.roms.as_path()),
            c,
        )),
        Kind::New => Facts::New(new_facts(&paths.home, all, c)),
    }
}

/// The asks, pictures, sound changes and calls running, as crate::next
/// reads them; the pictures and a role's sounds only for this theme, the
/// song changes only for this new game.
pub(crate) fn around(home: &Path, c: &Copy) -> Around {
    let here = c.dir.canonicalize().unwrap_or_else(|_| c.dir.clone());
    let same = |p: &Value| {
        ["theme", "theme_dir"].iter().any(|k| {
            p[*k].as_str().is_some_and(|t| {
                let t = Path::new(t);
                let t = t.canonicalize().unwrap_or_else(|_| t.to_owned());
                t == here || t.parent() == Some(here.as_path())
            })
        })
    };
    let runs = gp_mcp::activity::running(home);
    let by_page = |r: &Value| r["by"] == json!("page");
    Around {
        asks: gp_mcp::asks::pending(home),
        pictures: if c.kind == Kind::Theme {
            crate::theme_picture::for_next(home)
                .into_iter()
                .filter(same)
                .collect()
        } else {
            Vec::new()
        },
        sounds: if c.kind == Kind::New {
            gp_mcp::sound_proposal::for_next(home)
                .into_iter()
                .filter(|p| {
                    p["copy_dir"].as_str().is_some_and(|d| {
                        let d = Path::new(d);
                        d.canonicalize().unwrap_or_else(|_| d.to_owned()) == here
                    })
                })
                .collect()
        } else {
            // A role's sound for this theme (issue #30).
            gp_mcp::sound_proposal::for_next(home)
                .into_iter()
                .filter(|p| p["kind"] == json!("role"))
                .filter(same)
                .collect()
        },
        agent_runs: runs
            .iter()
            .filter(|r| !by_page(r))
            .filter_map(|r| r["what"].as_str().map(str::to_owned))
            .collect(),
        page_runs: runs
            .iter()
            .filter(|r| by_page(r))
            .filter_map(|r| r["what"].as_str().map(str::to_owned))
            .collect(),
    }
}

// ---------------------------------------------------------------------
// The answer.

/// One of a journey's steps as the page lists them.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct JourneyStep {
    pub n: u8,
    pub id: String,
    pub name: &'static str,
    /// Whose turn the step mostly is.
    pub who: Who,
    /// `done`, `now` or `later`.
    pub state: &'static str,
}

pub(crate) const THEME_STEPS: [(&str, Who); 6] = [
    ("Pick the game and a style", Who::Person),
    ("Make the pictures", Who::Agent),
    ("The look check", Who::Tools),
    ("See it in the game", Who::Person),
    ("Change pictures", Who::Person),
    ("Share it", Who::Person),
];

pub(crate) const NEW_STEPS: [(&str, Who); 6] = [
    ("Start from a template", Who::Person),
    ("Name it", Who::Person),
    ("See it play", Who::Tools),
    ("Change a stage", Who::Person),
    ("Give it a look", Who::Person),
    ("Play and share", Who::Person),
];

/// The step a theme is at, 1 to 6.
pub(crate) fn theme_stage(f: &ThemeFacts) -> u8 {
    if f.from.is_none() || f.style.is_none() {
        return 1;
    }
    if f.pictures == 0 {
        return 2;
    }
    let checked = f.check.as_ref().filter(|c| c.at_ms >= f.changed_ms);
    match checked {
        None => 3,
        Some(c) if c.problems > 0 => 3,
        Some(c) => {
            if f.played_ms.is_none_or(|p| p < c.at_ms) {
                4
            } else if !f.done.iter().any(|d| d == "pictures") {
                5
            } else {
                6
            }
        }
    }
}

/// The step a new game is at, 3 to 6 (a copy has passed 1 and 2).
pub(crate) fn new_stage(f: &NewFacts) -> u8 {
    if !f.assets || f.built_ms.is_none() {
        return 3;
    }
    if !f.done.iter().any(|d| d == "stage") {
        return 4;
    }
    if !f.done.iter().any(|d| d == "look") {
        return 5;
    }
    6
}

fn steps_of(list: &[(&'static str, Who); 6], prefix: char, at: u8) -> Vec<JourneyStep> {
    list.iter()
        .enumerate()
        .map(|(i, (name, who))| {
            let n = i as u8 + 1;
            JourneyStep {
                n,
                id: format!("{prefix}{n}"),
                name,
                who: *who,
                state: match n.cmp(&at) {
                    std::cmp::Ordering::Less => "done",
                    std::cmp::Ordering::Equal => "now",
                    std::cmp::Ordering::Greater => "later",
                },
            }
        })
        .collect()
}

fn start(action: &'static str) -> Act {
    Act::Start { action }
}

fn theme_answer(f: &ThemeFacts) -> (u8, String, Vec<Step>) {
    let n = theme_stage(f);
    let t = &f.title;
    let game = f.from.as_ref().map_or("its game", |g| g.title.as_str());
    let dir = f.dir.display();
    let g = f
        .from
        .as_ref()
        .map(Origin::game_arg)
        .unwrap_or_else(|| "GAME".into());
    let mut steps = Vec::new();
    let stands = match n {
        1 if f.from.is_none() => format!("Your theme {t} doesn't say which game it is for yet."),
        1 => format!("Your theme {t} is for {game}. It needs a style in a few words."),
        2 => format!(
            "Your theme {t} for {game} has no pictures yet{}.",
            f.style
                .as_ref()
                .map(|s| format!(" (the style: {s})"))
                .unwrap_or_default()
        ),
        3 => match &f.check {
            Some(c) if c.at_ms >= f.changed_ms && c.problems > 0 => format!(
                "The look check found {} in {t}, and {}.",
                n_of(c.problems, "problem", "problems"),
                n_of(c.warnings, "warning", "warnings")
            ),
            Some(_) => format!("{t} changed since its last look check."),
            None => format!(
                "{t} has {} and hasn't been checked yet.",
                n_of(f.pictures, "picture", "pictures")
            ),
        },
        4 => format!("{t} passes the look check. You haven't seen it in the game since."),
        5 => format!("{t} plays in {game} on this computer."),
        _ => format!("{t} is ready to share."),
    };
    match n {
        1 if f.from.is_none() => steps.push(
            step(Who::Person, "Tell your agent which game this theme is for.")
                .page("the theme's first step"),
        ),
        1 => steps.push(
            step(
                Who::Person,
                "Say the style in a few words, like \"paper cutouts\" or \"night time, neon\". Type it on this page or tell your agent.",
            )
            .page("the style box on the theme's first step"),
        ),
        2 => {
            steps.push(
                step(
                    Who::Agent,
                    format!(
                        "Your agent can make the pictures in your style. Making them with an image model costs money, so it asks you first on a card. Or bring your own pictures: put them in {dir}/theme/art and tell your agent."
                    ),
                )
                .cli(format!(
                    "the theme-generation skill, then gp theme look check {dir} --game {g}"
                )),
            );
        }
        3 => match f.check.as_ref().filter(|c| c.at_ms >= f.changed_ms) {
            Some(c) if c.all_to_draw => steps.push(
                step(
                    Who::Agent,
                    format!(
                        "Your agent can make the {} the game still needs, then check again.",
                        n_of(c.problems, "picture", "pictures")
                    ),
                )
                .cli(format!("gp theme look check {dir} --game {g}")),
            ),
            Some(c) => steps.push(
                step(
                    Who::Agent,
                    format!(
                        "Your agent can fix the first problem: {}. Then check again.",
                        c.first.as_deref().unwrap_or("see the list on this page")
                    ),
                )
                .cli(format!("gp theme look check {dir} --game {g}")),
            ),
            None => steps.push(
                step(
                    Who::Tools,
                    "Press CHECK MY THEME. The tools read your theme the way the game does and list what is wrong. It is free and takes under a minute.",
                )
                .page("CHECK MY THEME")
                .cli(format!("gp theme look check {dir} --game {g}"))
                .act(start("look-check")),
            ),
        },
        4 => steps.push(
            step(
                Who::Person,
                "Press SEE IT IN THE GAME. The game opens with your theme, on this computer only. Load your copy of the game in it and play every stage.",
            )
            .page("SEE IT IN THE GAME")
            .cli(format!("gp theme look play {dir} --game {g} --background"))
            .act(start("look-play")),
        ),
        5 => {
            steps.push(
                step(
                    Who::Person,
                    "Play it and tell your agent which pictures to change. A new picture shows here as a card with ACCEPT and REJECT. When you like it, press READY TO SHARE.",
                )
                .page("READY TO SHARE"),
            );
            steps.push(
                step(
                    Who::Agent,
                    "Your agent can propose a new picture for anything the person wants changed.",
                )
                .cli(format!(
                    "gp theme picture propose {dir} --game {g} --picture ART --png FILE --why WORDS"
                )),
            );
        }
        _ => steps.push(
            step(
                Who::Person,
                "Share it: send the request on this page to the registry, and once it is published, the share link opens the game in your theme. Nothing is published from this page.",
            )
            .page("the share step"),
        ),
    }
    (n, stands, steps)
}

fn new_answer(f: &NewFacts) -> (u8, String, Vec<Step>) {
    let n = new_stage(f);
    let name = &f.name;
    let from = f.from.as_ref().map_or("a template", |t| t.title.as_str());
    let mut steps = Vec::new();
    let stands = match n {
        3 if !f.assets => {
            format!("{name} is your copy of {from}. Its pictures and levels aren't ready yet.")
        }
        3 => {
            format!("{name} is your copy of {from}. It hasn't been built to play in a browser yet.")
        }
        4 => match &f.verify {
            Some((at, false, _)) if *at >= f.edited_ms => {
                format!(
                    "{name} plays in your browser. The stage check found a problem after your last change."
                )
            }
            _ if f.edits > 0 => format!(
                "{name} plays in your browser. You changed its stage {}.",
                n_of(f.edits, "time", "times")
            ),
            _ => format!("{name} plays in your browser, the same as {from}."),
        },
        5 if f.themes.is_empty() => {
            format!("{name}'s stage is how you want it. It has no look of its own yet.")
        }
        5 => format!("{name} has a theme on the way: {}.", f.themes.join(", ")),
        _ => format!(
            "{name} is made: a stage of yours{}.",
            if f.themes.is_empty() {
                ""
            } else {
                " and a look of its own"
            }
        ),
    };
    let built_old = f.built_ms.is_some_and(|b| b < f.changed_ms);
    match n {
        3 if !f.assets => steps.push(
            step(
                Who::Agent,
                format!("Your agent can get {name}'s pictures and levels ready: they are made from your own copy of the original game, or built for a game made here."),
            )
            .cli(format!("gp template build {} (a game made here), or gp assets lift ROM", f.dir.display())),
        ),
        3 => steps.push(
            step(
                Who::Tools,
                "Press BUILD IT TO PLAY. The tools build your game to play in this browser. It takes a few minutes the first time.",
            )
            .page("BUILD IT TO PLAY")
            .act(start("build")),
        ),
        4 => {
            match &f.verify {
                Some((at, false, said)) if *at >= f.edited_ms && f.edits > 0 => steps.push(
                    step(
                        Who::Agent,
                        format!(
                            "Your agent can fix the stage, or you can press UNDO THE LAST CHANGE. The check said: {}",
                            said.lines().last().unwrap_or("").trim()
                        ),
                    )
                    .cli(format!("gp level show {name}, then gp level edit {name} --op ...")),
                ),
                v if f.edits > 0
                    && f.no_check.is_none()
                    && v.as_ref().is_none_or(|(at, _, _)| *at < f.edited_ms) =>
                {
                    steps.push(
                        step(
                            Who::Tools,
                            "Press CHECK THE STAGE. The tools check your stage still follows the game's rules and can be finished. Free.",
                        )
                        .page("CHECK THE STAGE")
                        .cli(format!("gp level verify {name}"))
                        .act(start("verify")),
                    );
                }
                _ => {}
            }
            steps.push(
                step(
                    Who::Person,
                    "Tell your agent what to change on the stage, like \"move the start to the right\". When the stage is how you want it, press DONE WITH THE STAGE.",
                )
                .page("DONE WITH THE STAGE")
                .cli(format!("gp level show {name}; gp level edit {name} --op ...; gp level undo {name}")),
            );
        }
        5 => steps.push(
            step(
                Who::Person,
                if f.themes.is_empty() {
                    "Press MAKE A THEME FOR IT to give your game pictures of its own, or SKIP to keep the template's.".to_owned()
                } else {
                    format!("Carry on with the theme {}, or press DONE when you like the look.", f.themes[0])
                },
            )
            .page("MAKE A THEME FOR IT"),
        ),
        _ => steps.push(
            step(
                Who::Person,
                "Play it here, and press MAKE A SHARE LINK: the tools put your game in your own registry on this computer and give you the link that plays it. For other people to open the link, your page and registry have to be put online.",
            )
            .page("MAKE A SHARE LINK")
            .cli(format!("gp game share {}", f.dir.display()))
            .act(start("share")),
        ),
    }
    if built_old && n > 3 {
        steps.push(
            step(
                Who::Tools,
                "Your last change isn't in the game you play yet. Press BUILD IT TO PLAY again.",
            )
            .page("BUILD IT TO PLAY")
            .act(start("build")),
        );
    }
    (n, stands, steps)
}

fn n_of(n: u64, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The answer for a copy, from its facts alone.
pub(crate) fn answer(facts: &Facts, ar: &Around) -> Answer {
    let (journey, subject, (n, stands, mut steps), list, prefix) = match facts {
        Facts::Theme(f) => ("theme", f.name.clone(), theme_answer(f), &THEME_STEPS, 'T'),
        Facts::New(f) => ("new", f.name.clone(), new_answer(f), &NEW_STEPS, 'N'),
    };
    let mut waits = Vec::new();
    let mut first: Vec<Step> = Vec::new();
    for a in &ar.asks {
        let s = |k: &str| a[k].as_str().map(str::to_owned);
        let says = s("says").unwrap_or_default();
        waits.push(Yes {
            what: s("needs").unwrap_or_default(),
            says: says.clone(),
            id: s("id"),
            state: s("state"),
            game: s("game"),
            picture: None,
            sound: None,
        });
        if a["state"] == json!("open") {
            first.push(step(
                Who::Person,
                format!("Your agent waits for your yes: {says} Press YES or NO on the card."),
            ));
        }
    }
    for p in &ar.pictures {
        let s = |k: &str| p[k].as_str().map(str::to_owned);
        let state = s("state").unwrap_or_default();
        let pic = s("picture").unwrap_or_default();
        let says = match state.as_str() {
            "open" => format!("A new {pic}: {}", s("why").unwrap_or_default()),
            "accepted" => format!("You accepted the new {pic}."),
            "rejected" => format!("You rejected the new {pic}."),
            "undone" => format!("You put the old {pic} back."),
            _ => format!("The new {pic} wasn't used: the picture changed first."),
        };
        if state == "open" && p["place"] == json!(1) {
            first.push(step(
                Who::Person,
                format!(
                    "Your agent proposes a picture: {says} Press ACCEPT or REJECT on the card."
                ),
            ));
        }
        waits.push(Yes {
            what: "picture".into(),
            says,
            id: s("id"),
            state: Some(state),
            game: s("game"),
            picture: Some(p.clone()),
            sound: None,
        });
    }
    for p in &ar.sounds {
        let (y, st) = crate::next::sound_wait(p);
        if y.state.as_deref() == Some("open") {
            first.extend(st);
        }
        waits.push(y);
    }
    first.append(&mut steps);
    let mut seen = std::collections::BTreeSet::new();
    first.retain(|s| seen.insert(s.says.clone()));
    let next = first.remove(0);
    let mut also: Vec<Step> = Vec::new();
    if next.who != Who::Person {
        also.extend(first.iter().filter(|s| s.who == Who::Person).cloned());
    }
    also.extend(
        first
            .into_iter()
            .filter(|s| !also.contains(s))
            .collect::<Vec<_>>(),
    );
    also.truncate(2);
    let running = ar
        .agent_runs
        .iter()
        .map(|w| Running {
            what: format!("your agent: {w}"),
            look_again_s: 30,
        })
        .chain(ar.page_runs.iter().map(|w| Running {
            what: format!("started on the page: {w}"),
            look_again_s: 30,
        }))
        .collect();
    let facts = match facts {
        Facts::Theme(f) => json!({
            "dir": f.dir, "title": f.title, "from": f.from, "style": f.style,
            "pictures": f.pictures, "to_draw": f.to_draw,
            "check": f.check.as_ref().map(|c| json!({"at_ms": c.at_ms, "problems": c.problems,
                "warnings": c.warnings, "first": c.first, "current": c.at_ms >= f.changed_ms})),
            "played_ms": f.played_ms, "steps_done": f.done, "rom": f.rom,
            "package": f.package, "game_package": f.game_package,
            "share": share(f),
        }),
        Facts::New(f) => json!({
            "dir": f.dir, "from": f.from, "assets": f.assets, "built_ms": f.built_ms,
            "built_current": f.built_ms.is_some_and(|b| b >= f.changed_ms),
            "edits": f.edits,
            "verify": f.verify.as_ref().map(|(at, ok, said)| json!({"at_ms": at, "ok": ok, "said": said,
                "current": *at >= f.edited_ms})),
            "no_check": f.no_check,
            "steps_done": f.done, "themes": f.themes,
        }),
    };
    Answer {
        version: VERSION,
        journey,
        subject: Some(subject),
        stage: StageId {
            id: format!("{prefix}{n}"),
            n,
            of: 6,
            name: list[n as usize - 1].0,
        },
        stands,
        next,
        also,
        running,
        waits_for_yes: waits,
        clue: None,
        changes: 0,
        page_changes: 0,
        facts,
        steps: steps_of(list, prefix, n),
        copies: Vec::new(),
    }
}

/// The share link and the publish request of a theme, as words to send:
/// nothing is sent from here.
pub(crate) fn share(f: &ThemeFacts) -> Value {
    let (Some(theme), Some(game)) = (&f.package, &f.game_package) else {
        return json!({"link": null, "why": "a share link needs the theme's and the game's package names, and one of them isn't known"});
    };
    let game_title = f.from.as_ref().map_or(game.as_str(), |g| g.title.as_str());
    json!({
        "link": format!("{PUBLIC_PAGE}/p?g={game}&c={theme}"),
        "request": {
            "where": REGISTRY_ISSUES,
            "title": format!("Publish the theme {theme} for {game}"),
            "body": format!(
                "Please publish my theme {} ({theme}) for {game_title} ({game}).\n\nIts repo: LINK TO YOUR REPO\n\nIt passes gp theme look check with no problems, and I played it in the game with gp theme look play.\n",
                f.title
            ),
        },
    })
}

/// A short line for a copy in a list: its journey, step and whose turn.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct Line {
    pub journey: &'static str,
    pub name: String,
    pub from: Option<Origin>,
    pub touched_ms: u64,
    pub stage: StageId,
    pub who: Who,
    pub says: String,
}

/// Every copy as a line, for the first screen and every answer.
pub(crate) fn lines(paths: &gp_evidence::Paths, all: &[Copy]) -> Vec<Line> {
    all.iter()
        .map(|c| {
            let a = answer(&gather_with(paths, all, c, false), &Around::default());
            Line {
                journey: a.journey,
                name: c.name.clone(),
                from: c.from.clone(),
                touched_ms: c.touched_ms,
                stage: a.stage,
                who: a.next.who,
                says: a.next.says,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests;
