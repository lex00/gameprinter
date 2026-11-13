//! A clip as evidence: `gp mechanic video` (its video, checked as drawn),
//! `contacts` (where the player's head hit something, side hits, rests,
//! stomps, items and hurts, with the RAM bytes each changed), `compare`
//! (two clips' tallest jumps, measured) and `clip-save` (a clip from an
//! input log that runs from power-on).
//!
//! Ported from game-printer's mechanic.rs (`clip_video`,
//! `head_hits_and_boxes` and `measure_overlay`, `compare_text` and
//! `clip_jumps`, the `ClipSave` arm). The ROM is the game's in the games
//! checkout (the set chosen for it, else the scored one: `--rom` chooses,
//! where game-printer noted it in the session), and starts are replayed
//! through [`super::paths::original_at`], so every command runs on an
//! arcade romset too:
//!
//! - `video` records the clip with `gp record --source SET` and draws it
//!   with `gp video`, which draws the board's picture on arcade.
//! - `contacts` reads the board's work RAM (E000 to E7FF) and its sprite
//!   list where the NES's RAM and sprite table are, and the player's bytes
//!   from the ROM dataset or the game's mapping (entities.player[0].x and
//!   .y, Tapper's E2E9 and E2E6). On arcade a head hit must rise for two
//!   frames: a one-frame rise is a move between rows (Tapper's bartender
//!   changing bars), not a jump.
//! - `compare` measures jumps from the game's mapping on the NES
//!   (demo::measure_jumps, as `gp mechanic say --judge` does). An arcade
//!   game's player doesn't jump, so there it compares the player's place
//!   through the mapping: how far across the clip took them and the rows
//!   they stood on.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use gp_evidence::Paths;
use gp_evidence::input_log::InputLog;
use serde_json::{Value, json};

use super::detect as m;
use super::paths as clip_paths;
use super::session::Session;
use crate::Outcome;
use crate::original::{OriginalRuntime, Platform};

/// The game's folder and its ROM, and the set's id; `rom` chooses the set
/// first (kept for the game, as `gp mechanic start --rom` keeps it).
fn game_rom(paths: &Paths, game: &str, rom: Option<&Path>) -> Result<(PathBuf, Vec<u8>, String)> {
    if let Some(r) = rom {
        super::server::choose_rom(paths, None, game, &r.to_string_lossy())?;
    }
    let games =
        crate::roms_cmd::games_dir(None).context("no games checkout found: set GP_GAMES_DIR")?;
    let dir = crate::labels::game_folder(&games, game)
        .with_context(|| format!("no game {game} in {}", games.display()))?;
    let (r, id, _) = super::server::game_rom(paths, None, game)?;
    Ok((dir, r.bytes, id))
}

fn clip_ref(s: &Session, mechanic: &str, n: u64) -> Result<crate::clip_session::SessionClip> {
    if !s.dir.join(format!("clips/{n:03}.toml")).is_file() {
        bail!("no clip {n} in this session");
    }
    Ok(crate::clip_session::SessionClip {
        session: mechanic.to_owned(),
        dir: s.dir.clone(),
        n,
    })
}

// ---- video -----------------------------------------------------------------

/// `gp mechanic video`: the clip's start checked against the state hash it
/// was saved with, its inputs from power-on stored as a recording, and
/// `gp video` drawing its frames into $GP_HOME/videos.
pub(crate) fn clip_video(
    paths: &Paths,
    game: &str,
    mechanic: &str,
    clip: u64,
    rom: Option<&Path>,
) -> Result<Outcome> {
    let s = Session::open(paths, game, mechanic)?;
    let (_, bytes, set) = game_rom(paths, game, rom)?;
    let c = clip_ref(&s, mechanic, clip)?;
    // Refuses a clip whose start doesn't match its saved state.
    c.check_start(&paths.home, &bytes)?;
    let (log, start) = clip_paths::full_log(&paths.home, &s.dir, clip)?;
    let frames_in = InputLog::parse(&log)
        .map_err(|e| anyhow::anyhow!("clip {clip}: {e}"))?
        .frames
        .len()
        - start;
    let work = paths.home.join("work").join("clip-videos");
    fs::create_dir_all(&work)?;
    let input = work.join(format!("{game}-{mechanic}-{clip:03}.log"));
    fs::write(&input, &log)?;
    let exe = std::env::current_exe()?;
    let mut cmd = std::process::Command::new(&exe);
    cmd.args(["--json", "record"]);
    match rom {
        Some(r) => cmd.arg("--rom").arg(r),
        None => cmd.args(["--source", &set]),
    };
    let out = cmd
        .arg("--input")
        .arg(&input)
        .args([
            "--name",
            &format!("{game}-{mechanic}-{clip:03}"),
            "--no-trace",
        ])
        .output()?;
    if !out.status.success() {
        bail!("gp record: {}", String::from_utf8_lossy(&out.stderr));
    }
    let rec: Value = serde_json::from_slice(&out.stdout)?;
    let id = rec["data"]["id"]
        .as_str()
        .or_else(|| rec["id"].as_str())
        .context("gp record gave no id")?
        .to_owned();
    let videos = paths.home.join("videos");
    fs::create_dir_all(&videos)?;
    let png = videos.join(format!("{game}-{mechanic}-{clip:03}.png"));
    // Frame k of the log is drawn as frame k+1 (frame 0 is power-on).
    let frames = format!("{}..{}", start + 1, start + frames_in + 1);
    let mut cmd = std::process::Command::new(&exe);
    cmd.args(["video", &id, "--frames", &frames, "--force", "-o"])
        .arg(&png);
    if let Some(r) = rom {
        cmd.arg("--rom").arg(r);
    }
    let out = cmd.output()?;
    if !out.status.success() {
        bail!("gp video: {}", String::from_utf8_lossy(&out.stderr));
    }
    let hash = gp_evidence::Hash::of(&fs::read(&png)?).to_hex();
    Ok(Outcome::ok(
        json!({"recording": id, "video": png, "sha256": hash, "frames": frames}),
        format!(
            "{} (frames {frames}, recording {id}, hash {hash})",
            png.display()
        ),
    ))
}

// ---- clip-save -------------------------------------------------------------

/// What `gp mechanic clip-save` was asked.
pub(crate) struct ClipSave<'a> {
    pub game: &'a str,
    pub mechanic: &'a str,
    pub log: &'a Path,
    pub start: usize,
    pub frames: Option<usize>,
    pub note: &'a str,
    pub by: &'a str,
    pub replace: Option<u64>,
}

/// `gp mechanic clip-save`: the frames before `start` stored once as a
/// path, the state there hashed, and the clip keeping only its own frames.
pub(crate) fn clip_save(paths: &Paths, a: &ClipSave) -> Result<Outcome> {
    let s = Session::open(paths, a.game, a.mechanic)?;
    let (_, bytes, _) = game_rom(paths, a.game, None)?;
    let parsed = InputLog::parse(&fs::read_to_string(a.log)?)
        .map_err(|e| anyhow::anyhow!("{}: {e}", a.log.display()))?;
    let start = a.start;
    if start >= parsed.frames.len() {
        bail!(
            "the log has {} frames, none after {start}",
            parsed.frames.len()
        );
    }
    let end = a.frames.map_or(parsed.frames.len(), |f| {
        (start + f).min(parsed.frames.len())
    });
    let mut prefix = parsed.clone();
    prefix.frames.truncate(start);
    let (sha, _) = clip_paths::put_path(&paths.home, &prefix.to_text())?;
    let o = clip_paths::original_at(&paths.home, &bytes, &sha, start)?;
    let mut own = parsed.clone();
    own.frames = parsed.frames[start..end].to_vec();
    let (mut t, text) = clip_paths::clip_fields(&sha, start, &own);
    t.insert("state".into(), o.state_hex()?.into());
    let n = match a.replace {
        Some(n) => {
            if !s.dir.join(format!("clips/{n:03}.toml")).is_file() {
                bail!("no clip {n} in this session to write over");
            }
            t.insert("by".into(), a.by.into());
            t.insert("note".into(), a.note.into());
            fs::write(s.dir.join(format!("clips/{n:03}.log")), &text)?;
            fs::write(
                s.dir.join(format!("clips/{n:03}.toml")),
                toml::to_string(&t)?,
            )?;
            n
        }
        None => s.add_clip_meta(t, &text, a.note, a.by)?,
    };
    Ok(Outcome::ok(
        json!({"clip": n, "path": sha, "start": start, "frames": end - start}),
        format!(
            "clip {n}: path {sha}, start {start}, {} frames",
            end - start
        ),
    ))
}

// ---- compare ---------------------------------------------------------------

/// The heights of a clip's jumps, in the order they happened.
fn rises(j: &[Value]) -> Vec<i64> {
    j.iter().filter_map(|v| v["rose_px"].as_i64()).collect()
}

/// The measured answer to "how does clip NEW's jump compare with clip
/// OLD's": their tallest jumps, the same height or by how many pixels apart.
fn compare_text(new: u64, a: &[Value], old: u64, b: &[Value]) -> (String, Value) {
    let (ra, rb) = (rises(a), rises(b));
    let (Some(ta), Some(tb)) = (ra.iter().max(), rb.iter().max()) else {
        return (
            format!(
                "I found no jump to compare: clip {new} has {} and clip {old} has {}.",
                ra.len(),
                rb.len()
            ),
            json!("none"),
        );
    };
    let d = ta - tb;
    let v = match d {
        0 => "same",
        d if d > 0 => "higher",
        _ => "lower",
    };
    let text = format!(
        "The see-through Mario is clip {old}. The solid one is clip {new}.\n{}\n\nNothing to answer: this was measured.",
        match d {
            0 => format!("Their highest jumps are the same height, {ta} px."),
            d if d > 0 =>
                format!("Clip {new}'s highest jump is {d} px higher ({ta} px against {tb} px)."),
            d => format!(
                "Clip {new}'s highest jump is {} px lower ({ta} px against {tb} px).",
                -d
            ),
        }
    );
    (
        text,
        json!({"verdict": v, "new_px": ta, "old_px": tb, "by": "measurement"}),
    )
}

/// Where a clip took the player, from the game's mapping: the x it went
/// from and to, and the rows (y values held 8 frames or more) it stood on.
fn clip_places(paths: &Paths, s: &Session, n: u64, rom: &[u8], game_dir: &Path) -> Result<Value> {
    let (xb, yb) = super::demo::player_bytes(game_dir)?;
    let (text, start) = clip_paths::full_log(&paths.home, &s.dir, n)?;
    let log = InputLog::parse(&text).map_err(|e| anyhow::anyhow!("clip {n}: {e}"))?;
    let mut prefix = log.clone();
    prefix.frames.truncate(start);
    let (sha, _) = clip_paths::put_path(&paths.home, &prefix.to_text())?;
    let mut o = clip_paths::original_at(&paths.home, rom, &sha, start)?;
    let (mut xs, mut ys) = (Vec::new(), Vec::new());
    for f in &log.frames[start..] {
        super::probe::step(&mut o, f)?;
        if let Some(b) = xb {
            xs.push(i64::from(super::probe::peek(&o, b)?));
        }
        ys.push(i64::from(super::probe::peek(&o, yb)?));
    }
    let mut rows: Vec<i64> = Vec::new();
    let mut i = 0;
    while i < ys.len() {
        let mut j = i;
        while j + 1 < ys.len() && ys[j + 1] == ys[i] {
            j += 1;
        }
        if j + 1 - i >= 8 && !rows.contains(&ys[i]) {
            rows.push(ys[i]);
        }
        i = j + 1;
    }
    let (lo, hi) = (xs.iter().min().copied(), xs.iter().max().copied());
    Ok(json!({"x_min": lo, "x_max": hi, "x_span_px": hi.zip(lo).map(|(h, l)| h - l), "rows": rows}))
}

/// The measured answer for a game whose player doesn't jump: how far
/// across each clip took the player, and the rows each stood on.
fn compare_places_text(new: u64, a: &Value, old: u64, b: &Value) -> (String, Value) {
    let (Some(sa), Some(sb)) = (a["x_span_px"].as_i64(), b["x_span_px"].as_i64()) else {
        return (
            format!(
                "I found no place to compare: the game's mapping names no player x for clips {new} and {old}."
            ),
            json!("none"),
        );
    };
    let d = sa - sb;
    let v = match d {
        0 => "same",
        d if d > 0 => "wider",
        _ => "narrower",
    };
    let rows = |x: &Value| x["rows"].as_array().map_or(0, Vec::len);
    let text = format!(
        "The see-through player is clip {old}. The solid one is clip {new}.\n{} Clip {new} stood on {} row(s), clip {old} on {}.\n\nNothing to answer: this was measured.",
        match d {
            0 => format!("Both went {sa} px across."),
            d if d > 0 =>
                format!("Clip {new} went {d} px farther across ({sa} px against {sb} px)."),
            d => format!(
                "Clip {new} went {} px less far across ({sa} px against {sb} px).",
                -d
            ),
        },
        rows(a),
        rows(b)
    );
    (
        text,
        json!({"verdict": v, "new_px": sa, "old_px": sb, "by": "measurement", "measure": "x span"}),
    )
}

/// `gp mechanic compare`: a turn in the session with the measured answer.
pub(crate) fn compare(
    paths: &Paths,
    game: &str,
    mechanic: &str,
    new: u64,
    old: u64,
) -> Result<Outcome> {
    let s = Session::open(paths, game, mechanic)?;
    let (dir, rom, _) = game_rom(paths, game, None)?;
    clip_ref(&s, mechanic, new)?;
    clip_ref(&s, mechanic, old)?;
    let (text, answer, measured) = if Platform::of_file(&rom) == Platform::Arcade {
        let a = clip_places(paths, &s, new, &rom, &dir)?;
        let b = clip_places(paths, &s, old, &rom, &dir)?;
        let (t, ans) = compare_places_text(new, &a, old, &b);
        (t, ans, json!({"new": a, "old": b}))
    } else {
        let a = super::demo::measure_jumps(paths, &s, new, &rom, &dir)?;
        let b = super::demo::measure_jumps(paths, &s, old, &rom, &dir)?;
        let (t, ans) = compare_text(new, &a, old, &b);
        (t, ans, json!({"new": a, "old": b}))
    };
    let t = json!({"by": "agent", "text": text, "show": new, "against": old,
        "answer": answer, "measured": measured,
        "choices": ["Yes, those are the jumps I see", "No, a jump is missing or extra"]});
    let n = s.turn(t)?;
    Ok(Outcome::ok(
        json!({"turn": n, "answer": answer}),
        format!("said (turn {n}): {text}"),
    ))
}

// ---- contacts --------------------------------------------------------------

/// The clip replayed with a lead-in: each frame's RAM (the NES's 2 KB, or
/// the board's work RAM), sprite boxes, sprite count and pad.
struct Replay {
    ram: Vec<Vec<u8>>,
    boxes: Vec<Vec<[i32; 4]>>,
    sprites: Vec<i64>,
    pads: Vec<u8>,
    /// Per 256-byte page of the RAM: the frames it held the sprite table.
    oam_page: [usize; 8],
    /// The address of the RAM's first byte.
    base: u16,
    /// The clip's first frame, counted from the replay's.
    start: usize,
    arcade: bool,
}

/// Frames before the clip the detectors look back on.
const LEAD: usize = 64;

fn replay(paths: &Paths, s: &Session, clip: u64, rom: &[u8]) -> Result<Replay> {
    let (text, at) = clip_paths::full_log(&paths.home, &s.dir, clip)?;
    let parsed = InputLog::parse(&text).map_err(|e| anyhow::anyhow!("{e}"))?;
    // Only the clip and a lead-in before it are read, so the replay starts
    // a little before the clip; indices below count from there.
    let first = at.saturating_sub(LEAD);
    let mut prefix = parsed.clone();
    prefix.frames.truncate(first);
    let (sha, _) = clip_paths::put_path(&paths.home, &prefix.to_text())?;
    let mut o = clip_paths::original_at(&paths.home, rom, &sha, first)?;
    o.set_video(false);
    let arcade = matches!(o, OriginalRuntime::Arcade(_));
    let mut r = Replay {
        ram: Vec::new(),
        boxes: Vec::new(),
        sprites: Vec::new(),
        pads: Vec::new(),
        oam_page: [0; 8],
        base: if arcade {
            crate::arcade_stage0::RAM_BASE
        } else {
            0
        },
        start: at - first,
        arcade,
    };
    for f in &parsed.frames[first..] {
        super::probe::step(&mut o, f)?;
        match &o {
            OriginalRuntime::Arcade(a) => {
                let t = a.tapper();
                r.ram.push(
                    (crate::arcade_stage0::RAM_BASE..crate::arcade_stage0::RAM_END)
                        .map(|k| t.peek(k))
                        .collect(),
                );
                let sprites: Vec<u8> = (0xe800..0xea00u16).map(|k| t.peek(k)).collect();
                let list = gp_arcade::video::sprite_list(&sprites);
                r.sprites.push(list.len() as i64);
                r.boxes.push(m::merge_boxes(
                    list.iter()
                        .map(|s| [i32::from(s.x), i32::from(s.y), 16, 16])
                        .collect(),
                ));
                r.pads.push(crate::harvest::nes_pad(f.buttons_word() as u8));
            }
            _ => {
                let nes = o.nes().context("an NES runtime")?.nes();
                let w = nes.wram()[..0x800].to_vec();
                let oam = nes.oam();
                r.sprites
                    .push(oam.chunks(4).filter(|e| e[0] < 0xEF).count() as i64);
                r.boxes.push(m::sprite_boxes(
                    &oam,
                    if nes.tall_sprites() { 16 } else { 8 },
                ));
                for (p, n) in r.oam_page.iter_mut().enumerate() {
                    if w[p * 256..p * 256 + 256] == oam[..] {
                        *n += 1;
                    }
                }
                r.ram.push(w);
                r.pads.push(f.pads[0]);
            }
        }
    }
    Ok(r)
}

/// The first frame the player's y goes off the bottom of the screen: to
/// 224 or more, or round from above 200 to below 32 in a frame (a byte
/// that wraps).
fn fell_off_y(ys: &[i32], start: usize) -> Option<usize> {
    (start.max(1)..ys.len()).find(|&k| ys[k] >= 224 || (ys[k - 1] > 200 && ys[k] < 32))
}

/// The detectors over a clip: head hits, rests, side hits, stomps, items
/// and hurts, and each frame's boxes and the player's: what the page draws
/// over the clip (`clips/NNN.measure.json`).
fn measure_overlay(paths: &Paths, s: &Session, game: &str, clip: u64, rom: &[u8]) -> Result<Value> {
    const STILL: usize = 8; // frames a byte held before the hit
    const HIT: usize = 3; // frames after the hit that count as "at" it
    const AFTER: usize = 60; // frames after that, for what it set going
    let dir = paths.home.join("work/datasets").join(game);
    let (xb, yb, _) = m::player_bytes(&dir)?;
    let names = m::ram_names(&dir);
    let r = replay(paths, s, clip, rom)?;
    let Replay {
        ram,
        boxes,
        sprites,
        pads,
        oam_page,
        base,
        start,
        arcade,
    } = r;
    let ix = |a: u16| usize::from(a.wrapping_sub(base));
    let size = ram.first().map_or(0, Vec::len);
    let frames = ram.len().max(1);
    let skip = |a: usize| {
        let role = names
            .get(&(a as u16 + base))
            .map(String::as_str)
            .unwrap_or("");
        // The stack page, the sprite buffer, and bytes the dataset says are
        // scratch or sound: they change around any event.
        (!arcade && ((0x100..0x200).contains(&a) || oam_page[a / 256] * 2 > frames))
            || role.ends_with("scratch")
            || role.ends_with("sound")
    };
    let addr = |a: usize| format!("${:04X}", a + usize::from(base));
    let known = |a: usize| names.get(&(a as u16 + base)).cloned();
    let y = |i: usize| i32::from(ram[i][ix(yb)]);
    let mut hits = Vec::new();
    let mut ground = None::<i32>;
    for i in start.max(STILL + 2)..ram.len().saturating_sub(1) {
        let (d0, d1) = (y(i) - y(i - 1), y(i + 1) - y(i));
        if d0 == 0 && y(i - 1) == y(i - 2) {
            ground = Some(y(i));
        }
        // Rising, then falling by 3 px a frame more than the frame before:
        // gravity turns a jump over 1 px a frame at a time. On arcade the
        // rise must last two frames: one is a move between rows.
        if !(d0 < 0 && d1 >= 0 && d1 - d0 >= 3) || (arcade && y(i - 1) >= y(i - 2)) {
            continue;
        }
        let changed = |a: usize, from: usize, to: usize| -> Option<(u8, u8)> {
            let before = ram[from - 1][a];
            if (from - STILL..from).any(|k| ram[k][a] != before) {
                return None;
            }
            (from..=to.min(ram.len() - 1))
                .map(|k| ram[k][a])
                .find(|v| *v != before)
                .map(|v| (before, v))
        };
        // Lasting: it never went back to its old value for the rest of the
        // clip and changed at most twice more (a block's tile, not a timer).
        let last = ram.len() - 1;
        let row = |a: usize, (from, to): (u8, u8)| {
            let tail: Vec<u8> = (i..=last).map(|k| ram[k][a]).collect();
            let moves = tail.windows(2).filter(|w| w[0] != w[1]).count();
            let lasting = !tail[(HIT + 1).min(tail.len())..].contains(&from) && moves <= 3;
            json!({"addr": addr(a), "from": from, "to": to, "end": ram[last][a],
                   "lasting": lasting, "known_as": known(a)})
        };
        let mut at_hit = Vec::new();
        let mut after = Vec::new();
        for a in 0..size {
            if a == ix(yb) || skip(a) {
                continue;
            }
            // From the frame before: the game handles the hit on the frame
            // the rise ends, before the position shows it turning over.
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
    let walls = match xb {
        Some(xb) => {
            let xs: Vec<i32> = (0..ram.len()).map(|i| i32::from(ram[i][ix(xb)])).collect();
            m::walls(&xs, &ys, &pads, start)
        }
        None => Vec::new(),
    };
    // Stomps and hurts, each with the RAM bytes that changed at it (from
    // the frame before) and kept their new value to the clip's end.
    let track = m::player_track(&boxes, &ys, start);
    let last = ram.len().saturating_sub(1);
    let kept = |i: usize| -> Vec<Value> {
        let i = i.max(STILL + 2);
        if i > last {
            return Vec::new();
        }
        (0..size)
            .filter(|&a| a != ix(yb) && !skip(a))
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
                    json!({"addr": addr(a), "from": before, "to": to, "end": ram[last][a],
                           "known_as": known(a)})
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
    let mut hu_raw = m::hurts(&boxes, &track, start);
    // A fall off the screen that the boxes don't show (the player's
    // sprites go before it's below the last rows): the y byte says so. The
    // board's screen is taller than a byte's 224, so only on the NES.
    if !arcade
        && !hu_raw
            .iter()
            .any(|(_, v)| v["kind"] == json!("fell off the screen"))
        && let Some(k) = fell_off_y(&ys, start)
    {
        hu_raw.push((
            k,
            json!({"frame": k - start, "kind": "fell off the screen", "by": "y"}),
        ));
    }
    let hurts = with_kept(hu_raw);
    let hit_frames: Vec<usize> = hits
        .iter()
        .filter_map(|h| h["frame"].as_u64().map(|f| f as usize + start))
        .collect();
    let items = with_kept(m::items(&boxes, &track, &hit_frames, start));
    let rests = m::rests(&ys, &pads, start);
    let frames: Vec<Value> = boxes
        .iter()
        .zip(&track)
        .skip(start)
        .map(|(b, p)| json!({"boxes": b, "player": p}))
        .collect();
    Ok(
        json!({"clip": clip, "frames": frames, "hits": hits, "rests": rests, "walls": walls,
              "stomps": stomps, "hurts": hurts, "items": items}),
    )
}

/// `gp mechanic contacts`: the detectors over a clip, written beside it
/// for the page, and said in words.
pub(crate) fn contacts(
    paths: &Paths,
    game: &str,
    mechanic: &str,
    clip: u64,
    rom: Option<&Path>,
) -> Result<Outcome> {
    let s = Session::open(paths, game, mechanic)?;
    let (_, bytes, _) = game_rom(paths, game, rom)?;
    clip_ref(&s, mechanic, clip)?;
    let overlay = measure_overlay(paths, &s, game, clip, &bytes)?;
    fs::write(
        s.dir.join(format!("clips/{clip:03}.measure.json")),
        overlay.to_string(),
    )?;
    let hits = overlay["hits"].as_array().cloned().unwrap_or_default();
    let mut text = String::new();
    for r in overlay["rests"].as_array().into_iter().flatten() {
        let _ = writeln!(
            text,
            "stood {} px up, frames {} to {}{}",
            r["up_px"],
            r["from"],
            r["to"],
            if r["then"].is_object() {
                format!(
                    ", then {} to {} px up in {} frames",
                    r["then"]["how"].as_str().unwrap_or(""),
                    r["then"]["to_up_px"],
                    r["then"]["frames"]
                )
            } else {
                String::new()
            }
        );
    }
    for w in overlay["walls"].as_array().into_iter().flatten() {
        let _ = writeln!(
            text,
            "side hit at frame {}: moving {} at {} px a frame, {}",
            w["frame"],
            w["side"].as_str().unwrap_or(""),
            w["speed_px_a_frame"],
            if w["in_air"] == json!(true) {
                "in the air"
            } else {
                "on the ground"
            }
        );
    }
    for w in overlay["stomps"].as_array().into_iter().flatten() {
        let _ = writeln!(
            text,
            "stomp at frame {}: bounced {} px, the box under it {}; {} byte(s) kept a new value",
            w["frame"],
            w["bounced_px"],
            w["under"].as_str().unwrap_or(""),
            w["kept"].as_array().map_or(0, Vec::len)
        );
    }
    for w in overlay["items"].as_array().into_iter().flatten() {
        let legs: Vec<String> = w["legs"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|l| {
                format!(
                    "{} px across in {} frames ({} px a frame)",
                    l["dx_px"], l["frames"], l["px_a_frame"]
                )
            })
            .collect();
        let _ = writeln!(
            text,
            "item {}x{} came out at frame {}, rose {} px, then {}; it {}{}",
            w["size"][0],
            w["size"][1],
            w["frame"],
            w["rose_px"],
            legs.join(", then "),
            w["end"].as_str().unwrap_or(""),
            if w["touch_frame"].is_null() {
                String::new()
            } else {
                format!(
                    " at frame {}: nothing moved for {} frames, player box {} -> {} px tall, {} byte(s) kept a new value",
                    w["touch_frame"],
                    w["frozen_frames"],
                    w["player_h"][0],
                    w["player_h"][1],
                    w["kept"].as_array().map_or(0, Vec::len)
                )
            }
        );
    }
    for w in overlay["hurts"].as_array().into_iter().flatten() {
        let _ = writeln!(
            text,
            "player box {} at frame {}{}; {} byte(s) kept a new value",
            w["kind"].as_str().unwrap_or(""),
            w["frame"],
            match w["kind"].as_str() {
                Some("blinked") => format!(" for {} frames", w["frames"]),
                Some("fell off the screen") => String::new(),
                _ => format!(" ({} -> {} px tall)", w["from_h"], w["to_h"]),
            },
            w["kept"].as_array().map_or(0, Vec::len)
        );
    }
    let _ = write!(text, "{} head hit(s) in clip {clip}", hits.len());
    for h in &hits {
        let _ = write!(
            text,
            "\nframe {} (rose {} px): {} byte(s) changed at the hit, {} later; sprites {:+}",
            h["frame"],
            h["rose_px"],
            h["at_hit"].as_array().map_or(0, Vec::len),
            h["after"].as_array().map_or(0, Vec::len),
            h["sprites_added"].as_i64().unwrap_or(0)
        );
        for (k, list) in [("at hit", &h["at_hit"]), ("after", &h["after"])] {
            for b in list.as_array().into_iter().flatten() {
                let _ = write!(
                    text,
                    "\n  {k}: {} {} -> {}{}",
                    b["addr"].as_str().unwrap_or(""),
                    b["from"],
                    b["to"],
                    b["known_as"]
                        .as_str()
                        .map(|n| format!("  ({n})"))
                        .unwrap_or_default()
                );
            }
        }
    }
    Ok(Outcome::ok(json!({"hits": hits}), text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compare_says_which_jump_is_higher() {
        let j = |px: i64| json!({"rose_px": px});
        let (_, v) = compare_text(2, &[j(20), j(35)], 1, &[j(30)]);
        assert_eq!(
            (v["verdict"].as_str(), v["new_px"].as_i64()),
            (Some("higher"), Some(35))
        );
        let (t, v) = compare_text(2, &[], 1, &[j(30)]);
        assert_eq!(v, json!("none"));
        assert!(t.contains("no jump"));
    }

    #[test]
    fn compare_of_places_measures_the_walk_across() {
        let p = |span: i64| json!({"x_span_px": span, "rows": [48, 96]});
        let (t, v) = compare_places_text(5, &p(40), 4, &p(60));
        assert_eq!(v["verdict"], "narrower");
        assert!(t.contains("20 px less far"), "{t}");
        let (_, v) = compare_places_text(5, &json!({"x_span_px": null}), 4, &p(1));
        assert_eq!(v, json!("none"));
    }

    #[test]
    fn a_fall_off_the_screen_is_seen_in_the_y_byte() {
        assert_eq!(fell_off_y(&[100, 150, 230], 0), Some(2));
        assert_eq!(fell_off_y(&[100, 210, 10], 0), Some(2));
        assert_eq!(fell_off_y(&[100, 120], 0), None);
    }
}
