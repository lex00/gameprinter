//! `gp theme picture`: the agent proposes a new picture for one art file of
//! the person's own copy of a large-art theme, and the person accepts or
//! rejects it on their page (issue #4). A proposal is checked before
//! anyone sees it and kept apart from the theme; nothing in the theme
//! changes until the person's ACCEPT, and an accepted picture can be put
//! back.
//!
//! | File | What |
//! |---|---|
//! | `$GP_HOME/work/proposals/pictures/ID/proposal.json` | One proposal: `{id, theme, title, game, picture, rules, why, made_ms, state, words?, answered_ms?, told_ms?, old_sha, new_sha, old_px, new_px, checked}`; state is `open`, `accepted`, `rejected`, `undone` or `stale` |
//! | `.../ID/new.png` | The picture proposed, as the agent gave it |
//! | `.../ID/old.png` | The picture it replaces, as it was when proposed: the undo |
//!
//! The person's own copy is a theme folder under `$GP_HOME/work/themes/`
//! (the theme skill's work folder). The page answers through gp serve's
//! `/api/pictures` routes (serve/pictures.rs); in a terminal, `accept`,
//! `reject` and `undo` do the same. `gp_run` refuses those three: only the
//! person decides (gp_mcp::policy). The agent reads the answer with
//! `list`, which marks it told, and hears it come with `next` (wait_s):
//! an open proposal is in `waits_for_yes` as a card, and an answer the
//! agent hasn't read is there too.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use gp_evidence::Paths;
use gp_present::art::ArtRules;
use gp_present::looks::{LOOK_FILE, Look};
use gp_theme::pix::Rgba;
use serde_json::{Value, json};
use sha2::Digest as _;

use crate::Outcome;

#[derive(Subcommand)]
pub enum PictureCmd {
    /// Propose a new picture for one art file of the person's own copy of a
    /// large-art theme (a theme folder under $GP_HOME/work/themes/). It is
    /// checked first: a PNG, its shape fits every art.txt rule that draws
    /// it, `gp theme look check` on the theme with it in place finds no
    /// problem the theme didn't have, and it holds no ROM data. A proposal
    /// that passes waits on the person's page as a card with the old and
    /// new pictures, ACCEPT and REJECT; nothing in the theme changes until
    /// they accept. Writes only the proposal, under
    /// $GP_HOME/work/proposals/pictures/. Exits 1, with why, when a check
    /// refuses it. Example: gp theme picture propose
    /// ~/.gameprinter/work/themes/clay --game donkey-kong --picture
    /// art/hero.png --png /tmp/hero-red.png --why "A red cap, as you asked."
    Propose {
        /// The theme's folder (holding theme/look.txt, or look.txt), under
        /// $GP_HOME/work/themes/.
        dir: PathBuf,
        /// The game the theme is for: its folder, or its name in the games
        /// checkout.
        #[arg(long)]
        game: PathBuf,
        /// The picture to replace: its path in the theme folder as art.txt
        /// names it (art/hero.png), or the game's picture a `pic` rule
        /// draws with it (frame-0-0.png).
        #[arg(long)]
        picture: String,
        /// The new picture, a PNG file you wrote.
        #[arg(long)]
        png: PathBuf,
        /// One sentence for the person: why this picture.
        #[arg(long)]
        why: String,
        /// Your ROM of the game, for the ROM-data check (default: one under
        /// $GP_ROM_DIR the game accepts).
        #[arg(long)]
        rom: Option<PathBuf>,
    },
    /// The proposals and the person's answers: open ones still waiting,
    /// then the ones answered (accepted, rejected with their words, or put
    /// back), newest first. Reading an answer here marks it told, so
    /// `next` stops showing it. Example: gp theme picture list
    List {
        /// Only this theme's folder.
        #[arg(long)]
        theme: Option<PathBuf>,
        /// Every proposal, not only the open ones and the answers from the
        /// last day.
        #[arg(long)]
        all: bool,
    },
    /// The person's ACCEPT, from a terminal: put the proposed picture in
    /// the theme, keeping the old one for `undo`. The page's ACCEPT does
    /// the same. Refused to an agent. Example: gp theme picture accept
    /// 1763140000000-1a2b3c4d
    Accept {
        /// The proposal's id.
        id: String,
        /// Words for the agent, if any.
        #[arg(long)]
        words: Option<String>,
    },
    /// The person's REJECT, from a terminal: drop the proposal, with what
    /// to change for the agent. Refused to an agent. Example: gp theme
    /// picture reject 1763140000000-1a2b3c4d --words "Keep the hat blue."
    Reject {
        /// The proposal's id.
        id: String,
        /// What the agent should change.
        #[arg(long)]
        words: Option<String>,
    },
    /// Put the old picture back after an ACCEPT (the page's PUT THE OLD
    /// ONE BACK). Refused to an agent. Example: gp theme picture undo
    /// 1763140000000-1a2b3c4d
    Undo {
        /// The proposal's id.
        id: String,
    },
}

pub(crate) fn run(cmd: &PictureCmd, paths: &Paths) -> Result<Outcome> {
    match cmd {
        PictureCmd::Propose {
            dir,
            game,
            picture,
            png,
            why,
            rom,
        } => propose(dir, game, picture, png, why, rom.as_deref(), paths),
        PictureCmd::List { theme, all } => list(&paths.home, theme.as_deref(), *all),
        PictureCmd::Accept { id, words } => {
            let v = decide(&paths.home, id, Answer::Accept, words.as_deref())
                .map_err(anyhow::Error::msg)?;
            Ok(Outcome::ok(
                v.clone(),
                format!(
                    "Accepted: {} is in the theme now.",
                    v["picture"].as_str().unwrap_or("")
                ),
            ))
        }
        PictureCmd::Reject { id, words } => {
            let v = decide(&paths.home, id, Answer::Reject, words.as_deref())
                .map_err(anyhow::Error::msg)?;
            Ok(Outcome::ok(
                v,
                "Rejected; your agent hears your words.".into(),
            ))
        }
        PictureCmd::Undo { id } => {
            let v = decide(&paths.home, id, Answer::Undo, None).map_err(anyhow::Error::msg)?;
            Ok(Outcome::ok(
                v.clone(),
                format!(
                    "The old {} is back.",
                    v["picture"].as_str().unwrap_or("picture")
                ),
            ))
        }
    }
}

// ---------------------------------------------------------------------
// The store.

/// The largest PNG a proposal takes.
const MAX_BYTES: u64 = 16 << 20;
/// A picture larger than this on a side some phones' browsers won't draw
/// (theme_look's limit).
const MAX_SIDE: u32 = 4096;
/// How far the picture's shape may differ from a rule's width and height
/// (theme_look's stretch warning, here a refusal).
const STRETCH: f64 = 0.15;
/// How long an answer the agent hasn't read stays in `next`.
const UNREAD_FOR_MS: u64 = 24 * 60 * 60 * 1000;

pub(crate) fn dir(home: &Path) -> PathBuf {
    home.join("work").join("proposals").join("pictures")
}

/// Where the person's own theme copies live.
pub(crate) fn own_themes(home: &Path) -> PathBuf {
    home.join("work").join("themes")
}

fn id_ok(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

fn folder(home: &Path, id: &str) -> Option<PathBuf> {
    id_ok(id).then(|| dir(home).join(id))
}

pub(crate) fn read(home: &Path, id: &str) -> Option<Value> {
    let t = fs::read_to_string(folder(home, id)?.join("proposal.json")).ok()?;
    serde_json::from_str(&t).ok()
}

fn write_atomic(f: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(d) = f.parent() {
        fs::create_dir_all(d)?;
    }
    let name = f
        .file_name()
        .map_or_else(Default::default, |n| n.to_string_lossy().into_owned());
    let tmp = f.with_file_name(format!(".{name}.partial"));
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, f)
}

fn save(home: &Path, v: &Value) -> Result<(), String> {
    let id = v["id"].as_str().unwrap_or_default();
    let d = folder(home, id).ok_or_else(|| format!("{id:?} isn't a proposal's id"))?;
    write_atomic(
        &d.join("proposal.json"),
        serde_json::to_string_pretty(v).expect("JSON").as_bytes(),
    )
    .map_err(|e| format!("writing the proposal {id}: {e}"))
}

/// Every proposal, newest first.
pub(crate) fn all(home: &Path) -> Vec<Value> {
    let mut out: Vec<Value> = fs::read_dir(dir(home))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| fs::read_to_string(e.path().join("proposal.json")).ok())
        .filter_map(|t| serde_json::from_str::<Value>(&t).ok())
        .collect();
    out.sort_by_key(|v| std::cmp::Reverse(v["made_ms"].as_u64().unwrap_or(0)));
    out
}

/// The open proposals, oldest first: the queue the person answers.
pub(crate) fn queue(home: &Path) -> Vec<Value> {
    let mut q: Vec<Value> = all(home)
        .into_iter()
        .filter(|v| v["state"] == json!("open"))
        .collect();
    q.reverse();
    q
}

/// Answers the agent hasn't read yet, newest first.
pub(crate) fn unread(home: &Path) -> Vec<Value> {
    let now = gp_mcp::asks::now_ms();
    all(home)
        .into_iter()
        .filter(|v| v["state"] != json!("open") && v["told_ms"].is_null())
        .filter(|v| now.saturating_sub(v["answered_ms"].as_u64().unwrap_or(0)) < UNREAD_FOR_MS)
        .collect()
}

/// What `next` lists: the queue (each with its place in it) and the
/// answers the agent hasn't read.
pub(crate) fn for_next(home: &Path) -> Vec<Value> {
    let q = queue(home);
    let n = q.len();
    let mut out: Vec<Value> = q
        .into_iter()
        .enumerate()
        .map(|(i, mut v)| {
            v["place"] = json!(i + 1);
            v["of"] = json!(n);
            v
        })
        .collect();
    out.extend(unread(home));
    out
}

fn sha(bytes: &[u8]) -> String {
    hex::encode(sha2::Sha256::digest(bytes))
}

/// A proposal's own picture: `old` or `new`.
pub(crate) fn png(home: &Path, id: &str, which: &str) -> Option<Vec<u8>> {
    if !matches!(which, "old" | "new") {
        return None;
    }
    fs::read(folder(home, id)?.join(format!("{which}.png"))).ok()
}

// ---------------------------------------------------------------------
// Proposing.

/// The theme folder and its theme/ folder (where look.txt is).
pub(crate) fn theme_folders(dir: &Path) -> (PathBuf, PathBuf) {
    if dir.join(LOOK_FILE).is_file() && !dir.join("theme").is_dir() {
        (
            dir.parent().map_or_else(|| dir.to_owned(), Path::to_owned),
            dir.to_owned(),
        )
    } else {
        (dir.to_owned(), dir.join("theme"))
    }
}

/// Every size, in game pixels, a rule of art.txt draws `art` at, with the
/// rule's words.
fn sizes_of(rules: &ArtRules, art: &str) -> Vec<(String, [i64; 2])> {
    let mut out = Vec::new();
    for p in &rules.pics {
        if p.art.art == art {
            out.push((format!("pic {}", p.build), p.art.size));
        }
    }
    for t in &rules.timed {
        if t.art.art == art {
            out.push((format!("timed {}", t.build), t.art.size));
        }
    }
    if let Some(n) = &rules.near
        && n.art.art == art
    {
        out.push(("near".into(), n.art.size));
    }
    for p in &rules.props {
        if p.art == art {
            out.push((format!("prop {}", p.level), [p.rect[2], p.rect[3]]));
        }
    }
    if let Some(d) = &rules.drift {
        for (a, s) in &d.arts {
            if a == art {
                out.push(("drift_art".into(), [s[0] / 16, s[1] / 16]));
            }
        }
    }
    if let Some(st) = &rules.stage {
        for (name, a, s) in &st.pieces {
            if a == art {
                out.push((format!("stage piece {name}"), *s));
            }
        }
    }
    out
}

/// The problems `gp theme look check` reports, as (group, file, message).
pub(crate) fn problems(o: &Outcome) -> Vec<(String, String, String)> {
    o.json["findings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|f| f["level"] == json!("problem"))
        .map(|f| {
            (
                f["group"].as_str().unwrap_or("").to_owned(),
                f["file"].as_str().unwrap_or("").to_owned(),
                f["message"].as_str().unwrap_or("").to_owned(),
            )
        })
        .collect()
}

/// A copy of a folder, file by file (a clone where the disk can).
pub(crate) fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for e in fs::read_dir(from)? {
        let e = e?;
        let name = e.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        let p = e.path();
        if p.is_dir() {
            copy_tree(&p, &to.join(&name))?;
        } else {
            fs::copy(&p, to.join(&name))?;
        }
    }
    Ok(())
}

fn refused(why: impl Into<String>, extra: Value) -> Outcome {
    let why = why.into();
    let mut j = json!({"ok": false, "refused": why});
    if let Value::Object(m) = extra {
        for (k, v) in m {
            j[k] = v;
        }
    }
    Outcome {
        text: format!("Not proposed: {why}"),
        json: j,
        exit: 1,
    }
}

fn propose(
    dir_arg: &Path,
    game: &Path,
    picture: &str,
    png_path: &Path,
    why: &str,
    rom: Option<&Path>,
    paths: &Paths,
) -> Result<Outcome> {
    let home = &paths.home;
    let why = why.trim();
    if why.is_empty() {
        return Ok(refused(
            "--why is empty. Say in one sentence why this picture, for the person.",
            json!({}),
        ));
    }
    if why.len() > 400 {
        return Ok(refused(
            "--why is longer than one sentence (400 characters at most).",
            json!({}),
        ));
    }
    // The person's own copy, nowhere else.
    let Ok(dir) = dir_arg.canonicalize() else {
        bail!("{} isn't a folder", dir_arg.display());
    };
    let own = own_themes(home);
    let own = own.canonicalize().unwrap_or(own);
    if !dir.starts_with(&own) || dir == own {
        return Ok(refused(
            format!(
                "{} isn't the person's own copy of a theme. Proposals change only a theme folder under {}: copy the theme there first and propose on the copy.",
                dir.display(),
                own.display()
            ),
            json!({}),
        ));
    }
    let (root, theme) = theme_folders(&dir);
    let look = fs::read_to_string(theme.join(LOOK_FILE)).map_err(|_| {
        anyhow::anyhow!(
            "{} has no theme/look.txt: it isn't a large-art theme",
            root.display()
        )
    })?;
    let look = Look::read(&look).map_err(|e| anyhow::anyhow!("look.txt: {e}"))?;
    let Some(art_file) = look.art.clone() else {
        return Ok(refused(
            "the theme's look.txt names no art file (`art art.txt`), so it has no pictures to replace.",
            json!({}),
        ));
    };
    let rules_text = fs::read_to_string(theme.join(&art_file))
        .with_context(|| format!("reading {}", theme.join(&art_file).display()))?;
    let rules = ArtRules::read(&rules_text).map_err(|e| anyhow::anyhow!("{art_file}: {e}"))?;

    // Which art file.
    let files = rules.files();
    let art = if files.iter().any(|f| f == picture) {
        picture.to_owned()
    } else if let Some(p) = rules.pics.iter().find(|p| p.build == picture) {
        p.art.art.clone()
    } else {
        return Ok(refused(
            format!(
                "{art_file} names no picture {picture:?}. Give its path in the theme folder as {art_file} has it (such as {}), or a game picture a `pic` line draws.",
                files.first().map_or("art/hero.png", String::as_str)
            ),
            json!({}),
        ));
    };
    let target = theme.join(&art);
    let Ok(old) = fs::read(&target) else {
        return Ok(refused(
            format!(
                "{art} isn't in the theme yet. A proposal replaces a picture the theme has; draw the first one into the theme folder yourself."
            ),
            json!({}),
        ));
    };

    // The new picture.
    let meta = fs::metadata(png_path).with_context(|| format!("reading {}", png_path.display()))?;
    if meta.len() > MAX_BYTES {
        return Ok(refused(
            format!(
                "{} is {} bytes; a picture here is at most 16 MB.",
                png_path.display(),
                meta.len()
            ),
            json!({}),
        ));
    }
    let new = fs::read(png_path).with_context(|| format!("reading {}", png_path.display()))?;
    if !new.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Ok(refused(
            format!("{} isn't a PNG file.", png_path.display()),
            json!({}),
        ));
    }
    let img = match Rgba::from_png(&new) {
        Ok(i) => i,
        Err(e) => {
            return Ok(refused(
                format!("{} isn't a PNG that can be read ({e}).", png_path.display()),
                json!({}),
            ));
        }
    };
    if !Path::new(&art)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("png"))
    {
        return Ok(refused(
            format!(
                "{art} isn't a PNG in the theme, and a proposal brings a PNG. Change the rule in {art_file} by hand first."
            ),
            json!({}),
        ));
    }
    if new == old {
        return Ok(refused(
            format!("the new picture is the same as {art}, byte for byte."),
            json!({}),
        ));
    }
    if img.width > MAX_SIDE || img.height > MAX_SIDE || img.width == 0 || img.height == 0 {
        return Ok(refused(
            format!(
                "the picture is {} by {} pixels; it must be 1 to {MAX_SIDE} on a side.",
                img.width, img.height
            ),
            json!({}),
        ));
    }
    let sizes = sizes_of(&rules, &art);
    let own_shape = f64::from(img.width) / f64::from(img.height);
    for (rule, [w, h]) in &sizes {
        if *w <= 0 || *h <= 0 {
            continue;
        }
        let drawn = *w as f64 / *h as f64;
        if (own_shape / drawn - 1.0).abs() > STRETCH {
            return Ok(refused(
                format!(
                    "the picture is {} by {} pixels, but {art_file}'s `{rule}` draws {art} {w} by {h} game pixels, so it would be stretched. Draw it in that shape (at scale {}: {} by {}).",
                    img.width,
                    img.height,
                    rules.scale.max(1),
                    *w * i64::from(rules.scale.max(1)),
                    *h * i64::from(rules.scale.max(1)),
                ),
                json!({}),
            ));
        }
    }
    let old_px = Rgba::from_png(&old).map(|i| [i.width, i.height]).ok();

    // The theme check, before and with the picture in place.
    let before = crate::theme_look::check(&root, game, rom, paths)?;
    let scratch_parent = dir_for_checks(home)?;
    let scratch = tempfile::Builder::new()
        .prefix("check-")
        .tempdir_in(&scratch_parent)
        .context("making a folder to check the theme in")?;
    let copy = scratch
        .path()
        .join(root.file_name().unwrap_or_else(|| "theme".as_ref()));
    copy_tree(&root, &copy).context("copying the theme to check it")?;
    let copy_theme = copy.join(theme.strip_prefix(&root).unwrap_or(Path::new("theme")));
    fs::write(copy_theme.join(&art), &new).context("putting the picture in the copy")?;
    let after = crate::theme_look::check(&copy, game, rom, paths)?;
    let had = problems(&before);
    let new_problems: Vec<(String, String, String)> = problems(&after)
        .into_iter()
        .filter(|p| !had.contains(p))
        .collect();
    if !new_problems.is_empty() {
        let first = &new_problems[0];
        let rom_data = new_problems.iter().any(|p| p.0 == "rom_data");
        let lead = if rom_data {
            "the picture holds data from the ROM, and a theme may hold nothing from it"
        } else {
            "`gp theme look check` finds a problem with the picture in place that the theme didn't have"
        };
        return Ok(refused(
            format!(
                "{lead}: {}{}. Fix it and propose again.",
                if first.1.is_empty() {
                    String::new()
                } else {
                    format!("{}: ", first.1)
                },
                first.2
            ),
            json!({"new_problems": new_problems.iter().map(|p| json!({"group": p.0, "file": p.1, "message": p.2})).collect::<Vec<_>>()}),
        ));
    }
    let rom_note = after.json["findings"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|f| f["group"] == json!("rom_data"))
        .map(|f| {
            if f["level"] == json!("ok") {
                "compared with the ROM: holds none of its data".to_owned()
            } else {
                format!(
                    "not compared with the ROM: {}",
                    f["message"].as_str().unwrap_or("")
                )
            }
        })
        .unwrap_or_else(|| "not compared with the ROM".into());
    drop(scratch);

    // Kept.
    let made = gp_mcp::asks::now_ms();
    let new_sha = sha(&new);
    let id = format!("{made}-{}", &new_sha[..8]);
    let f = self::dir(home).join(&id);
    fs::create_dir_all(&f)?;
    write_atomic(&f.join("new.png"), &new)?;
    write_atomic(&f.join("old.png"), &old)?;
    let game_name = crate::theme_kit::template_dir(game)
        .ok()
        .and_then(|d| d.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| game.display().to_string());
    let v = json!({
        "id": id,
        "theme": dir.display().to_string(),
        "theme_dir": theme.display().to_string(),
        "title": look.title,
        "game": game_name,
        "picture": art,
        "rules": sizes.iter().map(|(r, s)| json!({"rule": r, "size": s})).collect::<Vec<_>>(),
        "size": sizes.first().map(|(_, s)| json!(s)),
        "scale": rules.scale.max(1),
        "why": why,
        "made_ms": made,
        "state": "open",
        "old_sha": sha(&old),
        "new_sha": new_sha,
        "old_px": old_px,
        "new_px": [img.width, img.height],
        "checked": {
            "problems_before": had.len(),
            "problems_after": problems(&after).len(),
            "rom": rom_note,
        },
    });
    save(home, &v).map_err(anyhow::Error::msg)?;
    let page = gp_mcp::asks::page_served(home);
    let place = queue(home).len();
    let tell = if page {
        "It waits on the person's page as a card with the old and new pictures, ACCEPT and REJECT. Tell them in one sentence what you changed and that the card is waiting; then call next with wait_s until waits_for_yes shows it answered, and read their answer with gp theme picture list."
    } else {
        "Their page isn't open: ask them to open it (gp serve), where it waits as a card with ACCEPT and REJECT, then call next with wait_s and read their answer with gp theme picture list."
    };
    let mut out = v.clone();
    out["ok"] = json!(true);
    out["page_open"] = json!(page);
    out["waiting"] = json!(place);
    out["next"] = json!(tell);
    let text = format!(
        "Proposed {id}: {art} in {} ({} of the open proposals). Checked: the shape fits {}, the theme check finds no new problem, {}.\n{tell}",
        look.title,
        place,
        art_file,
        v["checked"]["rom"].as_str().unwrap_or("")
    );
    Ok(Outcome::ok(out, text))
}

/// A scratch folder for the theme checks, under the proposals.
pub(crate) fn dir_for_checks(home: &Path) -> Result<PathBuf> {
    let d = dir(home).join(".checks");
    fs::create_dir_all(&d).with_context(|| format!("making {}", d.display()))?;
    Ok(d)
}

// ---------------------------------------------------------------------
// The person's answer.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Answer {
    Accept,
    Reject,
    Undo,
}

/// The person's ACCEPT, REJECT or PUT THE OLD ONE BACK on a proposal.
pub(crate) fn decide(
    home: &Path,
    id: &str,
    a: Answer,
    words: Option<&str>,
) -> Result<Value, String> {
    let mut v = read(home, id).ok_or_else(|| format!("there's no proposal {id}"))?;
    let theme_dir = PathBuf::from(v["theme_dir"].as_str().unwrap_or_default());
    let pic = v["picture"].as_str().unwrap_or_default().to_owned();
    if pic.is_empty() || pic.contains("..") || Path::new(&pic).is_absolute() {
        return Err(format!("the proposal {id} names no picture in its theme"));
    }
    let target = theme_dir.join(&pic);
    let now = gp_mcp::asks::now_ms();
    let words = words.map(str::trim).filter(|w| !w.is_empty()).map(|w| {
        let mut w = w.to_owned();
        w.truncate(2000);
        w
    });
    let state = v["state"].as_str().unwrap_or("").to_owned();
    let current = fs::read(&target).ok().map(|b| sha(&b));
    match a {
        Answer::Accept | Answer::Reject if state != "open" => {
            return Err(format!("this proposal was answered already ({state})"));
        }
        Answer::Accept => {
            if current.as_deref() != v["old_sha"].as_str() {
                v["state"] = json!("stale");
                v["answered_ms"] = json!(now);
                v["words"] = json!(
                    "The picture in the theme changed after this was proposed, so it wasn't replaced."
                );
                save(home, &v)?;
                return Err(format!(
                    "{pic} changed in the theme after your agent proposed this, so it wasn't replaced. Ask your agent to propose it again."
                ));
            }
            let new = png(home, id, "new").ok_or("the proposed picture is missing")?;
            write_atomic(&target, &new)
                .map_err(|e| format!("writing {}: {e}", target.display()))?;
            v["state"] = json!("accepted");
        }
        Answer::Reject => {
            v["state"] = json!("rejected");
        }
        Answer::Undo => {
            if state != "accepted" {
                return Err(format!(
                    "only an accepted picture can be put back (this one is {state})"
                ));
            }
            if current.as_deref() != v["new_sha"].as_str() {
                return Err(format!(
                    "{pic} changed in the theme since you accepted it, so the old one wasn't put back over it."
                ));
            }
            let old = png(home, id, "old").ok_or("the old picture is missing")?;
            write_atomic(&target, &old)
                .map_err(|e| format!("writing {}: {e}", target.display()))?;
            v["state"] = json!("undone");
            v["undone_ms"] = json!(now);
        }
    }
    if a != Answer::Undo {
        v["words"] = json!(words);
    }
    v["answered_ms"] = json!(now);
    // The agent hears each answer, the undo too.
    v["told_ms"] = Value::Null;
    save(home, &v)?;
    Ok(v)
}

/// How a proposal stands, in words for the agent.
fn says(v: &Value) -> String {
    let pic = v["picture"].as_str().unwrap_or("?");
    let words = v["words"]
        .as_str()
        .map(|w| format!(" Their words: \"{w}\""))
        .unwrap_or_default();
    match v["state"].as_str().unwrap_or("") {
        "open" => format!("{pic}: waiting for the person's answer."),
        "accepted" => format!("{pic}: the person accepted it; it is in the theme.{words}"),
        "rejected" => format!("{pic}: the person rejected it.{words}"),
        "undone" => format!("{pic}: the person accepted it, then put the old one back."),
        "stale" => format!(
            "{pic}: not used, the picture changed in the theme after it was proposed. Propose again if it still fits."
        ),
        s => format!("{pic}: {s}"),
    }
}

fn list(home: &Path, theme: Option<&Path>, every: bool) -> Result<Outcome> {
    let theme = theme.map(|t| t.canonicalize().unwrap_or_else(|_| t.to_owned()));
    let now = gp_mcp::asks::now_ms();
    let mine = |v: &Value| {
        theme
            .as_ref()
            .is_none_or(|t| v["theme"].as_str() == Some(&t.display().to_string()))
    };
    let open: Vec<Value> = queue(home).into_iter().filter(|v| mine(v)).collect();
    let mut answered: Vec<Value> = all(home)
        .into_iter()
        .filter(|v| v["state"] != json!("open") && mine(v))
        .filter(|v| {
            every
                || v["told_ms"].is_null()
                || now.saturating_sub(v["answered_ms"].as_u64().unwrap_or(0)) < UNREAD_FOR_MS
        })
        .collect();
    let mut text = Vec::new();
    for v in &open {
        text.push(format!("{} {}", v["id"].as_str().unwrap_or(""), says(v)));
    }
    for v in &mut answered {
        let fresh = v["told_ms"].is_null();
        text.push(format!(
            "{} {}{}",
            v["id"].as_str().unwrap_or(""),
            if fresh { "NEW ANSWER. " } else { "" },
            says(v)
        ));
        v["says"] = json!(says(v));
        if fresh {
            v["told_ms"] = json!(now);
            let _ = save(home, v);
            v["new_answer"] = json!(true);
        }
    }
    let open: Vec<Value> = open
        .into_iter()
        .map(|mut v| {
            v["says"] = json!(says(&v));
            v
        })
        .collect();
    if text.is_empty() {
        text.push("No picture proposals.".into());
    }
    Ok(Outcome::ok(
        json!({"open": open, "answered": answered}),
        text.join("\n"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_of(w: u32, h: u32, c: [u8; 4]) -> Vec<u8> {
        Rgba {
            width: w,
            height: h,
            pixels: vec![c; (w * h) as usize],
        }
        .png()
    }

    fn stored(home: &Path, theme_dir: &Path) -> String {
        let id = "1-abcdef01";
        let f = dir(home).join(id);
        fs::create_dir_all(&f).unwrap();
        let old = png_of(2, 2, [0, 0, 0, 255]);
        let new = png_of(2, 2, [255, 0, 0, 255]);
        fs::write(f.join("old.png"), &old).unwrap();
        fs::write(f.join("new.png"), &new).unwrap();
        fs::create_dir_all(theme_dir.join("art")).unwrap();
        fs::write(theme_dir.join("art/hero.png"), &old).unwrap();
        save(
            home,
            &json!({"id": id, "theme": "t", "theme_dir": theme_dir.display().to_string(), "picture": "art/hero.png",
                    "made_ms": 1, "state": "open", "old_sha": sha(&old), "new_sha": sha(&new), "why": "red"}),
        )
        .unwrap();
        id.into()
    }

    #[test]
    fn accept_puts_it_in_and_undo_puts_the_old_one_back() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("home");
        let t = d.path().join("theme");
        let id = stored(&home, &t);
        assert_eq!(queue(&home).len(), 1);
        assert!(unread(&home).is_empty());
        let v = decide(&home, &id, Answer::Accept, Some("nice")).unwrap();
        assert_eq!(v["state"], "accepted");
        assert_eq!(
            fs::read(t.join("art/hero.png")).unwrap(),
            png(&home, &id, "new").unwrap()
        );
        assert!(
            decide(&home, &id, Answer::Reject, None).is_err(),
            "answered once"
        );
        assert!(queue(&home).is_empty());
        assert_eq!(unread(&home).len(), 1);
        // The agent reads it, so next stops showing it.
        let l = list(&home, None, false).unwrap();
        assert_eq!(l.json["answered"][0]["new_answer"], true);
        assert!(l.text.contains("nice"));
        assert!(unread(&home).is_empty());
        decide(&home, &id, Answer::Undo, None).unwrap();
        assert_eq!(
            fs::read(t.join("art/hero.png")).unwrap(),
            png(&home, &id, "old").unwrap()
        );
        assert_eq!(unread(&home).len(), 1, "the agent hears the undo too");
        assert!(decide(&home, &id, Answer::Undo, None).is_err(), "one level");
    }

    #[test]
    fn reject_keeps_the_theme_and_carries_the_words() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("home");
        let t = d.path().join("theme");
        let id = stored(&home, &t);
        let before = fs::read(t.join("art/hero.png")).unwrap();
        let v = decide(&home, &id, Answer::Reject, Some("  keep the hat blue  ")).unwrap();
        assert_eq!(v["words"], "keep the hat blue");
        assert_eq!(fs::read(t.join("art/hero.png")).unwrap(), before);
        assert!(says(&v).contains("keep the hat blue"));
        assert!(decide(&home, &id, Answer::Undo, None).is_err());
    }

    #[test]
    fn a_picture_changed_since_is_not_replaced() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("home");
        let t = d.path().join("theme");
        let id = stored(&home, &t);
        fs::write(t.join("art/hero.png"), png_of(2, 2, [0, 255, 0, 255])).unwrap();
        let e = decide(&home, &id, Answer::Accept, None).unwrap_err();
        assert!(e.contains("changed"), "{e}");
        assert_eq!(read(&home, &id).unwrap()["state"], "stale");
        assert!(queue(&home).is_empty());
    }

    #[test]
    fn ids_and_pictures_stay_in_the_store() {
        let d = tempfile::tempdir().unwrap();
        assert!(read(d.path(), "../x").is_none());
        assert!(png(d.path(), "1-a", "../old").is_none());
        assert!(!id_ok("a/b") && id_ok("1763-ab12"));
    }

    #[test]
    fn sizes_come_from_every_rule_that_draws_the_art() {
        let r = ArtRules::read(
            "gp-art 1\nscale 4\npic frame-0-0.png art/a.png 16 16 0 0\ntimed frame-1-0.png 0 9 art/a.png 16 24 0 0\n",
        )
        .unwrap();
        let s = sizes_of(&r, "art/a.png");
        assert_eq!(s.len(), 2);
        assert_eq!(s[1].1, [16, 24]);
    }
}
