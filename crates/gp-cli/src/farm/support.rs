//! What the farms took from other modules of game-printer's gp-cli, which
//! gameprinter hasn't got in that form: the sprite boxes (mechanic.rs),
//! the play-start rule (rom_extract.rs, which comes whole with `dataset
//! rom` in a later wave), a session clip with its start's state
//! (clip.rs's `new_clip`), the repo root, the file walk, and the game's
//! folder and ROM from the games checkout (game-printer read templates/).

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use gp_evidence::Paths;
use gp_evidence::input_log::{InputFrame, InputLog};

pub(crate) use crate::mechanic::paths::{put_path, read_path};

/// Sprite boxes from a sprite table: each sprite's box, and boxes that
/// touch merged, as `[x, y, w, h]`.
pub(crate) fn sprite_boxes(oam: &[u8; 256], tall: i32) -> Vec<[i32; 4]> {
    let mut b: Vec<[i32; 4]> = oam
        .chunks(4)
        .filter(|e| e[0] < 0xEF)
        .map(|e| [i32::from(e[3]), i32::from(e[0]) + 1, 8, tall])
        .collect();
    let touch = |a: &[i32; 4], c: &[i32; 4]| {
        a[0] <= c[0] + c[2] && c[0] <= a[0] + a[2] && a[1] <= c[1] + c[3] && c[1] <= a[1] + a[3]
    };
    loop {
        let mut merged = false;
        'outer: for i in 0..b.len() {
            for j in i + 1..b.len() {
                if touch(&b[i], &b[j]) {
                    let (a, c) = (b[i], b[j]);
                    let x0 = a[0].min(c[0]);
                    let y0 = a[1].min(c[1]);
                    let x1 = (a[0] + a[2]).max(c[0] + c[2]);
                    let y1 = (a[1] + a[3]).max(c[1] + c[3]);
                    b[i] = [x0, y0, x1 - x0, y1 - y0];
                    b.swap_remove(j);
                    merged = true;
                    break 'outer;
                }
            }
        }
        if !merged {
            return b;
        }
    }
}

/// The repo this gp was built from (its bench files and docs), or the
/// current folder when gp runs from elsewhere.
pub(crate) fn repo_root() -> Result<PathBuf> {
    let built = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    if built.join("Cargo.toml").is_file() {
        return Ok(built.canonicalize()?);
    }
    Ok(std::env::current_dir()?)
}

/// Whether a game or session name is lower-case letters, digits and
/// dashes.
pub(crate) fn name_ok(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Every file under `dir`.
pub(crate) fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let path = entry?.path();
        if path.is_dir() {
            walk(&path, out)?;
        } else if path.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

/// A game's folder in the games checkout and its ROM (the package's
/// roms.toml, found in $GP_ROM_DIR by hash). game-printer read
/// templates/GAME and its accept list here.
pub(crate) fn game_and_rom(
    paths: &Paths,
    game: &str,
    games: Option<&Path>,
) -> Result<(PathBuf, Vec<u8>)> {
    let games = crate::roms_cmd::games_dir(games)
        .context("no games checkout: pass --games or set GP_GAMES_DIR")?;
    let dir = games.join(game);
    if !dir.is_dir() {
        bail!("no game {game} in {}", games.display());
    }
    let rom = crate::clip_score::rom_of(paths, &dir, None)?;
    crate::original::nes_only(crate::roms::Platform::of_file(&rom), "the farm's search")?;
    Ok((dir, rom))
}

/// A clip that starts at frame `start` of a stored path: its `.toml`
/// fields with the start's state hash, and its `.log` text.
pub(crate) fn new_clip(
    home: &Path,
    rom: &[u8],
    sha: &str,
    start: usize,
    inputs: &[InputFrame],
) -> Result<(toml::Table, String)> {
    if inputs.is_empty() {
        bail!("the clip has no frames");
    }
    let nes = crate::mechanic::paths::nes_at(home, rom, sha, start)?;
    let mut il = InputLog::new(1);
    il.frames = inputs.to_vec();
    let (mut t, text) = crate::mechanic::paths::clip_fields(sha, start, &il);
    t.insert("state".into(), nes.state_hash()?.to_hex().into());
    Ok((t, text))
}

// ---- play start (game-printer's rom_extract.rs) --------------------------

/// One run's frames as gp-track reads them: RAM, sprite table, scroll and
/// pad, a frame behind when the game builds its sprites a frame early.
pub(crate) fn frames_of(rom: &[u8], clip: &crate::reach::ClipInput) -> Result<gp_track::Frames> {
    let mut nes = gp_nes::Nes::power_on(rom, clip.power_on)?;
    nes.set_video(false);
    let (mut ram, mut oam, mut scroll, mut pad) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut tall = 0usize;
    for input in &clip.inputs {
        nes.step(*input)?;
        ram.push(nes.wram()[..0x800].to_vec());
        oam.push(nes.oam().to_vec());
        let sc = &nes.bus().ppu.scroll;
        let t = sc.t;
        scroll.push((t & 31) * 8 + u16::from(sc.fine_x as u8 & 7) + ((t >> 10) & 1) * 256);
        pad.push(input.pads[0]);
        tall += usize::from(nes.tall_sprites());
    }
    let equal_at = |lag: usize, p: usize| -> usize {
        (0..ram.len().saturating_sub(lag))
            .filter(|&f| {
                let o = &oam[f + lag];
                o.iter().any(|&b| b != 0) && ram[f][p * 256..p * 256 + 256] == o[..]
            })
            .count()
    };
    let best = |lag: usize| {
        (0..8)
            .map(|p| (equal_at(lag, p), p))
            .max()
            .unwrap_or((0, 0))
    };
    let (b0, b1) = (best(0), best(1));
    let lag = if b0.0 > b1.0 { 0 } else { 1 };
    let n = ram.len().saturating_sub(lag);
    ram.truncate(n);
    pad.truncate(n);
    Ok(gp_track::Frames {
        first: 1,
        ram,
        oam: oam.split_off(lag.min(oam.len())),
        tall: tall * 2 > clip.inputs.len(),
        sprites: None,
        scroll_x: scroll.split_off(lag.min(scroll.len())),
        pad,
    })
}

/// Where play starts: the first of `clips` in which the pad moves the
/// player ([`pad_x`]) and the frame it first does; else the first in which
/// the tracker sees a player, and the frame it appears.
pub(crate) fn play_start(
    bytes: &[u8],
    clips: &[crate::reach::ClipInput],
) -> Result<Option<(usize, usize)>> {
    // A run where the pad moves the player is in play at that frame; the
    // tracker's player can be a title screen's or an intro's, so it is used
    // only when no run has the first.
    let mut tracked = None;
    for (i, c) in clips.iter().enumerate() {
        let mut f = frames_of(bytes, c)?;
        if let Some(t) = pad_x(&f).and_then(|x| first_move(&f, x)).filter(|&t| t > 0) {
            return Ok(Some((i, t)));
        }
        if tracked.is_none()
            && let Ok(r) = gp_track::track(&mut f, &gp_track::Options::default())
            && let Some(s) = r.tracks.iter().filter(|t| t.player).map(|t| t.spawn).min()
            && s > 1
        {
            tracked = Some((i, (s - 1) as usize));
        }
    }
    Ok(tracked)
}

/// The first frame `x` moves the way the pad says, Left or Right held.
fn first_move(f: &gp_track::Frames, x: u16) -> Option<usize> {
    let a = usize::from(x);
    (1..f.ram.len().min(f.pad.len())).find(|&t| {
        let (r, l) = (f.pad[t] & 0x80 != 0, f.pad[t] & 0x40 != 0);
        let d = f.ram[t][a].wrapping_sub(f.ram[t - 1][a]) as i8;
        r != l && d != 0 && (d > 0) == r
    })
}

/// The byte that is the player's x in one run, read from the RAM, the pad
/// and the sprites, for games whose drawing the tracker can't bind (a hero
/// whose sprites shift when it turns, a camera that keeps it still):
///
/// 1. it moves the way the pad says (Right up, Left down) on 95% or more of
///    the 40 or more frames it moves while one of them is held;
/// 2. on 90% of those frames a sprite is drawn within 24 pixels of the
///    byte's place on screen (its value less the scroll), at one offset;
/// 3. on 10% or more of those frames the screen doesn't scroll from the
///    frame before to the frame after (a camera byte moves only when the
///    screen does, sometimes a frame ahead of the scroll register);
/// 4. in the holds it moves in, it moves on half the frames or more (a menu
///    cursor moves once a press), and a tenth or more of its agreeing moves
///    are under each of Right and Left (a byte counting down while Left
///    happens to be held goes one way only);
/// 5. no other byte passes 1 to 4 as often (a subpixel byte passes 1 and
///    fails 2).
fn pad_x(f: &gp_track::Frames) -> Option<u16> {
    let n = f.ram.len().min(f.oam.len()).min(f.pad.len());
    let mut best: Vec<(usize, u16)> = Vec::new();
    for a in 0..0x800usize {
        let (mut moved, mut agree, mut still) = (0usize, 0usize, 0usize);
        let mut frames = Vec::new();
        // Each hold of Left or Right the byte moves in, and the frames of
        // it the byte moves on: a player held to one side moves on most of
        // them, a menu cursor once a press.
        let (mut held_in, mut moved_in) = (0usize, 0usize);
        // Agreeing moves under each direction: the player goes both ways, a
        // byte counting down only one.
        let (mut agree_r, mut agree_l) = (0usize, 0usize);
        let (mut seg_len, mut seg_moves) = (0usize, 0usize);
        for t in 1..n {
            let pad = f.pad[t];
            let (r, l) = (pad & 0x80 != 0, pad & 0x40 != 0);
            if r == l || f.pad[t] != f.pad[t - 1] {
                if seg_moves > 0 {
                    held_in += seg_len;
                    moved_in += seg_moves;
                }
                seg_len = 0;
                seg_moves = 0;
            }
            if r == l {
                continue;
            }
            seg_len += 1;
            let d = f.ram[t][a].wrapping_sub(f.ram[t - 1][a]) as i8;
            if d == 0 {
                continue;
            }
            seg_moves += 1;
            if (d > 0) == r {
                if r {
                    agree_r += 1;
                } else {
                    agree_l += 1;
                }
            }
            moved += 1;
            if (d > 0) == r {
                agree += 1;
                frames.push(t);
            }
            // The screen still from the frame before to the frame after: a
            // camera byte can move a frame before the scroll register does.
            let s0 = f.scroll_x[t - 1];
            if f.scroll_x[t] == s0 && f.scroll_x.get(t + 1).is_none_or(|&s| s == s0) {
                still += 1;
            }
        }
        if seg_moves > 0 {
            held_in += seg_len;
            moved_in += seg_moves;
        }
        if moved < 40
            || agree * 100 < moved * 95
            || still * 10 < moved
            || moved_in * 2 < held_in
            || agree_r * 10 < agree
            || agree_l * 10 < agree
        {
            continue;
        }
        // One window of 24 pixels, at one offset from the byte's place on
        // screen, must hold a sprite on 90% of the frames.
        let window = (0..256usize)
            .map(|o0| {
                frames
                    .iter()
                    .filter(|&&t| {
                        let place = f.ram[t][a].wrapping_sub(f.scroll_x[t] as u8);
                        f.oam[t].as_chunks::<4>().0.iter().any(|s| {
                            s[0] < 0xEF
                                && usize::from(place.wrapping_sub(s[3]).wrapping_sub(o0 as u8)) < 24
                        })
                    })
                    .count()
            })
            .max()
            .unwrap_or(0);
        if window * 10 >= frames.len() * 9 {
            best.push((agree, a as u16));
        }
    }
    best.sort_unstable_by(|x, y| y.cmp(x));
    match best.as_slice() {
        [(n0, a), (n1, _), ..] if n0 > n1 => Some(*a),
        [(_, a)] => Some(*a),
        _ => None,
    }
}
