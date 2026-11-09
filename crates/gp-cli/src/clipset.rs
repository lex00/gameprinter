//! Clip recipes: a clip as a start and presses that anyone can make again
//! from the ROM, with no snapshot and no game data, kept in a game's
//! `clips.toml`.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use anyhow::{Context as _, bail};
use serde::{Deserialize, Serialize};

/// The clip set format's version.
pub(crate) const FORMAT: &str = "0.5.0";

/// Whether this gp reads a file of format `v`: the same major and minor,
/// or an older one, which each version only adds to.
fn readable(v: &str) -> bool {
    let mm = |v: &str| {
        let mut p = v.split('.').map(|x| x.parse::<u32>().ok());
        Some((p.next()??, p.next()??))
    };
    matches!((mm(v), mm(FORMAT)), (Some(a), Some(b)) if a.0 == b.0 && a.1 <= b.1)
}

/// The kept clip set in a game's folder.
pub(crate) const FILE: &str = "clips.toml";
/// The first line of a `.presses` file.
const PRESSES_HEAD: &str = "gp-presses 0";

/// What a clip set was made from.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Header {
    /// [`FORMAT`].
    pub format: String,
    /// The template's ID.
    #[serde(default)]
    pub game: String,
    /// The ROM's SHA-256.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub rom: String,
    /// What made the clips: gp's version and the corpus generator's.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub generator: String,
    /// The state mapping's schema the curves were measured with.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub mapping: String,
    /// The model the set was chosen on (informative).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
    /// The run it was chosen from (informative).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub run: String,
}

/// Where a clip starts: a frame of presses from power-on (a save point's,
/// in a `.presses` file beside the set), or of a recording, which this gp
/// has none of.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct StartRecipe {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recording: Option<String>,
    /// A `.presses` file of spans from power-on, relative to the set's
    /// folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presses_from_power_on: Option<String>,
    /// The original's frame.
    pub frame: u64,
    /// The model's frame there, as the lockstep counts it: checked when
    /// the start is made again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_frame: Option<u64>,
    /// The start's kind: `level_start`, `pit`, `wall`, a corpus context.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub context: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub level: String,
}

/// One clip.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Recipe {
    pub id: String,
    pub name: String,
    /// `generated`, `played` (a person's, from a save point), `aimed` or
    /// `dense` (chosen for the original's code it runs).
    #[serde(default = "generated")]
    pub source: String,
    /// Button spans from the start, `right+b@0-59`.
    #[serde(default)]
    pub presses: Vec<String>,
    /// A `.presses` file in place of `presses`, for a long one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presses_file: Option<String>,
    pub frames: u64,
    /// The hash of the original's mapped measures on every frame.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub curve: String,
    /// For a dense clip: the hash of the original's code map over the clip.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub coverage: String,
    /// What it exercises (the prune's parts).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub holds: Vec<String>,
    /// The mutants it catches.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub guards: Vec<String>,
    /// For an aimed clip: the mutants it is aimed at.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aims: Vec<String>,
    /// Why it is kept or made, in game words.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub why: String,
    /// The fixes found on it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub found: Vec<String>,
    /// Clips the queue needs to pass first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<String>,
    pub start: StartRecipe,
}

fn generated() -> String {
    "generated".into()
}

/// A clip set file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct ClipSet {
    pub clipset: Header,
    #[serde(default)]
    pub clips: Vec<Recipe>,
}

/// A clip set read from `file`, its format checked: same major and minor,
/// or an older one, which each version only adds to. A clip that plays a
/// sealed run is left out, as every reader of a set must, when this
/// program has a guard installed.
pub(crate) fn read(file: &Path) -> anyhow::Result<ClipSet> {
    let text = fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
    let mut set = parse(&text, file)?;
    let dir = file.parent().unwrap_or(Path::new("."));
    let before = set.clips.len();
    let of = gp_evidence::guard::Of::game(&set.clipset.game);
    set.clips.retain(|r| match recipe_log(dir, r) {
        Ok(Some((log, start))) => {
            gp_evidence::guard::input(&format!("clip {}", r.id), &of, &log, start).is_ok()
        }
        _ => true,
    });
    let left = before - set.clips.len();
    if left > 0 {
        eprintln!(
            "gp: {}: left out {left} clip{} that play{} a sealed run",
            file.display(),
            if left == 1 { "" } else { "s" },
            if left == 1 { "s" } else { "" },
        );
    }
    Ok(set)
}

/// A clip set's text, checked as [`read`] checks a file; `file` names it
/// in errors.
pub(crate) fn parse(text: &str, file: &Path) -> anyhow::Result<ClipSet> {
    let set: ClipSet =
        toml::from_str(text).with_context(|| format!("{} isn't a clip set", file.display()))?;
    if !readable(&set.clipset.format) {
        bail!(
            "{} is clip set format {}, this gp reads {FORMAT}",
            file.display(),
            set.clipset.format
        );
    }
    let mut seen = BTreeSet::new();
    for c in &set.clips {
        if !seen.insert(c.id.as_str()) {
            bail!("{}: clip {} is listed twice", file.display(), c.id);
        }
        if c.start.recording.is_some() == c.start.presses_from_power_on.is_some() {
            bail!(
                "{}: clip {} names neither or both of a recording and presses from power-on",
                file.display(),
                c.id
            );
        }
        for p in [&c.start.presses_from_power_on, &c.presses_file]
            .into_iter()
            .flatten()
        {
            if !inside(p) {
                bail!(
                    "{}: clip {}: {p} isn't inside the set's folder",
                    file.display(),
                    c.id
                );
            }
        }
    }
    Ok(set)
}

/// A relative path that stays in its folder.
fn inside(p: &str) -> bool {
    let path = Path::new(p);
    path.is_relative()
        && path
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
}

pub(crate) fn parse_presses(text: &str) -> anyhow::Result<(u64, Vec<String>)> {
    let mut lines = text.lines();
    if lines.next() != Some(PRESSES_HEAD) {
        bail!("a presses file starts with {PRESSES_HEAD:?}");
    }
    let frames = lines
        .next()
        .and_then(|l| l.strip_prefix("frames "))
        .and_then(|n| n.trim().parse::<u64>().ok())
        .context("a presses file's second line is `frames N`")?;
    let spans: Vec<String> = lines
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect();
    Ok((frames, spans))
}

/// Spans as an input log's text, `frames` long.
pub(crate) fn log_of(frames: u64, spans: &[String]) -> anyhow::Result<String> {
    let refs: Vec<&str> = spans.iter().map(String::as_str).collect();
    Ok(
        gp_evidence::input_log::generate(usize::try_from(frames).unwrap_or(usize::MAX), &refs)?
            .to_text(),
    )
}

/// A name usable in a file name.
pub(crate) fn file_key(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// A clip's input from power-on, its presses to its start then its own,
/// and its start; `None` for a clip that starts from a recording or whose
/// presses file is missing.
pub(crate) fn recipe_log(
    set_dir: &Path,
    r: &Recipe,
) -> anyhow::Result<Option<(gp_evidence::InputLog, usize)>> {
    let Some(p) = &r.start.presses_from_power_on else {
        return Ok(None);
    };
    let Ok(text) = fs::read_to_string(set_dir.join(p)) else {
        return Ok(None);
    };
    let (frames, spans) = parse_presses(&text)?;
    let mut log = gp_evidence::InputLog::parse(&log_of(frames, &spans)?)?;
    let start = usize::try_from(r.start.frame).unwrap_or(usize::MAX);
    log.frames.truncate(start);
    let start = log.frames.len();
    let presses = match &r.presses_file {
        Some(p) => parse_presses(&fs::read_to_string(set_dir.join(p))?)?.1,
        None => r.presses.clone(),
    };
    let own = gp_evidence::InputLog::parse(&log_of(r.frames, &presses)?)?;
    log.frames.extend(own.frames);
    Ok(Some((log, start)))
}
