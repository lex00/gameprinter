//! A coverage report over a set of recordings: each recording measured or
//! read from the store, then combined per ROM. [`build`] returns what
//! `gp coverage --json` prints, so anything else that serves coverage (the
//! inspector's endpoint) gives the same shape.

use std::collections::BTreeMap;
use std::time::Instant;

use gp_evidence::recordings::IndexEntry;
use gp_evidence::{Hash, Store};
use serde::{Deserialize, Serialize};

use crate::definition::Definition;
use crate::measure::{self, MeasureError};
use crate::{COVERAGE_VERSION, Check, Coverage, Summary, cache};

/// What a report needs from its caller: ROMs and definitions, found by the
/// ROM's headerless SHA-256.
pub trait Env {
    /// The ROM's bytes, or `None` if it isn't available.
    fn rom(&mut self, sha256: &str) -> Result<Option<Vec<u8>>, String>;
    /// The registry's source ID and title for the ROM, if it's registered.
    fn source(&mut self, sha256: &str) -> Option<(String, String)>;
    /// The coverage definition to use for the ROM, if there is one.
    fn definition(
        &mut self,
        sha256: &str,
        source_id: Option<&str>,
    ) -> Result<Option<Definition>, String>;
}

#[derive(Debug, Clone)]
pub struct Options {
    /// Store newly measured coverage, so the next report reads it.
    pub save: bool,
    pub threads: usize,
    /// RFC 3339, for the index lines.
    pub now: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub coverage_version: u32,
    /// One per ROM, with the coverage of every recording of it that was
    /// measured.
    pub groups: Vec<Group>,
    /// Every recording asked for, and what happened to it.
    pub recordings: Vec<RecordingResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Group {
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The definition file, as given or found.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition: Option<String>,
    pub summary: Summary,
    /// The definition's thresholds, checked, if it has any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check: Option<Check>,
    pub coverage: Coverage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    /// Replayed and measured now.
    Measured,
    /// Read from the store, measured earlier.
    Stored,
    /// Not measured: its ROM isn't available, or another runtime made it.
    Skipped,
    /// The replay didn't reach the recorded state.
    Diverged,
    /// Couldn't be measured.
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingResult {
    pub id: Hash,
    pub name: String,
    pub frames: u64,
    pub result: Outcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Time spent measuring, for a measured recording. Left out of the
    /// JSON, so the same report always serializes the same way.
    #[serde(skip)]
    pub ms: Option<u64>,
}

/// Measures `entries` (or reads their stored coverage) and combines them
/// per ROM. Recordings that fail are reported in `recordings` and left out
/// of the groups.
pub fn build(
    store: &Store,
    entries: &[IndexEntry],
    env: &mut dyn Env,
    opts: &Options,
) -> Result<Report, String> {
    let mut results = Vec::new();
    let mut by_rom: BTreeMap<String, Vec<Coverage>> = BTreeMap::new();
    let mut definitions: BTreeMap<String, Option<Definition>> = BTreeMap::new();
    let mut sources: BTreeMap<String, Option<(String, String)>> = BTreeMap::new();
    for entry in entries {
        let mut result = RecordingResult {
            id: entry.id,
            name: entry.name.clone(),
            frames: entry.frames,
            result: Outcome::Error,
            message: None,
            ms: None,
        };
        match one(store, entry, env, opts, &mut definitions, &mut sources) {
            Ok((outcome, Some(cov), ms)) => {
                result.result = outcome;
                result.ms = ms;
                by_rom.entry(cov.sha256.clone()).or_default().push(cov);
            }
            Ok((outcome, None, _)) => result.result = outcome,
            Err((outcome, message)) => {
                result.result = outcome;
                result.message = Some(message);
            }
        }
        if result.result == Outcome::Skipped && result.message.is_none() {
            result.message = Some("no ROM with its hash".into());
        }
        results.push(result);
    }
    let mut groups = Vec::new();
    for (sha256, covs) in by_rom {
        let coverage = Coverage::union(&covs)?;
        let def = definitions.get(&sha256).cloned().flatten();
        let (source, title) = sources
            .get(&sha256)
            .cloned()
            .flatten()
            .map_or((None, None), |(s, t)| (Some(s), Some(t)));
        groups.push(Group {
            summary: coverage.summary(),
            check: def
                .as_ref()
                .and_then(|d| d.thresholds.as_ref())
                .map(|t| coverage.check(t)),
            definition: def.as_ref().map(|d| d.path.display().to_string()),
            title: title.or_else(|| def.as_ref().map(|d| d.title.clone())),
            source: source.or_else(|| def.as_ref().map(|d| d.source.clone())),
            sha256,
            coverage,
        });
    }
    Ok(Report {
        coverage_version: COVERAGE_VERSION,
        groups,
        recordings: results,
    })
}

type One = (Outcome, Option<Coverage>, Option<u64>);

fn one(
    store: &Store,
    entry: &IndexEntry,
    env: &mut dyn Env,
    opts: &Options,
    definitions: &mut BTreeMap<String, Option<Definition>>,
    sources: &mut BTreeMap<String, Option<(String, String)>>,
) -> Result<One, (Outcome, String)> {
    let error = |e: String| (Outcome::Error, e);
    let manifest = gp_evidence::recordings::load_manifest(store, &entry.id)
        .map_err(|e| error(e.to_string()))?;
    if manifest.runtime.name != "gp-nes" {
        return Err((
            Outcome::Skipped,
            format!(
                "made by {}; coverage replays recordings in gp-nes",
                manifest.runtime.name
            ),
        ));
    }
    let Some(sha256) = manifest.source.nes_sha256().map(str::to_owned) else {
        return Err((
            Outcome::Skipped,
            "a model recording; coverage replays recordings of ROMs".into(),
        ));
    };
    let sha256 = &sha256;
    let source = sources
        .entry(sha256.clone())
        .or_insert_with(|| env.source(sha256))
        .clone();
    if !definitions.contains_key(sha256) {
        let def = env
            .definition(sha256, source.as_ref().map(|(id, _)| id.as_str()))
            .map_err(error)?;
        definitions.insert(sha256.clone(), def);
    }
    let def = definitions[sha256].as_ref();
    let def_hash = def.map(|d| d.hash);
    if let Some(cov) = cache::lookup(store, entry.id, def_hash).map_err(|e| error(e.to_string()))? {
        return Ok((Outcome::Stored, Some(cov), None));
    }
    let Some(rom) = env.rom(sha256).map_err(error)? else {
        return Ok((Outcome::Skipped, None, None));
    };
    let start = Instant::now();
    let cov = measure::measure(store, entry.id, &manifest, &rom, def, opts.threads).map_err(
        |e| match e {
            MeasureError::Diverged { .. } => (Outcome::Diverged, e.to_string()),
            e => error(e.to_string()),
        },
    )?;
    let ms = start.elapsed().as_millis() as u64;
    if opts.save {
        cache::save(store, &cov, &opts.now).map_err(|e| error(e.to_string()))?;
    }
    Ok((Outcome::Measured, Some(cov), Some(ms)))
}
