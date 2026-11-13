//! `gp bench`: whole-pipeline timings on baselines, recorded as metrics
//! (docs/benchmarks.md). Each baseline is recorded with its trace and event
//! stream into a temporary store, replayed and checked from power-on,
//! sought to the frames that cost the most to reach, and read from its
//! trace; with `--video`, written as a video too. Nothing goes to
//! `$GP_STORE`.
//!
//! Metrics are named `bench.<source>.<baseline>.<what>`, such as
//! `bench.mosshop.finish-level.replay.fps`, so budgets can hold each game
//! to its own numbers.
//!
//! Ported from game-printer. A baseline whose ROM is an arcade romset (its
//! `sha256` the romset's identity, its holds in the board's layout, `coin`,
//! `start`, `right` ... `tap`) is recorded, replayed and sought on gp-arcade
//! (crate::arcade_rec), at the board's 30 frames a second. The metrics go
//! to this run's record (crate::runrec), as every gameprinter command's do.
//! `--video` is refused in words: gameprinter has no `gp video` yet.

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use gp_evidence::baseline::{self, Baseline};
use gp_evidence::manifest::{PowerOn, Provenance, Region, SourceRef};
use gp_evidence::source::identify_nes;
use gp_evidence::{Paths, Store};
use gp_nes::recording::{self, Capture, RecordRequest};
use serde_json::{Value, json};

use crate::Outcome;
use crate::open::RomFolder;

/// What `gp bench` was asked for.
pub(crate) struct Request<'a> {
    pub baselines: &'a [PathBuf],
    pub rom: Option<&'a Path>,
    pub seeks: u32,
    pub video: bool,
}

/// Frames per second the console runs at, for bytes per recorded minute.
fn console_fps(region: Region) -> f64 {
    match region {
        Region::Ntsc => 39_375_000.0 / 655_171.0,
        Region::Pal => 50.0070,
    }
}

/// The frames to seek to: the last frame before each checkpoint (a restore
/// and `interval - 1` frames of replay, the most a seek costs), spread over
/// the recording, `count` of them at most, and the last frame.
fn seek_frames(frames: u64, interval: u64, count: u32) -> Vec<u64> {
    let worst: Vec<u64> = (0..)
        .map(|k| k * interval + interval - 1)
        .take_while(|f| *f <= frames)
        .collect();
    let count = (count as usize).max(1);
    let mut out: Vec<u64> = if worst.len() <= count {
        worst
    } else {
        (0..count)
            .map(|i| worst[i * (worst.len() - 1) / (count - 1).max(1)])
            .collect()
    };
    out.push(frames);
    out.dedup();
    out
}

fn ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1e3
}

/// The ROM's identity: an NES image's headerless SHA-256, a romset's
/// identity.
fn identity(bytes: &[u8]) -> Result<String> {
    if gp_arcade::romset::looks_like_zip(bytes) {
        return Ok(gp_arcade::Romset::from_zip(bytes)?.identity);
    }
    Ok(identify_nes(bytes)?.sha256)
}

/// The romset in $GP_ROM_DIR whose identity is `id`, by the files' bytes.
fn find_romset(paths: &Paths, id: &str) -> Option<Vec<u8>> {
    let mut stack = vec![paths.roms.clone()];
    let mut files = Vec::new();
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).ok()?.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("zip")) {
                files.push(p);
            }
        }
    }
    files.sort();
    files.into_iter().find_map(|p| {
        let b = std::fs::read(&p).ok()?;
        (gp_arcade::Romset::from_zip(&b).ok()?.identity == id).then_some(b)
    })
}

/// A baseline file. One whose ROM is an arcade romset (the one given, or
/// one in $GP_ROM_DIR with its `sha256` as identity) holds the board's
/// buttons, which the NES check of `Baseline::load` refuses, so it is read
/// as written and its holds are checked when its input is made.
fn load(file: &Path, given: Option<&(&Path, String, Vec<u8>)>, paths: &Paths) -> Result<Baseline> {
    let raw: Option<Baseline> = std::fs::read_to_string(file)
        .ok()
        .and_then(|t| toml::from_str(&t).ok());
    if let Some(b) = raw {
        let arcade = match given {
            Some((_, id, bytes)) => gp_arcade::romset::looks_like_zip(bytes) && *id == b.sha256,
            None => find_romset(paths, &b.sha256).is_some(),
        };
        if arcade {
            return Ok(b);
        }
    }
    Ok(Baseline::load(file)?)
}

pub(crate) fn run(req: &Request<'_>, paths: &Paths) -> Result<Outcome> {
    if req.video {
        bail!(
            "--video renders each recording with gp video, which gameprinter hasn't got yet; bench without it"
        );
    }
    let mut files = Vec::new();
    for f in req.baselines {
        if f.is_dir() {
            files.extend(baseline::list(f)?);
        } else {
            files.push(f.clone());
        }
    }
    if files.is_empty() {
        bail!("no baselines to bench; name .toml files under baselines/ or their folders");
    }
    let given = match req.rom {
        Some(path) => {
            let bytes =
                std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
            Some((path, identity(&bytes)?, bytes))
        }
        None => None,
    };
    let mut roms = RomFolder::new(paths);
    let testrom_sha256 = identify_nes(&gp_nes::testrom::build())?.sha256;
    let (mut lines, mut results) = (Vec::new(), Vec::new());
    let (mut benched, mut failed) = (0, 0);
    for file in &files {
        let b = load(file, given.as_ref(), paths)?;
        let rom = if let Some((path, sha256, bytes)) = &given {
            if b.sha256 != *sha256 {
                bail!(
                    "{} has SHA-256 {sha256}, but {} expects {}",
                    path.display(),
                    file.display(),
                    b.sha256
                );
            }
            bytes.clone()
        } else if b.source.as_deref() == Some(gp_nes::testrom::SOURCE_ID) {
            if b.sha256 != testrom_sha256 {
                bail!(
                    "the built-in test ROM's SHA-256 is {testrom_sha256}, but {} expects {}",
                    file.display(),
                    b.sha256
                );
            }
            gp_nes::testrom::build()
        } else if let Some(rom) = roms.read(&b.sha256)? {
            rom
        } else if let Some(set) = find_romset(paths, &b.sha256) {
            set
        } else {
            lines.push(format!(
                "skip   {}  (no ROM with its hash in {})",
                file.display(),
                paths.roms.display()
            ));
            results.push(json!({ "file": file, "result": "skip" }));
            continue;
        };
        let stem = file
            .file_stem()
            .map_or_else(|| "baseline".into(), |s| s.to_string_lossy().into_owned());
        let key = format!(
            "bench.{}.{stem}",
            b.source.as_deref().unwrap_or("unregistered")
        );
        let r = bench_one(&b, &key, &rom, req)?;
        for (name, value, unit) in &r.metrics {
            crate::runrec::metric(format!("{key}.{name}"), *value, *unit);
        }
        benched += 1;
        if r.diverged.is_some() {
            failed += 1;
        }
        lines.push(r.line(&key));
        results.push(r.json(file, &key));
    }
    if benched == 0 {
        lines.push("nothing was benched: every baseline was skipped".into());
    }
    let exit = if benched == 0 {
        2
    } else {
        u8::from(failed > 0)
    };
    Ok(Outcome {
        json: json!({ "results": results, "benched": benched, "failed": failed }),
        text: lines.join("\n"),
        exit,
    })
}

struct Benched {
    frames: u64,
    metrics: Vec<(&'static str, f64, &'static str)>,
    diverged: Option<String>,
}

impl Benched {
    fn get(&self, name: &str) -> f64 {
        self.metrics
            .iter()
            .find(|m| m.0 == name)
            .map_or(f64::NAN, |m| m.1)
    }

    fn line(&self, key: &str) -> String {
        let mut line = format!(
            "{key}: {} frames; record {:.0} frames/s, replay {:.0} frames/s, seek max {:.1} ms (mean {:.1}), trace read max {:.2} ms, {:.0} bytes ({:.2} MB per recorded minute)",
            self.frames,
            self.get("record.fps"),
            self.get("replay.fps"),
            self.get("seek.max_ms"),
            self.get("seek.mean_ms"),
            self.get("trace.read.max_ms"),
            self.get("record.bytes"),
            self.get("store.bytes_per_minute") / 1e6,
        );
        if self.metrics.iter().any(|m| m.0 == "video.fps") {
            line.push_str(&format!(
                ", video {:.0} frames/s ({:.2} MB per minute)",
                self.get("video.fps"),
                self.get("video.bytes_per_minute") / 1e6
            ));
        }
        if let Some(d) = &self.diverged {
            line.push_str(&format!("\n  FAILED: {d}"));
        }
        line
    }

    fn json(&self, file: &Path, key: &str) -> Value {
        let metrics: serde_json::Map<String, Value> = self
            .metrics
            .iter()
            .map(|(n, v, _)| (format!("{key}.{n}"), json!(v)))
            .collect();
        json!({
            "file": file,
            "result": if self.diverged.is_some() { "fail" } else { "ok" },
            "frames": self.frames,
            "metrics": metrics,
            "divergence": self.diverged,
        })
    }
}

/// The board's frames a second, for bytes per recorded minute on a romset.
const ARCADE_FPS: f64 = 30.0;

fn bench_one(b: &Baseline, key: &str, rom: &[u8], req: &Request<'_>) -> Result<Benched> {
    if gp_arcade::romset::looks_like_zip(rom) {
        return bench_arcade(b, key, &gp_arcade::Romset::from_zip(rom)?, req);
    }
    let _span = tracing::info_span!("bench", key).entered();
    let input = b.input()?;
    let image = identify_nes(rom)?;
    let tmp = tempfile::tempdir().context("creating a temporary store")?;
    let store = Store::open(tmp.path().join("store"))?;
    let power_on = PowerOn::default();
    let recorded = recording::record_with(
        &store,
        RecordRequest {
            rom,
            source: SourceRef::NesRom {
                sha256: image.sha256.clone(),
                mapper: image.header.mapper,
            },
            source_id: b.source.clone(),
            power_on,
            input: &input,
            checkpoint_interval: 60,
            provenance: Provenance::Scripted,
            name: key.to_owned(),
            created_at: "1970-01-01T00:00:00Z".into(),
            notes: None,
        },
        Capture::ALL,
    )?;
    let manifest = &recorded.manifest;
    let frames = manifest.frames;
    let minutes = frames as f64 / console_fps(power_on.region) / 60.0;
    let mut metrics = vec![
        ("record.fps", recorded.frames_per_second, "frames/s"),
        ("record.bytes", recorded.bytes as f64, "bytes"),
        (
            "store.bytes_per_minute",
            recorded.bytes as f64 / minutes,
            "bytes",
        ),
    ];

    let replayed = recording::replay(&store, manifest, rom, None)?;
    metrics.push(("replay.fps", replayed.frames_per_second, "frames/s"));
    let mut diverged = replayed.divergence.map(|d| {
        format!(
            "replay diverged at frame {} ({})",
            d.frame,
            d.what.describe()
        )
    });

    let (mut seek_max, mut seek_sum, mut trace_max) = (0f64, 0f64, 0f64);
    let targets = seek_frames(frames, 60, req.seeks);
    for &f in &targets {
        let t = Instant::now();
        let cursor = gp_nes::seek::open(&store, manifest, rom, f)?;
        let took = ms(t);
        seek_max = seek_max.max(took);
        seek_sum += took;
        if diverged.is_none() && !cursor.matches_recording()? {
            diverged = Some(format!("seeking to frame {f} reached another state"));
        }
        let t = Instant::now();
        recording::trace_state(&store, manifest, f)?;
        trace_max = trace_max.max(ms(t));
    }
    metrics.push(("seek.max_ms", seek_max, "ms"));
    metrics.push(("seek.mean_ms", seek_sum / targets.len() as f64, "ms"));
    metrics.push(("trace.read.max_ms", trace_max, "ms"));

    Ok(Benched {
        frames,
        metrics,
        diverged,
    })
}

/// [`bench_one`] on an arcade romset: the holds in the board's layout,
/// recorded, replayed and sought on gp-arcade.
fn bench_arcade(
    b: &Baseline,
    key: &str,
    set: &gp_arcade::Romset,
    req: &Request<'_>,
) -> Result<Benched> {
    let spans: Vec<&str> = b.holds.iter().map(String::as_str).collect();
    let input = gp_evidence::input_log::generate_arcade(
        b.frames,
        &crate::explore_z80::layout(set),
        &spans,
    )?;
    let tmp = tempfile::tempdir().context("creating a temporary store")?;
    let store = Store::open(tmp.path().join("store"))?;
    let recorded = crate::arcade_rec::record(
        &store,
        crate::arcade_rec::Request {
            set,
            source_id: b.source.clone(),
            input: &input,
            checkpoint_interval: 60,
            capture: Capture::ALL,
            provenance: Provenance::Scripted,
            name: key.to_owned(),
            created_at: "1970-01-01T00:00:00Z".into(),
            notes: None,
        },
    )?;
    let manifest = &recorded.manifest;
    let frames = manifest.frames;
    let minutes = frames as f64 / ARCADE_FPS / 60.0;
    let mut metrics = vec![
        ("record.fps", recorded.frames_per_second, "frames/s"),
        ("record.bytes", recorded.bytes as f64, "bytes"),
        (
            "store.bytes_per_minute",
            recorded.bytes as f64 / minutes,
            "bytes",
        ),
    ];
    let replayed = crate::arcade_rec::replay(&store, manifest, set, None)?;
    metrics.push(("replay.fps", replayed.frames_per_second, "frames/s"));
    let mut diverged = replayed.divergence.map(|d| {
        format!(
            "replay diverged at frame {} ({})",
            d.frame,
            crate::arcade_rec::describe(d.what)
        )
    });
    let (mut seek_max, mut seek_sum, mut trace_max) = (0f64, 0f64, 0f64);
    let targets = seek_frames(frames, 60, req.seeks);
    for &f in &targets {
        let t = Instant::now();
        let cursor = crate::arcade_rec::open(&store, manifest, set, f)?;
        let took = ms(t);
        seek_max = seek_max.max(took);
        seek_sum += took;
        if diverged.is_none() && !cursor.matches_recording() {
            diverged = Some(format!("seeking to frame {f} reached another state"));
        }
        let t = Instant::now();
        recording::trace_state(&store, manifest, f)?;
        trace_max = trace_max.max(ms(t));
    }
    metrics.push(("seek.max_ms", seek_max, "ms"));
    metrics.push(("seek.mean_ms", seek_sum / targets.len() as f64, "ms"));
    metrics.push(("trace.read.max_ms", trace_max, "ms"));
    Ok(Benched {
        frames,
        metrics,
        diverged,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeks_go_to_the_frames_before_checkpoints() {
        assert_eq!(seek_frames(300, 60, 8), [59, 119, 179, 239, 299, 300]);
        assert_eq!(seek_frames(1800, 60, 3), [59, 899, 1799, 1800]);
        assert_eq!(seek_frames(30, 60, 8), [30]);
        assert_eq!(seek_frames(59, 60, 8), [59]);
    }
}
