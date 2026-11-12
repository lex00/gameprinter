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

/// A set's table rows: each leaf command's words and first help line.
fn rows(cmd: &clap::Command, words: &str, out: &mut Vec<(String, String)>) {
    let mut subs = cmd
        .get_subcommands()
        .filter(|c| c.get_name() != "help")
        .peekable();
    if subs.peek().is_none() {
        let about = cmd.get_about().map(|a| a.to_string()).unwrap_or_default();
        let first = about.lines().next().unwrap_or("").trim().to_owned();
        out.push((words.to_owned(), first));
        return;
    }
    for c in subs {
        rows(c, &format!("{words} {}", c.get_name()), out);
    }
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
        let set = if PUBLIC.contains(&name) {
            &mut public
        } else if LAB.contains(&name)
            || LAB_EVENTS.contains(&name)
            || LAB_DETECTORS.contains(&name)
            || LAB_CODE.contains(&name)
            || LAB_AGENTS.contains(&name)
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
