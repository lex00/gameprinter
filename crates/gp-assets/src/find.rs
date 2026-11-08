//! Finding an asset map's spans in another ROM by their content
//! (docs/assets.md, "Finding assets in other dumps"): a dump with another
//! header, another revision or region, or a re-release that keeps the
//! game's data at other offsets or in other banks.
//!
//! The map's content section (crate::content) gives each span's BLAKE3 and
//! a fingerprint of each 16 bytes. One pass over the ROM's PRG and CHR
//! fingerprints the 16 bytes at every offset; each fingerprint that is a
//! span's chunk votes for where that span would start. The best starts,
//! and the shifts other spans moved by, are then checked chunk by chunk,
//! and the span is found unchanged when its BLAKE3 agrees, with small
//! changes when most of its chunks agree, and missing otherwise. A pointer
//! table into another asset is found after that asset, by its values less
//! the asset's new address. Both regions are searched for every span, so
//! tiles a re-release keeps in PRG (a CHR RAM board) are found too.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::hash::{BuildHasherDefault, Hasher};

use serde::Serialize;

use crate::content::{CHUNK, Content, blake3_hex, fingerprint, parse_prints};
use crate::map::{Asset, AssetMap, Region};
use crate::{Error, RomLayout};

/// A block counts as found with small changes when at least this share of
/// its bytes are in chunks that agree.
pub const CHANGED_SHARE: f64 = 0.5;

/// Where in a ROM a block is: the region and offset, and the bank and CPU
/// address its mapper would show it at, where that is fixed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Place {
    pub region: Region,
    pub offset: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bank: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bank_size: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu: Option<u32>,
}

impl Place {
    /// "PRG 0x4000 (bank 1, $C000)".
    pub fn text(&self) -> String {
        let mut s = format!("{} {:#06x}", self.region.name().to_uppercase(), self.offset);
        let mut extra = Vec::new();
        if let Some(b) = self.bank {
            extra.push(format!("bank {b}"));
        }
        if let Some(c) = self.cpu {
            extra.push(format!("${c:04X}"));
        }
        if !extra.is_empty() {
            s.push_str(&format!(" ({})", extra.join(", ")));
        }
        s
    }
}

/// What was found of one block.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum Found {
    /// The same bytes, here.
    Same { at: Place },
    /// Most of the bytes, here: `same_bytes` of the block's are in chunks
    /// that agree, and `differ` are the ranges (start and length, from the
    /// block's start) of the chunks that don't.
    Changed {
        at: Place,
        same_bytes: u32,
        differ: Vec<[u32; 2]>,
    },
    /// Not found: the best place agreed in `best_same_bytes` bytes.
    Missing { best_same_bytes: u32 },
}

impl Found {
    pub fn place(&self) -> Option<&Place> {
        match self {
            Found::Same { at } | Found::Changed { at, .. } => Some(at),
            Found::Missing { .. } => None,
        }
    }
}

/// One span of the map and what was found of it.
#[derive(Debug, Clone, Serialize)]
pub struct Block {
    pub asset: String,
    pub part: String,
    pub region: Region,
    pub offset: u32,
    pub len: u32,
    pub found: Found,
}

/// One asset, by its blocks: `same` when every block is unchanged,
/// `changed` when every block is found and some changed, `missing` when
/// any isn't found.
#[derive(Debug, Clone, Serialize)]
pub struct AssetState {
    pub name: String,
    pub state: &'static str,
    pub blocks: usize,
    pub same_bytes: u64,
    pub bytes: u64,
}

/// How much of the map's program the ROM holds: the PRG's distinct
/// 16-byte chunks outside every asset (fill left out) found anywhere in
/// the ROM's PRG.
#[derive(Debug, Clone, Serialize)]
pub struct CodeMatch {
    pub chunks: u32,
    pub found: u32,
}

impl CodeMatch {
    pub fn share(&self) -> f64 {
        if self.chunks == 0 {
            0.0
        } else {
            f64::from(self.found) / f64::from(self.chunks)
        }
    }
}

/// What [`find`] found of a map in a ROM.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    /// The map's source.
    pub source: String,
    /// The ROM's headerless SHA-256.
    pub sha256: String,
    /// Whether the ROM is the one the map describes.
    pub is_source: bool,
    pub mapper: u16,
    pub blocks: Vec<Block>,
    pub assets: Vec<AssetState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeMatch>,
    /// The shifts most unchanged blocks moved by, by region, most first:
    /// (region, shift, blocks).
    pub shifts: Vec<(Region, i64, u32)>,
}

impl Report {
    pub fn count(&self, state: &str) -> usize {
        self.assets.iter().filter(|a| a.state == state).count()
    }
}

/// A hasher for fingerprints, which are already mixed.
#[derive(Default)]
struct Ident(u64);
impl Hasher for Ident {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = self.0 << 8 | u64::from(b);
        }
    }
    fn write_u32(&mut self, n: u32) {
        self.0 = u64::from(n) | u64::from(n) << 32;
    }
}
/// A block's best find: its region, start, matching bytes, whether the
/// whole block is exact, and which of its pieces matched.
type Pick = (Region, u32, u32, bool, Vec<bool>);

type Fast<K, V> = HashMap<K, V, BuildHasherDefault<Ident>>;

/// A quick filter in front of a fingerprint table: one bit per value of
/// the fingerprint's top 16 bits.
struct Filter(Vec<u64>);
impl Filter {
    fn new() -> Self {
        Self(vec![0; 1 << 10])
    }
    fn add(&mut self, f: u32) {
        let i = (f >> 16) as usize;
        self.0[i >> 6] |= 1 << (i & 63);
    }
    fn has(&self, f: u32) -> bool {
        let i = (f >> 16) as usize;
        self.0[i >> 6] & (1 << (i & 63)) != 0
    }
}

/// The fingerprints of 16 bytes of one value, which are everywhere in ROMs
/// (fill) and say nothing about where a block is.
fn fills() -> BTreeSet<u32> {
    (0..=255u8).map(|b| fingerprint(&[b; CHUNK])).collect()
}

/// The bank and CPU address a mapper shows a region's offset at, where it
/// is fixed or a bank's number says it.
pub fn place(mapper: u16, region: Region, offset: u32, prg_len: usize, chr_len: usize) -> Place {
    let (bank_size, cpu) = match region {
        Region::Prg => {
            let last = |size: u32| offset / size == (prg_len as u32 / size).saturating_sub(1);
            match mapper {
                1 | 2 => (
                    Some(0x4000),
                    Some(if last(0x4000) { 0xC000 } else { 0x8000 } + offset % 0x4000),
                ),
                4 => (Some(0x2000), None),
                7 => (Some(0x8000), Some(0x8000 + offset % 0x8000)),
                _ if prg_len <= 0x4000 => (None, Some(0xC000 + offset)),
                _ if prg_len <= 0x8000 => (None, Some(0x8000 + offset)),
                _ => (Some(0x4000), None),
            }
        }
        Region::Chr => match mapper {
            3 | 66 => (Some(0x2000), None),
            1 => (Some(0x1000), None),
            4 => (Some(0x0400), None),
            _ if chr_len > 0x2000 => (Some(0x2000), None),
            _ => (None, None),
        },
    };
    Place {
        region,
        offset,
        bank: bank_size.map(|s| offset / s),
        bank_size,
        cpu,
    }
}

struct Wanted<'a> {
    c: &'a Content,
    asset: String,
    prints: Vec<u32>,
}

/// Searches `rom` for every span of `map` by its content. A map without a
/// content section finds nothing (every block missing); `gp assets hash`
/// writes one.
pub fn find(rom: &[u8], map: &AssetMap) -> Result<Report, Error> {
    let image = gp_evidence::source::identify_nes(rom)?;
    let layout = RomLayout::of(rom)?;
    let prg = &rom[layout.prg.clone()];
    let chr = &rom[layout.chr.clone()];
    let hay = |r: Region| if r == Region::Prg { prg } else { chr };
    let at = |r: Region, o: u32| place(image.header.mapper, r, o, prg.len(), chr.len());
    let owner: BTreeMap<String, String> = map
        .assets
        .iter()
        .flat_map(|a| a.spans().into_iter().map(|s| (s.part, a.name().to_owned())))
        .collect();
    let fill = fills();

    let mut wanted = Vec::new();
    for c in map.content.iter() {
        wanted.push(Wanted {
            c,
            asset: owner.get(&c.part).cloned().unwrap_or_default(),
            prints: parse_prints(&c.chunks).map_err(Error::Map)?,
        });
    }

    // The index: each full chunk of each block, by fingerprint.
    let mut index: Fast<u32, Vec<(u32, u32)>> = Fast::default();
    let mut filter = Filter::new();
    for (wi, w) in wanted.iter().enumerate() {
        for (k, &p) in w.prints.iter().enumerate() {
            if (k + 1) * CHUNK <= w.c.len as usize && !fill.contains(&p) {
                index.entry(p).or_default().push((wi as u32, k as u32));
                filter.add(p);
            }
        }
    }
    // The program's chunks, for picking the model.
    let mut code_set: Fast<u32, bool> = Fast::default();
    if let Some(code) = &map.code {
        let prints = parse_prints(&code.chunks).map_err(Error::Map)?;
        let mut covered = vec![false; prints.len()];
        for s in map.spans().iter().filter(|s| s.region == Region::Prg) {
            let first = s.offset as usize / CHUNK;
            let last = (s.offset as usize + s.len as usize).div_ceil(CHUNK);
            for c in covered.iter_mut().take(last).skip(first) {
                *c = true;
            }
        }
        let mut seen: Fast<u32, u32> = Fast::default();
        for (k, &p) in prints.iter().enumerate() {
            if (k + 1) * CHUNK <= code.len as usize && !covered[k] && !fill.contains(&p) {
                *seen.entry(p).or_default() += 1;
            }
        }
        for (p, n) in seen {
            if n == 1 {
                code_set.insert(p, false);
                filter.add(p);
            }
        }
    }

    // One pass: the 16 bytes at every offset of both regions.
    let mut votes: Fast<u64, u32> = Fast::default();
    let mut hits: Fast<u32, u32> = Fast::default();
    for region in [Region::Prg, Region::Chr] {
        let h = hay(region);
        if h.len() < CHUNK {
            continue;
        }
        for p in 0..=h.len() - CHUNK {
            let f = fingerprint(&h[p..p + CHUNK]);
            if !filter.has(f) {
                continue;
            }
            if region == Region::Prg
                && let Some(v) = code_set.get_mut(&f)
            {
                *v = true;
            }
            let Some(refs) = index.get(&f) else { continue };
            if refs.len() > 8 {
                continue;
            }
            let n = hits.entry(f).or_default();
            *n += 1;
            if *n > 256 {
                continue;
            }
            for &(wi, k) in refs {
                let off = k as usize * CHUNK;
                if p >= off {
                    let key = u64::from(wi) << 33
                        | u64::from(region == Region::Chr) << 32
                        | (p - off) as u64;
                    *votes.entry(key).or_default() += 1;
                }
            }
        }
    }
    let mut by_block: Vec<Vec<(u32, Region, u32)>> = vec![Vec::new(); wanted.len()];
    for (key, n) in votes {
        let wi = (key >> 33) as usize;
        let region = if key >> 32 & 1 == 1 {
            Region::Chr
        } else {
            Region::Prg
        };
        by_block[wi].push((n, region, key as u32));
    }
    for (wi, v) in by_block.iter_mut().enumerate() {
        v.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
        let least = if wanted[wi].prints.len() > 2 { 2 } else { 1 };
        v.retain(|c| c.0 >= least);
        v.truncate(4);
    }
    // Blocks shorter than a chunk: their one fingerprint, at every offset.
    let mut short: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (wi, w) in wanted.iter().enumerate() {
        if (w.c.len as usize) < CHUNK && w.c.len > 0 {
            short.entry(w.c.len as usize).or_default().push(wi);
        }
    }
    for (len, ws) in &short {
        for region in [Region::Prg, Region::Chr] {
            let h = hay(region);
            if h.len() < *len {
                continue;
            }
            for p in 0..=h.len() - len {
                let f = fingerprint(&h[p..p + len]);
                for &wi in ws {
                    if wanted[wi].prints[0] == f && by_block[wi].len() < 32 {
                        by_block[wi].push((1, region, p as u32));
                    }
                }
            }
        }
    }

    // How many of a block's bytes agree at a start, and whether all do.
    let score = |w: &Wanted, region: Region, start: u32| -> (u32, bool, Vec<bool>) {
        let h = hay(region);
        let len = w.c.len as usize;
        let mut same = 0u32;
        let mut ok = Vec::with_capacity(w.prints.len());
        for (k, &p) in w.prints.iter().enumerate() {
            let a = start as usize + k * CHUNK;
            let b = (a + CHUNK).min(start as usize + len);
            let agree = b <= h.len() && fingerprint(&h[a..b]) == p;
            if agree {
                same += (b - a) as u32;
            }
            ok.push(agree);
        }
        let end = start as usize + len;
        let exact = same as usize == len
            && end <= h.len()
            && blake3_hex(&h[start as usize..end]) == w.c.blake3;
        (same, exact, ok)
    };

    let mut best: Vec<Option<Pick>> = vec![None; wanted.len()];
    let pick = |w: &Wanted, cands: &[(Region, u32)], shifts: &[(Region, i64, u32)]| {
        let mut out: Option<(Region, u32, u32, bool, Vec<bool>, usize)> = None;
        for &(region, start) in cands {
            let (same, exact, ok) = score(w, region, start);
            // Lower is better: a shift most blocks moved by, the map's own
            // offset, then anything else.
            let rank = shifts
                .iter()
                .position(|&(r, d, _)| {
                    r == region
                        && region == w.c.region
                        && i64::from(start) - i64::from(w.c.offset) == d
                })
                .unwrap_or(if region == w.c.region && start == w.c.offset {
                    shifts.len()
                } else {
                    shifts.len() + 1
                });
            let better = match &out {
                None => true,
                Some((_, _, s, e, _, r)) => {
                    (exact, same, std::cmp::Reverse(rank)) > (*e, *s, std::cmp::Reverse(*r))
                }
            };
            if better {
                out = Some((region, start, same, exact, ok, rank));
            }
        }
        out.map(|(r, s, same, e, ok, _)| (r, s, same, e, ok))
    };
    let cands_of = |wi: usize, shifts: &[(Region, i64, u32)]| {
        let w = &wanted[wi];
        let mut c: Vec<(Region, u32)> = by_block[wi].iter().map(|&(_, r, s)| (r, s)).collect();
        c.push((w.c.region, w.c.offset));
        for &(r, d, _) in shifts {
            if r == w.c.region {
                let s = i64::from(w.c.offset) + d;
                if s >= 0 {
                    c.push((r, s as u32));
                }
            }
        }
        c.sort();
        c.dedup();
        c
    };
    for (wi, b) in best.iter_mut().enumerate() {
        *b = pick(&wanted[wi], &cands_of(wi, &[]), &[]);
    }
    let mut counts: BTreeMap<(Region, i64), u32> = BTreeMap::new();
    for (wi, b) in best.iter().enumerate() {
        if let Some((r, s, _, true, _)) = b
            && *r == wanted[wi].c.region
        {
            *counts
                .entry((*r, i64::from(*s) - i64::from(wanted[wi].c.offset)))
                .or_default() += 1;
        }
    }
    let mut shifts: Vec<(Region, i64, u32)> =
        counts.into_iter().map(|((r, d), n)| (r, d, n)).collect();
    shifts.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(&b.0)).then(a.1.cmp(&b.1)));
    shifts.truncate(4);
    for (wi, b) in best.iter_mut().enumerate() {
        *b = pick(&wanted[wi], &cands_of(wi, &shifts), &shifts);
    }

    let mut blocks = Vec::new();
    for (wi, w) in wanted.iter().enumerate() {
        let found = match &best[wi] {
            Some((r, s, _, true, _)) => Found::Same { at: at(*r, *s) },
            Some((r, s, same, false, ok))
                if f64::from(*same) >= CHANGED_SHARE * f64::from(w.c.len) && w.c.len > 0 =>
            {
                let mut differ: Vec<[u32; 2]> = Vec::new();
                for (k, agree) in ok.iter().enumerate() {
                    if *agree {
                        continue;
                    }
                    let a = (k * CHUNK) as u32;
                    let n = (CHUNK as u32).min(w.c.len - a);
                    match differ.last_mut() {
                        Some(d) if d[0] + d[1] == a => d[1] += n,
                        _ => differ.push([a, n]),
                    }
                }
                Found::Changed {
                    at: at(*r, *s),
                    same_bytes: *same,
                    differ,
                }
            }
            b => Found::Missing {
                best_same_bytes: b.as_ref().map_or(0, |b| b.2),
            },
        };
        blocks.push(Block {
            asset: w.asset.clone(),
            part: w.c.part.clone(),
            region: w.c.region,
            offset: w.c.offset,
            len: w.c.len,
            found,
        });
    }

    let mut assets = Vec::new();
    for a in &map.assets {
        let mine: Vec<&Block> = blocks.iter().filter(|b| b.asset == a.name()).collect();
        if mine.is_empty() {
            continue;
        }
        let state = if mine
            .iter()
            .any(|b| matches!(b.found, Found::Missing { .. }))
        {
            "missing"
        } else if mine.iter().all(|b| matches!(b.found, Found::Same { .. })) {
            "same"
        } else {
            "changed"
        };
        assets.push(AssetState {
            name: a.name().to_owned(),
            state,
            blocks: mine.len(),
            same_bytes: mine
                .iter()
                .map(|b| match &b.found {
                    Found::Same { .. } => u64::from(b.len),
                    Found::Changed { same_bytes, .. } => u64::from(*same_bytes),
                    Found::Missing { .. } => 0,
                })
                .sum(),
            bytes: mine.iter().map(|b| u64::from(b.len)).sum(),
        });
    }
    let code = map.code.as_ref().map(|_| CodeMatch {
        chunks: code_set.len() as u32,
        found: code_set.values().filter(|v| **v).count() as u32,
    });
    Ok(Report {
        source: map.source.clone(),
        sha256: image.sha256.clone(),
        is_source: image.sha256.eq_ignore_ascii_case(&map.sha256),
        mapper: image.header.mapper,
        blocks,
        assets,
        code,
        shifts,
    })
}

/// The assets that must load together: a string asset and the assets it
/// embeds, a pointer table and its targets, a level and its metatiles.
pub fn groups(map: &AssetMap) -> Vec<Vec<String>> {
    let names: Vec<&str> = map.assets.iter().map(Asset::name).collect();
    let mut parent: Vec<usize> = (0..names.len()).collect();
    fn root(p: &mut [usize], i: usize) -> usize {
        let mut i = i;
        while p[i] != i {
            p[i] = p[p[i]];
            i = p[i];
        }
        i
    }
    let idx = |n: &str| names.iter().position(|x| *x == n);
    for (i, a) in map.assets.iter().enumerate() {
        let mut links: Vec<String> = Vec::new();
        match a {
            Asset::PpuStrings(s) => links.extend(s.embeds.iter().map(|e| e.asset.clone())),
            Asset::Level {
                metatiles: Some(m), ..
            } => links.push(m.clone()),
            _ => {}
        }
        for l in links {
            if let Some(j) = idx(&l) {
                let (a, b) = (root(&mut parent, i), root(&mut parent, j));
                parent[a] = b;
            }
        }
    }
    let mut out: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    for (i, name) in names.iter().enumerate() {
        let r = root(&mut parent, i);
        out.entry(r).or_default().push((*name).to_owned());
    }
    let mut v: Vec<Vec<String>> = out.into_values().collect();
    v.sort_by_key(|g| idx(&g[0]));
    v
}

/// An image in the map's own layout (its PRG and CHR sizes, rounded up to
/// whole banks, mapper 0) holding what was found of each block of `rom`
/// at the block's offset in the map, with the pointer tables that moved
/// written back for the map's addresses, and the names of the assets every
/// block of which was found (unchanged or with small changes), whole
/// groups only ([`groups`]). Lifting the image with the map gives the
/// found assets as the ROM holds them.
pub fn assemble(
    rom: &[u8],
    map: &AssetMap,
    report: &Report,
) -> Result<(Vec<u8>, Vec<String>), Error> {
    let layout = RomLayout::of(rom)?;
    let src_prg = &rom[layout.prg.clone()];
    let src_chr = &rom[layout.chr.clone()];
    let end = |r: Region| {
        map.spans()
            .iter()
            .filter(|s| s.region == r)
            .map(|s| s.offset as usize + s.len as usize)
            .max()
            .unwrap_or(0)
    };
    let mut prg = vec![0u8; end(Region::Prg).div_ceil(0x4000).max(1) * 0x4000];
    let mut chr = vec![0u8; end(Region::Chr).div_ceil(0x2000) * 0x2000];
    for b in &report.blocks {
        let Some(p) = b.found.place() else { continue };
        let from = if p.region == Region::Prg {
            src_prg
        } else {
            src_chr
        };
        let a = p.offset as usize;
        let n = (b.len as usize).min(from.len().saturating_sub(a));
        let to = if b.region == Region::Prg {
            &mut prg
        } else {
            &mut chr
        };
        let o = b.offset as usize;
        to[o..o + n].copy_from_slice(&from[a..a + n]);
    }
    let state: BTreeMap<&str, &str> = report
        .assets
        .iter()
        .map(|a| (a.name.as_str(), a.state))
        .collect();
    let mut found = Vec::new();
    for g in groups(map) {
        // An asset with no blocks (no ROM span) goes with its group.
        let ok = g
            .iter()
            .all(|n| state.get(n.as_str()).is_none_or(|s| *s != "missing"))
            && g.iter().any(|n| state.contains_key(n.as_str()));
        if ok {
            found.extend(g);
        }
    }
    let mut image = b"NES\x1a".to_vec();
    image.push((prg.len() / 0x4000) as u8);
    image.push((chr.len() / 0x2000) as u8);
    image.resize(16, 0);
    image.extend(prg);
    image.extend(chr);
    Ok((image, found))
}

/// How well the chosen map's program matches a ROM.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Match {
    /// The ROM is the one the map describes.
    Exact,
    /// Every chunk of the map's program is in the ROM: the code is the
    /// same, though data or its places may differ.
    Code,
    /// Most of the program is there, changed in places: the game may not
    /// match the ROM's behaviour.
    Close,
    /// Too little of any map's program is there.
    None,
}

/// A program share at or over this is close; under it, no map matches.
pub const CLOSE_SHARE: f64 = 0.5;

/// Which of several maps' sources a ROM is, by its code: the one it is,
/// else the one with most of its program in the ROM (then most assets
/// found unchanged). A map without a `[code]` section is judged by the
/// share of its asset bytes found instead.
pub fn choose(reports: &[&Report]) -> Option<(usize, Match)> {
    if let Some(i) = reports.iter().position(|r| r.is_source) {
        return Some((i, Match::Exact));
    }
    let share = |r: &Report| match &r.code {
        Some(c) => c.share(),
        None => {
            let all: u64 = r.assets.iter().map(|a| a.bytes).sum();
            let same: u64 = r.assets.iter().map(|a| a.same_bytes).sum();
            if all == 0 {
                0.0
            } else {
                same as f64 / all as f64
            }
        }
    };
    let best = (0..reports.len()).max_by(|&a, &b| {
        let (ra, rb) = (reports[a], reports[b]);
        share(ra)
            .total_cmp(&share(rb))
            .then(ra.count("same").cmp(&rb.count("same")))
            .then(b.cmp(&a))
    })?;
    let s = share(reports[best]);
    let m = if reports[best]
        .code
        .as_ref()
        .is_some_and(|c| c.chunks > 0 && c.found == c.chunks)
    {
        Match::Code
    } else if s >= CLOSE_SHARE {
        Match::Close
    } else {
        Match::None
    };
    Some((best, m))
}
