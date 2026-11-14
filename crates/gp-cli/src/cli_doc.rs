//! `gp self docs`: CLI.md, generated from gp's own commands, so the list
//! can't drift. Every top-level command is in the public set or the lab set
//! (the sort in docs/ cli-gui-research: public for making and playing games,
//! lab for research and the workflow's own tools); a new command that is in
//! neither fails `just check` until it's placed.

use std::fs;
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use clap::CommandFactory;
use serde_json::json;

use crate::{Cli, Outcome};

/// Top-level commands in the public set.
const PUBLIC: &[&str] = &[
    "assets", "corpus", "game", "model", "package", "registry", "roms", "serve", "template",
];
/// Top-level commands in the lab set.
const LAB: &[&str] = &[
    "baseline", "clip", "interp", "labels", "level", "mechanic", "picture", "rule", "sealed",
    "self", "solve", "tool", "worker",
];

/// Lab commands of the recordings, events and press searches (wave 3a of
/// the port from game-printer), kept on a line of their own so the waves'
/// landings don't edit one line.
const LAB_EVENTS: &[&str] = &[
    "capture",
    "events",
    "explore",
    "movie",
    "query",
    "record",
    "recordings",
    "replay",
    "run-until",
    "trace",
    "watch",
];

/// Lab commands of the detectors (wave 4 of the port), on their own for
/// the same reason.
const LAB_DETECTORS: &[&str] = &["coverage", "gaps", "sound", "sprites", "track"];

/// Lab commands of the code reading (wave 3b of the port from
/// game-printer), on a line of their own as wave 3a's are.
const LAB_CODE: &[&str] = &["code", "cycles", "element", "reach"];

/// Lab commands of gp serve's clip agents (game-printer's gp agent), on a
/// line of their own as the waves' are.
const LAB_AGENTS: &[&str] = &["agent"];

/// Lab commands of derivation and validation (wave 5 of the port), on a
/// line of their own as the other waves' are.
const LAB_DERIVE: &[&str] = &["dataset", "validate"];

/// Lab commands of wave 7 of the port from game-printer: the store, the
/// farms, the authoring prototype, the host and the MCP server.
const LAB_W7: &[&str] = &["author", "farm", "farm-topdown", "host", "mcp", "store"];

/// Lab commands of wave 9 of the port from game-printer, on a line of their
/// own as the other waves' are.
const LAB_W9: &[&str] = &[
    "catalog",
    "chunks",
    "env",
    "input-log",
    "metrics",
    "noop",
    "runs",
    "scorecard",
];

/// Lab commands of the record of findings, the progress ladder, the
/// oracles, the test ROMs and the localizer, ported from game-printer on a
/// line of their own as the waves' are.
const LAB_TOOLS: &[&str] = &["findings", "localize", "oracle", "progress", "testroms"];

/// Lab commands of the benches (game-printer's gp bench), on a line of
/// their own as the waves' are.
const LAB_BENCH: &[&str] = &["bench"];

/// Lab command of a recording's video (the tools port), on its own line.
const LAB_VIDEO: &[&str] = &["video"];

/// Public commands of the theme pipeline (wave 8 of the port), on a line
/// of their own as the waves' are.
const PUBLIC_THEME: &[&str] = &["theme"];

/// Walks the commands under `cmd`, calling `f` with each one's words, the
/// command, and whether it is a leaf (has no subcommands). The walk
/// CLI.md and `gp self commands` share.
pub(crate) fn walk<'a>(
    cmd: &'a clap::Command,
    words: &str,
    f: &mut impl FnMut(&str, &'a clap::Command, bool),
) {
    let mut subs = cmd
        .get_subcommands()
        .filter(|c| c.get_name() != "help")
        .peekable();
    let leaf = subs.peek().is_none();
    f(words, cmd, leaf);
    for c in subs {
        let w = if words.is_empty() {
            c.get_name().to_owned()
        } else {
            format!("{words} {}", c.get_name())
        };
        walk(c, &w, f);
    }
}

/// A set's table rows: each leaf command's words and first help line.
fn rows(cmd: &clap::Command, words: &str, out: &mut Vec<(String, String)>) {
    walk(cmd, words, &mut |w, c, leaf| {
        if leaf {
            let about = c.get_about().map(|a| a.to_string()).unwrap_or_default();
            let first = about.lines().next().unwrap_or("").trim().to_owned();
            out.push((w.to_owned(), first));
        }
    });
}

fn set_table(cmds: &[(String, String)]) -> String {
    let mut t = String::from("| Command | What it does |\n|---|---|\n");
    for (name, about) in cmds {
        t.push_str(&format!("| `{name}` | {} |\n", about.replace('|', "\\|")));
    }
    t
}

fn render() -> Result<String> {
    render_of(&Cli::command())
}

/// CLI.md as gp's commands give it, or an `Err` naming a command in
/// neither set.
fn render_of(cli: &clap::Command) -> Result<String> {
    let mut public = Vec::new();
    let mut lab = Vec::new();
    let mut unplaced = Vec::new();
    for c in cli.get_subcommands().filter(|c| c.get_name() != "help") {
        let name = c.get_name();
        let set = if PUBLIC.contains(&name) || PUBLIC_THEME.contains(&name) {
            &mut public
        } else if LAB.contains(&name)
            || LAB_EVENTS.contains(&name)
            || LAB_DETECTORS.contains(&name)
            || LAB_CODE.contains(&name)
            || LAB_AGENTS.contains(&name)
            || LAB_DERIVE.contains(&name)
            || LAB_W7.contains(&name)
            || LAB_W9.contains(&name)
            || LAB_BENCH.contains(&name)
            || LAB_VIDEO.contains(&name)
            || LAB_TOOLS.contains(&name)
        {
            &mut lab
        } else {
            unplaced.push(name.to_owned());
            continue;
        };
        rows(c, &format!("gp {name}"), set);
    }
    if !unplaced.is_empty() {
        bail!(
            "commands in neither set: {} (put each in PUBLIC or LAB in crates/gp-cli/src/cli_doc.rs)",
            unplaced.join(", ")
        );
    }
    public.sort();
    lab.sort();
    Ok(format!(
        "# The gp commands\n\n\
         Generated from gp's own commands by `gp self docs`; `just check` fails if it differs. Edit the commands, not this file.\n\n\
         ## The public set\n\n\
         For making and playing games: what a person installing gp uses.\n\n\
         {}\n\
         ## The lab set\n\n\
         For research, tests and the workflow's own tools: what an agent or a contributor uses. May change without notice.\n\n\
         {}",
        set_table(&public),
        set_table(&lab)
    ))
}

pub(crate) fn run(file: &Path, check: bool) -> Result<Outcome> {
    let want = render()?;
    if check {
        let have = fs::read_to_string(file).with_context(|| file.display().to_string())?;
        if have != want {
            return Ok(Outcome {
                json: json!({"ok": false, "file": file}),
                text: format!(
                    "{} differs from gp's commands; run `gp self docs {}` and commit it",
                    file.display(),
                    file.display()
                ),
                exit: 1,
            });
        }
        return Ok(Outcome::ok(
            json!({"ok": true, "file": file}),
            format!("{} matches gp's commands", file.display()),
        ));
    }
    fs::write(file, &want).with_context(|| file.display().to_string())?;
    Ok(Outcome::ok(
        json!({"file": file}),
        format!("wrote {}", file.display()),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every top-level command is placed, and CLI.md is what gp generates.
    /// A new command in neither set fails here before `just check` does.
    #[test]
    fn the_command_list_is_placed_and_current() {
        let doc = render().expect("every command placed");
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../CLI.md");
        assert_eq!(
            fs::read_to_string(path).unwrap(),
            doc,
            "CLI.md is out of date: run `gp self docs`"
        );
    }

    #[test]
    fn a_command_in_neither_set_is_named() {
        let bad = Cli::command().subcommand(clap::Command::new("brand-new"));
        let err = render_of(&bad).unwrap_err().to_string();
        assert!(err.contains("brand-new"), "{err}");
    }
}
