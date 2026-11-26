//! `gp loop`: the loop's steps as the engine emits them (crate::steps,
//! docs/loop.md). `gp loop steps` lists the open steps of every game and
//! copy in the home, each with its id, who does it, what it touches and
//! the commands it runs. It reads files only, as `next` does. `gp loop
//! set` changes the home's loop settings (`$GP_HOME/loop.toml`,
//! gp_mcp::steps): whether gp_run needs a step for a write, and gp serve's
//! runner. `gp loop wrong` marks an open step wrong, which halts the loop,
//! and `gp loop resume` clears the halt once a landed change names the
//! objection (crate::wrong). Only the person runs these three: gp_run
//! refuses them.

use anyhow::Result;
use clap::Subcommand;
use serde_json::{Value, json};

use crate::Outcome;

#[derive(Subcommand)]
pub enum LoopCmd {
    /// The open steps of every game and copy in the home (docs/loop.md):
    /// each with its id (a hash of its command and the facts it was
    /// decided from), who does it, what it says, what it touches and the gp
    /// commands it runs. No two open steps touch the same thing. gp_run's
    /// `step` cites one of these ids. Reads files only.
    Steps,
    /// Is the loop set up to run on its own? The checks the agent watching
    /// the loop runs instead of doing the work: a gp serve answering for
    /// the home on the published build, the runner on with a worker
    /// command, no halt, the yes cards taken and none failing, no row
    /// marked working with no worker on it, the games checkout clean
    /// (local changes block every landing), 20 GB free on the home's disk
    /// (else the biggest folders gp loop clean clears), and, when nothing runs, what
    /// the loop waits on. Each problem says how to fix it. Then each game's
    /// scoreboard (GAME/scoreboard.toml's lines: the game state,
    /// background and sprites matched, and how each moved), as gp serve
    /// last measured it. Exits 1 on a problem. Reads only.
    Health,
    /// Clear the build folders the loop left: each ended worker's cargo
    /// target ($GP_HOME/work/target/loop-STEP, or under $GP_TARGET_DIR)
    /// and the folders of $GP_HOME/work/regenerate no run touched in six
    /// hours, and of those still in use (the gate's home's too) the score
    /// cache files older than a day, and print what each freed. A live
    /// worker's build is kept.
    /// gp serve's runner removes a worker's build itself when its step
    /// ends; this is for what is left (gp loop health says PROBLEM under
    /// 20 GB free).
    Clean {
        /// Also clear the tool gate's build folders (tool/target and
        /// tool/target-tests in its home, $GP_GATE_HOME else
        /// ~/.gameprinter/gate), unless a gate runs now; its next run
        /// builds from nothing. The build in an older gp's gate tree
        /// (tool/tree/target) goes even while a tool gate runs.
        #[arg(long)]
        gate: bool,
        /// List what would be cleared, with sizes, and remove nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Set aside the games checkout's local changes under GAME/ (the files
    /// gp loop health names under its problem), so a landing there can go
    /// on: they go into a git stash of the checkout named for the game and
    /// the day, files git doesn't track too, and `git stash apply` brings
    /// them back. Nothing is removed. Through gp_run it needs the person's
    /// yes (its card lists the files), and runs in the person's checkout,
    /// never a worker's worktree.
    SetAside {
        /// The game's folder in the games checkout ("rooftop").
        game: String,
        /// List the files it would set aside, and change nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Change the home's loop settings ($GP_HOME/loop.toml) and print them.
    /// With no flag, print them. Only the person runs this: gp_run refuses
    /// it, so an agent can't turn the loop's rules off.
    Set {
        /// on: gp_run runs a call that writes, lands or publishes only
        /// citing an open step that runs its command (off by default).
        #[arg(long, value_parser = ["on", "off"])]
        steps_required: Option<String>,
        /// on: gp serve's runner starts a worker on each open step of the
        /// agent's, up to --max-workers (off by default; each worker is an
        /// agent, which costs).
        #[arg(long, value_parser = ["on", "off"])]
        runner: Option<String>,
        /// The most workers the runner has at once (default 2).
        #[arg(long)]
        max_workers: Option<u32>,
        /// A note on spending the page shows beside the runner ("about 5
        /// dollars a day"); the runner keeps no count of money yet.
        #[arg(long)]
        spending: Option<String>,
        /// The model every claude worker runs on (default opus); "" leaves
        /// it to the person's own Claude settings.
        #[arg(long)]
        model: Option<String>,
        /// The workers' effort (default high); "" leaves it to the person's
        /// own.
        #[arg(long, value_parser = ["", "low", "medium", "high", "xhigh", "max"])]
        effort: Option<String>,
        /// The games whose quest-log items the loop works on, by name,
        /// comma-separated ("rooftop"); "all" for every game's.
        #[arg(long)]
        games: Option<String>,
        /// Hold the game work of these games, comma-separated ("rooftop"),
        /// or "all": the owner's "game work stops while the tooling is
        /// fixed". The runner starts no worker on their game rows (a worker
        /// at work goes on to its end), the finder makes no rows of their
        /// gaps, and no trouble is filed for their rows; their tool rows go
        /// on. Lift it with --go-on.
        #[arg(long, value_name = "GAMES")]
        hold: Option<String>,
        /// Lift the hold on these games, comma-separated, or "all": their
        /// game rows are the runner's again.
        #[arg(long, value_name = "GAMES")]
        go_on: Option<String>,
        /// End game GAME's port run ("all" for every game's): its workers
        /// stay on the gp the run started on, past the tool landings since,
        /// until its rows are done; after this its next worker starts on the
        /// newest published gp (crate::port_run).
        #[arg(long, value_name = "GAME")]
        move_on: Option<String>,
    },
    /// Mark an open step wrong, with why: the loop halts (next emits no
    /// steps, gp_run runs none, the runner starts no worker) until a change
    /// to the tool that names the objection's id lands through gp tool gate.
    /// The page's WRONG does the same. Only the person runs this.
    Wrong {
        /// The step's id, from gp loop steps or next.
        step: String,
        /// What is wrong with it, in the person's words.
        #[arg(long)]
        why: String,
    },
    /// Clear objection ID once a commit on the tool's branch adds a line
    /// naming it (a test that pins the rule change). The tool gate does this
    /// itself for each patch it lands; this is for one that landed another
    /// way or before. Run in the gameprinter checkout. Only the person runs
    /// this.
    Resume {
        /// The objection's id (w and 12 hex digits), from gp loop steps.
        id: String,
        /// The branch to look on (default: main).
        #[arg(long, default_value = "main")]
        onto: String,
    },
    /// The gp plugin's hooks for Claude Code (plugin/hooks/hooks.json,
    /// docs/plugin.md): reads the hook's JSON on stdin and prints its
    /// answer. session-start puts the loop's state and gp progress --next in
    /// the session; pre-tool-use refuses a hand edit in a game's folder of
    /// a games checkout; stop refuses the first stop while a yes waits to be
    /// used or a card raised in the last half hour waits for the person. A
    /// worker the runner started (GP_STEP) gets none of them. Reads only.
    Hook {
        /// The hook's event.
        #[arg(value_enum)]
        event: crate::loop_hook::HookEvent,
    },
}

/// A worker setting as the text says it: empty is the person's own.
fn or_own(v: &str) -> &str {
    if v.is_empty() { "the person's own" } else { v }
}

/// The games a `--hold` or `--go-on` names: comma-separated, "all" alone.
fn game_list(v: &str) -> Vec<String> {
    let names: Vec<String> = v
        .split(',')
        .map(|x| x.trim().to_owned())
        .filter(|x| !x.is_empty())
        .collect();
    if names.iter().any(|n| n == "all") {
        vec!["all".into()]
    } else {
        names
    }
}

/// The hold after `--hold` adds `hold` and `--go-on` lifts `go_on`; Err
/// when a game is lifted while every game's work is held.
fn new_hold(
    was: &[String],
    hold: Option<&str>,
    go_on: Option<&str>,
) -> std::result::Result<Vec<String>, String> {
    let mut h: Vec<String> = was.to_vec();
    if let Some(v) = go_on {
        let lift = game_list(v);
        if lift.iter().any(|g| g == "all") {
            h.clear();
        } else if h.iter().any(|g| g == "all") {
            return Err(format!(
                "every game's work is held: lift it with --go-on all (then --hold the games to keep held), not --go-on {}",
                lift.join(",")
            ));
        } else {
            h.retain(|g| !lift.contains(g));
        }
    }
    if let Some(v) = hold {
        let add = game_list(v);
        if add.iter().any(|g| g == "all") {
            h = add;
        } else if !h.iter().any(|g| g == "all") {
            for g in add {
                if !h.contains(&g) {
                    h.push(g);
                }
            }
        }
    }
    Ok(h)
}

/// The hold in words, for gp loop set and gp loop health.
fn hold_text(s: &gp_mcp::steps::Settings) -> Option<String> {
    if s.hold.is_empty() {
        return None;
    }
    let which = if s.hold.iter().any(|g| g == "all") {
        "every game".to_owned()
    } else {
        s.hold.join(", ")
    };
    Some(format!(
        "game work held for {which}: the runner starts no worker on their game rows and the finder makes none; their tool rows go on (gp loop set --go-on to lift it)"
    ))
}

fn settings_text(s: &gp_mcp::steps::Settings) -> String {
    let w = |b: bool| if b { "on" } else { "off" };
    format!(
        "steps required by gp_run: {}\nquest-log items of: {}{}\nrunner: {}, at most {} workers, model {}, effort {}{}",
        w(s.steps_required),
        if s.games.is_empty() {
            "every game".to_owned()
        } else {
            s.games.join(", ")
        },
        hold_text(s).map(|t| format!("\n{t}")).unwrap_or_default(),
        w(s.runner.on),
        s.runner.max_workers,
        or_own(&s.runner.model),
        or_own(&s.runner.effort),
        if s.runner.spending.is_empty() {
            String::new()
        } else {
            format!("; spending: {}", s.runner.spending)
        }
    )
}

/// The steps as JSON, for gp_run's check and the page.
pub(crate) fn steps_json(paths: &gp_evidence::Paths, games: Option<&std::path::Path>) -> Value {
    let t = std::time::Instant::now();
    let timing = std::env::var_os("GP_TIME_STEPS").is_some();
    let answers = crate::steps::answers(paths, games);
    if timing {
        eprintln!("steps_json: answers {:?}", t.elapsed());
    }
    // The runner's workers, newest first, with the live ones' steps.
    let mut workers = gp_mcp::steps::workers(&paths.home);
    workers.reverse();
    if timing {
        eprintln!("steps_json: workers {:?} ({})", t.elapsed(), workers.len());
    }
    let v = match crate::wrong::halt(&paths.home) {
        Some(h) => json!({"open": [], "halted": crate::wrong::halted_json(&h), "workers": workers}),
        None => json!({
            "open": open_with_rows(paths, games, &answers),
            "halted": null,
            "workers": workers,
        }),
    };
    if timing {
        eprintln!("steps_json: open rows {:?}", t.elapsed());
    }
    v
}

/// The engine's open steps, then the quest-log items' steps that touch
/// nothing those do (crate::steps::row_steps).
fn open_with_rows(
    paths: &gp_evidence::Paths,
    games: Option<&std::path::Path>,
    answers: &[crate::next::Answer],
) -> Vec<Value> {
    let mut open = crate::steps::open_of(answers, &gp_mcp::steps::held(&paths.home));
    // A row set back to the agent (status next) is the agent's: the
    // engine's step asking the person to play it gives way to the row's.
    let rows = crate::steps::row_steps(paths, games);
    let row_touch: std::collections::BTreeSet<String> = rows
        .iter()
        .flat_map(|s| s["touches"].as_array().cloned().unwrap_or_default())
        .filter_map(|t| {
            t.as_str()
                .filter(|t| t.starts_with("row:"))
                .map(str::to_owned)
        })
        .collect();
    open.retain(|s| {
        s["who"] != json!("person")
            || !s["touches"].as_array().is_some_and(|t| {
                t.iter()
                    .filter_map(Value::as_str)
                    .any(|t| row_touch.contains(t))
            })
    });
    // The engine's own steps on a game touch `game:NAME`, but the runner
    // never starts them (they are hints for an agent at the keyboard), so
    // they don't hold back the game's rows; the rows hold the game, or the
    // scoreboard line their target names, among themselves in the
    // runner's plan (crate::steps::clashes).
    let taken: std::collections::BTreeSet<String> = open
        .iter()
        .flat_map(|s| s["touches"].as_array().cloned().unwrap_or_default())
        .filter_map(|t| t.as_str().map(str::to_owned))
        .filter(|t| !t.starts_with("game:"))
        .collect();
    let free = |s: &Value| {
        s["touches"].as_array().is_some_and(|t| {
            t.iter()
                .filter_map(Value::as_str)
                .all(|t| !taken.contains(t))
        })
    };
    open.extend(
        crate::steps::row_steps(paths, games)
            .into_iter()
            .filter(free),
    );
    open
}

fn steps_text(v: &Value) -> String {
    if let Some(h) = v["halted"]["says"].as_str() {
        return h.to_owned();
    }
    let open = v["open"].as_array().cloned().unwrap_or_default();
    if open.is_empty() {
        return "No open steps.".into();
    }
    let mut out = vec![format!("{} open steps:", open.len())];
    for s in &open {
        let cmds = s["commands"]
            .as_array()
            .map(|c| {
                c.iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        out.push(format!(
            "  {} ({}, {}): {}{}",
            s["id"].as_str().unwrap_or("?"),
            s["who"].as_str().unwrap_or("?"),
            s["subject"].as_str().unwrap_or("the home"),
            s["says"].as_str().unwrap_or(""),
            if cmds.is_empty() {
                String::new()
            } else {
                format!(" [runs {cmds}]")
            }
        ));
    }
    out.join("\n")
}

pub(crate) fn run(cmd: &LoopCmd, paths: &gp_evidence::Paths) -> Result<Outcome> {
    match cmd {
        LoopCmd::Health => {
            let v = health(paths);
            let text = v["lines"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("\n");
            let bad = v["problems"].as_u64().unwrap_or(0) > 0;
            let mut o = Outcome::ok(v, text);
            if bad {
                o.exit = 1;
            }
            Ok(o)
        }
        LoopCmd::Clean { gate, dry_run } => {
            let home = &paths.home;
            let none = std::collections::BTreeSet::new();
            let gate_home = if *gate {
                Some(crate::gate::gate_home(None)?)
            } else {
                None
            };
            let g = gate_home.as_deref();
            // The gate's score cache is cleared with or without --gate: a
            // gate runs on without it, only scoring its clips again.
            let cache = match &gate_home {
                Some(h) => Some(h.clone()),
                None => crate::gate::gate_home(None).ok(),
            };
            let (done, skipped) = if *dry_run {
                (
                    crate::builds::clearable(home, &none, g, cache.as_deref()),
                    Vec::new(),
                )
            } else {
                crate::builds::clean(home, &none, g, cache.as_deref())
            };
            Ok(clean_outcome(home, *dry_run, &done, &skipped))
        }
        LoopCmd::SetAside { game, dry_run } => {
            let games = crate::roms_cmd::games_dir(None)
                .ok_or_else(|| anyhow::anyhow!("no games checkout: give $GP_GAMES_DIR"))?;
            let v = set_aside(&games, game, *dry_run, &crate::game_new::today())?;
            let text = v["says"].as_str().unwrap_or("").to_owned();
            Ok(Outcome::ok(v, text))
        }
        LoopCmd::Hook { event } => crate::loop_hook::run(*event, paths),
        LoopCmd::Steps => {
            let v = steps_json(paths, None);
            let text = steps_text(&v);
            Ok(Outcome::ok(v, text))
        }
        LoopCmd::Set {
            steps_required,
            runner,
            max_workers,
            spending,
            model,
            effort,
            games,
            hold,
            go_on,
            move_on,
        } => {
            let home = &paths.home;
            let was = gp_mcp::steps::settings(home);
            let held = new_hold(&was.hold, hold.as_deref(), go_on.as_deref())
                .map_err(anyhow::Error::msg)?;
            let mut moved = Vec::new();
            if let Some(g) = move_on {
                let g = g.trim();
                let which: Vec<String> = if g == "all" {
                    crate::port_run::all(home)
                        .iter()
                        .filter_map(|r| r["game"].as_str().map(str::to_owned))
                        .collect()
                } else {
                    vec![g.to_owned()]
                };
                for g in which {
                    if crate::port_run::end(home, &g) {
                        moved.push(g);
                    }
                }
            }
            let mut s = was;
            s.hold = held;
            if let Some(v) = steps_required {
                s.steps_required = v == "on";
            }
            if let Some(v) = runner {
                s.runner.on = v == "on";
            }
            if let Some(n) = max_workers {
                s.runner.max_workers = *n;
            }
            if let Some(w) = spending {
                s.runner.spending = w.trim().to_owned();
            }
            if let Some(m) = model {
                s.runner.model = m.trim().to_owned();
            }
            if let Some(e) = effort {
                s.runner.effort = e.clone();
            }
            if let Some(g) = games {
                s.games = if g.trim() == "all" {
                    Vec::new()
                } else {
                    g.split(',')
                        .map(|x| x.trim().to_owned())
                        .filter(|x| !x.is_empty())
                        .collect()
                };
            }
            let changed = games.is_some()
                || hold.is_some()
                || go_on.is_some()
                || steps_required.is_some()
                || runner.is_some()
                || max_workers.is_some()
                || spending.is_some()
                || model.is_some()
                || effort.is_some();
            if changed {
                gp_mcp::steps::save(home, &s).map_err(anyhow::Error::msg)?;
            }
            let mut text = settings_text(&s);
            if let Some(g) = move_on {
                text.push_str(&if moved.is_empty() {
                    format!("\nno port run of {} was on", g.trim())
                } else {
                    format!(
                        "\nmoved on: {}'s next worker starts on the newest gp",
                        moved.join(", ")
                    )
                });
            }
            let runs = crate::port_run::all(home);
            for r in &runs {
                text.push('\n');
                text.push_str(&crate::port_run::says(home, r));
            }
            Ok(Outcome::ok(
                json!({"settings": s, "file": gp_mcp::steps::settings_file(home), "changed": changed, "moved_on": moved, "port_runs": runs}),
                text,
            ))
        }
        LoopCmd::Wrong { step, why } => {
            let v = crate::wrong::mark(paths, None, step, why)?;
            let text = crate::wrong::says(&v);
            Ok(Outcome::ok(v, text))
        }
        LoopCmd::Resume { id, onto } => {
            let cwd = std::env::current_dir()?;
            let repo = std::path::PathBuf::from(crate::gate::git(
                &cwd,
                &["rev-parse", "--show-toplevel"],
            )?);
            let v = crate::wrong::resume(&paths.home, &repo, onto, id)?;
            let text = format!(
                "The loop resumes: objection {id} is cleared by {}.",
                v["cleared"]["commit"].as_str().unwrap_or("?")
            );
            Ok(Outcome::ok(v, text))
        }
    }
}

/// gp loop clean's outcome: what it cleared (or would), what it skipped
/// and the disk's free space after.
fn clean_outcome(
    home: &std::path::Path,
    dry_run: bool,
    done: &[crate::builds::Clear],
    skipped: &[String],
) -> Outcome {
    let gb = crate::builds::gb;
    let total: u64 = done.iter().map(|c| c.bytes).sum();
    let head = if done.is_empty() {
        "Nothing to clear.".to_owned()
    } else if dry_run {
        format!("Would clear {} in {} folders:", gb(total), done.len())
    } else {
        format!("Cleared {} in {} folders:", gb(total), done.len())
    };
    let mut lines = vec![head];
    for c in done {
        lines.push(format!(
            "  {} {}{}",
            gb(c.bytes),
            c.path.display(),
            if c.old_files {
                " (its files older than a day)"
            } else {
                ""
            }
        ));
    }
    for s in skipped {
        lines.push(format!("  skipped {s}"));
    }
    if let Some(f) = crate::builds::free_bytes(home) {
        lines.push(format!("{} free on the home's disk", gb(f)));
    }
    let cleared: Vec<Value> = done
        .iter()
        .map(|c| json!({"path": c.path, "bytes": c.bytes, "old_files": c.old_files}))
        .collect();
    Outcome::ok(
        json!({"dry_run": dry_run, "cleared": cleared, "bytes": total, "skipped": skipped}),
        lines.join("\n"),
    )
}

/// A card that failed in the last hour, as gp loop health's problem, unless
/// its row took it back (the runner set the row next with why, or a newer
/// card of the row has it): then the loop takes it up again and nothing is
/// wrong with the loop.
fn failed_card_problem(a: &Value, now: u64) -> Option<String> {
    if a["ran"]["state"] != json!("failed")
        || a["row_took_it"].as_bool() == Some(true)
        || now.saturating_sub(a["ran"]["done_ms"].as_u64().unwrap_or(0)) >= 3_600_000
    {
        return None;
    }
    let why = a["ran"]["lines"]
        .as_array()
        .and_then(|l| {
            l.iter()
                .rev()
                .filter_map(Value::as_str)
                .find(|l| !l.starts_with("(ran in"))
        })
        .unwrap_or("");
    Some(format!(
        "Card {} failed in the last hour: {}",
        a["id"].as_str().unwrap_or(""),
        crate::next::cut(why, 160)
    ))
}

/// The loop's health (`gp loop health`): each check's line, OK or PROBLEM
/// with how to fix it, and the count of problems. A problem the check
/// before printed too, a minute or more ago, is filed as a tools row of the
/// loop's own trouble (crate::trouble), and the open ones are counted.
pub(crate) fn health(paths: &gp_evidence::Paths) -> Value {
    health_in(paths, None)
}

/// [`health`] with the games checkout named (gp serve's runner runs it).
pub(crate) fn health_in(paths: &gp_evidence::Paths, games_dir: Option<&std::path::Path>) -> Value {
    let mut v = checks(paths, games_dir);
    let home = &paths.home;
    let problems: Vec<String> = v["lines"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter_map(|l| l.strip_prefix("PROBLEM  "))
        .map(str::to_owned)
        .collect();
    let Some(game) = crate::trouble::loop_game(home, games_dir) else {
        return v;
    };
    if crate::trouble::live_home(home) {
        crate::trouble::health_seen(paths, &game, &problems, gp_mcp::asks::now_ms());
    }
    if let Ok(f) = crate::mechanic::session::queue_file(paths, &game) {
        let q = crate::mechanic::session::read_queue(&f);
        let open = crate::trouble::open(&q);
        let line = if open.is_empty() {
            "OK       No row of the loop's own trouble is open".to_owned()
        } else {
            format!(
                "TROUBLE  {} row(s) of the loop's own trouble open in {game}'s quest log: {}",
                open.len(),
                open.iter()
                    .filter_map(|r| r["mechanic"].as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        if let Some(l) = v["lines"].as_array_mut() {
            l.push(json!(line));
        }
        v["trouble_open"] = json!(open.len());
    }
    v
}

/// What the published gp (`link`, $GP_HOME/bin/gp) lacks of the page's
/// files that go beside it (crate::publish::PAGES, in bin/gp-COMMIT.pages),
/// as a problem's words; None when it has them all, or no gp is published
/// as a link.
fn published_pages_problem(link: &std::path::Path) -> Option<String> {
    let real = std::fs::canonicalize(link).ok()?;
    let name = real.file_name()?.to_str()?.to_owned();
    if !name.starts_with("gp-") {
        return None;
    }
    let dir = real.with_file_name(format!("{name}.pages"));
    let missing: Vec<&str> = crate::publish::PAGES
        .into_iter()
        .filter(|rel| match *rel {
            "editor/dist" => !dir.join("editor/dist/index.html").is_file(),
            _ => !dir.join(rel).is_dir(),
        })
        .collect();
    if missing.is_empty() {
        return None;
    }
    Some(format!(
        "The published gp {name} has no {} in {}, so gp serve on it has no page of its own (no editor at /). The tool gate's next landing that changes more than words builds the editor and publishes them (a words-only landing publishes nothing); to publish them now, run `just publish` in a clean checkout of main.",
        missing.join(" and "),
        dir.display()
    ))
}

/// Where the published gp (`link`) stands against `branch` of `repo`: a
/// line when it is the branch's tip or behind it, a problem when its
/// commit isn't on the branch at all (a history rebased under the link),
/// None when no gp is published as a link or `repo` has no such branch.
fn published_against(
    link: &std::path::Path,
    repo: &std::path::Path,
    branch: &str,
) -> Option<Result<String, String>> {
    let real = std::fs::canonicalize(link).ok()?;
    let short = real.file_name()?.to_str()?.strip_prefix("gp-")?.to_owned();
    let git = |args: &[&str]| crate::gate::git(repo, args).ok();
    let tip = git(&[
        "rev-parse",
        "--verify",
        "-q",
        &format!("refs/heads/{branch}"),
    ])?;
    let at = &tip[..tip.len().min(12)];
    if tip.starts_with(&short) {
        return Some(Ok(format!("The published gp {short} is {branch}'s tip")));
    }
    // The commit on the branch the published build is of: its own, or one
    // with the very same files (a commit published and then landed again as
    // another, as a squash or a rebase onto the same tree does), whose page
    // is the published one.
    let mut base = short.clone();
    let mut is = format!("is on {branch},");
    if git(&["merge-base", "--is-ancestor", &short, &tip]).is_none() {
        let Some(same) = same_files_on(repo, &short, &tip) else {
            return Some(Err(format!(
                "The published gp {short} is of a commit {branch} doesn't hold (a history rebased under it?), so gp serve serves a page {branch} ({at}) may no longer have. The tool gate's next landing on {branch} moves the link; to move it now, run `just publish` in a clean checkout of {branch}."
            )));
        };
        let landed = &same[..12];
        if same == tip {
            return Some(Ok(format!(
                "The published gp {short} has the files of {branch}'s tip {at}, which landed them as a commit of its own"
            )));
        }
        is = format!(
            "has the files of {branch}'s {landed}, which landed them as a commit of its own, and is"
        );
        base = same;
    }
    let n = git(&["rev-list", "--count", &format!("{base}..{tip}")])?;
    let files = git(&["diff", "--name-only", &base, &tip])?;
    Some(Ok(if crate::tool_gate::docs_only(&files) {
        format!(
            "The published gp {short} {is} {n} commit(s) behind its tip {at} by words alone (a words-only landing publishes nothing)"
        )
    } else {
        format!(
            "The published gp {short} {is} {n} commit(s) behind its tip {at}, which change more than words (landed by hand, or a release's version): the tool gate's next landing publishes them, or `just publish` in a clean checkout of {branch}"
        )
    }))
}

/// The newest of the last 500 commits up to `tip` whose files (git's tree)
/// are `commit`'s, in full; None when there is none or git can't say.
fn same_files_on(repo: &std::path::Path, commit: &str, tip: &str) -> Option<String> {
    let git = |args: &[&str]| crate::gate::git(repo, args).ok();
    let tree = git(&["rev-parse", &format!("{commit}^{{tree}}")])?;
    let log = git(&["log", "-n", "500", "--format=%H %T", tip])?;
    log.lines().find_map(|l| {
        let (c, t) = l.split_once(' ')?;
        (t == tree).then(|| c.to_owned())
    })
}

fn checks(paths: &gp_evidence::Paths, games_dir: Option<&std::path::Path>) -> Value {
    use std::fs;
    use std::time::UNIX_EPOCH;
    let home = &paths.home;
    let now = gp_mcp::asks::now_ms();
    let mut lines: Vec<String> = Vec::new();
    let mut problems = 0u64;
    let ok = |l: &mut Vec<String>, s: String| l.push(format!("OK       {s}"));
    let bad = |l: &mut Vec<String>, n: &mut u64, s: String| {
        *n += 1;
        l.push(format!("PROBLEM  {s}"));
    };
    // The servers: each heartbeat's process alive, fresh, and started after
    // the published gp was written (else it runs an older build).
    // The published gp ($GP_HOME/bin/gp, what the gates publish).
    let published = fs::metadata(home.join("bin").join("gp"))
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as u64);
    let dir = gp_mcp::steps::loop_dir(home);
    let mut live = 0;
    for e in fs::read_dir(&dir).into_iter().flatten().flatten() {
        let n = e.file_name().to_string_lossy().into_owned();
        if !(n.starts_with("runner") && n.ends_with(".json")) {
            continue;
        }
        let Some(b) = fs::read(e.path())
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        else {
            continue;
        };
        let pid = b["pid"].as_u64().unwrap_or(0);
        let at_ms = b["at_ms"].as_u64().unwrap_or(0);
        // A pid given again to a newer process isn't the server that beat
        // (gp serve 52176's heartbeat, hours old, read as a stuck runner).
        if !gp_mcp::proc::alive_since(pid, at_ms) {
            continue;
        }
        live += 1;
        let age = now.saturating_sub(at_ms) / 1000;
        let started_ms = gp_mcp::proc::started_ms(pid).unwrap_or(0);
        let started = if started_ms > 0 {
            format!("{} s ago", now.saturating_sub(started_ms) / 1000)
        } else {
            "at a time ps doesn't say".to_owned()
        };
        // The server's own file (serve/reload.rs): a server that moved
        // onto a new gp keeps its pid and ps start time, so its start is
        // the file's; while it waits to move, the file says what for.
        let own: Value = fs::read(crate::serve::reload::file(home, pid as u32))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or(Value::Null);
        let started_ms = own["started_ms"].as_u64().unwrap_or(started_ms);
        if age > 30 {
            bad(
                &mut lines,
                &mut problems,
                format!("gp serve {pid}: its runner last looked {age} s ago (stuck?). Restart it."),
            );
        } else if let Some(says) = own["moving"]["says"].as_str() {
            ok(&mut lines, format!("gp serve {pid}: {says}"));
        } else if started_ms > 0 && published > started_ms + 1000 {
            bad(
                &mut lines,
                &mut problems,
                format!(
                    "gp serve {pid} runs an older gp than the one published (started {started}). Restart it on the published build."
                ),
            );
        } else {
            ok(
                &mut lines,
                format!(
                    "gp serve {pid}: answering, on the published build{}",
                    if b["agent"] == json!(true) {
                        ", worker command set"
                    } else {
                        ", no worker command"
                    }
                ),
            );
        }
        // Its start line said "the editor isn't built" (a server from
        // before this has no `editor` in its file at all).
        if own.get("editor").is_some_and(Value::is_null) {
            bad(
                &mut lines,
                &mut problems,
                format!(
                    "gp serve {pid} serves no editor: it found no editor/dist, in the published gp's page files or the checkout. Restart it once the published gp has its page's files."
                ),
            );
        }
    }
    // The page's files published beside the gp (crate::publish), which gp
    // serve serves the page from.
    if let Some(p) = published_pages_problem(&home.join("bin").join("gp")) {
        bad(&mut lines, &mut problems, p);
    }
    // The published gp against main's tip, in the checkout gp was built
    // from (the tool gate's tree, which shares the repo's branches).
    if let Some(repo) = crate::repo::built().filter(|r| r.join(".git").exists()) {
        match published_against(&home.join("bin").join("gp"), &repo, "main") {
            Some(Ok(l)) => ok(&mut lines, l),
            Some(Err(p)) => bad(&mut lines, &mut problems, p),
            None => {}
        }
    }
    if live == 0 {
        bad(&mut lines, &mut problems, "No gp serve runs for this home: start one (gp serve --port 7796 --games DIR --clip-agent claude).".into());
    }
    let s = gp_mcp::steps::settings(home);
    if s.runner.on {
        ok(
            &mut lines,
            format!(
                "Runner on, at most {} workers{}",
                s.runner.max_workers,
                if s.games.is_empty() {
                    String::new()
                } else {
                    format!(", games {}", s.games.join(", "))
                }
            ),
        );
    } else {
        bad(
            &mut lines,
            &mut problems,
            "The runner is off: gp loop set --runner on.".into(),
        );
    }
    if let Some(t) = hold_text(&s) {
        lines.push(format!("HELD     {t}"));
    }
    if s.runner.max_workers == 0 {
        bad(
            &mut lines,
            &mut problems,
            "The runner may start no worker (max 0): gp loop set --max-workers 1 or more.".into(),
        );
    }
    if let Some(h) = crate::wrong::halt(home) {
        bad(
            &mut lines,
            &mut problems,
            format!(
                "The loop is halted: {}",
                crate::wrong::halted_json(&h)["says"].as_str().unwrap_or("")
            ),
        );
    }
    // The yes cards: taken within a minute, and none failing.
    let mut waiting = 0;
    for a in gp_mcp::asks::all(home) {
        match yes_card_line(&a, now) {
            Some(Ok(l)) => ok(&mut lines, l),
            Some(Err(p)) => bad(&mut lines, &mut problems, p),
            None => {}
        }
        if let Some(p) = failed_card_problem(&a, now) {
            bad(&mut lines, &mut problems, p);
        }
        if gp_mcp::asks::is_open(&a, now) {
            waiting += 1;
        }
    }
    // Rows marked working with no live worker on them.
    let workers = gp_mcp::steps::workers(home);
    let held: std::collections::BTreeSet<String> = workers
        .iter()
        .filter(|w| gp_mcp::steps::holds(w))
        .flat_map(|w| w["step"]["touches"].as_array().cloned().unwrap_or_default())
        .filter_map(|t| t.as_str().map(str::to_owned))
        .collect();
    let games = crate::roms_cmd::games_dir(games_dir);
    let mut asking = Vec::new();
    for g in games
        .as_deref()
        .map(crate::next::games_in)
        .unwrap_or_default()
    {
        if !(s.games.is_empty() || s.games.contains(&g)) {
            continue;
        }
        let Ok(f) = crate::mechanic::session::queue_file(paths, &g) else {
            continue;
        };
        for r in crate::mechanic::session::read_queue(&f) {
            let row = r["mechanic"].as_str().unwrap_or("");
            // A held game's game row waits for the owner, not a worker.
            let waits = s.held(&g) && r["repo"] != json!("tools");
            if r["status"] == json!("agent working")
                && !waits
                && !crate::steps::taken_by_hand(&r)
                && !held.contains(&format!("row:{g}/{row}"))
            {
                bad(
                    &mut lines,
                    &mut problems,
                    format!(
                        "{g}/{row} is marked agent working, and no worker is on it: set it from its last worker's report."
                    ),
                );
            }
            if r["status"] == json!("asking you") {
                asking.push(format!("{g}/{row}"));
            }
        }
        // Every clip at the bars: a game is published only when they all
        // are, so a clip short of them is work the loop has, whatever its
        // row's status says.
        let short = crate::mechanic::scores::short_clips(home, &g, None);
        if short.is_empty() {
            ok(
                &mut lines,
                format!("Every scored clip of {g} meets the bars: it can be published"),
            );
        } else {
            bad(
                &mut lines,
                &mut problems,
                format!(
                    "{g} can't be published: clips of its corpus don't meet the bars on the gate's last scores, and a game is published only when every clip does. Set each of their rows that is done or parked to next and fix the model."
                ),
            );
            let rows: std::collections::BTreeSet<&str> =
                short.iter().map(|(_, r)| r.as_str()).collect();
            lines.push(format!(
                "           {} clip(s): {}",
                short.len(),
                short
                    .iter()
                    .map(|(id, _)| id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
            lines.push(format!(
                "           their rows: {}",
                rows.into_iter().collect::<Vec<_>>().join(", ")
            ));
        }
        // The games checkout clean: a local change blocks every landing.
        if let Some(d) = games.as_deref() {
            let out = std::process::Command::new("git")
                .args(["status", "--porcelain", "--", &g])
                .current_dir(d)
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
                .unwrap_or_default();
            let n = out.lines().count();
            if n > 0 {
                bad(
                    &mut lines,
                    &mut problems,
                    format!(
                        "The games checkout has {n} local changes under {g}/: a landing there fails until they are committed through a gate or set aside."
                    ),
                );
                // The files on a line of their own, so the problem's words
                // (and the trouble row they file into) stay the same.
                lines.push(format!("           {}", local_changes(&out)));
                lines.push(format!(
                    "           gp loop set-aside {g} puts them in a git stash of the checkout, on the person's yes"
                ));
            } else {
                ok(
                    &mut lines,
                    format!("The games checkout is clean under {g}/"),
                );
            }
        }
    }
    // Each game's port run, on the gp it started on (crate::port_run).
    for r in crate::port_run::all(home) {
        ok(&mut lines, crate::port_run::says(home, &r));
    }
    // The home's free disk: workers' builds fill it (loop-cleans-its-builds).
    match crate::builds::disk_line(home) {
        Some(Ok(l)) => ok(&mut lines, l),
        Some(Err(l)) => bad(&mut lines, &mut problems, l),
        None => {}
    }
    // The gates' locks (the model gate's and the tool gate's): who holds
    // each, and a holder that logs no step or names no pid
    // (gate-lock-orphan) is a problem.
    if let Some(g) = crate::gate::gate_home(None).ok().as_deref() {
        for lane in crate::gate_hold::Lane::ALL {
            match crate::gate_hold::health_line(g, lane, now) {
                Some(Ok(l)) => ok(&mut lines, l),
                Some(Err(l)) => bad(&mut lines, &mut problems, l),
                None => {}
            }
        }
    }
    // Each game's scoreboard, as gp serve last measured it.
    if let Some(d) = games.as_deref() {
        for g in crate::next::games_in(d) {
            if (s.games.is_empty() || s.games.contains(&g))
                && let Some(b) = crate::scoreboard::board(home, d, &g)
            {
                lines.extend(crate::scoreboard::text(&b));
            }
        }
    }
    let at_work = workers.iter().filter(|w| gp_mcp::steps::holds(w)).count();
    if at_work > 0 {
        ok(&mut lines, format!("{at_work} worker(s) at work"));
    } else if waiting > 0 || !asking.is_empty() {
        lines.push(format!("IDLE     No worker at work: the loop waits on the person ({waiting} card(s), rows asking: {})", if asking.is_empty() { "none".to_owned() } else { asking.join(", ") }));
    } else {
        lines.push("IDLE     No worker at work and nothing waits on the person: the quest log has no open item".into());
    }
    json!({"lines": lines, "problems": problems})
}

/// A yes card the runner hasn't started, for gp loop health: a problem
/// once a minute has gone by with nothing in hand; an OK line while the
/// runner holds it for the cards it stands on (`--after`, its
/// `after_held_ms` renewed each pass) or while a put-back yes waits its
/// minute. None for a card that isn't such a yes, or is younger.
fn yes_card_line(a: &Value, now: u64) -> Option<Result<String, String>> {
    if !gp_mcp::asks::is_yes(a, now) {
        return None;
    }
    let id = a["id"].as_str().unwrap_or("");
    let answered = a["answered_ms"].as_u64().unwrap_or(0);
    if now.saturating_sub(answered) <= 60_000 {
        return None;
    }
    if now.saturating_sub(gp_mcp::asks::yes_seen_ms(a)) > 60_000 {
        return Some(Err(format!(
            "Card {id}: the person said yes over a minute ago and no runner has taken it."
        )));
    }
    if gp_mcp::asks::kept_waits(a, now) {
        return Some(Ok(format!(
            "Card {id}: its yes runs again within a minute ({})",
            a["kept_why"].as_str().unwrap_or("it couldn't land")
        )));
    }
    Some(Ok(format!(
        "Card {id}: the runner holds its yes until {}",
        a["after_waits"]
            .as_str()
            .unwrap_or("the cards it stands on land")
            .trim_start_matches("it stands on ")
    )))
}

/// The local changes `git status --porcelain` printed, for the line under
/// the games checkout's problem: the files git changed, then the ones it
/// doesn't track, the first few of each by path, so whoever clears them
/// knows what they are without a shell in the checkout.
fn local_changes(porcelain: &str) -> String {
    const SHOWN: usize = 8;
    let (mut changed, mut untracked) = (Vec::new(), Vec::new());
    for l in porcelain.lines().filter(|l| l.len() > 3) {
        let (how, path) = l.split_at(3);
        if how.starts_with("??") {
            untracked.push(path.trim());
        } else {
            changed.push(path.trim());
        }
    }
    let part = |what: &str, files: &[&str]| {
        let more = files.len().saturating_sub(SHOWN);
        let mut s = format!(
            "{what} ({}): {}",
            files.len(),
            files[..files.len().min(SHOWN)].join(", ")
        );
        if more > 0 {
            s.push_str(&format!(" and {more} more"));
        }
        s
    };
    let mut parts = Vec::new();
    if !changed.is_empty() {
        parts.push(part("changed", &changed));
    }
    if !untracked.is_empty() {
        parts.push(part("not in git", &untracked));
    }
    parts.join("; ")
}

/// `git status --porcelain` of `game`/ in the games checkout, every file
/// git doesn't track named on its own.
fn game_status(games: &std::path::Path, game: &str) -> Result<String> {
    let o = std::process::Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=all", "--", game])
        .current_dir(games)
        .output()?;
    if !o.status.success() {
        anyhow::bail!(
            "git status in {} failed: {}",
            games.display(),
            String::from_utf8_lossy(&o.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&o.stdout).into_owned())
}

/// gp loop set-aside: the games checkout's local changes under `game`/
/// put in a git stash of the checkout named for the game and `day`, files
/// git doesn't track too, so nothing is lost and a landing there goes on;
/// with `dry_run` only listed. `replaces` names each file with `says`:
/// gp_run's card shows it as the plan, and the person's yes holds only
/// while the plan is the same (trouble-the-games-checkout-has-local-changes:
/// a refused corpus import stayed in the person's checkout for a day, since
/// no worker may run git stash and no gp command put it aside).
fn set_aside(games: &std::path::Path, game: &str, dry_run: bool, day: &str) -> Result<Value> {
    if game.is_empty()
        || game.starts_with('.')
        || game.contains(['/', '\\'])
        || !games.join(game).is_dir()
    {
        anyhow::bail!(
            "{game} is no game's folder in the games checkout {}",
            games.display()
        );
    }
    let out = game_status(games, game)?;
    let files: Vec<&str> = out
        .lines()
        .filter(|l| l.len() > 3)
        .map(|l| l[3..].trim())
        .collect();
    let replaces: Vec<Value> = out
        .lines()
        .filter(|l| l.len() > 3)
        .map(|l| {
            let path = l[3..].trim();
            let how = if l.starts_with("??") {
                "not in git"
            } else {
                "changed"
            };
            json!({"path": path, "says": format!("{path} ({how}): set aside in a git stash")})
        })
        .collect();
    let dir = games.display().to_string();
    let mut v = json!({
        "game": game, "games": dir, "dry_run": dry_run,
        "files": files, "replaces": replaces, "stash": null,
    });
    let n = files.len();
    if n == 0 {
        v["says"] = json!(format!(
            "Nothing to set aside: the games checkout is clean under {game}/."
        ));
        return Ok(v);
    }
    let named = local_changes(&out);
    if dry_run {
        v["says"] = json!(format!(
            "Would set aside {n} local changes under {game}/ in a git stash of the games checkout ({dir}): {named}"
        ));
        return Ok(v);
    }
    let message = format!("gp loop set-aside {game} {day}: {n} local changes");
    let o = std::process::Command::new("git")
        .args([
            "stash",
            "push",
            "--include-untracked",
            "-m",
            &message,
            "--",
            game,
        ])
        .current_dir(games)
        .output()?;
    if !o.status.success() {
        anyhow::bail!(
            "git stash in {dir} failed, and nothing was set aside: {}",
            String::from_utf8_lossy(&o.stderr).trim()
        );
    }
    let left = game_status(games, game)?.lines().count();
    v["stash"] = json!(message);
    v["left"] = json!(left);
    v["says"] = json!(format!(
        "Set aside {n} local changes under {game}/ in a git stash of the games checkout ({dir}), \"{message}\" in git stash list: {named}. `git stash apply` with that entry brings them back.{}",
        if left == 0 {
            format!(" The checkout is clean under {game}/.")
        } else {
            format!(" {left} are still there.")
        }
    ));
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The line under the games checkout's problem names the files, git's
    /// changed ones apart from those it doesn't track, a few of each.
    #[test]
    fn the_checkouts_local_changes_are_named() {
        let mut out = String::from(" M rooftop/model.toml\nA  rooftop/new.toml\n");
        for i in 0..10 {
            out.push_str(&format!("?? rooftop/inputs/c{i}.presses\n"));
        }
        let said = local_changes(&out);
        assert!(
            said.starts_with("changed (2): rooftop/model.toml, rooftop/new.toml; not in git (10): rooftop/inputs/c0.presses"),
            "{said}"
        );
        assert!(
            said.ends_with("rooftop/inputs/c7.presses and 2 more"),
            "{said}"
        );
        assert_eq!(local_changes("?? rooftop/a\n"), "not in git (1): rooftop/a");
    }

    /// gp loop set-aside puts a game's local changes, untracked files too,
    /// in a named git stash and leaves the checkout clean under the game;
    /// another game's changes stay, a dry run changes nothing, and
    /// `git stash apply` brings them back.
    #[test]
    fn set_aside_stashes_a_games_local_changes_and_nothing_else() {
        let t = tempfile::tempdir().unwrap();
        let g = t.path();
        let git = |args: &[&str]| {
            let o = std::process::Command::new("git")
                .args(["-c", "user.name=t", "-c", "user.email=t@t"])
                .args(args)
                .current_dir(g)
                .output()
                .unwrap();
            assert!(o.status.success(), "git {args:?}: {o:?}");
            String::from_utf8_lossy(&o.stdout).into_owned()
        };
        git(&["init", "-q", "-b", "main"]);
        for d in ["rooftop/inputs", "other"] {
            std::fs::create_dir_all(g.join(d)).unwrap();
        }
        std::fs::write(g.join("rooftop/corpus.toml"), "a\n").unwrap();
        std::fs::write(g.join("other/model.toml"), "a\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "first"]);

        let v = set_aside(g, "rooftop", false, "2026-11-25").unwrap();
        assert_eq!(v["files"], json!([]), "{v}");
        assert!(
            v["says"]
                .as_str()
                .unwrap()
                .starts_with("Nothing to set aside")
        );

        std::fs::write(g.join("rooftop/corpus.toml"), "b\n").unwrap();
        std::fs::write(g.join("rooftop/inputs/c1.presses"), "p\n").unwrap();
        std::fs::write(g.join("other/model.toml"), "b\n").unwrap();
        let dry = set_aside(g, "rooftop", true, "2026-11-25").unwrap();
        assert_eq!(
            dry["files"],
            json!(["rooftop/corpus.toml", "rooftop/inputs/c1.presses"])
        );
        assert_eq!(dry["replaces"].as_array().unwrap().len(), 2);
        assert_eq!(game_status(g, "rooftop").unwrap().lines().count(), 2);

        let done = set_aside(g, "rooftop", false, "2026-11-25").unwrap();
        assert_eq!(done["left"], json!(0), "{done}");
        assert_eq!(
            done["replaces"], dry["replaces"],
            "the card's plan is the run's"
        );
        assert_eq!(game_status(g, "rooftop").unwrap(), "");
        assert_eq!(game_status(g, "other").unwrap().lines().count(), 1);
        assert!(
            git(&["stash", "list"])
                .contains("gp loop set-aside rooftop 2026-11-25: 2 local changes")
        );
        git(&["stash", "apply", "-q"]);
        assert_eq!(
            std::fs::read_to_string(g.join("rooftop/corpus.toml")).unwrap(),
            "b\n"
        );
        assert!(g.join("rooftop/inputs/c1.presses").is_file());

        for bad in ["", "../x", ".git", "nope"] {
            assert!(set_aside(g, bad, true, "d").is_err(), "{bad}");
        }
    }

    /// A yes the runner holds for the cards it stands on, or a put-back
    /// yes waiting its minute, is an OK line; once the runner's last look
    /// at it is a minute old, or with nothing in hand, it is a problem.
    #[test]
    fn a_yes_held_for_the_cards_it_stands_on_is_not_a_problem() {
        let now = 10_000_000;
        let young = json!({"id": "y", "state": "yes", "answered_ms": now - 30_000});
        assert_eq!(yes_card_line(&young, now), None);
        let stuck = json!({"id": "s", "state": "yes", "answered_ms": now - 600_000});
        let said = yes_card_line(&stuck, now).unwrap().unwrap_err();
        assert!(said.contains("no runner has taken it"), "{said}");

        let held = json!({
            "id": "h", "state": "yes", "answered_ms": now - 600_000,
            "after_held_ms": now - 3_000,
            "after_waits": "it stands on cards not landed yet: 1-base (a fix)",
        });
        let said = yes_card_line(&held, now).unwrap().unwrap();
        assert!(
            said.contains("holds its yes until cards not landed yet: 1-base"),
            "{said}"
        );
        // The runner stopped looking: the hold is a minute stale.
        let stale = json!({
            "id": "h", "state": "yes", "answered_ms": now - 600_000,
            "after_held_ms": now - 90_000,
        });
        let said = yes_card_line(&stale, now).unwrap().unwrap_err();
        assert!(said.contains("no runner has taken it"), "{said}");

        let kept = json!({
            "id": "k", "state": "yes", "answered_ms": now - 600_000,
            "kept_until_ms": now + 20_000, "kept_why": "the tip kept moving",
        });
        let said = yes_card_line(&kept, now).unwrap().unwrap();
        assert!(
            said.contains("runs again within a minute (the tip kept moving)"),
            "{said}"
        );
        let running = json!({"id": "r", "state": "running", "answered_ms": now - 600_000});
        assert_eq!(yes_card_line(&running, now), None);
    }

    /// A published build whose .pages lacks the built editor is a problem
    /// naming it, and saying a words-only landing won't publish it; with
    /// mechanic/ and editor/dist/index.html beside it, none.
    #[cfg(unix)]
    #[test]
    fn a_published_gp_with_no_editor_dist_is_a_problem_until_its_pages_are_whole() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("bin");
        let pages = bin.join("gp-abc123def456.pages");
        std::fs::create_dir_all(pages.join("mechanic")).unwrap();
        std::fs::write(bin.join("gp-abc123def456"), "gp").unwrap();
        let link = bin.join("gp");
        std::os::unix::fs::symlink("gp-abc123def456", &link).unwrap();
        let said = published_pages_problem(&link).unwrap();
        assert!(said.contains("has no editor/dist"), "{said}");
        assert!(
            said.contains("words-only landing publishes nothing"),
            "{said}"
        );
        std::fs::create_dir_all(pages.join("editor/dist")).unwrap();
        std::fs::write(pages.join("editor/dist/index.html"), "editor").unwrap();
        assert_eq!(published_pages_problem(&link), None);
    }

    /// The published gp at main's tip is said so; behind it by words or by
    /// more, each said; on a commit main lost in a rebase, a problem.
    #[cfg(unix)]
    #[test]
    fn a_published_gp_main_no_longer_holds_is_a_problem() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        let run = |args: &[&str]| {
            let o = std::process::Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(args)
                .output()
                .unwrap();
            assert!(o.status.success(), "{o:?}");
            String::from_utf8_lossy(&o.stdout).trim().to_owned()
        };
        run(&["init", "-q", "-b", "main"]);
        run(&["config", "user.email", "test@example.invalid"]);
        run(&["config", "user.name", "health test"]);
        run(&["commit", "-q", "--allow-empty", "-m", "one"]);
        let one = run(&["rev-parse", "HEAD"]);
        let bin = tmp.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let link = bin.join("gp");
        let publish = |c: &str| {
            let name = format!("gp-{}", &c[..12]);
            std::fs::write(bin.join(&name), "gp").unwrap();
            let _ = std::fs::remove_file(&link);
            std::os::unix::fs::symlink(&name, &link).unwrap();
        };
        publish(&one);
        let said = published_against(&link, &repo, "main").unwrap().unwrap();
        assert!(said.contains("is main's tip"), "{said}");
        assert!(published_against(&link, &repo, "trunk").is_none());

        std::fs::write(repo.join("notes.md"), "words").unwrap();
        run(&["add", "notes.md"]);
        run(&["commit", "-q", "-m", "words"]);
        let said = published_against(&link, &repo, "main").unwrap().unwrap();
        assert!(
            said.contains("1 commit(s) behind") && said.contains("words alone"),
            "{said}"
        );
        std::fs::write(repo.join("main.rs"), "code").unwrap();
        run(&["add", "main.rs"]);
        run(&["commit", "-q", "-m", "code"]);
        let said = published_against(&link, &repo, "main").unwrap().unwrap();
        assert!(said.contains("more than words"), "{said}");

        // Rebased: the published commit is no longer on main.
        let lost = run(&["rev-parse", "HEAD"]);
        publish(&lost);
        run(&["reset", "-q", "--hard", &one]);
        run(&["commit", "-q", "--allow-empty", "-m", "version"]);
        let said = published_against(&link, &repo, "main")
            .unwrap()
            .unwrap_err();
        assert!(said.contains("main doesn't hold"), "{said}");

        // A wip commit published, then landed on main as another commit
        // with the same files: the published page is main's.
        run(&["checkout", "-q", "-b", "wip"]);
        std::fs::write(repo.join("main.rs"), "landed code").unwrap();
        run(&["add", "main.rs"]);
        run(&["commit", "-q", "-m", "wip"]);
        let wip = run(&["rev-parse", "HEAD"]);
        publish(&wip);
        run(&["checkout", "-q", "main"]);
        run(&["checkout", "-q", "wip", "--", "main.rs"]);
        run(&["commit", "-q", "-m", "the landing"]);
        let said = published_against(&link, &repo, "main").unwrap().unwrap();
        assert!(said.contains("files of main's tip"), "{said}");
        std::fs::write(repo.join("main.rs"), "comment fixed").unwrap();
        run(&["commit", "-q", "-am", "after"]);
        let said = published_against(&link, &repo, "main").unwrap().unwrap();
        assert!(
            said.contains("which landed them as a commit of its own, and is 1 commit(s) behind")
                && said.contains("more than words"),
            "{said}"
        );
    }

    /// --hold adds games (all holds every one), --go-on lifts them, and a
    /// game can't be lifted alone out of every game's hold.
    #[test]
    fn hold_adds_games_and_go_on_lifts_them() {
        let h = |was: &str, hold, go| {
            let was: Vec<String> = game_list(was);
            new_hold(&was, hold, go).map(|x| x.join(","))
        };
        let ok = |s: &str| Ok(s.to_owned());
        assert_eq!(h("", Some("hilltop, rooftop"), None), ok("hilltop,rooftop"));
        assert_eq!(h("hilltop", Some("hilltop"), None), ok("hilltop"));
        assert_eq!(h("hilltop", None, Some("hilltop")), ok(""));
        assert_eq!(h("hilltop", Some("all"), None), ok("all"));
        assert_eq!(h("all", Some("hilltop"), None), ok("all"));
        assert_eq!(h("all", None, Some("all")), ok(""));
        assert_eq!(h("a,b", None, Some("all")), ok(""));
        assert!(h("all", None, Some("hilltop")).is_err());
        // Lifting all and holding one at once leaves the one.
        assert_eq!(h("all", Some("rooftop"), Some("all")), ok("rooftop"));
        let s = gp_mcp::steps::Settings {
            hold: game_list("hilltop"),
            ..Default::default()
        };
        let t = settings_text(&s);
        assert!(t.contains("game work held for hilltop"), "{t}");
        assert!(t.contains("--go-on"), "{t}");
        assert!(!settings_text(&Default::default()).contains("held"));
    }

    #[test]
    fn a_failed_card_its_row_took_is_no_problem() {
        let now = 10_000_000;
        let lines = json!([
            "FAIL (patch): a three-way merge conflicts in x.rs",
            "(ran in /g)"
        ]);
        let failed =
            json!({"id": "c1", "ran": {"state": "failed", "done_ms": now - 1000, "lines": lines}});
        assert_eq!(
            failed_card_problem(&failed, now).as_deref(),
            Some(
                "Card c1 failed in the last hour: FAIL (patch): a three-way merge conflicts in x.rs"
            )
        );
        // Its row went back to next with why: the loop has it again.
        let mut took = failed.clone();
        took["row_took_it"] = json!(true);
        assert_eq!(failed_card_problem(&took, now), None);
        // Only settled (no row to take it) is still a problem.
        let mut settled = failed.clone();
        settled["row_settled"] = json!(true);
        assert!(failed_card_problem(&settled, now).is_some());
        // Over an hour ago, or landed, isn't.
        assert_eq!(failed_card_problem(&failed, now + 3_600_000), None);
        let landed = json!({"id": "c2", "ran": {"state": "landed", "done_ms": now}});
        assert_eq!(failed_card_problem(&landed, now), None);
    }
}
