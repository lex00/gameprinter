//! Approved looks: when the owner says yes to a look (a quest-log row's
//! clip and frames, judged on the screens), the frames' pictures are locked
//! as they were then, and the model gate holds every change to them
//! (docs/gate.md, "Approved looks").
//!
//! A lock is a table of the game's `approved.toml`: the corpus clip, its
//! frames, the look (the game's own pictures), and each frame's pixels
//! apart from the original then, by the screen measure (gp_interp::screen,
//! as `gp picture` draws it). The gate plays the clip again on the patched
//! game and fails the change when a locked frame is further apart on either
//! layer, or is no longer drawn. Only the owner moves a lock: `gp mechanic
//! approve` again, on their yes.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::Outcome;
use crate::picture::Apart;

/// The file in a game's folder holding its approved looks.
pub(crate) const FILE: &str = "approved.toml";

/// What a failing lock tells the person to do.
const AGAIN: &str = "it changes only on the owner's yes again (gp mechanic approve)";

/// One approved look.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Look {
    /// The quest-log row the owner said yes on.
    pub row: String,
    /// The corpus clip's id.
    pub clip: String,
    /// The first and last frame, from the clip's start.
    pub frames: [u64; 2],
    /// The pictures it was judged in: `game`, the game's own.
    pub look: String,
    /// The day of the yes.
    pub approved: String,
    /// Each locked frame: the frame, the sprites' and the background's
    /// pixels apart from the original when approved.
    pub apart: Vec<[u64; 3]>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(default)]
    look: Vec<Look>,
}

/// The game's approved looks; none without the file.
pub(crate) fn read(game: &Path) -> std::result::Result<Vec<Look>, String> {
    let Ok(text) = fs::read_to_string(game.join(FILE)) else {
        return Ok(Vec::new());
    };
    parse(&text)
}

/// The looks of an `approved.toml` text.
pub(crate) fn parse(text: &str) -> std::result::Result<Vec<Look>, String> {
    let f: File = match toml::from_str(text) {
        Ok(f) => f,
        Err(e) => return Err(format!("{FILE}: {}", e.message())),
    };
    for (k, l) in f.look.iter().enumerate() {
        let (n, look) = (k + 1, &l.look);
        if look != "game" {
            return Err(format!(
                "{FILE}: look {n} is in {look}: only the game's own pictures are locked yet"
            ));
        }
        if l.apart.is_empty() {
            return Err(format!("{FILE}: look {n} locks no frame"));
        }
    }
    Ok(f.look)
}

/// The file's text, one table a look, a frame a line.
pub(crate) fn text(looks: &[Look]) -> String {
    let mut s = String::new();
    s.push_str("# Looks the owner approved: the gate fails a change that draws a locked\n");
    s.push_str("# frame further from the original (docs/gate.md, \"Approved looks\").\n");
    s.push_str("# apart: frame, sprites' and background's pixels apart when approved.\n");
    let q = |x: &str| toml::Value::String(x.to_owned()).to_string();
    for l in looks {
        let [a, b] = l.frames;
        s.push_str("\n[[look]]\n");
        let _ = writeln!(s, "row = {}", q(&l.row));
        let _ = writeln!(s, "clip = {}", q(&l.clip));
        let _ = writeln!(s, "frames = [{a}, {b}]");
        let _ = writeln!(s, "look = {}", q(&l.look));
        let _ = writeln!(s, "approved = {}", q(&l.approved));
        s.push_str("apart = [\n");
        for [f, sp, bg] in &l.apart {
            let _ = writeln!(s, "  [{f}, {sp}, {bg}],");
        }
        s.push_str("]\n");
    }
    s
}

/// How `now` holds a look: None when every locked frame is drawn and no
/// further apart on either layer, else the first frame that isn't, in words.
pub(crate) fn worse(l: &Look, now: &[[u64; 3]]) -> Option<String> {
    let at: BTreeMap<u64, [u64; 2]> = now.iter().map(|[f, s, b]| (*f, [*s, *b])).collect();
    let (clip, day, row, [a, z]) = (&l.clip, &l.approved, &l.row, l.frames);
    let what = format!("{clip}: the look approved {day} on {row} (frames {a}-{z})");
    for [f, s, b] in &l.apart {
        let Some([ns, nb]) = at.get(f) else {
            return Some(format!(
                "{what}: frame {f} isn't compared any more; {AGAIN}"
            ));
        };
        for (layer, was, is) in [("sprites", s, ns), ("background", b, nb)] {
            if is > was {
                return Some(format!(
                    "{what}: frame {f}'s {layer} are {is} pixels apart from the original, {was} when approved; {AGAIN}"
                ));
            }
        }
    }
    None
}

/// Each compared frame's pixels apart for one corpus clip of `game`, as
/// `gp picture` draws it, played into the run `run`.
pub(crate) fn measure(
    paths: &gp_evidence::Paths,
    game: &Path,
    module: Option<&Path>,
    assets: Option<&Path>,
    run: &str,
    clip: &str,
) -> Result<Apart> {
    let args = crate::picture::PictureArgs {
        game: game.to_owned(),
        run: run.to_owned(),
        module: module.map(Path::to_owned),
        model: None,
        assets: assets.map(Path::to_owned),
        rom: None,
        clip: Some(clip.to_owned()),
        top: 0,
        shots: None,
        shots_dir: None,
        sprites: None,
        propose: false,
        line: None,
        apart: true,
    };
    let o = crate::picture::run(&args, paths)?;
    let row = &o.json["clips"][clip];
    if let Some(e) = row["error"].as_str() {
        bail!("{clip} can't be played: {e}");
    }
    let apart = serde_json::from_value(row["apart"].clone());
    apart.with_context(|| format!("{clip}: gp picture compared no frame"))
}

/// Every approved look of `game` the game as it is holds worse, in words;
/// a clip that can't be played now fails its looks too.
pub(crate) fn check(
    paths: &gp_evidence::Paths,
    game: &Path,
    module: Option<&Path>,
    assets: Option<&Path>,
    run: &str,
    looks: &[Look],
) -> Vec<String> {
    let mut by: BTreeMap<&str, std::result::Result<Apart, String>> = BTreeMap::new();
    let mut out = Vec::new();
    for l in looks {
        let clip = l.clip.as_str();
        let now = by.entry(clip).or_insert_with(|| {
            let m = measure(paths, game, module, assets, run, clip);
            m.map_err(|e| format!("{e:#}"))
        });
        let row = &l.row;
        match now {
            Ok(a) => out.extend(worse(l, a)),
            Err(e) => out.push(format!(
                "{clip}: the look approved on {row} can't be checked: {e}"
            )),
        }
    }
    out
}

/// A row's clip as a corpus id: its own id, or clip N of a row once it is
/// in the corpus as GAME.ROW.NNN.
fn corpus_id(game: &str, row: &str, clip: &Value, ids: &[String]) -> Result<String> {
    if let Some(id) = clip["id"].as_str() {
        if !ids.iter().any(|i| i == id) {
            bail!("the corpus has no clip {id}");
        }
        return Ok(id.to_owned());
    }
    let Some(n) = clip["n"].as_u64() else {
        bail!("the row's clip has no id and no number");
    };
    let of = clip["mechanic"].as_str().unwrap_or(row);
    let id = format!("{game}.{of}.{n:03}");
    if !ids.contains(&id) {
        bail!(
            "clip {n} of {of} isn't in the corpus (as {id}): import it first (gp corpus import {game}), then approve"
        );
    }
    Ok(id)
}

/// Whether quest-log row `r` asks the owner to confirm a look a yes can
/// lock: it is asking them, with a clip (a corpus id or a number) in the
/// game's own pictures.
pub(crate) fn confirmable(r: &Value) -> bool {
    let c = &r["clip"];
    r["status"] == json!("asking you")
        && c["look"] == json!("game")
        && (c["id"].is_string() || c["n"].is_u64())
}

/// Whether a typed answer is a plain yes to a look: "yes", "y", "looks
/// right", "lock it" and the like, nothing more.
pub(crate) fn says_yes(text: &str) -> bool {
    const YES: &[&str] = &[
        "yes",
        "y",
        "yep",
        "yeah",
        "ok",
        "okay",
        "confirm",
        "confirmed",
        "lock it",
        "it does",
        "it is",
        "looks right",
        "it looks right",
        "looks good",
        "it looks good",
    ];
    let t = text
        .trim()
        .trim_end_matches(['.', '!'])
        .trim()
        .to_lowercase();
    let rest = t.strip_prefix("yes, ").or_else(|| t.strip_prefix("yes "));
    YES.contains(&rest.unwrap_or(&t))
}

/// The owner's yes on row `row` of the quest log in `qf`, a row asking them
/// to confirm a look ([`confirmable`]): the row goes in line with
/// `confirmed` (when, and the words they said), and gp serve's runner
/// locks it with no worker (serve/runner.rs, `run_confirms`). `typed`: the
/// yes was their answer in the row's talk, whose turn has just set the row
/// agent working.
pub(crate) fn confirm(qf: &Path, row: &str, said: &str, typed: bool) -> Result<Value> {
    use crate::mechanic::session::{read_queue, write_queue};
    let mut q = read_queue(qf);
    let Some(r) = q.iter_mut().find(|r| r["mechanic"] == json!(row)) else {
        bail!("the quest log has no row {row}");
    };
    if r["confirmed"].is_object() {
        return Ok(r.clone());
    }
    let mut asked = r.clone();
    if typed && asked["status"] == json!("agent working") {
        asked["status"] = json!("asking you");
    }
    if !confirmable(&asked) {
        bail!(
            "row {row} doesn't ask you to confirm a look (it needs status asking you and a clip in the game's own pictures)"
        );
    }
    let now = gp_mcp::asks::now_ms();
    let Some(o) = r.as_object_mut() else {
        bail!("row {row} isn't a table");
    };
    if !o.contains_key("asked") {
        o.insert("asked".into(), o.get("note").cloned().unwrap_or(json!("")));
    }
    o.insert("status".into(), json!("in line"));
    o.insert("confirmed".into(), json!({"at_ms": now, "said": said}));
    o.insert(
        "note".into(),
        json!("You said yes to this look: the loop locks it now (gp mechanic approve, landed through the model gate), with no worker."),
    );
    o.insert("changed_ms".into(), json!(now));
    o.remove("worker");
    let out = r.clone();
    write_queue(qf, &q)?;
    Ok(out)
}

/// `gp mechanic approve GAME ROW`: locks the row's clip, frames and look as
/// the owner said yes to them, measured on the game as it is now.
pub(crate) fn approve(
    paths: &gp_evidence::Paths,
    game: &str,
    row: &str,
    games: Option<&Path>,
    module: Option<&Path>,
) -> Result<Outcome> {
    use crate::mechanic::session::{queue_file, read_queue};
    let q = read_queue(&queue_file(paths, game)?);
    let Some(r) = q.iter().find(|r| r["mechanic"] == json!(row)) else {
        bail!("{game}'s quest log has no row {row}");
    };
    let Some(clip) = r.get("clip").filter(|c| c.is_object()) else {
        bail!("row {row} has no clip to approve: give it one with gp mechanic queue --clip");
    };
    let look = clip["look"].as_str().unwrap_or("game");
    if look != "game" {
        bail!(
            "row {row}'s look is {look}: only the game's own pictures are locked yet (look game)"
        );
    }
    let games_dir = crate::roms_cmd::games_dir(games);
    let Some(dir) = games_dir.and_then(|g| crate::labels::game_folder(&g, game)) else {
        bail!("no game {game} in the games checkout: set GP_GAMES_DIR");
    };
    let set = crate::clipset::read_game(&dir)?;
    let ids: Vec<String> = set.clips.into_iter().map(|c| c.id).collect();
    let id = corpus_id(game, row, clip, &ids)?;
    let run = format!("approved-{game}");
    let all = measure(paths, &dir, module, None, &run, &id)?;
    let last = all.iter().map(|a| a[0]).max().unwrap_or(0);
    let from = clip["from"].as_u64().unwrap_or(0);
    let to = clip["to"].as_u64().unwrap_or(last);
    let in_range = |a: &[u64; 3]| (from..=to).contains(&a[0]);
    let apart: Vec<[u64; 3]> = all.into_iter().filter(in_range).collect();
    if apart.is_empty() {
        bail!("{id} has no compared frame in {from}-{to} (its last is {last})");
    }
    let mut looks = read(&dir).map_err(anyhow::Error::msg)?;
    let was = looks.iter().position(|l| l.row == row);
    let sp = apart.iter().filter(|a| a[1] == 0).count();
    let bg = apart.iter().filter(|a| a[2] == 0).count();
    let n = apart.len();
    let l = Look {
        row: row.to_owned(),
        clip: id.clone(),
        frames: [from, to],
        look: look.to_owned(),
        approved: crate::game_new::today(),
        apart,
    };
    match was {
        Some(i) => looks[i] = l,
        None => looks.push(l),
    }
    let file = dir.join(FILE);
    fs::write(&file, text(&looks))?;
    let how = match was {
        Some(_) => "approved again",
        None => "approved",
    };
    let shown = file.display();
    let text_out = format!(
        "{how}: {id} frames {from}-{to} for row {row} in {shown}: {n} frames locked (sprites the same as the original on {sp}, background on {bg}); the model gate now fails a change that draws one further apart"
    );
    let v = json!({"row": row, "clip": id, "frames": [from, to], "look": look,
        "frames_locked": n, "sprites_same": sp, "background_same": bg, "replaced": was.is_some()});
    Ok(Outcome::ok(v, text_out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn look() -> Look {
        Look {
            row: "r".into(),
            clip: "g.l1.001".into(),
            frames: [10, 12],
            look: "game".into(),
            approved: "2026-11-20".into(),
            apart: vec![[10, 0, 0], [11, 5, 0], [12, 0, 3]],
        }
    }

    #[test]
    fn the_same_or_closer_holds() {
        let l = look();
        assert_eq!(worse(&l, &[[10, 0, 0], [11, 5, 0], [12, 0, 3]]), None);
        let closer = [[9, 99, 99], [10, 0, 0], [11, 0, 0], [12, 0, 0]];
        assert_eq!(worse(&l, &closer), None);
    }

    #[test]
    fn further_apart_or_gone_fails() {
        let l = look();
        let w = worse(&l, &[[10, 0, 0], [11, 6, 0], [12, 0, 3]]).unwrap();
        assert!(w.contains("frame 11's sprites are 6 pixels apart"), "{w}");
        let w = worse(&l, &[[10, 0, 0], [11, 5, 0], [12, 0, 4]]).unwrap();
        assert!(w.contains("frame 12's background"), "{w}");
        let w = worse(&l, &[[10, 0, 0], [11, 5, 0]]).unwrap();
        assert!(w.contains("frame 12 isn't compared"), "{w}");
    }

    #[test]
    fn the_file_reads_back() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join(FILE);
        assert_eq!(read(d.path()), Ok(vec![]));
        let mut other = look();
        other.row = "s".into();
        let looks = vec![look(), other];
        fs::write(&f, text(&looks)).unwrap();
        assert_eq!(read(d.path()), Ok(looks));
        let mut theme = look();
        theme.look = "last-call".into();
        fs::write(&f, text(&[theme])).unwrap();
        let e = read(d.path()).unwrap_err();
        assert!(e.contains("only the game's own"), "{e}");
    }

    #[test]
    fn a_plain_yes_on_a_look_row_confirms_it() {
        for y in ["yes", "Yes.", "y", "Looks right!", "lock it"] {
            assert!(says_yes(y), "{y}");
        }
        assert!(says_yes("yes, it looks right"));
        for n in ["no", "yes but the mug is off", "not yet", ""] {
            assert!(!says_yes(n), "{n}");
        }
        let row = |s: &str, c: Value| json!({"mechanic": "r", "status": s, "clip": c});
        let asking = |c: Value| confirmable(&row("asking you", c));
        assert!(asking(json!({"n": 1, "look": "game"})));
        assert!(asking(json!({"id": "t.l1.001", "look": "game"})));
        assert!(!confirmable(&row("next", json!({"n": 1, "look": "game"}))));
        assert!(!asking(json!({"n": 1, "look": "last-call"})));
        assert!(!asking(json!({"n": 1})));
        assert!(!asking(Value::Null));
    }

    #[test]
    fn a_row_clip_is_found_in_the_corpus() {
        let ids = vec!["t.l1.001".to_owned(), "t.enemies.002".to_owned()];
        let id = |row: &str, clip: Value| corpus_id("t", row, &clip, &ids);
        assert_eq!(id("x", json!({"id": "t.l1.001"})).unwrap(), "t.l1.001");
        assert_eq!(id("enemies", json!({"n": 2})).unwrap(), "t.enemies.002");
        let other = json!({"n": 2, "mechanic": "enemies"});
        assert_eq!(id("x", other).unwrap(), "t.enemies.002");
        let e = id("x", json!({"n": 1})).unwrap_err();
        assert!(format!("{e}").contains("import it first"), "{e}");
        assert!(id("x", json!({"id": "t.l9.001"})).is_err());
    }
}
