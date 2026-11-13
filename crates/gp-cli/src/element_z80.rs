//! `gp element inputs` on an arcade game (port wave 6): the instruction
//! that last wrote a byte by a clip's frame, and a writer slice back from
//! it on the main Z80, in place of gp-nes's 6502 slice.
//!
//! The clip runs from power-on with gp-arcade's instruction trace and
//! access log on over its own frames. On the frame the byte was last
//! written, the slice walks back from the write through the instructions
//! the main CPU ran before it in that frame: the registers the stored value
//! came from, then the instructions that set them, and so on, for at most
//! [`STEPS`] instructions or until nothing is wanted. Each memory read on
//! the way is an input (`value`, or `index` when it set an address
//! register a later read or the write went through), with its value and
//! whether the frame wrote it before (`frame`) or it came from an earlier
//! frame (`before`). The conditional jumps the CPU ran between the first
//! input and the write, and which way each went, are the branches. The
//! routine is the call or interrupt the write ran in, by address, or by
//! the game's labels when it has them.
//!
//! Only `--byte` and `--frame` are read for an arcade game: the measures,
//! slots and entities come from a lockstep against a model, which an
//! arcade game's clip score doesn't give this way yet.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, bail};
use gp_arcade::dasm::{self, Addr, Flow, Insn, Ptr, reg};
use gp_arcade::mcr::{Access, AccessKind, Trace};
use gp_arcade::{Controls, Romset, Tapper};
use gp_evidence::input_log::InputFrame;
use serde_json::{Value, json};

use crate::element::Names;

/// How far back a slice walks.
pub const STEPS: usize = 400;

fn controls(f: &InputFrame) -> Controls {
    Controls::from_bits(f.buttons_word() & 0xff)
}

/// The main CPU's instructions of one frame with their accesses.
struct Ran {
    trace: Vec<Trace>,
    accesses: Vec<Vec<Access>>,
}

fn run_frame(t: &mut Tapper, c: Controls) -> Ran {
    t.set_trace(true);
    t.set_accesses(true);
    t.step(c);
    let mut trace: Vec<Trace> = t.take_trace().into_iter().filter(|e| e.cpu == 0).collect();
    trace.sort_by_key(|e| e.time);
    let acc: Vec<Access> = t
        .take_accesses()
        .into_iter()
        .filter(|a| a.cpu == 0)
        .collect();
    t.set_trace(false);
    t.set_accesses(false);
    let mut accesses = Vec::with_capacity(trace.len());
    let mut j = 0;
    for (i, _) in trace.iter().enumerate() {
        let end = trace.get(i + 1).map(|n| n.time);
        let from = j;
        while j < acc.len() && end.is_none_or(|end| acc[j].time < end) {
            j += 1;
        }
        accesses.push(acc[from..j].to_vec());
    }
    Ran { trace, accesses }
}

fn address_regs(i: &Insn) -> u32 {
    match i.mem.map(|m| m.addr) {
        Some(Addr::Pair(b)) => b,
        Some(Addr::Index(Ptr::Ix, _)) => reg::IX,
        Some(Addr::Index(Ptr::Iy, _)) => reg::IY,
        Some(Addr::Index(Ptr::Hl, _)) => reg::HL,
        Some(Addr::Stack) => reg::SP,
        _ => 0,
    }
}

/// The slice of the last write to `addr` in a frame, if it wrote it.
fn slice(ran: &Ran, addr: u16, names: &Names) -> Option<Value> {
    let (w_at, write) = ran
        .accesses
        .iter()
        .enumerate()
        .flat_map(|(i, a)| a.iter().map(move |x| (i, x)))
        .rfind(|(_, a)| a.kind == AccessKind::Write && a.addr == addr)?;
    let insns: Vec<Option<Insn>> = ran
        .trace
        .iter()
        .map(|e| e.interrupt.is_none().then(|| dasm::decode(&e.bytes, e.pc)))
        .collect();
    // The routine the write ran in: the call stack, forward to the write.
    let mut stack: Vec<u16> = Vec::new();
    for (i, e) in ran.trace.iter().enumerate().take(w_at + 1) {
        let Some(insn) = &insns[i] else {
            stack.push(e.pc);
            continue;
        };
        if i == w_at {
            break;
        }
        let next = ran.trace.get(i + 1).map(|n| n.pc);
        let fall = e.pc.wrapping_add(u16::from(insn.len));
        match insn.flow {
            Flow::Call(t) | Flow::Rst(t) => stack.push(t),
            Flow::CallIf(t) if next.is_some_and(|n| n != fall) => stack.push(t),
            Flow::Ret => {
                stack.pop();
            }
            Flow::RetIf if next.is_some_and(|n| n != fall) => {
                stack.pop();
            }
            _ => {}
        }
    }
    let routine = stack.last().copied();
    let w_insn = insns[w_at].as_ref()?;
    let w = &ran.trace[w_at];
    // Walk back from the write.
    let mut wanted = w_insn.reads & !address_regs(w_insn) & !reg::F;
    let mut index = address_regs(w_insn) & !reg::SP;
    let mut reads: Vec<Value> = Vec::new();
    let mut steps: Vec<Value> = Vec::new();
    let mut first_input = w_at;
    let written_before: BTreeSet<u16> = ran.accesses[..w_at]
        .iter()
        .flatten()
        .filter(|a| a.kind == AccessKind::Write)
        .map(|a| a.addr)
        .collect();
    let mut seen_reads: BTreeMap<(u16, u16), bool> = BTreeMap::new();
    for i in (0..w_at).rev().take(STEPS) {
        if wanted == 0 && index == 0 {
            break;
        }
        let Some(insn) = &insns[i] else { continue };
        let hit_value = insn.writes & wanted & !reg::F != 0;
        let hit_index = insn.writes & index != 0;
        if !hit_value && !hit_index {
            continue;
        }
        let e = &ran.trace[i];
        first_input = i;
        steps.push(json!({"pc": e.pc, "text": insn.text}));
        let srcs = if insn.constant {
            0
        } else {
            insn.reads & !address_regs(insn) & !reg::F
        };
        if hit_value {
            wanted = (wanted & !insn.writes) | srcs;
        }
        if hit_index {
            index = (index & !insn.writes) | srcs;
        }
        let a_regs = address_regs(insn) & !reg::SP;
        index |= a_regs;
        for a in ran.accesses[i]
            .iter()
            .filter(|a| a.kind == AccessKind::Read)
        {
            let how = if hit_value { "value" } else { "index" };
            if seen_reads.insert((e.pc, a.addr), true).is_none() {
                reads.push(json!({
                    "pc": e.pc, "addr": a.addr, "name": names.ram(a.addr), "value": a.value,
                    "how": how,
                    "from": if written_before.contains(&a.addr) { "frame" } else { "before" },
                }));
            }
        }
    }
    reads.reverse();
    steps.reverse();
    // The conditional branches run from the first input to the write.
    let mut branches: BTreeMap<u16, (u64, u64)> = BTreeMap::new();
    #[allow(clippy::needless_range_loop)] // i reads the trace past itself
    for i in first_input..w_at {
        let Some(insn) = &insns[i] else { continue };
        if !insn.flow.conditional() {
            continue;
        }
        let e = &ran.trace[i];
        let fall = e.pc.wrapping_add(u16::from(insn.len));
        let Some(next) = ran.trace[i + 1..].iter().find(|n| n.interrupt.is_none()) else {
            continue;
        };
        let b = branches.entry(e.pc).or_default();
        if next.pc == fall {
            b.1 += 1;
        } else {
            b.0 += 1;
        }
    }
    let routine_name = routine.map_or_else(|| "?".to_owned(), |r| names.routine(r));
    let from: Vec<String> = reads
        .iter()
        .filter(|r| r["how"] == "value")
        .filter_map(|r| r["name"].as_str().map(str::to_owned))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let sentence = format!(
        "{routine_name} wrote {} at ${:04X} (`{}`){}",
        names.ram(addr),
        w.pc,
        w_insn.text,
        if from.is_empty() {
            " from a constant or registers set before the frame's slice".to_owned()
        } else {
            format!(" from {}", from.join(", "))
        }
    );
    Some(json!({
        "writer": {"pc": w.pc, "text": w_insn.text, "routine": routine, "routine_name": routine_name},
        "value": write.value,
        "reads": reads,
        "steps": steps,
        "branches": branches.iter().map(|(pc, (t, f))| json!({"pc": pc, "taken": t, "fell": f})).collect::<Vec<_>>(),
        "sentence": sentence,
    }))
}

/// The bytes' slices on clip frame `frame` (counted from the clip's start
/// at `start` of `inputs`).
pub(crate) fn inputs(
    set: &Romset,
    inputs: &[InputFrame],
    start: usize,
    frame: u64,
    addrs: &[u16],
    names: &Names,
) -> Result<Vec<Value>> {
    let of = start as u64 + frame;
    if of as usize > inputs.len() {
        bail!(
            "frame {frame} is past the clip's {} frames",
            inputs.len() - start
        );
    }
    let mut t = Tapper::power_on(set);
    for f in &inputs[..start] {
        t.step(controls(f));
    }
    let mut last: Vec<Option<(u64, Value)>> = vec![None; addrs.len()];
    for (n, input) in inputs.iter().enumerate().take(of as usize).skip(start) {
        let ran = run_frame(&mut t, controls(input));
        let f = n as u64 + 1;
        for (k, &a) in addrs.iter().enumerate() {
            if let Some(v) = slice(&ran, a, names) {
                last[k] = Some((f, v));
            }
        }
    }
    let mut out = Vec::new();
    for (k, &a) in addrs.iter().enumerate() {
        let value = t.peek(a);
        out.push(match &last[k] {
            Some((f, v)) => {
                let mut v = v.clone();
                v["addr"] = json!(format!("${a:04X}"));
                v["name"] = json!(names.ram(a));
                v["written"] = json!(true);
                v["frame_written"] = json!(f - start as u64);
                v["original_frame_written"] = json!(f);
                v["now"] = json!(value);
                v
            }
            None => json!({
                "addr": format!("${a:04X}"), "name": names.ram(a), "written": false, "now": value,
                "note": format!("not written from the clip's start (frame {start}) to frame {of}"),
            }),
        });
    }
    Ok(out)
}

/// The text of an arcade `gp element inputs`.
pub(crate) fn text(v: &Value) -> String {
    let mut out = format!(
        "{} ({}), frame {} (the original's {}){}{}, main Z80",
        v["clip"].as_str().unwrap_or(""),
        v["clip_name"].as_str().unwrap_or(""),
        v["frame"],
        v["original_frame"],
        v["measure"]
            .as_str()
            .map_or(String::new(), |m| format!(": {m}")),
        v["slot"]
            .as_u64()
            .map_or(String::new(), |s| format!(", slot {s}")),
    );
    for b in v["bytes"].as_array().into_iter().flatten() {
        out.push_str("\n\n");
        if b["written"] != true {
            out.push_str(&format!(
                "{} ({}): {}",
                b["name"].as_str().unwrap_or(""),
                b["addr"].as_str().unwrap_or(""),
                b["note"].as_str().unwrap_or("")
            ));
            continue;
        }
        out.push_str(b["sentence"].as_str().unwrap_or(""));
        out.push_str(&format!(
            ", on frame {}, value ${:02X}",
            b["frame_written"],
            b["value"].as_u64().unwrap_or(0)
        ));
        out.push_str("\n  read            value  how    from    by");
        for r in b["reads"].as_array().into_iter().flatten() {
            out.push_str(&format!(
                "\n  {:<15} ${:02X}    {:<6} {:<7} ${:04X}",
                r["name"].as_str().unwrap_or(""),
                r["value"].as_u64().unwrap_or(0),
                r["how"].as_str().unwrap_or(""),
                r["from"].as_str().unwrap_or(""),
                r["pc"].as_u64().unwrap_or(0)
            ));
        }
        let br: Vec<String> = b["branches"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|x| {
                format!(
                    "${:04X} taken {} fell {}",
                    x["pc"].as_u64().unwrap_or(0),
                    x["taken"],
                    x["fell"]
                )
            })
            .collect();
        if !br.is_empty() {
            out.push_str(&format!("\n  branches: {}", br.join(", ")));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The busy program writes e002 from A, which came from e002 itself
    /// through `rlca` and `xor`: the slice names the read of e002 at 000c
    /// as its value, read from the frame before on the first loop.
    #[test]
    fn slices_the_busy_programs_step() {
        let set = gp_arcade::testrom::busy();
        let frames = vec![InputFrame::default(); 4];
        let out = inputs(&set, &frames, 2, 1, &[0xe002, 0xe7ff], &Names::empty()).unwrap();
        let b = &out[0];
        assert_eq!(b["written"], true);
        assert_eq!(b["writer"]["pc"], 0x0014);
        let reads = b["reads"].as_array().unwrap();
        assert!(
            reads
                .iter()
                .any(|r| r["pc"] == 0x000c && r["addr"] == 0xe002 && r["how"] == "value"),
            "{b}"
        );
        assert_eq!(out[1]["written"], false);
        let t = text(
            &json!({"clip": "c", "clip_name": "n", "frame": 1, "original_frame": 3, "bytes": out}),
        );
        assert!(t.contains("wrote"), "{t}");
    }
}
