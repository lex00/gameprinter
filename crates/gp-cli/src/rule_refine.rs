//! `gp rule`: the rule behind a mechanic, found from the clips and
//! checked frame by frame against the original (ported from game-printer's
//! rule_refine.rs).
//!
//! `gp rule jump GAME` finds the rule for the player's vertical motion in
//! the air. The game's state mapping names the player's y and vertical
//! speed bytes (`entities.player[0].y` and `.vy`, their first `ram`
//! address, as the old tool read a mapping). The rest is read from the
//! clips: the bytes that carry the fractions of y and of the speed (found by which bytes make the
//! position's change add up), then the gravity added each frame, with A
//! held and not, for each takeoff speed. The rule is then run from each
//! jump's real takeoff state with the clip's buttons, and its y is compared
//! with the original's on every frame.
//!
//! The clips are replayed on the original through `gp_runtime::Runtime`
//! ([`OriginalRuntime`]), reading its RAM with `query("ram[..]")`: a window
//! of 2 KB from the 2 KB page that holds the y byte (all of the NES's RAM).
//! The rule is fitted on one set of clips and tested on the other: the
//! game's corpus (`--fit-on corpus`, the default), the person's mechanic
//! session clips (`yours`) or the sessions named harvest-* (`harvest`).
//!
//! The result goes to $GP_HOME/mechanics/GAME/jump/refine.json, for the
//! mechanic page's refinement view (/refine); refine.status.json says what
//! it is doing. The old tool also read the routines that write y from a ROM
//! dataset; gameprinter has none yet, and the result says so.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use clap::Subcommand;
use gp_evidence::Paths;
use gp_runtime::Runtime as _;
use serde_json::{Value, json};

use crate::Outcome;
use crate::mechanic::http::{Request, Response};
use crate::original::OriginalRuntime;

#[path = "rule_lane.rs"]
pub(crate) mod lane;

#[derive(Subcommand, Debug)]
pub(crate) enum RuleCmd {
    /// The rule for the player's motion in the air during a jump: the bytes
    /// that carry its fractions and the gravity each frame, read from the
    /// clips, and how many frames of each jump the rule reproduces.
    Jump {
        /// The game's package folder (games/donkey-kong).
        game: PathBuf,
        /// The mechanic sessions whose clips to use (default: every one).
        #[arg(long = "session")]
        sessions: Vec<String>,
        /// What the rule is fitted on: the game's corpus ("corpus", the
        /// default), your mechanic session clips ("yours") or the clips
        /// harvested from a recorded run ("harvest"). The others are the
        /// held-out test.
        #[arg(long, default_value = "corpus", value_parser = ["corpus", "yours", "harvest"])]
        fit_on: String,
        /// An input log from power-on to test on as well, whole; held out,
        /// never fitted on.
        #[arg(long = "test-log")]
        test_logs: Vec<PathBuf>,
        /// The ROM; by default the one in $GP_ROM_DIR the package names.
        #[arg(long)]
        rom: Option<PathBuf>,
        /// Where to write the result (default the jump session's
        /// refine.json, which the page shows).
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// The rule for a player who moves between lanes (Tapper's bartender:
    /// the bars): each lane's row, the frames a move to another lane takes
    /// and keeps the stick busy, where the player lands on the new lane,
    /// the walk along a lane, and the step back after a send. Fitted from
    /// the clips and checked against the original frame by frame.
    Lane(lane::LaneArgs),
}

pub(crate) fn run(cmd: &RuleCmd, paths: &Paths) -> Result<Outcome> {
    match cmd {
        RuleCmd::Jump {
            game,
            sessions,
            fit_on,
            test_logs,
            rom,
            out,
        } => jump(
            paths,
            game,
            sessions,
            fit_on,
            test_logs,
            rom.as_deref(),
            out.as_deref(),
        ),
        RuleCmd::Lane(a) => lane::run(a, paths),
    }
}

/// The RAM a trace keeps each frame: 2 KB from `base`.
const WINDOW: usize = 0x800;

/// One clip replayed: the RAM after each frame and the buttons held.
struct Trace {
    session: String,
    clip: u64,
    start: usize,
    ram: Vec<Vec<u8>>,
    pads: Vec<u8>,
    /// Each frame's sprite boxes ([x, y, w, h] on the screen).
    boxes: Vec<Vec<[i32; 4]>>,
}

/// One jump in a trace: from the frame it left the ground to the frame it
/// stood again (or the clip ended, or something stopped it).
#[derive(Clone)]
struct Jump {
    trace: usize,
    from: usize,
    to: usize,
}

/// Writes what refinement is doing now, for the page's status line.
fn status(dir: &Path, text: &str) {
    let _ = fs::create_dir_all(dir);
    let _ = fs::write(
        dir.join("refine.status.json"),
        json!({"text": text}).to_string(),
    );
}

fn signed(b: u8) -> i32 {
    i32::from(b as i8)
}

fn jump(
    paths: &Paths,
    game_dir: &Path,
    only: &[String],
    fit_on: &str,
    test_logs: &[PathBuf],
    rom: Option<&Path>,
    out_file: Option<&Path>,
) -> Result<Outcome> {
    let game_dir = fs::canonicalize(game_dir).with_context(|| game_dir.display().to_string())?;
    let game = game_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_owned();
    let game = game.as_str();
    let home = paths.home.join("mechanics").join(game);
    let out_dir = home.join("jump");
    status(&out_dir, "reading the state mapping for the player's bytes");
    let bytes = crate::clip_score::rom_of(paths, &game_dir, rom)?;
    let (yb, vyb) = player_bytes(&game_dir)?;
    let vyb = vyb.context(
        "the state mapping reads no entities.player[0].vy from RAM; the rule needs the speed's byte",
    )?;
    let base = usize::from(yb) & !(WINDOW - 1);
    if usize::from(vyb) < base || usize::from(vyb) >= base + WINDOW {
        bail!("the player's y (${yb:04X}) and speed (${vyb:04X}) aren't in the same 2 KB page");
    }
    let (yb, vyb) = (usize::from(yb) - base, usize::from(vyb) - base);
    let addr = |b: usize| base + b;
    let code = code_side();

    // The clips the rule is fitted on; the rest are held out and only
    // tested.
    let harvested = |s: &str| s.starts_with("harvest-");
    let fitted_here = |s: &str| match fit_on {
        "harvest" => harvested(s),
        "yours" => !harvested(s) && (only.is_empty() || only.iter().any(|o| o == s)),
        _ => false,
    };
    status(&out_dir, "replaying the clips");
    let mut traces = load_traces(&bytes, base, &home, &|s| fitted_here(s))?;
    if fit_on == "corpus" {
        traces.extend(corpus_traces(&bytes, base, &game_dir)?);
    }
    if traces.is_empty() {
        bail!("no clips to fit on ({fit_on})");
    }
    let jumps = find_jumps(&traces, yb);
    if jumps.is_empty() {
        bail!("no jumps in the clips");
    }

    status(
        &out_dir,
        "finding the bytes that carry y's and speed's fractions",
    );
    let frac = find_fractions(&traces, &jumps, yb, vyb);

    status(&out_dir, "fitting gravity with A held and released");
    let rule = fit_rule(&traces, &jumps, yb, vyb, &frac);

    status(&out_dir, "running the rule against every jump");
    let (clips, matched, total, _) = simulate_all(&traces, &jumps, yb, vyb, &frac, &rule);

    status(
        &out_dir,
        "testing the rule on the harvested run, which it wasn't fitted on",
    );
    let mut test = load_traces(&bytes, base, &home, &|s| {
        !fitted_here(s) && (only.is_empty() || only.iter().any(|o| o == s) || harvested(s))
    })?;
    if fit_on != "corpus" {
        test.extend(corpus_traces(&bytes, base, &game_dir)?);
    }
    for log in test_logs {
        test.push(whole_log(&bytes, base, log)?);
    }
    let test_jumps = find_jumps(&test, yb);
    let (test_clips, test_matched, test_total, test_missing) =
        simulate_all(&test, &test_jumps, yb, vyb, &frac, &rule);
    let uncovered: usize = test_missing.values().sum();
    let mut constants = Vec::new();
    for c in &code {
        constants.push(c.clone());
    }
    let fitted_from: Vec<String> = traces
        .iter()
        .map(|t| format!("{} clip {}", t.session, t.clip))
        .collect();
    if let Some((pf, vf)) = frac.pos.zip(frac.speed) {
        let (pf, vf) = (addr(pf), addr(vf));
        constants.push(json!({"name": "y fraction byte", "value": format!("${pf:04X}"), "unit": "1/256 px", "source": "fitted", "from": "the byte that makes y's change add up to the speed", "clips": fitted_from.len()}));
        constants.push(json!({"name": "speed fraction byte", "value": format!("${vf:04X}"), "unit": "1/256 px a frame", "source": "fitted", "from": "the byte that makes the speed's change add up to the gravity", "clips": fitted_from.len()}));
    }
    for (k, g) in &rule.gravity {
        constants.push(json!({"name": format!("gravity, {k}"), "value": g, "unit": "1/256 px a frame, each frame", "source": "fitted", "from": "the change in speed each frame in the air", "clips": fitted_from.len()}));
    }
    constants.push(json!({"name": "fastest fall", "value": rule.cap_above, "unit": "1/256 px a frame", "source": "fitted", "from": "a falling speed that would reach this drops back instead (the threshold that best splits the speeds that dropped from those kept)", "clips": fitted_from.len()}));
    constants.push(json!({"name": "speed after the fastest fall", "value": rule.cap_to, "unit": "1/256 px a frame", "source": "fitted", "from": "the speed a fall drops back to", "clips": fitted_from.len()}));
    let _ = rule.max_fall;
    for (k, v) in &rule.takeoff {
        constants.push(json!({"name": format!("takeoff speed, {k}"), "value": v, "unit": "1/256 px a frame", "source": "fitted", "from": "the speed on the first frame in the air", "clips": fitted_from.len()}));
    }

    let result = json!({
        "mechanic": "jump",
        "game": game,
        "bytes": {"y": format!("${:04X}", addr(yb)), "speed_y": format!("${:04X}", addr(vyb))},
        "rule": constants,
        "order": rule.order,
        "matched": matched,
        "total": total,
        "clips": clips,
        "held_out": {"from": held_from(fit_on), "clips": test.len(), "jumps": test_jumps.len(), "matched": test_matched, "total": test_total, "not_covered_frames": uncovered, "not_covered": test_missing, "per_clip": test_clips},
        "from_clips": fitted_from,
    });
    // With no harvested clips this time, the last held-out test stays,
    // marked as from an earlier run.
    let mut result = result;
    if test.is_empty()
        && out_file.is_none()
        && let Some(prev) = fs::read_to_string(out_dir.join("refine.json"))
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        && prev["held_out"]["clips"].as_u64().unwrap_or(0) > 0
    {
        let mut h = prev["held_out"].clone();
        h["earlier_run"] = json!(true);
        result["held_out"] = h;
    }
    result["fitted_on"] = json!(fitted_from_words(fit_on));
    let out_path = out_file.map_or_else(|| out_dir.join("refine.json"), Path::to_path_buf);
    if let Some(d) = out_path.parent() {
        fs::create_dir_all(d)?;
    }
    fs::write(&out_path, serde_json::to_string(&result)?)?;
    let (fitted, held) = (fitted_from_words(fit_on), held_from(fit_on));
    status(
        &out_dir,
        &if out_file.is_none() {
            format!(
                "done: matches {matched} of {total} frames of the jumps in {fitted}; on {held}, {test_matched} of {test_total} frames it covers ({uncovered} frames are cases not in {fitted} yet)"
            )
        } else {
            "done (a test fit written elsewhere; this view still shows the rule fitted on your clips)".to_owned()
        },
    );
    let mut text = format!(
        "jump rule for {game}: matches {matched} of {total} frames in the air in the clips it was fitted on ({fitted})\n\
         held out ({held}, {} clips, {} jumps): {test_matched} of {test_total} frames the rule covers; {uncovered} frames in cases it has no gravity for\n",
        test.len(),
        test_jumps.len()
    );
    for (k, n) in &test_missing {
        text.push_str(&format!("  not covered: {k} ({n} frames)\n"));
    }
    for c in result["rule"].as_array().into_iter().flatten() {
        text.push_str(&format!(
            "  {}: {} {} ({})\n",
            c["name"].as_str().unwrap_or(""),
            c["value"],
            c["unit"].as_str().unwrap_or(""),
            c["source"].as_str().unwrap_or("")
        ));
    }
    Ok(Outcome::ok(result, text))
}

/// The fit's set in words.
fn fitted_from_words(fit_on: &str) -> &'static str {
    match fit_on {
        "harvest" => "the harvested run's clips",
        "yours" => "your clips",
        _ => "the game's corpus",
    }
}

/// The held-out set in words.
fn held_from(fit_on: &str) -> &'static str {
    match fit_on {
        "harvest" => "your clips, the corpus and the logs given",
        "yours" => "the harvested run, the corpus and the logs given",
        _ => "the mechanic sessions' clips and the logs given",
    }
}

/// Which part of the rule the code gives: the old tool read the routines
/// that write y and the speed from a ROM dataset, which this gp has none
/// of yet.
fn code_side() -> Vec<Value> {
    vec![json!({
        "name": "routines that move y",
        "value": "not read",
        "unit": "routine",
        "source": "code",
        "from": "no ROM dataset in this gp yet: the routines aren't named",
    })]
}

/// The player's y and vertical speed bytes: the first `ram` address of the
/// state mapping's `entities.player[0].y` and `.vy`, as game-printer's
/// `player_bytes` read a template's mapping.
fn player_bytes(game_dir: &Path) -> Result<(u16, Option<u16>)> {
    let file = game_dir.join(gp_interp::mapping::MAPPING_FILE);
    let m: toml::Table = toml::from_str(
        &fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?,
    )
    .with_context(|| file.display().to_string())?;
    let field = |path: &str| -> Option<u16> {
        m.get("field")?.as_array()?.iter().find_map(|f| {
            (f.get("path")?.as_str()? == path)
                .then(|| f.get("ram")?.as_array()?.first()?.as_integer())
                .flatten()
                .map(|a| a as u16)
        })
    };
    let y = field("entities.player[0].y")
        .context("the state mapping reads no entities.player[0].y from RAM")?;
    Ok((y, field("entities.player[0].vy")))
}

/// $GP_HOME from a game's mechanics folder ($GP_HOME/mechanics/GAME).
fn gp_home(game_dir: &Path) -> &Path {
    game_dir.ancestors().nth(2).unwrap_or(game_dir)
}

/// Replays the clips of the sessions (all of them, or `only`): each clip
/// keeps its RAM and buttons from a few frames before its start to its end.
/// Clips whose input logs are the start of a longer clip's log are cut from
/// that one replay, so a run harvested into many clips is played once.
fn load_traces(
    rom: &[u8],
    base: usize,
    home: &Path,
    only: &dyn Fn(&str) -> bool,
) -> Result<Vec<Trace>> {
    struct Clip {
        session: String,
        n: u64,
        start: usize,
        frames: usize,
        log: String,
    }
    let mut clips = Vec::new();
    let Ok(rd) = fs::read_dir(home) else {
        return Ok(Vec::new());
    };
    let mut dirs: Vec<PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.join("clips").is_dir())
        .collect();
    dirs.sort();
    for d in dirs {
        let session = d
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_owned();
        if !only(&session) {
            continue;
        }
        let mut logs: Vec<PathBuf> = fs::read_dir(d.join("clips"))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "log"))
            .collect();
        logs.sort();
        for log in logs {
            let n: u64 = log
                .file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            let meta: toml::Table =
                toml::from_str(&fs::read_to_string(log.with_extension("toml"))?)?;
            let start = meta.get("start").and_then(|v| v.as_integer()).unwrap_or(0) as usize;
            let frames = meta.get("frames").and_then(|v| v.as_integer()).unwrap_or(0) as usize;
            clips.push(Clip {
                session: session.clone(),
                n,
                start,
                frames,
                // The full log from power-on, also for a clip saved as a
                // path, a frame on it and its own buttons.
                log: crate::mechanic::paths::full_log(gp_home(home), &d, n)?.0,
            });
        }
    }
    // Longest logs first; each later log that one of them starts with is
    // cut from its replay.
    let mut order: Vec<usize> = (0..clips.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(clips[i].log.len()));
    let mut owner = vec![usize::MAX; clips.len()];
    let mut masters: Vec<usize> = Vec::new();
    for &i in &order {
        let body = clips[i].log.trim_end();
        match masters.iter().find(|&&m| clips[m].log.starts_with(body)) {
            Some(&m) => owner[i] = m,
            None => {
                masters.push(i);
                owner[i] = i;
            }
        }
    }
    const BEFORE: usize = 8;
    let mut out = Vec::new();
    for &m in &masters {
        let mine: Vec<usize> = (0..clips.len()).filter(|&i| owner[i] == m).collect();
        let parsed = gp_evidence::input_log::InputLog::parse(&clips[m].log)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let last = mine
            .iter()
            .map(|&i| clips[i].start + clips[i].frames)
            .max()
            .unwrap_or(0)
            .min(parsed.frames.len());
        let want = |f: usize| {
            mine.iter()
                .any(|&i| f + BEFORE >= clips[i].start && f < clips[i].start + clips[i].frames)
        };
        let (ram, boxes) = replay(rom, base, &parsed, last, &want)?;
        for &i in &mine {
            let c = &clips[i];
            let base = c.start.saturating_sub(BEFORE);
            let end = (c.start + c.frames).min(last);
            if end <= base {
                continue;
            }
            out.push(Trace {
                session: c.session.clone(),
                clip: c.n,
                start: c.start - base,
                ram: (base..end).map(|f| ram[&f].clone()).collect(),
                pads: (base..end).map(|f| parsed.frames[f].pads[0]).collect(),
                boxes: (base..end).map(|f| boxes[&f].clone()).collect(),
            });
        }
    }
    out.sort_by(|a, b| (a.session.as_str(), a.clip).cmp(&(b.session.as_str(), b.clip)));
    Ok(out)
}

/// The original played on `log` from power-on through `Runtime`, for its
/// first `last` frames: the RAM window from `base` and the sprite boxes
/// after each frame `want` keeps.
#[allow(clippy::type_complexity)]
fn replay(
    rom: &[u8],
    base: usize,
    log: &gp_evidence::InputLog,
    last: usize,
    want: &dyn Fn(usize) -> bool,
) -> Result<(BTreeMap<usize, Vec<u8>>, BTreeMap<usize, Vec<[i32; 4]>>)> {
    let mut o = OriginalRuntime::of_rom(rom)?;
    o.set_video(false);
    let inputs = crate::run::frames(log);
    let window = format!("ram[{base:#x}..{:#x}]", base + WINDOW);
    let byte = |e: &gp_runtime::Entry| e.value.as_int().unwrap_or(0) as u8;
    let mut ram: BTreeMap<usize, Vec<u8>> = BTreeMap::new();
    let mut boxes: BTreeMap<usize, Vec<[i32; 4]>> = BTreeMap::new();
    for (f, input) in inputs.iter().enumerate().take(last) {
        o.step(std::slice::from_ref(input))
            .map_err(|e| anyhow::anyhow!("the original: {e}"))?;
        if want(f) {
            let r = o.query(&window).map_err(anyhow::Error::msg)?;
            ram.insert(f, r.iter().map(byte).collect());
            // The sprites, where the runtime has an NES-style OAM.
            let b = match o.query("oam[*]") {
                Ok(oam) if oam.len() == 256 => {
                    let mut a = [0u8; 256];
                    for (x, e) in a.iter_mut().zip(&oam) {
                        *x = byte(e);
                    }
                    let tall = o
                        .query("ppu.ctrl")
                        .ok()
                        .and_then(|e| e.first().and_then(|e| e.value.as_int()))
                        .is_some_and(|c| c & 0x20 != 0);
                    sprite_boxes(&a, if tall { 16 } else { 8 })
                }
                _ => Vec::new(),
            };
            boxes.insert(f, b);
        }
    }
    Ok((ram, boxes))
}

/// The game's corpus clips, each replayed as a trace from a few frames
/// before its start to its end.
fn corpus_traces(rom: &[u8], base: usize, game_dir: &Path) -> Result<Vec<Trace>> {
    const BEFORE: usize = 8;
    let set = crate::clipset::read_game(game_dir)?;
    let mut out = Vec::new();
    for r in &set.clips {
        let Some((log, start)) = crate::clipset::recipe_log(game_dir, r)? else {
            continue;
        };
        let lo = start.saturating_sub(BEFORE);
        let end = log.frames.len();
        let (ram, _) = replay(rom, base, &log, end, &|f| f >= lo)?;
        out.push(Trace {
            session: format!("corpus {}", r.id),
            clip: 0,
            start: start - lo,
            ram: (lo..end).map(|f| ram[&f].clone()).collect(),
            pads: (lo..end).map(|f| log.frames[f].pads[0]).collect(),
            boxes: (lo..end).map(|_| Vec::new()).collect(),
        });
    }
    Ok(out)
}

/// A whole input log from power-on, replayed as one held-out trace.
fn whole_log(rom: &[u8], base: usize, log: &Path) -> Result<Trace> {
    let parsed = gp_evidence::input_log::InputLog::parse(&fs::read_to_string(log)?)
        .map_err(|e| anyhow::anyhow!("{}: {e}", log.display()))?;
    let n = parsed.frames.len();
    let (ram, _) = replay(rom, base, &parsed, n, &|_| true)?;
    Ok(Trace {
        session: format!(
            "log {}",
            log.file_stem().and_then(|s| s.to_str()).unwrap_or("")
        ),
        clip: 0,
        start: 0,
        ram: ram.into_values().collect(),
        pads: parsed.frames.iter().map(|f| f.pads[0]).collect(),
        boxes: (0..n).map(|_| Vec::new()).collect(),
    })
}

/// Sprites that touch, merged into boxes: [x, y, w, h] on the screen.
fn sprite_boxes(oam: &[u8; 256], tall: i32) -> Vec<[i32; 4]> {
    let mut b: Vec<[i32; 4]> = oam
        .chunks(4)
        .filter(|e| e[0] < 0xEF)
        .map(|e| [i32::from(e[3]), i32::from(e[0]) + 1, 8, tall])
        .collect();
    let touch = |a: &[i32; 4], c: &[i32; 4]| {
        a[0] <= c[0] + c[2] && c[0] <= a[0] + a[2] && a[1] <= c[1] + c[3] && c[1] <= a[1] + a[3]
    };
    'again: loop {
        for i in 0..b.len() {
            for k in i + 1..b.len() {
                if touch(&b[i], &b[k]) {
                    let (a, c) = (b[i], b[k]);
                    let (x0, y0) = (a[0].min(c[0]), a[1].min(c[1]));
                    let (x1, y1) = (
                        (a[0] + a[2]).max(c[0] + c[2]),
                        (a[1] + a[3]).max(c[1] + c[3]),
                    );
                    b[i] = [x0, y0, x1 - x0, y1 - y0];
                    b.swap_remove(k);
                    continue 'again;
                }
            }
        }
        return b;
    }
}

/// The player's box on each frame of a trace from its start: the box whose
/// top keeps the same distance from the player's y byte (that distance is
/// the median nearest one over the clip).
fn player_boxes(t: &Trace, yb: usize) -> Vec<Option<[i32; 4]>> {
    let mut offs: Vec<i32> = Vec::new();
    for (b, r) in t.boxes.iter().zip(&t.ram).skip(t.start) {
        let y = i32::from(r[yb]);
        if let Some(o) = b.iter().map(|q| q[1] - y).min_by_key(|o| o.abs()) {
            offs.push(o);
        }
    }
    offs.sort_unstable();
    let off = offs.get(offs.len() / 2).copied().unwrap_or(0);
    t.boxes
        .iter()
        .zip(&t.ram)
        .skip(t.start)
        .map(|(b, r)| {
            let y = i32::from(r[yb]);
            b.iter()
                .copied()
                .min_by_key(|q| (q[1] - y - off).abs())
                .filter(|q| (q[1] - y - off).abs() <= 8)
        })
        .collect()
}

/// The page's routes for the refinement view: the result, what it's doing
/// now, and a clip's input log from any session.
pub(crate) fn route(home: &Path, req: &Request) -> Result<Option<Response>> {
    if req.method != "GET" && req.method != "HEAD" {
        return Ok(None);
    }
    let name_ok = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
    };
    match req.path.as_str() {
        "/api/rule" => {
            let m = req.param("mechanic").unwrap_or("jump");
            if !name_ok(m) {
                return Ok(Some(Response::error(400, "bad mechanic")));
            }
            let d = home.join(m);
            let rule: Value = fs::read_to_string(d.join("refine.json"))
                .ok()
                .and_then(|t| serde_json::from_str(&t).ok())
                .unwrap_or(Value::Null);
            let st: Value = fs::read_to_string(d.join("refine.status.json"))
                .ok()
                .and_then(|t| serde_json::from_str(&t).ok())
                .unwrap_or(Value::Null);
            let mut rule = rule;
            if let Some(h) = rule.get_mut("held_out").and_then(Value::as_object_mut) {
                h.remove("per_clip");
            }
            Ok(Some(Response::json(
                &json!({"mechanic": m, "rule": rule, "status": st["text"]}),
            )))
        }
        "/api/rule/log" => {
            let (Some(sn), Some(c)) = (
                req.param("session"),
                req.param("clip").and_then(|c| c.parse::<u64>().ok()),
            ) else {
                return Ok(Some(Response::error(400, "session and clip")));
            };
            if !name_ok(sn) {
                return Ok(Some(Response::error(400, "bad session")));
            }
            if !home.join(sn).join(format!("clips/{c:03}.toml")).is_file() {
                return Ok(Some(Response::error(404, "no such clip")));
            }
            let (log, _) = crate::mechanic::paths::full_log(gp_home(home), &home.join(sn), c)?;
            Ok(Some(Response::bytes(
                "text/plain; charset=utf-8",
                log.into_bytes(),
            )))
        }
        _ => Ok(None),
    }
}

/// Each jump: the player rests (y the same 3 frames), then y goes up with
/// A pressed; it ends when y rests again or the clip ends.
fn find_jumps(traces: &[Trace], yb: usize) -> Vec<Jump> {
    let mut out = Vec::new();
    for (ti, t) in traces.iter().enumerate() {
        let y = |i: usize| t.ram[i][yb];
        let mut i = t.start.max(4);
        while i + 1 < t.ram.len() {
            let rest = y(i) == y(i - 1) && y(i - 1) == y(i - 2);
            let pressed = t.pads[i + 1] & 1 != 0 && t.pads[i] & 1 == 0 || t.pads[i + 1] & 1 != 0;
            if rest && y(i + 1) < y(i) && pressed {
                let from = i;
                let mut j = i + 1;
                while j + 2 < t.ram.len() && !(y(j) == y(j + 1) && y(j + 1) == y(j + 2)) {
                    j += 1;
                }
                out.push(Jump {
                    trace: ti,
                    from,
                    to: j,
                });
                i = j;
            }
            i += 1;
        }
    }
    out
}

struct Fractions {
    pos: Option<usize>,
    speed: Option<usize>,
    /// The game adds gravity before it moves (the speed after a frame is
    /// the move that frame made), rather than moving by the old speed.
    gravity_first: bool,
}

/// The byte holding y's fraction makes y*256+frac change by the speed
/// (speed_y*256 + its fraction) each frame in the air. Every byte is tried
/// as y's fraction, then every byte as the speed's.
fn find_fractions(traces: &[Trace], jumps: &[Jump], yb: usize, vyb: usize) -> Fractions {
    let f = find_fractions_order(traces, jumps, yb, vyb, false);
    if f.pos.is_some() {
        return f;
    }
    find_fractions_order(traces, jumps, yb, vyb, true)
}

fn find_fractions_order(
    traces: &[Trace],
    jumps: &[Jump],
    yb: usize,
    vyb: usize,
    gravity_first: bool,
) -> Fractions {
    // Frames well inside each jump: (trace, frame).
    let frames: Vec<(usize, usize)> = jumps
        .iter()
        .flat_map(|j| (j.from + 2..j.to.saturating_sub(2)).map(move |i| (j.trace, i)))
        .collect();
    if frames.len() < 20 {
        return Fractions {
            pos: None,
            speed: None,
            gravity_first,
        };
    }
    // At most 600 frames, spread over all the jumps: enough to tell the
    // bytes apart, and the search tries every pair of bytes.
    let step = frames.len().div_ceil(600);
    let frames: Vec<(usize, usize)> = frames.into_iter().step_by(step).collect();
    let mut best: Option<(usize, usize, usize)> = None; // (hits, pos, speed)
    for p in 0..0x800usize {
        if p == yb || p == vyb {
            continue;
        }
        // d = the position's change less the whole-pixel speed: the
        // speed's fraction, if p is y's fraction.
        let d: Vec<i32> = frames
            .iter()
            .map(|&(ti, i)| {
                let r0 = &traces[ti].ram[i];
                let r1 = &traces[ti].ram[i + 1];
                let pos = |r: &Vec<u8>| i32::from(r[yb]) * 256 + i32::from(r[p]);
                // The speed that made the move: the old one, or with
                // gravity first, the new one.
                let by = if gravity_first { r1 } else { r0 };
                pos(r1) - pos(r0) - signed(by[vyb]) * 256
            })
            .collect();
        let inside = d.iter().filter(|v| (0..256).contains(*v)).count();
        if inside * 10 < frames.len() * 9 {
            continue;
        }
        for s in 0..0x800usize {
            if s == yb || s == vyb || s == p {
                continue;
            }
            let hits = frames
                .iter()
                .zip(&d)
                .filter(|(fr, dv)| {
                    let at = if gravity_first { fr.1 + 1 } else { fr.1 };
                    i32::from(traces[fr.0].ram[at][s]) == **dv
                })
                .count();
            if hits * 10 >= frames.len() * 9 && best.is_none_or(|b| hits > b.0) {
                best = Some((hits, p, s));
            }
        }
    }
    match best {
        Some((_, p, s)) => Fractions {
            pos: Some(p),
            speed: Some(s),
            gravity_first,
        },
        None => Fractions {
            pos: None,
            speed: None,
            gravity_first,
        },
    }
}

struct Rule {
    /// Gravity by case ("A held, rising, takeoff class N", ...).
    gravity: BTreeMap<String, i32>,
    takeoff: BTreeMap<String, i32>,
    max_fall: i32,
    /// Falling: a new speed at or above `cap_above` drops to `cap_to`
    /// (both in 1/256 px a frame).
    cap_above: i32,
    cap_to: i32,
    order: &'static str,
    /// Takeoff classes: horizontal speed thresholds that split them.
    classes: Vec<i32>,
}

/// A takeoff's class: its exact first speed in the air. Takeoffs that
/// differ by a fraction are different classes (the game sets the speed by
/// how fast the player ran), and each has its own gravity.
fn class_of(classes: &mut Vec<i32>, v0: i32) -> String {
    let c = match classes.iter().find(|c| **c == v0) {
        Some(c) => *c,
        None => {
            classes.push(v0);
            v0
        }
    };
    format!("takeoff {c}/256 px a frame")
}

/// Gravity's case on a frame: A held since takeoff while still rising, or
/// anything else. Letting go of A latches: pressing it again during the
/// same rise doesn't bring the lighter gravity back.
fn case(class: &str, held: bool, v: i32) -> String {
    if held && v < 0 {
        format!("{class}, A held while rising")
    } else {
        format!("{class}, otherwise")
    }
}

fn speed16(r: &[u8], vyb: usize, vf: Option<usize>) -> i32 {
    signed(r[vyb]) * 256 + vf.map_or(0, |f| i32::from(r[f]))
}

/// The horizontal speed at takeoff, in whole pixels: the player's x over
/// the frames before it left the ground is not known here, so the class
/// comes from the takeoff speed itself.
fn fit_rule(traces: &[Trace], jumps: &[Jump], _yb: usize, vyb: usize, frac: &Fractions) -> Rule {
    let mut gravity_samples: BTreeMap<String, BTreeMap<i32, usize>> = BTreeMap::new();
    let mut takeoff: BTreeMap<String, i32> = BTreeMap::new();
    let mut max_fall = 0;
    let mut classes = Vec::new();
    let mut top = 0;
    let mut drops: BTreeMap<i32, usize> = BTreeMap::new();
    let mut before_drop: BTreeMap<i32, usize> = BTreeMap::new();
    for j in jumps {
        let t = &traces[j.trace];
        let v0 = speed16(&t.ram[j.from + 1], vyb, frac.speed);
        let class = class_of(&mut classes, v0);
        takeoff.entry(class.clone()).or_insert(v0);
        let mut held_since = true;
        for i in j.from + 1..j.to.saturating_sub(1) {
            let v = speed16(&t.ram[i], vyb, frac.speed);
            let v1 = speed16(&t.ram[i + 1], vyb, frac.speed);
            max_fall = max_fall.max(signed(t.ram[i + 1][vyb]));
            top = top.max(v1);
            if v > 0 && v1 < v {
                *drops.entry(v1).or_default() += 1;
                *before_drop.entry(v).or_default() += 1;
            }
            held_since &= t.pads[i + 1] & 1 != 0;
            let key = case(&class, held_since, v);
            *gravity_samples
                .entry(key)
                .or_default()
                .entry(v1 - v)
                .or_default() += 1;
        }
    }
    // A class seen in only one or two frames is noise (a bounce, a hit).
    gravity_samples.retain(|_, m| m.values().sum::<usize>() >= 2);
    // Each case's gravity is its most common change (a cap or a landing
    // shows as an outlier).
    let gravity: BTreeMap<String, i32> = gravity_samples
        .into_iter()
        .filter_map(|(k, m)| m.into_iter().max_by_key(|(_, n)| *n).map(|(g, _)| (k, g)))
        .collect();
    let _ = before_drop;
    let cap_to = drops
        .into_iter()
        .max_by_key(|(_, n)| *n)
        .map_or(top, |(v, _)| v);
    // The fastest fall: a falling speed that would reach a threshold drops
    // back to `cap_to` instead. The threshold is found from which new speeds
    // dropped and which were kept.
    let (mut dropped, mut kept) = (Vec::new(), Vec::new());
    let mut classes2 = Vec::new();
    for j in jumps {
        let t = &traces[j.trace];
        let class = class_of(&mut classes2, speed16(&t.ram[j.from + 1], vyb, frac.speed));
        let mut held_since = true;
        for i in j.from + 1..j.to.saturating_sub(1) {
            let v = speed16(&t.ram[i], vyb, frac.speed);
            let v1 = speed16(&t.ram[i + 1], vyb, frac.speed);
            held_since &= t.pads[i + 1] & 1 != 0;
            let Some(g) = gravity.get(&case(&class, held_since, v)) else {
                continue;
            };
            let next = v + g;
            if v <= 0 || next <= cap_to {
                continue;
            }
            if v1 == cap_to && next != v1 {
                dropped.push(next);
            } else if v1 == next {
                kept.push(next);
            }
        }
    }
    dropped.sort_unstable();
    kept.sort_unstable();
    // The threshold that best splits them: the most dropped speeds at or
    // above it and kept speeds below it (a rare other state, a spring or a
    // bounce, lands on the wrong side and is outvoted).
    let score = |t: i32| {
        dropped.iter().filter(|d| **d >= t).count() + kept.iter().filter(|k| **k < t).count()
    };
    let cap_at = dropped
        .iter()
        .chain(kept.iter())
        .copied()
        .max_by_key(|t| (score(*t), std::cmp::Reverse(*t)))
        .unwrap_or(top + 1);
    Rule {
        gravity,
        takeoff,
        max_fall,
        cap_above: cap_at,
        cap_to,
        order: if frac.gravity_first {
            "add gravity, then move by the speed"
        } else {
            "move by the speed, then add gravity"
        },
        classes,
    }
}

fn simulate_all(
    traces: &[Trace],
    jumps: &[Jump],
    yb: usize,
    vyb: usize,
    frac: &Fractions,
    rule: &Rule,
) -> (Vec<Value>, usize, usize, BTreeMap<String, usize>) {
    let mut classes = rule.classes.clone();
    let mut missing: BTreeMap<String, usize> = BTreeMap::new();
    let mut by_clip: BTreeMap<(String, u64), (usize, Vec<Value>)> = BTreeMap::new();
    let mut boxes_of: BTreeMap<(String, u64), Value> = BTreeMap::new();
    for t in traces {
        boxes_of.insert(
            (t.session.clone(), t.clip),
            json!({"frames": t.ram.len() - t.start, "player_box": player_boxes(t, yb)}),
        );
    }
    let (mut matched, mut total) = (0, 0);
    for j in jumps {
        let t = &traces[j.trace];
        let r0 = &t.ram[j.from + 1];
        let mut pos = i32::from(r0[yb]) * 256 + frac.pos.map_or(0, |p| i32::from(r0[p]));
        let mut v = speed16(r0, vyb, frac.speed);
        let class = class_of(&mut classes, v);
        let mut predicted = Vec::new();
        let mut reals = Vec::new();
        let mut matches = Vec::new();
        let mut ended = "landed";
        let mut held_since = true;
        // Up to the frame before it stood again: on that frame the floor
        // stops it, which is landing, not the jump's rule.
        for i in j.from + 1..j.to {
            if i > j.from + 1 {
                // Something stopped the rise (a hit): the speed turned over
                // by more than gravity ever adds.
                let real_v = speed16(&t.ram[i], vyb, frac.speed);
                let prev_v = speed16(&t.ram[i - 1], vyb, frac.speed);
                if prev_v < 0 && real_v - prev_v > 256 {
                    ended = "stopped by a hit";
                    break;
                }
                held_since &= t.pads[i] & 1 != 0;
                let key = case(&class, held_since, v);
                let Some(g) = rule.gravity.get(&key).copied() else {
                    // A case the fitted clips never showed: the rule can't
                    // say, so the rest of this jump isn't counted.
                    ended = "a case the rule has no gravity for";
                    *missing.entry(key).or_default() += j.to - i;
                    break;
                };
                // Something else changed the speed (a landing on something
                // that moves, a stomp's bounce, a push): the real speed is
                // more than 1 px a frame off what the rule makes of the last
                // real speed, or a fall stopped dead. A smaller miss stays a
                // miss of the rule.
                let mut expect = prev_v + g;
                if expect >= rule.cap_above {
                    expect = rule.cap_to;
                }
                if (real_v - expect).abs() > 256 || (prev_v > 0 && real_v == 0) {
                    ended = "something else changed the speed";
                    break;
                }
                pos += v;
                v += g;
                if v >= rule.cap_above {
                    v = rule.cap_to;
                }
            }
            // The y byte wraps above the screen; compare it as a byte.
            let py = pos.div_euclid(256).rem_euclid(256);
            let real = i32::from(t.ram[i][yb]);
            predicted.push(py);
            reals.push(real);
            let ok = py == real;
            matches.push(ok);
            matched += usize::from(ok);
            total += 1;
        }
        if std::env::var_os("GP_RULE_DUMP").is_some() && matches.iter().any(|m| !m) {
            for i in j.from..j.to {
                eprintln!(
                    "  f{} y {} v {} pad {:02x}",
                    i - t.start,
                    t.ram[i][yb],
                    speed16(&t.ram[i], vyb, frac.speed),
                    t.pads[i]
                );
            }
        }
        if std::env::var_os("GP_RULE_TRACE").is_some() {
            let first = matches.iter().position(|m| !m);
            eprintln!(
                "{} clip {} frame {}: {} frames, {class}, {ended}, first miss {:?}, real {:?}",
                t.session,
                t.clip,
                j.from + 1 - t.start,
                matches.len(),
                first,
                first.map(|k| (predicted[k], t.ram[j.from + 1 + k][yb]))
            );
        }
        let e = by_clip
            .entry((t.session.clone(), t.clip))
            .or_insert((t.start, Vec::new()));
        e.1.push(json!({"from": j.from + 1 - t.start, "predicted_y": predicted, "real_y": reals, "match": matches, "ended": ended}));
    }
    let clips = by_clip
        .into_iter()
        .map(|((s, c), (_, jumps))| {
            let b = boxes_of.get(&(s.clone(), c)).cloned().unwrap_or(Value::Null);
            json!({"session": s, "clip": c, "jumps": jumps, "frames": b["frames"], "player_box": b["player_box"]})
        })
        .collect();
    (clips, matched, total, missing)
}

/// The jump rule fitted from given traces (each frame's RAM and pad, from
/// the frame the trace starts), with no file read: for a tool that plays
/// the game itself, as `gp model blind` does. `yb` is the player's y byte
/// and `vyb` its vertical speed's whole byte. Returns the rule (takeoff
/// speeds and gravities in 1/256 px a frame, by case), the fraction bytes
/// found, how many jumps it saw, and the frames it reproduces.
pub(crate) fn fit_traces(traces: Vec<(Vec<Vec<u8>>, Vec<u8>)>, yb: usize, vyb: usize) -> Value {
    let traces: Vec<Trace> = traces
        .into_iter()
        .enumerate()
        .map(|(i, (ram, pads))| Trace {
            session: "given".into(),
            clip: i as u64,
            start: 0,
            ram,
            pads,
            boxes: Vec::new(),
        })
        .collect();
    let jumps = find_jumps(&traces, yb);
    let frac = find_fractions(&traces, &jumps, yb, vyb);
    let rule = fit_rule(&traces, &jumps, yb, vyb, &frac);
    json!({
        "jumps": jumps.len(),
        "y_fraction": frac.pos,
        "speed_fraction": frac.speed,
        "takeoff": rule.takeoff,
        "gravity": rule.gravity,
        "max_fall": rule.max_fall,
        "cap_above": rule.cap_above,
        "cap_to": rule.cap_to,
        "order": rule.order,
        "gravity_first": frac.gravity_first,
    })
}

/// Fits the rule on `train` and plays it on `test`, both given as traces
/// (each frame's RAM and pad), with no file read: a held-out check for a
/// tool that gathers its own traces. Returns the rule as [`fit_traces`]
/// does, plus the test's frames matched and total and the cases it had no
/// gravity for.
pub(crate) fn fit_and_test(
    train: Vec<(Vec<Vec<u8>>, Vec<u8>)>,
    test: Vec<(Vec<Vec<u8>>, Vec<u8>)>,
    yb: usize,
    vyb: usize,
) -> Value {
    let to_traces = |v: Vec<(Vec<Vec<u8>>, Vec<u8>)>, name: &str| -> Vec<Trace> {
        v.into_iter()
            .enumerate()
            .map(|(i, (ram, pads))| Trace {
                session: name.into(),
                clip: i as u64,
                start: 0,
                ram,
                pads,
                boxes: Vec::new(),
            })
            .collect()
    };
    let train = to_traces(train, "train");
    let test = to_traces(test, "test");
    let jumps = find_jumps(&train, yb);
    let frac = find_fractions(&train, &jumps, yb, vyb);
    let rule = fit_rule(&train, &jumps, yb, vyb, &frac);
    let fit = simulate_all(&train, &jumps, yb, vyb, &frac, &rule);
    let test_jumps = find_jumps(&test, yb);
    let held = simulate_all(&test, &test_jumps, yb, vyb, &frac, &rule);
    json!({
        "jumps": jumps.len(),
        "test_jumps": test_jumps.len(),
        "y_fraction": frac.pos,
        "speed_fraction": frac.speed,
        "takeoff": rule.takeoff,
        "gravity": rule.gravity,
        "max_fall": rule.max_fall,
        "cap_above": rule.cap_above,
        "cap_to": rule.cap_to,
        "order": rule.order,
        "gravity_first": frac.gravity_first,
        "fitted": {"matched": fit.1, "total": fit.2},
        "held_out": {"matched": held.1, "total": held.2, "uncovered": held.3},
    })
}
