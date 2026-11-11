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
}

impl Rebuild {
    /// The module given, else `$GP_HOME/work/modules/GAME.wasm` if it's
    /// there.
    pub fn new(paths: &Paths, game_dir: &Path, game: &str, module: Option<&Path>) -> Self {
        let module = module.map(Path::to_path_buf).or_else(|| {
            let f = paths
                .home
                .join("work")
                .join("modules")
                .join(format!("{game}.wasm"));
            f.is_file().then_some(f)
        });
        Self {
            game: game_dir.to_owned(),
            module,
        }
    }
}

fn dir_of(session: &Path, n: u64) -> PathBuf {
    session.join("clips").join(format!("{n:03}.review"))
}

/// What the review depends on: the model, its mapping and the module.
fn key(r: &Rebuild) -> Result<String> {
    let mut h = blake3::Hasher::new();
    for f in [gp_model::MODEL_FILE, gp_interp::mapping::MAPPING_FILE] {
        let p = r.game.join(f);
        h.update(&fs::read(&p).with_context(|| format!("reading {}", p.display()))?);
    }
    if let Some(m) = &r.module {
        h.update(&fs::read(m).with_context(|| format!("reading {}", m.display()))?);
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
    let rom = crate::clip_score::rom_of(paths, &r.game, None)?;
    let assets = crate::corpus_score::assets_of(paths, &r.game, &r.game, None, &rom)?;
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
                return;
            };
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
    let ls = crate::clip_score::pictures(&ctx, &log, start, keep)?;
    let (count, have, size, visible, err) = shots.borrow().clone();
    if let Some(e) = err {
        bail!("writing clip {n}'s pictures: {e}");
    }
    let together = gp_interp::lockstep::together(&ls);
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
