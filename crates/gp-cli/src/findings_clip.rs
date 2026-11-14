//! What `gp findings confirm` runs on a mechanic session's clip: its video
//! and its detectors.
//!
//! game-printer's gate called `gp mechanic video` (a `gp record` of the
//! clip, then `gp video` drawing it with every frame checked) and `gp
//! mechanic contacts` (the head hits with the RAM they changed, side hits,
//! rests, stomps, hurts and items). gameprinter has neither command, so
//! both are here, over the platform: the clip's start is checked against
//! its saved state on [`OriginalRuntime`] (gp-nes or gp-arcade), the clip
//! is recorded with `gp record` and checked frame by frame with `gp
//! replay`, and its frames are drawn into an animated PNG. The detectors
//! are crate::mechanic::detect's, fed from [`Machine`]: the NES's sprite
//! table and work RAM, or the arcade board's sprite list and work RAM
//! (indices from E000), with an arcade frame's controls put in the NES
//! pad's bits the detectors read.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use gp_evidence::Paths;
use gp_evidence::input_log::{InputFrame, InputLog};
use serde_json::{Value, json};

use crate::farm::machine::Machine;
use crate::harvest::nes_pad;
use crate::mechanic::detect as m;

/// A game's folder in the games checkout and the ROM its package names.
pub(crate) fn game_rom(paths: &Paths, game: &str) -> Result<(PathBuf, Vec<u8>)> {
    let games =
        crate::roms_cmd::games_dir(None).context("no games checkout found: set GP_GAMES_DIR")?;
    let dir = crate::labels::game_folder(&games, game)
        .with_context(|| format!("no game {game} in {}", games.display()))?;
    let rom = crate::clip_score::rom_of(paths, &dir, None)?;
    Ok((dir, rom))
}

fn clip_toml(session: &Path, n: u64) -> Result<toml::Table> {
    let f = session.join(format!("clips/{n:03}.toml"));
    Ok(toml::from_str(&fs::read_to_string(&f).with_context(
        || format!("no clip {n} in {}", session.display()),
    )?)?)
}

/// Refuses a clip whose start doesn't reach the state its `.toml` saved.
fn check_start(home: &Path, rom: &[u8], session: &Path, n: u64) -> Result<()> {
    let t = clip_toml(session, n)?;
    let (Some(path), Some(want)) = (
        t.get("path").and_then(|v| v.as_str()),
        t.get("state").and_then(|v| v.as_str()),
    ) else {
        // A clip written the old way (a full log) has no saved start.
        return Ok(());
    };
    let start = t.get("start").and_then(|v| v.as_integer()).unwrap_or(0) as usize;
    let o = crate::mechanic::paths::original_at(home, rom, path, start)?;
    let got = o.state_hex()?;
    if got != want {
        bail!("the clip's start state is {got}, not the {want} it was saved with");
    }
    Ok(())
}

/// Runs gp with these arguments and its `--json` output.
fn gp_json(args: &[&std::ffi::OsStr]) -> Result<Value> {
    let exe = std::env::current_exe()?;
    let out = std::process::Command::new(&exe)
        .arg("--json")
        .args(args)
        .output()?;
    if !out.status.success() {
        bail!(
            "gp {}: {}{}",
            args.first()
                .map(|a| a.to_string_lossy().to_string())
                .unwrap_or_default(),
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout)
        );
    }
    Ok(serde_json::from_slice(&out.stdout).unwrap_or(Value::Null))
}

/// The picture after a frame, as RGB.
fn rgb(mc: &Machine) -> (usize, usize, Vec<u8>) {
    match mc {
        Machine::Nes(n) => {
            let mut b = Box::new([0u8; gp_nes::video::WIDTH * gp_nes::video::HEIGHT * 3]);
            n.frame_rgb(&mut b);
            (gp_nes::video::WIDTH, gp_nes::video::HEIGHT, b.to_vec())
        }
        Machine::Arcade(t) => (
            gp_arcade::video::WIDTH,
            gp_arcade::video::HEIGHT,
            t.picture()
                .iter()
                .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8])
                .collect(),
        ),
    }
}

/// A clip's video: the start checked against its saved state, the clip
/// from power-on recorded into the store (`gp record`) and replayed with
/// every frame checked against the recording (`gp replay`), and its own
/// frames drawn to `$GP_HOME/videos/GAME-SESSION-NNN.png`. As JSON:
/// recording, video, sha256, frames.
pub(crate) fn video_of_clip(paths: &Paths, game: &str, session: &str, n: u64) -> Result<Value> {
    let (_, rom) = game_rom(paths, game)?;
    let dir = paths.home.join("mechanics").join(game).join(session);
    check_start(&paths.home, &rom, &dir, n)?;
    let (log, start) = crate::mechanic::paths::full_log(&paths.home, &dir, n)?;
    let parsed = InputLog::parse(&log).map_err(|e| anyhow::anyhow!("{e}"))?;
    let work = paths.home.join("work").join("clip-videos");
    fs::create_dir_all(&work)?;
    let name = format!("{game}-{session}-{n:03}");
    let input = work.join(format!("{name}.log"));
    fs::write(&input, &log)?;
    // gp record reads a ROM file: a copy of the bytes found, gone when done.
    let rom_tmp = tempfile::Builder::new()
        .suffix(
            if crate::original::Platform::of_file(&rom) == crate::original::Platform::Arcade {
                ".zip"
            } else {
                ".nes"
            },
        )
        .tempfile()?;
    fs::write(rom_tmp.path(), &rom)?;
    let rom_file = rom_tmp.path().to_path_buf();
    let rec = gp_json(&[
        "record".as_ref(),
        "--rom".as_ref(),
        rom_file.as_os_str(),
        "--input".as_ref(),
        input.as_os_str(),
        "--name".as_ref(),
        name.as_ref(),
    ])?;
    let id = rec["id"]
        .as_str()
        .or_else(|| rec["data"]["id"].as_str())
        .or_else(|| rec["recording"].as_str())
        .context("gp record gave no id")?
        .to_owned();
    gp_json(&[
        "replay".as_ref(),
        id.as_ref(),
        "--rom".as_ref(),
        rom_file.as_os_str(),
    ])?;
    drop(rom_tmp);
    let mut mc = Machine::power_on(&rom, gp_evidence::manifest::PowerOn::default())?;
    let frames = parsed.frames.len().saturating_sub(start);
    let videos = paths.home.join("videos");
    fs::create_dir_all(&videos)?;
    let png = videos.join(format!("{name}.png"));
    let mut apng = None;
    for (i, f) in parsed.frames.iter().enumerate() {
        if i == start
            && let Machine::Nes(nes) = &mut mc
        {
            nes.set_video(true);
        }
        mc.step(*f)?;
        if i >= start {
            let (w, h, px) = rgb(&mc);
            if apng.is_none() {
                apng = Some(crate::theme_apng::Apng::new(
                    std::io::BufWriter::new(fs::File::create(&png)?),
                    w,
                    h,
                    frames.max(1) as u32,
                    (1, 60),
                )?);
            }
            if let Some(a) = apng.as_mut() {
                a.frame(&px)?;
            }
        }
    }
    apng.context("the clip has no frames")?.finish()?;
    let hash = gp_evidence::hash::Hash::of(&fs::read(&png)?).to_hex();
    let range = format!("{}..{}", start + 1, parsed.frames.len() + 1);
    Ok(json!({"recording": id, "video": png, "sha256": hash, "frames": range}))
}

/// The fall off the screen the boxes may not show: the y byte says so.
fn fell_off_y(ys: &[i32], start: usize) -> Option<usize> {
    (start.max(1)..ys.len()).find(|&k| ys[k] >= 224 || (ys[k - 1] > 200 && ys[k] < 32))
}

/// A clip's detectors, as `gp mechanic contacts` measured them in
/// game-printer: hits (head hits with the RAM bytes changed at them and
/// after), walls, rests, stomps, items and hurts, frames counted from the
/// clip's start. `xb` and `yb` are the player's x and y bytes.
pub(crate) fn detect_in_clip(
    paths: &Paths,
    game: &str,
    session: &str,
    n: u64,
    (xb, yb): (Option<u16>, u16),
) -> Result<Value> {
    const STILL: usize = 8; // frames a byte held before the hit
    const HIT: usize = 3; // frames after the hit that count as "at" it
    const AFTER: usize = 60; // frames after that, for what it set going
    const LEAD: usize = 64; // frames before the clip the detectors look back on
    let (game_dir, rom) = game_rom(paths, game)?;
    let _ = game_dir;
    let names = m::ram_names(&paths.home.join("work/datasets").join(game));
    let dir = paths.home.join("mechanics").join(game).join(session);
    let (log, start) = crate::mechanic::paths::full_log(&paths.home, &dir, n)?;
    let parsed = InputLog::parse(&log).map_err(|e| anyhow::anyhow!("{e}"))?;
    let base = start.saturating_sub(LEAD);
    let start = start - base;
    let mut mc = Machine::power_on(&rom, gp_evidence::manifest::PowerOn::default())?;
    let arcade = mc.is_arcade();
    let rb = mc.ram_base();
    let ix = |a: u16| {
        let a = usize::from(a);
        if a >= rb { a - rb } else { a }
    };
    for f in &parsed.frames[..base] {
        mc.step(*f)?;
    }
    let mut ram: Vec<Vec<u8>> = Vec::new();
    let mut boxes: Vec<Vec<[i32; 4]>> = Vec::new();
    let mut sprites: Vec<i64> = Vec::new();
    // A RAM page that holds the same bytes as the NES's sprite table is
    // the sprite buffer the game copies up each frame.
    let mut oam_page = [0usize; 8];
    for f in &parsed.frames[base..] {
        mc.step(*f)?;
        let w = mc.wram().to_vec();
        let b = mc.boxes();
        if let Machine::Nes(nes) = &mc {
            let oam = nes.oam();
            sprites.push(oam.chunks(4).filter(|e| e[0] < 0xEF).count() as i64);
            for (p, k) in oam_page.iter_mut().enumerate() {
                if w[p * 256..p * 256 + 256] == oam[..] {
                    *k += 1;
                }
            }
        } else {
            sprites.push(b.len() as i64);
        }
        boxes.push(b);
        ram.push(w);
    }
    if ram.is_empty() {
        bail!("clip {n} has no frames");
    }
    let size = ram[0].len();
    let frames = ram.len();
    let skip = |a: usize| {
        let role = names
            .get(&((a + rb) as u16))
            .map(String::as_str)
            .unwrap_or("");
        (!arcade && (0x100..0x200).contains(&a))
            || oam_page.get(a / 256).is_some_and(|p| p * 2 > frames)
            || role.ends_with("scratch")
            || role.ends_with("sound")
    };
    let yi = ix(yb);
    let y = |i: usize| i32::from(ram[i][yi]);
    let last = ram.len() - 1;
    let mut hits = Vec::new();
    let mut ground = None::<i32>;
    for i in start.max(STILL + 2)..ram.len().saturating_sub(1) {
        let (d0, d1) = (y(i) - y(i - 1), y(i + 1) - y(i));
        if d0 == 0 && y(i - 1) == y(i - 2) {
            ground = Some(y(i));
        }
        if !(d0 < 0 && d1 >= 0 && d1 - d0 >= 3) {
            continue;
        }
        let changed = |a: usize, from: usize, to: usize| -> Option<(u8, u8)> {
            let before = ram[from - 1][a];
            if (from - STILL..from).any(|k| ram[k][a] != before) {
                return None;
            }
            (from..=to.min(last))
                .map(|k| ram[k][a])
                .find(|v| *v != before)
                .map(|v| (before, v))
        };
        let row = |a: usize, (from, to): (u8, u8)| {
            let tail: Vec<u8> = (i..=last).map(|k| ram[k][a]).collect();
            let moves = tail.windows(2).filter(|w| w[0] != w[1]).count();
            let lasting = !tail[(HIT + 1).min(tail.len())..].contains(&from) && moves <= 3;
            json!({"addr": format!("${:04X}", a + rb), "from": from, "to": to, "end": ram[last][a],
                   "lasting": lasting, "known_as": names.get(&((a + rb) as u16))})
        };
        let (mut at_hit, mut after) = (Vec::new(), Vec::new());
        for a in 0..size {
            if a == yi || skip(a) {
                continue;
            }
            if let Some(c) = changed(a, i - 1, i + HIT) {
                at_hit.push(row(a, c));
            } else if let Some(c) = changed(a, i + HIT + 1, i + HIT + AFTER) {
                after.push(row(a, c));
            }
        }
        let added = (i..(i + AFTER).min(sprites.len()))
            .map(|k| sprites[k])
            .max()
            .unwrap_or(0)
            - sprites[i - 1];
        hits.push(json!({
            "frame": i - start,
            "rose_px": ground.map(|g| g - y(i)),
            "at_hit": at_hit,
            "after": after,
            "sprites_added": added,
        }));
    }
    let ys: Vec<i32> = (0..ram.len()).map(y).collect();
    let pads: Vec<u8> = parsed.frames[base..]
        .iter()
        .map(|f| {
            if arcade {
                nes_pad(f.pads[0])
            } else {
                f.pads[0]
            }
        })
        .collect();
    let walls = match xb {
        Some(xb) => {
            let xi = ix(xb);
            let xs: Vec<i32> = ram.iter().map(|r| i32::from(r[xi])).collect();
            m::walls(&xs, &ys, &pads, start)
        }
        None => Vec::new(),
    };
    let track = m::player_track(&boxes, &ys, start);
    let kept = |i: usize| -> Vec<Value> {
        let i = i.max(STILL + 2).min(last);
        (0..size)
            .filter(|&a| a != yi && !skip(a))
            .filter_map(|a| {
                let before = ram[i - 2][a];
                if (i - 1 - STILL..i - 1).any(|k| ram[k][a] != before) {
                    return None;
                }
                let to = (i - 1..=(i + HIT).min(last))
                    .map(|k| ram[k][a])
                    .find(|v| *v != before)?;
                let tail: Vec<u8> = (i..=last).map(|k| ram[k][a]).collect();
                let moves = tail.windows(2).filter(|w| w[0] != w[1]).count();
                (!tail[HIT.min(tail.len() - 1)..].contains(&before) && moves <= 3).then(|| {
                    json!({"addr": format!("${:04X}", a + rb), "from": before, "to": to, "end": ram[last][a],
                           "known_as": names.get(&((a + rb) as u16))})
                })
            })
            .collect()
    };
    let with_kept = |v: Vec<(usize, Value)>| -> Vec<Value> {
        v.into_iter()
            .map(|(i, mut e)| {
                e["kept"] = json!(kept(i));
                e
            })
            .collect()
    };
    let stomps = with_kept(m::stomps(&ys, &pads, &boxes, &track, start));
    let mut hu = m::hurts(&boxes, &track, start);
    if !hu
        .iter()
        .any(|(_, v)| v["kind"] == json!("fell off the screen"))
        && let Some(k) = fell_off_y(&ys, start)
    {
        hu.push((
            k,
            json!({"frame": k - start, "kind": "fell off the screen", "by": "y"}),
        ));
    }
    let hurts = with_kept(hu);
    let hit_frames: Vec<usize> = hits
        .iter()
        .filter_map(|h| h["frame"].as_u64().map(|f| f as usize + start))
        .collect();
    let items = with_kept(m::items(&boxes, &track, &hit_frames, start));
    let rests = m::rests(&ys, &pads, start);
    Ok(
        json!({"clip": n, "hits": hits, "rests": rests, "walls": walls,
              "stomps": stomps, "hurts": hurts, "items": items}),
    )
}

/// The inputs of a clip's start as a stored path, for a new clip.
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
    let o = crate::mechanic::paths::original_at(home, rom, sha, start)?;
    let path = crate::mechanic::paths::read_path(home, sha)?;
    let il = InputLog {
        controllers: 1,
        frames: inputs.to_vec(),
        arcade: path.arcade.clone(),
    };
    let (mut t, text) = crate::mechanic::paths::clip_fields(sha, start, &il);
    t.insert("state".into(), o.state_hex()?.into());
    Ok((t, text))
}

/// A log in the spelling the ROM's board reads: a farm run on an arcade
/// romset writes its path as an NES log of the board's control bits, and
/// a clip's start on gp-arcade wants an arcade log (`gp-arcade-input 0`).
pub(crate) fn board_log(rom: &[u8], mut log: InputLog) -> Result<InputLog> {
    if log.arcade.is_none() {
        let board = crate::mechanic::probe::Board::of_rom(rom)?;
        if board.arcade.is_some() {
            log.arcade = board.arcade;
            log.controllers = 1;
        }
    }
    Ok(log)
}
