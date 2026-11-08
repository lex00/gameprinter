//! Fuzzes the decoders that read stored bytes: trace and event chunks, field
//! tables, frame hashes and input logs. Inputs are random bytes and mutated
//! valid chunks (bit flips, forged header fields, truncation, and bodies
//! changed then recompressed, so the body parsers see damage too). Every
//! decode must return, without a panic, and must allocate no more than its
//! input and the manifest allow. A counting allocator (`stats_alloc`)
//! measures the bytes allocated during each decode.
//!
//! The generator is a fixed-seed xorshift, so a failure repeats. Set
//! GP_FUZZ_ITERATIONS to run longer than the default, which is sized for
//! `just check`.

use std::sync::Mutex;

use gp_evidence::chunk::max_decompressed;
use gp_evidence::events::{self, Event, EventKind, EventWriter};
use gp_evidence::trace::{self, FieldDef, FieldTable, TraceWriter, Width};
use gp_evidence::{InputLog, frame_hashes};
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};

#[global_allocator]
static GLOBAL: &StatsAlloc<std::alloc::System> = &INSTRUMENTED_SYSTEM;

/// The counts are process-wide, so the tests here take turns.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

/// The bytes `f` allocated, counting each reallocation's growth: an upper
/// bound on the most it held at once.
fn peak_of<T>(f: impl FnOnce() -> T) -> (T, usize) {
    let region = Region::new(GLOBAL);
    let out = f();
    let change = region.change();
    let grown = usize::try_from(change.bytes_reallocated.max(0)).unwrap_or(usize::MAX);
    (out, change.bytes_allocated.saturating_add(grown))
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }

    fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.next() as u8).collect()
    }

    /// Up to `max` random bytes.
    fn some_bytes(&mut self, max: usize) -> Vec<u8> {
        let n = self.below(max);
        self.bytes(n)
    }

    /// A value that tends to find edges: 0, 1, maxima, or random.
    fn edgy(&mut self) -> u64 {
        match self.below(6) {
            0 => 0,
            1 => 1,
            2 => u64::from(u32::MAX),
            3 => u64::MAX,
            4 => self.next() % 100_000,
            _ => self.next(),
        }
    }
}

fn table() -> FieldTable {
    FieldTable {
        state_magic: *b"FUZZ",
        state_version: 1,
        fields: vec![
            FieldDef {
                name: "a".into(),
                width: Width::Fixed(2),
            },
            FieldDef {
                name: "ram".into(),
                width: Width::Fixed(64),
            },
            FieldDef {
                name: "mem".into(),
                width: Width::Region,
            },
        ],
    }
}

/// The same state layout with the region as two fixed fields, so the
/// table's largest state is the state itself and the manifest's body bound
/// ([`trace::max_body`]) is far below what the compressed bytes could hold.
fn fixed_table() -> FieldTable {
    let mut t = table();
    t.fields.pop();
    for (name, width) in [("mem.len", 4), ("mem", 32)] {
        t.fields.push(FieldDef {
            name: name.into(),
            width: Width::Fixed(width),
        });
    }
    t
}

fn state(rng: &mut Rng, base: &mut Vec<u8>) -> Vec<u8> {
    if base.is_empty() {
        base.extend_from_slice(b"FUZZ");
        base.extend_from_slice(&1u32.to_le_bytes());
        base.extend(rng.bytes(66));
        base.extend_from_slice(&32u32.to_le_bytes());
        base.extend(rng.bytes(32));
    }
    // Change a few bytes after the header, not the region's length.
    for _ in 0..rng.below(4) {
        let i = 8 + rng.below(66);
        base[i] = rng.next() as u8;
    }
    base.clone()
}

/// Valid trace chunks of a recording of `frames` frames, `chunk_frames` each.
fn trace_chunks(rng: &mut Rng, table: &FieldTable, chunk_frames: u32, frames: u64) -> Vec<Vec<u8>> {
    let mut w = TraceWriter::new(chunk_frames, table.hash());
    let mut base = Vec::new();
    let mut out = Vec::new();
    for _ in 0..=frames {
        out.extend(w.push(&state(rng, &mut base)).unwrap());
    }
    out.extend(w.finish());
    out
}

fn event_chunks(rng: &mut Rng, chunk_frames: u32, frames: u64) -> Vec<Vec<u8>> {
    let mut w = EventWriter::new(chunk_frames);
    let mut out = Vec::new();
    for frame in 0..=frames {
        let evs: Vec<Event> = (0..rng.below(12))
            .map(|_| Event {
                frame,
                kind: EventKind::ALL[rng.below(8)],
                scanline: rng.below(262) as u16,
                dot: rng.below(341) as u16,
                addr: rng.next() as u16,
                value: rng.next() as u8,
            })
            .collect();
        out.extend(w.push(&evs));
    }
    out.extend(w.finish());
    out
}

/// One to four mutations of a valid chunk whose header is `header` bytes,
/// with the body length as the header's last u32.
fn mutate(rng: &mut Rng, chunk: &[u8], header: usize) -> Vec<u8> {
    let mut c = chunk.to_vec();
    for _ in 0..1 + rng.below(4) {
        match rng.below(7) {
            0 if !c.is_empty() => {
                let i = rng.below(c.len());
                c[i] ^= 1 << rng.below(8);
            }
            1 if !c.is_empty() => {
                let i = rng.below(c.len());
                c[i] = rng.next() as u8;
            }
            2 => c.truncate(rng.below(c.len() + 1)),
            3 => c.extend(rng.some_bytes(16)),
            // A forged header field: first frame, frame count or body length.
            4 if header > 0 && c.len() >= header => {
                let at = [8usize, 16, header - 4][rng.below(3)];
                let v = rng.edgy();
                if at == 8 {
                    c[8..16].copy_from_slice(&v.to_le_bytes());
                } else {
                    c[at..at + 4].copy_from_slice(&(v as u32).to_le_bytes());
                }
            }
            // A damaged body, recompressed so it decompresses.
            5 if header > 0 && c.len() >= header => {
                let len = u32::from_le_bytes(c[header - 4..header].try_into().unwrap());
                let Ok(mut body) = lz4_flex::block::decompress(&c[header..], len as usize) else {
                    continue;
                };
                for _ in 0..1 + rng.below(3) {
                    if body.is_empty() {
                        break;
                    }
                    let i = rng.below(body.len());
                    match rng.below(3) {
                        0 => body[i] = rng.next() as u8,
                        1 => body.truncate(i),
                        _ => body
                            .splice(i..i, rng.some_bytes(4).into_iter().chain([0]))
                            .for_each(drop),
                    };
                }
                c.truncate(header);
                c[header - 4..].copy_from_slice(&(body.len() as u32).to_le_bytes());
                c.extend(lz4_flex::block::compress(&body));
            }
            _ => {}
        }
    }
    c
}

fn iterations() -> usize {
    std::env::var("GP_FUZZ_ITERATIONS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3_000)
}

#[test]
fn decoders_never_panic_and_stay_within_their_bounds() {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let tables = [table(), fixed_table()];
    let table = &tables[0];
    let chunk_frames = 8;
    let frames = 20;
    let traces = [
        trace_chunks(&mut rng, &tables[0], chunk_frames, frames),
        trace_chunks(&mut rng, &tables[1], chunk_frames, frames),
    ];
    let streams = event_chunks(&mut rng, chunk_frames, frames);
    let mut decoded = [0usize; 5];
    for _ in 0..iterations() {
        let t = rng.below(2);
        let (tbl, chunks) = (&tables[t], &traces[t]);
        let i = rng.below(chunks.len());
        let expected = gp_evidence::chunk::chunk_range(i, chunk_frames, frames);
        let n = expected.end - expected.start;
        let manifest_body = trace::max_body(n as u32, tbl.max_state_len());

        let mut c = if rng.below(8) == 0 {
            rng.some_bytes(200)
        } else {
            mutate(&mut rng, &chunks[i], 56)
        };
        // A body length over what the manifest allows but within what the
        // compressed bytes could hold, so only the manifest's bound stops it.
        if rng.below(4) == 0 && c.len() > 56 {
            let most = max_decompressed(c.len() - 56);
            if most > manifest_body {
                let len = manifest_body + 1 + rng.below(most - manifest_body);
                c[52..56].copy_from_slice(&(len.min(u32::MAX as usize) as u32).to_le_bytes());
            }
        }
        // What the decoder may allocate: the body, bounded by the manifest
        // and by the compressed bytes, and the states it keeps.
        let body = max_decompressed(c.len()).min(manifest_body);
        let (r, peak) = peak_of(|| trace::decode_chunk_covering(&c, expected.clone(), tbl));
        let state = body.min(tbl.max_state_len());
        let bound = 2 * body + (2 * n as usize + 2) * (state + 64) + 4096;
        assert!(peak <= bound, "trace chunk: {peak} bytes, bound {bound}");
        decoded[0] += usize::from(r.is_ok());
        let k = expected.start + rng.below(n as usize) as u64;
        let (one, peak) = peak_of(|| trace::decode_state(&c, expected.clone(), tbl, k));
        assert!(
            peak <= 2 * body + 3 * (state + 64) + 4096,
            "one state: {peak} bytes"
        );
        // The one state agrees with the whole chunk's, success or failure.
        match (&r, &one) {
            (Ok(chunk), Ok(one)) => {
                assert_eq!(&chunk.states[(k - expected.start) as usize], one);
                assert_eq!(chunk.states.len() as u64, n);
                decoded[1] += 1;
            }
            (Err(_), Err(_)) => {}
            (r, one) => panic!("chunk {:?}, state {:?}", r.is_ok(), one.is_ok()),
        }
        decoded[4] += usize::from(t == 1 && r.is_ok());

        let i = rng.below(streams.len());
        let expected = gp_evidence::chunk::chunk_range(i, chunk_frames, frames);
        let c = if rng.below(8) == 0 {
            rng.some_bytes(200)
        } else {
            mutate(&mut rng, &streams[i], 24)
        };
        let body = max_decompressed(c.len());
        // Each event takes at least 5 body bytes and 16 decoded, and the
        // vector can hold twice what it uses.
        let bound = body + 2 * (body / 5 + 1) * 16 + 4096;
        let (r, peak) = peak_of(|| events::decode_chunk_covering(&c, expected.clone()));
        assert!(peak <= bound, "event chunk: {peak} bytes, bound {bound}");
        if let Ok(chunk) = &r {
            assert_events_in_bounds(chunk);
        }
        decoded[2] += usize::from(r.is_ok());
        let (_, peak) = peak_of(|| events::decode_chunk(&c));
        assert!(peak <= bound, "unchecked event chunk: {peak} bytes");

        // Formats with no compression: allocation follows the input.
        let raw = if rng.below(2) == 0 {
            rng.some_bytes(300)
        } else {
            mutate(&mut rng, &table.encode(), 0)
        };
        let (r, peak) = peak_of(|| FieldTable::decode(&raw));
        assert!(peak <= 64 * raw.len() + 4096, "field table: {peak} bytes");
        decoded[3] += usize::from(r.is_ok());
        let (_, peak) = peak_of(|| frame_hashes::decode(&raw));
        assert!(peak <= 2 * raw.len() + 4096, "frame hashes: {peak} bytes");
        let text = String::from_utf8_lossy(&raw);
        let (_, peak) = peak_of(|| InputLog::parse(&text));
        assert!(peak <= 64 * raw.len() + 4096, "input log: {peak} bytes");
    }
    // The mutations reach the decoders' success paths as well as their
    // errors, so they test more than the header checks.
    assert!(decoded.iter().all(|&n| n > 0), "{decoded:?}");
}

/// The forged headers the open question names, exactly: a small chunk
/// claiming 80,000 frames, a 4 GB body, or a first frame near u64::MAX.
#[test]
fn forged_headers_allocate_nothing_large() {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let mut rng = Rng(7);
    let table = table();
    let trace = &trace_chunks(&mut rng, &table, 8, 7)[0];
    let stream = &event_chunks(&mut rng, 8, 7)[0];
    for (at, value) in [
        (16usize, 80_000u64),
        (52, u64::from(u32::MAX)),
        (8, u64::MAX),
    ] {
        let mut c = trace.clone();
        if at == 8 {
            c[8..16].copy_from_slice(&value.to_le_bytes());
        } else {
            c[at..at + 4].copy_from_slice(&(value as u32).to_le_bytes());
        }
        let (r, peak) = peak_of(|| trace::decode_chunk_covering(&c, 0..8, &table));
        assert!(r.is_err() && peak < 64 * 1024, "trace {at}: {peak} bytes");
    }
    for (at, value) in [
        (16usize, 80_000u64),
        (20, u64::from(u32::MAX)),
        (8, u64::MAX),
    ] {
        let mut c = stream.clone();
        if at == 8 {
            c[8..16].copy_from_slice(&value.to_le_bytes());
        } else {
            c[at..at + 4].copy_from_slice(&(value as u32).to_le_bytes());
        }
        let (r, peak) = peak_of(|| events::decode_chunk_covering(&c, 0..8));
        assert!(r.is_err() && peak < 64 * 1024, "events {at}: {peak} bytes");
        let (_, peak) = peak_of(|| events::decode_chunk(&c));
        assert!(peak < 1 << 20, "unchecked events {at}: {peak} bytes");
    }
}

/// What every decoded event chunk must satisfy, whatever its bytes: no
/// frame over [`events::MAX_EVENTS_PER_FRAME`], and every position inside a
/// 312-scanline frame.
fn assert_events_in_bounds(chunk: &events::EventChunk) {
    let mut per_frame = std::collections::HashMap::<u64, usize>::new();
    for e in &chunk.events {
        assert!(
            e.scanline < events::MAX_SCANLINES && e.dot < events::DOTS,
            "{e:?}"
        );
        *per_frame.entry(e.frame).or_default() += 1;
    }
    assert!(
        per_frame
            .values()
            .all(|&n| n <= events::MAX_EVENTS_PER_FRAME),
        "a frame holds more than {} events",
        events::MAX_EVENTS_PER_FRAME
    );
}

/// A one-frame event chunk of `count` records, each of kind 0 at the
/// position change `zigzags` gives in turn (repeating the last).
fn event_chunk_of(count: usize, zigzags: &[u64]) -> Vec<u8> {
    let mut body = Vec::new();
    put_varint(&mut body, count as u64);
    for i in 0..count {
        body.push(0);
        put_varint(&mut body, zigzags[i.min(zigzags.len() - 1)]);
        body.extend_from_slice(&[0, 0, 0]);
    }
    let mut out = events::MAGIC.to_vec();
    out.extend_from_slice(&events::SCHEMA.to_le_bytes());
    out.extend_from_slice(&0u64.to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend(lz4_flex::block::compress(&body));
    out
}

fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let byte = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

/// Event chunks at the edges of the per-frame cap and the frame's last
/// position: each bound is what refuses the chunk just past it.
#[test]
fn event_limits_hold_at_their_edges() {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let cap = events::MAX_EVENTS_PER_FRAME;
    let last = u64::from(events::MAX_SCANLINES) * u64::from(events::DOTS) - 1;
    for (count, zigzags, ok) in [
        (cap, vec![0], true),
        (cap + 1, vec![0], false),
        (1, vec![last * 2], true),
        (1, vec![(last + 1) * 2], false),
        // Up to the last position and one past it, in steps.
        (2, vec![last * 2, 0], true),
        (2, vec![last * 2, 2], false),
    ] {
        let c = event_chunk_of(count, &zigzags);
        let r = events::decode_chunk_covering(&c, 0..1);
        if let Ok(chunk) = &r {
            assert_events_in_bounds(chunk);
        }
        assert_eq!(r.is_ok(), ok, "{count} events, {zigzags:?}");
    }
}
