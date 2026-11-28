//! `gp theme look where`: where a clip draws a game's pictures, for a theme
//! maker measuring an arcade game drawn by its board's art. It plays one
//! corpus clip through `gp picture`'s lockstep with `--sprites` (or reads a
//! sprite list written before) and answers the questions a theme maker asks
//! of it: the stretches of frames a picture code is drawn and where, the
//! codes drawn near one with their commonest offsets, every code on a span
//! of frames, each code's count, sizes and rows, the stretches the codes'
//! x keeps changing, and how many of one code sit beside another, frame by
//! frame. No game's codes are in it: the codes are the ones asked for.
//!
//! A code of three hex digits (`04d`) is the board's sprite code as the
//! sprite list writes it, and matches it alone; a code of one or two
//! (`4d`) is its low byte, as a theme's names write it (sprite-mug-4d), and
//! matches every bank's.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use clap::Args;
use gp_evidence::Paths;
use serde_json::{Value, json};

use crate::Outcome;

#[derive(Args)]
pub struct WhereArgs {
    /// The picture codes to ask about, in hex: three digits (04d) for the
    /// sprite list's own code, one or two (4d) for every code with that low
    /// byte, as a theme names them (sprite-mug-4d). None: every code the
    /// clip draws, with how often, its sizes and its rows.
    pub codes: Vec<String>,
    /// The game: its folder, or its name in the games checkout.
    #[arg(long, required_unless_present = "sprites")]
    pub game: Option<PathBuf>,
    /// The corpus clip to play, by ID. Its sprite list is written under
    /// $GP_HOME/work/theme-where/GAME/CLIP.jsonl, for --sprites later.
    #[arg(long, requires = "game", required_unless_present = "sprites")]
    pub clip: Option<String>,
    /// Read a sprite list `gp picture --clip --sprites` (or this command)
    /// wrote, in place of playing a clip.
    #[arg(long, conflicts_with_all = ["game", "clip"])]
    pub sprites: Option<PathBuf>,
    /// Your romset of the game (default: one under $GP_ROM_DIR the game
    /// accepts).
    #[arg(long, requires = "clip")]
    pub rom: Option<PathBuf>,
    /// Only these frames, A-B; with /STEP (A-B/5), --each lists every
    /// STEP-th frame from A.
    #[arg(long)]
    pub frames: Option<String>,
    /// Every code drawn within N pixels of each code asked about (across
    /// and down), how often and its commonest offsets from it.
    #[arg(long)]
    pub near: Option<i64>,
    /// The stretches of at least 20 frames the codes asked about, taken as
    /// one picture (its first sprite on each frame), keep moving across on
    /// one row; a stretch ends after 6 frames still.
    #[arg(long)]
    pub moving: bool,
    /// How many sprites of this code sit within --near pixels (48 if not
    /// given) of each code asked about, frame by frame while it stays at
    /// one place, as runs: COUNTxFRAMES.
    #[arg(long)]
    pub count: Option<String>,
    /// Every frame (within --frames) with each code drawn on it and its x
    /// and y, in the sprite list's order; with codes, only theirs.
    #[arg(long)]
    pub each: bool,
    /// Read the original's sprites, not the model's (the same on a clip
    /// that plays exact).
    #[arg(long)]
    pub original: bool,
}

/// A frame of a sprite list: its number and its sprites, code and box
/// (x, y, width, height), in the list's order.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Line {
    pub frame: u64,
    pub sprites: Vec<(u16, [i64; 4])>,
}

/// A code asked about: its value, and whether it is a low byte only.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Code {
    pub text: String,
    value: u16,
    low: bool,
}

impl Code {
    pub(crate) fn parse(s: &str) -> Result<Code> {
        let t = s.trim().trim_start_matches("0x").to_ascii_lowercase();
        let value = u16::from_str_radix(&t, 16)
            .ok()
            .filter(|_| !t.is_empty() && t.len() <= 4)
            .ok_or_else(|| {
                anyhow!("{s} isn't a picture code in hex (04d, or 4d for its low byte)")
            })?;
        let low = t.len() <= 2;
        let text = if low {
            format!("{value:02x}")
        } else {
            format!("{value:03x}")
        };
        Ok(Code { text, value, low })
    }

    fn matches(&self, c: u16) -> bool {
        if self.low {
            c & 0xff == self.value
        } else {
            c == self.value
        }
    }

    /// Another code, named the way this one is written.
    fn name(&self, c: u16) -> String {
        if self.low {
            format!("{:02x}", c & 0xff)
        } else {
            format!("{c:03x}")
        }
    }
}

/// A stretch of frames a code is drawn on, with its first sprite's box on
/// the first.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Stretch {
    pub first: u64,
    pub last: u64,
    pub at: [i64; 4],
}

/// The stretches of consecutive frames `code` is drawn on.
pub(crate) fn stretches(lines: &[Line], code: &Code) -> Vec<Stretch> {
    let mut out: Vec<Stretch> = Vec::new();
    for l in lines {
        let Some((_, b)) = l.sprites.iter().find(|(c, _)| code.matches(*c)) else {
            continue;
        };
        match out.last_mut() {
            Some(s) if s.last + 1 == l.frame => s.last = l.frame,
            Some(s) if s.last == l.frame => {}
            _ => out.push(Stretch {
                first: l.frame,
                last: l.frame,
                at: *b,
            }),
        }
    }
    out
}

/// A code drawn near another: how often, and each offset (across, down)
/// with how often, the commonest first (ties in the order first seen).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Near {
    pub code: String,
    pub times: u64,
    pub offsets: Vec<([i64; 2], u64)>,
}

/// Every code within `within` pixels of `code`'s first sprite on each
/// frame it is drawn, the most often first (ties in the order first seen).
pub(crate) fn near(lines: &[Line], code: &Code, within: i64) -> Vec<Near> {
    let mut out: Vec<Near> = Vec::new();
    for l in lines {
        let Some((_, me)) = l.sprites.iter().find(|(c, _)| code.matches(*c)) else {
            continue;
        };
        for (c, b) in &l.sprites {
            let d = [b[0] - me[0], b[1] - me[1]];
            if code.matches(*c) || d[0].abs() > within || d[1].abs() > within {
                continue;
            }
            let name = code.name(*c);
            let i = match out.iter().position(|n| n.code == name) {
                Some(i) => i,
                None => {
                    out.push(Near {
                        code: name,
                        times: 0,
                        offsets: Vec::new(),
                    });
                    out.len() - 1
                }
            };
            let n = &mut out[i];
            n.times += 1;
            match n.offsets.iter_mut().find(|(o, _)| *o == d) {
                Some(o) => o.1 += 1,
                None => n.offsets.push((d, 1)),
            }
        }
    }
    // Stable sorts: ties keep the order first seen.
    out.sort_by_key(|n| std::cmp::Reverse(n.times));
    for n in &mut out {
        n.offsets.sort_by_key(|o| std::cmp::Reverse(o.1));
    }
    out
}

/// A stretch the codes keep moving across: its first and last frames, its
/// row, its x at each end, and the codes drawn for it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Moving {
    pub first: u64,
    pub last: u64,
    pub y: i64,
    pub from_x: i64,
    pub to_x: i64,
    pub codes: Vec<String>,
}

/// Frames a moving stretch spans at least, and frames still that end one.
const MOVING_FRAMES: u64 = 20;
const STILL_FRAMES: usize = 6;

/// The stretches of at least [`MOVING_FRAMES`] frames the first sprite of
/// any of `codes` keeps changing its x on one row, a stretch ending after
/// [`STILL_FRAMES`] frames without.
pub(crate) fn moving(lines: &[Line], codes: &[Code]) -> Vec<Moving> {
    let hit = |c: u16| codes.iter().any(|k| k.matches(c));
    let track: Vec<(u64, Option<[i64; 2]>)> = lines
        .iter()
        .map(|l| {
            let at = l
                .sprites
                .iter()
                .find(|(c, _)| hit(*c))
                .map(|(_, b)| [b[0], b[1]]);
            (l.frame, at)
        })
        .collect();
    let mut out = Vec::new();
    let mut emit = |start: usize, end: usize| {
        let (Some(a), Some(b)) = (track[start].1, track[end].1) else {
            return;
        };
        if track[end].0 - track[start].0 < MOVING_FRAMES {
            return;
        }
        let seen: BTreeSet<String> = lines[start..=end]
            .iter()
            .flat_map(|l| l.sprites.iter())
            .filter_map(|(c, _)| codes.iter().find(|k| k.matches(*c)).map(|k| k.name(*c)))
            .collect();
        out.push(Moving {
            first: track[start].0,
            last: track[end].0,
            y: a[1],
            from_x: a[0],
            to_x: b[0],
            codes: seen.into_iter().collect(),
        });
    };
    let (mut start, mut still) = (None::<usize>, 0usize);
    for i in 1..track.len() {
        let moved = match (track[i - 1].1, track[i].1) {
            (Some(p), Some(n)) => n[0] != p[0] && n[1] == p[1],
            _ => false,
        };
        if moved {
            start.get_or_insert(i - 1);
            still = 0;
        } else if let Some(s) = start {
            still += 1;
            if still > STILL_FRAMES {
                emit(s, i - still);
                start = None;
                still = 0;
            }
        }
    }
    if let Some(s) = start {
        emit(s, track.len() - 1 - still);
    }
    out
}

/// One place `code` stays drawn at, from `first` on, and how many sprites
/// of `counted` sit within `within` pixels of it on each frame, as runs
/// (count, frames).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Counted {
    pub first: u64,
    pub at: [i64; 2],
    pub frames: u64,
    pub runs: Vec<(usize, u64)>,
}

pub(crate) fn counted(lines: &[Line], code: &Code, counted: &Code, within: i64) -> Vec<Counted> {
    let mut live: BTreeMap<[i64; 2], (u64, Vec<usize>)> = BTreeMap::new();
    let mut done: Vec<([i64; 2], u64, Vec<usize>)> = Vec::new();
    for l in lines {
        let mut here = BTreeSet::new();
        for (c, b) in &l.sprites {
            if !code.matches(*c) {
                continue;
            }
            let at = [b[0], b[1]];
            if !here.insert(at) {
                continue;
            }
            let n = l
                .sprites
                .iter()
                .filter(|(k, o)| {
                    counted.matches(*k)
                        && (o[0] - at[0]).abs() <= within
                        && (o[1] - at[1]).abs() <= within
                })
                .count();
            live.entry(at)
                .or_insert_with(|| (l.frame, Vec::new()))
                .1
                .push(n);
        }
        let gone: Vec<[i64; 2]> = live
            .keys()
            .filter(|k| !here.contains(*k))
            .copied()
            .collect();
        for k in gone {
            let (f, v) = live.remove(&k).expect("a place drawn");
            done.push((k, f, v));
        }
    }
    done.extend(live.into_iter().map(|(k, (f, v))| (k, f, v)));
    done.sort_by_key(|d| (d.1, d.0));
    done.into_iter()
        .map(|(at, first, counts)| {
            let mut runs: Vec<(usize, u64)> = Vec::new();
            for n in &counts {
                match runs.last_mut() {
                    Some(r) if r.0 == *n => r.1 += 1,
                    _ => runs.push((*n, 1)),
                }
            }
            Counted {
                first,
                at,
                frames: counts.len() as u64,
                runs,
            }
        })
        .collect()
}

/// Each code a clip draws: how many times, on how many frames, at what
/// sizes (with how often) and on what rows.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Census {
    pub code: u16,
    pub times: u64,
    pub frames: u64,
    pub sizes: BTreeMap<[i64; 2], u64>,
    pub rows: BTreeSet<i64>,
}

pub(crate) fn census(lines: &[Line]) -> Vec<Census> {
    let mut by: BTreeMap<u16, Census> = BTreeMap::new();
    for l in lines {
        let mut on = BTreeSet::new();
        for (c, b) in &l.sprites {
            let e = by.entry(*c).or_insert_with(|| Census {
                code: *c,
                times: 0,
                frames: 0,
                sizes: BTreeMap::new(),
                rows: BTreeSet::new(),
            });
            e.times += 1;
            if on.insert(*c) {
                e.frames += 1;
            }
            *e.sizes.entry([b[2], b[3]]).or_default() += 1;
            e.rows.insert(b[1]);
        }
    }
    let mut out: Vec<Census> = by.into_values().collect();
    out.sort_by_key(|c| std::cmp::Reverse(c.times));
    out
}

/// A sprite list's lines (one JSON object a frame, as `gp picture
/// --sprites` writes them), the model's side or the original's.
pub(crate) fn parse_list(text: &str, original: bool) -> Result<Vec<Line>> {
    let side = if original { "original" } else { "model" };
    let mut out = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        if raw.trim().is_empty() {
            continue;
        }
        let v: Value =
            serde_json::from_str(raw).with_context(|| format!("line {} isn't JSON", i + 1))?;
        let frame = v["frame"]
            .as_u64()
            .ok_or_else(|| anyhow!("line {} has no frame", i + 1))?;
        let mut sprites = Vec::new();
        for s in v[side].as_array().into_iter().flatten() {
            let code = match &s[0] {
                Value::String(t) => u16::from_str_radix(t, 16).map_err(|_| {
                    anyhow!(
                        "frame {frame} draws {t}, a body and not a board's sprite code: the game isn't drawn by its board's art"
                    )
                })?,
                Value::Number(n) => n
                    .as_u64()
                    .and_then(|n| u16::try_from(n).ok())
                    .ok_or_else(|| anyhow!("frame {frame}: {n} isn't a sprite code"))?,
                _ => bail!("frame {frame}: a sprite with no code"),
            };
            let b: [i64; 4] = serde_json::from_value(s[1].clone())
                .with_context(|| format!("frame {frame}: a sprite with no box"))?;
            sprites.push((code, b));
        }
        out.push(Line { frame, sprites });
    }
    Ok(out)
}

/// `--frames A-B` or `A-B/STEP`: the first and last frames and the step.
fn frames_arg(s: &str) -> Result<(u64, u64, u64)> {
    let (range, step) = match s.split_once('/') {
        Some((r, st)) => (r, st.trim().parse::<u64>().ok().filter(|n| *n > 0)),
        None => (s, Some(1)),
    };
    let step =
        step.ok_or_else(|| anyhow!("--frames {s}: the step after / is a whole number over 0"))?;
    let (a, b) = match range.split_once('-') {
        Some((a, b)) => (a.trim().parse::<u64>(), b.trim().parse::<u64>()),
        None => (range.trim().parse::<u64>(), range.trim().parse::<u64>()),
    };
    match (a, b) {
        (Ok(a), Ok(b)) if a <= b => Ok((a, b, step)),
        _ => bail!("--frames {s} isn't A-B or A-B/STEP with A no more than B"),
    }
}

/// Play `clip` of the game at `game_arg` through `gp picture --sprites`,
/// into `$GP_HOME/work/theme-where/GAME/CLIP.jsonl`.
fn play(game_arg: &Path, clip: &str, rom: Option<&Path>, paths: &Paths) -> Result<PathBuf> {
    let game = crate::theme_kit::template_dir(game_arg)?;
    let game = game.canonicalize().unwrap_or(game);
    let (model, _) = crate::theme::read_model(&game)?;
    if !gp_look::board::names_board(&model) {
        bail!(
            "{} isn't drawn by an arcade board's art (its model names no board_sprite or present.board), so its sprite list names bodies, not picture codes",
            model.model.title
        );
    }
    let name = &model.model.name;
    let out = paths
        .home
        .join("work/theme-where")
        .join(name)
        .join(format!("{clip}.jsonl"));
    std::fs::create_dir_all(out.parent().expect("a folder"))?;
    let args = crate::picture::PictureArgs {
        game,
        run: format!("where-{name}"),
        module: None,
        model: None,
        assets: None,
        rom: rom.map(Path::to_owned),
        clip: Some(clip.to_owned()),
        top: 0,
        shots: None,
        shots_dir: None,
        sprites: Some(out.clone()),
        propose: false,
        line: None,
        apart: false,
    };
    let o = crate::picture::run(&args, paths)?;
    if let Some(e) = o.json["clips"][clip]["error"].as_str() {
        bail!("{clip} can't be played: {e}");
    }
    Ok(out)
}

fn xy(b: &[i64; 4]) -> String {
    format!("{},{}", b[0], b[1])
}

pub(crate) fn run(a: &WhereArgs, paths: &Paths) -> Result<Outcome> {
    let codes: Vec<Code> = a
        .codes
        .iter()
        .map(|c| Code::parse(c))
        .collect::<Result<_>>()?;
    let count = a.count.as_deref().map(Code::parse).transpose()?;
    if codes.is_empty() && (a.near.is_some() || a.moving || count.is_some()) {
        bail!("--near, --moving and --count ask about the codes named: name one or more");
    }
    let frames = a.frames.as_deref().map(frames_arg).transpose()?;
    let path = match (&a.sprites, &a.game, &a.clip) {
        (Some(p), _, _) => p.clone(),
        (None, Some(g), Some(c)) => play(g, c, a.rom.as_deref(), paths)?,
        _ => bail!("give --game and --clip, or --sprites"),
    };
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let mut lines = parse_list(&text, a.original)?;
    if let Some((from, to, _)) = frames {
        lines.retain(|l| (from..=to).contains(&l.frame));
    }
    let mut out = String::new();
    let side = if a.original {
        "the original's"
    } else {
        "the model's"
    };
    let _ = writeln!(
        out,
        "{}: {} frames of {side} sprites{}",
        path.display(),
        lines.len(),
        lines
            .first()
            .zip(lines.last())
            .map_or(String::new(), |(f, l)| format!(
                ", frames {} to {}",
                f.frame, l.frame
            ))
    );
    let mut j = json!({"sprites": path, "frames": lines.len(), "side": if a.original { "original" } else { "model" }});

    if codes.is_empty() && !a.each {
        let c = census(&lines);
        let _ = writeln!(
            out,
            "\nEvery code drawn, the most first: code, times, frames, sizes (WxH times), rows"
        );
        for e in &c {
            let sizes: Vec<String> = e
                .sizes
                .iter()
                .map(|(s, n)| format!("{}x{} {n}", s[0], s[1]))
                .collect();
            let rows: Vec<String> = e.rows.iter().map(i64::to_string).collect();
            let _ = writeln!(
                out,
                "  {:03x}  {} times on {} frames  sizes {}  rows {}",
                e.code,
                e.times,
                e.frames,
                sizes.join(", "),
                rows.join(" ")
            );
        }
        j["codes"] = c
            .iter()
            .map(|e| {
                json!({"code": format!("{:03x}", e.code), "times": e.times, "frames": e.frames,
                    "sizes": e.sizes.iter().map(|(s, n)| json!([s[0], s[1], n])).collect::<Vec<_>>(),
                    "rows": e.rows})
            })
            .collect();
    }

    let mut asked = Vec::new();
    for code in &codes {
        let st = stretches(&lines, code);
        let total: u64 = st.iter().map(|s| s.last - s.first + 1).sum();
        if st.is_empty() {
            let _ = writeln!(out, "\n{}: not drawn", code.text);
        } else {
            let _ = writeln!(
                out,
                "\n{}: drawn on {total} frames in {} stretches (first frame, last, x,y on the first):",
                code.text,
                st.len()
            );
            for s in &st {
                let _ = writeln!(out, "  {} to {} at {}", s.first, s.last, xy(&s.at));
            }
        }
        let mut cj = json!({
            "code": code.text,
            "frames": total,
            "stretches": st.iter().map(|s| json!({"first": s.first, "last": s.last, "at": s.at})).collect::<Vec<_>>(),
        });
        if let Some(n) = a.near {
            let found = near(&lines, code, n);
            let _ = writeln!(
                out,
                "  within {n} pixels of it ({} codes; code, times, offsets across,down):",
                found.len()
            );
            for x in &found {
                let offs: Vec<String> = x
                    .offsets
                    .iter()
                    .take(3)
                    .map(|(o, k)| format!("{},{} x{k}", o[0], o[1]))
                    .collect();
                let _ = writeln!(out, "    {} {} times, {}", x.code, x.times, offs.join("; "));
            }
            cj["near"] = found
                .iter()
                .map(|x| json!({"code": x.code, "times": x.times,
                    "offsets": x.offsets.iter().map(|(o, k)| json!([o[0], o[1], k])).collect::<Vec<_>>()}))
                .collect();
        }
        if let Some(k) = &count {
            let within = a.near.unwrap_or(48);
            let c = counted(&lines, code, k, within);
            let _ = writeln!(
                out,
                "  {} within {within} pixels, at each place it stays (COUNTxFRAMES):",
                k.text
            );
            for p in &c {
                let runs: Vec<String> = p.runs.iter().map(|(n, f)| format!("{n}x{f}")).collect();
                let _ = writeln!(
                    out,
                    "    frame {} at {},{} for {}: {}",
                    p.first,
                    p.at[0],
                    p.at[1],
                    p.frames,
                    runs.join(" ")
                );
            }
            cj["count"] = json!({"code": k.text, "within": within,
                "places": c.iter().map(|p| json!({"first": p.first, "at": p.at, "frames": p.frames,
                    "runs": p.runs.iter().map(|(n, f)| json!([n, f])).collect::<Vec<_>>()})).collect::<Vec<_>>()});
        }
        asked.push(cj);
    }
    if !codes.is_empty() {
        j["asked"] = Value::Array(asked);
    }

    if a.moving {
        let m = moving(&lines, &codes);
        let names: Vec<&str> = codes.iter().map(|c| c.text.as_str()).collect();
        let _ = writeln!(
            out,
            "\nWhere {} keeps moving across ({} stretches of {MOVING_FRAMES} frames or more):",
            names.join(" "),
            m.len()
        );
        for s in &m {
            let _ = writeln!(
                out,
                "  frames {} to {} on row {}, x {} to {}, drawn as {}",
                s.first,
                s.last,
                s.y,
                s.from_x,
                s.to_x,
                s.codes.join(" ")
            );
        }
        j["moving"] = m
            .iter()
            .map(|s| json!({"first": s.first, "last": s.last, "y": s.y, "from_x": s.from_x, "to_x": s.to_x, "codes": s.codes}))
            .collect();
    }

    if a.each {
        let (from, step) = frames.map_or((0, 1), |(f, _, s)| (f, s));
        let _ = writeln!(out, "\nEach frame (code@x,y):");
        let mut each = Vec::new();
        for l in lines.iter().filter(|l| (l.frame - from) % step == 0) {
            let shown: Vec<(u16, [i64; 4])> = l
                .sprites
                .iter()
                .filter(|(c, _)| codes.is_empty() || codes.iter().any(|k| k.matches(*c)))
                .copied()
                .collect();
            let words: Vec<String> = shown
                .iter()
                .map(|(c, b)| format!("{c:03x}@{}", xy(b)))
                .collect();
            let _ = writeln!(out, "  {} {}", l.frame, words.join(" "));
            each.push(json!({"frame": l.frame,
                "sprites": shown.iter().map(|(c, b)| json!([format!("{c:03x}"), b])).collect::<Vec<_>>()}));
        }
        j["each"] = Value::Array(each);
    }
    Ok(Outcome::ok(j, out))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A made-up sprite list, as `gp picture --sprites` writes it: a mug
    /// (04d) poured on frames 10 to 12 and again on 15, with a stream (00a)
    /// 16 pixels under it and a hand (13a) beside it on 11 and 12, and
    /// another bank's mug (14d) on frame 15.
    fn list() -> String {
        [
            r#"{"frame":10,"original":[],"model":[["04d",[40,100,16,16]],["00a",[40,116,8,8]]],"missing":[],"extra":[]}"#,
            r#"{"frame":11,"original":[],"model":[["04d",[40,100,16,16]],["00a",[40,116,8,8]],["13a",[24,100,16,16]]],"missing":[],"extra":[]}"#,
            r#"{"frame":12,"original":[],"model":[["04d",[42,100,16,16]],["04d",[90,60,16,16]],["00a",[42,116,8,8]],["13a",[26,100,16,16]]],"missing":[],"extra":[]}"#,
            r#"{"frame":13,"original":[],"model":[["00a",[200,10,8,8]]],"missing":[],"extra":[]}"#,
            r#"{"frame":14,"original":[],"model":[],"missing":[],"extra":[]}"#,
            r#"{"frame":15,"original":[],"model":[["14d",[60,30,16,16]],["04d",[70,30,16,16]],["00a",[70,46,8,8]]],"missing":[],"extra":[]}"#,
        ]
        .join("\n")
    }

    fn lines() -> Vec<Line> {
        parse_list(&list(), false).unwrap()
    }

    /// find_codes.py SPRITES 04d prints `04d [(10, 12, 40, 100), (15, 15,
    /// 70, 30)]`: the stretches, and x and y on each one's first frame.
    #[test]
    fn where_gives_find_codes_stretches() {
        let got: Vec<(u64, u64, i64, i64)> = stretches(&lines(), &Code::parse("04d").unwrap())
            .iter()
            .map(|s| (s.first, s.last, s.at[0], s.at[1]))
            .collect();
        assert_eq!(got, vec![(10, 12, 40, 100), (15, 15, 70, 30)]);
        // pours.py's question, the stream: its stretches 10 to 13 and 15.
        let st = stretches(&lines(), &Code::parse("0a").unwrap());
        let got: Vec<(u64, u64)> = st.iter().map(|s| (s.first, s.last)).collect();
        assert_eq!(got, vec![(10, 13), (15, 15)]);
        // Two digits match every bank's: 4d is 14d on frame 15 first.
        let st = stretches(&lines(), &Code::parse("4d").unwrap());
        assert_eq!(st[1].at, [60, 30, 16, 16]);
    }

    /// near.py SPRITES 4d prints `with 0a [((0, 16), 3), ((10, 16), 1)]`
    /// and `with 3a [((-16, 0), 2)]`: each code within 48 pixels of the
    /// mug's first sprite, how often, and its commonest offsets.
    #[test]
    fn where_near_gives_near_py_offsets() {
        let n = near(&lines(), &Code::parse("4d").unwrap(), 48);
        let near_of = |code: &str, times: u64, offsets: Vec<([i64; 2], u64)>| Near {
            code: code.to_owned(),
            times,
            offsets,
        };
        // Frame 12's second mug is the code asked about, never near itself;
        // frame 15's first mug is 14d (60,30), so the stream is 10,16 off.
        assert_eq!(
            n,
            vec![
                near_of("0a", 4, vec![([0, 16], 3), ([10, 16], 1)]),
                near_of("3a", 2, vec![([-16, 0], 2)]),
            ]
        );
        // Three digits name the others by theirs; 14d is 30 across from 04d.
        let n = near(&lines(), &Code::parse("04d").unwrap(), 48);
        let names: Vec<&str> = n.iter().map(|x| x.code.as_str()).collect();
        assert_eq!(names, vec!["00a", "13a", "14d"]);
        assert_eq!(n[2].offsets, vec![([-10, 0], 1)]);
        // A box of 15 leaves out the stream (16 down) and the hand (16
        // across): only 14d is left.
        assert_eq!(near(&lines(), &Code::parse("04d").unwrap(), 15).len(), 1);
    }

    #[test]
    fn where_counts_census_moving_and_frames() {
        let c = counted(
            &lines(),
            &Code::parse("04d").unwrap(),
            &Code::parse("0a").unwrap(),
            20,
        );
        let place = |first: u64, at: [i64; 2], runs: Vec<(usize, u64)>| Counted {
            first,
            at,
            frames: runs.iter().map(|r| r.1).sum(),
            runs,
        };
        assert_eq!(
            c,
            vec![
                place(10, [40, 100], vec![(1, 2)]),
                place(12, [42, 100], vec![(1, 1)]),
                place(12, [90, 60], vec![(0, 1)]),
                place(15, [70, 30], vec![(1, 1)]),
            ]
        );
        let cs = census(&lines());
        assert_eq!((cs[0].code, cs[0].times, cs[0].frames), (0x00a, 5, 5));
        assert_eq!(cs[1].code, 0x04d);
        assert_eq!((cs[1].times, cs[1].frames), (5, 4));
        assert_eq!(cs[1].rows, BTreeSet::from([30, 60, 100]));
        // A walk across one row for 30 frames, a stop, and a short one.
        let walk: Vec<Line> = (0..60u64)
            .map(|f| {
                let x = if f < 30 {
                    f as i64 * 2
                } else if f < 50 {
                    58
                } else {
                    58 + (f as i64 - 49)
                };
                Line {
                    frame: f,
                    sprites: vec![(if f % 2 == 0 { 0x036 } else { 0x037 }, [x, 80, 16, 32])],
                }
            })
            .collect();
        let m = moving(
            &walk,
            &[Code::parse("36").unwrap(), Code::parse("37").unwrap()],
        );
        assert_eq!(m.len(), 1);
        assert_eq!(
            (m[0].first, m[0].last, m[0].y, m[0].from_x, m[0].to_x),
            (0, 29, 80, 0, 58)
        );
        assert_eq!(m[0].codes, vec!["36", "37"]);
        assert_eq!(frames_arg("10-20/5").unwrap(), (10, 20, 5));
        assert_eq!(frames_arg("7").unwrap(), (7, 7, 1));
        assert!(frames_arg("20-10").is_err() && frames_arg("1-2/0").is_err());
        assert!(Code::parse("zz").is_err());
        assert!(parse_list(r#"{"frame":1,"model":[["player",[0,0,8,8]]]}"#, false).is_err());
    }
}
