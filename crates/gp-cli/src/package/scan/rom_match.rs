//! ROM content typed out in a text: the numbers a file holds, in order,
//! compared with the ROM's bytes.
//!
//! A text's numbers are read into two streams: every byte-sized value
//! (`bytes`), and every value with 16-bit ones as two bytes, low first
//! (`words`, for pointer tables). Three detectors compare them with each
//! ROM: the same bytes in the same order (`exact`, with up to `MAX_SKIP`
//! other numbers between them), the same bytes in another order
//! (`reordered`), and the bytes scaled or offset (`scaled`: each value
//! k times the ROM's plus one constant). A match counts only when it is
//! unlikely by chance: its expected number of chance matches, from the
//! ROM's own byte frequencies, the ROM's size and the number of places
//! tried, is below `ALPHA`. Nothing here prints a ROM byte; a match is
//! where it is in the file and in the ROM, and its length.

use std::collections::HashMap;

/// A match counts when the number of chance matches expected over every
/// place tried is below this.
pub const ALPHA: f64 = 0.01;
/// The fewest values a match of any kind may have.
pub const MIN_RUN: usize = 4;
/// The fewest values a reordered or scaled match may have.
pub const MIN_GROUP: usize = 6;
/// The longest window the reordered and scaled detectors try.
pub const MAX_WINDOW: usize = 16;
/// Between two matched values a text may hold this many other numbers
/// (a row number, a count) and the run goes on, when the next `SYNC`
/// match again.
pub const MAX_SKIP: usize = 2;
const SYNC: usize = 3;
/// Seeds are 3-byte windows; a window the ROM holds more often than this
/// seeds nothing (fill, common code).
const SEED_CAP: usize = 256;
/// The scales the scaled detector tries: each value k times the ROM's.
const SCALES: &[i64] = &[1, 2, 3, 4, 5, 6, 8, 10, 16, 32, 64, 128, 256];

/// One number in a text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Num {
    pub value: i64,
    /// Byte offsets of the token in the text.
    pub start: usize,
    pub end: usize,
    /// Written in hex with more than two digits (an address or a word).
    pub wide: bool,
}

/// Every integer token of a text, in order: decimal (with a leading `-`
/// after a space, `=`, `,`, `[` or `(`), `0x` or `$` hex, `0b` binary,
/// with an optional Rust type suffix. Floats, digits inside words and
/// version numbers (`0.4`) aren't numbers here.
pub fn numbers(text: &str) -> Vec<Num> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        let prev = if i > 0 { b[i - 1] } else { b' ' };
        let starts = (c.is_ascii_digit() || c == b'$')
            && !word(prev)
            && prev != b'.'
            && prev != b'$'
            && prev != b'#';
        if !starts {
            i += 1;
            continue;
        }
        let start = i;
        let (radix, digits_at) = if c == b'$' {
            (16, i + 1)
        } else if c == b'0' && matches!(b.get(i + 1), Some(b'x' | b'X')) {
            (16, i + 2)
        } else if c == b'0' && matches!(b.get(i + 1), Some(b'b' | b'B')) {
            (2, i + 2)
        } else {
            (10, i)
        };
        let mut j = digits_at;
        while j < b.len() && ((b[j] as char).is_digit(radix) || b[j] == b'_') {
            j += 1;
        }
        let digits: String = text[digits_at..j].chars().filter(|&c| c != '_').collect();
        // A type suffix.
        let mut end = j;
        for s in [
            "u8", "i8", "u16", "i16", "u32", "i32", "u64", "i64", "usize", "isize",
        ] {
            if text[end..].starts_with(s) && !b.get(end + s.len()).is_some_and(|&c| word(c)) {
                end += s.len();
                break;
            }
        }
        let float = b.get(end) == Some(&b'.') && b.get(end + 1).is_some_and(u8::is_ascii_digit);
        if digits.is_empty() || float || b.get(end).is_some_and(|&c| word(c) || c == b'$') {
            i = end.max(i + 1);
            while i < b.len() && (word(b[i]) || b[i] == b'.') {
                i += 1;
            }
            continue;
        }
        let Ok(mut v) = i64::from_str_radix(&digits, radix) else {
            i = end;
            continue;
        };
        let mut s = start;
        if radix == 10 && start > 0 && b[start - 1] == b'-' {
            let before = if start > 1 { b[start - 2] } else { b' ' };
            if before.is_ascii_whitespace() || matches!(before, b'=' | b',' | b'[' | b'(') {
                v = -v;
                s = start - 1;
            }
        }
        out.push(Num {
            value: v,
            start: s,
            end,
            wide: radix == 16 && digits.len() > 2,
        });
        i = end;
    }
    out
}

/// A stream of bytes read from a text's numbers: each byte with the index
/// of the number it came from, or None where a number doesn't fit.
pub type Stream = Vec<Option<(u8, usize)>>;

/// The `bytes` stream: byte-sized values (-128 to 255) as one byte.
pub fn byte_stream(nums: &[Num]) -> Stream {
    nums.iter()
        .enumerate()
        .map(|(k, n)| {
            (-128..=255)
                .contains(&n.value)
                .then_some((n.value as u8, k))
        })
        .collect()
}

/// The `words` stream: byte-sized values as one byte, wide or 16-bit
/// values as two, low first.
pub fn word_stream(nums: &[Num]) -> Stream {
    let mut out = Vec::new();
    for (k, n) in nums.iter().enumerate() {
        if (0..=0xFFFF).contains(&n.value) && (n.wide || n.value > 255) {
            out.push(Some((n.value as u8, k)));
            out.push(Some(((n.value >> 8) as u8, k)));
        } else if (-128..=255).contains(&n.value) {
            out.push(Some((n.value as u8, k)));
        } else {
            out.push(None);
        }
    }
    out
}

/// A ROM (or one region of it) indexed for matching.
pub struct RomIndex<'a> {
    pub name: String,
    pub data: &'a [u8],
    /// ln of each byte value's frequency in the data.
    ln_f: [f64; 256],
    /// ln of each step's frequency (data[i+1] - data[i], wrapping, as i8).
    ln_fd: [f64; 256],
    seeds: HashMap<[u8; 3], Vec<u32>>,
    /// Multiset hashes of every window of each length MIN_GROUP..=MAX_WINDOW.
    multisets: Vec<HashMap<u64, u32>>,
    /// Hashes of every run of steps of each length MIN_GROUP-1..MAX_WINDOW.
    steps: Vec<HashMap<u64, u32>>,
}

/// A value's random 64-bit key, for multiset hashes: the sum of the keys
/// of a window's bytes is the same in any order.
fn key(v: u8) -> u64 {
    let mut x = u64::from(v).wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

fn seq_hash(s: impl Iterator<Item = i64>) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for v in s {
        h ^= v as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01B3);
    }
    h
}

impl<'a> RomIndex<'a> {
    /// Indexes `data`; the reordered and scaled detectors look only in its
    /// first `tables` bytes (PRG, where tables are: CHR is pictures, whose
    /// regular rows line up with anything in another order or scale, and a
    /// picture typed out is found exactly).
    pub fn new(name: impl Into<String>, data: &'a [u8], tables: usize) -> Self {
        let tables = tables.min(data.len());
        let n = data.len().max(1) as f64;
        let mut counts = [0u32; 256];
        let mut dcounts = [0u32; 256];
        for &b in data {
            counts[b as usize] += 1;
        }
        for w in data.windows(2) {
            dcounts[w[1].wrapping_sub(w[0]) as usize] += 1;
        }
        let mut ln_f = [0.0; 256];
        let mut ln_fd = [0.0; 256];
        for v in 0..256 {
            ln_f[v] = ((f64::from(counts[v]) + 1.0) / (n + 256.0)).ln();
            ln_fd[v] = ((f64::from(dcounts[v]) + 1.0) / (n + 256.0)).ln();
        }
        let mut seeds: HashMap<[u8; 3], Vec<u32>> = HashMap::new();
        for (i, w) in data.windows(3).enumerate() {
            let v = seeds.entry([w[0], w[1], w[2]]).or_default();
            if v.len() <= SEED_CAP {
                v.push(i as u32);
            }
        }
        seeds.retain(|_, v| v.len() <= SEED_CAP);
        let mut multisets = Vec::new();
        let mut steps = Vec::new();
        for w in MIN_GROUP..=MAX_WINDOW {
            let mut m = HashMap::new();
            let mut s = HashMap::new();
            if tables >= w {
                let mut h: u64 = data[..w].iter().map(|&b| key(b)).fold(0, u64::wrapping_add);
                for i in 0..=tables - w {
                    if i > 0 {
                        h = h
                            .wrapping_sub(key(data[i - 1]))
                            .wrapping_add(key(data[i + w - 1]));
                    }
                    m.entry(h).or_insert(i as u32);
                    let st = data[i..i + w]
                        .windows(2)
                        .map(|p| i64::from(p[1].wrapping_sub(p[0]) as i8));
                    s.entry(seq_hash(st)).or_insert(i as u32);
                }
            }
            multisets.push(m);
            steps.push(s);
        }
        Self {
            name: name.into(),
            data,
            ln_f,
            ln_fd,
            seeds,
            multisets,
            steps,
        }
    }

    fn ln_r(&self) -> f64 {
        (self.data.len().max(1) as f64).ln()
    }

    /// ln of a value's chance after `prev`: the larger of its own
    /// frequency and its step's from `prev`, so the ROM's runs and ramps
    /// (tile numbers, counters) count as likely.
    fn ln_p(&self, prev: Option<u8>, v: u8) -> f64 {
        let own = self.ln_f[v as usize];
        match prev {
            Some(p) => own.max(self.ln_fd[v.wrapping_sub(p) as usize]),
            None => own,
        }
    }
}

/// What kind of copy a match is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    Exact,
    Reordered,
    Scaled,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Exact => "exact",
            Kind::Reordered => "reordered",
            Kind::Scaled => "scaled",
        }
    }
}

/// One match: numbers `first..=last` of the text hold `len` bytes of the
/// ROM from `rom_offset`.
#[derive(Debug, Clone, PartialEq)]
pub struct Match {
    pub kind: Kind,
    pub stream: &'static str,
    pub rom: String,
    pub rom_offset: usize,
    pub len: usize,
    pub first: usize,
    pub last: usize,
    /// For `scaled`: each value is `scale` times the ROM's plus `offset`.
    pub scale: i64,
    pub offset: i64,
    /// ln of the expected number of chance matches.
    pub ln_expected: f64,
}

/// Whether a window's distinct values are a block of consecutive values
/// (any order): a metatile's tile numbers, a list of slots. Too common to
/// say where it came from.
pub fn block(w: &[u8]) -> bool {
    let mut v: Vec<u8> = w.to_vec();
    v.sort_unstable();
    v.dedup();
    v.len() >= 2 && usize::from(v[v.len() - 1] - v[0]) < v.len() + 1
}

/// Whether a window is too regular to say where it came from: fewer than
/// 3 distinct values, or one step repeated (a ramp).
pub fn regular(w: &[u8]) -> bool {
    let mut seen = [false; 256];
    let mut distinct = 0;
    for &b in w {
        if !seen[b as usize] {
            seen[b as usize] = true;
            distinct += 1;
        }
    }
    if distinct < 3 {
        return true;
    }
    let step = w[1].wrapping_sub(w[0]);
    w.windows(2).all(|p| p[1].wrapping_sub(p[0]) == step)
}

fn ln_choose_multiset(w: &[u8]) -> f64 {
    // ln(n! / prod m_v!)
    let lnfact = |n: usize| (1..=n).map(|k| (k as f64).ln()).sum::<f64>();
    let mut counts = [0usize; 256];
    for &b in w {
        counts[b as usize] += 1;
    }
    lnfact(w.len()) - counts.iter().map(|&c| lnfact(c)).sum::<f64>()
}

/// Exact runs: the stream's bytes in the ROM's order. `tries` is the
/// number of places tried over the whole scan, for the chance bound.
pub fn exact(stream: &Stream, how: &'static str, rom: &RomIndex, tries: f64, out: &mut Vec<Match>) {
    let s = stream;
    let d = rom.data;
    let ln_alpha = ALPHA.ln();
    let mut i = 0;
    while i + 3 <= s.len() {
        let (Some((a, _)), Some((b, _)), Some((c, _))) = (s[i], s[i + 1], s[i + 2]) else {
            i += 1;
            continue;
        };
        let mut best: Option<(usize, usize, f64, usize)> = None; // (end, len, lnE, rom start)
        for &p in rom.seeds.get(&[a, b, c]).into_iter().flatten() {
            let p = p as usize;
            let (mut si, mut ri) = (i, p);
            let mut ln_p = 0.0;
            let mut matched: Vec<u8> = Vec::new();
            loop {
                if let (Some(Some((v, _))), true) = (s.get(si), ri < d.len())
                    && *v == d[ri]
                {
                    ln_p += rom.ln_p(matched.last().copied(), *v);
                    matched.push(*v);
                    si += 1;
                    ri += 1;
                    continue;
                }
                let resync = (1..=MAX_SKIP).find(|k| {
                    let a = si + k;
                    a + SYNC <= s.len()
                        && ri + SYNC <= d.len()
                        && (0..SYNC).all(|t| s[a + t].map(|x| x.0) == Some(d[ri + t]))
                });
                match resync {
                    Some(k) => {
                        // Each skip is more ways to line up by chance.
                        ln_p += ((MAX_SKIP + 1) as f64).ln();
                        si += k;
                    }
                    None => break,
                }
            }
            let len = ri - p;
            // The text's numbers the run spans, skipped ones too: a ramp
            // with gaps is still a ramp.
            let spanned: Vec<u8> = s[i..si].iter().filter_map(|x| x.map(|x| x.0)).collect();
            if len < MIN_RUN || regular(&matched) || regular(&spanned) || block(&matched) {
                continue;
            }
            let ln_e = rom.ln_r() + ln_p + tries.ln();
            if best.is_none_or(|b| ln_e < b.2) {
                best = Some((si, len, ln_e, p));
            }
        }
        match best {
            Some((end, len, ln_e, p)) if ln_e < ln_alpha => {
                let first = s[i].map_or(0, |x| x.1);
                let last = s[..end]
                    .iter()
                    .rev()
                    .find_map(|x| x.map(|x| x.1))
                    .unwrap_or(first);
                out.push(Match {
                    kind: Kind::Exact,
                    stream: how,
                    rom: rom.name.clone(),
                    rom_offset: p,
                    len,
                    first,
                    last,
                    scale: 1,
                    offset: 0,
                    ln_expected: ln_e,
                });
                i = end;
            }
            _ => i += 1,
        }
    }
}

/// A group of numbers to try in other orders and scales: an array, an
/// inline table or a table's fields, as indexes into the text's numbers.
pub type Group = Vec<usize>;

/// Reordered and scaled copies within groups. Windows of
/// MIN_GROUP..=MAX_WINDOW consecutive values of a group are tried; the
/// longest that counts at each start is kept, and the next try starts after
/// it.
pub fn in_groups(nums: &[Num], groups: &[Group], rom: &RomIndex, tries: f64, out: &mut Vec<Match>) {
    let ln_alpha = ALPHA.ln();
    // Every window length is tried at each start.
    let tries = tries * (MAX_WINDOW - MIN_GROUP + 1) as f64;
    let d = rom.data;
    for g in groups {
        let vals: Vec<i64> = g.iter().map(|&k| nums[k].value).collect();
        let mut i = 0;
        while i + MIN_GROUP <= vals.len() {
            let mut found: Option<(usize, Match)> = None;
            for w in (MIN_GROUP..=MAX_WINDOW.min(vals.len() - i)).rev() {
                let win = &vals[i..i + w];
                // Reordered: the same bytes as a ROM window, in another order.
                if win.iter().all(|v| (-128..=255).contains(v)) {
                    let bytes: Vec<u8> = win.iter().map(|&v| v as u8).collect();
                    let h = bytes.iter().map(|&b| key(b)).fold(0u64, u64::wrapping_add);
                    if !regular(&bytes)
                        && !block(&bytes)
                        && let Some(&p) = rom.multisets[w - MIN_GROUP].get(&h)
                    {
                        let p = p as usize;
                        let mut a = bytes.clone();
                        let mut b = d[p..p + w].to_vec();
                        a.sort_unstable();
                        b.sort_unstable();
                        let same_order = bytes == d[p..p + w];
                        if a == b && !same_order {
                            let ln_p: f64 = bytes.iter().map(|&v| rom.ln_f[v as usize]).sum();
                            let ln_e = rom.ln_r() + ln_choose_multiset(&bytes) + ln_p + tries.ln();
                            if ln_e < ln_alpha {
                                found = Some((
                                    w,
                                    Match {
                                        kind: Kind::Reordered,
                                        stream: "group",
                                        rom: rom.name.clone(),
                                        rom_offset: p,
                                        len: w,
                                        first: g[i],
                                        last: g[i + w - 1],
                                        scale: 1,
                                        offset: 0,
                                        ln_expected: ln_e,
                                    },
                                ));
                                break;
                            }
                        }
                    }
                }
                // Scaled: the steps between values are k times the ROM's.
                let steps: Vec<i64> = win.windows(2).map(|p| p[1] - p[0]).collect();
                let mut distinct = steps.clone();
                distinct.sort_unstable();
                distinct.dedup();
                if distinct.len() < 3 {
                    continue;
                }
                for &k in SCALES {
                    for k in [k, -k] {
                        if steps.iter().any(|s| s % k != 0) {
                            continue;
                        }
                        let unit: Vec<i64> = steps.iter().map(|s| s / k).collect();
                        if unit.iter().any(|u| !(-128..=127).contains(u)) {
                            continue;
                        }
                        let Some(&p) =
                            rom.steps[w - MIN_GROUP].get(&seq_hash(unit.iter().copied()))
                        else {
                            continue;
                        };
                        let p = p as usize;
                        let rs: Vec<i64> = d[p..p + w]
                            .windows(2)
                            .map(|q| i64::from(q[1].wrapping_sub(q[0]) as i8))
                            .collect();
                        if rs != unit {
                            continue;
                        }
                        // The ROM's values as signed or unsigned, whichever
                        // gives one offset for the whole window.
                        let off_u = win[0] - k * i64::from(d[p]);
                        let off_s = win[0] - k * i64::from(d[p] as i8);
                        let fits = |signed: bool, off: i64| {
                            (0..w).all(|t| {
                                let r = if signed {
                                    i64::from(d[p + t] as i8)
                                } else {
                                    i64::from(d[p + t])
                                };
                                win[t] == k * r + off
                            })
                        };
                        let offset = if fits(false, off_u) {
                            off_u
                        } else if fits(true, off_s) {
                            off_s
                        } else {
                            continue;
                        };
                        if k == 1 && offset == 0 {
                            // The exact detector's.
                            continue;
                        }
                        let ln_p: f64 = unit
                            .iter()
                            .map(|&u| rom.ln_fd[(u as i8 as u8) as usize])
                            .sum();
                        let ln_e =
                            rom.ln_r() + ln_p + tries.ln() + (2.0 * SCALES.len() as f64).ln();
                        if ln_e < ln_alpha {
                            found = Some((
                                w,
                                Match {
                                    kind: Kind::Scaled,
                                    stream: "group",
                                    rom: rom.name.clone(),
                                    rom_offset: p,
                                    len: w,
                                    first: g[i],
                                    last: g[i + w - 1],
                                    scale: k,
                                    offset,
                                    ln_expected: ln_e,
                                },
                            ));
                            break;
                        }
                    }
                    if found.is_some() {
                        break;
                    }
                }
                if found.is_some() {
                    break;
                }
            }
            match found {
                Some((w, m)) => {
                    out.push(m);
                    i += w;
                }
                None => i += 1,
            }
        }
    }
}

/// Every match of a text against a ROM: exact runs in both streams, then
/// reordered and scaled copies in the groups, less any that cover the same
/// numbers as an earlier one.
pub fn scan_text(nums: &[Num], groups: &[Group], rom: &RomIndex, tries: f64) -> Vec<Match> {
    let mut out = Vec::new();
    exact(&byte_stream(nums), "bytes", rom, tries, &mut out);
    exact(&word_stream(nums), "words", rom, tries, &mut out);
    in_groups(nums, groups, rom, tries, &mut out);
    // One match per set of numbers, the first found.
    let mut kept: Vec<Match> = Vec::new();
    for m in out {
        if !kept.iter().any(|k| k.first <= m.last && m.first <= k.last) {
            kept.push(m);
        }
    }
    kept.sort_by_key(|m| m.first);
    kept
}
