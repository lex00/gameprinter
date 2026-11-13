//! `gp model blind` for a player who changes lanes (Wave T): what
//! discovery found (the lane's y and number, the held button, the score)
//! measured on scripted runs and the search's path, the lane rule fitted
//! from them with `gp rule lane`'s fitter and checked frame by frame, the
//! system order from the board's access log, and the facts and mapping
//! written as the jump path writes its own.

use super::*;
use crate::rule_refine::lane::{self as rule_lane, LaneTrace};

/// What discovery found that the lane path reads.
pub(super) struct Found {
    pub lanes: crate::farm::lanes::Lanes,
    pub held: Option<crate::farm::lanes::Held>,
    pub score: Option<Vec<usize>>,
    pub pads: crate::farm::machine::Pads,
    pub at: usize,
    pub power_on: gp_evidence::manifest::PowerOn,
}

fn trace(name: &str, seen: &[Seen], b: (usize, usize, usize)) -> LaneTrace {
    LaneTrace {
        name: name.to_owned(),
        start: 0,
        seen: seen
            .iter()
            .map(|s| rule_lane::Seen {
                lane: i32::from(s.wram[b.0]),
                x: i32::from(s.wram[b.1]),
                y: i32::from(s.wram[b.2]),
            })
            .collect(),
        taps: seen.iter().map(|_| Vec::new()).collect(),
        pads: seen.iter().map(|s| s.pad).collect(),
    }
}

#[allow(clippy::too_many_lines)]
pub(super) fn run(
    a: &BlindArgs,
    paths: &Paths,
    bytes: &[u8],
    rom_ref: &str,
    st: &mut Start,
    f: Found,
) -> Result<Outcome> {
    let base = st.nes.ram_base();
    let hex = |v: usize| format!("0x{:04X}", v + base);
    let ln = &f.lanes;
    let note = |what: &str, from: usize, to: usize| {
        format!("{rom_ref} frames {}-{}: {what}", f.at + from, f.at + to)
    };
    const PROBE: &str = "scripted probe";
    let mut facts: Vec<(String, Value, String, &str)> = Vec::new();
    let arcade = st.nes.is_arcade();
    facts.push((
        "input.device".into(),
        json!(if arcade { "arcade" } else { "nes-pad" }),
        note("the controls the game reads", 0, 0),
        PROBE,
    ));
    for (bit, why) in [
        (f.pads.left, "the stick moves the player along its lane"),
        (f.pads.right, "the stick moves the player along its lane"),
        (ln.up, "the stick moves the player to another lane"),
        (ln.down, "the stick moves the player to another lane"),
    ] {
        let name = f.pads.name(bit);
        facts.push((
            format!("input.actions.{name}"),
            json!(name),
            note(why, 0, 0),
            PROBE,
        ));
    }
    if let Some(h) = &f.held {
        let name = f.pads.name(h.button);
        facts.push((
            format!("input.actions.{name}"),
            json!(name),
            note("held, this button acts after a count", 0, 0),
            PROBE,
        ));
    }
    facts.push((
        "entities.player.role".into(),
        json!("player"),
        note("the sprite that follows the stick", 0, 0),
        PROBE,
    ));

    // Scripted runs: a walk each way, two lanes one way and back, and the
    // button held twice.
    let walk = st.play(&[(0, 4), (st.back, 40), (st.forward, 40)])?;
    // Up four times and down four: every lane of four, and the wrap if
    // the lanes come round.
    // Nothing held first: a press on a trace's first frame has no frame
    // before it to be fresh from.
    let mut lane_pads = vec![(0, 4)];
    for way in [ln.up, ln.down] {
        for _ in 0..4 {
            lane_pads.push((way, 10));
            lane_pads.push((0, 30));
        }
    }
    let lanes_run = st.play(&lane_pads)?;
    let button = f.held.as_ref().map_or(0, |h| h.button);
    let acts = st.play(&[(0, 4), (button, 12), (0, 20), (button, 12), (0, 20)])?;
    if let Some(v) = speed_across(&walk[44..84], st.x, None) {
        facts.push((
            "entities.player.run.walk_speed".into(),
            fixed(v.abs()),
            note("speed along the lane with the stick held", 52, 84),
            PROBE,
        ));
    }
    // The player's size: the sprites that moved with x over the walk's
    // first 20 frames (by x's move, at the same height), put together.
    if walk.len() > 20 {
        let (s0, s1) = (&walk[44], &walk[64]);
        let dx = i64::from(s1.wram[st.x]) - i64::from(s0.wram[st.x]);
        let mine: Vec<[i64; 4]> = s0
            .cells
            .iter()
            .copied()
            .filter(|c| {
                s1.cells
                    .iter()
                    .any(|d| (d[1] - c[1]).abs() <= 2 && (d[0] - c[0] - dx).abs() <= 2)
            })
            .collect();
        if dx != 0
            && let (Some(x0), Some(y0), Some(x1), Some(y1)) = (
                mine.iter().map(|c| c[0]).min(),
                mine.iter().map(|c| c[1]).min(),
                mine.iter().map(|c| c[0] + c[2]).max(),
                mine.iter().map(|c| c[1] + c[3]).max(),
            )
        {
            facts.push((
                "entities.player.size".into(),
                json!([x1 - x0, y1 - y0]),
                note("the sprites that moved with x on a walk", 44, 64),
                PROBE,
            ));
        }
    }

    // The search, with what discovery found as its moves and the score as
    // progress.
    let mut played = Value::Null;
    let mut path_seen: Vec<Seen> = Vec::new();
    if a.play_rounds > 0 {
        let out_dir = a.model.with_extension("play");
        let o = crate::farm::blind::Opts {
            decide: a.decide.clone(),
            rounds: a.play_rounds,
            top: 3,
            unsure: 0.4,
            frontier: 256,
            seed: 1,
            seconds: f64::INFINITY,
            levels: 1,
            session: None,
        };
        let v = crate::farm::blind::farm(&a.rom, &o, paths, &out_dir)
            .context("the search over the level")?;
        if let Ok(text) = fs::read_to_string(out_dir.join("path.log")) {
            path_seen = replay(bytes, f.power_on, &text)?;
        }
        played = v;
    }

    // The lane rule, fitted from those runs (`gp rule lane`'s fitter) and
    // played against each frame by frame.
    let mut rule_v = Value::Null;
    if let Some(lane) = ln.lane {
        let b = (lane, st.x, ln.y);
        let mut traces = vec![
            trace("walk", &walk, b),
            trace("lanes", &lanes_run, b),
            trace("button", &acts, b),
        ];
        if path_seen.len() > f.at {
            traces.push(LaneTrace {
                start: f.at,
                ..trace("search path", &path_seen, b)
            });
        }
        let pad = rule_lane::Pad {
            right: f.pads.right,
            left: f.pads.left,
            down: ln.down,
            up: ln.up,
            button,
        };
        match rule_lane::fit(&traces, &pad) {
            Ok(rule) => {
                let checked: Vec<Value> = traces
                    .iter()
                    .map(|t| {
                        let c = rule_lane::check(&rule, &pad, t);
                        json!({"run": t.name, "matched": c.matched, "frames": c.frames, "first_difference": c.first})
                    })
                    .collect();
                let p = |k: &str, v: Value, why: &str| {
                    (
                        format!("entities.player.ext_params.{k}"),
                        v,
                        note(why, 0, lanes_run.len()),
                        "fitted",
                    )
                };
                facts.push(p(
                    "lanes",
                    json!(rule.lanes),
                    "the lane byte's values with a row of their own",
                ));
                facts.push(p("row_first", json!(rule.row_top), "lane 0's y"));
                facts.push(p(
                    "row_height",
                    json!(rule.row_height),
                    "y's step from one lane to the next",
                ));
                facts.push(p(
                    "switch_frames",
                    json!(rule.switch_frames),
                    "from the press to the lane changing",
                ));
                facts.push(p(
                    "switch_busy",
                    json!(rule.switch_busy),
                    "after a move's press, the frames another press does nothing",
                ));
                facts.push(p(
                    "walk_step",
                    json!(rule.walk_step),
                    "x's step a frame along the lane",
                ));
                facts.push(p(
                    "wrap",
                    json!(i32::from(rule.wrap)),
                    "a move past the last lane comes round to the first",
                ));
                facts.push(p(
                    "up_step",
                    json!(rule.up_step),
                    "the lane a press of up moves to, from the lane's number",
                ));
                if let Some(s) = rule.send_step {
                    facts.push(p(
                        "send_step",
                        json!(s),
                        "how far x steps back when the button is let go after it acted",
                    ));
                }
                facts.push(p(
                    "pour_frames",
                    json!(rule.pour_frames),
                    "the fewest frames held before a release that sent",
                ));
                if let Some(h) = &f.held {
                    facts.push(p(
                        "hold_frames",
                        json!(h.frames),
                        "the frames the held button counts before it acts, from the press",
                    ));
                }
                facts.push(p(
                    "y_up",
                    json!(i32::from(ln.y_up)),
                    "y counts up the screen: the sprites move the other way",
                ));
                rule_v = json!({"rule": format!("{rule:?}"), "checked": checked});
            }
            Err(e) => rule_v = json!({"error": format!("{e:#}")}),
        }
    }

    // The system order, from the board's access log over the scripted runs.
    let at_state = (st.snap.clone(), st.frame, st.held);
    let mut player = vec![st.x, ln.y];
    player.extend(ln.lane);
    let order_pads = [
        (st.back, 40),
        (st.forward, 40),
        (ln.down, 10),
        (0, 30),
        (button, 12),
        (0, 20),
    ];
    let systems_trace =
        crate::farm::lanes::system_order(&mut st.nes, &at_state, &order_pads, &player, st.x)?
            .unwrap_or(Value::Null);
    let player_first = systems_trace["routines"][0]["median_rank"] == json!(0);
    let mut order: Vec<&str> = vec![
        "rules",
        "entity_states",
        "counters",
        "spawn",
        "platformer.player",
        "platformer.enemies",
        "platformer.contact",
        "exits",
        "camera",
    ];
    if !systems_trace.is_null() && !player_first {
        order.retain(|s| *s != "platformer.player");
        let at = order
            .iter()
            .position(|s| *s == "platformer.enemies")
            .unwrap_or(0)
            + 1;
        order.insert(at, "platformer.player");
    }

    // model.toml and mapping.toml.
    fs::create_dir_all(&a.model)?;
    let mf = a.model.join(gp_model::MODEL_FILE);
    let mut doc: toml::Value = match fs::read_to_string(&mf) {
        Ok(t) => toml::from_str(&t)?,
        Err(_) => toml::Value::Table(Default::default()),
    };
    let model_name = doc
        .get("model")
        .and_then(|m| m.get("name"))
        .and_then(|n| n.as_str())
        .unwrap_or("blind")
        .to_owned();
    let source = doc
        .get("model")
        .and_then(|m| m.get("source"))
        .and_then(|n| n.as_str())
        .unwrap_or("")
        .to_owned();
    set(
        &mut doc,
        &format!("model.extensions.{}", gp_model::platformer::NAME),
        json!(gp_model::platformer::VERSION),
    )?;
    for (path, v, _, _) in &facts {
        set(&mut doc, path, v.clone())?;
    }
    set(&mut doc, "model.systems", json!(order))?;
    {
        let ev = doc
            .as_table_mut()
            .context("model.toml is a table")?
            .entry("evidence")
            .or_insert(toml::Value::Table(Default::default()));
        let ev = ev.as_table_mut().context("evidence is a table")?;
        for (path, _, why, how) in &facts {
            let mut e = toml::map::Map::new();
            e.insert("source".into(), "observed".into());
            e.insert("confidence".into(), toml::Value::Integer(80));
            let mut r = toml::map::Map::new();
            r.insert("at".into(), why.clone().into());
            e.insert(
                "refs".into(),
                toml::Value::Array(vec![toml::Value::Table(r)]),
            );
            e.insert("note".into(), format!("found by: {how}").into());
            ev.insert(path.clone(), toml::Value::Table(e));
        }
    }
    fs::write(&mf, toml::to_string(&doc)?)?;
    let mut mapping = format!(
        "# Written by `gp model blind` from a run of the ROM ({rom_ref}).\n\
         schema = {}\nmodel = \"{model_name}\"\nsource = \"{source}\"\nalign = \"polled\"\n\n\
         [[field]]\npath = \"entities.player[0].x\"\nram = [{}]\nnote = \"follows the stick along the lane\"\n\n\
         [[field]]\npath = \"entities.player[0].y\"\nram = [{}]\nnote = \"one row a lane, {} a lane{}\"\n",
        gp_interp::mapping::SCHEMA,
        hex(st.x),
        hex(ln.y),
        ln.step.abs(),
        if ln.y_up {
            "; counts up the screen"
        } else {
            ""
        },
    );
    if let Some(l) = ln.lane {
        mapping.push_str(&format!(
            "\n[[field]]\npath = \"entities.player[0].lane\"\nram = [{}]\nnote = \"the lane, changing on the frame y does\"\n",
            hex(l)
        ));
    }
    if let Some(d) = &f.score {
        mapping.push_str(&format!(
            "\n[[field]]\npath = \"counters.score\"\nram = [{}]\ndigits = true\nnote = \"a decimal digit a byte, most significant first; rises in play and never falls\"\n",
            d.iter().map(|&a| hex(a)).collect::<Vec<_>>().join(", ")
        ));
    }
    fs::write(a.model.join("mapping.toml"), mapping)?;

    let lane_cmd = ln.lane.map(|l| {
        format!(
            "gp rule lane GAME --lane {} --x {} --y {}",
            hex(l),
            hex(st.x),
            hex(ln.y)
        )
    });
    let v = json!({
        "model": a.model.display().to_string(),
        "kind": "lanes",
        "x": hex(st.x),
        "y": hex(ln.y),
        "lane": ln.lane.map(hex),
        "y_counts_up_the_screen": ln.y_up,
        "lane_step": ln.step,
        "up": f.pads.name(ln.up),
        "down": f.pads.name(ln.down),
        "held": f.held.as_ref().map(|h| json!({
            "button": f.pads.name(h.button),
            "timer": hex(h.timer),
            "done": h.done.map(hex),
            "frames": h.frames,
        })),
        "score": f.score.as_ref().map(|d| d.iter().map(|&a| hex(a)).collect::<Vec<_>>()),
        "written": facts.iter().map(|(p, v, w, h)| json!({"path": p, "value": v, "evidence": w, "found_by": h})).collect::<Vec<_>>(),
        "played": played,
        "lane_rule": rule_v,
        "rule_lane_command": lane_cmd,
        "systems_trace": systems_trace,
        "systems": {"order": order, "found_by": if systems_trace.is_null() { "docs/model.md's order" } else { "the player against the other movers from the access log; the rest in docs/model.md's order" }},
    });
    let text = facts
        .iter()
        .map(|(p, v, _, h)| format!("{p} = {v}  ({h})"))
        .collect::<Vec<_>>()
        .join("\n");
    Ok(Outcome::ok(
        v,
        format!(
            "{text}\nmapping: x [{}], y [{}], lane {}\n{}",
            hex(st.x),
            hex(ln.y),
            ln.lane.map_or("none".into(), hex),
            lane_cmd.unwrap_or_default()
        ),
    ))
}
