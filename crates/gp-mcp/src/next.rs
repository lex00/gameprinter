//! The tool `next`: where the port of a game stands, the one next step and
//! who does it, from `gp --json progress --next` (the same answer as the
//! terminal and the page's status line, crates/gp-cli/src/next.rs). It
//! reads files only and starts nothing.
//!
//! With `wait_s` it asks again every [`POLL`] until the answer changes or
//! the time is up (at most [`MAX_WAIT`]), so an agent that works only over
//! MCP hears when the person hands it something or answers on the page.

use std::time::{Duration, Instant};

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{Config, Reply, run_gp};

/// The longest `next` waits for a change.
pub const MAX_WAIT: Duration = Duration::from_secs(30);
/// How often it asks again while it waits.
pub const POLL: Duration = Duration::from_secs(2);

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NextArgs {
    /// The game, by its folder's name in the games checkout (donkey-kong).
    /// Default: the game whose page was opened last, or the only game.
    #[serde(default)]
    pub game: Option<String>,
    /// Seconds to wait for the answer to change (a hand-off or an answer
    /// from the page, a score), at most 30. 0 answers at once.
    #[serde(default)]
    pub wait_s: u64,
}

/// A game's folder name: letters, digits, `-` and `_`.
fn game_ok(g: &str) -> bool {
    !g.is_empty()
        && g.len() <= 64
        && g.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Asks `ask` until its answer differs from `first` or `wait` is over,
/// every `poll`. Returns the last answer and whether it changed.
pub fn until_changed(
    first: Value,
    wait: Duration,
    poll: Duration,
    mut ask: impl FnMut() -> Result<Value, String>,
) -> Result<(Value, bool), String> {
    let until = Instant::now() + wait.min(MAX_WAIT);
    let mut last = first.clone();
    while Instant::now() + poll <= until {
        std::thread::sleep(poll);
        last = ask()?;
        if last != first {
            return Ok((last, true));
        }
    }
    Ok((last, false))
}

/// `next`.
pub fn next(config: &Config, a: NextArgs) -> Result<Reply, String> {
    let mut args: Vec<&std::ffi::OsStr> = vec!["progress".as_ref(), "--next".as_ref()];
    if let Some(g) = &a.game {
        if !game_ok(g) {
            return Err(format!(
                "a game is its folder's name in the games checkout: letters, digits, - and _ (got {g:?})"
            ));
        }
        args.push("--template".as_ref());
        args.push(g.as_ref());
    }
    let ask = || -> Result<Value, String> {
        let (code, v) = run_gp(config, &args)?;
        if code != 0 {
            return Err(v["error"]
                .as_str()
                .map_or_else(|| v.to_string(), str::to_owned));
        }
        Ok(v)
    };
    let started = Instant::now();
    let first = ask()?;
    let (mut v, changed) = if a.wait_s == 0 {
        (first, false)
    } else {
        until_changed(first, Duration::from_secs(a.wait_s), POLL, ask)?
    };
    v["changed"] = json!(changed);
    v["waited_s"] = json!(started.elapsed().as_secs());
    Ok(Reply::ok(v))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_returns_when_the_answer_changes() {
        let mut n = 0;
        let (v, changed) = until_changed(
            json!({"next": "a"}),
            Duration::from_secs(5),
            Duration::from_millis(5),
            || {
                n += 1;
                Ok(json!({"next": if n < 3 { "a" } else { "b" }}))
            },
        )
        .unwrap();
        assert!(changed);
        assert_eq!(v["next"], "b");
        assert_eq!(n, 3);
    }

    #[test]
    fn it_stops_at_the_wait_and_never_past_the_bound() {
        let t = Instant::now();
        let (_, changed) = until_changed(
            json!(1),
            Duration::from_millis(60),
            Duration::from_millis(10),
            || Ok(json!(1)),
        )
        .unwrap();
        assert!(!changed);
        assert!(t.elapsed() < Duration::from_secs(1));
        // A wait past the bound is cut to it: with a poll longer than the
        // bound, nothing is asked again.
        let t = Instant::now();
        let (_, changed) = until_changed(
            json!(1),
            Duration::from_secs(3600),
            MAX_WAIT + Duration::from_secs(1),
            || panic!("asked past the bound"),
        )
        .unwrap();
        assert!(!changed);
        assert!(t.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn a_game_name_is_a_folder_name() {
        assert!(game_ok("donkey-kong") && game_ok("t_1"));
        assert!(!game_ok("../x") && !game_ok("") && !game_ok("a b"));
    }

    /// The tool runs `gp --json progress --next [--template GAME]` and
    /// passes its answer through, with `changed` and `waited_s`.
    #[test]
    fn the_tool_passes_gps_answer_through() {
        let d = tempfile::tempdir().unwrap();
        let gp = d.path().join("gp");
        std::fs::write(
            &gp,
            "#!/bin/sh\necho \"$@\" > \"$(dirname \"$0\")/args\"\necho '{\"version\": \"next 1\", \"stands\": \"No game file is on this computer yet.\", \"next\": {\"who\": \"person\"}}'\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&gp, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let config = Config {
            gp: gp.clone(),
            repo: d.path().into(),
            work: d.path().join("work"),
            home: d.path().join("home"),
            rom: None,
            assets: None,
            games: None,
        };
        let r = next(
            &config,
            NextArgs {
                game: Some("tapper".into()),
                wait_s: 0,
            },
        )
        .unwrap();
        assert!(!r.is_error);
        assert_eq!(r.json["next"]["who"], "person");
        assert_eq!(r.json["changed"], false);
        let args = std::fs::read_to_string(d.path().join("args")).unwrap();
        assert_eq!(args.trim(), "--json progress --next --template tapper");
        assert!(
            next(
                &config,
                NextArgs {
                    game: Some("../x".into()),
                    wait_s: 0
                }
            )
            .is_err()
        );
    }
}
