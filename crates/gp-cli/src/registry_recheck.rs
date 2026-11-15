//! `gp theme recheck GAME THEME --registry TARGET`: a theme a registry
//! offers for a game, read again on this machine as the page's lift reads
//! it, and the outcome recorded in the version's by-game line as `recheck`
//! (gp_package::registry::Recheck), inside the file the list signature
//! covers. The page still offers a theme whose recheck failed and shows why
//! (gp_web_registry::Offered::recheck); `gp registry check --for` lists it.

use std::path::Path;

use anyhow::{Result, anyhow, bail};
use gp_package::registry::{Reader, Recheck, record_recheck};
use serde_json::{Value, json};

use crate::Outcome;
use crate::registry::{Writing, look_check_at, open, signing, utc, writing};
use crate::theme_publish::RecheckArgs;

/// What a recheck's findings (`gp theme look check`'s "On the page" group,
/// as [`crate::registry::look_check_at`] returns them) come to: `pass` or
/// `fail` and why, or None when nothing says the theme was drawn or left
/// out, so there is nothing to record.
pub(crate) fn verdict(found: &Value) -> Option<(&'static str, String)> {
    let all = found["findings"].as_array().cloned().unwrap_or_default();
    let text = |f: &Value| {
        f["message"]
            .as_str()
            .unwrap_or("")
            .trim()
            .trim_end_matches('.')
            .to_owned()
    };
    let problems: Vec<String> = all
        .iter()
        .filter(|f| f["level"] == "problem")
        .map(text)
        .collect();
    if !problems.is_empty() {
        return Some(("fail", problems.join("; ")));
    }
    if all.iter().any(|f| f["level"] == "ok") {
        return Some((
            "pass",
            "the page's lift reads its files and draws it over a ROM of the game".into(),
        ));
    }
    None
}

/// The words `gp registry check --for` adds to an offered theme's line
/// for a failed recheck, empty for none.
pub(crate) fn said(r: Option<&Recheck>) -> String {
    match r {
        Some(r) if r.failed() => format!("; its recheck on {} failed: {}", r.date, r.reason),
        _ => String::new(),
    }
}

pub(crate) fn run(a: &RecheckArgs, target: &str) -> Result<Outcome> {
    let (game, name) = (a.game.as_str(), a.theme.as_str());
    for (what, n) in [("GAME", game), ("THEME", name)] {
        if !gp_package::registry::name_ok(n) {
            bail!(
                "with --registry, {what} is a package name, NAMESPACE/NAME (lex00/donkey-kong, friend/night), not {n:?}"
            );
        }
    }
    let t = open(target);
    let now = utc(0)?;
    let r = Reader::unsigned(&*t).map_err(|e| anyhow!(e))?;
    let lines = r
        .by_game(game, &now)
        .map_err(|e| anyhow!("the registry's list for {game} can't be read: {e}"))?
        .line;
    let yanks = r.yanks(&now).map(|y| y.yank).unwrap_or_default();
    let offer = gp_web_registry::choose(game, &lines, &yanks);
    let (version, title) = match offer.verdict(name) {
        Some(Ok(o)) => (o.version.clone(), o.title.clone()),
        Some(Err(l)) => (l.version.clone(), l.title.clone()),
        None => bail!(
            "{target}'s list for {game} (by-game/) doesn't name {name}, so there is no line to recheck"
        ),
    };
    let id = lines
        .iter()
        .find(|l| l.package == name && l.version == version)
        .map(|l| l.id.clone())
        .ok_or_else(|| anyhow!("{name}@{version} isn't in the list for {game}"))?;
    let head = format!("recheck of {name}@{version} ({title}) for {game} in {target}");
    let found = match look_check_at(&r, (name, &version, &id), game, a.rom.as_deref()) {
        Ok(v) => v,
        Err(e) => {
            return Ok(not_checked(
                &head,
                name,
                &version,
                &format!("its files weren't read as the page's lift reads them: {e:#}"),
            ));
        }
    };
    let Some((result, reason)) = verdict(&found) else {
        return Ok(not_checked(
            &head,
            name,
            &version,
            "the page's lift said neither that it draws the theme nor that it leaves it out",
        ));
    };
    let recheck = Recheck {
        date: now[..10].to_owned(),
        result: result.into(),
        reason,
    };
    let how = Writing {
        dry_run: a.dry_run,
        no_push: a.no_push,
    };
    let message = format!("Record a recheck of {name}@{version} for {game}");
    let mut o = record(
        target,
        game,
        (name, &version),
        &recheck,
        a.key.as_deref(),
        how,
    )?;
    o.text = format!("{head}\n{}", o.text);
    o.json["findings"] = found["findings"].clone();
    o.json["git_message"] = json!(message);
    if recheck.failed() && o.exit == 0 {
        // Recorded: exit 1 all the same, as a theme that failed.
        o.exit = 1;
    }
    Ok(o)
}

/// A recheck that couldn't run: nothing recorded, exit 1.
fn not_checked(head: &str, name: &str, version: &str, why: &str) -> Outcome {
    Outcome {
        json: json!({ "rechecked": false, "name": name, "version": version, "why": why }),
        text: format!(
            "{head}\nnot checked: {why}\nnothing was recorded in the registry; a recheck that can't run says nothing about the theme"
        ),
        exit: 1,
    }
}

/// Writes `recheck` into `name@version`'s line of the game's by-game file
/// in `target`, through `writing` (a dry run writes nothing; a git: target
/// commits), signed with `key` when given.
pub(crate) fn record(
    target: &str,
    game: &str,
    (name, version): (&str, &str),
    recheck: &Recheck,
    key: Option<&Path>,
    how: Writing,
) -> Result<Outcome> {
    let message = format!("Record a recheck of {name}@{version} for {game}");
    writing(target, &message, how, |t| {
        let (now, expires) = (utc(0)?, utc(7)?);
        let line = signing(key, |s| {
            record_recheck(t, game, name, version, recheck.clone(), s, &now, &expires)
        })?;
        let text = if recheck.failed() {
            format!(
                "{name}@{version} failed its recheck on {}: {}\nrecorded in by-game/{game}.toml; the page still offers it and says why",
                recheck.date, recheck.reason
            )
        } else {
            format!(
                "{name}@{version} passed its recheck on {}: {}\nrecorded in by-game/{game}.toml",
                recheck.date, recheck.reason
            )
        };
        Ok(Outcome {
            json: json!({
                "rechecked": true, "name": name, "version": version, "game": game,
                "recheck": line.recheck,
            }),
            text,
            exit: 0,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gp_package::registry::{Folder, Publish, Signer, Target, init, publish, reindex};

    fn registry() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let t = Folder(d.path().to_owned());
        let none = Signer::none();
        init(
            &t,
            "t",
            &none,
            &[],
            &[("friend".into(), vec![])],
            "2026-11-14",
        )
        .unwrap();
        let files = std::collections::BTreeMap::from([(
            "theme/look.txt".to_owned(),
            b"made up\n".to_vec(),
        )]);
        publish(
            &t,
            &Publish {
                name: "friend/night",
                version: "0.1.0",
                kind: "content",
                title: "Night",
                for_games: &["lex00/game".to_owned()],
                published: "2026-11-14T00:00:00Z",
            },
            &files,
            &none,
        )
        .unwrap();
        let v = [("friend/night".to_owned(), "0.1.0".to_owned())];
        reindex(
            &t,
            &v,
            &none,
            "2026-11-14T00:00:00Z",
            "2099-01-01T00:00:00Z",
        )
        .unwrap();
        d
    }

    /// Made-up findings: a problem fails it with the problem's words, an ok
    /// passes it, and neither records nothing.
    #[test]
    fn findings_come_to_pass_fail_or_nothing() {
        let f = |l: &[(&str, &str)]| json!({ "findings": l.iter().map(|(level, m)| json!({ "level": level, "message": m })).collect::<Vec<_>>() });
        let (r, why) = verdict(&f(&[("ok", "drawn"), ("problem", "made up: no sheet.")])).unwrap();
        assert_eq!(r, "fail");
        assert_eq!(why, "made up: no sheet");
        assert_eq!(
            verdict(&f(&[("warning", "w"), ("ok", "drawn")])).unwrap().0,
            "pass"
        );
        assert!(verdict(&f(&[("note", "n")])).is_none());
        assert!(verdict(&json!({})).is_none());
    }

    /// A failed recheck lands in the version's by-game line, a dry run
    /// writes nothing, and the registry check's line says why.
    #[test]
    fn a_failed_recheck_is_recorded_and_a_dry_run_writes_nothing() {
        let d = registry();
        let target = d.path().to_string_lossy().into_owned();
        let key = "by-game/lex00/game.toml";
        let before = Folder(d.path().to_owned()).read(key).unwrap();
        let failed = Recheck {
            date: "2026-11-14".into(),
            result: "fail".into(),
            reason: "the page's lift leaves it out: made up".into(),
        };
        let dry = Writing {
            dry_run: true,
            no_push: false,
        };
        let o = record(
            &target,
            "lex00/game",
            ("friend/night", "0.1.0"),
            &failed,
            None,
            dry,
        )
        .unwrap();
        assert!(o.text.contains("dry run"), "{}", o.text);
        assert_eq!(Folder(d.path().to_owned()).read(key).unwrap(), before);
        let o = record(
            &target,
            "lex00/game",
            ("friend/night", "0.1.0"),
            &failed,
            None,
            Writing::default(),
        )
        .unwrap();
        assert!(
            o.text.contains("failed its recheck on 2026-11-14"),
            "{}",
            o.text
        );
        let t = Folder(d.path().to_owned());
        let line = Reader::unsigned(&t)
            .unwrap()
            .by_game("lex00/game", "2026-11-14T00:00:00Z")
            .unwrap()
            .line
            .remove(0);
        assert_eq!(line.recheck.as_ref(), Some(&failed));
        assert_eq!(
            said(line.recheck.as_ref()),
            "; its recheck on 2026-11-14 failed: the page's lift leaves it out: made up"
        );
        let passed = Recheck {
            result: "pass".into(),
            ..failed
        };
        assert_eq!(said(Some(&passed)), "");
    }
}
