//! Micro-benchmarks of the paths every recording, replay and seek runs
//! through (docs/benchmarks.md): stepping a frame, hashing the canonical
//! state, restoring a checkpoint, encoding and decoding trace and event
//! chunks, and seeking in a stored recording. They use the built-in test
//! ROM, so they need no ROM folder. `just bench` runs them and records
//! their means as metrics; `gp bench` times the whole pipeline.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use gp_evidence::Store;
use gp_evidence::chunk::{DEFAULT_CHUNK_FRAMES, chunk_range};
use gp_evidence::events::{self, EventWriter};
use gp_evidence::input_log::{InputFrame, generate};
use gp_evidence::manifest::{PowerOn, Provenance, SourceRef};
use gp_evidence::trace::{self, TraceWriter};
use gp_nes::recording::{self, Capture, RecordRequest};
use gp_nes::{Nes, seek, testrom};

/// A console that has run `frames` frames with A held now and then.
fn running(rom: &[u8], frames: u64) -> Nes {
    let mut nes = Nes::power_on(rom, PowerOn::default()).unwrap();
    for f in 0..frames {
        nes.step(InputFrame {
            polled: true,
            pads: [u8::from(f % 40 < 10), 0, 0, 0],
            reset: false,
        })
        .unwrap();
    }
    nes
}

fn console(c: &mut Criterion) {
    let rom = testrom::build();
    let mut nes = running(&rom, 60);
    c.bench_function("nes/step", |b| {
        b.iter(|| nes.step(black_box(InputFrame::default())).unwrap())
    });
    c.bench_function("nes/state_hash", |b| b.iter(|| nes.state_hash().unwrap()));
    let snapshot = nes.snapshot().unwrap();
    let frame = nes.frame();
    let held = nes.held();
    c.bench_function("nes/snapshot", |b| b.iter(|| nes.snapshot().unwrap()));
    c.bench_function("nes/restore", |b| {
        b.iter(|| nes.restore(black_box(&snapshot), frame, held).unwrap())
    });
}

fn chunks(c: &mut Criterion) {
    let rom = testrom::build();
    let mut nes = Nes::power_on(&rom, PowerOn::default()).unwrap();
    nes.set_events(true);
    let (mut states, mut frame_events) = (Vec::new(), Vec::new());
    for f in 0..u64::from(DEFAULT_CHUNK_FRAMES) {
        states.push(nes.canonical_state().unwrap());
        nes.step(InputFrame {
            polled: true,
            pads: [u8::from(f % 16 < 4), 0, 0, 0],
            reset: false,
        })
        .unwrap();
        frame_events.push(nes.take_events());
    }
    let table = gp_nes::canonical::field_table();
    let fields = table.hash();
    let encode_trace = || {
        let mut w = TraceWriter::new(DEFAULT_CHUNK_FRAMES, fields);
        let mut out = None;
        for s in &states {
            out = w.push(s).unwrap().or(out);
        }
        out.unwrap()
    };
    c.bench_function("trace/encode_chunk", |b| b.iter(encode_trace));
    let chunk = encode_trace();
    let covers = chunk_range(0, DEFAULT_CHUNK_FRAMES, u64::from(DEFAULT_CHUNK_FRAMES));
    c.bench_function("trace/decode_chunk", |b| {
        b.iter(|| trace::decode_chunk_covering(black_box(&chunk), covers.clone(), &table).unwrap())
    });
    c.bench_function("trace/decode_last_state", |b| {
        b.iter(|| {
            trace::decode_state(black_box(&chunk), covers.clone(), &table, covers.end - 1).unwrap()
        })
    });
    let encode_events = || {
        let mut w = EventWriter::new(DEFAULT_CHUNK_FRAMES);
        let mut out = w.push(&[]);
        for e in &frame_events[..frame_events.len() - 1] {
            out = w.push(e).or(out);
        }
        w.finish().or(out).unwrap()
    };
    c.bench_function("events/encode_chunk", |b| b.iter(encode_events));
    let echunk = encode_events();
    c.bench_function("events/decode_chunk", |b| {
        b.iter(|| events::decode_chunk_covering(black_box(&echunk), covers.clone()).unwrap())
    });
}

fn seeking(c: &mut Criterion) {
    let rom = testrom::build();
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let input = generate(600, &["a@10-40", "right@100-"]).unwrap();
    let manifest = recording::record_with(
        &store,
        RecordRequest {
            rom: &rom,
            source: SourceRef::NesRom {
                sha256: gp_evidence::source::identify_nes(&rom).unwrap().sha256,
                mapper: 0,
            },
            source_id: None,
            power_on: PowerOn::default(),
            input: &input,
            checkpoint_interval: 60,
            provenance: Provenance::Scripted,
            name: "bench".into(),
            created_at: "1970-01-01T00:00:00Z".into(),
            notes: None,
        },
        Capture::ALL,
    )
    .unwrap()
    .manifest;
    // The last frame before a checkpoint: a restore and 59 frames.
    c.bench_function("seek/checkpoint_and_59_frames", |b| {
        b.iter(|| seek::open(&store, &manifest, &rom, black_box(359)).unwrap())
    });
    c.bench_function("seek/trace_state", |b| {
        b.iter(|| recording::trace_state(&store, &manifest, black_box(359)).unwrap())
    });
}

criterion_group!(benches, console, chunks, seeking);
criterion_main!(benches);
