//! Clip recipes: a clip as a start and presses that anyone can make again
//! from the ROM, with no snapshot and no game data, kept in a game's
//! `clips.toml`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, bail};
use gp_evidence::Paths;
use gp_evidence::manifest::PowerOn;
use gp_interp::mapping::Mapping;
use gp_interp::scenario::{Outcome as Ran, Scenario, Source, StartAt};
use gp_nes::protocol::NesRuntime;
use gp_runtime::Runtime as _;
use serde::{Deserialize, Serialize};
use serde_json::json;

/// The clip set format's version.
pub(crate) const FORMAT: &str = "0.5.0";

/// Whether this gp reads a file of format `v`: the same major and minor,
/// or an older one, which each version only adds to.
pub(crate) fn readable(v: &str) -> bool {
    let mm = |v: &str| {
        let mut p = v.split('.').map(|x| x.parse::<u32>().ok());
        Some((p.next()??, p.next()??))
    };
    matches!((mm(v), mm(FORMAT)), (Some(a), Some(b)) if a.0 == b.0 && a.1 <= b.1)
}

/// The kind of a dense clip (no game has one yet; a set with one is refused).
pub(crate) const DENSE: &str = "dense";
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
    /// Named recordings starts are taken from; carried through unread, as
    /// `mutant` and `import` are, so a set this gp rewrites keeps them.
    #[serde(default, skip_serializing_if = "toml::Table::is_empty")]
    pub recordings: toml::Table,
    #[serde(default)]
    pub clips: Vec<Recipe>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mutant: Vec<toml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub import: Option<toml::Value>,
    /// Read from a corpus plan ([`crate::plan`]): its clips have no curves
    /// to compare with, the curves are derived.
    #[serde(skip)]
    pub plan: bool,
}

/// A clip set read from `file`, its format checked: same major and minor,
/// or an older one, which each version only adds to. A clip that plays a
/// sealed run is left out, as every reader of a set must, when this
/// program has a guard installed.
pub(crate) fn read(file: &Path) -> anyhow::Result<ClipSet> {
    let text = fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
    let set = parse(&text, file)?;
    let dir = file.parent().unwrap_or(Path::new("."));
    Ok(without_sealed(set, dir, file))
}

/// The clip set of the game in `dir`: its corpus plan when `corpus.toml` is
/// one, else its `clips.toml`.
pub(crate) fn read_game(dir: &Path) -> anyhow::Result<ClipSet> {
    if let Some(p) = crate::plan::load(dir)? {
        return Ok(without_sealed(
            p.set,
            dir,
            &dir.join(crate::corpus_score::FILE),
        ));
    }
    read(&dir.join(FILE))
}

/// `set` with any clip that plays a sealed run left out, as every reader of
/// a set must, when this program has a guard installed.
fn without_sealed(mut set: ClipSet, dir: &Path, file: &Path) -> ClipSet {
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
    set
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
pub(crate) fn inside(p: &str) -> bool {
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

/// A `.presses` file: its frames and spans.
pub(crate) fn presses_text(frames: u64, spans: &[String]) -> String {
    let mut s = format!("{PRESSES_HEAD}\nframes {frames}\n");
    for p in spans {
        s.push_str(p);
        s.push('\n');
    }
    s
}

/// A clip's recipe hash: the BLAKE3 of what makes it and what it must
/// give, as JSON (docs/formats/scoring-corpus.md): its ID, its start (the
/// recording, or the hash of the presses from power-on, the frame and the
/// model's frame), its presses (from the `.presses` file when it has one),
/// its frames and its curve, and for a dense clip its coverage hash (left
/// out when there is none, so a set without dense clips hashes as it did
/// under 0.2). Names, reasons, parts and guards are metadata and aren't
/// in it. `file` reads a path beside the set.
pub(crate) fn recipe_hash(
    r: &Recipe,
    file: &dyn Fn(&str) -> anyhow::Result<String>,
) -> anyhow::Result<String> {
    let presses = match &r.presses_file {
        Some(p) => parse_presses(&file(p)?)?.1,
        None => r.presses.clone(),
    };
    let from = match &r.start.presses_from_power_on {
        Some(p) => {
            let (frames, spans) = parse_presses(&file(p)?)?;
            Some(gp_evidence::Hash::of(presses_text(frames, &spans).as_bytes()).to_string())
        }
        None => None,
    };
    let mut v = json!({
        "id": r.id,
        "start": {
            "recording": r.start.recording,
            "presses_from_power_on": from,
            "frame": r.start.frame,
            "model_frame": r.start.model_frame,
        },
        "presses": presses,
        "frames": r.frames,
        "curve": r.curve,
    });
    if !r.coverage.is_empty() {
        v["coverage"] = json!(r.coverage);
    }
    Ok(gp_evidence::Hash::of(&serde_json::to_vec(&v)?).to_string())
}

/// A clip set's content hash, over its clips' recipe hashes by ID: the
/// same clips give the same hash whatever their order or metadata.
pub(crate) fn content_hash(recipes: &BTreeMap<String, String>) -> String {
    let mut s = String::from("gp-corpus 1\n");
    for (id, h) in recipes {
        s.push_str(id);
        s.push(' ');
        s.push_str(h);
        s.push('\n');
    }
    gp_evidence::Hash::of(s.as_bytes()).to_string()
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

fn rel(home: &Path, p: &Path) -> String {
    p.strip_prefix(home)
        .unwrap_or(p)
        .to_string_lossy()
        .into_owned()
}

/// `stem` with `.ext` added: a stem's own dots are kept.
fn with_ext(stem: &Path, ext: &str) -> PathBuf {
    let mut s = stem.as_os_str().to_owned();
    s.push(".");
    s.push(ext);
    PathBuf::from(s)
}

/// A start taken: where its snapshot is, its frames, and the picture.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Taken {
    /// The original's frame (the first that reads the controller at or
    /// after the one asked for).
    pub frame: u64,
    pub model_frame: u64,
    /// Relative to `$GP_HOME`.
    pub snapshot: String,
    /// The input log's BLAKE3, so a changed log takes the start again.
    pub log: String,
}

/// The original at frame `frame` of `log_text`, its snapshot kept at
/// `stem.snap` with `stem.json` saying the frames; made once, read after.
pub(crate) fn take(
    home: &Path,
    rom: &[u8],
    mapping: &gp_interp::mapping::Mapping,
    log_text: &str,
    frame: u64,
    stem: &Path,
) -> anyhow::Result<Taken> {
    let log_hash = gp_evidence::Hash::of(log_text.as_bytes()).to_string();
    let meta = with_ext(stem, "json");
    if let Some(t) = fs::read(&meta)
        .ok()
        .and_then(|b| serde_json::from_slice::<Taken>(&b).ok())
        && t.log == log_hash
        && home.join(&t.snapshot).is_file()
    {
        return Ok(t);
    }
    let log = gp_evidence::InputLog::parse(log_text)?;
    let inputs = crate::run::frames(&log);
    // Found with the video off, which is much faster, then played again
    // with the video on for the last frame, so the picture there is drawn
    // as a start's must be for the screen measure. Both from the start
    // cache's console a few frames before `frame` when it has one.
    let (original, from, count) = scan_start(home, rom, mapping, &log, &inputs, frame)?;
    let (found, model_frame) = gp_interp::corpus::scan::at_frame_from(
        original.fork(),
        mapping,
        &inputs,
        from,
        count,
        frame,
    )
    .map_err(anyhow::Error::msg)?;
    let at = usize::try_from(found.frame()).unwrap_or(usize::MAX);
    let mut o = original.fork();
    for (k, i) in inputs[..at.min(inputs.len())].iter().enumerate().skip(from) {
        if k + 1 == at {
            o.set_video(true);
        }
        o.step(std::slice::from_ref(i))
            .map_err(anyhow::Error::msg)?;
    }
    o.set_video(true);
    if o.frame() != found.frame() {
        bail!(
            "the original's frame count isn't its inputs' at frame {}",
            found.frame()
        );
    }
    if let Some(p) = stem.parent() {
        fs::create_dir_all(p)?;
    }
    let snap = with_ext(stem, "snap");
    fs::write(&snap, o.snapshot())?;
    let t = Taken {
        frame: o.frame(),
        model_frame,
        snapshot: rel(home, &snap),
        log: log_hash,
    };
    fs::write(&meta, serde_json::to_vec_pretty(&t)?)?;
    Ok(t)
}

/// Where [`take`] starts looking for its frame: the original from the
/// start cache ([`crate::clip::cache`]) [`crate::clip::cache::START_LEAD`]
/// frames before `frame`, with the scan's count there kept beside it
/// under the mapping's start condition (made on first use from power-on);
/// else powered on. The input frames it stands after, and the count.
fn scan_start(
    home: &Path,
    rom: &[u8],
    mapping: &gp_interp::mapping::Mapping,
    log: &gp_evidence::InputLog,
    inputs: &[gp_runtime::InputFrame],
    frame: u64,
) -> anyhow::Result<(NesRuntime, usize, gp_interp::corpus::scan::ScanAt)> {
    use crate::clip::cache;
    use gp_interp::corpus::scan::{ScanAt, at_frame_prefix};
    let power_on =
        || NesRuntime::load(rom, PowerOn::default()).map_err(|e| anyhow::anyhow!("the ROM: {e}"));
    let k = usize::try_from(frame)
        .unwrap_or(usize::MAX)
        .saturating_sub(cache::START_LEAD)
        .min(inputs.len());
    if k == 0 || !cache::starts_on() {
        return Ok((power_on()?, 0, ScanAt::default()));
    }
    let key = cache::start_key(rom, &PowerOn::default(), &log.frames, k);
    let tag = format!("scan at_frame 1\nstart {:?}\n", mapping.start);
    let note =
        cache::read_note(home, &key, &tag).and_then(|b| serde_json::from_slice::<ScanAt>(&b).ok());
    let runtime = |s: &cache::Saved| -> anyhow::Result<NesRuntime> {
        NesRuntime::new(s.nes(rom, PowerOn::default())?).map_err(anyhow::Error::msg)
    };
    if let Some(count) = note
        && let Some(saved) = cache::read_start(home, &key, rom, PowerOn::default())
    {
        return Ok((runtime(&saved)?, k, count));
    }
    let mut o = power_on()?;
    let Ok(count) = at_frame_prefix(&mut o, mapping, inputs, k, frame) else {
        return Ok((power_on()?, 0, ScanAt::default()));
    };
    let saved = cache::write_start(home, &key, o.nes())?;
    cache::write_note(home, &key, &tag, &serde_json::to_vec(&count)?);
    Ok((runtime(&saved)?, k, count))
}

/// A save point's start ([`take_on`]): where it is taken, and the clip's
/// own presses and frames from there.
pub(crate) struct TakenOn {
    pub taken: Taken,
    /// The frames the start moved past the recipe's, cut from the front of
    /// the clip's own presses; 0 when the recipe's frame reads the
    /// controller.
    pub shift: u64,
    /// The input log from power-on to the start, as text.
    pub start_log: String,
    pub presses: Vec<String>,
    pub frames: u64,
}

/// A save point's start: [`take`] on its presses from power-on (`log_text`)
/// at `frame`. When they end before a frame that reads the controller, as a
/// clip from power-on's do (frame 0, no presses: the game reads the
/// controller a few frames in), the clip's own presses (`presses`, `frames`
/// long) are played on to the first frame that does, as the mechanic page
/// scores such a clip (`crate::clip_score`, the lockstep from its start).
/// The clip starts there: the frames before it move from the front of its
/// presses to the end of the start's input, so the original plays the same
/// input to the same last frame.
// Each argument is one input the caller already holds; the command's own
// plumbing, with no group of them that belongs together.
#[allow(clippy::too_many_arguments)]
pub(crate) fn take_on(
    home: &Path,
    rom: &[u8],
    mapping: &gp_interp::mapping::Mapping,
    log_text: &str,
    presses: &[String],
    frames: u64,
    frame: u64,
    stem: &Path,
) -> anyhow::Result<TakenOn> {
    let first = match take(home, rom, mapping, log_text, frame, stem) {
        Ok(t) => {
            return Ok(TakenOn {
                taken: t,
                shift: 0,
                start_log: log_text.to_owned(),
                presses: presses.to_vec(),
                frames,
            });
        }
        Err(e) => e,
    };
    let start = gp_evidence::InputLog::parse(log_text)?;
    let own = gp_evidence::InputLog::parse(&log_of(frames, presses)?)?;
    // Only a start whose presses end at it goes on into the clip's own.
    if start.frames.len() as u64 != frame || own.frames.is_empty() {
        return Err(first);
    }
    let mut joined = gp_evidence::InputLog::new(start.controllers.max(own.controllers));
    joined.frames = start.frames.clone();
    joined.frames.extend(own.frames.iter().cloned());
    let t = take(home, rom, mapping, &joined.to_text(), frame, stem).map_err(|e| {
        anyhow::anyhow!(
            "{first:#}; played on into the clip's own {} frames: {e:#}",
            own.frames.len()
        )
    })?;
    let shift = t.frame - frame;
    let cut = usize::try_from(shift).unwrap_or(usize::MAX);
    if cut >= own.frames.len() {
        bail!(
            "the clip's frames end at frame {} before one that reads the controller",
            frame + own.frames.len() as u64
        );
    }
    let mut start_log = joined.clone();
    start_log.frames.truncate(start.frames.len() + cut);
    let rest = &own.frames[cut..];
    let presses = gp_evidence::input_log::spans_of(rest)
        .context("the clip presses reset or a third or fourth controller, which spans can't say")?;
    Ok(TakenOn {
        taken: t,
        shift,
        start_log: start_log.to_text(),
        presses,
        frames: rest.len() as u64,
    })
}

/// A scored clip's start and presses, made from its recipe: the scenario the
/// scorer runs.
pub(crate) struct Made {
    pub scenario: Scenario,
}

/// Makes `recipes` (from the set in folder `set_dir`) into scenarios of the
/// run in `dir`: each start's input played from power-on on the original
/// once, its snapshot kept in the run's `generated/starts/`. Each recipe's
/// result, in order. A clip that starts from a recording is refused: no
/// such clip exists in the games that have moved so far.
pub(crate) fn make(
    home: &Path,
    dir: &Path,
    set_dir: &Path,
    rom: &[u8],
    mapping: &Mapping,
    recipes: &[Recipe],
) -> anyhow::Result<Vec<Result<Made, String>>> {
    let gdir = dir.join("generated");
    let logs = gdir.join("logs");
    let sdir = gdir.join("starts");
    fs::create_dir_all(&logs)?;
    // The save points' starts on the helper threads: each plays its
    // presses from power-on once, and the clips after read them back.
    let mut saves: Vec<(String, u64, PathBuf)> = Vec::new();
    for r in recipes {
        let Some(p) = &r.start.presses_from_power_on else {
            continue;
        };
        let Ok(text) = fs::read_to_string(set_dir.join(p)) else {
            continue;
        };
        let Ok((frames, spans)) = parse_presses(&text) else {
            continue;
        };
        let Ok(log) = log_of(frames, &spans) else {
            continue;
        };
        saves.push((
            log,
            r.start.frame,
            sdir.join(format!("recipe.save.{}", file_key(&r.id))),
        ));
    }
    let _ = crate::pool::map(
        &saves,
        || (),
        |_, _, (log, frame, stem)| take(home, rom, mapping, log, *frame, stem).is_ok(),
        &mut |_| {},
    );
    let mut out = Vec::new();
    for r in recipes {
        out.push(
            make_one(home, set_dir, rom, mapping, &logs, &sdir, r).map_err(|e| format!("{e:#}")),
        );
    }
    Ok(out)
}

fn make_one(
    home: &Path,
    set_dir: &Path,
    rom: &[u8],
    mapping: &Mapping,
    logs: &Path,
    sdir: &Path,
    r: &Recipe,
) -> anyhow::Result<Made> {
    if r.source == DENSE {
        bail!(
            "clip {} is a dense clip, which this gp can't make yet",
            r.id
        );
    }
    let (Some(p), None) = (&r.start.presses_from_power_on, &r.start.recording) else {
        bail!(
            "clip {} starts from a recording, which this gp can't make",
            r.id
        );
    };
    let text = fs::read_to_string(set_dir.join(p))
        .with_context(|| format!("reading {p} beside the clip set"))?;
    let (frames, spans) = parse_presses(&text)?;
    let log_file = logs.join(format!("save.{}.log", file_key(&r.id)));
    fs::write(&log_file, log_of(frames, &spans)?)?;
    let input = format!("save point {}", r.id);
    let stem = sdir.join(format!("recipe.save.{}", file_key(&r.id)));
    let log_text = fs::read_to_string(&log_file)?;
    let presses = match &r.presses_file {
        Some(p) => {
            parse_presses(
                &fs::read_to_string(set_dir.join(p))
                    .with_context(|| format!("reading {p} beside the clip set"))?,
            )?
            .1
        }
        None => r.presses.clone(),
    };
    let o = take_on(
        home,
        rom,
        mapping,
        &log_text,
        &presses,
        r.frames,
        r.start.frame,
        &stem,
    )?;
    if o.shift > 0 {
        fs::write(&log_file, &o.start_log)?;
    }
    let (t, presses, frames, shift) = (o.taken, o.presses, o.frames, o.shift);
    if t.frame != r.start.frame + shift {
        bail!(
            "frame {} of {input} doesn't read the controller; the next that does is {}",
            r.start.frame,
            t.frame
        );
    }
    if let Some(mf) = r.start.model_frame
        && mf != t.model_frame
    {
        bail!(
            "the model's frame at the start is {} here, the recipe says {mf}",
            t.model_frame
        );
    }
    let played = r.source == "played";
    Ok(Made {
        scenario: Scenario {
            id: r.id.clone(),
            name: r.name.clone(),
            tier: if played { 0 } else { 3 },
            mechanic: String::new(),
            params: Vec::new(),
            frames,
            presses,
            log: None,
            requires: r.requires.clone(),
            start: Some(StartAt {
                input,
                frame: t.frame,
                snapshot: Some(t.snapshot.clone()),
                log: Some(rel(home, &log_file)),
                model_frame: Some(t.model_frame),
            }),
            source: if played {
                Source::Played {
                    when: String::new(),
                    by: "free play".into(),
                }
            } else {
                Source::Generated
            },
        },
    })
}

/// The state mapping's schema in a model folder.
pub(crate) fn mapping_schema(mdir: &Path) -> String {
    fs::read_to_string(mdir.join("mapping.toml"))
        .ok()
        .and_then(|t| toml::from_str::<toml::Value>(&t).ok())
        .and_then(|v| v.get("schema").map(|s| s.to_string()))
        .unwrap_or_default()
}

/// A clip set made again into a run and scored once.
pub(crate) struct Regen {
    pub set: ClipSet,
    /// The run's state mapping's schema.
    pub mapping: String,
    pub differ: usize,
    pub measured: usize,
    pub failed: usize,
    /// Clips stopped at their first split, whose curves aren't compared.
    pub stopped: usize,
    /// Each clip's curve as measured now, by ID.
    pub curves: BTreeMap<String, String>,
    /// Each clip's score on the model, by ID.
    pub outcomes: BTreeMap<String, Result<Ran, String>>,
    /// The bench the clips were scored on, and the model's text.
    pub bench: crate::scenario::Bench,
    pub model_text: String,
}

/// How [`regen_with`] scores: whether each clip's reads are recorded, and
/// the clips to stop at their first split.
#[derive(Default)]
pub(crate) struct Scoring {
    pub reads: bool,
    pub stop: BTreeSet<String>,
}

/// Where a [`regen_with`] finds what it needs: the run, the model and its
/// assets, the ROM's bytes and the hosted model.
pub(crate) struct Work<'a> {
    pub paths: &'a Paths,
    pub run_dir: &'a Path,
    pub run: &'a str,
    pub mdir: &'a Path,
    pub assets: &'a Path,
    pub rom: Vec<u8>,
    pub source: Arc<crate::hosted::Source>,
}

/// The bench clips are scored on: the model, its assets and the ROM. A
/// clip brings its own input from power-on, so the bench's is empty.
pub(crate) fn bench_for(w: &Work) -> anyhow::Result<crate::scenario::Bench> {
    crate::scenario::Bench::new(
        w.mdir,
        w.assets,
        w.rom.clone(),
        gp_evidence::InputLog::new(1).to_text(),
        "",
        &w.paths.home.join("work").join("regenerate").join(w.run),
        w.source.clone(),
    )
}

/// Every clip of the set in `set_dir` (those `only` names, when given) made
/// again from the ROM into the run, scored once on the model, and each
/// curve compared with the set's.
pub(crate) fn regen_with(
    w: &Work,
    set_dir: &Path,
    only: Option<&BTreeSet<String>>,
    how: &Scoring,
) -> anyhow::Result<Regen> {
    let set = read_game(set_dir)?;
    if let Some(d) = set.clips.iter().find(|r| r.source == DENSE) {
        bail!(
            "clip {} is a dense clip, which this gp can't score yet",
            d.id
        );
    }
    let sha = crate::roms::sha256_of(&w.rom)?;
    if !set.clipset.rom.is_empty() && set.clipset.rom != sha {
        bail!(
            "the ROM is {sha}, the clip set is for {}; give the ROM the package names",
            set.clipset.rom
        );
    }
    let mapping = mapping_schema(w.mdir);
    let same_mapping = set.clipset.mapping.is_empty() || set.clipset.mapping == mapping;
    let recipes: Vec<Recipe> = set
        .clips
        .iter()
        .filter(|r| only.is_none_or(|o| o.contains(&r.id)))
        .cloned()
        .collect();
    let mapping_parsed = Mapping::parse(
        &fs::read_to_string(w.mdir.join("mapping.toml"))
            .context("reading the model's state mapping (mapping.toml)")?,
    )
    .map_err(anyhow::Error::msg)?;
    let made = make(
        &w.paths.home,
        w.run_dir,
        set_dir,
        &w.rom,
        &mapping_parsed,
        &recipes,
    )?;
    let mut list: Vec<Scenario> = made
        .iter()
        .filter_map(|m| m.as_ref().ok().map(|m| m.scenario.clone()))
        .collect();
    crate::scenario::resolve_logs(&mut list, &w.paths.home);
    let b = bench_for(w)?;
    let mut b = b;
    b.reads = how.reads;
    b.stop.clone_from(&how.stop);
    let (m, text) = b.model()?;
    let ran = crate::scenario::score_each(
        &b,
        &m,
        &text,
        &crate::scenario::Started::none(),
        &list,
        true,
        &mut |_, _, _| {},
    )?;
    let outcomes: BTreeMap<String, Result<Ran, String>> = list
        .iter()
        .zip(ran)
        .map(|(s, r)| (s.id.clone(), r.map_err(|e| format!("{e:#}"))))
        .collect();
    let (mut differ, mut measured, mut failed, mut stopped) = (0, 0, 0, 0);
    let mut curves = BTreeMap::new();
    for (r, mr) in recipes.iter().zip(&made) {
        let curve = outcomes
            .get(&r.id)
            .map(|o| o.as_ref().map(|o| crate::scenario::curve_hash(o, false)));
        if let Some(Ok(c)) = &curve {
            curves.insert(r.id.clone(), c.clone());
        }
        match (mr, &curve) {
            (Err(_), _) => failed += 1,
            (Ok(_), Some(Err(_))) | (Ok(_), None) => failed += 1,
            (Ok(_), Some(Ok(_))) if matches!(outcomes.get(&r.id), Some(Ok(o)) if o.stopped) => {
                stopped += 1;
            }
            // A plan's curves are derived here, so there is nothing to compare.
            (Ok(_), Some(Ok(_))) if set.plan => {}
            (Ok(_), Some(Ok(_))) if !same_mapping => measured += 1,
            (Ok(_), Some(Ok(_))) if r.curve.is_empty() => measured += 1,
            (Ok(_), Some(Ok(c))) if *c != r.curve => differ += 1,
            (Ok(_), Some(Ok(_))) => {}
        }
    }
    Ok(Regen {
        set,
        mapping,
        differ,
        measured,
        failed,
        stopped,
        curves,
        outcomes,
        bench: b,
        model_text: text,
    })
}
