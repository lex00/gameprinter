//! `gp localize` on an arcade romset: steps 1 and 2 of docs/localize.md
//! over the main Z80, where crate::localize reads the NES's 6502.
//!
//! 1. One-way branches: each source is replayed from power-on with
//!    gp-arcade's instruction trace on, and each conditional jump, call
//!    and return (`jr cc`, `jp cc`, `call cc`, `ret cc`, `djnz`) is counted
//!    the way it went: where the main CPU's next instruction starts, as
//!    crate::covered counts it. One an interrupt cut into counts no way.
//! 2. Guards: the frame each one-way jump first ran is replayed with the
//!    trace and the access log, and the latest instruction before the jump
//!    that set the flag its condition reads (Z, C, S or P/V; B for `djnz`)
//!    is the setter. The walk back from the setter is
//!    crate::element_z80::walk_back, the one `gp element inputs` walks back
//!    from a write with: the RAM bytes the compared value came from, with
//!    an interrupt's handler passed over when the jump didn't run in it.
//!    When the setter compares one value with a constant (or a register
//!    the frame held), the values that would send the jump the other way
//!    are worked out, as the 6502's are.
//!
//! 3. Places: for a model in the board's own units (its state mapping's
//!    `board_units`, feature `platform.board_units`), each guard whose
//!    compared value came from the level grid (`[cells]`) is placed as on
//!    the NES: the cells that held a value that sends it the other way,
//!    and where its cell sat from the player each time it ran, a cell
//!    being the model's `tile_size` of the player's units ([`Board`]).
//!    Without the marker the step says why in one sentence
//!    ([`UNITS_STOP`]).
//!
//! Step 4 (probes sent to a place) isn't run on arcade: see [`PROBE_STOP`].

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;

use anyhow::{Result, bail};
use gp_arcade::dasm::{self, Addr, Alu, Flow, Insn, Op, reg};
use gp_arcade::mcr::AccessKind;
use gp_arcade::{Controls, Romset, Tapper};
use serde_json::{Value, json};

use crate::Outcome;
use crate::element::Names;
use crate::element_z80::{Ran, Walk, run_frame, walk_back};
use crate::localize::{LocalizeArgs, Src};
use crate::observe::{Grid, LevelKey};
use gp_evidence::Paths;

/// Why the places step stops on an arcade model whose positions are the
/// RAM's bytes: a place meets a grid cell from the player, so the grid and
/// the player's place must be in one unit, which only a model in the
/// board's own units promises.
pub(crate) const UNITS_STOP: &str = "no cell is placed: the places step meets a grid cell from the player in the board's own units, which the state mapping says with board_units = true (feature platform.board_units), and this game's doesn't";

/// Why step 4 stops on an arcade game: the probes play the player to a
/// place with Go-Explore on the NES's save states, which gp-arcade's
/// aren't wired to.
pub(crate) const PROBE_STOP: &str = "no probe is sent: a probe plays the player to a place from the NES emulator's save states, and gp-arcade's aren't wired to it";

/// The first byte of the MCR main board's RAM (work RAM at E000, then
/// sprite, palette and video RAM): reads below it are the program's ROM.
const RAM: u16 = 0xe000;

/// A flag a condition reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Flag {
    Z,
    C,
    S,
    Pv,
    /// `djnz`: B, which the jump counts down itself.
    B,
}

impl Flag {
    fn name(self) -> &'static str {
        match self {
            Flag::Z => "Z",
            Flag::C => "C",
            Flag::S => "S",
            Flag::Pv => "P/V",
            Flag::B => "B",
        }
    }
}

/// The flag a conditional's text reads and the value that sends it to
/// its target (taken): `jr nc,$0014` is (C, false).
pub(crate) fn condition(i: &Insn) -> Option<(Flag, bool)> {
    if matches!(i.flow, Flow::Djnz(_)) {
        return Some((Flag::B, true));
    }
    if !i.flow.conditional() {
        return None;
    }
    let cc = i
        .text
        .split_whitespace()
        .nth(1)?
        .split(',')
        .next()
        .unwrap_or("");
    Some(match cc {
        "nz" => (Flag::Z, false),
        "z" => (Flag::Z, true),
        "nc" => (Flag::C, false),
        "c" => (Flag::C, true),
        "po" => (Flag::Pv, false),
        "pe" => (Flag::Pv, true),
        "p" => (Flag::S, false),
        "m" => (Flag::S, true),
        _ => return None,
    })
}

/// Whether an instruction leaves `f` as its own result (as the Z80
/// manual's flag tables have it), so it, and nothing before it, set the
/// flag a later condition reads.
pub(crate) fn sets(i: &Insn, f: Flag) -> bool {
    if f == Flag::B {
        return false;
    }
    if i.writes & reg::F == 0 {
        return false;
    }
    let m = i.text.split_whitespace().next().unwrap_or("");
    let wide = |t: &str| {
        let d = i.text.split_whitespace().nth(1).unwrap_or("");
        d.starts_with(t)
    };
    match i.op {
        // add hl,rr (and ix, iy) sets only C of these; adc and sbc of a
        // pair set them all.
        Op::Alu(Alu::Add) if wide("hl,") || wide("ix,") || wide("iy,") => f == Flag::C,
        Op::Alu(Alu::Other) => match m {
            "scf" | "ccf" => f == Flag::C,
            "cpl" => false,
            _ => true,
        },
        Op::Alu(_) => true,
        Op::Count => f != Flag::C,
        Op::Shift => match m {
            "rlca" | "rrca" | "rla" | "rra" => f == Flag::C,
            "rld" | "rrd" => f != Flag::C,
            _ => true,
        },
        Op::Bit => f == Flag::Z,
        Op::Block => match m {
            "ldi" | "ldd" | "ldir" | "lddr" => f == Flag::Pv,
            _ => f != Flag::C,
        },
        // pop af, ex af,af', ld a,i, ld a,r, in r,(c).
        Op::Load => f != Flag::C || m == "pop" || m == "ex",
        Op::Other => true,
    }
}

/// The flags an instruction leaves for `v` in the value it tests, the
/// other operand `o` as the frame held it: (Z, C, S, P/V), each None when
/// it isn't worked out. Only the tests games use for a choice are: the
/// eight-bit arithmetic and logic, inc and dec, bit, and the rotates'
/// carry.
pub(crate) fn flags_for(i: &Insn, v: u8, o: u8) -> Option<[Option<bool>; 4]> {
    let m = i.text.split_whitespace().next().unwrap_or("");
    let parity = |r: u8| Some(r.count_ones().is_multiple_of(2));
    let zs = |r: u8| (Some(r == 0), Some(r & 0x80 != 0));
    Some(match (i.op, m) {
        (Op::Alu(Alu::Cp | Alu::Sub), _) => {
            let d = v.wrapping_sub(o);
            let (z, s) = zs(d);
            [z, Some(v < o), s, Some((v ^ o) & (v ^ d) & 0x80 != 0)]
        }
        (Op::Alu(Alu::Add), _) if !i.text.contains("a,") => return None,
        (Op::Alu(Alu::Add), _) => {
            let d = v.wrapping_add(o);
            let (z, s) = zs(d);
            [
                z,
                Some(u16::from(v) + u16::from(o) > 0xff),
                s,
                Some(!(v ^ o) & (v ^ d) & 0x80 != 0),
            ]
        }
        (Op::Alu(Alu::And), _) => {
            let (z, s) = zs(v & o);
            [z, Some(false), s, parity(v & o)]
        }
        (Op::Alu(Alu::Or), _) => {
            let (z, s) = zs(v | o);
            [z, Some(false), s, parity(v | o)]
        }
        (Op::Alu(Alu::Xor), _) => {
            let (z, s) = zs(v ^ o);
            [z, Some(false), s, parity(v ^ o)]
        }
        (Op::Count, "inc") => {
            let (z, s) = zs(v.wrapping_add(1));
            [z, None, s, Some(v == 0x7f)]
        }
        (Op::Count, "dec") => {
            let (z, s) = zs(v.wrapping_sub(1));
            [z, None, s, Some(v == 0x80)]
        }
        (Op::Bit, "bit") => {
            let b: u32 = i
                .text
                .split_whitespace()
                .nth(1)?
                .split(',')
                .next()?
                .parse()
                .ok()?;
            [Some(v >> b & 1 == 0), None, None, None]
        }
        (Op::Shift, "rlca" | "rla" | "rlc" | "rl" | "sla" | "sll") => {
            [None, Some(v & 0x80 != 0), None, None]
        }
        (Op::Shift, "rrca" | "rra" | "rrc" | "rr" | "sra" | "srl") => {
            [None, Some(v & 1 != 0), None, None]
        }
        _ => return None,
    })
}

/// Whether a conditional goes to its target for those flags.
fn taken_by(flag: Flag, when: bool, f: [Option<bool>; 4]) -> Option<bool> {
    let k = match flag {
        Flag::Z => f[0],
        Flag::C => f[1],
        Flag::S => f[2],
        Flag::Pv => f[3],
        Flag::B => return None,
    }?;
    Some(k == when)
}

/// What a value lives in: a register (by its [`reg`] bit) or a memory
/// byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Loc {
    Reg(u32),
    Mem,
}

/// The register an eight-bit operand names, as text.
fn reg_of(name: &str) -> Option<u32> {
    Some(match name {
        "a" => reg::A,
        "b" => reg::B,
        "c" => reg::C,
        "d" => reg::D,
        "e" => reg::E,
        "h" => reg::H,
        "l" => reg::L,
        "ixh" => reg::IXH,
        "ixl" => reg::IXL,
        "iyh" => reg::IYH,
        "iyl" => reg::IYL,
        _ => return None,
    })
}

/// An eight-bit register's value before the instruction `e` ran.
fn value_of(e: &gp_arcade::mcr::Trace, r: u32) -> u8 {
    let hi = |w: u16| (w >> 8) as u8;
    let lo = |w: u16| w as u8;
    match r {
        reg::A => hi(e.af),
        reg::B => hi(e.bc),
        reg::C => lo(e.bc),
        reg::D => hi(e.de),
        reg::E => lo(e.de),
        reg::H => hi(e.hl),
        reg::L => lo(e.hl),
        reg::IXH => hi(e.ix),
        reg::IXL => lo(e.ix),
        reg::IYH => hi(e.iy),
        reg::IYL => lo(e.iy),
        _ => 0,
    }
}

/// The last operand of a text, as an immediate byte (`cp $1c`).
fn immediate(text: &str) -> Option<u8> {
    let last = text.rsplit([' ', ',']).next()?;
    let h = last.strip_prefix('$')?;
    (h.len() == 2).then(|| u8::from_str_radix(h, 16).ok())?
}

/// The value a setter tests and the other operand: (where the tested one
/// lives, its value, the other's value, the constant if the other is one).
fn operands(
    i: &Insn,
    e: &gp_arcade::mcr::Trace,
    mem: Option<u8>,
) -> Option<(Loc, u8, u8, Option<u8>)> {
    let arg = i.text.split_whitespace().nth(1).unwrap_or("");
    let last = arg.rsplit(',').next().unwrap_or("");
    let a = value_of(e, reg::A);
    match i.op {
        Op::Alu(_) => {
            if let Some(k) = immediate(&i.text) {
                return Some((Loc::Reg(reg::A), a, k, Some(k)));
            }
            if last.starts_with('(') {
                return Some((Loc::Reg(reg::A), a, mem?, None));
            }
            let r = reg_of(last)?;
            Some((Loc::Reg(reg::A), a, value_of(e, r), None))
        }
        Op::Count | Op::Shift | Op::Bit => {
            if matches!(i.text.as_str(), "rlca" | "rrca" | "rla" | "rra") {
                return Some((Loc::Reg(reg::A), a, 0, None));
            }
            if last.starts_with('(') {
                return Some((Loc::Mem, mem?, 0, None));
            }
            let r = reg_of(last)?;
            Some((Loc::Reg(r), value_of(e, r), 0, None))
        }
        _ => None,
    }
}

/// A one-way jump's guard on the main Z80.
#[derive(Debug, Clone)]
pub(crate) struct Guard {
    /// The way no run went: true = to the target.
    pub(crate) want_taken: bool,
    pub(crate) flag: Flag,
    /// The instruction that set the flag, and its text.
    pub(crate) setter: u16,
    pub(crate) setter_text: String,
    pub(crate) constant: Option<u8>,
    /// The other operand when it was a register or memory byte: its value.
    pub(crate) against: Option<u8>,
    /// The RAM byte the tested value came from, and its value then.
    pub(crate) source: Option<(u16, u8)>,
    /// Through an address register (`(hl)`, `(ix+d)`): a table or a pointer.
    pub(crate) indirect: bool,
    /// The tested value when the flag was set.
    pub(crate) value: u8,
    /// RAM bytes the tested value was copied through, latest first.
    pub(crate) chain: Vec<u16>,
    /// Values of the tested value that send the jump the other way; None
    /// when the setter isn't one this works out.
    pub(crate) flips: Option<Vec<u8>>,
    /// The RAM bytes the walk back read, oldest first, with their values.
    pub(crate) reads: Vec<(u16, u8)>,
}

/// Each step's interrupt context: the trace index of the interrupt entry
/// whose handler it runs in, or None in the code the frame interrupted.
/// A handler ends with the return that runs at the entry's stack pointer.
fn contexts(ran: &Ran, insns: &[Option<Insn>]) -> Vec<Option<usize>> {
    let mut out = Vec::with_capacity(ran.trace.len());
    let mut stack: Vec<(usize, u16)> = Vec::new();
    for (i, e) in ran.trace.iter().enumerate() {
        if e.interrupt.is_some() {
            stack.push((i, e.sp));
            out.push(Some(i));
            continue;
        }
        out.push(stack.last().map(|s| s.0));
        let Some(insn) = &insns[i] else { continue };
        let ret = match insn.flow {
            Flow::Ret => true,
            Flow::RetIf => {
                let fall = e.pc.wrapping_add(u16::from(insn.len));
                ran.trace
                    .get(i + 1)
                    .is_some_and(|n| n.interrupt.is_none() && n.pc != fall)
            }
            _ => false,
        };
        if ret && stack.last().is_some_and(|s| s.1 == e.sp) {
            stack.pop();
        }
    }
    out
}

/// The RAM bytes a value was copied through before step `at`: back
/// through register copies, loads, and the stores in the frame that
/// filled the bytes loaded. The first load is the value's source.
fn copy_chain(
    ran: &Ran,
    insns: &[Option<Insn>],
    keep: &dyn Fn(usize) -> bool,
    at: usize,
    reg0: u32,
) -> (Vec<u16>, Option<(u16, u8, bool)>) {
    let mut out = Vec::new();
    let mut source = None;
    let (mut j, mut r) = (at, reg0);
    for _ in 0..16 {
        let Some(k) = (0..j)
            .rev()
            .filter(|&k| keep(k))
            .find(|&k| insns[k].as_ref().is_some_and(|i| i.writes & r != 0))
        else {
            break;
        };
        let insn = insns[k].as_ref().expect("kept");
        if insn.op != Op::Load {
            break;
        }
        if let Some(m) = insn
            .mem
            .filter(|m| m.read && !m.word && m.addr != Addr::Stack)
        {
            let Some(rd) = ran.accesses[k]
                .iter()
                .find(|a| a.kind == AccessKind::Read && a.addr >= RAM)
            else {
                break;
            };
            out.push(rd.addr);
            source.get_or_insert((rd.addr, rd.value, !matches!(m.addr, Addr::Abs(_))));
            // The store in the frame that filled it.
            let Some(w) = (0..k).rev().filter(|&w| keep(w)).find(|&w| {
                ran.accesses[w]
                    .iter()
                    .any(|a| a.kind == AccessKind::Write && a.addr == rd.addr)
            }) else {
                break;
            };
            let Some(wi) = insns[w].as_ref().filter(|i| i.op == Op::Load) else {
                break;
            };
            let from = wi.reads & !crate::draws_find_z80::address_regs(wi) & !reg::F;
            if from == 0 || from.count_ones() > 1 {
                break;
            }
            (j, r) = (w, from);
            continue;
        }
        if insn.mem.is_some() || insn.constant || insn.io.is_some() {
            break;
        }
        // A register copy: ld a,b and the like.
        let from = insn.reads & !reg::F;
        if from == 0 || from.count_ones() > 1 {
            break;
        }
        (j, r) = (k, from);
    }
    (out, source)
}

/// The guard of the conditional at step `bi` of a frame, for the way no
/// run went.
pub(crate) fn guard_at(
    ran: &Ran,
    insns: &[Option<Insn>],
    ctx: &[Option<usize>],
    bi: usize,
    want_taken: bool,
    names: &Names,
) -> Option<Guard> {
    let jump = insns[bi].as_ref()?;
    let (flag, when) = condition(jump)?;
    let here = ctx[bi];
    let keep = |i: usize| ctx[i] == here;
    let ram_reads = |w: &Walk| -> Vec<(u16, u8)> {
        let mut v: Vec<(u16, u8)> = Vec::new();
        for r in &w.reads {
            let (Some(a), Some(x)) = (r["addr"].as_u64(), r["value"].as_u64()) else {
                continue;
            };
            let p = (a as u16, x as u8);
            if p.0 >= RAM && !v.contains(&p) {
                v.push(p);
            }
        }
        v
    };
    if flag == Flag::B {
        // djnz counts B down and jumps while it isn't zero.
        let e = &ran.trace[bi];
        let b = value_of(e, reg::B);
        let w = walk_back(ran, insns, bi, reg::B, 0, names, &keep);
        let (chain, source) = copy_chain(ran, insns, &keep, bi, reg::B);
        let flips = (0..=255u8)
            .filter(|&v| (v.wrapping_sub(1) != 0) == want_taken)
            .collect();
        return Some(Guard {
            want_taken,
            flag,
            setter: e.pc,
            setter_text: jump.text.clone(),
            constant: None,
            against: None,
            source: source.map(|(a, v, _)| (a, v)),
            indirect: source.is_some_and(|s| s.2),
            value: b,
            chain,
            flips: Some(flips),
            reads: ram_reads(&w),
        });
    }
    let si = (0..bi)
        .rev()
        .filter(|&i| keep(i))
        .find(|&i| insns[i].as_ref().is_some_and(|x| sets(x, flag)))?;
    let s = insns[si].as_ref()?;
    let e = &ran.trace[si];
    let mem_read = ran.accesses[si]
        .iter()
        .find(|a| a.kind == AccessKind::Read && s.mem.is_some_and(|m| m.addr != Addr::Stack));
    let ops = operands(s, e, mem_read.map(|a| a.value));
    let wanted = s.reads & !crate::draws_find_z80::address_regs(s) & !reg::F;
    let index = crate::draws_find_z80::address_regs(s) & !reg::SP;
    let w = walk_back(ran, insns, si, wanted, index, names, &keep);
    let mut reads = ram_reads(&w);
    if let Some(m) = mem_read.filter(|a| a.addr >= RAM) {
        reads.push((m.addr, m.value));
    }
    let (loc, value, other, constant) = match ops {
        Some(o) => o,
        None => {
            return Some(Guard {
                want_taken,
                flag,
                setter: e.pc,
                setter_text: s.text.clone(),
                constant: None,
                against: None,
                source: None,
                indirect: false,
                value: 0,
                chain: Vec::new(),
                flips: None,
                reads,
            });
        }
    };
    let (chain, source, indirect) = match loc {
        Loc::Mem => {
            let m = mem_read?;
            (
                vec![m.addr],
                (m.addr >= RAM).then_some((m.addr, m.value)),
                s.mem.is_some_and(|m| !matches!(m.addr, Addr::Abs(_))),
            )
        }
        Loc::Reg(r) => {
            let (c, src) = copy_chain(ran, insns, &keep, si, r);
            (c, src.map(|(a, v, _)| (a, v)), src.is_some_and(|x| x.2))
        }
    };
    // A setter that tests a register against itself (`or a`, `and a`).
    let same = constant.is_none()
        && matches!(loc, Loc::Reg(_))
        && {
            let arg = s.text.split_whitespace().nth(1).unwrap_or("");
            arg == "a" || arg.ends_with(",a") && !arg.starts_with('(')
        }
        && s.op != Op::Bit;
    let flips = flags_for(s, value, other).map(|_| {
        (0..=255u8)
            .filter(|&v| {
                let o = if same { v } else { other };
                flags_for(s, v, o).and_then(|f| taken_by(flag, when, f)) == Some(want_taken)
            })
            .collect()
    });
    Some(Guard {
        want_taken,
        flag,
        setter: e.pc,
        setter_text: s.text.clone(),
        constant,
        against: (constant.is_none() && !same && !matches!(s.op, Op::Count | Op::Shift | Op::Bit))
            .then_some(other),
        source,
        indirect,
        value,
        chain,
        flips,
        reads,
    })
}

/// What the places step reads on a board: the level grid, a cell being
/// the model's `tile_size` of the player's units, and the player's place
/// as the state mapping reads it, in the board's own units.
pub(crate) struct Board {
    pub(crate) grid: Grid,
    x: gp_interp::mapping::Field,
    y: gp_interp::mapping::Field,
}

impl Board {
    /// The places step's reading of the game in `dir`, or why it can't
    /// run (one sentence): no `board_units`, no `[cells]`, no player.
    /// `level` is the level looked in: a model level by that name gives
    /// the cell, or else the model's first level.
    pub(crate) fn load(dir: &Path, level: &str) -> Result<std::result::Result<Board, String>> {
        use gp_interp::mapping::{MAPPING_FILE, Mapping};
        let file = dir.join(MAPPING_FILE);
        let Ok(text) = fs::read_to_string(&file) else {
            return Ok(Err(format!(
                "no cell is placed: {} isn't there",
                file.display()
            )));
        };
        let m = Mapping::parse(&text).map_err(|e| anyhow::anyhow!("{}: {e}", file.display()))?;
        if !m.board_units {
            return Ok(Err(UNITS_STOP.to_owned()));
        }
        let Some(mut grid) = Grid::load(dir) else {
            return Ok(Err(format!(
                "no cell is placed: {} has no [cells], the level grid a guard's value is read from",
                file.display()
            )));
        };
        let model = gp_model::Model::parse(&fs::read_to_string(dir.join(gp_model::MODEL_FILE))?)
            .map_err(|e| anyhow::anyhow!("{}: {e}", dir.join(gp_model::MODEL_FILE).display()))?;
        let Some(l) = model
            .levels
            .get(level)
            .or_else(|| model.levels.values().next())
        else {
            return Ok(Err(
                "no cell is placed: the model has no level to take a cell's size from".to_owned(),
            ));
        };
        grid.cell = i64::from(l.core.tile_size.max(1));
        let field = |p: &str| m.fields.iter().find(|f| f.path == p).cloned();
        let (Some(x), Some(y)) = (field("entities.player[0].x"), field("entities.player[0].y"))
        else {
            return Ok(Err(format!(
                "no cell is placed: {} maps no entities.player[0].x and .y, the place a cell is met from",
                file.display()
            )));
        };
        Ok(Ok(Board { grid, x, y }))
    }

    /// The player's place now, in the board's units, with `mem` (64 KB,
    /// by the CPU's addresses) filled from the board's RAM.
    fn player(&self, t: &Tapper, mem: &mut [u8]) -> (i64, i64) {
        for a in RAM..=0xffff {
            mem[usize::from(a)] = t.peek(a);
        }
        let n = |f: &gp_interp::mapping::Field| match f.read_ram(mem) {
            gp_runtime::Value::Fixed(raw) => raw >> 16,
            gp_runtime::Value::Int(n) => n,
            _ => 0,
        };
        (n(&self.x), n(&self.y))
    }

    /// A grid byte's cell: its left and top in the board's units. The
    /// board doesn't scroll, so a page's columns follow the one before's.
    pub(crate) fn at(&self, addr: u16) -> Option<(i64, i64)> {
        let (page, row, col) = self.grid.place(addr)?;
        Some((
            (page * self.grid.cols + col) as i64 * self.grid.cell,
            self.grid.row_y(row),
        ))
    }
}

/// The grid byte a guard's compared value came from: its source if that
/// is on the grid, else a byte it was copied through, else a grid byte the
/// walk back read holding the same value.
fn grid_source(g: &Guard, grid: &Grid) -> Option<u16> {
    if let Some((ad, _)) = g.source
        && grid.place(ad).is_some()
    {
        return Some(ad);
    }
    if let Some(ad) = g.chain.iter().find(|ad| grid.place(**ad).is_some()) {
        return Some(*ad);
    }
    g.reads
        .iter()
        .find(|(ad, v)| *v == g.value && grid.place(*ad).is_some())
        .map(|(ad, _)| *ad)
}

/// A one-way conditional placed on the level grid.
pub(crate) struct Placed {
    pub(crate) grid_source: u16,
    /// Cells that held a value that sends it the other way.
    pub(crate) cells: usize,
    /// Fewer such values and cells, a more specific lead.
    pub(crate) score: f64,
    /// The lead with its `grid_source`, `meets` and `cells`.
    pub(crate) json: Value,
}

/// Step 3: each guard on a grid byte placed, as the NES's are
/// (crate::localize::run): the grid's cells that held a value that sends
/// the jump the other way while the level was played (every 8th frame),
/// and where its cell sat from the player each time it ran, the nearer
/// half of the samples' medians.
pub(crate) fn places(
    set: &Romset,
    srcs: &[Src],
    level: &LevelKey,
    name: &str,
    one_way: &[(u16, Seen)],
    guards: &HashMap<u16, Guard>,
    board: &Board,
) -> BTreeMap<u16, Placed> {
    let gr = &board.grid;
    let wanted: Vec<(u16, &Seen, &Guard, u16, &[u8])> = one_way
        .iter()
        .filter_map(|(pc, s)| {
            let g = guards.get(pc)?;
            let f = g
                .flips
                .as_deref()
                .filter(|f| !f.is_empty() && f.len() < 256)?;
            Some((*pc, s, g, grid_source(g, gr)?, f))
        })
        .collect();
    let mut out = BTreeMap::new();
    if wanted.is_empty() {
        return out;
    }
    let mut mem = vec![0u8; 0x10000];
    // The grid's values as the level was played: (address) -> {value ->
    // (source, frame)}.
    let mut grid_seen: BTreeMap<u16, BTreeMap<u8, (usize, usize)>> = BTreeMap::new();
    for (si, s) in srcs.iter().enumerate() {
        let mut t = Tapper::power_on(set);
        let mut n = 0usize;
        for (fi, f) in s.frames.iter().enumerate() {
            t.step(controls(f));
            if level_of(&t, level, &mut mem).is_some_and(|l| l == name) {
                if n.is_multiple_of(8) {
                    for &base in &gr.pages {
                        for i in 0..gr.cols * gr.rows {
                            let ad = base.wrapping_add(i as u16);
                            grid_seen
                                .entry(ad)
                                .or_default()
                                .entry(t.peek(ad))
                                .or_insert((si, fi));
                        }
                    }
                }
                n += 1;
            }
        }
    }
    for (pc, s, g, source, flips) in wanted {
        let mut cells: Vec<Value> = Vec::new();
        for (ad, vals) in &grid_seen {
            for (v, (si, fi)) in vals {
                if flips.contains(v)
                    && let Some((x, y)) = board.at(*ad)
                {
                    cells.push(
                        json!({"addr": format!("{ad:04X}"), "value": v, "level_x": x,
                        "y": y, "first": [srcs[*si].name, fi]}),
                    );
                }
            }
        }
        // How the guard met its cell each time it ran: where the cell sat
        // from the player, and the player's move up or down.
        let mut offs: Vec<(i64, i64, i64)> = Vec::new();
        let mut need: BTreeMap<usize, std::collections::BTreeSet<usize>> = BTreeMap::new();
        for &(si, fi) in &s.at {
            need.entry(si).or_default().insert(fi);
        }
        for (si, frames) in &need {
            let last = *frames.iter().max().expect("a frame");
            let mut t = Tapper::power_on(set);
            let mut ys: Vec<i64> = Vec::new();
            for (k, f) in srcs[*si].frames[..=last].iter().enumerate() {
                if !frames.contains(&k) {
                    t.step(controls(f));
                    ys.push(board.player(&t, &mut mem).1);
                    continue;
                }
                let ran = run_frame(&mut t, controls(f));
                let (px, py) = board.player(&t, &mut mem);
                ys.push(py);
                let vy = py - ys.get(k.saturating_sub(2)).copied().unwrap_or(py);
                let insns: Vec<Option<Insn>> = ran
                    .trace
                    .iter()
                    .map(|e| e.interrupt.is_none().then(|| dasm::decode(&e.bytes, e.pc)))
                    .collect();
                let ctx = contexts(&ran, &insns);
                let mut best: Option<(i64, i64)> = None;
                for (bi, e) in ran.trace.iter().enumerate() {
                    if e.pc != pc || e.interrupt.is_some() {
                        continue;
                    }
                    let Some(gx) = guard_at(&ran, &insns, &ctx, bi, g.want_taken, &Names::empty())
                    else {
                        continue;
                    };
                    let Some((cx, cy)) = grid_source(&gx, gr).and_then(|ad| board.at(ad)) else {
                        continue;
                    };
                    let o = (cx - px, cy - py);
                    if best.is_none_or(|b| o.0.abs() + o.1.abs() < b.0.abs() + b.1.abs()) {
                        best = Some(o);
                    }
                }
                if let Some(o) = best {
                    offs.push((o.0, o.1, vy));
                }
            }
        }
        offs.sort_by_key(|o| o.0.abs() + o.1.abs());
        offs.truncate(offs.len().div_ceil(2).max(1));
        let med = |mut v: Vec<i64>| {
            v.sort_unstable();
            v.get(v.len() / 2).copied().unwrap_or(0)
        };
        let meets = json!({
            "cell_from_player_x": med(offs.iter().map(|o| o.0).collect()),
            "cell_from_player_y": med(offs.iter().map(|o| o.1).collect()),
            "player_moving_y": med(offs.iter().map(|o| o.2).collect()),
            "samples": offs.len(),
        });
        let score = 1.0 / (flips.len() as f64 * cells.len().max(1) as f64);
        out.insert(
            pc,
            Placed {
                grid_source: source,
                cells: cells.len(),
                score,
                json: json!({
                    "branch": format!("{pc:04X}"),
                    "went": if s.taken > 0 { "only taken" } else { "only fell through" },
                    "grid_source": format!("{source:04X}"),
                    "meets": meets,
                    "cells": cells,
                }),
            },
        );
    }
    out
}

/// One conditional's record over the runs.
#[derive(Default, Clone)]
pub(crate) struct Seen {
    pub(crate) taken: u64,
    pub(crate) fell: u64,
    /// (source, frame) of the first runs, up to a few.
    pub(crate) at: Vec<(usize, usize)>,
}

/// What step 1 found: each conditional's ways, the code that ran, and the
/// frames counted in the level.
pub(crate) struct Scan {
    pub(crate) seen: HashMap<u16, Seen>,
    pub(crate) exec: Vec<u64>,
    pub(crate) frames_in: usize,
}

fn controls(f: &gp_evidence::input_log::InputFrame) -> Controls {
    Controls::from_bits(f.buttons_word() & 0xff)
}

/// The level the board's RAM is in: the work RAM and the key's bytes, by
/// the CPU's addresses.
fn level_of(t: &Tapper, level: &LevelKey, mem: &mut [u8]) -> Option<String> {
    if level.ram.is_empty() && level.when.is_none() {
        return level.of(mem);
    }
    for a in RAM..RAM + 0x800 {
        mem[usize::from(a)] = t.peek(a);
    }
    for &a in &level.ram {
        mem[usize::from(a)] = t.peek(a);
    }
    level.of(mem)
}

/// Step 1: every source replayed with the main CPU's trace on, each
/// conditional's way counted on the frames in `name`.
pub(crate) fn scan(set: &Romset, srcs: &[Src], level: &LevelKey, name: &str) -> Scan {
    let mut seen: HashMap<u16, Seen> = HashMap::new();
    let mut exec = vec![0u64; 1024];
    let mut frames_in = 0;
    // Each address's flow and length, decoded once (the program is ROM;
    // an instruction whose bytes differ is decoded again).
    let mut flows: Vec<Option<([u8; 4], Flow, u8)>> = vec![None; 0x10000];
    let mut mem = vec![0u8; 0x10000];
    for (si, s) in srcs.iter().enumerate() {
        let mut t = Tapper::power_on(set);
        let mut in_level = level_of(&t, level, &mut mem).is_some_and(|l| l == name);
        for (fi, f) in s.frames.iter().enumerate() {
            if !in_level {
                t.step(controls(f));
                in_level = level_of(&t, level, &mut mem).is_some_and(|l| l == name);
                continue;
            }
            let trace = crate::covered::frame_trace(&mut t, controls(f));
            frames_in += 1;
            let mut prev: Option<(u16, Flow, u16)> = None;
            for e in trace.iter().filter(|e| e.cpu == 0) {
                if e.interrupt.is_some() {
                    prev = None;
                    continue;
                }
                exec[usize::from(e.pc >> 6)] |= 1 << (e.pc & 63);
                if let Some((pc, flow, fall)) = prev
                    && flow.conditional()
                {
                    let en = seen.entry(pc).or_default();
                    if e.pc == fall {
                        en.fell += 1;
                    } else {
                        en.taken += 1;
                    }
                    if en.at.len() < 12 && en.at.last().is_none_or(|l| l.1 + 30 < fi || l.0 != si) {
                        en.at.push((si, fi));
                    }
                }
                let slot = &mut flows[usize::from(e.pc)];
                let (flow, len) = match slot {
                    Some((b, fl, l)) if *b == e.bytes => (*fl, *l),
                    _ => {
                        let i = dasm::decode(&e.bytes, e.pc);
                        *slot = Some((e.bytes, i.flow, i.len));
                        (i.flow, i.len)
                    }
                };
                prev = Some((e.pc, flow, e.pc.wrapping_add(u16::from(len))));
            }
            in_level = level_of(&t, level, &mut mem).is_some_and(|l| l == name);
        }
    }
    Scan {
        seen,
        exec,
        frames_in,
    }
}

/// The one-way conditionals of a scan, by address.
pub(crate) fn one_way(scan: &Scan) -> Vec<(u16, Seen)> {
    let mut v: Vec<(u16, Seen)> = scan
        .seen
        .iter()
        .filter(|(_, s)| (s.taken == 0) != (s.fell == 0))
        .map(|(p, s)| (*p, s.clone()))
        .collect();
    v.sort_by_key(|(p, _)| *p);
    v
}

/// Step 2: each one-way conditional's guard, read on the first frame it
/// ran (one replay a source, the frames in order).
pub(crate) fn guards(
    set: &Romset,
    srcs: &[Src],
    one_way: &[(u16, Seen)],
    names: &Names,
) -> HashMap<u16, Guard> {
    let mut want: BTreeMap<usize, BTreeMap<usize, Vec<(u16, bool)>>> = BTreeMap::new();
    for (pc, s) in one_way {
        if let Some(&(si, fi)) = s.at.first() {
            want.entry(si)
                .or_default()
                .entry(fi)
                .or_default()
                .push((*pc, s.taken == 0));
        }
    }
    let mut out = HashMap::new();
    for (si, frames) in &want {
        let mut t = Tapper::power_on(set);
        let last = *frames.keys().max().expect("a frame");
        for (fi, f) in srcs[*si].frames.iter().enumerate().take(last + 1) {
            let Some(pcs) = frames.get(&fi) else {
                t.step(controls(f));
                continue;
            };
            let ran = run_frame(&mut t, controls(f));
            let insns: Vec<Option<Insn>> = ran
                .trace
                .iter()
                .map(|e| e.interrupt.is_none().then(|| dasm::decode(&e.bytes, e.pc)))
                .collect();
            let ctx = contexts(&ran, &insns);
            for &(pc, want_taken) in pcs {
                let Some(bi) = ran
                    .trace
                    .iter()
                    .position(|e| e.pc == pc && e.interrupt.is_none())
                else {
                    continue;
                };
                if let Some(g) = guard_at(&ran, &insns, &ctx, bi, want_taken, names) {
                    out.insert(pc, g);
                }
            }
        }
    }
    out
}

/// Instructions behind `start` that no run executed: reached from there,
/// calls followed, stopping at code that ran and at the board's RAM.
pub(crate) fn unrun_behind(set: &Romset, start: u16, ran: &dyn Fn(u16) -> bool) -> usize {
    let mut seen = std::collections::BTreeSet::new();
    let mut todo = vec![start];
    while let Some(pc) = todo.pop() {
        if seen.len() >= 4096 || pc >= RAM || ran(pc) || !seen.insert(pc) {
            continue;
        }
        let at = usize::from(pc);
        let mut b = [0u8; 4];
        for (k, x) in b.iter_mut().enumerate() {
            *x = set.maincpu.get(at + k).copied().unwrap_or(0);
        }
        let i = dasm::decode(&b, pc);
        let next = pc.wrapping_add(u16::from(i.len));
        if let Some(t) = i.flow.target() {
            todo.push(t);
        }
        if i.flow.falls_through() {
            todo.push(next);
        }
    }
    seen.len()
}

/// The way a one-way conditional never went starts here: its target or
/// the next instruction; None for a `ret cc` that never returned.
pub(crate) fn unrun_start(set: &Romset, pc: u16, s: &Seen) -> Option<u16> {
    let at = usize::from(pc);
    let mut b = [0u8; 4];
    for (k, x) in b.iter_mut().enumerate() {
        *x = set.maincpu.get(at + k).copied().unwrap_or(0);
    }
    let i = dasm::decode(&b, pc);
    let fall = pc.wrapping_add(u16::from(i.len));
    if s.taken == 0 {
        i.flow.target()
    } else {
        Some(fall)
    }
}

/// A guard as the lead's JSON, in the 6502 guard's terms plus the flag.
pub(crate) fn guard_json(g: &Guard) -> Value {
    json!({
        "set_by": format!("{:04X}", g.setter),
        "other_way": if g.want_taken { "taken" } else { "fell through" },
        "set_by_text": g.setter_text,
        "flag": g.flag.name(),
        "constant": g.constant,
        "against": g.against,
        "source": g.source.map(|(ad, v)| format!("{ad:04X}={v}")),
        "grid_source": Value::Null,
        "value": g.value,
        "through_pointer": g.indirect,
        "chain": g.chain.iter().map(|a| format!("{a:04X}")).collect::<Vec<_>>(),
        "flips_on": g.flips.as_ref().map(Vec::len),
        "flips": g.flips.as_ref().map(|f| f.iter().take(16).copied().collect::<Vec<_>>()),
        "reads": g.reads.iter().take(6).map(|(ad, v)| format!("{ad:04X}={v}")).collect::<Vec<_>>(),
    })
}

/// One line of a guard for the text.
pub(crate) fn guard_line(g: &Guard) -> String {
    let what = match (g.constant, g.against) {
        (Some(k), _) => format!(" against ${k:02X}"),
        (None, Some(o)) => format!(" against ${o:02X} (not a constant)"),
        _ => String::new(),
    };
    let src = g
        .source
        .map(|(a, v)| {
            format!(
                ", from {a:04X}{} = ${v:02X}",
                if g.indirect {
                    " (through a pointer)"
                } else {
                    ""
                }
            )
        })
        .unwrap_or_default();
    let flips = match &g.flips {
        Some(f) => format!("; {} values go the other way", f.len()),
        None => "; the other way's values aren't worked out for it".to_owned(),
    };
    format!(
        "{} at {:04X} (`{}`){what}{src}, value ${:02X}{flips}",
        g.flag.name(),
        g.setter,
        g.setter_text,
        g.value
    )
}

/// `gp localize` on an arcade romset: steps 1 and 2, the leads and their
/// guards, written where the 6502's are (work/localize/GAME/LEVEL.json).
pub(crate) fn run(
    a: &LocalizeArgs,
    paths: &Paths,
    rom: &[u8],
    game_dir: Option<&Path>,
    t0: std::time::Instant,
) -> Result<Outcome> {
    for (given, what) in [
        (a.collect.is_some(), "--collect"),
        (a.check_log.is_some(), "--check-log"),
        (a.grade.is_some(), "--grade"),
        (a.poke, "--poke"),
    ] {
        if given {
            bail!(
                "{what} isn't read on an arcade romset: it plays the player to a place on the level grid, and {PROBE_STOP}. Without it, gp localize lists the one-way jumps, their guards and their places"
            );
        }
    }
    let set = Romset::from_zip(rom)?;
    let identity = set.identity.clone();
    let name = a
        .game
        .clone()
        .unwrap_or_else(|| format!("rom-{}", &identity[..12]));
    let observe = game_dir.filter(|d| d.join("observe.toml").is_file());
    let keyed = observe.is_some_and(|d| {
        fs::read_to_string(d.join("observe.toml"))
            .ok()
            .and_then(|t| t.parse::<toml::Table>().ok())
            .is_some_and(|t| t.contains_key("level"))
    });
    let (level, level_name, level_note) = match observe {
        Some(d) if keyed => (LevelKey::read(d)?, a.level.clone(), None),
        _ => (
            LevelKey::any(),
            "all".to_owned(),
            Some(format!(
                "{} names no [level]: every frame counts, as level all",
                observe.map_or_else(
                    || "the game has no observe.toml, so it".to_owned(),
                    |d| d.join("observe.toml").display().to_string()
                )
            )),
        ),
    };
    let names = game_dir.map_or_else(Names::empty, |d| crate::element::names_for(d, None).0);
    let srcs = crate::localize::sources(paths, &name, &identity, true)?;
    if srcs.is_empty() {
        bail!(
            "no clips of {name} on this romset to read (the mechanic sessions' clips under {}; a blind run is the NES's)",
            paths.home.join("mechanics").join(&name).display()
        );
    }
    let sc = scan(&set, &srcs, &level, &level_name);
    let ow = one_way(&sc);
    let gs = guards(&set, &srcs, &ow, &names);
    let ran = |pc: u16| sc.exec[usize::from(pc >> 6)] >> (pc & 63) & 1 == 1;
    let behind: HashMap<u16, usize> = ow
        .iter()
        .map(|(p, s)| {
            (
                *p,
                unrun_start(&set, *p, s).map_or(0, |st| unrun_behind(&set, st, &ran)),
            )
        })
        .collect();
    let mut by_code: Vec<(u16, usize)> = behind.iter().map(|(p, n)| (*p, *n)).collect();
    by_code.sort_by_key(|(p, n)| (std::cmp::Reverse(*n), *p));
    let grid = game_dir.and_then(Grid::load).is_some();
    // 3. Places, for a model in the board's own units.
    let board = match game_dir {
        Some(d) => Board::load(d, &level_name)?,
        None => Err("no cell is placed: a place is read from the game's state mapping and model, and there is no game folder".to_owned()),
    };
    let placed = match &board {
        Ok(b) => places(&set, &srcs, &level, &level_name, &ow, &gs, b),
        Err(_) => BTreeMap::new(),
    };
    let places_note = match &board {
        Ok(b) => format!(
            "{} placed on the level grid, a cell {} of the player's units",
            placed.values().filter(|p| p.cells > 0).count(),
            b.grid.cell
        ),
        Err(e) => e.clone(),
    };
    let mut ranked: Vec<(f64, u16, Value)> = placed
        .iter()
        .filter(|(_, p)| p.cells > 0)
        .map(|(pc, p)| (p.score, *pc, p.json.clone()))
        .collect();
    ranked.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    let mut leads: Vec<Value> = Vec::new();
    for (pc, s) in &ow {
        let mut guard = gs.get(pc).map_or(json!("not read"), guard_json);
        if let Some(p) = placed.get(pc) {
            guard["grid_source"] = json!(format!("{:04X}", p.grid_source));
        }
        leads.push(json!({
            "branch": format!("{pc:04X}"),
            "routine": names.routine(*pc),
            "unrun_behind": behind.get(pc).copied().unwrap_or(0),
            "went": if s.taken > 0 { "only taken" } else { "only fell through" },
            "runs": s.taken + s.fell,
            "first": s.at.first().map(|(si, fi)| json!([srcs[*si].name, fi])),
            "guard": guard,
            "meets": placed.get(pc).map(|p| p.json["meets"].clone()),
            "cells": placed.get(pc).map(|p| p.json["cells"].clone()),
        }));
    }
    let v = json!({
        "game": name, "platform": "arcade", "cpu": "main Z80", "level": level_name,
        "level_note": level_note,
        "sources": srcs.len(), "frames_in_level": sc.frames_in,
        "branches_seen": sc.seen.len(), "one_way": ow.len(), "guards_read": gs.len(),
        "placed": ranked.len(),
        "ranked": ranked.iter().map(|(_, _, v)| v.clone()).collect::<Vec<_>>(),
        "places": places_note, "probes_note": PROBE_STOP, "mapping_has_cells": grid,
        "by_code": by_code.iter().take(20).map(|(p, n)| json!({"branch": format!("{p:04X}"), "unrun_behind": n})).collect::<Vec<_>>(),
        "probes": [], "pokes": Value::Null, "graded": Value::Null,
        "seconds": (t0.elapsed().as_secs_f64() * 10.0).round() / 10.0,
    });
    let out = a.leads_dir(&paths.home, &name);
    fs::create_dir_all(&out)?;
    let file = out.join(format!("{level_name}.json"));
    fs::write(
        &file,
        serde_json::to_string_pretty(&json!({"summary": v, "leads": leads}))?,
    )?;
    let mut text = format!(
        "{} sources, {} frames in {} on the main Z80: {} conditionals, {} one way, {} guards read ({:.0} s)",
        srcs.len(),
        sc.frames_in,
        level_name,
        sc.seen.len(),
        ow.len(),
        gs.len(),
        t0.elapsed().as_secs_f64()
    );
    if let Some(n) = &level_note {
        text.push_str(&format!("\n{n}"));
    }
    text.push_str("\nby code behind the unrun side:");
    for (i, (p, n)) in by_code.iter().take(8).enumerate() {
        let s = &ow.iter().find(|(q, _)| q == p).expect("a lead").1;
        text.push_str(&format!(
            "\n  {}. {} at {p:04X} ({}): {n} instructions no run executed",
            i + 1,
            if s.taken > 0 {
                "only taken"
            } else {
                "only fell through"
            },
            names.routine(*p),
        ));
        match gs.get(p) {
            Some(g) => text.push_str(&format!("\n     guard: {}", guard_line(g))),
            None => text.push_str("\n     guard: not read"),
        }
    }
    for (_, pc, v) in ranked.iter().take(8) {
        text.push_str(&format!(
            "\n  place of {pc:04X}: its cell {} from the player at x {}, y {}; {} cells held a value that sends it the other way",
            v["grid_source"].as_str().unwrap_or("?"),
            v["meets"]["cell_from_player_x"],
            v["meets"]["cell_from_player_y"],
            v["cells"].as_array().map_or(0, Vec::len)
        ));
    }
    text.push_str(&format!(
        "\nplaces: {places_note}\nprobes: {PROBE_STOP}\nleads: {}",
        file.display()
    ));
    Ok(Outcome::ok(v, text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gp_evidence::input_log::InputFrame;

    /// A made-up program: it reads port 1 into e000, loads it back,
    /// compares it with $ef (port 1 with tap held) and jumps over a store
    /// to e001 when it isn't; then counts e002 and loops.
    fn program() -> Romset {
        gp_arcade::testrom::romset(
            &[
                0x31, 0xf0, 0xe7, // 0000: ld sp,e7f0
                0xdb, 0x01, // 0003: in a,(01)
                0x32, 0x00, 0xe0, // 0005: ld (e000),a
                0x3a, 0x00, 0xe0, // 0008: ld a,(e000)
                0xfe, 0xef, // 000b: cp $ef
                0x20, 0x03, // 000d: jr nz,0012
                0x32, 0x01, 0xe0, // 000f: ld (e001),a
                0x21, 0x02, 0xe0, // 0012: ld hl,e002
                0x34, // 0015: inc (hl)
                0x18, 0xeb, // 0016: jr 0003
            ],
            &[0x18, 0xfe],
        )
    }

    fn src(frames: usize) -> Vec<Src> {
        vec![Src {
            name: "made-up".into(),
            frames: vec![InputFrame::default(); frames],
        }]
    }

    /// With nothing held the jump at 000d only ever goes to its target;
    /// its guard is the `cp $ef` at 000b, on A loaded from e000, which the
    /// frame filled from port 1; only $ef sends it the other way, and the
    /// code behind that way (the store at 000f) never ran.
    #[test]
    fn a_one_way_jump_is_guarded_by_its_compare() {
        let set = program();
        let srcs = src(3);
        let any = LevelKey {
            ram: Vec::new(),
            when: None,
            names: BTreeMap::from([(Vec::new(), "all".to_owned())]),
        };
        let sc = scan(&set, &srcs, &any, "all");
        assert_eq!(sc.frames_in, 3);
        let ow = one_way(&sc);
        let (pc, s) = ow.iter().find(|(p, _)| *p == 0x000d).expect("000d one way");
        assert!(
            s.taken > 0 && s.fell == 0,
            "taken {} fell {}",
            s.taken,
            s.fell
        );
        assert!(
            !sc.seen.contains_key(&0x0016),
            "an unconditional jump isn't counted"
        );
        let gs = guards(&set, &srcs, &ow, &Names::empty());
        let g = &gs[pc];
        assert!(!g.want_taken);
        assert_eq!(g.flag, Flag::Z);
        assert_eq!(g.setter, 0x000b);
        assert_eq!(g.constant, Some(0xef));
        assert_eq!(g.value, 0xff);
        assert_eq!(g.source, Some((0xe000, 0xff)));
        assert_eq!(g.chain, vec![0xe000]);
        assert!(!g.indirect);
        assert_eq!(g.flips, Some(vec![0xef]));
        let ran = |p: u16| sc.exec[usize::from(p >> 6)] >> (p & 63) & 1 == 1;
        let start = unrun_start(&set, *pc, s).unwrap();
        assert_eq!(start, 0x000f);
        assert_eq!(unrun_behind(&set, start, &ran), 1, "only the store");
        let j = guard_json(g);
        assert_eq!(j["set_by"], "000B");
        assert_eq!(j["source"], "E000=255");
        assert!(guard_line(g).contains("against $EF"), "{}", guard_line(g));
    }

    /// A made-up level grid at e100 (32 by 30 bytes): the program puts 7
    /// in e110 (row 0, column 16), the player's bytes at e200 and e201,
    /// then tests the cell e105 against 7 and jumps over a store when it
    /// isn't, which it never is.
    fn grid_program() -> Romset {
        gp_arcade::testrom::romset(
            &[
                0x31, 0xf0, 0xe7, // 0000: ld sp,e7f0
                0x3e, 0x07, // 0003: ld a,07
                0x32, 0x10, 0xe1, // 0005: ld (e110),a
                0x3e, 0x40, // 0008: ld a,40
                0x32, 0x00, 0xe2, // 000a: ld (e200),a
                0x3e, 0xc1, // 000d: ld a,c1
                0x32, 0x01, 0xe2, // 000f: ld (e201),a
                0x3a, 0x05, 0xe1, // 0012: ld a,(e105)
                0xfe, 0x07, // 0015: cp $07
                0x20, 0x03, // 0017: jr nz,001c
                0x32, 0x01, 0xe0, // 0019: ld (e001),a
                0x18, 0xe5, // 001c: jr 0003
            ],
            &[0x18, 0xfe],
        )
    }

    /// The made-up game's folder: its mapping, with or without
    /// `board_units`, and a model whose level has tiles of 8.
    fn grid_game(dir: &Path, units: bool) {
        let mapping = format!(
            "platform = \"arcade\"\n{}schema = 22\nmodel = \"cab\"\nsource = \"cab\"\nalign = \"polled\"\n\n\
             [[field]]\npath = \"entities.player[0].x\"\nram = [0xE200]\noffset = -3\n\n\
             [[field]]\npath = \"entities.player[0].y\"\nram = [0xE201]\nscale = -1\noffset = 241\n\n\
             [cells]\npages = [0xE100]\npage_columns = 32\nrows = 30\ntop = 0\nview = 32\nkinds = [\"solid\"]\n",
            if units { "board_units = true\n" } else { "" }
        );
        fs::write(dir.join("mapping.toml"), mapping).unwrap();
        let model = "[model]\nschema = \"0.4\"\nname = \"cab\"\ntitle = \"Cab\"\ndescription = \"A made-up arcade game\"\nsource = \"cab\"\nview = \"side\"\nframe_rate = \"30\"\nsystems = [\"rules\", \"entity_states\", \"counters\", \"spawn\", \"exits\", \"camera\"]\n\n[input]\ndevice = \"arcade\"\n\n[input.actions]\nleft = \"left\"\n\n[entities.player]\nrole = \"player\"\nsize = [16, 16]\nhitbox = { x = 0, y = 0, w = 16, h = 16 }\n\n[levels.main]\ndescription = \"A screen\"\nempty = true\nsize = [32, 30]\ntile_size = 8\n";
        fs::write(dir.join("model.toml"), model).unwrap();
    }

    /// In the board's units the guard at 0017 is placed: its cell e105 is
    /// at (40, 0), 8 units a cell, the player at (0x40 - 3, 241 - 0xc1) =
    /// (61, 48), so the cell sits (-21, -48) from it, and the one cell that
    /// held 7 is e110 at (128, 0). Without the marker no cell is placed,
    /// in one sentence that names it.
    #[test]
    fn a_guard_on_the_grid_is_placed_in_the_boards_units() {
        let set = grid_program();
        let srcs = src(3);
        let any = LevelKey::any();
        let sc = scan(&set, &srcs, &any, "all");
        let ow = one_way(&sc);
        assert!(ow.iter().any(|(p, _)| *p == 0x0017), "0017 one way");
        let gs = guards(&set, &srcs, &ow, &Names::empty());
        assert_eq!(gs[&0x0017].source.map(|s| s.0), Some(0xe105));
        let d = tempfile::tempdir().unwrap();
        grid_game(d.path(), false);
        let e = Board::load(d.path(), "all").unwrap().err().unwrap();
        assert!(e.contains("board_units = true") && !e.contains(". "), "{e}");
        grid_game(d.path(), true);
        let b = Board::load(d.path(), "all").unwrap().unwrap();
        assert_eq!(b.grid.cell, 8);
        assert_eq!(b.at(0xe105), Some((40, 0)));
        let placed = places(&set, &srcs, &any, "all", &ow, &gs, &b);
        let p = &placed[&0x0017];
        assert_eq!(p.grid_source, 0xe105);
        assert_eq!(p.cells, 1);
        assert_eq!(p.json["cells"][0]["addr"], "E110");
        assert_eq!(p.json["cells"][0]["level_x"], 128);
        assert_eq!(p.json["meets"]["cell_from_player_x"], -21);
        assert_eq!(p.json["meets"]["cell_from_player_y"], -48);
    }

    /// The flags of the tests games choose by, worked out as the Z80 sets
    /// them, and which instructions set which flag.
    #[test]
    fn flags_are_worked_out_as_the_z80_sets_them() {
        let d = |b: &[u8]| dasm::decode(b, 0);
        // cp $10 with 5: borrow (C), not zero, sign set.
        assert_eq!(
            flags_for(&d(&[0xfe, 0x10]), 5, 0x10),
            Some([Some(false), Some(true), Some(true), Some(false)])
        );
        // and $01: zero when the bit is clear; carry cleared.
        assert_eq!(flags_for(&d(&[0xe6, 0x01]), 2, 1).unwrap()[0], Some(true));
        // dec b from 1 is zero; bit 3,a reads bit 3.
        assert_eq!(flags_for(&d(&[0x05]), 1, 0).unwrap()[0], Some(true));
        assert_eq!(
            flags_for(&d(&[0xcb, 0x5f]), 0x08, 0).unwrap()[0],
            Some(false)
        );
        // rlca's carry is bit 7.
        assert_eq!(flags_for(&d(&[0x07]), 0x80, 0).unwrap()[1], Some(true));
        // inc doesn't set C; add hl,de sets only C; ld a,(nn) sets none.
        assert!(!sets(&d(&[0x3c]), Flag::C));
        assert!(sets(&d(&[0x3c]), Flag::Z));
        assert!(sets(&d(&[0x19]), Flag::C) && !sets(&d(&[0x19]), Flag::Z));
        assert!(!sets(&d(&[0x3a, 0, 0xe0]), Flag::Z));
        // The conditions.
        assert_eq!(condition(&d(&[0x30, 0x02])), Some((Flag::C, false)));
        assert_eq!(condition(&d(&[0xc8])), Some((Flag::Z, true)));
        assert_eq!(condition(&d(&[0x10, 0xfe])), Some((Flag::B, true)));
        assert_eq!(condition(&d(&[0x18, 0xfe])), None);
        assert_eq!(immediate("cp $1c"), Some(0x1c));
        assert_eq!(immediate("ld a,($e000)"), None);
    }

    /// An interrupt's handler between the setter and the jump is passed
    /// over: its context differs from the jump's.
    #[test]
    fn a_handler_is_its_own_context() {
        use gp_arcade::mcr::Trace;
        let at = |pc: u16, sp: u16, irq: Option<u8>, bytes: [u8; 4]| Trace {
            cpu: 0,
            time: Default::default(),
            pc,
            af: 0,
            bc: 0,
            de: 0,
            hl: 0,
            ix: 0,
            iy: 0,
            sp,
            r: 0,
            r2: 0,
            i: 0,
            bytes,
            cycles: 0,
            interrupt: irq,
        };
        let trace = vec![
            at(0x10, 0xe7f0, None, [0xfe, 0x01, 0, 0]), // cp $01
            at(0x38, 0xe7ee, Some(0), [0; 4]),          // the interrupt
            at(0x38, 0xe7ee, None, [0xb7, 0, 0, 0]),    // or a
            at(0x39, 0xe7ee, None, [0xed, 0x4d, 0, 0]), // reti
            at(0x12, 0xe7f0, None, [0x28, 0x02, 0, 0]), // jr z
        ];
        let ran = Ran {
            accesses: vec![Vec::new(); trace.len()],
            trace,
        };
        let insns: Vec<Option<Insn>> = ran
            .trace
            .iter()
            .map(|e| e.interrupt.is_none().then(|| dasm::decode(&e.bytes, e.pc)))
            .collect();
        let ctx = contexts(&ran, &insns);
        assert_eq!(ctx, vec![None, Some(1), Some(1), Some(1), None]);
        let g = guard_at(&ran, &insns, &ctx, 4, true, &Names::empty()).unwrap();
        assert_eq!(g.setter, 0x10, "the handler's `or a` is passed over");
    }
}
