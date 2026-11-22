//! The loop's steps (docs/loop.md, issue #43): every step `next` emits
//! gets a stable id, what it touches and the gp commands it runs, and the
//! steps that can run at once are all open, no two touching the same
//! thing. gp_run checks a write against them (gp_mcp::steps), and gp
//! serve's runner starts a worker on each open step of the agent's.
//!
//! An id is a hash of the step's subject, who does it, its command (its
//! words when it has none), what it touches and the facts the answer was
//! decided from, less the background's live counts, which differ between
//! the page's server and a terminal. So it stays the same while the facts
//! do, from the CLI, the MCP tool and the page alike, and a step decided
//! from facts that have changed since is a new one.

use std::collections::BTreeSet;
use std::path::Path;

use clap::CommandFactory;
use serde_json::{Value, json};

use crate::next::{Act, Answer, Step, Who};

/// The id of one step decided from `facts`.
pub(crate) fn id_of(journey: &str, subject: Option<&str>, s: &Step, facts: &Value) -> String {
    let mut facts = facts.clone();
    if let Some(o) = facts.as_object_mut() {
        o.remove("background");
    }
    let what = match &s.how.cli {
        Some(c) => json!({"cli": c}),
        None => json!({"says": s.says}),
    };
    let key = json!({
        "journey": journey,
        "subject": subject,
        "who": s.who,
        "what": what,
        "touches": s.touches,
        "facts": facts,
    });
    let h = blake3::hash(key.to_string().as_bytes()).to_hex();
    format!("s{}", &h[..12])
}

/// The gp commands a step's command line runs: each `gp` in it followed
/// by the longest path of words that is one of gp's commands with no
/// subcommands of its own. "gp mechanic show GAME ROW, then gp mechanic
/// say ..." runs `mechanic show` and `mechanic say`.
pub(crate) fn commands_of(cli: &str) -> Vec<String> {
    let root = crate::Cli::command();
    let words: Vec<&str> = cli
        .split_whitespace()
        .map(|w| w.trim_end_matches([',', '.', ';', ':']))
        .collect();
    let mut out: Vec<String> = Vec::new();
    for (i, w) in words.iter().enumerate() {
        if *w != "gp" {
            continue;
        }
        let mut at = &root;
        let mut path: Vec<&str> = Vec::new();
        for x in &words[i + 1..] {
            match at.find_subcommand(x) {
                Some(c) if c.get_name() != "help" => {
                    at = c;
                    path.push(x);
                }
                _ => break,
            }
        }
        let leaf = at.get_subcommands().all(|c| c.get_name() == "help");
        if !path.is_empty() && leaf {
            let p = path.join(" ");
            if !out.contains(&p) {
                out.push(p);
            }
        }
    }
    out
}

/// What a step touches when it names nothing itself: the row a person
/// plays, the page for the person's other steps, and the subject for
/// the agent's and the tools' (a game's folder, or one of the person's
/// copies).
fn default_touches(journey: &str, subject: Option<&str>, s: &Step) -> Vec<String> {
    match (s.who, &s.act, subject) {
        (Who::Person, Some(Act::PlayFrom { row }), Some(g)) => vec![format!("row:{g}/{row}")],
        (Who::Person, _, _) => vec!["page".into()],
        (_, _, Some(g)) if journey == "port" => vec![format!("game:{g}")],
        (_, _, Some(c)) => vec![format!("copy:{journey}/{c}")],
        (_, _, None) => vec!["home".into()],
    }
}

/// Fills in each candidate's touches, commands and id, in the order the
/// rules put them, and returns the open ones: the first, then each that
/// touches nothing an earlier open one does.
pub(crate) fn settle(
    journey: &str,
    subject: Option<&str>,
    facts: &Value,
    steps: &mut [Step],
) -> Vec<Step> {
    let mut taken: BTreeSet<String> = BTreeSet::new();
    let mut open = Vec::new();
    for s in steps.iter_mut() {
        if s.touches.is_empty() {
            s.touches = default_touches(journey, subject, s);
        }
        s.commands = s.how.cli.as_deref().map(commands_of).unwrap_or_default();
        s.id = id_of(journey, subject, s, facts);
        if s.touches.iter().all(|t| !clashes(&taken, t)) {
            taken.extend(s.touches.iter().cloned());
            open.push(s.clone());
        }
    }
    open
}

/// One open step as `gp loop steps` lists it, with the answer it is from.
pub(crate) fn listed(a: &Answer, s: &Step) -> Value {
    json!({
        "id": s.id,
        "journey": a.journey,
        "subject": a.subject,
        "who": s.who,
        "says": s.says,
        "commands": s.commands,
        "touches": s.touches,
        "how": s.how,
    })
}

/// What a quest-log item's step may run: the item's own record, the
/// checks and the gate its change lands through.
pub(crate) const ROW_CLI: &str = "gp mechanic show GAME ROW, gp mechanic start GAME ROW, gp mechanic busy GAME ROW, gp mechanic log GAME ROW, gp mechanic say GAME ROW, gp mechanic queue GAME ROW, gp mechanic probe GAME, gp clip presses GAME, gp corpus import GAME, gp model gate GAME";

/// What a row of the tool's work runs besides: the gate it lands through.
pub(crate) const TOOL_ROW_CLI: &str = ", gp tool gate --patch FILE";

/// What a game's quest-log row claims besides itself: the scoreboard line
/// its target names (`line:GAME/LINE`, the line lower case, as targets are
/// read), else the whole game (`game:GAME`).
pub(crate) fn claim_of(game: &str, row: &Value) -> String {
    match row["target"]
        .as_str()
        .and_then(|t| crate::scoreboard::parse_target(t).ok())
    {
        Some(t) => format!("line:{game}/{}", t.line),
        None => format!("game:{game}"),
    }
}

/// Whether claim `t` clashes with one in `taken`: the same claim, or a
/// game's whole claim (`game:GAME`) and a line of it (`line:GAME/LINE`).
pub(crate) fn clashes(taken: &BTreeSet<String>, t: &str) -> bool {
    if taken.contains(t) {
        return true;
    }
    if let Some(g) = t.strip_prefix("game:") {
        let lines = format!("line:{g}/");
        return taken.iter().any(|x| x.starts_with(&lines));
    }
    if let Some((g, _)) = t.strip_prefix("line:").and_then(|l| l.split_once('/')) {
        return taken.contains(&format!("game:{g}"));
    }
    false
}

/// The quest-log items that wait for work, as steps of the agent's: each
/// game's rows that are next or being worked on, in the log's order, one
/// step each, touching `row:GAME/ROW`. A step's id holds while the row's
/// status does, so a worker that ends with its row still open isn't
/// started again on it until the row's status or note changes.
pub(crate) fn row_steps(paths: &gp_evidence::Paths, games: Option<&Path>) -> Vec<Value> {
    let dir = crate::roms_cmd::games_dir(games);
    let all = dir
        .as_deref()
        .map(crate::next::games_in)
        .unwrap_or_default();
    let only = gp_mcp::steps::settings(&paths.home).games;
    let mut out = Vec::new();
    for g in all.iter().filter(|g| only.is_empty() || only.contains(g)) {
        let Ok(f) = crate::mechanic::session::queue_file(paths, g) else {
            continue;
        };
        for r in crate::mechanic::session::read_queue(&f) {
            let Some(row) = r["mechanic"].as_str() else {
                continue;
            };
            let status = r["status"].as_str().unwrap_or("");
            // The owner's yes to a look waits for gp serve's runner to lock
            // it (serve/runner.rs, run_confirms): no worker's.
            if r["confirmed"].is_object() {
                continue;
            }
            // The person's turn is the last in the row's talk: they answered
            // or asked, so the row is the agent's whatever its status says.
            let talk = std::fs::read_to_string(
                paths
                    .home
                    .join("mechanics")
                    .join(g)
                    .join(row)
                    .join("talk.ndjson"),
            )
            .unwrap_or_default();
            let last: Option<Value> = talk
                .lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .and_then(|l| serde_json::from_str(l).ok());
            let answered =
                last.as_ref().is_some_and(|t| t["by"] == json!("player")) && status != "done";
            if !answered && !matches!(status, "next" | "agent working") {
                continue;
            }
            let turns = talk.lines().filter(|l| !l.trim().is_empty()).count();
            let cli = ROW_CLI.replace("GAME", g).replace("ROW", row);
            let note = r["note"].as_str().unwrap_or("");
            let note = if answered {
                format!(
                    "The person answered in the row's talk: \"{}\". Act on it now. {note}",
                    crate::next::cut(
                        last.as_ref().and_then(|t| t["text"].as_str()).unwrap_or(""),
                        300
                    )
                )
            } else {
                note.to_owned()
            };
            let note = note.as_str();
            let says = if note.is_empty() {
                format!("Work on {row}.")
            } else {
                format!("Work on {row}: {}", crate::next::cut(note, 200))
            };
            // A row of the tool's work holds only its row: each worker has a
            // gameprinter worktree of its own, and the tool gate merges each
            // patch onto the tip (a conflict sends the row back to be redone
            // there), so tool rows run side by side up to the runner's limit.
            // A game's row with a target holds the scoreboard line it names
            // (`line:GAME/LINE`), so rows of other lines of the game run
            // beside it: each worker's tree has the game's pending patches
            // (serve/runner.rs, pending_patches) and the gate's order settles
            // their patches. A game's row with no target holds the whole
            // game, which every line of it clashes with ([`clashes`]).
            let tools = r["repo"] == json!("tools");
            let touches = if tools {
                vec![format!("row:{g}/{row}")]
            } else {
                vec![format!("row:{g}/{row}"), claim_of(g, &r)]
            };
            // A tool row lands through the tool gate.
            let cli = if r["repo"] == json!("tools") {
                format!("{cli}{TOOL_ROW_CLI}")
            } else {
                cli
            };
            let mut commands = commands_of(&cli);
            commands.push("*".into());
            // The note is in the key: a row reopened with new words is a new
            // step, which the runner starts again.
            let key = json!({"journey": "quest log", "game": g, "row": row, "status": status, "note": note, "turns": turns});
            let h = blake3::hash(key.to_string().as_bytes()).to_hex();
            out.push(json!({
                "id": format!("s{}", &h[..12]),
                "journey": "quest log",
                "subject": g,
                "who": "agent",
                "says": says,
                "commands": commands,
                "touches": touches,
                "how": {"cli": cli},
                "repo": if tools { "tools" } else { "games" },
            }));
        }
    }
    out
}

/// Every answer the home has: each game's port and each of the person's
/// copies.
pub(crate) fn answers(paths: &gp_evidence::Paths, games: Option<&Path>) -> Vec<Answer> {
    let mut out = Vec::new();
    let dir = crate::roms_cmd::games_dir(games);
    let all = dir
        .as_deref()
        .map(crate::next::games_in)
        .unwrap_or_default();
    for g in &all {
        if let Ok(f) = crate::next::gather(paths, games, Some(g), None) {
            out.push(crate::next::answer(&f));
        }
    }
    if all.is_empty()
        && let Ok(f) = crate::next::gather(paths, games, None, None)
    {
        out.push(crate::next::answer(&f));
    }
    use crate::journeys as j;
    let copies = j::list(&paths.home, games);
    for c in &copies {
        out.push(j::answer(
            &j::gather(paths, &copies, c),
            &j::around(&paths.home, c),
        ));
    }
    out
}

/// The open steps of every answer, no two touching the same thing: an
/// answer's steps that touch something an earlier answer's open step
/// does (the page, a card of every game) are left for later, and so are
/// steps that touch what a step the runner's live workers hold (`held`)
/// touches, but for that step itself.
pub(crate) fn open_of(answers: &[Answer], held: &[Value]) -> Vec<Value> {
    let ids: BTreeSet<&str> = held.iter().filter_map(|s| s["id"].as_str()).collect();
    let mut taken: BTreeSet<String> = held
        .iter()
        .flat_map(|s| s["touches"].as_array().cloned().unwrap_or_default())
        .filter_map(|t| t.as_str().map(str::to_owned))
        .collect();
    let mut out = Vec::new();
    for a in answers {
        for s in &a.open {
            if ids.contains(s.id.as_str()) || s.touches.iter().all(|t| !clashes(&taken, t)) {
                taken.extend(s.touches.iter().cloned());
                out.push(listed(a, s));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::next::step;

    #[test]
    fn a_command_line_names_the_commands_it_runs() {
        assert_eq!(
            commands_of("gp mechanic show g row, then gp mechanic say g row WORDS"),
            ["mechanic show", "mechanic say"]
        );
        assert_eq!(commands_of("gp corpus import g --write"), ["corpus import"]);
        assert_eq!(
            commands_of("gp progress --template g --run g --measure"),
            ["progress"]
        );
        // A group alone, or no gp at all, runs nothing.
        assert!(commands_of("gp mechanic").is_empty());
        assert!(commands_of("LET THE TOOLS TRY").is_empty());
    }

    #[test]
    fn the_id_holds_while_the_facts_do_and_the_background_is_left_out() {
        let s = step(Who::Agent, "Add the clips.").cli("gp corpus import g --write");
        let f1 = json!({"not_kept": ["a 1"], "background": {"on": true, "jobs": 3}});
        let f2 = json!({"not_kept": ["a 1"], "background": {"on": false, "jobs": 0}});
        let f3 = json!({"not_kept": ["a 1", "a 2"], "background": {"on": false, "jobs": 0}});
        let id = id_of("port", Some("g"), &s, &f1);
        assert!(id.starts_with('s') && id.len() == 13, "{id}");
        assert_eq!(id, id_of("port", Some("g"), &s, &f2));
        assert_ne!(id, id_of("port", Some("g"), &s, &f3));
        assert_ne!(id, id_of("port", Some("h"), &s, &f1));
    }

    #[test]
    fn two_steps_that_touch_the_same_thing_are_not_both_open() {
        let mut steps = vec![
            step(Who::Agent, "Add the clips.").cli("gp corpus import g --write"),
            step(Who::Tools, "Score them."),
            step(Who::Agent, "Answer the hand-off.").touch("row:g/jump"),
            step(Who::Person, "Play the jump.").act(Act::PlayFrom { row: "jump".into() }),
            step(Who::Person, "Press YES on the card."),
        ];
        let open = settle("port", Some("g"), &json!({}), &mut steps);
        let says: Vec<&str> = open.iter().map(|s| s.says.as_str()).collect();
        // The tools' step touches the game's folder as the first does.
        assert_eq!(
            says,
            [
                "Add the clips.",
                "Answer the hand-off.",
                "Press YES on the card."
            ]
        );
        assert_eq!(open[0].touches, ["game:g"]);
        assert_eq!(open[0].commands, ["corpus import"]);
        assert!(steps.iter().all(|s| !s.id.is_empty()));
        let ids: BTreeSet<&str> = steps.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids.len(), steps.len(), "every step has its own id");
    }
}
