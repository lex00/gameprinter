//! Clips that start from a snapshot, as the mechanic page saves them. A clip
//! is a start state plus the buttons pressed from there. The start state is
//! the state at frame N of a path, a real-input log from power-on stored
//! once under `$GP_HOME/work/paths/<sha256>.log`, so the snapshot is always
//! one real play reached. The clip's `.toml` names the path, the frame and
//! the hash of the canonical state there; its `.log` holds only the clip's
//! own inputs. A clip written the old way (a full log from power-on and a
//! start frame) still reads the same.
//!
//! Ported from game-printer's clip.rs. The console at a frame of a path
//! comes from the start cache in [`crate::clip::cache`].

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use gp_evidence::input_log::{InputFrame, InputLog};
use gp_evidence::manifest::PowerOn;

use crate::clip::cache;
use crate::original::{OriginalRuntime, Platform};

/// Where paths are kept.
fn paths_dir(home: &Path) -> PathBuf {
    home.join("work").join("paths")
}

fn path_file(home: &Path, sha: &str) -> Result<PathBuf> {
    if sha.len() != 64 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("a path is named by its sha256 in hex: {sha}");
    }
    Ok(paths_dir(home).join(format!("{sha}.log")))
}

/// Stores a real-input log from power-on as a path, once: its name is the
/// sha256 of its text. Returns the name and its frame count.
pub(crate) fn put_path(home: &Path, text: &str) -> Result<(String, usize)> {
    let log = InputLog::parse(text).map_err(|e| anyhow::anyhow!("the path's input log: {e}"))?;
    // Stored in one spelling, so the same play always gets the same name.
    let text = log.to_text();
    let sha = crate::template::sha256_hex(text.as_bytes());
    let f = path_file(home, &sha)?;
    if !f.is_file() {
        fs::create_dir_all(paths_dir(home))?;
        let tmp = f.with_extension(format!("tmp{}", std::process::id()));
        fs::write(&tmp, &text)?;
        fs::rename(&tmp, &f)?;
    }
    Ok((sha, log.frames.len()))
}

/// A stored path's frames.
pub(crate) fn read_path(home: &Path, sha: &str) -> Result<InputLog> {
    let f = path_file(home, sha)?;
    let text = fs::read_to_string(&f).with_context(|| format!("no path {sha}"))?;
    InputLog::parse(&text).map_err(|e| anyhow::anyhow!("path {sha}: {e}"))
}

/// A path made of a stored path's frames and more after them, for a page
/// that sends only what was played since its last upload.
pub(crate) fn extend_path(home: &Path, base: &str, tail: &[InputFrame]) -> Result<(String, usize)> {
    let mut log = read_path(home, base)?;
    log.frames.extend_from_slice(tail);
    put_path(home, &log.to_text())
}

/// The console at frame `n` of a path, from the start cache when it holds
/// that start; else replayed from power-on and kept there.
pub(crate) fn nes_at(home: &Path, rom: &[u8], sha: &str, n: usize) -> Result<gp_nes::Nes> {
    let log = read_path(home, sha)?;
    if n > log.frames.len() {
        bail!("path {sha} has {} frames, not {n}", log.frames.len());
    }
    let power_on = PowerOn::default();
    let key = cache::start_key(rom, &power_on, &log.frames, n);
    if cache::starts_on()
        && let Some(saved) = cache::read_start(home, &key, rom, power_on)
    {
        return saved.nes(rom, power_on);
    }
    let mut nes = gp_nes::Nes::power_on(rom, power_on)?;
    nes.set_video(false);
    for f in &log.frames[..n] {
        nes.step(*f)?;
    }
    if cache::starts_on() {
        cache::write_start(home, &key, &nes)?;
    }
    Ok(nes)
}

/// The original at frame `n` of a path on either platform: [`nes_at`] for
/// an NES image; for a romset, the start cache's snapshot or the path's
/// frames replayed on gp-arcade from power-on, and kept there.
pub(crate) fn original_at(home: &Path, rom: &[u8], sha: &str, n: usize) -> Result<OriginalRuntime> {
    use gp_runtime::Runtime as _;
    if Platform::of_file(rom) == Platform::Nes {
        let nes = nes_at(home, rom, sha, n)?;
        return Ok(OriginalRuntime::Nes(
            gp_nes::protocol::NesRuntime::new(nes).map_err(anyhow::Error::msg)?,
        ));
    }
    let log = read_path(home, sha)?;
    if n > log.frames.len() {
        bail!("path {sha} has {} frames, not {n}", log.frames.len());
    }
    if log.arcade.is_none() {
        bail!("path {sha} is an NES input log, and the game is an arcade romset");
    }
    let power_on = PowerOn::default();
    let key = cache::start_key(rom, &power_on, &log.frames, n);
    if cache::starts_on()
        && let Some(saved) = cache::read_start(home, &key, rom, power_on)
    {
        return OriginalRuntime::from_saved(rom, &saved);
    }
    let mut o = OriginalRuntime::load(Platform::Arcade, rom)?;
    let inputs: Vec<gp_runtime::InputFrame> = log.frames[..n]
        .iter()
        .map(|f| gp_runtime::InputFrame {
            buttons: f.buttons_word(),
            ..Default::default()
        })
        .collect();
    o.step(&inputs).map_err(anyhow::Error::msg)?;
    if cache::starts_on() {
        cache::write_start_of(home, &key, &o)?;
    }
    Ok(o)
}

/// What a clip's `.toml` says about where it starts.
struct Meta {
    path: Option<String>,
    start: usize,
}

fn meta(dir: &Path, n: u64) -> Result<Meta> {
    let t: toml::Table =
        toml::from_str(&fs::read_to_string(dir.join(format!("clips/{n:03}.toml")))?)?;
    Ok(Meta {
        path: t.get("path").and_then(|v| v.as_str()).map(str::to_owned),
        start: t.get("start").and_then(|v| v.as_integer()).unwrap_or(0) as usize,
    })
}

/// A clip as a full log from power-on and its start frame, whichever way it
/// was written: what the page replays.
pub(crate) fn full_log(home: &Path, dir: &Path, n: u64) -> Result<(String, usize)> {
    let m = meta(dir, n)?;
    let own = fs::read_to_string(dir.join(format!("clips/{n:03}.log")))?;
    let Some(sha) = m.path else {
        return Ok((own, m.start));
    };
    let mut log = read_path(home, &sha)?;
    log.frames.truncate(m.start);
    let clip = InputLog::parse(&own).map_err(|e| anyhow::anyhow!("clip {n}: {e}"))?;
    // A clip on controller 2 keeps it when its path has only controller 1.
    log.controllers = log.controllers.max(clip.controllers);
    log.frames.extend(clip.frames);
    Ok((log.to_text(), m.start))
}

/// The `.toml` fields and the `.log` text of a clip that starts at frame
/// `start` of a stored path, without the state's hash (added once a replay
/// to the start has found it).
pub(crate) fn clip_fields(sha: &str, start: usize, inputs: &InputLog) -> (toml::Table, String) {
    let mut t = toml::Table::new();
    t.insert("path".into(), sha.into());
    t.insert("start".into(), (start as i64).into());
    t.insert("frames".into(), (inputs.frames.len() as i64).into());
    let log = InputLog {
        controllers: controllers_of(&inputs.frames),
        frames: inputs.frames.clone(),
        arcade: inputs.arcade.clone(),
    };
    (t, log.to_text())
}

/// The controllers a log of these frames needs: 2 when controller 2 holds
/// a button on any frame (a two-player game), else 1.
fn controllers_of(frames: &[InputFrame]) -> u8 {
    if frames.iter().any(|f| f.pads[1] != 0) {
        2
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 16KB NROM whose reset counts a RAM byte up forever, so its state
    /// moves every frame. Made up here: no game's bytes.
    fn counting_rom() -> Vec<u8> {
        let mut rom = b"NES\x1a\x01\x01\x00\x00".to_vec();
        rom.resize(16, 0);
        let mut prg = vec![0u8; 0x4000];
        // C000: INC $10; C002: LDA $4016; C005: JMP $C000
        prg[..8].copy_from_slice(&[0xE6, 0x10, 0xAD, 0x16, 0x40, 0x4C, 0x00, 0xC0]);
        prg[0x3FFA..].copy_from_slice(&[0x00, 0xC0, 0x00, 0xC0, 0x00, 0xC0]);
        rom.extend(prg);
        rom.extend(vec![0u8; 0x2000]);
        rom
    }

    fn log_of(n: usize) -> String {
        let mut s = String::from("gp-input-log 1\ncontrollers 1\n");
        for i in 0..n {
            s.push_str(if i % 5 == 0 {
                "P .......A\n"
            } else {
                "P ........\n"
            });
        }
        s
    }

    /// A path is kept once under the hash of its one spelling, and grows
    /// by a tail into another path.
    #[test]
    fn a_path_is_kept_once_and_extended() {
        let d = tempfile::tempdir().unwrap();
        let (a, n) = put_path(d.path(), &log_of(10)).unwrap();
        assert_eq!(n, 10);
        assert_eq!(put_path(d.path(), &log_of(10)).unwrap().0, a);
        let tail = InputLog::parse(&log_of(4)).unwrap().frames;
        let (b, n) = extend_path(d.path(), &a, &tail).unwrap();
        assert_eq!(n, 14);
        assert_ne!(a, b);
        assert_eq!(read_path(d.path(), &b).unwrap().frames.len(), 14);
        assert!(read_path(d.path(), "../x").is_err());
    }

    /// The console at a frame of a path has the same state from the cache
    /// as replayed from power-on, and a clip's full log is its path to the
    /// start then its own frames.
    #[test]
    fn the_start_of_a_clip_is_the_same_cached_or_replayed() {
        let d = tempfile::tempdir().unwrap();
        let rom = counting_rom();
        let (sha, _) = put_path(d.path(), &log_of(30)).unwrap();
        let first = original_at(d.path(), &rom, &sha, 20)
            .unwrap()
            .state_hex()
            .unwrap();
        let again = original_at(d.path(), &rom, &sha, 20)
            .unwrap()
            .state_hex()
            .unwrap();
        assert_eq!(first, again);
        assert_ne!(
            first,
            original_at(d.path(), &rom, &sha, 21)
                .unwrap()
                .state_hex()
                .unwrap()
        );
        assert!(original_at(d.path(), &rom, &sha, 31).is_err());

        let own = InputLog::parse(&log_of(3)).unwrap();
        let (t, log) = clip_fields(&sha, 20, &own);
        let dir = d.path().join("s");
        fs::create_dir_all(dir.join("clips")).unwrap();
        fs::write(dir.join("clips/001.toml"), toml::to_string(&t).unwrap()).unwrap();
        fs::write(dir.join("clips/001.log"), log).unwrap();
        let (full, start) = full_log(d.path(), &dir, 1).unwrap();
        assert_eq!(start, 20);
        assert_eq!(InputLog::parse(&full).unwrap().frames.len(), 23);
    }

    /// An arcade game's path and clip, in the arcade log's spelling, with
    /// the owner's rbtapper romset: the start is the same cached or
    /// replayed, and the clip's full log is its path then its frames.
    /// Skipped without the romset.
    #[test]
    fn an_arcade_clip_starts_the_same_cached_or_replayed() {
        let Some(zip) = gp_arcade::romset::local_romset("rbtapper") else {
            eprintln!("skipped: no rbtapper romset");
            return;
        };
        let rom = fs::read(zip).unwrap();
        let d = tempfile::tempdir().unwrap();
        let play = "gp-arcade-input 0\nsystem rbtapper\ncolumns CSRLDUT\n40 .......\n6 C......\n20 .......\n";
        let (sha, n) = put_path(d.path(), play).unwrap();
        assert_eq!(n, 66);
        let first = original_at(d.path(), &rom, &sha, 50).unwrap();
        assert_eq!(gp_runtime::Runtime::frame(&first), 50);
        let again = original_at(d.path(), &rom, &sha, 50).unwrap();
        assert_eq!(first.state_hex().unwrap(), again.state_hex().unwrap());
        let own =
            InputLog::parse("gp-arcade-input 0\nsystem rbtapper\ncolumns CSRLDUT\n5 .S.....\n")
                .unwrap();
        let (t, log) = clip_fields(&sha, 50, &own);
        assert!(log.starts_with("gp-arcade-input 0\n"), "{log}");
        let dir = d.path().join("s");
        fs::create_dir_all(dir.join("clips")).unwrap();
        fs::write(dir.join("clips/001.toml"), toml::to_string(&t).unwrap()).unwrap();
        fs::write(dir.join("clips/001.log"), log).unwrap();
        let (full, start) = full_log(d.path(), &dir, 1).unwrap();
        assert_eq!(start, 50);
        let full = InputLog::parse(&full).unwrap();
        assert_eq!(full.frames.len(), 55);
        assert_eq!(full.frames[52].pads[0], 2);
        // An NES log is no path for an arcade game.
        let (nes_sha, _) = put_path(d.path(), &log_of(10)).unwrap();
        assert!(original_at(d.path(), &rom, &nes_sha, 5).is_err());
    }
}
