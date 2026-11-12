//! `gp mechanic import GAME --from OLD_HOME`: an old home's mechanic work
//! for a game, brought into this home, so the mechanic page shows what the
//! old one did: the sessions (clips, conversation, watch files), the quest
//! log (queue.json), the gate's scores ($GP_HOME/work/refine/GAME/) and the
//! sealed runs ($GP_HOME/sealed/GAME/).
//!
//! The old home named the game as game-printer did: for Donkey Kong that is
//! donkey-kong-oe, the prefix the games repo's corpus.toml gives its
//! imported clips (`[import] prefix`), so the old name is that prefix
//! unless `--old-name` says otherwise, and the game itself when there is
//! neither.
//!
//! Nothing here is overwritten. A file already in this home is kept, with
//! one exception: a session this home has made but never used (no clip and
//! no conversation) takes the old session.toml, its game named as here.
//! A session with clips or conversation of its own is left as it is and
//! listed. `--dry-run` lists what would be copied and writes nothing.

use super::session::{game_dir, name_ok};
use crate::Outcome;
use anyhow::{Context, Result, bail};
use gp_evidence::Paths;
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) struct Import<'a> {
    pub game: &'a str,
    pub from: &'a Path,
    pub old_name: Option<&'a str>,
    pub games: Option<&'a Path>,
    pub dry_run: bool,
}

/// The name the old home kept the game under.
fn old_name(a: &Import<'_>) -> String {
    if let Some(n) = a.old_name {
        return n.to_owned();
    }
    crate::roms_cmd::games_dir(a.games)
        .and_then(|g| fs::read_to_string(g.join(a.game).join("corpus.toml")).ok())
        .and_then(|t| t.parse::<toml::Table>().ok())
        .and_then(|t| t.get("import")?.get("prefix")?.as_str().map(str::to_owned))
        .unwrap_or_else(|| a.game.to_owned())
}

/// What one import does, file by file.
#[derive(Default)]
struct Plan {
    /// (from, to) for each file copied as it is.
    copy: Vec<(PathBuf, PathBuf)>,
    /// (to, text) for each session.toml written with the game renamed.
    write: Vec<(PathBuf, String)>,
    /// Sessions left as they are, and why.
    kept: Vec<String>,
    sessions: Vec<String>,
    /// Files already here, left as they are.
    present: usize,
}

/// Every file under `dir`, relative to it.
fn files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![PathBuf::new()];
    while let Some(rel) = stack.pop() {
        let d = dir.join(&rel);
        for e in fs::read_dir(&d).with_context(|| format!("reading {}", d.display()))? {
            let e = e?;
            let r = rel.join(e.file_name());
            let t = e.file_type()?;
            if t.is_dir() {
                stack.push(r);
            } else if t.is_file() {
                out.push(r);
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Whether a session folder here holds work of its own: a clip or a line
/// of conversation.
fn used(dir: &Path) -> bool {
    let clips = fs::read_dir(dir.join("clips")).is_ok_and(|mut rd| rd.next().is_some());
    let talk = fs::metadata(dir.join("talk.ndjson")).is_ok_and(|m| m.len() > 0);
    clips || talk
}

/// Plans the copy of the tree `from` into `to`, keeping whatever is there.
fn tree(plan: &mut Plan, from: &Path, to: &Path) -> Result<()> {
    if !from.is_dir() {
        return Ok(());
    }
    for rel in files(from)? {
        let dst = to.join(&rel);
        if dst.exists() {
            plan.present += 1;
        } else {
            plan.copy.push((from.join(&rel), dst));
        }
    }
    Ok(())
}

pub(crate) fn run(paths: &Paths, a: &Import<'_>) -> Result<Outcome> {
    let old = old_name(a);
    if !name_ok(&old) {
        bail!("an old game name is lower-case letters, digits and dashes: {old}");
    }
    let src = a.from.join("mechanics").join(&old);
    if !src.is_dir() {
        bail!("{} has no mechanic sessions for {old}", a.from.display());
    }
    if a.from.canonicalize().ok() == paths.home.canonicalize().ok() && old == a.game {
        bail!("that is this home and this game already");
    }
    let dst = game_dir(paths, a.game)?;
    let mut plan = Plan::default();
    let mut entries: Vec<_> = fs::read_dir(&src)?.filter_map(Result::ok).collect();
    entries.sort_by_key(fs::DirEntry::file_name);
    for e in entries {
        let name = e.file_name().to_string_lossy().into_owned();
        let from = e.path();
        let to = dst.join(&name);
        if !from.is_dir() {
            // queue.json, `current` and the like: only when there's none.
            if to.exists() {
                plan.present += 1;
            } else {
                plan.copy.push((from, to));
            }
            continue;
        }
        let toml_from = from.join("session.toml");
        if !toml_from.is_file() {
            tree(&mut plan, &from, &to)?;
            continue;
        }
        if used(&to) {
            plan.kept.push(format!(
                "{name}: this home's session has clips or conversation of its own"
            ));
            continue;
        }
        plan.sessions.push(name.clone());
        let text = fs::read_to_string(&toml_from)?;
        let renamed = text
            .lines()
            .map(|l| {
                if l.trim_start().starts_with("game") && l.contains(&format!("\"{old}\"")) {
                    format!("game = \"{}\"", a.game)
                } else {
                    l.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        plan.write.push((to.join("session.toml"), renamed));
        for rel in files(&from)? {
            if rel == Path::new("session.toml") {
                continue;
            }
            let d = to.join(&rel);
            if d.exists() {
                plan.present += 1;
            } else {
                plan.copy.push((from.join(&rel), d));
            }
        }
    }
    let refine = (
        a.from.join("work/refine").join(&old),
        paths.home.join("work/refine").join(a.game),
    );
    tree(&mut plan, &refine.0, &refine.1)?;
    let sealed = (
        a.from.join("sealed").join(&old),
        paths.home.join("sealed").join(a.game),
    );
    tree(&mut plan, &sealed.0, &sealed.1)?;

    if !a.dry_run {
        for (to, text) in &plan.write {
            fs::create_dir_all(to.parent().unwrap_or(Path::new(".")))?;
            fs::write(to, text).with_context(|| format!("writing {}", to.display()))?;
        }
        for (from, to) in &plan.copy {
            fs::create_dir_all(to.parent().unwrap_or(Path::new(".")))?;
            fs::copy(from, to)
                .with_context(|| format!("copying {} to {}", from.display(), to.display()))?;
        }
    }
    let verb = if a.dry_run { "would copy" } else { "copied" };
    let mut text = format!(
        "{} {}'s mechanic work as {} from {}: {} sessions ({}), {} files {verb}, {} already here kept",
        if a.dry_run { "Dry run:" } else { "Imported" },
        old,
        a.game,
        a.from.display(),
        plan.sessions.len(),
        plan.sessions.join(", "),
        plan.copy.len() + plan.write.len(),
        plan.present,
    );
    for k in &plan.kept {
        text.push_str(&format!("\nkept as it is: {k}"));
    }
    Ok(Outcome::ok(
        json!({
            "old_name": old,
            "game": a.game,
            "dry_run": a.dry_run,
            "sessions": plan.sessions,
            "kept": plan.kept,
            "files": plan.copy.len() + plan.write.len(),
            "present": plan.present,
        }),
        text,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(p: &Path, text: &str) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }

    #[test]
    fn an_old_home_comes_in_under_this_name_and_nothing_here_is_overwritten() {
        let old = tempfile::tempdir().unwrap();
        let new = tempfile::tempdir().unwrap();
        let o = old.path();
        put(
            &o.join("mechanics/dk-oe/queue.json"),
            "[{\"mechanic\":\"walk\",\"status\":\"in line\"}]",
        );
        put(
            &o.join("mechanics/dk-oe/walk/session.toml"),
            "game = \"dk-oe\"\nmechanic = \"walk\"\nprompt = \"old\"\n",
        );
        put(&o.join("mechanics/dk-oe/walk/talk.ndjson"), "{\"n\":1}\n");
        put(&o.join("mechanics/dk-oe/walk/clips/001.log"), "log");
        put(
            &o.join("mechanics/dk-oe/jump/session.toml"),
            "game = \"dk-oe\"\nmechanic = \"jump\"\n",
        );
        put(&o.join("mechanics/dk-oe/jump/clips/001.log"), "old jump");
        put(&o.join("work/refine/dk-oe/status.json"), "{}");
        put(&o.join("sealed/dk-oe/sealed-run/clips/001.toml"), "x");
        let n = new.path();
        // An unused walk (as gp mechanic start makes) and a jump with a clip.
        put(
            &n.join("mechanics/dk/walk/session.toml"),
            "game = \"dk\"\nmechanic = \"walk\"\nprompt = \"\"\n",
        );
        fs::create_dir_all(n.join("mechanics/dk/walk/clips")).unwrap();
        put(
            &n.join("mechanics/dk/jump/session.toml"),
            "game = \"dk\"\nmechanic = \"jump\"\n",
        );
        put(&n.join("mechanics/dk/jump/clips/001.log"), "new jump");
        let paths = Paths {
            home: n.to_path_buf(),
            store: n.join("store"),
            roms: n.join("roms"),
        };
        let a = Import {
            game: "dk",
            from: o,
            old_name: Some("dk-oe"),
            games: None,
            dry_run: true,
        };
        let r = run(&paths, &a).unwrap();
        assert!(r.text.starts_with("Dry run:"), "{}", r.text);
        assert!(
            !n.join("mechanics/dk/queue.json").exists(),
            "a dry run writes nothing"
        );

        let a = Import {
            dry_run: false,
            ..a
        };
        let r = run(&paths, &a).unwrap();
        assert!(r.text.contains("kept as it is: jump"), "{}", r.text);
        let walk = fs::read_to_string(n.join("mechanics/dk/walk/session.toml")).unwrap();
        assert!(
            walk.contains("game = \"dk\"") && walk.contains("prompt = \"old\""),
            "{walk}"
        );
        assert_eq!(
            fs::read_to_string(n.join("mechanics/dk/walk/clips/001.log")).unwrap(),
            "log"
        );
        assert_eq!(
            fs::read_to_string(n.join("mechanics/dk/jump/clips/001.log")).unwrap(),
            "new jump"
        );
        assert!(n.join("mechanics/dk/queue.json").is_file());
        assert!(n.join("work/refine/dk/status.json").is_file());
        assert!(n.join("sealed/dk/sealed-run/clips/001.toml").is_file());

        // Again: everything is here now, so nothing is copied.
        let r = run(&paths, &a).unwrap();
        assert!(r.text.contains("0 files copied"), "{}", r.text);
    }
}
