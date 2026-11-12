//! `gp clip log` and `gp clip score`: one corpus clip as an input log from
//! power-on, and its score on a model.
//!
//! A score runs the original and the model in lockstep from the clip's
//! start twice, straight through (free) and with resync, with the
//! original's random draws when the mapping sets `score_draws`. It prints
//! the frames together both ways, how the two games' frames pair, the
//! exact stretches with resync, the first lasting split straight through
//! and the splits by measure, and keeps the lockstep's JSON in an output
//! folder of the caller's own.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use clap::Subcommand;
use gp_evidence::{InputLog, Paths};
use gp_interp::lockstep::{Lockstep, Mode, seconds, together};
use gp_interp::mapping::Mapping;
use serde_json::{Value, json};

use crate::Outcome;
use crate::clipset;
use crate::hosted::Source;

/// Frames a measure must stay split for, from a frame on, for that frame to
/// be the first lasting split; one that splits closer to the clip's end
/// must stay split to the end.
pub(crate) const LASTING: usize = 30;

#[derive(Subcommand)]
pub enum ClipCmd {
    /// Write a corpus clip's input log from power-on: the presses to its
    /// start, then its own presses or presses file. Prints the file and the
    /// frame the clip starts at.
    Log {
        /// The game's package folder (games/donkey-kong).
        game: PathBuf,
        /// A clip of the game's corpus by its ID (dk.hammer.018).
        clip: String,
        /// Where to write it, `-` for standard output (default: CLIP.log in
        /// the output folder, $GP_HOME/work/clip-score/$GP_WORKER).
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Score corpus clips on a model: each clip's lockstep from its start,
    /// straight through and with resync, both with the original's random
    /// draws when the state mapping sets `score_draws`. Prints the frames
    /// together both ways, the frame pairing, the exact stretches, the first
    /// lasting split and the splits by measure, and writes each clip's log,
    /// both lockstep JSONs and its score into OUT/CLIP/.
    Score {
        /// The game's package folder (games/donkey-kong).
        game: PathBuf,
        /// Clips of the game's corpus by their IDs.
        #[arg(required = true)]
        clips: Vec<String>,
        /// The game's built module. Needed for a model with hooks.
        #[arg(long)]
        module: Option<PathBuf>,
        /// The model folder (model.toml and mapping.toml); by default the
        /// game's own.
        #[arg(long)]
        model: Option<PathBuf>,
        /// The model's lifted assets (default: $GP_HOME/work/assets/SOURCE).
        #[arg(long)]
        assets: Option<PathBuf>,
        /// The ROM; by default the one in $GP_ROM_DIR the package names.
        #[arg(long)]
        rom: Option<PathBuf>,
        /// The output folder (default: $GP_HOME/work/clip-score/$GP_WORKER,
        /// a folder for each caller; `default` without GP_WORKER).
        #[arg(long)]
        out: Option<PathBuf>,
        /// A mapped path whose values on each frame straight through, the
        /// original's and the model's, go into the clip's `values.json`
        /// with the frame pairing. Repeatable.
        #[arg(long = "values")]
        values: Vec<String>,
    },
}

pub(crate) fn run(cmd: &ClipCmd, paths: &Paths) -> Result<Outcome> {
    match cmd {
        ClipCmd::Log { game, clip, out } => log_cmd(paths, game, clip, out.as_deref()),
        ClipCmd::Score {
            game,
            clips,
            module,
            model,
            assets,
            rom,
            out,
            values,
        } => score_cmd(
            paths,
            game,
            clips,
            &Open {
                module: module.as_deref(),
                model: model.as_deref(),
                assets: assets.as_deref(),
                rom: rom.as_deref(),
            },
            out.as_deref(),
            values,
        ),
    }
}

/// The caller's own output folder: `$GP_HOME/work/clip-score/$GP_WORKER`.
pub(crate) fn out_dir(paths: &Paths) -> PathBuf {
    let who = std::env::var("GP_WORKER")
        .ok()
        .map(|w| clipset::file_key(w.trim()))
        .filter(|w| !w.is_empty() && w != "." && w != "..")
        .unwrap_or_else(|| "default".into());
    paths.home.join("work").join("clip-score").join(who)
}

/// A corpus clip: a recipe of a game's corpus, named by its ID.
pub(crate) struct CorpusClip {
    set_dir: PathBuf,
    pub recipe: clipset::Recipe,
}

impl CorpusClip {
    /// Its input from power-on and its start, as the corpus's scorer plays
    /// it: the presses to its start, then its own presses or presses file
    /// ([`clipset::recipe_log`]).
    pub(crate) fn log(&self) -> Result<(InputLog, usize)> {
        clipset::recipe_log(&self.set_dir, &self.recipe)?.with_context(|| {
            format!(
                "corpus clip {} starts from a recording or its presses file is missing",
                self.recipe.id
            )
        })
    }
}

/// The corpus clip `spec` names in the game's corpus. A sealed clip is
/// left out of the set when it is read.
pub(crate) fn corpus_clip(game: &Path, spec: &str) -> Result<CorpusClip> {
    let set = clipset::read_game(game)?;
    set.clips
        .into_iter()
        .find(|r| r.id == spec)
        .map(|recipe| CorpusClip {
            set_dir: game.to_owned(),
            recipe,
        })
        .with_context(|| format!("{} has no clip {spec}", game.display()))
}

fn log_cmd(paths: &Paths, game: &Path, spec: &str, out: Option<&Path>) -> Result<Outcome> {
    let r = corpus_clip(game, spec)?;
    let (log, start) = r.log()?;
    let name = clipset::file_key(&r.recipe.id);
    let text = log.to_text();
    let j = json!({"clip": spec, "start": start, "frames": log.frames.len()});
    if out == Some(Path::new("-")) {
        return Ok(Outcome::ok(j, text.trim_end().to_owned()));
    }
    let file = match out {
        Some(f) => f.to_owned(),
        None => out_dir(paths).join(format!("{name}.log")),
    };
    if let Some(p) = file.parent() {
        fs::create_dir_all(p)?;
    }
    fs::write(&file, &text).with_context(|| format!("writing {}", file.display()))?;
    let mut j = j;
    j["file"] = json!(file);
    Ok(Outcome::ok(
        j,
        format!(
            "wrote {}: {} frames from power-on; the clip starts at frame {start}",
            file.display(),
            log.frames.len()
        ),
    ))
}

/// What every clip's score shares: the model, its mapping and the ROM.
pub(crate) struct Ctx {
    model_dir: PathBuf,
    source: Source,
    mapping: Mapping,
    rom: Vec<u8>,
    assets: Option<PathBuf>,
    /// $GP_HOME, where the start cache is (`$GP_HOME/work/snapshots`).
    home: PathBuf,
    /// Whether clips start from the start cache.
    pub starts: bool,
    /// Mapped paths whose values, the original's and the model's on each
    /// frame straight through, go into each clip's `values.json`.
    pub values: Vec<String>,
}

/// What a tool was told about the model and the ROM.
pub(crate) struct Open<'a> {
    pub module: Option<&'a Path>,
    pub model: Option<&'a Path>,
    pub assets: Option<&'a Path>,
    pub rom: Option<&'a Path>,
}

impl Ctx {
    pub(crate) fn open(paths: &Paths, game: &Path, how: &Open) -> Result<Ctx> {
        let model_dir = how.model.unwrap_or(game).to_owned();
        let (source, assets) = Source::open_dir(&model_dir, how.assets, paths, how.module)?;
        let mfile = model_dir.join(gp_interp::mapping::MAPPING_FILE);
        let mapping = Mapping::parse(
            &fs::read_to_string(&mfile).with_context(|| format!("reading {}", mfile.display()))?,
        )
        .map_err(|e| anyhow::anyhow!("{}: {e}", mfile.display()))?;
        Ok(Ctx {
            rom: rom_of(paths, game, how.rom)?,
            model_dir,
            source,
            mapping,
            assets: Some(assets),
            home: paths.home.clone(),
            starts: true,
            values: Vec::new(),
        })
    }
}

/// The scored ROM: the first the game's `package.toml` names, from `--rom`
/// or found by its hash in the ROM folder.
pub(crate) fn rom_of(paths: &Paths, game: &Path, rom: Option<&Path>) -> Result<Vec<u8>> {
    let file = game.join("package.toml");
    let pkg: toml::Table = fs::read_to_string(&file)
        .with_context(|| format!("reading {}", file.display()))?
        .parse()
        .with_context(|| file.display().to_string())?;
    let entries = pkg
        .get("rom")
        .and_then(|r| r.get("entry"))
        .and_then(|e| e.as_array())
        .cloned()
        .unwrap_or_default();
    let sha = |e: &toml::Value| {
        e.get("sha256")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned()
    };
    let scored = entries
        .iter()
        .find(|e| e.get("role").and_then(|v| v.as_str()) == Some("scored"))
        .or(entries.first())
        .with_context(|| format!("{} names no ROM", file.display()))?;
    let want = sha(scored);
    let given: Vec<PathBuf> = rom.iter().map(|p| p.to_path_buf()).collect();
    let mut found = crate::roms::find(std::slice::from_ref(&want), &given, &paths.roms)?;
    let r = found.remove(&want).with_context(|| {
        format!(
            "no ROM with hash {want} in {}; give one with --rom or set GP_ROM_DIR",
            paths.roms.display()
        )
    })?;
    // The original's runtime is picked by the package's platform; a ROM of
    // the other form is refused here, so the file's form, which the start
    // cache and the factory read again, always agrees with it.
    let platform = crate::package::Package::load(game)?.platform()?;
    if r.platform != platform {
        bail!(
            "{} says the ROM is for the {} platform, and the file found is for the {}",
            file.display(),
            platform.name(),
            r.platform.name()
        );
    }
    Ok(r.bytes)
}

/// One clip's score: its JSON and its lines.
pub(crate) struct Scored {
    pub json: Value,
    pub text: String,
}

/// `{"together": T, "frames": N}` as a percentage to a tenth.
pub(crate) fn percent(v: &Value) -> String {
    let (t, n) = (
        v["together"].as_u64().unwrap_or(0),
        v["frames"].as_u64().unwrap_or(0),
    );
    let pm = (t * 1000).checked_div(n).unwrap_or(0);
    format!("{}.{}%", pm / 10, pm % 10)
}

/// The first lasting split: the first compared frame from which some
/// measure stays past its tolerance for `hold` frames, or to the end. The
/// frame's index and the measures that last from it.
pub(crate) fn first_lasting(ls: &Lockstep, hold: usize) -> Option<(usize, Vec<usize>)> {
    let n = ls.frames.len();
    let split: Vec<Vec<bool>> = ls
        .paths
        .iter()
        .map(|p| p.distance.iter().map(|d| *d > p.tolerance).collect())
        .collect();
    // For each measure, how many frames it stays split from each frame.
    let runs: Vec<Vec<usize>> = split
        .iter()
        .map(|s| {
            let mut r = vec![0; s.len() + 1];
            for i in (0..s.len()).rev() {
                r[i] = if s[i] { r[i + 1] + 1 } else { 0 };
            }
            r
        })
        .collect();
    (0..n).find_map(|i| {
        let lasting: Vec<usize> = runs
            .iter()
            .enumerate()
            .filter(|(_, r)| {
                r.get(i)
                    .is_some_and(|&k| k > 0 && (k >= hold || i + k >= n))
            })
            .map(|(p, _)| p)
            .collect();
        (!lasting.is_empty()).then_some((i, lasting))
    })
}

/// The clip's start from the start cache (`clip::cache::start_state`):
/// the original [`crate::clip::cache::START_LEAD`] frames before the
/// clip's start and where a lockstep from power-on stands there
/// ([`gp_interp::lockstep::Prefix`]), the counts kept beside the
/// snapshot under the mapping's start condition. Made on first use by
/// playing the lockstep's first frames once. `None` with the cache off,
/// or for a clip that starts too early to have one.
fn resume_of(
    ctx: &Ctx,
    log: &InputLog,
    inputs: &[gp_runtime::InputFrame],
    start: usize,
) -> Result<Option<(crate::clip::cache::Saved, gp_interp::lockstep::Prefix)>> {
    use crate::clip::cache;
    use gp_evidence::manifest::PowerOn;
    let k = start.saturating_sub(cache::START_LEAD);
    if !ctx.starts || !cache::starts_on() || k == 0 || k > log.frames.len() {
        return Ok(None);
    }
    let home = &ctx.home;
    let key = cache::start_key(&ctx.rom, &PowerOn::default(), &log.frames, k);
    let tag = format!(
        "lockstep prefix 1\nstart {:?}\nalign {:?}\n",
        ctx.mapping.start, ctx.mapping.align
    );
    let opts = gp_interp::lockstep::Options {
        mode: Mode::Free,
        from: Some(start as u64),
        ..Default::default()
    };
    let note = cache::read_note(home, &key, &tag)
        .and_then(|b| serde_json::from_slice::<gp_interp::lockstep::Prefix>(&b).ok())
        .filter(|p| p.inputs == k);
    if let Some(p) = note
        && let Some(saved) = cache::read_start(home, &key, &ctx.rom, PowerOn::default())
    {
        return Ok(Some((saved, p)));
    }
    // Made: the lockstep's first k frames from power-on, once.
    let mut original = crate::original::OriginalRuntime::of_rom(&ctx.rom)?;
    let mut interp = ctx.source.open()?;
    let spec = ctx.source.spec.clone();
    let Ok(p) = gp_interp::lockstep::prefix(
        &mut original,
        &mut interp,
        &spec,
        &ctx.mapping,
        inputs,
        &opts,
        k,
    ) else {
        return Ok(None);
    };
    let saved = cache::write_start_of(home, &key, &original)?;
    cache::write_note(home, &key, &tag, &serde_json::to_vec(&p)?);
    Ok(Some((saved, p)))
}

/// Plays one clip on the model straight through, as [`score_one`] does,
/// with the original's random draws, and gives `keep` each compared frame's
/// two pictures: the mechanic page's review of a clip. With `values`, the
/// lockstep keeps each frame's mapped values too, for the page's list of
/// what differs. Returns the lockstep, whose `frames` the pictures are in
/// the order of, its report, and the original's draws the model was given.
pub(crate) fn pictures(
    ctx: &Ctx,
    log: &InputLog,
    start: usize,
    keep: crate::screen::Keep,
    values: bool,
) -> Result<(Lockstep, Value, Option<gp_sim::DrawScript>)> {
    let from = start as u64;
    let inputs = crate::run::frames(log);
    let resume = resume_of(ctx, log, &inputs, start)?;
    let draws = if ctx.mapping.score_draws {
        Some(match &resume {
            Some((saved, p)) => crate::lockstep::record_draws_on(
                saved.nes(&ctx.rom, gp_evidence::manifest::PowerOn::default())?,
                &ctx.mapping,
                &log.frames[p.inputs..],
                from,
            )?,
            None => crate::lockstep::record_draws(&ctx.rom, &ctx.mapping, &log.frames, from)?,
        })
    } else {
        None
    };
    let (ls, report, _) = crate::lockstep::run_resumed(
        &ctx.source,
        &ctx.model_dir,
        &ctx.mapping,
        &ctx.rom,
        &inputs,
        Mode::Free,
        Some(from),
        values,
        ctx.assets.as_deref(),
        draws.clone(),
        resume.as_ref().map(|(s, p)| (s, *p)),
        Some(keep),
    )?;
    Ok((ls, report, draws))
}

/// Scores one clip, given as its full log from power-on and its start,
/// and writes its files into `out`.
pub(crate) fn score_one(
    ctx: &Ctx,
    name: &str,
    log: &InputLog,
    start: usize,
    out: &Path,
) -> Result<Scored> {
    fs::create_dir_all(out).with_context(|| format!("making {}", out.display()))?;
    fs::write(out.join("full.log"), log.to_text())?;
    let from = start as u64;
    let inputs = crate::run::frames(log);
    let resume = resume_of(ctx, log, &inputs, start)?;
    let draws = if ctx.mapping.score_draws {
        let d = match &resume {
            Some((saved, p)) => crate::lockstep::record_draws_on(
                saved.nes(&ctx.rom, gp_evidence::manifest::PowerOn::default())?,
                &ctx.mapping,
                &log.frames[p.inputs..],
                from,
            )?,
            None => crate::lockstep::record_draws(&ctx.rom, &ctx.mapping, &log.frames, from)?,
        };
        fs::write(out.join("draws.txt"), d.text())?;
        Some(d)
    } else {
        None
    };
    let run = |mode: Mode, values: bool| {
        crate::lockstep::run_resumed(
            &ctx.source,
            &ctx.model_dir,
            &ctx.mapping,
            &ctx.rom,
            &inputs,
            mode,
            Some(from),
            values,
            ctx.assets.as_deref(),
            draws.clone(),
            resume.as_ref().map(|(s, p)| (s, *p)),
            None,
        )
    };
    let (free, free_j, _) = run(Mode::Free, true)?;
    let (rs, rs_j, rs_lines) = run(Mode::Resync, false)?;
    let write = |f: &str, v: &Value| -> Result<()> {
        fs::write(out.join(f), serde_json::to_string(v)?)?;
        Ok(())
    };
    write("free.json", &json!({"lockstep": free_j}))?;
    if !ctx.values.is_empty() {
        let show =
            |x: Option<&gp_runtime::Value>| x.map(gp_interp::mapping::show).unwrap_or_default();
        let mut by = serde_json::Map::new();
        for want in &ctx.values {
            let Some(p) = free.paths.iter().position(|x| &x.path == want) else {
                bail!("--values {want}: the mapping has no such path");
            };
            let rows: Vec<Value> = free
                .values
                .iter()
                .map(|(o, m)| json!([show(o.get(p)), show(m.get(p))]))
                .collect();
            by.insert(want.clone(), Value::Array(rows));
        }
        write("values.json", &json!({"frames": free.frames, "values": by}))?;
    }
    write("resync.json", &json!({"lockstep": rs_j}))?;
    let count = |ls: &Lockstep| {
        let t = together(ls);
        json!({"together": t.iter().filter(|x| **x).count(), "frames": t.len()})
    };
    let straight = count(&free);
    let mut resync = count(&rs);
    resync["splits"] = json!(rs.splits.len());
    let own = log.frames.len().saturating_sub(start);
    let mut lines = vec![format!(
        "{name}: {own} frames from frame {start} of its play from power-on"
    )];
    // How the two games' frames pair, and where the mapping's scope ends.
    let pair = gp_interp::lockstep::pairing(&free.frames);
    match &pair {
        Some(p) => lines.push(format!("  pairing: {}", p.says())),
        None => lines.push("  pairing: no frame compared".into()),
    }
    let pair_j = json!(pair);
    if let Some(f) = free.scope_end {
        lines.push(format!(
            "  out of scope from the original's frame {f}: the mapping's [scope] ends there, {} frames left out",
            free.out_of_scope
        ));
    }
    lines.push(format!(
        "  straight through: {}/{} frames together ({})",
        straight["together"],
        straight["frames"],
        percent(&straight)
    ));
    lines.push(format!(
        "  with resync: {}/{} frames together ({}), {} splits",
        resync["together"],
        resync["frames"],
        percent(&resync),
        rs.splits.len()
    ));
    let lasting = first_lasting(&free, LASTING).map(|(i, ps)| {
        let (m, o) = free.frames[i];
        let at = |p: usize| {
            let v = free.values.get(i);
            let show = |x: Option<&gp_runtime::Value>| {
                x.map(gp_interp::mapping::show).unwrap_or_default()
            };
            json!({
                "path": free.paths[p].path,
                "original": show(v.and_then(|v| v.0.get(p))),
                "model": show(v.and_then(|v| v.1.get(p))),
            })
        };
        json!({"frame": m, "original_frame": o, "paths": ps.iter().map(|&p| at(p)).collect::<Vec<_>>()})
    });
    match &lasting {
        Some(l) => {
            let ps = l["paths"].as_array().cloned().unwrap_or_default();
            let first = &ps[0];
            lines.push(format!(
                "  first lasting split (straight through): the original's frame {} (model frame {}): {} original {}, model {}{}",
                l["original_frame"],
                l["frame"],
                first["path"].as_str().unwrap_or(""),
                first["original"].as_str().unwrap_or(""),
                first["model"].as_str().unwrap_or(""),
                if ps.len() > 1 {
                    format!(" (and {} more)", ps.len() - 1)
                } else {
                    String::new()
                }
            ));
        }
        None => lines.push("  first lasting split (straight through): none".into()),
    }
    // The resync run's stretches, splits by state and by measure, and draws.
    let keep = rs_lines
        .iter()
        .skip_while(|l| !l.starts_with("stretches ("))
        .map(|l| format!("  {}", l.trim_start()));
    lines.extend(keep);
    let st = &rs_j["stretches"];
    lines.push(format!(
        "  longest stretch with resync: {} s",
        seconds(st["longest_any"].as_u64().unwrap_or(0))
    ));
    // The sound straight through, when the model makes its own.
    let sound = crate::lockstep::sound_report(&free);
    if let Some((_, sl)) = &sound {
        lines.extend(sl.iter().map(|l| {
            format!(
                "  {}",
                l.trim_start()
                    .replacen("sound:", "sound (straight through):", 1)
            )
        }));
    }
    lines.push(format!("  wrote {}", out.display()));
    let j = json!({
        "clip": name,
        "start": start,
        "frames": own,
        "pairing": pair_j,
        "scope": free.scope_end.map(|f| json!({"original_frame": f, "frames": free.out_of_scope})),
        "straight": straight,
        "resync": resync,
        "stretches": st,
        "first_lasting": lasting,
        // The first split straight through, as `gp corpus score` gives it.
        "first": free.splits.first().map(|f| json!({
            "frame": f.original_frame.saturating_sub(from),
            "original_frame": f.original_frame,
            "paths": f.paths.iter().map(|p| p.path.clone()).collect::<Vec<_>>(),
        })),
        "splits_by_measure": st["splits_by_measure"],
        "draws": rs_j["draws"],
        "sound": sound.map(|s| s.0),
        "out": out,
    });
    let text = lines.join("\n");
    write("score.json", &j)?;
    fs::write(out.join("score.txt"), format!("{text}\n"))?;
    Ok(Scored { json: j, text })
}

// Each argument is one input the caller already holds; the command's own
// plumbing, with no group of them that belongs together.
#[allow(clippy::too_many_arguments)]
fn score_cmd(
    paths: &Paths,
    game: &Path,
    specs: &[String],
    how: &Open,
    out: Option<&Path>,
    values: &[String],
) -> Result<Outcome> {
    let corpus: Vec<CorpusClip> = specs
        .iter()
        .map(|c| corpus_clip(game, c))
        .collect::<Result<_>>()?;
    if corpus.is_empty() {
        bail!("no clips to score");
    }
    let mut ctx = Ctx::open(paths, game, how)?;
    ctx.values = values.to_vec();
    let out = out.map_or_else(|| out_dir(paths), Path::to_owned);
    let results = crate::pool::map(
        &corpus,
        || (),
        |_, _, c| -> Result<Scored, String> {
            let (log, start) = c.log().map_err(|e| format!("{e:#}"))?;
            score_one(
                &ctx,
                &c.recipe.id,
                &log,
                start,
                &out.join(clipset::file_key(&c.recipe.id)),
            )
            .map_err(|e| format!("{e:#}"))
        },
        &mut |_| {},
    );
    let mut text = Vec::new();
    let mut rows = Vec::new();
    let mut failed = 0;
    for (c, r) in corpus.iter().zip(results) {
        match r {
            Ok(sc) => {
                text.push(sc.text);
                rows.push(sc.json);
            }
            Err(e) => {
                failed += 1;
                text.push(format!("{}: couldn't be scored: {e}", c.recipe.id));
                rows.push(json!({"clip": c.recipe.id, "error": e}));
            }
        }
    }
    let mut o = Outcome::ok(json!({"clips": rows, "out": out}), text.join("\n"));
    if failed > 0 {
        o.exit = 1;
    }
    Ok(o)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gp_interp::lockstep::PathSeries;
    use gp_sim::Fx;

    fn ls(frames: Vec<(u64, u64)>, dist: &[&[i64]]) -> Lockstep {
        Lockstep {
            mode: Mode::Free,
            original_frames: 0,
            model_frames: 0,
            before_start: 0,
            lag_frames: 0,
            start: None,
            frames,
            paths: dist
                .iter()
                .enumerate()
                .map(|(i, d)| PathSeries {
                    path: format!("p{i}"),
                    tolerance: Fx::ZERO,
                    distance: d.iter().map(|x| Fx::from_int(*x)).collect(),
                    within: 0,
                    max: Fx::ZERO,
                    first_split: None,
                })
                .collect(),
            splits: Vec::new(),
            resync_notes: Vec::new(),
            hashes: Vec::new(),
            values: Vec::new(),
            unmeasured: Vec::new(),
            levels: Vec::new(),
            events: Vec::new(),
            screen: None,
            bodies: Vec::new(),
            scene: Vec::new(),
            scope_end: None,
            out_of_scope: 0,
            sound: Vec::new(),
        }
    }

    #[test]
    fn the_first_lasting_split_skips_short_ones() {
        let frames: Vec<(u64, u64)> = (0..8).map(|i| (i, i + 3)).collect();
        // p0 splits for one frame at 1; p1 from 4 to the end.
        let l = ls(
            frames.clone(),
            &[&[0, 1, 0, 0, 0, 0, 0, 0], &[0, 0, 0, 0, 1, 1, 1, 1]],
        );
        assert_eq!(first_lasting(&l, 3), Some((4, vec![1])));
        // Held to 5 frames, p1's four to the end still last.
        assert_eq!(first_lasting(&l, 5), Some((4, vec![1])));
        // A split of 3 frames that heals before the end lasts at 3, not 4.
        let l = ls(frames, &[&[0, 1, 1, 1, 0, 0, 0, 0]]);
        assert_eq!(first_lasting(&l, 3), Some((1, vec![0])));
        assert_eq!(first_lasting(&l, 4), None);
    }

    #[test]
    fn the_output_folder_is_the_callers() {
        let d = tempfile::tempdir().unwrap();
        let p = Paths {
            home: d.path().to_path_buf(),
            store: d.path().join("store"),
            roms: d.path().join("roms"),
        };
        let o = out_dir(&p);
        assert!(o.starts_with(d.path().join("work/clip-score")));
    }
}
