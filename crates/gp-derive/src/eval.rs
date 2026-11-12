//! A constant-propagation pass over 6502 routines in source form: the
//! value each register holds at a line, where the code before it in its
//! routine decides it, and the values a routine stores. It reads the
//! source, not the ROM, so it runs wherever the disassembly is, and it
//! follows branches inside the window it reads: a label is entered with
//! what every branch and fall-through into it agrees on, and with nothing
//! known when code outside the window can jump there. A `jsr` forgets the
//! registers, since the routine called may change them.

use std::collections::BTreeMap;

use crate::asm::{Expr, Line, Mode, Source, Stmt, is_global, mentions};

/// The most lines the pass reads back from a line to its routine's start.
pub const WINDOW: usize = 160;

/// What is known at one point: each register's value, the carry, and the
/// bytes this window stored by name.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct State {
    pub a: Option<u8>,
    pub x: Option<u8>,
    pub y: Option<u8>,
    pub carry: Option<bool>,
    pub mem: BTreeMap<String, u8>,
}

impl State {
    fn join(&self, o: &State) -> State {
        let pick = |a: Option<u8>, b: Option<u8>| if a == b { a } else { None };
        State {
            a: pick(self.a, o.a),
            x: pick(self.x, o.x),
            y: pick(self.y, o.y),
            carry: if self.carry == o.carry {
                self.carry
            } else {
                None
            },
            mem: self
                .mem
                .iter()
                .filter(|(k, v)| o.mem.get(*k) == Some(v))
                .map(|(k, v)| (k.clone(), *v))
                .collect(),
        }
    }

    fn forget(&mut self) {
        *self = State::default();
    }
}

/// A value the pass found at a line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// What it is: `immediate`, `stored` (`sta`/`stx`/`sty`, the register's
    /// value), `loaded` (a load from a table at a known index, or from a
    /// byte the window stored), `result` (an arithmetic result).
    pub what: &'static str,
    /// The symbol stored to or loaded from, when there is one.
    pub target: Option<String>,
    pub value: i64,
}

fn branch(m: &str) -> bool {
    matches!(
        m,
        "bcc" | "bcs" | "beq" | "bne" | "bmi" | "bpl" | "bvc" | "bvs"
    )
}

/// A window of a file's lines and the label each line has, for the pass.
struct Window<'a> {
    src: &'a Source,
    file: &'a str,
    lines: &'a [Line],
    /// First line index (0-based) of the window, and one past the last.
    start: usize,
    end: usize,
}

impl Window<'_> {
    fn index_of(&self, label: &str) -> Option<usize> {
        (self.start..self.end).find(|&i| self.lines[i].label.as_deref() == Some(label))
    }

    /// Whether code outside the window can reach the label at line `i`.
    fn reached_from_outside(&self, i: usize) -> bool {
        let Some(l) = self.lines[i].label.as_deref() else {
            return false;
        };
        if !is_global(l) {
            // ca65's `:` and `+`/`-` labels are reached by relative
            // branches this pass doesn't resolve; treat them as unknown.
            return l == ":" || l.starts_with(['+', '-']);
        }
        let inside = (self.start..self.end)
            .filter(|&k| match &self.lines[k].stmt {
                Stmt::Op { arg: Some(e), .. } => mentions(e, l),
                _ => false,
            })
            .count();
        self.src.references(l) > inside
    }
}

fn byte(v: i64) -> u8 {
    (v & 0xFF) as u8
}

/// Steps one instruction. Returns the found value at it, if any, and
/// where control goes: fall through, a branch target name, or neither.
fn step(src: &Source, st: &mut State, line: &Line) -> (Option<Found>, Flow) {
    let Stmt::Op {
        mnemonic,
        mode,
        arg,
    } = &line.stmt
    else {
        return (None, Flow::Next);
    };
    let m = mnemonic.as_str();
    let val = arg.as_ref().and_then(|e| src.eval(e));
    let name = arg
        .as_ref()
        .and_then(|e| Source::name_of(e))
        .map(str::to_owned);
    // The byte an operand reads: an immediate, a byte the window stored, or
    // an entry of a data table at a known index.
    let read = |st: &State| -> Option<(u8, &'static str)> {
        match mode {
            Mode::Immediate => val.map(|v| (byte(v), "immediate")),
            Mode::Address => name
                .as_ref()
                .and_then(|n| st.mem.get(n).copied())
                .map(|v| (v, "loaded")),
            Mode::AddressX | Mode::AddressY => {
                let idx = if *mode == Mode::AddressX { st.x } else { st.y }?;
                let table = src.table(name.as_ref()?)?;
                let flat: Vec<Option<i64>> = table.into_iter().flatten().collect();
                flat.get(idx as usize)
                    .copied()
                    .flatten()
                    .map(|v| (byte(v), "loaded"))
            }
            _ => None,
        }
    };
    let found = |what, value: Option<u8>, st: &State| {
        let _ = st;
        value.map(|v| Found {
            what,
            target: name.clone(),
            value: i64::from(v),
        })
    };
    let mut out = None;
    match m {
        "lda" | "ldx" | "ldy" => {
            let r = read(st);
            let v = r.map(|x| x.0);
            match m {
                "lda" => st.a = v,
                "ldx" => st.x = v,
                _ => st.y = v,
            }
            out = r.map(|(v, what)| Found {
                what,
                target: name.clone(),
                value: i64::from(v),
            });
        }
        "sta" | "stx" | "sty" => {
            let v = match m {
                "sta" => st.a,
                "stx" => st.x,
                _ => st.y,
            };
            if *mode == Mode::Address
                && let Some(n) = &name
            {
                match v {
                    Some(v) => {
                        st.mem.insert(n.clone(), v);
                    }
                    None => {
                        st.mem.remove(n);
                    }
                }
            }
            out = found("stored", v, st);
        }
        "tax" => st.x = st.a,
        "tay" => st.y = st.a,
        "txa" => st.a = st.x,
        "tya" => st.a = st.y,
        "inx" => st.x = st.x.map(|v| v.wrapping_add(1)),
        "iny" => st.y = st.y.map(|v| v.wrapping_add(1)),
        "dex" => st.x = st.x.map(|v| v.wrapping_sub(1)),
        "dey" => st.y = st.y.map(|v| v.wrapping_sub(1)),
        "clc" => st.carry = Some(false),
        "sec" => st.carry = Some(true),
        "asl" | "lsr" | "rol" | "ror" if matches!(mode, Mode::Implied | Mode::Accumulator) => {
            let (a, c) = (st.a, st.carry);
            let (na, nc) = match (m, a) {
                ("asl", Some(a)) => (Some(a << 1), Some(a & 0x80 != 0)),
                ("lsr", Some(a)) => (Some(a >> 1), Some(a & 1 != 0)),
                ("rol", Some(a)) => (c.map(|c| (a << 1) | u8::from(c)), Some(a & 0x80 != 0)),
                ("ror", Some(a)) => (c.map(|c| (a >> 1) | (u8::from(c) << 7)), Some(a & 1 != 0)),
                _ => (None, None),
            };
            st.a = na;
            st.carry = nc;
            out = found("result", na, st);
        }
        "asl" | "lsr" | "rol" | "ror" | "inc" | "dec" => {
            if let Some(n) = &name {
                st.mem.remove(n);
            }
            if m != "inc" && m != "dec" {
                st.carry = None;
            }
        }
        "adc" | "sbc" => {
            let r = read(st).map(|x| x.0);
            let res = match (st.a, r, st.carry) {
                (Some(a), Some(b), Some(c)) if m == "adc" => {
                    let s = u16::from(a) + u16::from(b) + u16::from(c);
                    st.carry = Some(s > 0xFF);
                    Some(s as u8)
                }
                (Some(a), Some(b), Some(c)) => {
                    let s = i16::from(a) - i16::from(b) - i16::from(!c);
                    st.carry = Some(s >= 0);
                    Some(s as u8)
                }
                _ => {
                    st.carry = None;
                    None
                }
            };
            st.a = res;
            out = match mode {
                Mode::Immediate => found("immediate", r, st),
                _ => found("result", res, st),
            };
        }
        "and" | "ora" | "eor" => {
            let r = read(st).map(|x| x.0);
            st.a = match (st.a, r) {
                (Some(a), Some(b)) => Some(match m {
                    "and" => a & b,
                    "ora" => a | b,
                    _ => a ^ b,
                }),
                (_, Some(0)) if m == "and" => Some(0),
                _ => None,
            };
            if *mode == Mode::Immediate {
                out = found("immediate", r, st);
            }
        }
        "cmp" | "cpx" | "cpy" | "bit" => {
            let r = read(st).map(|x| x.0);
            st.carry = None;
            if *mode == Mode::Immediate {
                out = found("immediate", r, st);
            }
        }
        "pla" | "tsx" => {
            if m == "pla" {
                st.a = None;
            } else {
                st.x = None;
            }
        }
        "jsr" => st.forget(),
        "jmp" | "rts" | "rti" | "brk" => return (out, Flow::Stop),
        b if branch(b) => {
            return (out, Flow::Branch(name));
        }
        _ => {}
    }
    (out, Flow::Next)
}

enum Flow {
    Next,
    Branch(Option<String>),
    Stop,
}

/// Runs the pass over lines `start..=last` (0-based) of `file`, entering
/// `start` with nothing known. Returns the state before each line and the
/// value found at each.
fn run(src: &Source, file: &str, start: usize, last: usize) -> Vec<(State, Option<Found>)> {
    let lines = &src.files[file].lines;
    let w = Window {
        src,
        file,
        lines,
        start,
        end: last + 1,
    };
    let _ = w.file;
    let n = w.end - w.start;
    let mut entry: Vec<Option<State>> = vec![None; n];
    entry[0] = Some(State::default());
    let outside: Vec<bool> = (w.start..w.end)
        .map(|i| i > w.start && w.reached_from_outside(i))
        .collect();
    let mut found: Vec<Option<Found>> = vec![None; n];
    let mut work = vec![0usize];
    let mut visits = 0;
    while let Some(k) = work.pop() {
        visits += 1;
        if visits > 20 * n + 100 {
            break;
        }
        let i = w.start + k;
        let mut st = entry[k].clone().expect("queued with a state");
        if outside[k] {
            st.forget();
        }
        let (f, flow) = step(src, &mut st, &lines[i]);
        found[k] = f;
        let mut send = |to: usize, st: &State, work: &mut Vec<usize>| {
            let to_k = to - w.start;
            let next = match &entry[to_k] {
                None => st.clone(),
                Some(old) => old.join(st),
            };
            if entry[to_k].as_ref() != Some(&next) {
                entry[to_k] = Some(next);
                work.push(to_k);
            }
        };
        match flow {
            Flow::Stop => {}
            Flow::Next => {
                if i + 1 < w.end {
                    send(i + 1, &st, &mut work);
                }
            }
            Flow::Branch(target) => {
                if let Some(t) = target.as_deref().and_then(|t| w.index_of(t)) {
                    send(t, &st, &mut work);
                }
                if i + 1 < w.end {
                    send(i + 1, &st, &mut work);
                }
            }
        }
    }
    entry
        .into_iter()
        .zip(found)
        .map(|(s, f)| (s.unwrap_or_default(), f))
        .collect()
}

/// Lines in `lines[from..=to]` that name `label` in an operand.
fn refs_in(lines: &[Line], from: usize, to: usize, label: &str) -> usize {
    lines[from..=to]
        .iter()
        .filter(|l| matches!(&l.stmt, Stmt::Op { arg: Some(e), .. } if mentions(e, label)))
        .count()
}

/// The window's first line for `line` (0-based) of `file`: the nearest
/// label before it, extended back to the label before that while every
/// reference to it is inside the window (or, for a label nothing refers
/// to, while the code before it falls into it), at most [`WINDOW`] lines.
fn window_start(src: &Source, file: &str, line: usize) -> usize {
    let lines = &src.files[file].lines;
    let floor = line.saturating_sub(WINDOW);
    let labels: Vec<usize> = (floor..=line)
        .rev()
        .filter(|&i| lines[i].label.is_some())
        .collect();
    let Some(&first) = labels.first() else {
        return floor;
    };
    let mut start = first;
    for &prev in &labels[1..] {
        let l = lines[start].label.as_deref().expect("a label line");
        if !is_global(l) && !l.starts_with('@') && !l.starts_with('.') {
            break;
        }
        let total = src.references(l);
        if total == 0 {
            let ends = (prev..start).rev().find_map(|k| match &lines[k].stmt {
                Stmt::Op { mnemonic, .. } => {
                    Some(matches!(mnemonic.as_str(), "rts" | "rti" | "jmp"))
                }
                Stmt::Data { .. } => Some(true),
                _ => None,
            });
            if ends != Some(false) {
                break;
            }
        } else if refs_in(lines, prev, line, l) < total {
            break;
        }
        start = prev;
    }
    start
}

/// The value found at line `line` (1-based) of `file`, from the code before
/// it in its routine: an immediate operand, the register a store writes,
/// or a load from a table at a known index. `None` when the line isn't an
/// instruction or nothing is known there.
pub fn value_at(src: &Source, file: &str, line: usize) -> Option<Found> {
    let f = src.files.get(file)?;
    let i = line.checked_sub(1)?;
    if i >= f.lines.len() {
        return None;
    }
    let start = window_start(src, file, i);
    run(src, file, start, i).pop().and_then(|(_, f)| f)
}

/// The routine that starts at `line` (1-based) of `file`: every value its
/// code stores, loads from a table or compares with, from its label to
/// its first `rts`, `rti` or `jmp` that nothing branches past, at most
/// [`WINDOW`] lines.
pub fn routine_values(src: &Source, file: &str, line: usize) -> Vec<Found> {
    let Some(f) = src.files.get(file) else {
        return Vec::new();
    };
    let Some(start) = line.checked_sub(1).filter(|&i| i < f.lines.len()) else {
        return Vec::new();
    };
    let mut last = start;
    let limit = (start + WINDOW).min(f.lines.len() - 1);
    for i in start..=limit {
        last = i;
        if i > start && f.lines[i].global_label().is_some() && src.references_label(&f.lines[i]) {
            // The next routine.
            last = i - 1;
            break;
        }
        if let Stmt::Op { mnemonic, .. } = &f.lines[i].stmt
            && matches!(mnemonic.as_str(), "rts" | "rti" | "jmp")
            && !branched_past(f.lines.as_slice(), start, i)
        {
            break;
        }
    }
    run(src, file, start, last)
        .into_iter()
        .filter_map(|(_, f)| f)
        .collect()
}

impl Source {
    fn references_label(&self, l: &Line) -> bool {
        l.global_label().is_some_and(|g| self.references(g) > 0)
    }
}

/// Whether a branch between `start` and `at` goes to a label after `at`.
fn branched_past(lines: &[Line], start: usize, at: usize) -> bool {
    let later: Vec<&str> = lines[at + 1..(at + WINDOW).min(lines.len())]
        .iter()
        .filter_map(|l| l.label.as_deref())
        .collect();
    lines[start..at].iter().any(|l| match &l.stmt {
        Stmt::Op {
            mnemonic,
            arg: Some(Expr::Sym(t)),
            ..
        } => branch(mnemonic) && later.contains(&t.as_str()),
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn src(text: &str) -> Source {
        let mut s = Source::default();
        s.add("made-up.s", text);
        s
    }

    #[test]
    fn a_store_carries_the_value_loaded_before_it() {
        // A made-up routine: a timer set from an immediate, shifted, and a
        // speed read from a table at a known index.
        let s = src("HURT_TIME = 4\n\
             Hurt:\n\
             \tlda #HURT_TIME\n\
             \tsta Timer\n\
             \tasl\n\
             \tsta Sound\n\
             \tldy #2\n\
             \tlda SpeedTable,y\n\
             \tsta Speed\n\
             \trts\n\
             SpeedTable:\n\
             \t.db $10, $18, $28\n");
        assert_eq!(value_at(&s, "made-up.s", 4).map(|f| f.value), Some(4));
        assert_eq!(value_at(&s, "made-up.s", 6).map(|f| f.value), Some(8));
        let f = value_at(&s, "made-up.s", 9).expect("found");
        assert_eq!((f.what, f.value), ("stored", 0x28));
        let all: Vec<i64> = routine_values(&s, "made-up.s", 2)
            .iter()
            .map(|f| f.value)
            .collect();
        assert_eq!(all, vec![4, 4, 8, 8, 2, 0x28, 0x28]);
    }

    #[test]
    fn branches_join_and_calls_forget() {
        let s = src("Pick:\n\
             \tlda #1\n\
             \tldx Flag\n\
             \tbeq Same\n\
             \tlda #1\n\
             Same:\n\
             \tsta Out\n\
             \tldx #3\n\
             \tbne Diff\n\
             \tlda #5\n\
             Diff:\n\
             \tsta Out2\n\
             \tlda #7\n\
             \tjsr Other\n\
             \tsta Out3\n\
             \trts\n");
        // Both ways in hold 1.
        assert_eq!(value_at(&s, "made-up.s", 7).map(|f| f.value), Some(1));
        // One way holds 1 and the other 5.
        assert_eq!(value_at(&s, "made-up.s", 12), None);
        // The call may change A.
        assert_eq!(value_at(&s, "made-up.s", 15), None);
    }

    #[test]
    fn a_label_reached_from_elsewhere_starts_unknown() {
        let s = src("First:\n\
             \tlda #9\n\
             Shared:\n\
             \tsta Out\n\
             \trts\n\
             Elsewhere:\n\
             \tjmp Shared\n");
        assert_eq!(value_at(&s, "made-up.s", 4), None);
        let s = src("First:\n\
             \tlda #9\n\
             FallenInto:\n\
             \tsta Out\n\
             \trts\n");
        assert_eq!(value_at(&s, "made-up.s", 4).map(|f| f.value), Some(9));
    }

    #[test]
    fn carry_arithmetic() {
        let s =
            src("Add:\n\tlda #$f0\n\tclc\n\tadc #$20\n\tsta Lo\n\tlda #0\n\tadc #0\n\tsta Hi\n");
        assert_eq!(value_at(&s, "made-up.s", 5).map(|f| f.value), Some(0x10));
        assert_eq!(value_at(&s, "made-up.s", 8).map(|f| f.value), Some(1));
    }
}
