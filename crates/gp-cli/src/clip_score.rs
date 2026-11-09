//! `gp clip log`: one corpus clip as an input log from power-on.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use clap::Subcommand;
use gp_evidence::{InputLog, Paths};
use serde_json::json;

use crate::Outcome;
use crate::clipset;

#[derive(Subcommand)]
pub enum ClipCmd {
    /// Write a corpus clip's input log from power-on: the presses to its
    /// start, then its own presses or presses file. Prints the file and the
    /// frame the clip starts at.
    Log {
        /// The game's package folder (games/donkey-kong).
        game: PathBuf,
        /// A clip of the game's clips.toml by its ID (dk.hammer.018).
        clip: String,
        /// Where to write it, `-` for standard output (default: CLIP.log in
        /// the output folder, $GP_HOME/work/clip-score/$GP_WORKER).
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

pub(crate) fn run(cmd: &ClipCmd, paths: &Paths) -> Result<Outcome> {
    match cmd {
        ClipCmd::Log { game, clip, out } => log_cmd(paths, game, clip, out.as_deref()),
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

/// A corpus clip: a recipe of a game's clips.toml, named by its ID.
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

/// The corpus clip `spec` names in the game's clips.toml. A sealed clip is
/// left out of the set when it is read.
pub(crate) fn corpus_clip(game: &Path, spec: &str) -> Result<CorpusClip> {
    let file = game.join(clipset::FILE);
    let set = clipset::read(&file)?;
    set.clips
        .into_iter()
        .find(|r| r.id == spec)
        .map(|recipe| CorpusClip {
            set_dir: game.to_owned(),
            recipe,
        })
        .with_context(|| format!("{} has no clip {spec}", file.display()))
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
