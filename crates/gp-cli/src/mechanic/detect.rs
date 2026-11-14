//! The detectors the mechanic sessions ran over a clip (game-printer's
//! mechanic.rs): the player's box from the sprites, head hits' items,
//! stomps, hurts, side hits and rests, and the ROM dataset's names for RAM
//! bytes. `gp mechanic harvest` runs them over a whole recording. They read
//! the NES's sprite table.

use std::path::Path;

use anyhow::{Context as _, Result};
use serde_json::{Value, json};

/// What the ROM dataset says a RAM byte is, in a few words ("player y").
pub(crate) fn ram_names(dir: &Path) -> std::collections::BTreeMap<u16, String> {
    let mut out = std::collections::BTreeMap::new();
    let Ok(d) = crate::rom_dataset::read(dir) else {
        return out;
    };
    for r in d.rows("ram") {
        let s = |k: &str| match r.get(k) {
            Some(crate::rom_dataset::Cell::S(Some(v))) => v.clone(),
            _ => String::new(),
        };
        let Some(crate::rom_dataset::Cell::I(a)) = r.get("addr") else {
            continue;
        };
        let words: Vec<String> = [s("element"), s("quantity"), s("role")]
            .into_iter()
            .filter(|w| !w.is_empty())
            .collect();
        if !words.is_empty() {
            out.insert(*a as u16, words.join(" "));
        }
    }
    out
}

/// The player's x, y and vertical speed bytes from a ROM dataset.
pub(crate) fn player_bytes(dir: &Path) -> Result<(Option<u16>, u16, Option<u16>)> {
    // A game with no ROM dataset yet, or one whose dataset names no
    // player (an arcade romset's, which has no element reader), can state
    // its player's bytes in its package's state mapping
    // (entities.player[0].x and .y).
    if crate::rom_dataset::read(dir).map_or(true, |d| {
        !d.rows("ram").iter().any(|r| {
            matches!(r.get("element"), Some(crate::rom_dataset::Cell::S(Some(e))) if e == "player")
                && matches!(r.get("quantity"), Some(crate::rom_dataset::Cell::S(Some(q))) if q == "y")
        })
    }) && let Some(game) = dir.file_name().and_then(|g| g.to_str())
        && let Some(games) = crate::roms_cmd::games_dir(None)
        && let Some(pkg) = crate::labels::game_folder(&games, game)
        && let Ok(text) = std::fs::read_to_string(pkg.join("mapping.toml"))
        && let Ok(m) = toml::from_str::<toml::Table>(&text)
    {
        let field = |path: &str| -> Option<u16> {
            m.get("field")?.as_array()?.iter().find_map(|f| {
                (f.get("path")?.as_str()? == path)
                    .then(|| f.get("ram")?.as_array()?.first()?.as_integer())
                    .flatten()
                    .map(|a| a as u16)
            })
        };
        if let Some(y) = field("entities.player[0].y") {
            return Ok((
                field("entities.player[0].x"),
                y,
                field("entities.player[0].vy"),
            ));
        }
    }
    let d = crate::rom_dataset::read(dir).with_context(|| {
        format!(
            "no ROM dataset at {}: write one with gp dataset rom ROM --out {}",
            dir.display(),
            dir.display()
        )
    })?;
    let (mut x, mut y, mut vy) = (None, None, None);
    for r in d.rows("ram") {
        let s = |k: &str| match r.get(k) {
            Some(crate::rom_dataset::Cell::S(Some(v))) => v.clone(),
            _ => String::new(),
        };
        let Some(crate::rom_dataset::Cell::I(a)) = r.get("addr") else {
            continue;
        };
        if s("element") != "player" {
            continue;
        }
        let part = s("part");
        match s("quantity").as_str() {
            "x" if part != "hi" => x = Some(*a as u16),
            "y" => y = Some(*a as u16),
            "speed_y" => vy = Some(*a as u16),
            _ => {}
        }
    }
    let y = y.context("the dataset has no player y byte; a demo needs one to see a jump")?;
    Ok((x, y, vy))
}

/// The frames in a clip where the player's rise stopped dead, with the RAM
/// bytes that changed there and the sprites that appeared after.
/// Sprites that touch, merged into boxes: [x, y, w, h] on the screen.
pub(crate) fn sprite_boxes(oam: &[u8; 256], tall: i32) -> Vec<[i32; 4]> {
    let b: Vec<[i32; 4]> = oam
        .chunks(4)
        .filter(|e| e[0] < 0xEF)
        .map(|e| [i32::from(e[3]), i32::from(e[0]) + 1, 8, tall])
        .collect();
    merge_boxes(b)
}

/// An arcade board's sprite RAM as boxes: its sprite list's 16x16 sprites,
/// those that touch merged.
pub(crate) fn arcade_boxes(sprite_ram: &[u8]) -> Vec<[i32; 4]> {
    merge_boxes(
        gp_arcade::video::sprite_list(sprite_ram)
            .iter()
            .map(|s| [i32::from(s.x), i32::from(s.y), 16, 16])
            .collect(),
    )
}

/// Sprite rectangles ([x, y, w, h]) that touch, merged into boxes: the
/// NES's sprite table's ([`sprite_boxes`]) or an arcade sprite list's.
pub(crate) fn merge_boxes(mut b: Vec<[i32; 4]>) -> Vec<[i32; 4]> {
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

/// Each frame's player box: the one whose top keeps the same distance from
/// the player's y byte. The distance is the median over the clip of each
/// frame's nearest box; each frame then takes the box nearest it, if one is
/// within 8 px.
pub(crate) fn player_track(
    boxes: &[Vec<[i32; 4]>],
    ys: &[i32],
    start: usize,
) -> Vec<Option<usize>> {
    let mut offs: Vec<i32> = Vec::new();
    for (b, y) in boxes.iter().zip(ys).skip(start) {
        if let Some(o) = b.iter().map(|r| r[1] - y).min_by_key(|o| o.abs()) {
            offs.push(o);
        }
    }
    offs.sort_unstable();
    let off = offs.get(offs.len() / 2).copied().unwrap_or(0);
    // A box that jumps across the screen from the last one isn't the player.
    let mut last_x: Option<i32> = None;
    boxes
        .iter()
        .zip(ys)
        .enumerate()
        .map(|(k, (b, y))| {
            if k < start {
                return None;
            }
            let p = b
                .iter()
                .enumerate()
                .filter(|(_, r)| last_x.is_none_or(|x| (r[0] - x).abs() <= 24))
                .min_by_key(|(_, r)| (r[1] - y - off).abs())
                .filter(|(_, r)| (r[1] - y - off).abs() <= 8)
                .map(|(i, _)| i);
            if let Some(i) = p {
                last_x = Some(b[i][0]);
            }
            p
        })
        .collect()
}

/// The player's usual box size: the most common (w, h) of its boxes.
pub(crate) fn usual_size(
    boxes: &[Vec<[i32; 4]>],
    track: &[Option<usize>],
    start: usize,
) -> (i32, i32) {
    let mut n: std::collections::BTreeMap<(i32, i32), usize> = std::collections::BTreeMap::new();
    for (b, t) in boxes.iter().zip(track).skip(start) {
        if let Some(i) = t {
            *n.entry((b[*i][2], b[*i][3])).or_default() += 1;
        }
    }
    n.into_iter()
        .max_by_key(|(_, c)| *c)
        .map(|(k, _)| k)
        .unwrap_or((16, 16))
}

/// Items: after each head hit, a new box that rises out of the spot above
/// the player's head and stays 60 frames or more. Its path (rise, then each
/// stretch across with its speed), and how it ended: touching the player's
/// box (with how many frames after that nothing on screen moved, and the
/// player's box height before and after), leaving the screen, or still
/// there when the clip ends.
pub(crate) fn items(
    boxes: &[Vec<[i32; 4]>],
    track: &[Option<usize>],
    hits: &[usize],
    start: usize,
) -> Vec<(usize, Value)> {
    let mut out = Vec::new();
    let pbox = |k: usize| track[k].map(|t| boxes[k][t]);
    let others = |k: usize| -> Vec<[i32; 4]> {
        boxes[k]
            .iter()
            .enumerate()
            .filter(|(j, _)| Some(*j) != track[k])
            .map(|(_, r)| *r)
            .collect()
    };
    for &i in hits {
        let Some(p) = pbox(i).or_else(|| pbox(i - 1)) else {
            continue;
        };
        let before = others(i - 1);
        // Candidates: boxes from i to i+40 above the head, not there before.
        let mut found = None;
        'look: for k in i..(i + 40).min(boxes.len()) {
            for c in others(k) {
                let above = c[1] + c[3] / 2 < p[1] + p[3] / 2 && c[1] >= p[1] - 64;
                let over = (c[0] + c[2] / 2 - (p[0] + p[2] / 2)).abs() <= 24;
                let new = !before
                    .iter()
                    .any(|b| (b[0] - c[0]).abs() <= 2 && (b[1] - c[1]).abs() <= 2);
                // No bigger than the player: a box that is is two
                // things drawn as one.
                let alone = c[2] <= p[2] + 2 && c[3] <= p[3];
                if above && over && new && alone && c[2] >= 12 && c[3] >= 12 {
                    // Follow it frame to frame by the nearest box of its size.
                    let mut path = vec![(k, c)];
                    let mut cur = c;
                    let mut end = "still there when the clip ends";
                    let mut touch = None;
                    for (f, fb) in boxes.iter().enumerate().skip(k + 1) {
                        if let Some(pb) = pbox(f)
                            && cur[0] < pb[0] + pb[2]
                            && pb[0] < cur[0] + cur[2]
                            && cur[1] < pb[1] + pb[3]
                            && pb[1] < cur[1] + cur[3]
                        {
                            touch = Some(f);
                            end = "touched the player";
                            break;
                        }
                        let next = others(f)
                            .into_iter()
                            .filter(|r| r[2] == cur[2] && r[3] == cur[3])
                            .min_by_key(|r| (r[0] - cur[0]).abs() + (r[1] - cur[1]).abs())
                            .filter(|r| (r[0] - cur[0]).abs() + (r[1] - cur[1]).abs() <= 14);
                        match next {
                            Some(n) => {
                                cur = n;
                                path.push((f, n));
                            }
                            None => {
                                // Drawn as one with something bigger where it
                                // was (the player and it merged): a touch.
                                let merged = fb.iter().any(|r| {
                                    r[2] * r[3] > cur[2] * cur[3]
                                        && cur[0] < r[0] + r[2]
                                        && r[0] < cur[0] + cur[2]
                                        && cur[1] < r[1] + r[3]
                                        && r[1] < cur[1] + cur[3]
                                        && r[1] + r[3] > cur[1] + cur[3]
                                });
                                if merged {
                                    touch = Some(f);
                                    end = "touched the player";
                                } else {
                                    end = "left the screen or vanished";
                                }
                                break;
                            }
                        }
                    }
                    if path.len() < 60 && touch.is_none() {
                        continue;
                    }
                    // Stretches across: runs of frames with the same sign of
                    // x movement, with their speed.
                    let mut legs: Vec<Value> = Vec::new();
                    let mut j = 1;
                    while j < path.len() {
                        let sign = (path[j].1[0] - path[j - 1].1[0]).signum();
                        let a = j - 1;
                        while j < path.len() && (path[j].1[0] - path[j - 1].1[0]).signum() == sign {
                            j += 1;
                        }
                        let b = j - 1;
                        let frames = (path[b].0 - path[a].0).max(1) as f64;
                        let dx = path[b].1[0] - path[a].1[0];
                        let dy = path[b].1[1] - path[a].1[1];
                        if b - a >= 4 {
                            legs.push(json!({"from": path[a].0 - start, "frames": b - a,
                                "dx_px": dx, "dy_px": dy,
                                "px_a_frame": ((f64::from(dx) / frames) * 100.0).round() / 100.0}));
                        }
                    }
                    let rose =
                        path[0].1[1] - path.iter().map(|(_, r)| r[1]).min().unwrap_or(path[0].1[1]);
                    let mut v = json!({"frame": k - start, "size": [c[2], c[3]], "rose_px": rose,
                        "legs": legs, "end": end, "path": path.iter().map(|(f, r)| json!([f - start, r[0], r[1]])).collect::<Vec<_>>()});
                    if let Some(t) = touch {
                        // Frames after the touch in which no box moved.
                        let still = (t + 1..boxes.len())
                            .take_while(|&f| boxes[f] == boxes[f - 1])
                            .count();
                        let h_before = pbox(t - 1).map(|r| r[3]);
                        let h_after = (t + 1..(t + 120).min(boxes.len()))
                            .rev()
                            .find_map(pbox)
                            .map(|r| r[3]);
                        v["touch_frame"] = json!(t - start);
                        v["frozen_frames"] = json!(still);
                        v["player_h"] = json!([h_before, h_after]);
                        out.push((t, v));
                    } else {
                        out.push((k, v));
                    }
                    found = Some(());
                    break 'look;
                }
            }
        }
        let _ = found;
    }
    out
}

/// Stomps: the player falling, then rising at once (3 px a frame or more
/// turned around) with no fresh A press, off a box under its feet. What
/// that box did in the next 30 frames: flattened (shorter), gone, or kept.
pub(crate) fn stomps(
    ys: &[i32],
    pads: &[u8],
    boxes: &[Vec<[i32; 4]>],
    track: &[Option<usize>],
    start: usize,
) -> Vec<(usize, Value)> {
    let mut out = Vec::new();
    for i in start.max(2)..ys.len().saturating_sub(1) {
        let (d0, d1) = (ys[i] - ys[i - 1], ys[i + 1] - ys[i]);
        if !(d0 >= 2 && d1 <= -2) {
            continue;
        }
        let fresh_a = |k: usize| {
            pads.get(k).is_some_and(|p| p & 1 != 0) && pads.get(k - 1).is_some_and(|p| p & 1 == 0)
        };
        if fresh_a(i) || fresh_a(i + 1) {
            continue;
        }
        // Something under its feet: a box there, or the player's own box
        // reaching below its usual height (the two drawn as one).
        let (_, uh) = usual_size(boxes, track, start);
        let near = |k: usize| track[k].map(|t| boxes[k][t]);
        let Some(p) = near(i).or_else(|| near(i - 1)) else {
            continue;
        };
        let feet = p[1] + uh;
        let separate = boxes[i].iter().enumerate().any(|(k, c)| {
            Some(k) != track[i]
                && c[0] < p[0] + p[2]
                && p[0] < c[0] + c[2]
                && (c[1] - feet).abs() <= 10
        });
        let merged = [i - 1, i, i + 1]
            .iter()
            .filter_map(|&k| near(k))
            .any(|r| r[3] >= uh + 6);
        if !separate && !merged {
            continue;
        }
        // What was under it, once the player is up and away: a short box
        // left at that spot is flattened, no box there is gone.
        let spot = [p[0] - 8, feet - 4, p[2] + 16, 24];
        let mut what = "kept its shape";
        let mut seen = false;
        for k in i + 4..(i + 30).min(boxes.len()) {
            let there: Vec<&[i32; 4]> = boxes[k]
                .iter()
                .enumerate()
                .filter(|(j, _)| Some(*j) != track[k])
                .map(|(_, r)| r)
                .filter(|r| {
                    r[0] < spot[0] + spot[2]
                        && spot[0] < r[0] + r[2]
                        && r[1] < spot[1] + spot[3]
                        && spot[1] < r[1] + r[3]
                })
                .collect();
            if there.iter().any(|r| r[3] <= uh / 2) {
                what = "flattened";
                break;
            }
            seen |= !there.is_empty();
        }
        if what != "flattened" && !seen {
            what = "was gone";
        }
        let c = spot;
        let top = (i + 1..ys.len())
            .take_while(|&k| ys[k] <= ys[k - 1])
            .map(|k| ys[k])
            .min()
            .unwrap_or(ys[i]);
        out.push((
            i,
            json!({"frame": i - start, "bounced_px": ys[i] - top, "under": what,
                             "box": c, "under_was_hit": true}),
        ));
    }
    out
}

/// Hurts: the player's box changing height while the pad held no up or
/// down (shrinking or growing), and stretches where the player's box blinks
/// (missing every few frames for at least 8 frames).
pub(crate) fn hurts(
    boxes: &[Vec<[i32; 4]>],
    track: &[Option<usize>],
    start: usize,
) -> Vec<(usize, Value)> {
    let mut out = Vec::new();
    let (uw, _) = usual_size(boxes, track, start);
    // Height only from frames where the box is the player alone (its usual
    // width), and a change only when both heights hold 20 frames.
    let h = |k: usize| {
        track[k]
            .map(|i| boxes[k][i])
            .filter(|r| (r[2] - uw).abs() <= 1)
            .map(|r| r[3])
    };
    let alone: Vec<(usize, i32)> = (start..boxes.len())
        .filter_map(|k| h(k).map(|v| (k, v)))
        .collect();
    let mut run_start = 0;
    for j in 1..alone.len() {
        if alone[j].1 != alone[j - 1].1 {
            let held_before = alone[j - 1].0 - alone[run_start].0 >= 20;
            let after = alone[j..]
                .iter()
                .take_while(|(_, v)| *v == alone[j].1)
                .last()
                .map_or(0, |(k, _)| *k);
            if held_before && after >= alone[j].0 + 20 {
                let (before, now) = (alone[j - 1].1, alone[j].1);
                out.push((alone[j].0, json!({"frame": alone[j].0 - start,
                    "kind": if now < before { "shrank" } else { "grew" }, "from_h": before, "to_h": now})));
            }
            run_start = j;
        }
    }
    // Off the bottom of the screen: the player's box reached the last rows
    // and then wasn't there for 5 frames.
    for k in start + 1..boxes.len().saturating_sub(5) {
        if let Some(r) = track[k - 1].map(|i| boxes[k - 1][i])
            && r[1] + r[3] >= 224
            && (k..k + 5).all(|j| track[j].is_none())
        {
            out.push((
                k,
                json!({"frame": k - start, "kind": "fell off the screen"}),
            ));
            break;
        }
    }
    // Blinking: in a 16-frame window the player is missing on 4 or more
    // frames, in at least 3 separate gaps.
    let mut k = start;
    while k + 16 <= boxes.len() {
        let miss: Vec<bool> = (k..k + 16).map(|j| track[j].is_none()).collect();
        let gaps = miss.windows(2).filter(|w| !w[0] && w[1]).count();
        if miss.iter().filter(|m| **m).count() >= 4 && gaps >= 3 {
            let mut end = k + 16;
            while end + 8 <= boxes.len() && (end..end + 8).any(|j| track[j].is_none()) {
                end += 8;
            }
            out.push((
                k,
                json!({"frame": k - start, "kind": "blinked", "frames": end - k}),
            ));
            k = end;
            continue;
        }
        k += 1;
    }
    out
}

/// Where the player ran into something from the side: it was moving across
/// and then didn't move for 4 frames while the pad still held that way.
pub(crate) fn walls(xs: &[i32], ys: &[i32], pads: &[u8], start: usize) -> Vec<Value> {
    const STOP: usize = 4;
    let dx = |i: usize| (xs[i] - xs[i - 1] + 384) % 256 - 128;
    let mut out = Vec::new();
    let mut i = start.max(5);
    while i + STOP < xs.len() {
        let moving = (1..=4).map(|k| dx(i - k + 1)).collect::<Vec<_>>();
        let net: i32 = moving.iter().sum();
        let dir = if net >= 2 && moving.iter().all(|d| *d >= 0) {
            1
        } else if net <= -2 && moving.iter().all(|d| *d <= 0) {
            -1
        } else {
            0
        };
        // Right is bit 7 of the pad, left bit 6 (input log order).
        let held = |k: usize| {
            let p = pads.get(k).copied().unwrap_or(0);
            (dir > 0 && p & 0x80 != 0) || (dir < 0 && p & 0x40 != 0)
        };
        // Stopped: no headway that way over the next frames (a wall may
        // push back a pixel), with the pad still held toward it.
        let ahead = |k: usize| dx(k) * dir;
        if dir != 0 && (i + 1..=i + STOP).all(|k| ahead(k) <= 0 && ahead(k) >= -1 && held(k)) {
            let speed: f64 = moving.iter().map(|d| f64::from(*d)).sum::<f64>() / 4.0;
            out.push(json!({
                "frame": i + 1 - start,
                "side": if dir > 0 { "right" } else { "left" },
                "in_air": ys[i + 1] != ys[i] || ys[i] != ys[i - 1],
                "speed_px_a_frame": (speed * 100.0).round() / 100.0,
            }));
            i += 30;
            continue;
        }
        i += 1;
    }
    out
}

/// Where the player stood still in a clip and how it got from one place to
/// the next: each rest's height above the lowest one, and each move between
/// rests as a jump (A pressed as it left) or a fall (it left without A,
/// walking off an edge).
pub(crate) fn rests(ys: &[i32], pads: &[u8], start: usize) -> Vec<Value> {
    // Longer than a jump hangs at its top (about 6 frames).
    const STILL: usize = 8;
    let mut runs: Vec<(usize, usize, i32)> = Vec::new();
    let mut i = start;
    while i < ys.len() {
        let mut j = i;
        while j + 1 < ys.len() && ys[j + 1] == ys[i] {
            j += 1;
        }
        if j + 1 - i >= STILL {
            runs.push((i, j, ys[i]));
        }
        i = j + 1;
    }
    let low = runs.iter().map(|r| r.2).max().unwrap_or(0);
    let mut out = Vec::new();
    for (k, &(a, b, y)) in runs.iter().enumerate() {
        let mut v = json!({"from": a - start, "to": b - start, "y": y, "up_px": low - y});
        if let Some(&(na, _, ny)) = runs.get(k + 1) {
            let jumped = pads.get(b + 1).is_some_and(|p| p & 1 != 0)
                || pads.get(b).is_some_and(|p| p & 1 != 0);
            v["then"] = json!({
                "how": if jumped { "jump" } else if ny > y { "fall" } else { "rise" },
                "frames": na - b - 1,
                "to_up_px": low - ny,
            });
        }
        out.push(v);
    }
    out
}
