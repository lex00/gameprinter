//! Hook code that transcribes a routine instead of describing it.
//!
//! A hook function cites the routine it models: by its name (`fn e166`,
//! `fn dfe4_body`) or by a `$XXXX` address of $8000 or more in its doc
//! comment. The function's operands are the numbers in its body in order
//! (literals, and addresses written in its comments), less 0 and 1 and the
//! literals added to an accumulator (`c += 7`). The routine's operands are
//! the 6502 instructions' operands in address order (immediates, the
//! addresses they read and write, the targets of JSR), from walking the
//! routine in the ROM to its RTS. A function transcribes the routine when
//! the longest common subsequence of the two is at least `MIN_LCS`, covers
//! at least `MIN_COVER` of the shorter, and is at least twice the longest
//! common subsequence with the routine's operands reversed (the same values
//! in another order, so the match is about order, not about which values).
//! A function that counts instruction cycles (an accumulator with
//! `MIN_CYCLE_ADDS` or more literal adds of `MIN_CYCLE` or more) while
//! citing a routine is a transcription of how the routine runs, whatever
//! its operands. An add of 1 is a counter's or an index's step: no 6502
//! instruction takes fewer than 2 cycles, so `i += 1` counts no cycles.

use gp_nes::codemap::{Mn, Mode, decode};

pub const MIN_LCS: usize = 6;
pub const MIN_COVER: f64 = 0.4;
pub const MIN_CYCLE_ADDS: usize = 4;
/// The fewest cycles a 6502 instruction takes (an implied or immediate
/// one): an add below it isn't an instruction's cycles.
pub const MIN_CYCLE: i64 = 2;
/// The most instructions read from one routine.
const MAX_INSNS: usize = 2048;
/// A JMP to within this many bytes after the routine's start is part of it.
const JMP_NEAR: u16 = 0x400;

use super::rom_match::{Num, numbers};

/// One item of a Rust file: a function, or a const or static.
#[derive(Debug, Clone)]
pub struct Item {
    pub kind: &'static str,
    pub name: String,
    /// The doc comment's text.
    pub doc: String,
    /// Byte span of the item from its doc comment to its end.
    pub start: usize,
    pub end: usize,
    /// Byte span of the body (inside the braces), for a function.
    pub body: (usize, usize),
    pub line: usize,
}

/// The functions, consts and statics of a Rust source, at any depth (in
/// impls and modules too), each with its doc comment.
pub fn items(src: &str) -> Vec<Item> {
    let b = src.as_bytes();
    let starts: Vec<usize> = std::iter::once(0)
        .chain(src.match_indices('\n').map(|(i, _)| i + 1))
        .collect();
    let lines: Vec<&str> = src.split('\n').collect();
    let mut out = Vec::new();
    let mut doc_from: Option<usize> = None;
    let mut doc = String::new();
    let mut li = 0;
    while li < lines.len() {
        let line = lines[li];
        let t = line.trim_start();
        if let Some(d) = t.strip_prefix("///") {
            if doc_from.is_none() {
                doc_from = Some(starts[li]);
                doc.clear();
            }
            doc.push_str(d.trim());
            doc.push(' ');
            li += 1;
            continue;
        }
        if t.starts_with("#[") || (t.is_empty() && doc_from.is_some()) {
            li += 1;
            continue;
        }
        let mut words = t
            .split(|c: char| !(c.is_alphanumeric() || c == '_'))
            .filter(|w| !w.is_empty());
        let mut kind = None;
        let mut name = None;
        for w in words.by_ref() {
            match w {
                "pub" | "crate" | "super" | "in" | "self" | "unsafe" | "async" | "extern"
                | "default" => {}
                "fn" => {
                    kind = Some("fn");
                    name = words.next();
                    break;
                }
                "const" | "static" => {
                    // `const fn` is a function.
                    let next = words.next();
                    if next == Some("fn") {
                        kind = Some("fn");
                        name = words.next();
                    } else if next == Some("mut") {
                        kind = Some("static");
                        name = words.next();
                    } else {
                        kind = Some(if w == "const" { "const" } else { "static" });
                        name = next;
                    }
                    break;
                }
                _ => break,
            }
        }
        let (Some(kind), Some(name)) = (kind, name) else {
            doc_from = None;
            li += 1;
            continue;
        };
        let at = starts[li];
        let start = doc_from.take().unwrap_or(at);
        // The end: a function's matching brace, or a const's `;`.
        let mut i = at;
        let mut depth = 0i32;
        let mut body = (at, at);
        let mut end = src.len();
        while i < b.len() {
            match b[i] {
                b'/' if b.get(i + 1) == Some(&b'/') => {
                    while i < b.len() && b[i] != b'\n' {
                        i += 1;
                    }
                    continue;
                }
                b'/' if b.get(i + 1) == Some(&b'*') => {
                    i += 2;
                    while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                        i += 1;
                    }
                    i += 2;
                    continue;
                }
                b'"' => {
                    i += 1;
                    while i < b.len() && b[i] != b'"' {
                        if b[i] == b'\\' {
                            i += 1;
                        }
                        i += 1;
                    }
                }
                b'\'' => {
                    // A char literal ('a', '\n') or a lifetime ('a).
                    if b.get(i + 2) == Some(&b'\'') {
                        i += 2;
                    } else if b.get(i + 1) == Some(&b'\\') {
                        i += 1;
                        while i < b.len() && b[i] != b'\'' {
                            i += 1;
                        }
                    }
                }
                b'{' | b'(' | b'[' => {
                    if b[i] == b'{' && depth == 0 && kind == "fn" {
                        body.0 = i + 1;
                    }
                    depth += 1;
                }
                b'}' | b')' | b']' => {
                    depth -= 1;
                    if depth == 0 && b[i] == b'}' && kind == "fn" {
                        body.1 = i;
                        end = i + 1;
                        break;
                    }
                }
                b';' if depth == 0 => {
                    end = i + 1;
                    break;
                }
                _ => {}
            }
            i += 1;
        }
        let line_no = li + 1;
        out.push(Item {
            kind,
            name: name.to_owned(),
            doc: std::mem::take(&mut doc),
            start,
            end,
            body,
            line: line_no,
        });
        // Items nested in a function's body are part of it.
        li += 1;
        while li < starts.len() && starts[li] < end {
            li += 1;
        }
    }
    out
}

/// The routines a function cites: its name as an address (`e166`,
/// `dfe4_body`, `body_d28b`), then the `$XXXX` addresses of $8000 or more
/// in its doc comment, in order, without repeats.
pub fn cited(item: &Item) -> Vec<u16> {
    let mut out = Vec::new();
    for part in item.name.split('_') {
        if part.len() == 4
            && part.bytes().all(|c| c.is_ascii_hexdigit())
            && part.bytes().any(|c| c.is_ascii_alphabetic())
            && let Ok(a) = u16::from_str_radix(part, 16)
            && a >= 0x8000
        {
            out.push(a);
        }
    }
    for n in numbers(&item.doc) {
        if n.wide && (0x8000..=0xFFFF).contains(&n.value) && item.doc[n.start..].starts_with('$') {
            out.push(n.value as u16);
        }
    }
    let mut seen = std::collections::HashSet::new();
    out.retain(|a| seen.insert(*a));
    out
}

/// A function body's operands in order and its count of literal adds to
/// an accumulator.
pub fn fn_operands(src: &str, item: &Item) -> (Vec<i64>, usize) {
    let body = &src[item.body.0..item.body.1];
    // Accumulators: `let mut X = <int>` with `X += <int>` adds.
    let mut accs: Vec<String> = Vec::new();
    for line in body.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("let mut ")
            && let Some((name, val)) = rest.split_once('=')
        {
            let name = name.split(':').next().unwrap_or("").trim();
            let val = val.trim().trim_end_matches(';').trim();
            let first = val
                .split(|c: char| !c.is_ascii_digit())
                .next()
                .unwrap_or("");
            if !first.is_empty() && !name.is_empty() {
                accs.push(name.to_owned());
            }
        }
    }
    let mut adds = 0usize;
    let mut skip = std::collections::HashSet::new();
    for acc in &accs {
        let pat = format!("{acc} += ");
        for (i, _) in body.match_indices(&pat) {
            let after = &body[i + pat.len()..];
            if after.starts_with(|c: char| c.is_ascii_digit()) {
                let lit: String = after
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric())
                    .collect();
                let v = numbers(&lit).first().map_or(0, |n| n.value);
                if v >= MIN_CYCLE {
                    adds += 1;
                }
                skip.insert(i + pat.len());
            }
        }
        // `c + 9` in returns and `c += if .. { 9 } else { 13 }`.
        let pat2 = format!("{acc} + ");
        for (i, _) in body.match_indices(&pat2) {
            if body[i + pat2.len()..].starts_with(|c: char| c.is_ascii_digit()) {
                skip.insert(i + pat2.len());
            }
        }
    }
    // Literals in `if .. { N } else { M }` after an accumulator's `+=`.
    for acc in &accs {
        let pat = format!("{acc} += if ");
        for (i, _) in body.match_indices(&pat) {
            let rest = &body[i..];
            let stop = rest.find(';').unwrap_or(rest.len());
            for n in numbers(&rest[..stop]) {
                let at = i + n.start;
                if rest[..n.start]
                    .rfind('{')
                    .is_some_and(|b| rest[b + 1..n.start].trim().is_empty())
                {
                    skip.insert(at);
                }
            }
        }
    }
    let ops: Vec<i64> = numbers(body)
        .into_iter()
        .filter(|n: &Num| !skip.contains(&n.start))
        .map(|n| n.value)
        .filter(|&v| v != 0 && v != 1)
        .collect();
    (ops, if accs.is_empty() { 0 } else { adds })
}

/// A routine's operands in address order, from walking it in `image`
/// ($8000 to $FFFF) from `start` to its returns: every instruction reached
/// by falling through, branching or a JMP near its start.
pub fn routine_operands(image: &[u8], start: u16) -> Vec<i64> {
    let read = |a: u16| -> u8 {
        if a >= 0x8000 {
            image[usize::from(a - 0x8000) % image.len().max(1)]
        } else {
            0
        }
    };
    let mut seen = std::collections::BTreeMap::new();
    let mut todo = vec![start];
    while let Some(pc) = todo.pop() {
        if pc < 0x8000 || seen.contains_key(&pc) || seen.len() >= MAX_INSNS {
            continue;
        }
        let Some((mn, mode)) = decode(read(pc)) else {
            continue;
        };
        let len = mode.len();
        let lo = u16::from(read(pc.wrapping_add(1)));
        let hi = u16::from(read(pc.wrapping_add(2)));
        let arg = match len {
            2 => lo,
            3 => lo | hi << 8,
            _ => 0,
        };
        let next = pc.wrapping_add(len);
        let op: Option<i64> = match mode {
            Mode::Imp | Mode::Acc | Mode::Rel => None,
            _ if mn == Mn::Jmp => None,
            _ => Some(i64::from(arg)),
        };
        seen.insert(pc, op);
        match mn {
            Mn::Rts | Mn::Rti | Mn::Brk => {}
            Mn::Jmp => {
                if mode == Mode::Abs && arg >= start && arg < start.saturating_add(JMP_NEAR) {
                    todo.push(arg);
                }
            }
            Mn::Bcc | Mn::Bcs | Mn::Beq | Mn::Bmi | Mn::Bne | Mn::Bpl | Mn::Bvc | Mn::Bvs => {
                let t = next.wrapping_add(lo as i8 as i16 as u16);
                todo.push(t);
                todo.push(next);
            }
            _ => todo.push(next),
        }
    }
    seen.into_values()
        .flatten()
        .filter(|&v| v != 0 && v != 1)
        .collect()
}

/// The length of the longest common subsequence.
pub fn lcs(a: &[i64], b: &[i64]) -> usize {
    if a.is_empty() || b.is_empty() {
        return 0;
    }
    let mut prev = vec![0usize; b.len() + 1];
    let mut cur = vec![0usize; b.len() + 1];
    for x in a {
        for (j, y) in b.iter().enumerate() {
            cur[j + 1] = if x == y {
                prev[j] + 1
            } else {
                cur[j].max(prev[j + 1])
            };
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// What the comparison of one function with one routine found.
#[derive(Debug, Clone)]
pub struct Verdict {
    pub routine: u16,
    pub fn_ops: usize,
    pub routine_ops: usize,
    pub lcs: usize,
    pub null: usize,
    pub cycle_adds: usize,
    /// Which rules fired: "same-order" and "cycle-count".
    pub signals: Vec<&'static str>,
}

/// The best comparison of a function with the routines it cites, and
/// whether it is a transcription (`signals` not empty).
pub fn judge(src: &str, item: &Item, image: &[u8]) -> Option<Verdict> {
    let routines = cited(item);
    if routines.is_empty() || item.kind != "fn" {
        return None;
    }
    let (ops, adds) = fn_operands(src, item);
    let mut best: Option<Verdict> = None;
    for &r in routines.iter().take(4) {
        let rops = routine_operands(image, r);
        let l = lcs(&ops, &rops);
        let mut rev = rops.clone();
        rev.reverse();
        let null = lcs(&ops, &rev);
        let shorter = ops.len().min(rops.len()).max(1);
        let mut signals = Vec::new();
        if l >= MIN_LCS && l as f64 >= MIN_COVER * shorter as f64 && l >= 2 * null.max(1) {
            signals.push("same-order");
        }
        if adds >= MIN_CYCLE_ADDS {
            signals.push("cycle-count");
        }
        let v = Verdict {
            routine: r,
            fn_ops: ops.len(),
            routine_ops: rops.len(),
            lcs: l,
            null,
            cycle_adds: adds,
            signals,
        };
        let better = match &best {
            None => true,
            Some(b) => (v.signals.len(), v.lcs) > (b.signals.len(), b.lcs),
        };
        if better {
            best = Some(v);
        }
    }
    best
}
