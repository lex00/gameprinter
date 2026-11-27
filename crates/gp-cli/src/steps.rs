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

/// What a row gp loop health filed runs besides: the set-aside its problem
/// names for the games checkout's local changes, a card for the person's
/// yes. A step's `*` covers no ask, so without it the row's worker had
/// nothing to raise that card with (docs/agents.md).
pub(crate) const HEALTH_ROW_CLI: &str = ", gp loop set-aside GAME";

/// The game's scoreboard lines as a row's claim reads them: each line's
/// name, lower case as targets are read, and the corpus clip it measures.
pub(crate) fn board_lines(game_dir: &Path) -> Vec<(String, String)> {
    crate::scoreboard::lines(game_dir)
        .and_then(Result::ok)
        .unwrap_or_default()
        .into_iter()
        .map(|l| (l.name.to_lowercase(), l.clip))
        .collect()
}

/// The files whose change is the whole game's: a row whose note or where
/// it came from names one of them claims the game.
const WHOLE_GAME: [&str; 3] = ["model.toml", "mapping.toml", "ext/"];

/// What a game's quest-log row claims besides itself, the narrowest thing
/// that holds its work: the scoreboard line its target names
/// (`line:GAME/LINE`, the line lower case, as targets are read), else the
/// line of the clip it was filed with (its `clip`, a corpus clip one of
/// `lines` measures) or of the gap it was found on (`line`, from `gp
/// picture --propose --line`); and with it the line of every corpus clip
/// it was seen in (`seen_in`, and its `clip`), or `clip:GAME/ID` for a clip
/// no line measures. Two rows seen in one clip claim the same thing, so the
/// second waits for the first's worker and starts from its pending patch
/// (two rows of one cause on two lines' targets went to two workers who
/// each brought the same fix: docs/loop.md). With
/// none of these, a row whose note or from names model.toml, mapping.toml
/// or ext/ claims the whole game (`game:GAME`); else nothing but the row.
pub(crate) fn claim_of(game: &str, row: &Value, lines: &[(String, String)]) -> Vec<String> {
    let line = |l: &str| format!("line:{game}/{}", l.trim().to_lowercase());
    let line_of_clip = |id: &str| lines.iter().find(|(_, c)| c == id).map(|(n, _)| line(n));
    let mut out: Vec<String> = Vec::new();
    let mut add = |c: String| {
        if !out.contains(&c) {
            out.push(c);
        }
    };
    let clip = row["clip"]["id"].as_str();
    if let Some(t) = row["target"]
        .as_str()
        .and_then(|t| crate::scoreboard::parse_target(t).ok())
    {
        add(line(&t.line));
    } else if let Some(l) = clip.and_then(line_of_clip) {
        add(l);
    } else if let Some(l) = row["line"].as_str().filter(|l| !l.trim().is_empty()) {
        add(line(l));
    }
    let seen = row["seen_in"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str);
    for id in clip.into_iter().chain(seen) {
        let id = id.trim();
        if !id.is_empty() {
            add(line_of_clip(id).unwrap_or_else(|| format!("clip:{game}/{id}")));
        }
    }
    if out.is_empty() {
        let said = format!(
            "{} {}",
            row["note"].as_str().unwrap_or(""),
            row["from"].as_str().unwrap_or("")
        );
        if WHOLE_GAME.iter().any(|f| said.contains(f)) {
            out.push(format!("game:{game}"));
        }
    }
    out
}

/// Whether a row is taken by hand: "agent working" with a worker that
/// isn't one of the loop's (`loop-ID`), set by a session that works it
/// itself (`gp mechanic queue GAME ROW --status "agent working" --worker
/// NAME`). The loop starts no worker on it and `gp loop health` doesn't
/// call it stale; another status hands it back (a worker repeated a row a
/// session was doing by hand: docs/loop.md).
pub(crate) fn taken_by_hand(row: &Value) -> bool {
    row["status"] == json!("agent working")
        && row["worker"]
            .as_str()
            .map(str::trim)
            .is_some_and(|w| !w.is_empty() && !w.starts_with("loop-"))
}

/// Whether a step claims a whole game (`game:GAME`).
pub(crate) fn claims_game(step: &Value) -> bool {
    step["touches"].as_array().is_some_and(|t| {
        t.iter()
            .any(|t| t.as_str().is_some_and(|t| t.starts_with("game:")))
    })
}

/// Why open step `s` waits for a live worker's step (`held`), in words:
/// the first of them whose claim clashes with one of `s`'s, as "waits for
/// ROW, which holds the game" (or "the line LINE", or "the row"). None
/// when nothing live holds what it claims.
pub(crate) fn waits_for(s: &Value, held: &[Value]) -> Option<String> {
    let mine: Vec<&str> = s["touches"]
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .collect();
    for h in held {
        if h["id"].is_string() && h["id"] == s["id"] {
            continue;
        }
        let theirs: BTreeSet<String> = h["touches"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|t| t.as_str().map(str::to_owned))
            .collect();
        // The row's own worker, on a step of its row from before its
        // status changed, is the row at work, not a wait.
        if mine
            .iter()
            .any(|t| t.starts_with("row:") && theirs.contains(*t))
        {
            continue;
        }
        // What it holds that this step claims: the game, a line, the row.
        let Some(x) = theirs.iter().find(|x| {
            mine.iter()
                .any(|t| clashes(&BTreeSet::from([(*x).clone()]), t))
        }) else {
            continue;
        };
        let row = theirs
            .iter()
            .find_map(|t| t.strip_prefix("row:"))
            .map(|r| r.split_once('/').map_or(r, |(_, r)| r).to_owned())
            .unwrap_or_else(|| h["id"].as_str().unwrap_or("another step").to_owned());
        let what = if x.starts_with("game:") {
            "the game".to_owned()
        } else if let Some((_, l)) = x.strip_prefix("line:").and_then(|l| l.split_once('/')) {
            format!("the line {l}")
        } else if let Some((_, c)) = x.strip_prefix("clip:").and_then(|c| c.split_once('/')) {
            format!("the clip {c}")
        } else {
            "the row".to_owned()
        };
        return Some(format!("Waits for {row}, which holds {what}."));
    }
    None
}

/// Whether claim `t` clashes with one in `taken`: the same claim, or a
/// game's whole claim (`game:GAME`) and a line or clip of it
/// (`line:GAME/LINE`, `clip:GAME/ID`).
pub(crate) fn clashes(taken: &BTreeSet<String>, t: &str) -> bool {
    if taken.contains(t) {
        return true;
    }
    if let Some(g) = t.strip_prefix("game:") {
        let (lines, clips) = (format!("line:{g}/"), format!("clip:{g}/"));
        return taken
            .iter()
            .any(|x| x.starts_with(&lines) || x.starts_with(&clips));
    }
    if let Some((g, _)) = ["line:", "clip:"]
        .iter()
        .find_map(|k| t.strip_prefix(k))
        .and_then(|l| l.split_once('/'))
    {
        return taken.contains(&format!("game:{g}"));
    }
    false
}

/// Whether quest-log row `r` has no measurable done: neither a target (a
/// measure on the scoreboard, which closes it) nor `measure_needed` (why
/// it has none and what would measure it). A row the loop's own trouble
/// filed has its measure in the trouble itself: it closes when its
/// problem is gone (crate::trouble). The runner starts no worker on a row
/// with no measure, since a worker that picks its own finish line picks a
/// near one (issue 55).
pub(crate) fn no_measure(r: &Value) -> bool {
    let has = |k: &str| r[k].as_str().is_some_and(|v| !v.trim().is_empty());
    !has("target") && !has("measure_needed") && !r["trouble"].is_object()
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
    let s = gp_mcp::steps::settings(&paths.home);
    let only = &s.games;
    let mut out = Vec::new();
    for g in all.iter().filter(|g| only.is_empty() || only.contains(g)) {
        let Ok(f) = crate::mechanic::session::queue_file(paths, g) else {
            continue;
        };
        let mut lines: Option<Vec<(String, String)>> = None;
        for r in crate::mechanic::session::read_queue(&f) {
            let Some(row) = r["mechanic"].as_str() else {
                continue;
            };
            // The game's work is held (gp loop set --hold): its game rows
            // wait, answered or not, until the owner lifts it; its tool rows
            // go on.
            if s.held(g) && r["repo"] != json!("tools") {
                continue;
            }
            let status = r["status"].as_str().unwrap_or("");
            // The owner's yes to a look waits for gp serve's runner to lock
            // it (serve/runner.rs, run_confirms): no worker's.
            if r["confirmed"].is_object() {
                continue;
            }
            // A session works the row by hand ([`taken_by_hand`]): no
            // worker's, until another status hands it back.
            if taken_by_hand(&r) {
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
            // A row in line was parked (on a card, a row, a stop, a duplicate
            // NO): the person's words written before the park don't wake it,
            // words after it do (their COMMENT, a card's answer box). Its
            // park time is `parked_ms`, else when it last changed.
            let parked_at = r["parked_ms"]
                .as_u64()
                .or_else(|| r["changed_ms"].as_u64())
                .unwrap_or(0);
            let after_park = |t: &Value| t["at_ms"].as_u64().is_some_and(|at| at > parked_at);
            let woke = |t: &Value| status != "in line" || after_park(t);
            let by_person = last.as_ref().filter(|t| t["by"] == json!("player"));
            let answered = status != "done" && by_person.is_some_and(woke);
            if !answered && !matches!(status, "next" | "agent working") {
                continue;
            }
            let turns = talk.lines().filter(|l| !l.trim().is_empty()).count();
            let cli = ROW_CLI.replace("GAME", g).replace("ROW", row);
            let note = r["note"].as_str().unwrap_or("");
            // A row the answer set next has the words on its note already
            // (mechanic::session::answered_row).
            let note = if answered && !note.starts_with(crate::mechanic::session::ANSWERED) {
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
            // A game's row holds the narrowest thing its work needs
            // ([`claim_of`]): the scoreboard line its target or clip is on
            // (`line:GAME/LINE`) and the lines of the clips it was seen in,
            // so rows of other lines of the game run beside it; the whole game only for a row that names
            // model.toml, mapping.toml or ext/, which every line of it
            // clashes with ([`clashes`]); else only itself. Each worker's
            // tree has the game's pending patches (serve/runner.rs,
            // pending_patches) and the gate's order settles their patches.
            let tools = r["repo"] == json!("tools");
            let mut touches = vec![format!("row:{g}/{row}")];
            if !tools {
                let lines = lines.get_or_insert_with(|| {
                    dir.as_deref()
                        .map(|d| board_lines(&d.join(g)))
                        .unwrap_or_default()
                });
                touches.extend(claim_of(g, &r, lines));
            }
            // A tool row lands through the tool gate.
            let cli = if r["repo"] == json!("tools") {
                format!("{cli}{TOOL_ROW_CLI}")
            } else {
                cli
            };
            let cli = if r["trouble"]["kind"] == json!("health") {
                format!("{cli}{}", HEALTH_ROW_CLI.replace("GAME", g))
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
                // The row's measure, for the runner (issue 55): a row with
                // neither a target nor words for what would measure it
                // gets no worker ([`no_measure`]).
                "target": r["target"],
                "measure_needed": r["measure_needed"],
                "no_measure": no_measure(&r),
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
    let timing = std::env::var_os("GP_TIME_STEPS").is_some();
    for g in &all {
        let t = std::time::Instant::now();
        if let Ok(f) = crate::next::gather(paths, games, Some(g), None) {
            out.push(crate::next::answer(&f));
        }
        if timing {
            eprintln!("answers: gather {g} {:?}", t.elapsed());
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

    /// The owner's rule (2026-11-22): the person's words on a parked row
    /// reach a worker (their COMMENT, a card's answer box), unless the row
    /// was parked after they wrote them. A comment on a parked row gives a
    /// step; parking it after the comment gives none; a second comment
    /// gives one again; a row that goes in line is parked from then.
    #[test]
    fn a_comment_on_a_parked_row_is_a_step_unless_the_park_came_after() {
        use crate::mechanic::session::{queue_file, read_queue, write_queue};
        let t = tempfile::tempdir().unwrap();
        let p = crate::test_paths(&t.path().join("home"));
        let games = t.path().join("games");
        let g = games.join("g");
        std::fs::create_dir_all(&g).unwrap();
        std::fs::write(g.join(gp_model::MODEL_FILE), "# a model\n").unwrap();
        let q = queue_file(&p, "g").unwrap();
        let d = p.home.join("mechanics").join("g").join("parked");
        std::fs::create_dir_all(&d).unwrap();
        let talk = d.join("talk.ndjson");
        let turn = |at: u64| format!("{{\"by\":\"player\",\"text\":\"the rim\",\"at_ms\":{at}}}\n");
        let r = |at: u64| vec![json!({"mechanic": "parked", "status": "in line", "parked_ms": at})];
        let named = || -> Vec<String> {
            row_steps(&p, Some(&games))
                .iter()
                .filter_map(|s| s["touches"][0].as_str().map(str::to_owned))
                .collect()
        };
        write_queue(&q, &r(1000)).unwrap();
        std::fs::write(&talk, turn(2000)).unwrap();
        assert_eq!(named(), ["row:g/parked"], "a comment after the park");
        write_queue(&q, &r(3000)).unwrap();
        assert!(named().is_empty(), "parked after the comment");
        std::fs::write(&talk, format!("{}{}", turn(2000), turn(4000))).unwrap();
        assert_eq!(named(), ["row:g/parked"], "a second comment");
        // A row set in line with no park time of its own is parked now.
        let next = json!({"mechanic": "parked", "status": "next", "note": "card up"});
        write_queue(&q, std::slice::from_ref(&next)).unwrap();
        let mut back = next;
        back["status"] = json!("in line");
        write_queue(&q, &[back]).unwrap();
        let at = read_queue(&q)[0]["parked_ms"].as_u64().unwrap();
        assert!(at > 4000, "{at}");
        assert!(named().is_empty());
    }

    /// A held game's game rows make no step while its tool rows do, and
    /// another game's rows go on; lifting the hold gives the rows back.
    #[test]
    fn a_held_games_game_rows_wait_and_its_tool_rows_go_on() {
        let t = tempfile::tempdir().unwrap();
        let p = crate::test_paths(&t.path().join("home"));
        let games = t.path().join("games");
        for g in ["hilltop", "rooftop"] {
            std::fs::create_dir_all(games.join(g)).unwrap();
            std::fs::write(games.join(g).join("package.toml"), "").unwrap();
            let q = json!([
                {"mechanic": "fix-art", "status": "next", "note": "a game row"},
                {"mechanic": "fix-tool", "status": "next", "note": "a tool row", "repo": "tools"},
            ]);
            let f = crate::mechanic::session::queue_file(&p, g).unwrap();
            crate::mechanic::session::write_queue(&f, q.as_array().unwrap()).unwrap();
        }
        let rows = |p: &gp_evidence::Paths| -> Vec<String> {
            row_steps(p, Some(&games))
                .iter()
                .filter_map(|s| s["touches"][0].as_str().map(str::to_owned))
                .collect()
        };
        let every = [
            "row:hilltop/fix-art",
            "row:hilltop/fix-tool",
            "row:rooftop/fix-art",
            "row:rooftop/fix-tool",
        ];
        assert_eq!(rows(&p), every);
        let mut s = gp_mcp::steps::settings(&p.home);
        s.hold = vec!["hilltop".into()];
        gp_mcp::steps::save(&p.home, &s).unwrap();
        assert_eq!(
            rows(&p),
            [
                "row:hilltop/fix-tool",
                "row:rooftop/fix-art",
                "row:rooftop/fix-tool"
            ]
        );
        s.hold = vec!["all".into()];
        gp_mcp::steps::save(&p.home, &s).unwrap();
        assert_eq!(rows(&p), ["row:hilltop/fix-tool", "row:rooftop/fix-tool"]);
        s.hold.clear();
        gp_mcp::steps::save(&p.home, &s).unwrap();
        assert_eq!(rows(&p), every);
    }

    /// A row gp loop health filed has gp loop set-aside of its game among its
    /// step's commands, the card its problem names; another tool row and a
    /// trouble row from the page don't.
    #[test]
    fn a_health_rows_step_can_set_aside_the_games_checkout() {
        let t = tempfile::tempdir().unwrap();
        let p = crate::test_paths(&t.path().join("home"));
        let games = t.path().join("games");
        std::fs::create_dir_all(games.join("g")).unwrap();
        std::fs::write(games.join("g").join("package.toml"), "").unwrap();
        let q = json!([
            {"mechanic": "trouble-checkout", "status": "next", "repo": "tools", "trouble": {"kind": "health"}},
            {"mechanic": "trouble-page", "status": "next", "repo": "tools", "trouble": {"kind": "page"}},
            {"mechanic": "fix-tool", "status": "next", "repo": "tools"},
        ]);
        let f = crate::mechanic::session::queue_file(&p, "g").unwrap();
        crate::mechanic::session::write_queue(&f, q.as_array().unwrap()).unwrap();
        let steps = row_steps(&p, Some(&games));
        let sets_aside = |row: &str| {
            let s = steps
                .iter()
                .find(|s| s["touches"][0] == json!(format!("row:g/{row}")))
                .unwrap();
            assert!(
                s["commands"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("tool gate")),
                "{s}"
            );
            s["commands"]
                .as_array()
                .unwrap()
                .contains(&json!("loop set-aside"))
        };
        assert!(sets_aside("trouble-checkout"));
        assert!(!sets_aside("trouble-page"));
        assert!(!sets_aside("fix-tool"));
        let s = &steps[0];
        assert!(
            s["how"]["cli"]
                .as_str()
                .unwrap()
                .ends_with(", gp loop set-aside g"),
            "{s}"
        );
    }

    /// A row a session takes by hand ("agent working" with a worker that
    /// isn't the loop's) makes no step, answered or not; a loop worker's row
    /// and a row with no worker do, and another status hands it back.
    #[test]
    fn a_row_taken_by_hand_makes_no_step() {
        let t = tempfile::tempdir().unwrap();
        let p = crate::test_paths(&t.path().join("home"));
        let games = t.path().join("games");
        std::fs::create_dir_all(games.join("g")).unwrap();
        std::fs::write(games.join("g").join("package.toml"), "").unwrap();
        let f = crate::mechanic::session::queue_file(&p, "g").unwrap();
        let rows = |q: Value| -> Vec<String> {
            crate::mechanic::session::write_queue(&f, q.as_array().unwrap()).unwrap();
            row_steps(&p, Some(&games))
                .iter()
                .filter_map(|s| s["touches"][0].as_str().map(str::to_owned))
                .collect()
        };
        let by_hand =
            json!({"mechanic": "space-bar", "status": "agent working", "worker": "a-session"});
        assert!(taken_by_hand(&by_hand));
        // The person's words in its talk don't hand it to a worker.
        let d = p.home.join("mechanics/g/space-bar");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(
            d.join("talk.ndjson"),
            "{\"by\":\"player\",\"text\":\"the bar\",\"at_ms\":5}\n",
        )
        .unwrap();
        let q = json!([
            by_hand,
            {"mechanic": "a", "status": "agent working", "worker": "loop-s0123"},
            {"mechanic": "b", "status": "agent working"},
            {"mechanic": "c", "status": "next", "worker": "a-session"},
        ]);
        assert_eq!(rows(q), ["row:g/a", "row:g/b", "row:g/c"]);
        let back = json!([{"mechanic": "space-bar", "status": "next", "worker": "a-session"}]);
        assert_eq!(rows(back), ["row:g/space-bar"]);
    }

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
