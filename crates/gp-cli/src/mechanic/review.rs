//! The review of a clip on the model, for the mechanic page's Review
//! panel: the clip's presses played through the original and the game's
//! module in the lockstep `gp clip score` runs straight through, with the
//! original's random draws, and each compared frame's two pictures kept,
//! the original's from the emulator and the model's drawn by the tool's own
//! raster (crate::screen). The page shows them side by side, one over the
//! other, or as their difference, and marks the frames where the mapped
//! state disagrees.
//!
//! Kept in the session beside the clip, `clips/NNN.review/`: `meta.json`
//! and `o-KKKKK.png`, `m-KKKKK.png` a frame. The pictures show the game's
//! own art, so they stay in $GP_HOME with the clip, never in a repo. The
//! review is made again when the model, the mapping or the module changes
//! (`meta.json`'s `key`).
//!
//! The same review is REVIEW REBUILD's clip (`/api/clips/N.rebuild.json`),
//! which game-printer drew from a rebuild of its own: `meta.json` also
//! keeps, for each compared frame, the model's drawn things (its entities'
//! boxes and names, for the outlines), whether it drew a sprite, and the
//! mapped measures the two games disagree on with both values; and the
//! original's random draws the model was given, with the first desync.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use gp_evidence::Paths;
use gp_present::raster::Frame;
use serde_json::{Value, json};

/// What the review of a game's clips needs beside the session.
pub(crate) struct Rebuild {
    /// The game's package folder (games/GAME).
    pub game: PathBuf,
    /// The game's built module, for a model with hooks.
    pub module: Option<PathBuf>,
    /// The ROM set the page plays and its identity, when it isn't the
    /// package's scored one: the review and the rebuild play it.
    pub rom: Option<(std::sync::Arc<Vec<u8>>, String)>,
    /// The model's pictures lifted from that set.
    pub assets: Option<PathBuf>,
}

impl Rebuild {
    /// The module given, else the package's hook crate built from this
    /// gp's kit (crate::module::built_for), so a module built for another
    /// gp or older hook code is never used.
    pub fn new(paths: &Paths, game_dir: &Path, game: &str, module: Option<&Path>) -> Self {
        let _ = game;
        let module = module.map(Path::to_path_buf).or_else(|| {
            match crate::module::built_for(&paths.home, game_dir) {
                Ok(m) => m.map(|m| m.0),
                Err(e) => {
                    eprintln!("{}: the module didn't build: {e:#}", game_dir.display());
                    None
                }
            }
        });
        Self {
            game: game_dir.to_owned(),
            module,
            rom: None,
            assets: None,
        }
    }

    /// The model's pictures: the set's own when lifted, else the scored
    /// set's.
    pub fn assets_for(&self, paths: &Paths, rom: &[u8]) -> Result<PathBuf> {
        match &self.assets {
            Some(a) => Ok(a.clone()),
            None => crate::corpus_score::assets_of(paths, &self.game, &self.game, None, rom),
        }
    }
}

fn dir_of(session: &Path, n: u64) -> PathBuf {
    session.join("clips").join(format!("{n:03}.review"))
}

/// What the review depends on: the model, its mapping and the module, and
/// what `meta.json` holds (its `version`).
fn key(r: &Rebuild) -> Result<String> {
    let mut h = blake3::Hasher::new();
    h.update(b"review 2");
    for f in [gp_model::MODEL_FILE, gp_interp::mapping::MAPPING_FILE] {
        let p = r.game.join(f);
        h.update(&fs::read(&p).with_context(|| format!("reading {}", p.display()))?);
    }
    if let Some(m) = &r.module {
        h.update(&fs::read(m).with_context(|| format!("reading {}", m.display()))?);
    }
    if let Some((_, sha)) = &r.rom {
        h.update(sha.as_bytes());
    }
    Ok(h.finalize().to_hex().to_string())
}

fn png(f: &Frame) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut e = png::Encoder::new(&mut out, f.w, f.h);
        e.set_color(png::ColorType::Rgb);
        e.set_depth(png::BitDepth::Eight);
        let mut w = e.write_header()?;
        let flat: Vec<u8> = f.rgb.iter().flatten().copied().collect();
        w.write_image_data(&flat)?;
    }
    Ok(out)
}

/// The part of the screen both pictures draw: their `visible` boxes
/// (left, top, width, height) overlapped.
fn both_draw(a: [u32; 4], b: [u32; 4]) -> [u32; 4] {
    let (l, t) = (a[0].max(b[0]), a[1].max(b[1]));
    let (r, bt) = (
        (a[0] + a[2]).min(b[0] + b[2]),
        (a[1] + a[3]).min(b[1] + b[3]),
    );
    [l, t, r.saturating_sub(l), bt.saturating_sub(t)]
}

/// The review of clip `n` of a session: made if it isn't kept for this
/// model, mapping and module. Its `meta.json`.
pub(crate) fn review(paths: &Paths, r: &Rebuild, session: &Path, n: u64) -> Result<Value> {
    let dir = dir_of(session, n);
    let key = key(r)?;
    if let Ok(t) = fs::read_to_string(dir.join("meta.json"))
        && let Ok(v) = serde_json::from_str::<Value>(&t)
        && v["key"] == json!(key)
    {
        return Ok(v);
    }
    let (text, start) = super::paths::full_log(&paths.home, session, n)?;
    let log = gp_evidence::InputLog::parse(&text).map_err(|e| anyhow::anyhow!("clip {n}: {e}"))?;
    let rom = match &r.rom {
        Some((bytes, _)) => bytes.to_vec(),
        None => crate::clip_score::rom_of(paths, &r.game, None)?,
    };
    let assets = r.assets_for(paths, &rom)?;
    let ctx = crate::clip_score::Ctx::open(
        paths,
        &r.game,
        &crate::clip_score::Open {
            module: r.module.as_deref(),
            model: None,
            assets: Some(&assets),
            rom: None,
        },
    )
    .map_err(|e| match &r.module {
        Some(_) => e,
        None => e.context(format!(
            "with no module: build it with `gp game kit KIT` and `gp game build {} --kit KIT --out {}`, then start the page again (or give --module)",
            r.game.display(),
            paths.home.join("work/modules/GAME.wasm").display()
        )),
    })?;

    // Made in a folder beside, then put in place, so a half-made review is
    // never read.
    let tmp = session
        .join("clips")
        .join(format!(".{n:03}.review.partial"));
    if tmp.exists() {
        fs::remove_dir_all(&tmp)?;
    }
    fs::create_dir_all(&tmp)?;
    // Frames seen, which have pictures, the size, the part both draw (left,
    // top, width, height: the screen score compares only that), and an
    // error writing them.
    type Shots = (
        usize,
        Vec<bool>,
        Option<(u32, u32)>,
        Option<[u32; 4]>,
        Option<String>,
        Vec<Value>,
    );
    let shots: std::rc::Rc<std::cell::RefCell<Shots>> = Default::default();
    let keep = {
        let shots = shots.clone();
        let tmp = tmp.clone();
        Box::new(move |pair: Option<(&Frame, &Frame)>| {
            let mut s = shots.borrow_mut();
            let k = s.0;
            s.0 += 1;
            let Some((o, m)) = pair else {
                s.1.push(false);
                s.5.push(json!({"boxes": [], "sprites": false}));
                return;
            };
            // The model's entities as it drew them, for the page's
            // outlines: its bodies on the screen, not the HUD's fields (named
            // "the score", or their text in quotes) or a body with no
            // picture there.
            let (w, h) = (i64::from(m.w), i64::from(m.h));
            let boxes: Vec<Value> = m
                .things
                .iter()
                .filter(|t| {
                    !t.name.is_empty() && !t.name.starts_with("the ") && !t.name.starts_with('"')
                })
                .filter(|t| t.rect[2] > 1 && t.rect[3] > 1 && t.rect[0] < w && t.rect[1] < h)
                .map(|t| json!([t.rect[0], t.rect[1], t.rect[2], t.rect[3], t.name]))
                .collect();
            s.5.push(json!({"boxes": boxes, "sprites": m.sprite.iter().any(|&x| x)}));
            if s.4.is_some() {
                s.1.push(false);
                return;
            }
            let wrote = png(o)
                .and_then(|b| Ok(fs::write(tmp.join(format!("o-{k:05}.png")), b)?))
                .and_then(|()| png(m))
                .and_then(|b| Ok(fs::write(tmp.join(format!("m-{k:05}.png")), b)?));
            match wrote {
                Ok(()) => {
                    s.1.push(true);
                    s.2.get_or_insert((o.w, o.h));
                    s.3.get_or_insert(both_draw(o.visible, m.visible));
                }
                Err(e) => {
                    s.1.push(false);
                    s.4 = Some(format!("{e:#}"));
                }
            }
        }) as crate::screen::Keep
    };
    let (ls, report, draws) = crate::clip_score::pictures(&ctx, &log, start, keep, true)?;
    let (count, have, size, visible, err, drawn) = shots.borrow().clone();
    if let Some(e) = err {
        bail!("writing clip {n}'s pictures: {e}");
    }
    let together = gp_interp::lockstep::together(&ls);
    // What the two games disagree on at each compared frame, measure by
    // measure, with both values (at most 16 a frame).
    let diffs: Vec<Value> = (0..ls.frames.len())
        .map(|k| {
            let Some((o, m)) = ls.values.get(k) else {
                return json!([]);
            };
            let unmeasured = ls.unmeasured.get(k);
            Value::from(
                ls.paths
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| !unmeasured.and_then(|u| u.get(*i)).copied().unwrap_or(false))
                    .filter(|(_, p)| p.distance.get(k).is_some_and(|d| *d > p.tolerance))
                    .filter_map(|(i, p)| Some(json!([p.path, shown(o.get(i)?), shown(m.get(i)?)])))
                    .take(16)
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    let mut note: Vec<String> = Vec::new();
    if draws.is_none() {
        note.push("the mapping gives the model no random draws: it rolls its own".to_owned());
    }
    if !drawn.iter().any(|d| d["sprites"] == json!(true)) {
        note.push(
            "it draws no sprites on these frames; outline its entities to see them".to_owned(),
        );
    }
    let desync = &report["draws"]["desync"];
    if let Some(f) = desync["original_frame"].as_u64() {
        note.push(format!(
            "the model's random draws left the original's at frame {f}: {}",
            desync["why"].as_str().unwrap_or("")
        ));
    }
    let draws = draws.map(|d| {
        json!({
            "list": d.draws.iter().map(|x| json!([x.frame, x.site, x.value])).collect::<Vec<_>>(),
            "taken": report["draws"]["taken"],
            "desync": if desync.is_null() { Value::Null } else { json!({"frame": desync["original_frame"], "why": desync["why"]}) },
        })
    });
    let loaded = ls.start.as_ref().map(|s| {
        let missing = s.notes.iter().filter(|x| x.how.name() == "missing").count();
        json!({"frame": s.original_frame, "notes": s.notes.len(), "missing": missing})
    });
    let (w, h) = size.unwrap_or((256, 240));
    let meta = json!({
        "key": key,
        "clip": n,
        "start": start,
        "frames": count,
        "w": w,
        "h": h,
        "visible": visible.unwrap_or([0, 0, w, h]),
        "have": have,
        "together": together,
        "together_count": together.iter().filter(|t| **t).count(),
        "original_frames": ls.frames.iter().map(|f| f.1).collect::<Vec<_>>(),
        "module": r.module.as_ref().map(|m| m.display().to_string()),
        "drawn": drawn,
        "diffs": diffs,
        "draws": draws,
        "note": note,
        "loaded": loaded,
    });
    fs::write(tmp.join("meta.json"), serde_json::to_string(&meta)?)?;
    if dir.exists() {
        let old = session.join("clips").join(format!(".{n:03}.review.old"));
        if old.exists() {
            fs::remove_dir_all(&old)?;
        }
        fs::rename(&dir, &old)?;
        fs::remove_dir_all(&old)?;
    }
    fs::rename(&tmp, &dir)?;
    Ok(meta)
}

/// A review as REVIEW REBUILD reads it (`/api/clips/N.rebuild.json`): a
/// frame each, with the original's frame it stands at, the addresses of
/// its two pictures (none on a frame the model drew nothing for), whether
/// the mapped state agrees there, the model's entities and what differs.
pub(crate) fn as_rebuild(meta: &Value, n: u64, mechanic: &str) -> Value {
    let q = format!("?mechanic={mechanic}");
    let at = |k: usize, key: &str| meta[key].get(k).cloned().unwrap_or(Value::Null);
    let count = meta["frames"].as_u64().unwrap_or(0) as usize;
    let frames: Vec<Value> = (0..count)
        .map(|k| {
            let have = meta["have"][k] == json!(true);
            let pic =
                |kind: &str| have.then(|| format!("api/clips/{n}/review/{kind}-{k:05}.png{q}"));
            let drawn = at(k, "drawn");
            json!({
                "k": k,
                "f": at(k, "original_frames"),
                "o": pic("o"),
                "m": pic("m"),
                "together": at(k, "together"),
                "boxes": drawn["boxes"],
                "sprites": drawn["sprites"],
                "diffs": at(k, "diffs"),
            })
        })
        .collect();
    json!({
        "clip": n,
        "mechanic": mechanic,
        "start": meta["start"],
        "w": meta["w"],
        "h": meta["h"],
        "visible": meta["visible"],
        "together_count": meta["together_count"],
        "frames": frames,
        "draws": meta["draws"],
        "note": meta["note"],
        "loaded": meta["loaded"],
    })
}

/// A measure's value as a person reads it: a fixed-point number in pixels
/// (text for the page only; nothing computes with it).
fn shown(v: &gp_runtime::Value) -> String {
    match v {
        gp_runtime::Value::Fixed(r) => {
            let x = *r as f64 / 65536.0;
            let t = format!("{x:.2}");
            t.trim_end_matches('0').trim_end_matches('.').to_owned()
        }
        other => other.to_string(),
    }
}

/// A kept picture of clip `n`'s review: `o-KKKKK.png` or `m-KKKKK.png`.
pub(crate) fn picture(session: &Path, n: u64, name: &str) -> Option<Vec<u8>> {
    let ok = name.len() == 11
        && (name.starts_with("o-") || name.starts_with("m-"))
        && name.ends_with(".png")
        && name[2..7].bytes().all(|b| b.is_ascii_digit());
    if !ok {
        return None;
    }
    fs::read(dir_of(session, n).join(name)).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_reviews_own_pictures_are_read() {
        let d = tempfile::tempdir().unwrap();
        let dir = dir_of(d.path(), 1);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("o-00000.png"), b"png").unwrap();
        fs::write(dir.join("meta.json"), b"{}").unwrap();
        assert_eq!(picture(d.path(), 1, "o-00000.png").unwrap(), b"png");
        for bad in [
            "meta.json",
            "../001.log",
            "o-0000a.png",
            "x-00000.png",
            "o-00000.png/",
        ] {
            assert!(picture(d.path(), 1, bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn the_part_both_draw_is_their_overlap() {
        assert_eq!(
            both_draw([0, 0, 256, 240], [0, 8, 256, 224]),
            [0, 8, 256, 224]
        );
        assert_eq!(
            both_draw([8, 0, 248, 240], [0, 8, 256, 224]),
            [8, 8, 248, 224]
        );
        assert_eq!(both_draw([0, 0, 10, 10], [20, 20, 5, 5]), [20, 20, 0, 0]);
    }

    #[test]
    fn a_picture_is_a_png_of_its_size() {
        let f = Frame {
            w: 2,
            h: 1,
            rgb: vec![[255, 0, 0], [0, 0, 255]],
            sprite: vec![false, false],
            visible: [0, 0, 2, 1],
            things: Vec::new(),
        };
        let b = png(&f).unwrap();
        assert_eq!(&b[1..4], b"PNG");
        let d = png::Decoder::new(std::io::Cursor::new(&b))
            .read_info()
            .unwrap();
        assert_eq!((d.info().width, d.info().height), (2, 1));
    }
}
