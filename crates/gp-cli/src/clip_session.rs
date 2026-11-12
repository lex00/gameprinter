//! A mechanic session's clips as `gp clip log` and `gp clip score` name
//! them: `SESSION:N` for one clip, `SESSION` for every clip of a session,
//! and every session of a game for `--all`. Sessions are
//! `$GP_HOME/mechanics/GAME/SESSION/`, GAME the package folder's name.
//!
//! Ported from game-printer's clip_score.rs. A sealed session (its name
//! starts with `sealed`), a clip kept under `$GP_HOME/sealed`, or a clip
//! whose path, start and inputs play a sealed run's (`crate::sealed`) is
//! refused, and left out of `--all`.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use gp_evidence::{InputLog, Paths};

/// A clip of a session: the session's name, its folder and the number.
pub(crate) struct SessionClip {
    pub session: String,
    pub dir: PathBuf,
    pub n: u64,
}

impl SessionClip {
    /// `SESSION:N` as the scores print it.
    pub(crate) fn name(&self) -> String {
        format!("{}:{}", self.session, self.n)
    }

    /// `SESSION-NNN`, its output folder's and log file's name.
    pub(crate) fn key(&self) -> String {
        format!("{}-{:03}", self.session, self.n)
    }

    /// Its input log from power-on and the frame it starts at.
    pub(crate) fn log(&self, home: &Path) -> Result<(InputLog, usize)> {
        let (text, start) = crate::mechanic::paths::full_log(home, &self.dir, self.n)
            .with_context(|| format!("clip {}", self.name()))?;
        let log = InputLog::parse(&text).map_err(|e| anyhow::anyhow!("{}: {e}", self.name()))?;
        Ok((log, start))
    }

    /// Replays the original to the clip's start and checks its state
    /// against the hash the clip was saved with, when it has one.
    pub(crate) fn check_start(&self, home: &Path, rom: &[u8]) -> Result<()> {
        let t: toml::Table = toml::from_str(
            &fs::read_to_string(self.dir.join(format!("clips/{:03}.toml", self.n)))
                .with_context(|| format!("clip {}", self.name()))?,
        )
        .with_context(|| format!("clip {}", self.name()))?;
        let (Some(path), Some(want)) = (
            t.get("path").and_then(|v| v.as_str()),
            t.get("state").and_then(|v| v.as_str()),
        ) else {
            return Ok(());
        };
        let start = t.get("start").and_then(|v| v.as_integer()).unwrap_or(0) as usize;
        let got = crate::mechanic::paths::original_at(home, rom, path, start)?.state_hex()?;
        if got != want {
            bail!(
                "clip {} starts at frame {start} of path {path}, but the state there is {got}, not {want}",
                self.name()
            );
        }
        Ok(())
    }
}

/// Whether a clip spec names a session's clips rather than a corpus clip:
/// corpus IDs have no `:` and are dotted (dk.hammer.018); a session's name
/// is lower-case letters, digits and dashes.
pub(crate) fn is_session_spec(spec: &str) -> bool {
    spec.contains(':') || (!spec.contains('.') && crate::mechanic::name_ok(spec))
}

/// The game's name for its sessions: its package folder's.
pub(crate) fn game_name(game: &Path) -> Result<String> {
    let abs = fs::canonicalize(game).with_context(|| game.display().to_string())?;
    abs.file_name()
        .and_then(|n| n.to_str())
        .map(str::to_owned)
        .with_context(|| format!("{} names no folder", game.display()))
}

/// Refuses a sealed session: its name starts with `sealed`.
pub(crate) fn refuse_session(s: &str) -> Result<()> {
    if s.starts_with("sealed") {
        bail!(
            "the session {s} is a sealed run (its name starts with sealed): gp sealed score is the coordinator's"
        );
    }
    Ok(())
}

fn refuse_dir(home: &Path, dir: &Path) -> Result<()> {
    let sealed = home.join("sealed");
    if dir.starts_with(&sealed) {
        bail!(
            "{} is kept under $GP_HOME/sealed: gp sealed score is the coordinator's",
            dir.display()
        );
    }
    Ok(())
}

fn session_dir(paths: &Paths, game: &str, s: &str) -> Result<PathBuf> {
    if !crate::mechanic::name_ok(s) {
        bail!("no session {s}");
    }
    refuse_session(s)?;
    let dir = paths.home.join("mechanics").join(game).join(s);
    if !dir.is_dir() {
        bail!("{game} has no session {s}");
    }
    refuse_dir(&paths.home, &dir)?;
    Ok(dir)
}

/// A session's clip numbers, in order.
fn clips_of(dir: &Path) -> Vec<u64> {
    let mut v: Vec<u64> = fs::read_dir(dir.join("clips"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            (p.extension()? == "toml")
                .then(|| p.file_stem()?.to_str()?.parse().ok())
                .flatten()
        })
        .collect();
    v.sort_unstable();
    v
}

/// The clips `spec` names: `SESSION:N`, or every clip of `SESSION`; each
/// refused if it plays a sealed run.
pub(crate) fn clips(paths: &Paths, game: &str, spec: &str) -> Result<Vec<SessionClip>> {
    let out = named(paths, game, spec)?;
    let runs = crate::sealed::runs(&paths.home, game);
    for c in &out {
        crate::sealed::refuse_clip_in(&paths.home, &runs, &c.dir, c.n)?;
    }
    Ok(out)
}

fn named(paths: &Paths, game: &str, spec: &str) -> Result<Vec<SessionClip>> {
    if let Some((s, n)) = spec.split_once(':') {
        let n: u64 = n
            .parse()
            .with_context(|| format!("{spec}: the clip's number"))?;
        let dir = session_dir(paths, game, s)?;
        if !dir.join(format!("clips/{n:03}.toml")).is_file() {
            bail!("{game} has no clip {n} in {s}");
        }
        return Ok(vec![SessionClip {
            session: s.to_owned(),
            dir,
            n,
        }]);
    }
    let dir = session_dir(paths, game, spec)?;
    Ok(clips_of(&dir)
        .into_iter()
        .map(|n| SessionClip {
            session: spec.to_owned(),
            dir: dir.clone(),
            n,
        })
        .collect())
}

/// Every clip of every session of the game, the sealed sessions and the
/// clips that play a sealed run left out: the clips, how many sessions and
/// how many clips were left out.
pub(crate) fn all(paths: &Paths, game: &str) -> Result<(Vec<SessionClip>, usize, usize)> {
    let root = paths.home.join("mechanics").join(game);
    let mut sessions: Vec<String> = fs::read_dir(&root)
        .with_context(|| format!("{game} has no sessions"))?
        .flatten()
        .filter(|e| e.path().join("clips").is_dir())
        .filter_map(|e| e.file_name().to_str().map(str::to_owned))
        .collect();
    let before = sessions.len();
    sessions.retain(|s| refuse_session(s).is_ok());
    let sealed = before - sessions.len();
    sessions.sort();
    let runs = crate::sealed::runs(&paths.home, game);
    let (mut out, mut left_out) = (Vec::new(), 0);
    for s in sessions {
        let dir = root.join(&s);
        for n in clips_of(&dir) {
            if crate::sealed::refuse_clip_in(&paths.home, &runs, &dir, n).is_err() {
                left_out += 1;
                continue;
            }
            out.push(SessionClip {
                session: s.clone(),
                dir: dir.clone(),
                n,
            });
        }
    }
    Ok((out, sealed, left_out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(home: &Path, game: &str, s: &str, clips: &[u64]) {
        let dir = home.join("mechanics").join(game).join(s).join("clips");
        fs::create_dir_all(&dir).unwrap();
        for n in clips {
            fs::write(dir.join(format!("{n:03}.toml")), "start = 0\n").unwrap();
            fs::write(
                dir.join(format!("{n:03}.log")),
                "gp-input-log 0\ncontrollers 1\nP ........\n",
            )
            .unwrap();
        }
    }

    #[test]
    fn session_clips_are_named_one_by_one_by_session_and_all_without_sealed_ones() {
        let d = tempfile::tempdir().unwrap();
        let paths = Paths {
            home: d.path().to_owned(),
            store: d.path().join("store"),
            roms: d.path().join("roms"),
        };
        session(d.path(), "dk", "jump", &[1, 2, 10]);
        session(d.path(), "dk", "walk", &[3]);
        session(d.path(), "dk", "sealed-1", &[1]);
        let one = clips(&paths, "dk", "jump:2").unwrap();
        assert_eq!(one.len(), 1);
        assert_eq!(
            (one[0].name(), one[0].key()),
            ("jump:2".into(), "jump-002".into())
        );
        let names: Vec<String> = clips(&paths, "dk", "jump")
            .unwrap()
            .iter()
            .map(SessionClip::name)
            .collect();
        assert_eq!(names, ["jump:1", "jump:2", "jump:10"]);
        assert!(clips(&paths, "dk", "jump:4").is_err());
        assert!(clips(&paths, "dk", "sealed-1:1").is_err());
        assert!(clips(&paths, "dk", "../x").is_err());
        let (every, sealed, left_out) = all(&paths, "dk").unwrap();
        assert_eq!((sealed, left_out), (1, 0));
        let names: Vec<String> = every.iter().map(SessionClip::name).collect();
        assert_eq!(names, ["jump:1", "jump:2", "jump:10", "walk:3"]);
        let (log, start) = every[3].log(d.path()).unwrap();
        assert_eq!((log.frames.len(), start), (1, 0));
    }

    #[test]
    fn a_corpus_id_is_not_a_session() {
        assert!(is_session_spec("jump:3"));
        assert!(is_session_spec("climb-ladder"));
        assert!(!is_session_spec("dk.hammer.018"));
    }
}
