//! `gp model static` on an arcade romset: the main Z80's code read alone
//! by gp-arcade's harvest ([`gp_arcade::harvest`], the port of gp-nes's to
//! the Z80), in the
//! NES harvest's output (gp_nes::harvest::Harvest), so static.json, the
//! text, `--model` and `--grade` are the same shape.
//!
//! The romset version reads the code without running it: `--clips` and a
//! run's starts are the NES's still and are refused in words, and the
//! asset map isn't read (an MCR asset map places graphics in the GFX
//! chips, not in program ROM). `--unread` reports where the Z80 reading
//! stopped ([`unread`]): the reader runs every routine of the code map, so
//! no routine is unread; its stops are the indirect jumps it couldn't
//! follow.

use std::fs;
use std::path::Path;
use std::time::Instant;

use anyhow::{Result, bail};
use serde_json::{Value, json};

use crate::Outcome;
use crate::model_static::StaticArgs;
use gp_arcade::harvest as z;
use gp_nes::harvest as n;

fn value(v: &z::Value) -> Value {
    match v {
        z::Value::Int(i) => json!(i),
        z::Value::Str(s) => json!(s),
        z::Value::Bool(b) => json!(b),
        z::Value::List(l) => Value::Array(l.iter().map(value).collect()),
    }
}

/// The Z80 harvest in the NES harvest's types.
pub(crate) fn to_nes(h: z::Harvest) -> n::Harvest {
    let ev = |e: z::Evidence| n::Evidence {
        routine: e.routine,
        pc: e.pc,
        insn: e.insn,
    };
    n::Harvest {
        schema: h.schema,
        routines: h
            .routines
            .into_iter()
            .map(|r| n::RoutineRow {
                name: r.name,
                addr: r.addr,
                size: r.size,
                element: r.element,
                why: r.why,
                calls: r.calls,
            })
            .collect(),
        dispatch: h
            .dispatch
            .into_iter()
            .map(|d| n::Dispatch {
                kind: d.kind,
                at: d.at,
                subject: d.subject,
                cases: d.cases,
            })
            .collect(),
        elements: h
            .elements
            .into_iter()
            .map(|e| n::Element {
                name: e.name,
                kind: e.kind,
                routines: e.routines,
                mapping: e
                    .mapping
                    .into_iter()
                    .map(|m| n::MapRow {
                        byte: m.byte,
                        role: m.role,
                        read_by: m.read_by,
                        written_by: m.written_by,
                    })
                    .collect(),
            })
            .collect(),
        machines: h
            .machines
            .into_iter()
            .map(|m| n::Machine {
                byte: m.byte,
                name: m.name,
                initial: m.initial,
                path: m.path,
                states: m
                    .states
                    .into_iter()
                    .map(|s| n::State {
                        value: s.value,
                        name: s.name,
                    })
                    .collect(),
                transitions: m
                    .transitions
                    .into_iter()
                    .map(|t| n::Transition {
                        from: t.from,
                        to: t.to,
                        at: t.at,
                        routine: t.routine,
                        guards: t.guards,
                        after: t.after,
                        counter: None,
                    })
                    .collect(),
            })
            .collect(),
        facts: h
            .facts
            .into_iter()
            .map(|f| n::Fact {
                value: value(&f.value),
                path: f.path,
                rule: f.rule,
                evidence: f.evidence.into_iter().map(ev).collect(),
            })
            .collect(),
        open: h
            .open
            .into_iter()
            .map(|o| n::Open {
                what: o.what,
                routine: o.routine,
                pc: o.pc,
                bytes: o.bytes,
                why: o.why,
            })
            .collect(),
        constants: h
            .constants
            .into_iter()
            .map(|c| n::Constant {
                routine: c.routine,
                pc: c.pc,
                insn: c.insn,
                value: c.value,
                what: c.what,
                then: c.then,
                cited: c.cited,
            })
            .collect(),
    }
}

/// The harvest of a romset's main CPU, in the NES harvest's types: what
/// `gp dataset rom` turns into rows (crate::rom_extract::harvest_rows).
pub(crate) fn harvest_of(set: &gp_arcade::Romset) -> n::Harvest {
    to_nes(z::harvest(&z::Board::mcr_main(&set.maincpu)))
}

/// The harvest of a romset's main CPU, and where its reading stopped.
fn harvest_romset_stops(bytes: &[u8]) -> Result<(n::Harvest, z::Stops)> {
    let set = gp_arcade::Romset::from_zip(bytes)?;
    let board = z::Board::mcr_main(&set.maincpu);
    let (h, st) = z::harvest_stops(&board);
    Ok((to_nes(h), st))
}

/// The Z80's stops in the 6502 report's shape (unread.json, the same
/// text): every routine read, each `jp (..)` with no target known a stop
/// (`pointer not known` where a path reached it, `site not reached` where
/// none did), with the RAM places its target was made from; `behind` is 0,
/// since the code past such a jump isn't in the code map to count.
pub(crate) fn unread(st: &z::Stops) -> n::unread::Unread {
    let hex = |a: u16| format!("{a:04X}");
    let stops: Vec<n::unread::Stop> = st
        .stops
        .iter()
        .map(|s| {
            let reason = if s.reached {
                n::unread::POINTER
            } else {
                n::unread::NOT_REACHED
            };
            n::unread::Stop {
                id: format!("{reason}@{}", hex(s.pc)),
                reason: reason.into(),
                pc: hex(s.pc),
                routine: format!("Z80_{}", hex(s.routine)),
                target: String::new(),
                bytes: s.places.clone(),
                behind: 0,
                examples: Vec::new(),
            }
        })
        .collect();
    let mut by_reason: std::collections::BTreeMap<String, (usize, usize)> = Default::default();
    for s in &stops {
        by_reason.entry(s.reason.clone()).or_default().0 += 1;
    }
    n::unread::Unread {
        schema: n::unread::SCHEMA.into(),
        prg_bytes: st.rom_bytes,
        code_bytes: st.code_bytes,
        routines: st.routines,
        read: st.routines,
        unread: 0,
        by_reason,
        stops,
        unread_routines: Vec::new(),
    }
}

pub(crate) fn run(a: &StaticArgs, rom: &Path, bytes: &[u8], t0: Instant) -> Result<Outcome> {
    if !a.clips.is_empty() {
        bail!(
            "gp model static on an arcade romset reads the code alone: --clips is the NES's (gp code listing and gp reach run a romset's clips)"
        );
    }
    let (h, stops) = harvest_romset_stops(bytes)?;
    let unread = a.unread.then(|| unread(&stops));
    let secs = t0.elapsed().as_secs_f64();
    let doc = serde_json::to_value(&h)?;
    if let Some(d) = &a.write {
        fs::create_dir_all(d)?;
        fs::write(
            d.join("static.json"),
            serde_json::to_string_pretty(&doc)? + "\n",
        )?;
        if let Some(u) = &unread {
            fs::write(
                d.join("unread.json"),
                serde_json::to_string_pretty(u)? + "\n",
            )?;
        }
    }
    if let Some(d) = &a.model {
        fs::create_dir_all(d)?;
        let facts: Vec<(String, Value)> = match &a.names {
            Some(k) => crate::static_grade::in_model_names(&h, k)?
                .into_iter()
                .map(|(p, v, _)| (p, v))
                .collect(),
            None => h
                .facts
                .iter()
                .map(|f| (f.path.clone(), f.value.clone()))
                .collect(),
        };
        fs::write(
            d.join(gp_model::MODEL_FILE),
            crate::model_static::model_text(&facts, "static")?,
        )?;
    }
    let grade = match &a.grade {
        Some(k) => Some(crate::static_grade::grade(&h, k)?),
        None => None,
    };
    let mut out = doc;
    if let Some(g) = &grade {
        out["grade"] = g.clone();
    }
    let mut text = crate::model_static::summary(&h, rom, secs, grade.as_ref());
    text += "\nRead from the main CPU's code alone (no runs).\n";
    if let Some(u) = &unread {
        out["unread"] = serde_json::to_value(u)?;
        text += &crate::model_static::unread_text(u);
        text += "  (the Z80 reader runs every routine of the code map; its stops are the jp (..) it couldn't follow, and the code past one isn't in the map to count)\n";
    }
    Ok(Outcome::ok(out, text))
}
