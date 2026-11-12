//! Sound requests (docs/gaps.md, "Sound requests"): the game asking its
//! sound engine for an effect, found for any game from the code observer,
//! and named where the disassembly names it.
//!
//! Games ask for a sound in one of two ways. Either game code writes an
//! effect ID to a RAM byte (a queue) that the sound engine reads and
//! clears, or it calls a play-sound routine with the ID in A. A queue is a
//! byte that code at three or more places sets to a nonzero value, that the
//! engine clears within three frames of most sets, and whose sets are
//! followed by a note starting on the APU (in at least 97% of them, clearly
//! more than music alone gives). A play-sound routine is a `JSR` target
//! called from four or more places with three or more values of A that is
//! the engine's way in (most calls write the APU) or feeds code that runs
//! after it; one the engine calls too is a utility. A label with a sound
//! name (sound, sfx, music, snd, song) admits either with weaker evidence.
//! Writes and calls made by the sound engine itself (code under a routine
//! that writes the APU on most of its calls, or in the PRG pieces the APU is
//! written from) aren't requests. Each set or call is one
//! request: its frame, the effect ID, a name (a constant of the
//! disassembly with that value and a sound name that the source uses
//! within three lines before the interface's name), and the routine that
//! asked.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::{NO_ROM, Run, trigger};

/// Frames after a request a queue is cleared in, or a note starts in.
const WITHIN: usize = 3;

/// Whether a name looks like sound.
pub fn sound_name(s: &str) -> bool {
    let l = s.to_ascii_lowercase();
    ["sound", "sfx", "music", "snd", "song"]
        .iter()
        .any(|w| l.contains(w))
}

/// The sound engine's code: the PRG pieces the APU is written from on a
/// tenth of the frames or more (8KB banks on a banked board, 1KB pages on
/// one that maps all its PRG at once). A write from there takes a request
/// off a queue, or is the engine's own state.
pub struct Engine {
    piece: u32,
    pieces: BTreeSet<u32>,
}

impl Engine {
    /// The PRG windows the engine's code is in: its pieces, widened to the
    /// 8KB window around each (all of PRG on a board with 32KB or less).
    pub fn windows(&self, prg_len: usize) -> Vec<std::ops::Range<usize>> {
        if prg_len <= 0x8000 {
            return if self.pieces.is_empty() {
                Vec::new()
            } else {
                std::iter::once(0..prg_len).collect()
            };
        }
        let mut w: BTreeSet<usize> = BTreeSet::new();
        for p in &self.pieces {
            w.insert((*p * self.piece) as usize / 0x2000);
        }
        w.into_iter()
            .map(|b| b * 0x2000..((b + 1) * 0x2000).min(prg_len))
            .collect()
    }

    pub fn of(run: &Run) -> Self {
        let piece: u32 = if run.prg.len() > 0x8000 {
            0x2000
        } else {
            0x400
        };
        // A piece the APU is written from on a tenth of the frames or more:
        // the engine runs every frame, where start-up code that silences
        // the APU once doesn't.
        let mut per: std::collections::BTreeMap<u32, u64> = Default::default();
        let mut with_code = 0u64;
        for c in run.frames.iter().filter_map(|f| f.code.as_ref()) {
            with_code += 1;
            let mine: BTreeSet<u32> = c
                .apu_pc
                .iter()
                .filter(|&&pc| pc != NO_ROM)
                .map(|pc| pc / piece)
                .collect();
            for p in mine {
                *per.entry(p).or_default() += 1;
            }
        }
        let pieces = per
            .into_iter()
            .filter(|&(_, n)| n * 10 >= with_code)
            .map(|(p, _)| p)
            .collect();
        Self { piece, pieces }
    }

    /// Whether the code at PRG offset `pc` is the engine's.
    pub fn has(&self, pc: u32) -> bool {
        pc != NO_ROM && self.pieces.contains(&(pc / self.piece))
    }
}

/// One way the game asks for sound.
#[derive(Debug, Clone, Serialize)]
pub struct Interface {
    /// `queue` (a RAM byte) or `routine` (a play-sound routine).
    pub kind: String,
    pub name: String,
    pub addr: Option<u16>,
    pub prg: Option<u32>,
    /// Distinct places that ask through it.
    pub setters: usize,
    pub requests: u64,
    /// The evidence it was taken on.
    pub why: String,
}

/// One request.
#[derive(Debug, Clone, Serialize)]
pub struct Request {
    /// Index into [`Run::frames`].
    pub at: usize,
    pub frame: u64,
    pub via: String,
    pub id: u8,
    /// The effect's name, or `<via> <id>`.
    pub name: String,
    pub requester: String,
}

fn effect_name(run: &Run, via: &str, id: u8) -> String {
    let all: Vec<&crate::Constant> = run
        .constants
        .iter()
        .filter(|c| c.value == i64::from(id) && sound_name(&c.name))
        .collect();
    // Constants the source uses next to this interface's name, if any.
    let tied: Vec<&crate::Constant> = all
        .iter()
        .copied()
        .filter(|c| c.near.iter().any(|n| n == via))
        .collect();
    // A constant of the same value used elsewhere is a guess; only one the
    // source uses with this interface names the effect.
    let names: Vec<&str> = tied.iter().map(|c| c.name.as_str()).take(3).collect();
    if names.is_empty() {
        format!("{via} {id:#04x}")
    } else {
        names.join("/")
    }
}

/// Finds the interfaces and every request through them, in frame order.
pub fn detect(run: &Run) -> (Vec<Interface>, Vec<Request>) {
    let k = &run.known;
    let n = run.frames.len();
    let note: Vec<bool> = run
        .frames
        .iter()
        .map(|f| f.apu.iter().any(|&(r, v)| trigger(r, v).is_some()))
        .collect();
    let note_after = |i: usize| (i..(i + WITHIN + 1).min(n)).any(|j| note[j]);
    // How often a note starts within the window anyway (music plays most
    // of the time): a queue's sets must beat it clearly.
    let base = (0..n).filter(|&i| note_after(i)).count() as f64 / n.max(1) as f64;
    let beats = |r: f64| r >= 0.97 && r - base >= 0.5 * (1.0 - base);

    let engine = Engine::of(run);
    let engine_pc = |pc: u32| engine.has(pc);
    // Queues.
    struct Byte {
        sets: Vec<(usize, u8, u32, u32, bool)>,
        /// Frames it was cleared in, and whether the engine cleared it.
        clears: Vec<(usize, bool)>,
        setters: BTreeSet<u32>,
        last: u8,
    }
    let mut bytes: BTreeMap<u16, Byte> = BTreeMap::new();
    for (i, f) in run.frames.iter().enumerate() {
        let Some(c) = &f.code else { continue };
        for w in &c.ram {
            let b = bytes.entry(w.addr).or_insert(Byte {
                sets: Vec::new(),
                clears: Vec::new(),
                setters: BTreeSet::new(),
                last: 0,
            });
            if w.value == 0 {
                b.clears.push((i, w.engine || engine_pc(w.pc)));
            } else {
                // A queue of flags asks for the bits a write adds.
                let id = match w.value & !b.last {
                    0 => w.value,
                    bits => bits,
                };
                b.sets.push((i, id, w.pc, w.routine, w.engine));
                if !w.engine {
                    b.setters.insert(w.pc);
                }
            }
            b.last = w.value;
        }
    }
    let mut ifaces = Vec::new();
    let mut reqs = Vec::new();
    for (addr, b) in &mut bytes {
        // Only the game's own sets ask; the engine's take a request off.
        b.sets.retain(|s| !s.4 && !engine_pc(s.2));
        if b.sets.is_empty() || (0x100..0x200).contains(addr) {
            continue;
        }
        let (name, _) = k.ram_name(*addr);
        let named = sound_name(&name);
        // The clear after each set, if within the window: the engine takes
        // a request off its queue, where game code resets its own state.
        let clear_after = |i: usize| {
            let j = b.clears.partition_point(|c| c.0 < i);
            b.clears.get(j).filter(|c| c.0 - i <= WITHIN)
        };
        let sets = b.sets.len() as f64;
        let cleared = b.sets.iter().filter(|s| clear_after(s.0).is_some()).count() as f64 / sets;
        let by_engine = b
            .sets
            .iter()
            .filter(|s| clear_after(s.0).is_some_and(|c| c.1))
            .count() as f64
            / sets;
        let noted =
            b.sets.iter().filter(|(i, ..)| note_after(*i)).count() as f64 / b.sets.len() as f64;
        // A queue is asked now and then, not set over and over each frame
        // as a temporary is.
        let occasional = b.sets.len() as f64 <= 0.5 * n as f64;
        let pattern = b.sets.len() >= 3
            && b.setters.len() >= 3
            && cleared >= 0.6
            && by_engine >= 0.5
            && beats(noted)
            && occasional;
        let by_name = named && cleared >= 0.5 && by_engine >= 0.3 && noted >= 0.5 && occasional;
        // Set from several places and nearly always taken off by the sound
        // engine within the window: a queue even when what it asks for
        // starts no note (a stop, a fade, a jingle that waits).
        let consumed = b.setters.len() >= 3 && cleared >= 0.9 && by_engine >= 0.9 && occasional;
        if !(pattern || by_name || consumed) {
            continue;
        }
        ifaces.push(Interface {
            kind: "queue".into(),
            name: name.clone(),
            addr: Some(*addr),
            prg: None,
            setters: b.setters.len(),
            requests: b.sets.len() as u64,
            why: format!(
                "set from {} places, cleared within {WITHIN} frames {:.0}% ({:.0}% by the engine), a note follows {:.0}%{}",
                b.setters.len(),
                cleared * 100.0,
                by_engine * 100.0,
                noted * 100.0,
                if named { ", sound label" } else if !pattern && consumed { ", taken off by the engine" } else { "" }
            ),
        });
        for &(i, id, pc, routine, _) in &b.sets {
            reqs.push(Request {
                at: i,
                frame: run.frames[i].frame,
                via: name.clone(),
                id,
                name: effect_name(run, &name, id),
                requester: k.writer_name(pc, routine),
            });
        }
    }

    // Play-sound routines. One called in the frame a queue asked is the
    // engine taking the request off the queue, not a second interface.
    let queued: BTreeSet<usize> = reqs.iter().map(|r| r.at).collect();
    let watched: BTreeSet<u32> = run.watched.iter().copied().collect();
    for st in &run.calls {
        if !watched.contains(&st.target) || st.target == NO_ROM {
            continue;
        }
        let name = k.routine_name(st.target);
        let named = sound_name(&name);
        // Called from many places with many IDs, leading to the APU; a
        // sound-named one may be called from fewer, unless it is the
        // engine's own entry (most of its calls write the APU).
        // A routine the engine calls too is a utility (a bank switch), not
        // a way to ask.
        let mut calls_all = 0u64;
        let mut from_engine = 0u64;
        let mut fed = 0u64;
        let mut call_frames = 0u64;
        let mut passes = 0u64;
        let mut wrote = 0u64;
        let mut asked_a: BTreeSet<u8> = BTreeSet::new();
        let mut carried_a: BTreeSet<u8> = BTreeSet::new();
        let mut callers: BTreeSet<u32> = BTreeSet::new();
        for (i, f) in run.frames.iter().enumerate() {
            let Some(c) = &f.code else { continue };
            let mine: Vec<&(u32, u32, u8, bool)> =
                c.calls.iter().filter(|x| x.0 == st.target).collect();
            if mine.is_empty() {
                continue;
            }
            call_frames += 1;
            // The ID goes where the request goes: the routine writes A's
            // value (or its bits) to RAM in the frame it's called.
            // Only writes that changed a byte are seen, so a repeat of the
            // last ID shows nothing; frames with a write decide.
            let own_writes = c.ram.iter().any(|w| w.routine == st.target && !w.engine);
            let mut passed = false;
            for &&(_, _, a, eng) in &mine {
                if eng {
                    continue;
                }
                asked_a.insert(a);
                if c.ram.iter().any(|w| {
                    w.routine == st.target
                        && !w.engine
                        && a != 0
                        && (w.value == a || w.value & a == a)
                }) {
                    carried_a.insert(a);
                    passed = true;
                }
            }
            wrote += u64::from(own_writes);
            passes += u64::from(passed);
            callers.extend(mine.iter().filter(|x| !x.3).map(|x| x.1));
            calls_all += mine.len() as u64;
            from_engine += mine.iter().filter(|x| x.3 || engine_pc(x.1)).count() as u64;
            // Feeds something that runs later: a byte it writes in this
            // frame is written by other code in the frames after (the
            // engine taking the request off).
            let own: BTreeSet<u16> = c
                .ram
                .iter()
                .filter(|w| w.routine == st.target && !w.engine)
                .map(|w| w.addr)
                .collect();
            let feeds = !own.is_empty()
                && run.frames[(i + 1).min(n)..(i + WITHIN + 1).min(n)]
                    .iter()
                    .filter_map(|g| g.code.as_ref())
                    .any(|g| {
                        g.ram
                            .iter()
                            .any(|w| w.routine != st.target && own.contains(&w.addr))
                    });
            fed += u64::from(feeds);
        }
        let utility = from_engine as f64 > 0.05 * calls_all as f64;
        // Asked now and then, not called every frame as a renderer is.
        let occasional = (calls_all as f64) <= 0.25 * n as f64;
        let noted_frames = run
            .frames
            .iter()
            .enumerate()
            .filter(|(i, f)| {
                f.code
                    .as_ref()
                    .is_some_and(|c| c.calls.iter().any(|x| x.0 == st.target))
                    && note_after(*i)
            })
            .count() as f64;
        let noted = noted_frames / (call_frames.max(1) as f64);
        let feeds = call_frames > 0 && fed * 2 >= call_frames;
        // A routine whose writes don't carry A asks for one fixed sound
        // (or none): what it writes to is the interface.
        // Most of the IDs it's asked with show up in what it writes (a
        // repeat of one shows nothing, so this counts IDs, not calls).
        let carries_a =
            wrote > 0 && (passes * 2 >= wrote || carried_a.len() * 2 >= asked_a.len().max(1));
        // Callers that load A with a constant just before the call
        // (`lda #id` / `jsr`): asking for a sound by its ID. A routine
        // called with whatever A held is passed no ID.
        let immediate = callers
            .iter()
            .filter(|&&pc| pc >= 2 && pc != NO_ROM && run.prg.get(pc as usize - 2) == Some(&0xA9))
            .count();
        let by_id = !callers.is_empty() && immediate * 5 >= callers.len() * 2;
        // Called from many places with many IDs, and either the engine's
        // own way in (most calls write the APU) or feeding it; a
        // sound-named one may be called from fewer.
        let pattern =
            st.callers >= 4 && st.distinct_a >= 3 && by_id && (st.engine || (feeds && carries_a));
        let by_name = named && st.callers >= 2 && st.distinct_a >= 2 && feeds && carries_a && by_id;
        if utility || !occasional || !(pattern || by_name) {
            continue;
        }
        let mut count = 0;
        let mut with_queue = 0;
        let first = reqs.len();
        for (i, f) in run.frames.iter().enumerate() {
            let Some(c) = &f.code else { continue };
            for &(t, caller, a, engine) in &c.calls {
                if t == st.target && !engine && !engine_pc(caller) {
                    count += 1;
                    if (i.saturating_sub(1)..=i).any(|j| queued.contains(&j)) {
                        with_queue += 1;
                    }
                    reqs.push(Request {
                        at: i,
                        frame: f.frame,
                        via: name.clone(),
                        id: a,
                        name: effect_name(run, &name, a),
                        requester: k.writer_name(caller, NO_ROM),
                    });
                }
            }
        }
        // A queue this routine fills is the same requests seen from inside
        // it: keep the routine's, which name the caller.
        let inside =
            |r: &Request| r.requester == name || r.requester.starts_with(&format!("{name}+"));
        let own = reqs[..first].iter().filter(|r| inside(r)).count();
        if own > 0 {
            let filled: BTreeSet<String> = reqs[..first]
                .iter()
                .filter(|r| inside(r))
                .map(|r| r.via.clone())
                .collect();
            let mut kept: Vec<Request> = reqs[..first]
                .iter()
                .filter(|r| !inside(r))
                .cloned()
                .collect();
            kept.extend(reqs.drain(first..));
            reqs = kept;
            ifaces.retain(|i| !(i.kind == "queue" && filled.contains(&i.name)));
        } else if count == 0 || with_queue as f64 >= 0.8 * count as f64 {
            reqs.truncate(first);
            continue;
        }
        ifaces.push(Interface {
            kind: "routine".into(),
            name,
            addr: None,
            prg: Some(st.target),
            setters: st.callers,
            requests: count,
            why: format!(
                "called from {} places with {} values of A, a note follows {:.0}%{}{}",
                st.callers,
                st.distinct_a,
                noted * 100.0,
                if st.engine {
                    ", writes the APU"
                } else {
                    ", feeds the engine"
                },
                if named { ", sound label" } else { "" }
            ),
        });
    }
    // Effects no constant names: the engine's pointer table, when one
    // points at a label for most IDs asked through the interface.
    let vias: BTreeSet<String> = reqs.iter().map(|r| r.via.clone()).collect();
    for via in vias {
        let unnamed = |r: &Request| r.via == via && r.name == format!("{via} {:#04x}", r.id);
        let ids: BTreeSet<u8> = reqs.iter().filter(|r| unnamed(r)).map(|r| r.id).collect();
        if ids.is_empty() {
            continue;
        }
        let names = pointer_table_names(run, &engine, &ids);
        for r in reqs.iter_mut().filter(|r| unnamed(r)) {
            if let Some(n) = names.get(&r.id) {
                r.name = n.clone();
            }
        }
    }
    reqs.sort_by_key(|r| (r.at, r.via.clone()));
    (ifaces, reqs)
}

/// Names for effect IDs from the sound engine's pointer table: a table of
/// two-byte pointers in the engine's code windows whose entry for an ID
/// points at the start of a ROM label (data or code). The table is the one
/// an indexed load of the engine reads whose entries point at labels for
/// the most of `ids` (at least eight and 90%; a queue of flags indexes no
/// table); a pointer is read as the CPU address in the table's own 16KB bank,
/// its own 8KB window, or the fixed last 16KB. Each ID whose entry hits a
/// label gets that label's name. Requiring the engine's read keeps a table
/// read one entry off (whose entries point at labels too) from being
/// taken for it.
pub fn pointer_table_names(run: &Run, engine: &Engine, ids: &BTreeSet<u8>) -> BTreeMap<u8, String> {
    let prg = &run.prg;
    let k = &run.known;
    let starts: BTreeMap<u32, &str> = k
        .rom
        .iter()
        .chain(&k.data)
        .map(|l| (l.prg, l.name.as_str()))
        .collect();
    let mut out = BTreeMap::new();
    // A queue of flags (every ID a single bit) indexes no table.
    let flags = ids.iter().all(|i| i.count_ones() == 1);
    if prg.is_empty() || starts.is_empty() || ids.len() < 8 || flags {
        return out;
    }
    let len = prg.len();
    let to_prg = |table: usize, p: u16, h: u8| -> Option<u32> {
        let p = usize::from(p);
        let o = match h {
            _ if len <= 0x8000 => p.checked_sub(0x8000)? % len,
            0 => (table & !0x3FFF) + (p.checked_sub(0x8000)? & 0x3FFF),
            1 => (table & !0x1FFF) + (p & 0x1FFF),
            _ => len.checked_sub(0x4000)? + p.checked_sub(0xC000)?,
        };
        (o < len).then_some(o as u32)
    };
    let max_id = usize::from(*ids.iter().max().unwrap_or(&0));
    // The engine's indexed loads (`lda abs,y` and the like): the table it
    // reads is one of their operands, which tells a table from the same
    // table read one entry off. Compared by the low 13 bits, the place in
    // an 8KB window.
    let windows = engine.windows(len);
    let mut operands: BTreeSet<usize> = BTreeSet::new();
    for w in &windows {
        for o in w.clone() {
            if matches!(prg[o], 0xB9 | 0xBD | 0xBE | 0xBC | 0x79 | 0x7D) && o + 2 < len {
                let a = u16::from_le_bytes([prg[o + 1], prg[o + 2]]);
                if a >= 0x8000 {
                    operands.insert(usize::from(a) & 0x1FFF);
                }
            }
        }
    }
    let mut best: (usize, usize, u8) = (0, 0, 0);
    for w in windows {
        for t in w.clone() {
            if t + 2 * max_id + 1 >= len {
                break;
            }
            for h in 0..3u8 {
                let hits = ids
                    .iter()
                    .filter(|&&id| {
                        let at = t + 2 * usize::from(id);
                        let p = u16::from_le_bytes([prg[at], prg[at + 1]]);
                        to_prg(t, p, h).is_some_and(|o| starts.contains_key(&o))
                    })
                    .count();
                // Only a table the engine reads.
                let read =
                    operands.contains(&(t & 0x1FFF)) || operands.contains(&((t + 1) & 0x1FFF));
                if read && hits > best.0 {
                    best = (hits, t, h);
                }
            }
        }
    }
    let (_, t, h) = best;
    let hits = ids
        .iter()
        .filter(|&&id| {
            let at = t + 2 * usize::from(id);
            let p = u16::from_le_bytes([prg[at], prg[at + 1]]);
            to_prg(t, p, h).is_some_and(|o| starts.contains_key(&o))
        })
        .count();
    if hits < 8 || hits * 10 < ids.len() * 9 {
        return out;
    }
    for &id in ids {
        let at = t + 2 * usize::from(id);
        let p = u16::from_le_bytes([prg[at], prg[at + 1]]);
        if let Some(n) = to_prg(t, p, h).and_then(|o| starts.get(&o)) {
            out.insert(id, (*n).to_owned());
        }
    }
    out
}

/// What each effect is, from when it's asked for and what the engine does
/// with it: one asked for within a second of a scene change on most of its
/// requests is `music`, and so is a request whose
/// ID the engine copies into a byte of its own that keeps it for 300
/// frames or more (the music now playing) is `music`; one kept for less
/// (an effect's channel) is an `effect`; one the engine keeps nowhere,
/// after which fewer notes start than before it, is a `command` (a stop
/// or a fade). Empty when it can't be told. By interface and ID.
pub fn classes(run: &Run, reqs: &[Request]) -> BTreeMap<(String, u8), String> {
    let engine = Engine::of(run);
    let n = run.frames.len();
    // Each RAM byte's change frames, for how long a value is kept.
    let mut changes: Vec<Vec<u32>> = vec![Vec::new(); 0x800];
    for (i, f) in run.frames.iter().enumerate() {
        for &a in &f.ram_changed {
            changes[usize::from(a & 0x7FF)].push(i as u32);
        }
    }
    let notes = |a: usize, b: usize| {
        run.frames[a.min(n)..b.min(n)]
            .iter()
            .flat_map(|f| f.apu.iter())
            .filter(|&&(r, v)| trigger(r, v).is_some())
            .count()
    };
    // Frames within a second of a scene change (a background redraw, play
    // starting or stopping): music is asked for as a scene begins.
    let mut scene = vec![false; n];
    for i in 0..n {
        let f = &run.frames[i];
        if f.cells.len() > crate::changes::REDRAW_CELLS
            || (i > 0 && run.frames[i - 1].play != f.play)
        {
            scene[i.saturating_sub(60)..(i + 61).min(n)].fill(true);
        }
    }
    let mut at_scene: BTreeMap<(String, u8), (u32, u32)> = BTreeMap::new();
    for r in reqs {
        let e = at_scene.entry((r.via.clone(), r.id)).or_default();
        e.0 += 1;
        e.1 += u32::from(scene[r.at]);
    }
    let mut per: BTreeMap<(String, u8), (Vec<u32>, u32, u32)> = BTreeMap::new();
    for r in reqs {
        let mut hold: Option<u32> = None;
        for g in r.at..(r.at + WITHIN + 1).min(n) {
            let Some(c) = &run.frames[g].code else {
                continue;
            };
            for w in &c.ram {
                let carries = w.value == r.id || (r.id.count_ones() == 1 && w.value & r.id != 0);
                if !carries || !(w.engine || engine.has(w.pc)) {
                    continue;
                }
                let a = usize::from(w.addr & 0x7FF);
                let after = changes[a].partition_point(|&x| x as usize <= g);
                let until = changes[a].get(after).map_or(n as u32, |&x| x);
                hold = Some(hold.unwrap_or(0).max(until - g as u32));
            }
        }
        let e = per.entry((r.via.clone(), r.id)).or_default();
        match hold {
            Some(h) => e.0.push(h),
            None => {
                let before = notes(r.at.saturating_sub(60), r.at);
                let after = notes(r.at + 5, r.at + 65);
                e.1 += 1;
                e.2 += u32::from(after < before);
            }
        }
    }
    let mut out: BTreeMap<(String, u8), String> = per
        .into_iter()
        .map(|(key, (mut holds, kept_nowhere, quieter))| {
            let total = holds.len() as u32 + kept_nowhere;
            let (asked, by_scene) = at_scene.get(&key).copied().unwrap_or((1, 0));
            let class = if by_scene * 2 > asked && by_scene >= 1 {
                "music"
            } else if kept_nowhere * 2 > total {
                if quieter * 2 > kept_nowhere {
                    "command"
                } else {
                    ""
                }
            } else {
                holds.sort_unstable();
                if holds[holds.len() / 2] >= 300 {
                    "music"
                } else {
                    "effect"
                }
            };
            (key, class.to_owned())
        })
        .collect();
    // One interface that asks for both, with music in one range of IDs and
    // effects in another: an ID left unclassed takes its range's class.
    let vias: BTreeSet<String> = out.keys().map(|k| k.0.clone()).collect();
    for via in vias {
        let ids = |c: &str| -> Vec<u8> {
            out.iter()
                .filter(|(k, v)| k.0 == via && v.as_str() == c)
                .map(|(k, _)| k.1)
                .collect()
        };
        let (music, effect) = (ids("music"), ids("effect"));
        if music.is_empty() || effect.is_empty() {
            continue;
        }
        // The split (music below it) that the most classed IDs agree with.
        let (wrong, split) = (0..=256u16)
            .map(|t| {
                let w = music.iter().filter(|&&m| u16::from(m) >= t).count()
                    + effect.iter().filter(|&&e| u16::from(e) < t).count();
                (w, t)
            })
            .min()
            .unwrap_or((usize::MAX, 0));
        if wrong * 5 > music.len() + effect.len() {
            continue;
        }
        for (k, v) in out.iter_mut().filter(|(k, _)| k.0 == via) {
            // Commands keep their class.
            if v != "command" {
                *v = if u16::from(k.1) < split {
                    "music"
                } else {
                    "effect"
                }
                .to_owned();
            }
        }
    }
    out
}

/// The routines worth watching calls to: sound-named labels, and every
/// target the run's call statistics show as a play-sound candidate.
pub fn candidates(run: &Run) -> Vec<u32> {
    let mut out: BTreeSet<u32> = run
        .known
        .rom
        .iter()
        .filter(|l| sound_name(&l.name))
        .map(|l| l.prg)
        .collect();
    for st in &run.calls {
        if st.callers >= 4 && st.distinct_a >= 3 && st.reaches_apu && st.target != NO_ROM {
            out.insert(st.target);
        }
    }
    out.into_iter().collect()
}
