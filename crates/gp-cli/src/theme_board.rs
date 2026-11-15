//! An arcade game's board pictures for a large-art theme
//! (docs/formats/board-art.md, "Themes"): the names a theme may give them
//! (a screen by its `board-screen` slot, a sprite by its entity and picture
//! code, `sprite-mug-4d`, a counter's digits and tile, `counter-score-7`
//! and `counter-lives`), from the game's model and the person's own lift;
//! and `gp theme look sheet`, which plays the game's recorded inputs on the
//! model and writes, under the person's home or a folder they name outside
//! any repo, which of those names the game showed and a contact sheet of
//! each entity's sprite codes. Nothing of it is committed: the codes an
//! entity shows and the pictures come from the person's own romset.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use gp_evidence::Paths;
use gp_look::board::{BoardArt, CELL_SIZE, SCREEN_SIZE, SPRITE_SIZE, sprite_name};
use gp_model::Model;
use gp_theme::pix::Rgba;
use serde_json::json;

use crate::Outcome;

/// The file of the names a game showed, in the sheet's folder.
pub(crate) const SEEN_FILE: &str = "names.toml";

/// An arcade game's board-drawn pictures: the names a theme may give
/// them with their sizes, the board's presentation of them (what the
/// page resolves a theme's rules against), and, once `gp theme look
/// sheet` has run on this machine, how many frames the game was seen to
/// show each.
pub(crate) struct BoardKit {
    pub names: Vec<(String, [u32; 2])>,
    pub present: gp_present::Present,
    pub seen: Option<BTreeMap<String, u64>>,
    /// Where the sheet is (or would be written).
    pub sheet: PathBuf,
}

/// The sheet's usual folder: `$GP_HOME/work/theme-kits/MODEL/board`, the
/// local part of a kit (`gp theme kit` writes the NES one beside it).
pub(crate) fn sheet_dir(home: &Path, model: &str) -> PathBuf {
    home.join("work")
        .join("theme-kits")
        .join(model)
        .join("board")
}

/// The board kit of a model drawn by its board, from the lifted folder:
/// None for one that names no board art.
pub(crate) fn kit(
    model: &Model,
    spec: &gp_sim::Spec,
    lifted: &Path,
    home: &Path,
) -> Result<Option<BoardKit>, String> {
    let Some(art) = BoardArt::load(model, spec, lifted)? else {
        return Ok(None);
    };
    let sheet = sheet_dir(home, &model.model.name);
    Ok(Some(BoardKit {
        names: art.names(),
        present: art.present(),
        seen: read_seen(&sheet),
        sheet,
    }))
}

/// The names a sheet recorded and their frames, when there is one.
pub(crate) fn read_seen(dir: &Path) -> Option<BTreeMap<String, u64>> {
    let t: toml::Table = fs::read_to_string(dir.join(SEEN_FILE)).ok()?.parse().ok()?;
    Some(
        t.get("seen")?
            .as_table()?
            .iter()
            .filter_map(|(k, v)| Some((k.clone(), u64::try_from(v.as_integer()?).ok()?)))
            .collect(),
    )
}

/// The board's names a model gives without a lift, for `gp theme look new`:
/// each screen slot its levels' looks show, each entity the board draws,
/// and each counter drawn as digits or as a tile repeated.
pub(crate) struct ModelNames {
    pub screens: Vec<String>,
    pub entities: Vec<String>,
    pub digits: Vec<String>,
    pub tiles: Vec<String>,
}

pub(crate) fn model_names(model: &Model) -> Option<ModelNames> {
    if !gp_look::board::names_board(model) {
        return None;
    }
    let mut n = ModelNames {
        screens: Vec::new(),
        entities: Vec::new(),
        digits: Vec::new(),
        tiles: Vec::new(),
    };
    let add = |v: &mut Vec<String>, s: &str| {
        if !v.iter().any(|x| x == s) {
            v.push(s.to_owned());
        }
    };
    for l in model.levels.values() {
        let Some(b) = l.core.present.as_ref().and_then(|p| p.board.as_ref()) else {
            continue;
        };
        for s in b
            .screen
            .iter()
            .chain(b.looks.iter().filter_map(|o| o.screen.as_ref()))
        {
            add(&mut n.screens, s);
        }
        for c in &b.counters {
            add(&mut n.digits, &c.counter);
        }
        for r in &b.repeats {
            add(&mut n.tiles, &r.counter);
        }
    }
    for (name, e) in &model.entities {
        if e.core.board_sprite.is_some() {
            add(&mut n.entities, name);
        }
    }
    Some(n)
}

/// The lines `gp theme look new` writes into art.txt for an arcade game:
/// each name to draw, with its size in the board's picture's pixels, and
/// the codes the sheet saw each entity show when there is a sheet.
pub(crate) fn new_lines(
    n: &ModelNames,
    seen: Option<&BTreeMap<String, u64>>,
    game: &str,
) -> (String, usize) {
    let mut out = String::new();
    let mut slots = 0;
    let _ = writeln!(
        out,
        "\n# {game} is drawn by its board: sizes and places are in the board's picture's pixels\n# (512 by 480). `gp theme look sheet --game GAME` lists the sprite codes each entity\n# shows, from your own romset, with a contact sheet of them (never commit it)."
    );
    for s in &n.screens {
        slots += 1;
        let _ = writeln!(
            out,
            "\n# TO DRAW the screen {s}: {} by {} (`pic {s} art/{s}.png {} {} 0 0`)",
            SCREEN_SIZE[0], SCREEN_SIZE[1], SCREEN_SIZE[0], SCREEN_SIZE[1]
        );
    }
    for e in &n.entities {
        slots += 1;
        let prefix = format!("sprite-{e}-");
        let codes: Vec<&str> = seen
            .map(|s| s.keys().filter_map(|k| k.strip_prefix(&prefix)).collect())
            .unwrap_or_default();
        let which = if codes.is_empty() {
            "each picture code it shows (run `gp theme look sheet` to list them)".to_owned()
        } else {
            format!("the codes your sheet saw it show: {}", codes.join(", "))
        };
        let _ = writeln!(
            out,
            "\n# TO DRAW {e}: {} by {} a picture, one line per code, sprite-{e}-CODE; {which}",
            SPRITE_SIZE[0], SPRITE_SIZE[1]
        );
    }
    for c in &n.digits {
        slots += 1;
        let _ = writeln!(
            out,
            "\n# TO DRAW the digits of {c}: {} by {} each, counter-{c}-0 to counter-{c}-9",
            CELL_SIZE[0], CELL_SIZE[1]
        );
    }
    for c in &n.tiles {
        slots += 1;
        let _ = writeln!(
            out,
            "\n# TO DRAW the tile {c} repeats: {} by {}, counter-{c}",
            CELL_SIZE[0], CELL_SIZE[1]
        );
    }
    (out, slots)
}

/// Whether a folder is inside a git repository (a `.git` in it or above).
fn in_repo(dir: &Path) -> Option<PathBuf> {
    let mut d = dir.canonicalize().ok().or_else(|| Some(dir.to_owned()))?;
    loop {
        if d.join(".git").exists() {
            return Some(d);
        }
        if !d.pop() {
            return None;
        }
    }
}

/// The nearest existing folder at or above `p`, so a folder still to be
/// made is judged by where it would go.
fn existing(p: &Path) -> PathBuf {
    let mut d = p.to_owned();
    while !d.exists() {
        if !d.pop() {
            return PathBuf::from(".");
        }
    }
    d
}

/// The recorded plays of a game: each `inputs/ID.start.presses` from
/// power-on with its clip's `inputs/ID.presses` after it, as frames.
fn plays(game: &Path) -> Result<Vec<(String, Vec<gp_runtime::InputFrame>)>> {
    let dir = game.join("inputs");
    let mut starts: Vec<PathBuf> = fs::read_dir(&dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().ends_with(".start.presses"))
        })
        .collect();
    starts.sort();
    let frames_of = |p: &Path| -> Result<Vec<gp_runtime::InputFrame>> {
        let text = fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?;
        let presses = crate::clipset::parse_presses(&text)?;
        let log = gp_evidence::InputLog::parse(&crate::clipset::log_of_presses(&presses)?)
            .map_err(|e| anyhow!("{}: {e}", p.display()))?;
        Ok(crate::run::frames(&log))
    };
    let mut out = Vec::new();
    for s in starts {
        let name = s
            .file_name()
            .map(|n| {
                n.to_string_lossy()
                    .trim_end_matches(".start.presses")
                    .to_owned()
            })
            .unwrap_or_default();
        let mut frames = frames_of(&s)?;
        let clip = dir.join(format!("{name}.presses"));
        if clip.is_file() {
            frames.extend(frames_of(&clip)?);
        }
        out.push((name, frames));
    }
    Ok(out)
}

/// `gp theme look sheet`: plays the game's recorded inputs on the model
/// with the person's own lift, and writes into `out` (default the sheet's
/// usual folder) `names.toml` (each board name the game showed and in how
/// many frames), a contact sheet per board-drawn entity
/// (`sprites-ENTITY.png`, each code it showed, labelled) and each screen it
/// showed (`SLOT.png`). Refuses a folder inside a repo.
pub(crate) fn sheet(
    game_arg: &Path,
    rom: Option<&Path>,
    out: Option<&Path>,
    paths: &Paths,
) -> Result<Outcome> {
    let game_dir = crate::theme_kit::template_dir(game_arg)?;
    let game_dir = game_dir.canonicalize().unwrap_or(game_dir);
    let (model, _) = crate::theme::read_model(&game_dir)?;
    if !gp_look::board::names_board(&model) {
        bail!(
            "{} isn't drawn by an arcade board's art (its model names no board_sprite or present.board), so a theme names its pictures as `gp theme look check` lists them",
            model.model.title
        );
    }
    let out = match out {
        Some(o) => o.to_owned(),
        None => sheet_dir(&paths.home, &model.model.name),
    };
    if let Some(repo) = in_repo(&existing(&out)) {
        bail!(
            "{} is inside the repository {}: the sheet holds pictures of your ROM's game, which never go in a repo. Name a folder outside it, or leave out --out for {}",
            out.display(),
            repo.display(),
            sheet_dir(&paths.home, &model.model.name).display()
        );
    }
    let t = crate::template_extract::Template::load(&game_dir)?;
    let lifted = crate::template_font::lifted_dir(&t, rom, paths)?.map_err(|e| {
        anyhow!("{e}: the sheet is drawn from your own romset; add it with `gp roms add`, or give it with --rom")
    })?;
    let bundle = gp_look::bundle(&model, lifted.path()).map_err(anyhow::Error::msg)?;
    let spec = gp_interp::compile::compile(&model, &bundle.assets).map_err(anyhow::Error::msg)?;
    let units = gp_interp::mapping::board_units_in(&game_dir).map_err(anyhow::Error::msg)?;
    let art = BoardArt::load(&model, &spec, lifted.path())
        .map_err(anyhow::Error::msg)?
        .ok_or_else(|| anyhow!("the model names no board art"))?
        .in_board_units(units);
    let present = gp_look::present::art(&model, &spec, lifted.path())
        .map_err(anyhow::Error::msg)?
        .present;
    let kind_names: Vec<String> = spec.kinds.iter().map(|k| k.name.clone()).collect();

    // Every board name each frame shows, and a frame's flags and level for
    // each sprite name (the colour bank it was seen in), and each screen's
    // first picture.
    let mut seen: BTreeMap<String, u64> = BTreeMap::new();
    let mut first: BTreeMap<String, (usize, u8)> = BTreeMap::new();
    let mut screens: BTreeMap<String, Rgba> = BTreeMap::new();
    let runs = plays(&game_dir)?;
    if runs.is_empty() {
        bail!(
            "{} has no recorded plays (inputs/*.start.presses) to play for the sheet",
            model.model.title
        );
    }
    let mut frames = 0u64;
    let mut not_run = Vec::new();
    for (_, inputs) in &runs {
        let mut rt = crate::theme_run::Run::open(&bundle, &game_dir, paths)?;
        not_run = rt.not_run.clone();
        let mut anim = gp_present::Animator::new();
        for f in inputs {
            rt.advance(f).map_err(anyhow::Error::msg)?;
            let (spec, w) = (rt.spec(), rt.world());
            anim.step(&present, spec, w);
            let scene = anim.scene(&present, spec, w);
            let shown: Vec<bool> = scene.bodies.iter().map(Option::is_some).collect();
            let Some(pic) = art.layout(w, &shown, &scene.rank) else {
                continue;
            };
            frames += 1;
            let mut names: Vec<String> = Vec::new();
            for d in &pic.drawn {
                let n = sprite_name(&kind_names[d.kind], d.code);
                first.entry(n.clone()).or_insert((w.level, d.flags));
                names.push(n);
            }
            if let Some(s) = &pic.screen {
                if !screens.contains_key(s) {
                    let none = gp_look::board::Omit {
                        bodies: vec![true; w.bodies.len()],
                        ..Default::default()
                    };
                    if let Some(p) = art.draw_omitting(w, &shown, &scene.rank, &none) {
                        screens.insert(s.clone(), rgba_of(&p));
                    }
                }
                names.push(s.clone());
            }
            names.extend(pic.cells.iter().map(|c| c.name.clone()));
            names.sort();
            names.dedup();
            for n in names {
                *seen.entry(n).or_default() += 1;
            }
        }
    }

    fs::create_dir_all(&out).with_context(|| format!("creating {}", out.display()))?;
    let mut toml_text = format!(
        "# The board's pictures {} showed in its recorded plays ({} plays, {frames} frames),\n# played on your own lift by `gp theme look sheet`: names a theme may draw, each\n# with the frames it showed in. `gp theme look check` reads this file. Pictures of\n# your ROM's game are beside it: never commit this folder.\n[seen]\n",
        model.model.title,
        runs.len()
    );
    for (n, c) in &seen {
        let _ = writeln!(toml_text, "\"{n}\" = {c}");
    }
    fs::write(out.join(SEEN_FILE), toml_text)?;
    let mut written = vec![SEEN_FILE.to_owned()];
    for (s, img) in &screens {
        let f = format!("{s}.png");
        fs::write(out.join(&f), img.png())?;
        written.push(f);
    }
    let mut per_entity: BTreeMap<&str, Vec<(u16, String)>> = BTreeMap::new();
    for n in seen.keys() {
        let Some(rest) = n.strip_prefix("sprite-") else {
            continue;
        };
        let Some((e, code)) = rest.rsplit_once('-') else {
            continue;
        };
        if let Ok(c) = u16::from_str_radix(code, 16) {
            per_entity.entry(e).or_default().push((c, n.clone()));
        }
    }
    let mut entity_lines = Vec::new();
    for (e, codes) in &per_entity {
        let img = contact_sheet(&art, codes, &first);
        let f = format!("sprites-{e}.png");
        fs::write(out.join(&f), img.png())?;
        written.push(f);
        let list: Vec<String> = codes.iter().map(|(c, _)| format!("{c:02x}")).collect();
        entity_lines.push(format!("{e}: {} codes ({})", codes.len(), list.join(" ")));
    }
    let mut text = format!(
        "Played {} recorded plays of {} ({frames} frames with the board drawn) on your own lift, and wrote {} files to {}:\n",
        runs.len(),
        model.model.title,
        written.len(),
        out.display()
    );
    for l in &entity_lines {
        let _ = writeln!(text, "  {l}");
    }
    let _ = writeln!(
        text,
        "  screens: {}",
        screens.keys().cloned().collect::<Vec<_>>().join(", ")
    );
    if !not_run.is_empty() {
        let _ = writeln!(
            text,
            "These hooks didn't run (no module for the game), so the plays may show less than the game does: {}",
            not_run.join(", ")
        );
    }
    let _ = write!(
        text,
        "A theme names them as sprite-ENTITY-CODE with the code in lowercase hex as listed (pictures {} by {}), a screen by its slot ({} by {}) and a counter's digits as counter-NAME-D ({} by {}). These are pictures of your ROM's game: never commit them.",
        SPRITE_SIZE[0], SPRITE_SIZE[1], SCREEN_SIZE[0], SCREEN_SIZE[1], CELL_SIZE[0], CELL_SIZE[1]
    );
    Ok(Outcome::ok(
        json!({
            "dir": out.display().to_string(),
            "plays": runs.len(),
            "frames": frames,
            "files": written,
            "seen": seen,
        }),
        text,
    ))
}

/// A board picture as RGBA, a pen left to a theme clear.
fn rgba_of(p: &gp_look::board::Picture) -> Rgba {
    let mut img = Rgba::new(SCREEN_SIZE[0], SCREEN_SIZE[1]);
    for (i, pen) in p.pens.iter().enumerate() {
        if *pen == gp_look::board::TRANSPARENT {
            continue;
        }
        let c = p.palette[(pen & 0x3f) as usize];
        img.set(
            i as u32 % SCREEN_SIZE[0],
            i as u32 / SCREEN_SIZE[0],
            [(c >> 16) as u8, (c >> 8) as u8, c as u8, 255],
        );
    }
    img
}

/// An entity's codes as a contact sheet: each sprite at twice its size on
/// grey, its code under it, eight to a row.
fn contact_sheet(
    art: &BoardArt,
    codes: &[(u16, String)],
    first: &BTreeMap<String, (usize, u8)>,
) -> Rgba {
    const PER_ROW: u32 = 8;
    const CELL: [u32; 2] = [80, 92];
    let rows = (codes.len() as u32).div_ceil(PER_ROW).max(1);
    let mut img = Rgba::new(CELL[0] * PER_ROW, CELL[1] * rows);
    for y in 0..img.height {
        for x in 0..img.width {
            img.set(x, y, [96, 96, 104, 255]);
        }
    }
    for (k, (code, name)) in codes.iter().enumerate() {
        let (cx, cy) = (
            (k as u32 % PER_ROW) * CELL[0],
            (k as u32 / PER_ROW) * CELL[1],
        );
        let (level, flags) = first.get(name).copied().unwrap_or((0, 0));
        if let Some(px) = art.sprite_rgba(level, *code, flags) {
            for (i, c) in px.iter().enumerate() {
                if c[3] == 0 {
                    continue;
                }
                let (x, y) = (i as u32 % 32, i as u32 / 32);
                for dy in 0..2 {
                    for dx in 0..2 {
                        img.set(cx + 8 + x * 2 + dx, cy + 4 + y * 2 + dy, *c);
                    }
                }
            }
        }
        crate::theme_sheet::text(
            &mut img,
            cx + 8,
            cy + 72,
            &format!("{code:02x}"),
            2,
            [255, 255, 255, 255],
            64,
        );
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A made-up arcade model: one board-drawn entity, two screens (the
    /// level's and a look's), a score drawn as digits and lives as a tile.
    const MODEL: &str = r#"
[model]
schema = "0.4"
name = "cab"
title = "Cab"
view = "side"
frame_rate = "30"
systems = ["rules"]

[entities.player]
role = "player"
size = [16, 16]
hitbox = { x = 0, y = 0, w = 16, h = 16 }

[entities.player.board_sprite]
slot = "sprites"
picture = { word = 1, byte = 1 }

[entities.wall]
role = "enemy"
size = [16, 16]
hitbox = { x = 0, y = 0, w = 16, h = 16 }

[levels.main]
size = [32, 30]
tile_size = 16

[levels.main.present.board]
palette = "colors"
tiles = "tiles"
screen = "stage"

[[levels.main.present.board.looks]]
when = [{ counter = "bar", cmp = "==", value = 1 }]
screen = "stage2"

[[levels.main.present.board.counters]]
counter = "score"
at = [2, 1]
digits = 4
tile = { slot = "codes" }
high = { slot = "codes", column = 1 }

[[levels.main.present.board.repeats]]
counter = "lives"
at = [8, 2]
most = 3
tile = { slot = "codes", column = 3 }
high = { slot = "codes", column = 1 }
"#;

    #[test]
    fn a_board_drawn_game_names_its_screens_entities_and_counters() {
        let m = Model::parse(MODEL).unwrap();
        let n = model_names(&m).unwrap();
        assert_eq!(n.screens, ["stage", "stage2"]);
        assert_eq!(n.entities, ["player"]);
        assert_eq!(
            (n.digits.as_slice(), n.tiles.as_slice()),
            (&["score".to_owned()][..], &["lives".to_owned()][..])
        );
        // Without a sheet: each to draw, the codes not listed.
        let (text, slots) = new_lines(&n, None, "Cab");
        assert_eq!(slots, 5);
        assert!(
            text.contains(
                "# TO DRAW the screen stage: 512 by 480 (`pic stage art/stage.png 512 480 0 0`)"
            ),
            "{text}"
        );
        assert!(text.contains("# TO DRAW player: 32 by 32 a picture, one line per code, sprite-player-CODE; each picture code it shows (run `gp theme look sheet` to list them)"), "{text}");
        assert!(
            text.contains("counter-score-0 to counter-score-9") && text.contains("counter-lives"),
            "{text}"
        );
        // With one: the codes it saw.
        let seen: BTreeMap<String, u64> = [
            ("sprite-player-4d".to_owned(), 3),
            ("sprite-player-12".to_owned(), 1),
            ("stage".to_owned(), 4),
        ]
        .into();
        let (text, _) = new_lines(&n, Some(&seen), "Cab");
        assert!(
            text.contains("the codes your sheet saw it show: 12, 4d"),
            "{text}"
        );
        // Every line is a note to a reader of the rules: a page reads none.
        let rules = gp_present::art::ArtRules::read(&format!("gp-art 1\n{text}")).unwrap();
        assert!(rules.pics.is_empty());
        // A game its board doesn't draw has none.
        let plain = Model::parse(&MODEL.replace("[entities.player.board_sprite]\nslot = \"sprites\"\npicture = { word = 1, byte = 1 }\n", "").replace("[levels.main.present.board]", "[levels.main.present.unused]")).ok();
        assert!(plain.is_none_or(|m| model_names(&m).is_none()));
    }

    #[test]
    fn a_sheet_is_read_back_and_never_written_into_a_repo() {
        let d = tempfile::tempdir().unwrap();
        fs::write(
            d.path().join(SEEN_FILE),
            "# made up\n[seen]\n\"sprite-player-4d\" = 3\n\"stage\" = 10\n",
        )
        .unwrap();
        let s = read_seen(d.path()).unwrap();
        assert_eq!(s["sprite-player-4d"], 3);
        assert_eq!(s.len(), 2);
        assert!(read_seen(&d.path().join("none")).is_none());
        fs::create_dir_all(d.path().join("repo/.git")).unwrap();
        let inside = d.path().join("repo/sheets/new");
        assert_eq!(
            in_repo(&existing(&inside)),
            Some(d.path().join("repo").canonicalize().unwrap())
        );
        assert_eq!(
            in_repo(&existing(&d.path().join("elsewhere")))
                .filter(|r| r.starts_with(d.path().canonicalize().unwrap())),
            None
        );
    }
}
