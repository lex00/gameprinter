//! `gp mechanic harvest GAME RECORDING`: mechanics from a long recording
//! (a TAS run in the store). The recording is replayed once; every detector
//! the mechanic sessions use (head hits, side hits, raised landings, stomps,
//! items, hurts and deaths) runs over the whole of it, with the RAM bytes
//! each event changed for good. Also found: stretches where the player
//! doesn't move while the pad changes, moments when many bytes change and
//! no detector says why, routines that run for the first time, and object
//! types that appear for the first time. Levels are cut where the
//! template's `observe.toml` [level] key changes (template data, not the
//! tool's). Each kind of event gets an example clip per level, a session
//! `harvest-LEVEL` with a measure.json the page can draw, and the events are
//! grouped into candidate mechanics across levels, matched against the
//! queue's rows.
//!
//! Ported from game-printer's harvest.rs. The level key, the slots and the
//! ROM come from the game's package in the games checkout (its
//! observe.toml, mapping.toml and package.toml), where game-printer read
//! templates/GAME; the sessions go where gameprinter's mechanic sessions
//! are. The detectors are crate::mechanic::detect. The recording is
//! replayed on gp-nes or, for an arcade game, on gp-arcade
//! ([`replay_arcade`]): its work RAM, its sprite list and its main Z80's
//! trace stand for the NES's RAM, sprite table and 6502 routines.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde_json::{Value, json};

use crate::mechanic::detect as m;
use crate::mechanic::session;
use gp_evidence::Paths;

/// Frames before an event its clip starts, and the clip's length.
const LEAD_IN: usize = 60;
const CLIP: usize = 180;

struct Level {
    ram: Vec<u16>,
    when: Option<gp_runtime::select::Condition>,
    names: BTreeMap<Vec<u8>, String>,
}

fn level_def(game_dir: &Path) -> Option<Level> {
    let t: toml::Value =
        toml::from_str(&fs::read_to_string(game_dir.join("observe.toml")).ok()?).ok()?;
    let l = t.get("level")?;
    let ram: Vec<u16> = l
        .get("ram")?
        .as_array()?
        .iter()
        .filter_map(|v| v.as_integer().map(|n| n as u16))
        .collect();
    let when = l
        .get("when")
        .and_then(|w| w.as_str())
        .and_then(|w| gp_runtime::select::Condition::parse(w).ok());
    let mut names = BTreeMap::new();
    for n in l
        .get("names")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
    {
        let a = n.as_array()?;
        let key: Vec<u8> = a
            .first()?
            .as_str()?
            .split_whitespace()
            .filter_map(|h| u8::from_str_radix(h.trim_start_matches("0x"), 16).ok())
            .collect();
        names.insert(key, a.get(1)?.as_str()?.to_owned());
    }
    Some(Level { ram, when, names })
}

fn holds(c: &gp_runtime::select::Condition, ram: &[u8], base: u16) -> bool {
    c.eval(&mut |sel| match (sel.name().as_str(), sel.last_index()?) {
        ("ram", Some(gp_runtime::select::Index::At(n)))
            if (i64::from(base)..i64::from(base) + 0x800).contains(&n) =>
        {
            Ok(i64::from(ram[(n - i64::from(base)) as usize]))
        }
        _ => Err("only ram[N]".into()),
    })
    .unwrap_or(false)
}

/// The object slots, from the template's mapping.toml [slots] (count,
/// active, id): template data.
fn slots_def(game_dir: &Path) -> Option<(usize, u16, u16)> {
    let t: toml::Value =
        toml::from_str(&fs::read_to_string(game_dir.join("mapping.toml")).ok()?).ok()?;
    let s = t.get("slots")?;
    Some((
        s.get("count")?.as_integer()? as usize,
        s.get("active")?.as_integer()? as u16,
        s.get("id")?.as_integer()? as u16,
    ))
}

struct Event {
    frame: usize,
    kind: String,
    what: String,
    data: Value,
}

pub(crate) fn harvest(
    paths: &Paths,
    game: &str,
    recording: &str,
    add: bool,
    top: usize,
) -> Result<(Value, String)> {
    let t0 = std::time::Instant::now();
    if !session::name_ok(game) {
        bail!("a game name is lower-case letters, digits and dashes: {game}");
    }
    let games =
        crate::roms_cmd::games_dir(None).context("no games checkout found: set GP_GAMES_DIR")?;
    let game_dir = crate::labels::game_folder(&games, game)
        .with_context(|| format!("no game {game} in {}", games.display()))?;
    let rom = crate::clip_score::rom_of(paths, &game_dir, None)?;
    // An arcade game's recording replays on gp-arcade ([`replay_arcade`]);
    // its RAM window is the work RAM at E000, so an address the game's
    // files name is `base` on from the window's index.
    let arcade = crate::original::Platform::of_file(&rom) == crate::original::Platform::Arcade;
    let base: u16 = if arcade {
        crate::arcade_stage0::RAM_BASE
    } else {
        0
    };
    let ix = |a: u16| usize::from(a.wrapping_sub(base));
    let store = gp_evidence::store::Store::open(&paths.store)?;
    let entry = gp_evidence::recordings::resolve(&store, recording)?;
    let manifest = gp_evidence::recordings::load_manifest(&store, &entry.id)?;
    let log = gp_evidence::InputLog::parse(
        &String::from_utf8(store.get(&manifest.input_log)?).context("the input log isn't text")?,
    )?;
    let dir = paths.home.join("work/datasets").join(game);
    let (xb, yb, _) = m::player_bytes(&dir)?;
    let names = m::ram_names(&dir);
    let level = level_def(&game_dir);
    let slots = slots_def(&game_dir);

    let n = log.frames.len();
    let mut seen_routines: BTreeSet<u32> = BTreeSet::new();
    let mut events: Vec<Event> = Vec::new();
    let mut seen_ids: BTreeSet<u8> = BTreeSet::new();
    // Branches the dataset lists, by CPU address, on a board without banks
    // (a banked board's branches need the bank too; they're left out).
    let data = crate::rom_dataset::read(&dir).ok();
    let mut branch_at: std::collections::HashMap<u16, (String, String)> =
        std::collections::HashMap::new();
    for r in data.iter().flat_map(|d| d.rows("branches")) {
        let get = |k: &str| match r.get(k) {
            Some(crate::rom_dataset::Cell::S(Some(v))) => v.clone(),
            _ => String::new(),
        };
        let pc = get("pc");
        if pc.contains(':') {
            continue;
        }
        if let Ok(a) = u16::from_str_radix(pc.trim_start_matches('$'), 16) {
            branch_at.insert(a, (get("op").to_lowercase(), get("routine")));
        }
    }
    // One replay: RAM, sprite boxes, scroll, the routines entered and the
    // branches run, each frame.
    let r = if arcade {
        replay_arcade(
            &rom,
            &log,
            &branch_at,
            &crate::arcade_stage0::dataset_routines(data.as_ref()),
        )?
    } else {
        replay_nes(&rom, &manifest, &log, &branch_at)?
    };
    let Replay {
        ram,
        boxes,
        scroll,
        entered,
        ran_branches,
        branch_n,
        oam_page,
    } = r;
    let mut levels: Vec<Option<String>> = Vec::with_capacity(n);
    for (i, w) in ram.iter().enumerate() {
        let lv = level.as_ref().and_then(|l| {
            if l.when.as_ref().is_some_and(|c| !holds(c, w, base)) {
                return None;
            }
            let key: Vec<u8> = l.ram.iter().map(|a| w[ix(*a)]).collect();
            Some(l.names.get(&key).cloned().unwrap_or_else(|| {
                key.iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<Vec<_>>()
                    .join("")
            }))
        });
        // Routines run for the first time, once play has started (a level).
        let fresh: Vec<u32> = entered[i]
            .iter()
            .copied()
            .filter(|o| seen_routines.insert(*o))
            .collect();
        if lv.is_some() && !fresh.is_empty() && i > 0 {
            events.push(Event {
                frame: i,
                kind: "new-code".into(),
                what: format!("{} routine(s) ran for the first time", fresh.len()),
                data: json!({"routines": fresh.iter().map(|o| if arcade { format!("{o:04X}") } else { format!("{o:05X}") }).collect::<Vec<_>>()}),
            });
        }
        if let (Some((count, active, id)), Some(_)) = (slots, &lv) {
            for k in 0..count {
                if w[ix(active) + k] != 0 {
                    let t = w[ix(id) + k];
                    if seen_ids.insert(t) {
                        events.push(Event {
                            frame: i,
                            kind: "new-object".into(),
                            what: format!("object type {t:#04x} appeared for the first time"),
                            data: json!({"type": t}),
                        });
                    }
                }
            }
        }
        levels.push(lv);
    }
    let replay_secs = t0.elapsed().as_secs_f64();

    let ys: Vec<i32> = ram.iter().map(|r| i32::from(r[ix(yb)])).collect();
    // The detectors read the NES pad's bits; an arcade frame's controls
    // are put in them (the stick as the d-pad, tap as A, Start as Start,
    // the coin as Select).
    let pads: Vec<u8> = log
        .frames
        .iter()
        .map(|f| {
            if arcade {
                nes_pad(f.pads[0])
            } else {
                f.pads[0]
            }
        })
        .collect();
    let in_level = |i: usize| levels[i].is_some();
    let track: Vec<Option<usize>> = m::player_track(&boxes, &ys, 0)
        .into_iter()
        .enumerate()
        .map(|(i, t)| t.filter(|_| in_level(i)))
        .collect();
    // Bytes the game rewrites as the screen scrolls: most of their changes
    // come within 3 frames after the scroll crosses a 16 px column.
    let mut near = vec![false; n];
    for i in 1..n {
        if scroll[i] / 8 != scroll[i - 1] / 8 {
            near[i..(i + 3).min(n)].fill(true);
        }
    }
    let mut scrolled = vec![false; 0x800];
    for (a, sc) in scrolled.iter_mut().enumerate() {
        let (mut all, mut at) = (0u32, 0u32);
        for i in 1..n {
            if ram[i][a] != ram[i - 1][a] {
                all += 1;
                at += u32::from(near[i]);
            }
        }
        *sc = all >= 8 && at * 10 >= all * 8;
    }
    let scroll_bytes = scrolled.iter().filter(|b| **b).count();
    if std::env::var_os("GP_TRACE_SCROLL").is_some() {
        let near_share = near.iter().filter(|b| **b).count() as f64 / n as f64;
        let list: Vec<String> = (0..0x800)
            .filter(|a| scrolled[*a])
            .map(|a| format!("{a:04X}"))
            .collect();
        eprintln!(
            "near a column crossing on {:.0}% of frames; scroll bytes: {}",
            near_share * 100.0,
            list.join(" ")
        );
        for a in [0x346usize, 0x342, 0x721, 0x6A0] {
            let (mut all, mut at) = (0, 0);
            for i in 1..n {
                if ram[i][a] != ram[i - 1][a] {
                    all += 1;
                    if near[i] {
                        at += 1;
                    }
                }
            }
            eprintln!("{a:04X}: {at} of {all} changes near a crossing");
        }
    }
    // Bytes that step to a new value they keep, on more than one frame in
    // 300 of play: a running state (a timer, a counter, sound), which
    // changes around any moment and explains none.
    let mut churn = vec![false; 0x800];
    let play = (0..n).filter(|&i| levels[i].is_some()).count().max(1);
    for (a, c) in churn.iter_mut().enumerate() {
        let steps = (1..n.saturating_sub(60))
            .filter(|&i| {
                ram[i][a] != ram[i - 1][a] && (i + 1..i + 60).all(|k| ram[k][a] == ram[i][a])
            })
            .count();
        *c = steps * 300 > play;
    }
    let churn_bytes = churn.iter().filter(|b| **b).count();
    let skip = |a: usize| {
        let role = names
            .get(&(a as u16 + base))
            .map(String::as_str)
            .unwrap_or("");
        scrolled[a]
            || churn[a]
            || (!arcade && (0x100..0x200).contains(&a))
            || oam_page[a / 256] * 2 > n
            || role.ends_with("scratch")
            || role.ends_with("sound")
    };
    // Bytes that held 8 frames, changed between the frame before and 3
    // after, and kept the new value 60 frames.
    let kept = |i: usize| -> Vec<Value> {
        if i < 10 || i + 64 >= n {
            return Vec::new();
        }
        (0..0x800usize)
            .filter(|&a| a != ix(yb) && !skip(a))
            .filter_map(|a| {
                let before = ram[i - 2][a];
                if (i - 10..i - 1).any(|k| ram[k][a] != before) {
                    return None;
                }
                let to = (i - 1..=i + 3).map(|k| ram[k][a]).find(|v| *v != before)?;
                (i + 4..i + 64).all(|k| ram[k][a] != before).then(|| {
                    json!({"addr": format!("${:04X}", a + usize::from(base)), "from": before, "to": to, "known_as": names.get(&(a as u16 + base))})
                })
            })
            .collect()
    };

    // Head hits: a rise that turns over by 3 px a frame at once.
    let mut hit_frames = Vec::new();
    for i in 2..n - 1 {
        if !in_level(i) || track[i].is_none() {
            continue;
        }
        let (d0, d1) = (ys[i] - ys[i - 1], ys[i + 1] - ys[i]);
        if d0 < 0 && d1 >= 0 && d1 - d0 >= 3 {
            hit_frames.push(i);
            events.push(Event {
                frame: i,
                kind: "head-hit".into(),
                what: "the player's rise stopped dead".into(),
                data: json!({}),
            });
        }
    }
    if let Some(xb) = xb {
        let xs: Vec<i32> = ram.iter().map(|r| i32::from(r[ix(xb)])).collect();
        for w in m::walls(&xs, &ys, &pads, 0) {
            let f = w["frame"].as_u64().unwrap_or(0) as usize;
            if in_level(f) {
                let air = w["in_air"] == json!(true);
                events.push(Event {
                    frame: f,
                    kind: if air { "side-hit-air" } else { "side-hit" }.into(),
                    what: format!(
                        "ran into something on the {}",
                        w["side"].as_str().unwrap_or("")
                    ),
                    data: w,
                });
            }
        }
    }
    for r in m::rests(&ys, &pads, 0) {
        let f = r["from"].as_u64().unwrap_or(0) as usize;
        if in_level(f)
            && r["up_px"].as_i64().unwrap_or(0) >= 16
            && r["to"].as_u64().unwrap_or(0) as usize - f >= 8
        {
            events.push(Event {
                frame: f,
                kind: "raised-landing".into(),
                what: format!("stood {} px above the lowest ground", r["up_px"]),
                data: r,
            });
        }
    }
    for (f, v) in m::stomps(&ys, &pads, &boxes, &track, 0) {
        if in_level(f) {
            events.push(Event {
                frame: f,
                kind: "stomp".into(),
                what: format!(
                    "bounced off something, which {}",
                    v["under"].as_str().unwrap_or("")
                ),
                data: v,
            });
        }
    }
    for (f, v) in m::hurts(&boxes, &track, 0) {
        if in_level(f) || levels.get(f.saturating_sub(1)).is_some_and(Option::is_some) {
            let k = v["kind"].as_str().unwrap_or("").replace(' ', "-");
            events.push(Event {
                frame: f,
                kind: format!("player-{k}"),
                what: format!("the player's box {}", v["kind"].as_str().unwrap_or("")),
                data: v,
            });
        }
    }
    for (f, v) in m::items(&boxes, &track, &hit_frames, 0) {
        if in_level(f) {
            let touched = v["end"] == json!("touched the player");
            events.push(Event {
                frame: f,
                kind: if touched { "item-taken" } else { "item-out" }.into(),
                what: format!(
                    "a {}x{} item came out of something hit from below and {}",
                    v["size"][0],
                    v["size"][1],
                    v["end"].as_str().unwrap_or("")
                ),
                data: v,
            });
        }
    }
    // Input ignored: 60+ frames in a level where the pad changes 3 or more
    // times and the player's box doesn't move.
    let mut i = 1;
    while i + 60 < n {
        let pb = |k: usize| track[k].map(|t| boxes[k][t]);
        if in_level(i) && pb(i).is_some() {
            let mut j = i;
            while j + 1 < n
                && in_level(j + 1)
                && pb(j + 1).is_some()
                && pb(j + 1).map(|b| (b[0], b[1])) == pb(i).map(|b| (b[0], b[1]))
            {
                j += 1;
            }
            let changes = (i + 1..=j).filter(|&k| pads[k] != pads[k - 1]).count();
            if j - i >= 60 && changes >= 3 {
                events.push(Event { frame: i, kind: "input-ignored".into(),
                    what: format!("the player didn't move for {} frames while the pad changed {changes} times", j - i), data: json!({"frames": j - i}) });
                i = j + 1;
                continue;
            }
            i = j.max(i) + 1;
        } else {
            i += 1;
        }
    }
    // Unexplained bursts: 16 or more bytes changed for good in one frame,
    // with no event within 10 frames.
    let mut explained: BTreeSet<usize> = BTreeSet::new();
    for e in &events {
        if e.kind != "new-code" && e.kind != "new-object" {
            for k in e.frame.saturating_sub(10)..e.frame + 10 {
                explained.insert(k);
            }
        }
    }
    let mut k = 12;
    while k + 64 < n {
        if in_level(k) && !explained.contains(&k) {
            let c = kept(k);
            if c.len() >= 16 {
                events.push(Event {
                    frame: k,
                    kind: "unexplained".into(),
                    what: format!(
                        "{} bytes changed for good and no detector says why",
                        c.len()
                    ),
                    data: json!({"kept": c.len(), "bytes": c.iter().map(|b| b["addr"].clone()).collect::<Vec<_>>()}),
                });
                k += 30;
                continue;
            }
        }
        k += 4;
    }
    events.sort_by_key(|e| e.frame);
    let detect_secs = t0.elapsed().as_secs_f64() - replay_secs;

    // Per level: counts by kind, and one example clip per kind.
    let lv_of = |f: usize| {
        levels[f]
            .clone()
            .or_else(|| levels[..f].iter().rev().find_map(Clone::clone))
            .unwrap_or_else(|| "before-play".into())
    };
    let mut per_level: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    let mut examples: BTreeMap<(String, String), usize> = BTreeMap::new();
    let mut levels_of_kind: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut count_of_kind: BTreeMap<String, usize> = BTreeMap::new();
    for (ix, e) in events.iter().enumerate() {
        let l = lv_of(e.frame);
        *per_level
            .entry(l.clone())
            .or_default()
            .entry(e.kind.clone())
            .or_default() += 1;
        examples.entry((l.clone(), e.kind.clone())).or_insert(ix);
        levels_of_kind.entry(e.kind.clone()).or_default().insert(l);
        *count_of_kind.entry(e.kind.clone()).or_default() += 1;
    }
    let root = session::game_dir(paths, game)?;
    let mut example_clip: BTreeMap<String, String> = BTreeMap::new();
    let mut clip_n: BTreeMap<String, u64> = BTreeMap::new();
    let mut written_bytes = 0usize;
    // The recording's inputs as one stored path, for clips that start from
    // a snapshot along it.
    let mut unhashed: Vec<(PathBuf, usize)> = Vec::new();
    let rec_path = if manifest.power_on == gp_evidence::manifest::PowerOn::default() {
        Some(crate::mechanic::paths::put_path(&paths.home, &log.to_text())?.0)
    } else {
        None
    };
    for ((l, kind), ix) in &examples {
        let e = &events[*ix];
        let sess = format!("harvest-{}", l.to_lowercase().replace('_', "-"));
        if !session::name_ok(&sess) {
            continue;
        }
        let d = root.join(&sess);
        fs::create_dir_all(d.join("clips"))?;
        if !d.join("session.toml").is_file() {
            let mut t = toml::Table::new();
            t.insert("game".into(), game.into());
            t.insert("mechanic".into(), sess.clone().into());
            t.insert(
                "prompt".into(),
                format!("what recording {} shows in {l}", &entry.id.to_string()[..8]).into(),
            );
            t.insert("rom".into(), manifest_rom(&rom).into());
            fs::write(d.join("session.toml"), toml::to_string(&t)?)?;
        }
        let num = clip_n.entry(sess.clone()).or_insert_with(|| {
            fs::read_dir(d.join("clips"))
                .map(|r| {
                    r.filter(|e| {
                        e.as_ref()
                            .is_ok_and(|e| e.path().extension().is_some_and(|x| x == "log"))
                    })
                    .count() as u64
                })
                .unwrap_or(0)
        });
        *num += 1;
        let start = e.frame.saturating_sub(LEAD_IN);
        let end = (start + CLIP).min(n);
        let base = d.join(format!("clips/{:03}", *num));
        // A clip on the recording's path (stored once), starting from the
        // shared snapshot cache, when the recording powers on the default
        // way; else its full log from power-on, as before.
        let mut t = if let Some(sha) = &rec_path {
            // Its start state's hash is added below, all clips' in one replay.
            let mut part = gp_evidence::input_log::InputLog::new(log.controllers);
            part.frames = log.frames[start..end].to_vec();
            let (t, text) = crate::mechanic::paths::clip_fields(sha, start, &part);
            written_bytes += text.len();
            fs::write(base.with_extension("log"), text)?;
            unhashed.push((base.with_extension("toml"), start));
            t
        } else {
            let mut cl = gp_evidence::input_log::InputLog::new(log.controllers);
            cl.frames = log.frames[..end].to_vec();
            let text = cl.to_text();
            written_bytes += text.len();
            fs::write(base.with_extension("log"), text)?;
            let mut t = toml::Table::new();
            t.insert("frames".into(), ((end - start) as i64).into());
            t.insert("start".into(), (start as i64).into());
            t
        };
        t.insert("by".into(), "harvest".into());
        t.insert("note".into(), format!("{kind}: {}", e.what).into());
        fs::write(base.with_extension("toml"), toml::to_string(&t)?)?;
        // The overlay, from this run's data: frames and the events in the clip.
        let frames: Vec<Value> = (start..end)
            .map(|k| json!({"boxes": boxes[k], "player": track[k]}))
            .collect();
        let rel = |f: usize| f - start;
        let mut hits = Vec::new();
        let mut walls = Vec::new();
        let mut stomps = Vec::new();
        let mut hurts = Vec::new();
        let mut items = Vec::new();
        for ev in events
            .iter()
            .filter(|ev| ev.frame >= start && ev.frame < end)
        {
            let mut v = ev.data.clone();
            v["frame"] = json!(rel(ev.frame));
            match ev.kind.as_str() {
                "head-hit" => {
                    hits.push(json!({"frame": rel(ev.frame), "rose_px": null, "at_hit": kept(ev.frame).into_iter().map(|mut c| { c["lasting"] = json!(true); c }).collect::<Vec<_>>(), "after": [], "sprites_added": 0}));
                }
                "side-hit" | "side-hit-air" => walls.push(v),
                "stomp" => stomps.push(v),
                k if k.starts_with("player-") => hurts.push(v),
                "item-taken" | "item-out" => {
                    if let Some(p) = v["path"].as_array() {
                        let shifted: Vec<Value> = p
                            .iter()
                            .filter_map(|q| {
                                let f = q[0].as_u64()? as usize;
                                (f >= start && f < end).then(|| json!([f - start, q[1], q[2]]))
                            })
                            .collect();
                        v["path"] = json!(shifted);
                    }
                    if let Some(tf) = v["touch_frame"].as_u64() {
                        v["touch_frame"] = json!((tf as usize).saturating_sub(start));
                    }
                    for l in v["legs"].as_array_mut().into_iter().flatten() {
                        if let Some(f) = l["from"].as_u64() {
                            l["from"] = json!((f as usize).saturating_sub(start));
                        }
                    }
                    v["frame"] =
                        json!(rel(ev.frame).min(v["frame"].as_u64().unwrap_or(0) as usize));
                    items.push(v);
                }
                _ => {}
            }
        }
        let overlay = json!({"clip": *num, "frames": frames, "hits": hits, "rests": [], "walls": walls,
                             "stomps": stomps, "hurts": hurts, "items": items});
        fs::write(base.with_extension("measure.json"), overlay.to_string())?;
        example_clip
            .entry(kind.clone())
            .or_insert_with(|| format!("{sess} clip {}", *num));
    }

    // Each path clip's start state, hashed in one replay along the run.
    if !unhashed.is_empty() {
        let at: BTreeSet<usize> = unhashed.iter().map(|u| u.1).collect();
        let h = hashes_at(&rom, manifest.power_on, &log.frames, &at)?;
        for (f, start) in &unhashed {
            let mut t: toml::Table = toml::from_str(&fs::read_to_string(f)?)?;
            t.insert("state".into(), h[start].clone().into());
            fs::write(f, toml::to_string(&t)?)?;
        }
    }

    // Candidate mechanics: one per kind, matched to queue rows by words.
    let qf = root.join("queue.json");
    let mut q = session::read_queue(&qf);
    let matches: &[(&str, &[&str])] = &[
        ("head-hit", &["hit-block-from-below", "multi-coin-block"]),
        ("side-hit", &["bump-block-side"]),
        ("side-hit-air", &["bump-block-side"]),
        ("raised-landing", &["land-on-block"]),
        ("stomp", &["stomp-enemy"]),
        ("item-out", &["get-item", "fire-flower", "star"]),
        ("item-taken", &["get-item", "fire-flower", "star"]),
        (
            "player-fell-off-the-screen",
            &["touch-enemy", "fall-in-pit"],
        ),
        ("player-shrank", &["touch-enemy"]),
        ("player-grew", &["get-item"]),
        ("player-blinked", &["touch-enemy", "star"]),
        ("input-ignored", &["flagpole", "pipe-down", "touch-enemy"]),
    ];
    let mut leads: Vec<Value> = Vec::new();
    for (kind, c) in &count_of_kind {
        let lv: Vec<String> = levels_of_kind[kind].iter().cloned().collect();
        let rows: Vec<String> = matches
            .iter()
            .filter(|(k, _)| k == kind)
            .flat_map(|(_, r)| r.iter())
            .filter(|r| q.iter().any(|row| row["mechanic"] == json!(r)))
            .map(|r| (*r).to_owned())
            .collect();
        leads.push(
            json!({"kind": kind, "count": c, "levels": lv.len(), "level_names": lv,
            "example": example_clip.get(kind), "queue": rows}),
        );
    }
    leads.sort_by(|a, b| {
        b["levels"]
            .as_u64()
            .cmp(&a["levels"].as_u64())
            .then(b["count"].as_u64().cmp(&a["count"].as_u64()))
    });
    if add {
        for l in &leads {
            let kind = l["kind"].as_str().unwrap_or("");
            let note = format!(
                "the run shows this in {} event(s) over {} level(s)",
                l["count"], l["levels"]
            );
            let rows: Vec<String> = l["queue"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect();
            if rows.is_empty() {
                let name = format!("harvest-{kind}");
                if !q.iter().any(|r| r["mechanic"] == json!(name)) {
                    q.push(json!({"mechanic": name, "status": "next", "note": note}));
                }
            } else {
                for r in q
                    .iter_mut()
                    .filter(|r| rows.iter().any(|x| r["mechanic"] == json!(x)))
                {
                    let old = r["note"].as_str().unwrap_or("").to_owned();
                    if !old.contains("the run shows this") {
                        r["note"] = json!(format!("{old}; {note}"));
                    }
                }
            }
        }
        fs::write(&qf, serde_json::to_string_pretty(&q)?)?;
    }
    // Object types: first level and frame, frames active, events while
    // active, and the routines that run (almost) only while it is active,
    // with their branches taken both ways, one way, or never reached.
    let mut types: Vec<Value> = Vec::new();
    if let Some((count, active, id)) = slots {
        let mut routine_off: BTreeMap<String, u32> = BTreeMap::new();
        if !arcade {
            let (prg, mapper) = gp_nes::harvest::ines_prg(&rom).context("not an iNES ROM")?;
            let board = gp_nes::codemap::Board::of(prg, mapper);
            for r in data.iter().flat_map(|d| d.rows("routines")) {
                if let Some(crate::rom_dataset::Cell::S(Some(name))) = r.get("routine")
                    && let Some(o) =
                        gp_nes::codemap::va_parse(name).and_then(|v| board.prg_offset(v))
                {
                    routine_off.insert(name.clone(), o as u32);
                }
            }
        }
        let active_types = |i: usize| -> BTreeSet<u8> {
            (0..count)
                .filter(|k| ram[i][ix(active) + k] != 0)
                .map(|k| ram[i][ix(id) + k])
                .collect()
        };
        let mut frames_of: BTreeMap<u8, Vec<usize>> = BTreeMap::new();
        let mut entered_with: BTreeMap<u32, (usize, BTreeMap<u8, usize>)> = BTreeMap::new();
        for (i, ent) in entered.iter().enumerate().take(n) {
            if !in_level(i) {
                continue;
            }
            let ts = active_types(i);
            for t in &ts {
                frames_of.entry(*t).or_default().push(i);
            }
            for o in ent {
                let e = entered_with.entry(*o).or_default();
                e.0 += 1;
                for t in &ts {
                    *e.1.entry(*t).or_default() += 1;
                }
            }
        }
        // A branch is a type's when 95% of the frames it ran on had that
        // type out, the rarest such type taking it.
        let mut branch_with: BTreeMap<u16, (usize, BTreeMap<u8, usize>)> = BTreeMap::new();
        for (i, ran) in ran_branches.iter().enumerate().take(n) {
            if !in_level(i) {
                continue;
            }
            let ts = active_types(i);
            for pc in ran {
                let e = branch_with.entry(*pc).or_default();
                e.0 += 1;
                for t in &ts {
                    *e.1.entry(*t).or_default() += 1;
                }
            }
        }
        let mut owner_b: BTreeMap<u8, BTreeSet<u16>> = BTreeMap::new();
        for (pc, (all, by)) in &branch_with {
            if *all < 3 {
                continue;
            }
            if let Some((t, _)) = by
                .iter()
                .filter(|(_, c)| **c * 100 >= *all * 95)
                .min_by_key(|(t, _)| frames_of.get(t).map_or(usize::MAX, Vec::len))
            {
                owner_b.entry(*t).or_default().insert(*pc);
            }
        }
        // A branch never reached belongs to the type owning other branches
        // of its routine, when only one type does.
        let mut routine_types: BTreeMap<String, BTreeSet<u8>> = BTreeMap::new();
        for (t, pcs) in &owner_b {
            for pc in pcs {
                if let Some((_, r)) = branch_at.get(pc) {
                    routine_types.entry(r.clone()).or_default().insert(*t);
                }
            }
        }
        let owner: BTreeMap<u8, Vec<u32>> = BTreeMap::new();
        let _ = &entered_with;
        let off_name: BTreeMap<u32, String> =
            routine_off.iter().map(|(k, v)| (*v, k.clone())).collect();
        for (t, fs_) in &frames_of {
            let first = fs_[0];
            let mut near_events: BTreeMap<String, usize> = BTreeMap::new();
            let set: BTreeSet<usize> = fs_.iter().copied().collect();
            for e in &events {
                if set.contains(&e.frame)
                    && matches!(
                        e.kind.as_str(),
                        "stomp" | "side-hit" | "side-hit-air" | "item-taken"
                    )
                    || (set.contains(&e.frame) && e.kind.starts_with("player-"))
                {
                    *near_events.entry(e.kind.clone()).or_default() += 1;
                }
            }
            let mine: Vec<String> = routine_types
                .iter()
                .filter(|(_, ts)| ts.len() == 1 && ts.contains(t))
                .map(|(r, _)| r.clone())
                .collect();
            let _ = (&owner, &off_name);
            let (mut both, mut one, mut never) = (0, 0, 0);
            let mut untaken: Vec<String> = Vec::new();
            let mine_b = owner_b.get(t).cloned().unwrap_or_default();
            for (pc, (op, routine)) in &branch_at {
                let only_mine = routine_types
                    .get(routine)
                    .is_some_and(|ts| ts.len() == 1 && ts.contains(t));
                if !(mine_b.contains(pc) || (only_mine && !branch_n.contains_key(pc))) {
                    continue;
                }
                match branch_n.get(pc) {
                    Some((a, b)) if *a > 0 && *b > 0 => both += 1,
                    Some((a, _)) => {
                        one += 1;
                        untaken.push(format!(
                            "{pc:04X} {op} never {}",
                            if *a > 0 { "fell through" } else { "taken" }
                        ));
                    }
                    None => {
                        never += 1;
                        untaken.push(format!("{pc:04X} {op} never reached"));
                    }
                }
            }
            untaken.sort();
            types.push(json!({"type": format!("{t:#04x}"), "first_level": lv_of(first), "first_frame": first,
                "frames_active": fs_.len(), "levels": fs_.iter().filter_map(|&i| levels[i].clone()).collect::<BTreeSet<_>>().len(),
                "interactions": near_events, "routines": mine, "branches_both": both, "branches_one_way": one,
                "branches_never": never, "untaken": untaken}));
        }
        types.sort_by_key(|v| {
            std::cmp::Reverse(
                v["branches_one_way"].as_u64().unwrap_or(0) * 2
                    + v["branches_never"].as_u64().unwrap_or(0),
            )
        });
    }
    let unexplained: Vec<&Event> = events.iter().filter(|e| e.kind == "unexplained").collect();
    let mut un_bytes: BTreeMap<String, usize> = BTreeMap::new();
    for e in &unexplained {
        for b in e.data["bytes"].as_array().into_iter().flatten() {
            *un_bytes
                .entry(b.as_str().unwrap_or("").to_owned())
                .or_default() += 1;
        }
    }
    let mut un_top: Vec<(String, usize)> = un_bytes.into_iter().collect();
    un_top.sort_by_key(|(_, c)| std::cmp::Reverse(*c));
    let mut text = format!(
        "{} frames replayed in {:.0} s, detectors {:.0} s, {} events over {} levels, {} example clips ({:.0} MB of logs)\n",
        n,
        replay_secs,
        detect_secs,
        events.len(),
        per_level.len(),
        examples.len(),
        written_bytes as f64 / 1e6
    );
    for (i, l) in leads.iter().take(top).enumerate() {
        let qs: Vec<&str> = l["queue"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        text.push_str(&format!(
            "{}. {}: {} events in {} levels; e.g. {}; {}\n",
            i + 1,
            l["kind"].as_str().unwrap_or(""),
            l["count"],
            l["levels"],
            l["example"].as_str().unwrap_or("-"),
            if qs.is_empty() {
                "new".to_owned()
            } else {
                format!("matches {}", qs.join(", "))
            }
        ));
    }
    text.push_str(&format!(
        "left out of \"unexplained\": {scroll_bytes} bytes that change with the scroll's column, {churn_bytes} running bytes (a new value more than once in 300 frames); {} unexplained moments remain\n",
        unexplained.len()
    ));
    text.push_str(&format!(
        "bytes most often in unexplained moments: {}\n",
        un_top
            .iter()
            .take(12)
            .map(|(b, c)| format!(
                "{b} {c}{}",
                names
                    .get(&u16::from_str_radix(b.trim_start_matches('$'), 16).unwrap_or(0))
                    .map(|n| format!(" ({n})"))
                    .unwrap_or_default()
            ))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    text.push_str(
        "object types (by untaken branches of the routines that run only while the type is out):\n",
    );
    for t in &types {
        let inter: Vec<String> = t["interactions"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(k, v)| format!("{k} {v}"))
            .collect();
        text.push_str(&format!(
            "  type {}: first in {} at frame {}, {} frames out in {} levels; {} routine(s), branches both ways {}, one way {}, never reached {}; with Mario: {}\n",
            t["type"].as_str().unwrap_or(""), t["first_level"].as_str().unwrap_or(""), t["first_frame"], t["frames_active"], t["levels"],
            t["routines"].as_array().map_or(0, Vec::len), t["branches_both"], t["branches_one_way"], t["branches_never"],
            if inter.is_empty() { "nothing seen".to_owned() } else { inter.join(", ") }
        ));
    }
    text.push_str("per level: events | head hits | side hits | landings | stomps | items | hurts | unexplained | new types\n");
    for (l, k) in &per_level {
        let total: usize = k.values().sum();
        let g = |ks: &[&str]| -> usize {
            k.iter()
                .filter(|(n, _)| ks.iter().any(|x| n.starts_with(x)))
                .map(|(_, c)| *c)
                .sum()
        };
        text.push_str(&format!(
            "  {l}: {total} | {} | {} | {} | {} | {} | {} | {} | {}\n",
            g(&["head-hit"]),
            g(&["side-hit"]),
            g(&["raised-landing"]),
            g(&["stomp"]),
            g(&["item-"]),
            g(&["player-"]),
            g(&["unexplained"]),
            g(&["new-object"])
        ));
    }
    Ok((
        json!({"events": events.len(), "per_level": per_level, "leads": leads, "types": types, "scroll_bytes": scroll_bytes, "seconds": t0.elapsed().as_secs_f64()}),
        text,
    ))
}

/// The canonical state's hash at each of `at` (frames into `frames`), from
/// one plain replay from power-on (game-printer's clip cache did this
/// through its checkpoints).
fn hashes_at(
    rom: &[u8],
    power_on: gp_evidence::manifest::PowerOn,
    frames: &[gp_evidence::input_log::InputFrame],
    at: &BTreeSet<usize>,
) -> Result<BTreeMap<usize, String>> {
    let mut out = BTreeMap::new();
    let Some(&last) = at.last() else {
        return Ok(out);
    };
    if crate::original::Platform::of_file(rom) == crate::original::Platform::Arcade {
        let mut o = crate::original::OriginalRuntime::of_rom(rom)?;
        for i in 0..=last.min(frames.len()) {
            if at.contains(&i) {
                out.insert(i, o.state_hex()?);
            }
            if i < frames.len() {
                let f = gp_runtime::InputFrame::pads(frames[i].pads);
                gp_runtime::Runtime::step(&mut o, &[f]).map_err(anyhow::Error::msg)?;
            }
        }
        return Ok(out);
    }
    let mut nes = gp_nes::Nes::power_on(rom, power_on)?;
    for i in 0..=last.min(frames.len()) {
        if at.contains(&i) {
            out.insert(i, nes.state_hash()?.to_hex());
        }
        if i < frames.len() {
            nes.step(frames[i])?;
        }
    }
    Ok(out)
}

fn manifest_rom(rom: &[u8]) -> String {
    if crate::original::Platform::of_file(rom) == crate::original::Platform::Arcade {
        return gp_arcade::Romset::from_zip(rom)
            .map(|s| s.identity)
            .unwrap_or_default();
    }
    gp_evidence::source::identify_nes(rom)
        .map(|i| i.sha256)
        .unwrap_or_default()
}

/// What one replay of the recording gives, frame by frame: the RAM the
/// detectors read (the NES's 2 KB, or the arcade board's work RAM at
/// E000-E7FF), the sprite boxes, the scroll, the routines entered and the
/// dataset's branches run, with each branch's ways over the whole run.
struct Replay {
    ram: Vec<Vec<u8>>,
    boxes: Vec<Vec<[i32; 4]>>,
    scroll: Vec<i32>,
    entered: Vec<Vec<u32>>,
    ran_branches: Vec<Vec<u16>>,
    branch_n: std::collections::HashMap<u16, (u64, u64)>,
    /// Per 256-byte page of the RAM: the frames it held the sprite table.
    oam_page: [usize; 8],
}

/// The replay on gp-nes, with gp-events' collector: routines entered by
/// PRG offset, branches by the 6502's flags.
fn replay_nes(
    rom: &[u8],
    manifest: &gp_evidence::manifest::RecordingManifest,
    log: &gp_evidence::InputLog,
    branch_at: &std::collections::HashMap<u16, (String, String)>,
) -> Result<Replay> {
    let mut nes = gp_nes::Nes::power_on(rom, manifest.power_on)?;
    let layout = gp_events::Layout::of(nes.bus())?;
    let act =
        gp_events::activity::Activity::watching(layout, &gp_events::Bits::new(0), BTreeSet::new());
    let mut col = gp_events::Collector::with_activity(layout, false, act);
    col.start(nes.bus_mut_for_observers());
    let n = log.frames.len();
    let mut ram: Vec<Vec<u8>> = Vec::with_capacity(n);
    let mut boxes: Vec<Vec<[i32; 4]>> = Vec::with_capacity(n);
    let mut oam_page = [0usize; 8];
    // Per branch: (taken, not taken) over the run.
    let mut branch_n: std::collections::HashMap<u16, (u64, u64)> = std::collections::HashMap::new();
    let mut scroll: Vec<i32> = Vec::with_capacity(n);
    let mut entered: Vec<Vec<u32>> = Vec::with_capacity(n);
    // The branches that ran on each frame.
    let mut ran_branches: Vec<Vec<u16>> = Vec::with_capacity(n);
    let mut this_frame: BTreeSet<u16> = BTreeSet::new();
    for f in &log.frames {
        if f.reset {
            col.will_reset();
        }
        nes.step_with(*f, |bus| {
            col.before(bus);
            let pc = bus.cpu.pc;
            if let Some((op, _)) = branch_at.get(&pc) {
                let p = bus.cpu.status.bits();
                let flag = |b: u8| p & b != 0;
                let taken = match op.as_str() {
                    "beq" => flag(0x02),
                    "bne" => !flag(0x02),
                    "bcs" => flag(0x01),
                    "bcc" => !flag(0x01),
                    "bmi" => flag(0x80),
                    "bpl" => !flag(0x80),
                    "bvs" => flag(0x40),
                    "bvc" => !flag(0x40),
                    _ => return,
                };
                this_frame.insert(pc);
                let e = branch_n.entry(pc).or_default();
                if taken {
                    e.0 += 1;
                } else {
                    e.1 += 1;
                }
            }
        })?;
        let fa = col.end_frame_activity(nes.bus()).context("watching")?;
        let sc = &nes.bus().ppu.scroll;
        scroll.push(i32::from(
            (sc.t & 31) * 8 + u16::from(sc.fine_x as u8 & 7) + ((sc.t >> 10) & 1) * 256,
        ));
        entered.push(fa.entered.iter().map(|(o, _)| *o).collect());
        ran_branches.push(std::mem::take(&mut this_frame).into_iter().collect());
        let w = nes.wram()[..0x800].to_vec();
        let oam = nes.oam();
        // The sprite buffer: a page equal to OAM this frame, or to the next
        // frame's OAM (a game that copies at the next interrupt).
        for (p, c) in oam_page.iter_mut().enumerate() {
            let last = ram.last().map(|r: &Vec<u8>| &r[p * 256..p * 256 + 256]);
            if w[p * 256..p * 256 + 256] == oam[..] || last == Some(&oam[..]) {
                *c += 1;
            }
        }
        boxes.push(m::sprite_boxes(
            &oam,
            if nes.tall_sprites() { 16 } else { 8 },
        ));
        ram.push(w);
    }
    Ok(Replay {
        ram,
        boxes,
        scroll,
        entered,
        ran_branches,
        branch_n,
        oam_page,
    })
}

/// The replay on gp-arcade, with the main Z80's instruction trace: the
/// work RAM at E000-E7FF; the sprite boxes from the sprite RAM through
/// gp-arcade's sprite list (each sprite 16x16 in the game's half-pixel
/// units, as `gp track` reads them); no scroll (the board has none); the
/// routines entered (calls', restarts' and interrupts' targets and the
/// dataset's routines, by address, [`crate::arcade_stage0::entries`]); and a dataset branch's way by where the next instruction
/// starts. The sprite RAM is outside the window, so no page of it is the
/// sprite table.
fn replay_arcade(
    rom: &[u8],
    log: &gp_evidence::InputLog,
    branch_at: &std::collections::HashMap<u16, (String, String)>,
    known: &BTreeSet<u16>,
) -> Result<Replay> {
    let set = gp_arcade::Romset::from_zip(rom)?;
    let mut t = gp_arcade::Tapper::power_on(&set);
    let n = log.frames.len();
    let mut out = Replay {
        ram: Vec::with_capacity(n),
        boxes: Vec::with_capacity(n),
        scroll: vec![0; n],
        entered: Vec::with_capacity(n),
        ran_branches: Vec::with_capacity(n),
        branch_n: std::collections::HashMap::new(),
        oam_page: [0; 8],
    };
    let base = crate::arcade_stage0::RAM_BASE;
    for (i, f) in log.frames.iter().enumerate() {
        let bits = f.buttons_word();
        if bits > 0xff || f.reset {
            bail!(
                "frame {}: an arcade input holds bits 0 to 7 and no reset",
                i + 1
            );
        }
        let trace = crate::covered::frame_trace(&mut t, gp_arcade::Controls::from_bits(bits));
        let entered: Vec<u32> = crate::arcade_stage0::entries(&trace, known)
            .into_iter()
            .map(u32::from)
            .collect();
        let mut this_frame: BTreeSet<u16> = BTreeSet::new();
        let main: Vec<&gp_arcade::mcr::Trace> = trace
            .iter()
            .filter(|e| e.cpu == 0 && e.interrupt.is_none())
            .collect();
        for (k, e) in main.iter().enumerate() {
            if branch_at.contains_key(&e.pc)
                && let Some(next) = main.get(k + 1)
            {
                let len = gp_arcade::dasm::decode(&e.bytes, e.pc).len;
                let taken = next.pc != e.pc.wrapping_add(u16::from(len));
                this_frame.insert(e.pc);
                let c = out.branch_n.entry(e.pc).or_default();
                if taken {
                    c.0 += 1;
                } else {
                    c.1 += 1;
                }
            }
        }
        out.entered.push(entered);
        out.ran_branches.push(this_frame.into_iter().collect());
        out.ram.push(
            (0..crate::arcade_stage0::RAM_END - base)
                .map(|k| t.peek(base + k))
                .collect(),
        );
        let sprites: Vec<u8> = (0xe800..0xea00u16).map(|a| t.peek(a)).collect();
        out.boxes.push(m::arcade_boxes(&sprites));
    }
    Ok(out)
}

/// An arcade frame's controls (coin, start, right, left, down, up, tap
/// from bit 0) as the NES pad's bits the detectors read.
pub(crate) fn nes_pad(c: u8) -> u8 {
    use gp_evidence::input_log::button as b;
    use gp_explore::arcade as a;
    [
        (a::COIN, b::SELECT),
        (a::START, b::START),
        (a::RIGHT, b::RIGHT),
        (a::LEFT, b::LEFT),
        (a::DOWN, b::DOWN),
        (a::UP, b::UP),
        (a::TAP, b::A),
    ]
    .iter()
    .filter(|(x, _)| c & x != 0)
    .fold(0, |p, (_, y)| p | y)
}
