//! `gp model static` on an arcade romset: the main Z80's code read alone
//! by gp-arcade's harvest ([`gp_arcade::harvest`], the port of gp-nes's to
//! the Z80), in the
//! NES harvest's output (gp_nes::harvest::Harvest), so static.json, the
//! text, `--model` and `--grade` are the same shape.
//!
//! The romset version reads the code without running it: `--clips`, a
//! run's starts and `--unread` are the NES's still and are refused in
//! words, and the asset map isn't read (an MCR asset map places graphics in
//! the GFX chips, not in program ROM).

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

/// The harvest of a romset's main CPU.
pub(crate) fn harvest_romset(bytes: &[u8]) -> Result<n::Harvest> {
    let set = gp_arcade::Romset::from_zip(bytes)?;
    let board = z::Board::mcr_main(&set.maincpu);
    Ok(to_nes(z::harvest(&board)))
}

pub(crate) fn run(a: &StaticArgs, rom: &Path, bytes: &[u8], t0: Instant) -> Result<Outcome> {
    if !a.clips.is_empty() {
        bail!(
            "gp model static on an arcade romset reads the code alone: --clips is the NES's (gp code listing and gp reach run a romset's clips)"
        );
    }
    if a.unread {
        bail!("gp model static --unread is the NES's; on an arcade romset it isn't written yet");
    }
    let h = harvest_romset(bytes)?;
    let secs = t0.elapsed().as_secs_f64();
    let doc = serde_json::to_value(&h)?;
    if let Some(d) = &a.write {
        fs::create_dir_all(d)?;
        fs::write(
            d.join("static.json"),
            serde_json::to_string_pretty(&doc)? + "\n",
        )?;
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
    Ok(Outcome::ok(out, text))
}
