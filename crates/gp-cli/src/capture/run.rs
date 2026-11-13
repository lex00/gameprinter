//! The run commands of game-printer's `gp capture` (capture.rs and
//! serve/background.rs), over gameprinter's runs. In game-printer a run was
//! a capture run (`$GP_HOME/capture/RUN`: a copy of a model, its stages'
//! outputs, an event log, a background queue). In gameprinter a run is a
//! game of the games checkout, named by its folder, and `gp serve` keeps
//! what it does for it in `$GP_HOME/work/serve/GAME/` (serve/game.rs): the
//! background work's state and rounds, the agents' record and the working
//! copy of the model. These commands read and change that.
//!
//! Where an existing command already does the job, the capture command is
//! that command under its old name, with the same output:
//!
//! | Old command | Folded into |
//! |---|---|
//! | `capture fidelity RUN` | `gp corpus score games/RUN --run RUN` (its fidelity line and record) |
//! | `capture regenerate --template DIR --run RUN` | `gp corpus score DIR --run RUN` (every clip made again from the ROM and checked against its curve) |
//! | `capture impact RUN --reuse FILE` | `gp corpus score games/RUN --run RUN --reuse FILE --impact` (what changed and the clips it would score again, scoring none) |
//! | `capture verify RUN` | `gp corpus verify games/RUN --run RUN` |
//!
//! The queue's on and off live in the running `gp serve` (it starts every
//! queue paused), so `background --start` and `--stop`, and `events`, ask
//! the server at `--server` (default `$GP_SERVE_URL`, else
//! http://127.0.0.1:7710).

use std::io::{Read as _, Write as _};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use clap::Args;
use gp_evidence::Paths;
use serde_json::{Value, json};

use crate::Outcome;
use crate::serve::Ctx;
use crate::serve::background;
use crate::serve::game::{Run, run_named};

/// Where `gp serve` is, for what only the running server holds.
#[derive(Args, Clone, Default)]
pub struct Server {
    /// The running `gp serve` (default: $GP_SERVE_URL, else
    /// http://127.0.0.1:7710).
    #[arg(long)]
    pub server: Option<String>,
}

/// The games checkout whose games are the runs.
#[derive(Args, Clone, Default)]
pub struct Games {
    /// The games checkout (default: $GP_GAMES_DIR, else a folder of games at
    /// or above the working directory).
    #[arg(long)]
    pub games: Option<PathBuf>,
}

#[derive(Args)]
pub struct BackgroundArgs {
    pub run: String,
    #[command(flatten)]
    pub games: Games,
    #[command(flatten)]
    pub server: Server,
    /// Score up to this many active clips here in the foreground, those
    /// not scored on the model as it stands, stopping early when there's
    /// nothing to do. Refine rounds run in `gp serve`.
    #[arg(long, default_value_t = 0)]
    pub steps: usize,
    /// Turn the queue on in the running `gp serve`, which starts with every
    /// queue paused.
    #[arg(long, alias = "on")]
    pub start: bool,
    /// Pause it.
    #[arg(long)]
    pub stop: bool,
    /// Score every clip again on the next turn, carrying none over.
    #[arg(long)]
    pub rescore_all: bool,
    /// Print every round instead of the state.
    #[arg(long)]
    pub history: bool,
    /// How many clips it works on at once, 1 to the machine's cores less
    /// one: kept with the run.
    #[arg(long)]
    pub workers: Option<u64>,
    /// How many clip agents `gp serve` hands the run's stuck clips to at
    /// once, 0 for none: kept with the run.
    #[arg(long)]
    pub agents: Option<u64>,
}

/// What every alias takes: the run, the module and the ROM.
#[derive(Args)]
pub struct ScoreArgs {
    pub run: String,
    #[command(flatten)]
    pub games: Games,
    /// The game's built module (default: built from its hook code).
    #[arg(long)]
    pub module: Option<PathBuf>,
    /// The ROM; by default the one in $GP_ROM_DIR the package names.
    #[arg(long)]
    pub rom: Option<PathBuf>,
}

fn ctx_of(paths: &Paths, g: &Games) -> Result<Ctx> {
    Ctx::for_cli(paths, g.games.as_deref())
}

fn run_of(ctx: &Ctx, name: &str) -> Result<Run> {
    run_named(ctx, name)
        .map_err(|f| anyhow::anyhow!("{}; `gp capture list` names the runs", f.message))
}

/// The game folder `RUN` names.
pub(crate) fn game_of(paths: &Paths, g: &Games, run: &str) -> Result<PathBuf> {
    let ctx = ctx_of(paths, g)?;
    Ok(run_of(&ctx, run)?.game)
}

/// `gp capture list`: every run with the background work's state.
pub(crate) fn list(paths: &Paths, g: &Games) -> Result<Outcome> {
    let ctx = ctx_of(paths, g)?;
    let mut names: Vec<String> = std::fs::read_dir(&ctx.games)
        .with_context(|| format!("reading {}", ctx.games.display()))?
        .flatten()
        .filter(|e| e.path().join("package.toml").is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    let mut runs = Vec::new();
    for n in names {
        let Ok(r) = run_of(&ctx, &n) else { continue };
        let v = background::saved_state(&r);
        let model = background::model_key(&ctx, &r);
        let scores = v["scores"].as_object();
        let on_model = scores.map_or(0, |m| {
            m.values()
                .filter(|s| s["model"].as_str() == Some(model.as_str()))
                .count()
        });
        let exact = scores.map_or(0, |m| m.values().filter(|s| s["exact"] == true).count());
        let rounds = background::rounds(&r).len();
        let state = if r.dir.is_dir() { "idle" } else { "new" };
        runs.push(
            json!({"run": n, "game": r.game, "dir": r.dir, "round": rounds,
                         "state": state, "platform": format!("{:?}", r.platform()).to_lowercase(),
                         "scored": on_model, "exact": exact,
                         "working_copy": r.copy_dir().join(gp_model::MODEL_FILE).is_file()}),
        );
    }
    let text = runs
        .iter()
        .map(|r| {
            format!(
                "{}  round {}  {}",
                r["run"].as_str().unwrap_or(""),
                r["round"],
                r["state"].as_str().unwrap_or("")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    Ok(Outcome::ok(json!({"runs": runs}), text))
}

/// `gp capture schema`.
pub(crate) fn schema() -> Outcome {
    Outcome::ok(
        gp_capture::schema(),
        gp_capture::schema_text().trim_end().to_owned(),
    )
}

/// The server's address, as `host:port`.
fn server_addr(s: &Server) -> String {
    let url = s
        .server
        .clone()
        .or_else(|| std::env::var("GP_SERVE_URL").ok())
        .unwrap_or_else(|| "http://127.0.0.1:7710".into());
    url.trim_start_matches("http://")
        .trim_end_matches('/')
        .to_owned()
}

/// One request to `gp serve`, its JSON reply.
fn ask(s: &Server, method: &str, path: &str, body: &Value) -> Result<Value> {
    let addr = server_addr(s);
    let sock = addr
        .parse::<std::net::SocketAddr>()
        .or_else(|_| {
            std::net::ToSocketAddrs::to_socket_addrs(&addr)?
                .next()
                .context("no address")
        })
        .with_context(|| format!("no server address {addr}"))?;
    let mut c =
        TcpStream::connect_timeout(&sock, Duration::from_millis(800)).with_context(|| {
            format!("no gp serve at {addr}: start it (gp serve), or name it with --server")
        })?;
    c.set_read_timeout(Some(Duration::from_secs(60)))?;
    let body = if method == "POST" {
        body.to_string()
    } else {
        String::new()
    };
    write!(
        c,
        "{method} {path} HTTP/1.0\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )?;
    let mut out = Vec::new();
    c.read_to_end(&mut out)?;
    let text = String::from_utf8_lossy(&out);
    let (head, rest) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let status = head
        .split_whitespace()
        .nth(1)
        .and_then(|x| x.parse::<u16>().ok())
        .unwrap_or(0);
    let v: Value = serde_json::from_str(rest).unwrap_or(Value::String(rest.to_owned()));
    if !(200..300).contains(&status) {
        bail!(
            "gp serve answered {status}: {}",
            v["error"].as_str().or(v.as_str()).unwrap_or("")
        );
    }
    Ok(v)
}

/// `gp capture events RUN --after N`: the run's events from the running
/// `gp serve`, one JSON object a line.
pub(crate) fn events(run: &str, after: u64, s: &Server) -> Result<Outcome> {
    let v = ask(
        s,
        "GET",
        &format!("/api/capture/{run}/events?after={after}"),
        &Value::Null,
    )?;
    let list = v["events"]
        .as_array()
        .cloned()
        .unwrap_or_else(|| v.as_array().cloned().unwrap_or_default());
    let text = list
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    Ok(Outcome::ok(json!({"run": run, "events": list}), text))
}

/// `gp capture background RUN`.
pub(crate) fn background(paths: &Paths, a: &BackgroundArgs) -> Result<Outcome> {
    let ctx = ctx_of(paths, &a.games)?;
    let r = run_of(&ctx, &a.run)?;
    let mut lines: Vec<String> = Vec::new();
    // The server's own view when it was asked: on and off live there.
    let mut served: Option<Value> = None;
    if let Some(w) = a.workers {
        let max = background::max_workers() as u64;
        anyhow::ensure!((1..=max).contains(&w), "--workers is 1 to {max}");
        background::update_state(&r, |v| v["workers"] = json!(w))
            .map_err(|f| anyhow::anyhow!(f.message))?;
        lines.push(format!("Workers: {w}."));
    }
    if let Some(n) = a.agents {
        background::update_state(&r, |v| v["agents"] = json!(n))
            .map_err(|f| anyhow::anyhow!(f.message))?;
        lines.push(format!("Clip agents at once: {n}."));
    }
    if a.rescore_all {
        background::update_state(&r, |v| {
            if let Some(m) = v["scores"].as_object_mut() {
                for s in m.values_mut() {
                    s["model"] = json!("");
                }
            }
        })
        .map_err(|f| anyhow::anyhow!(f.message))?;
        lines.push("Every clip is scored again on the next turn.".into());
    }
    if a.start || a.stop {
        let what = if a.start { "start" } else { "stop" };
        served = Some(ask(
            &a.server,
            "POST",
            &format!("/api/capture/{}/background/{what}", a.run),
            &json!({}),
        )?);
        lines.push(if a.start {
            "The queue is on in gp serve.".into()
        } else {
            "The queue is paused in gp serve.".into()
        });
    }
    if a.steps > 0 {
        let done = steps(&ctx, &r, a.steps)?;
        for d in &done {
            lines.push(format!(
                "Scored {}: {}",
                d["clip"].as_str().unwrap_or(""),
                d["says"].as_str().unwrap_or("")
            ));
        }
        if done.is_empty() {
            lines.push(
                "Nothing to do: every active clip is scored on the model as it stands.".into(),
            );
        }
    }
    if a.history {
        let rounds = background::rounds(&r);
        let text = if rounds.is_empty() {
            "No rounds yet.".to_owned()
        } else {
            rounds
                .iter()
                .map(|h| {
                    format!(
                        "round {}  {}  {}",
                        h["round"],
                        h["clip"].as_str().unwrap_or(""),
                        h["says"].as_str().unwrap_or("")
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        lines.push(text);
        return Ok(Outcome::ok(
            json!({"run": a.run, "rounds": rounds}),
            lines.join("\n"),
        ));
    }
    let v = served.unwrap_or_else(|| background::state_view(&ctx, &r));
    lines.push(v["says"].as_str().unwrap_or("").to_owned());
    let t = &v["totals"];
    lines.push(format!(
        "{} active clips, {} scored, {} exact; {} workers, {} clip agents.",
        t["active"], t["scored"], t["exact"], v["workers"], v["agents"]
    ));
    Ok(Outcome::ok(v, lines.join("\n")))
}

/// Scores up to `n` active clips not scored on the model as it stands, as
/// a worker of the queue does, and keeps each score in the run's state.
fn steps(ctx: &Ctx, r: &Run, n: usize) -> Result<Vec<Value>> {
    let model = background::model_key(ctx, r);
    let v = background::saved_state(r);
    let todo: Vec<background::Active> = background::active_now(ctx, r)
        .into_iter()
        .filter(|a| v["scores"][&a.id]["model"].as_str() != Some(model.as_str()))
        .take(n)
        .collect();
    if todo.is_empty() {
        return Ok(Vec::new());
    }
    let rb = r.rebuild(ctx);
    let mdir = r.model_dir();
    let sc = crate::clip_score::Ctx::open(
        &ctx.paths,
        &r.game,
        &crate::clip_score::Open {
            module: rb.module.as_deref(),
            model: Some(&mdir),
            assets: None,
            rom: None,
        },
    )?;
    let root = r.dir.join("background").join("scores");
    let mut out = Vec::new();
    for a in todo {
        let score = background::score_clip_in(ctx, r, &sc, &a.id, &model, &root);
        background::update_state(r, |v| {
            let before = v["scores"][&a.id]["score"].as_u64();
            if v["first"].get(&a.id).is_none() && score["error"].is_null() {
                v["first"][&a.id] = score["exact"].clone();
            }
            let mut s = score.clone();
            s["fixed"] = json!(v["first"][&a.id] == json!(false) && s["exact"] == json!(true));
            v["scores"][&a.id] = s;
            background::push_done(
                v,
                json!({"kind": "score", "clip": a.id, "name": a.name, "before": before,
                       "after": score["score"], "exact": score["exact"],
                       "error": score["error"], "says": score["says"]}),
            );
        })
        .map_err(|f| anyhow::anyhow!(f.message))?;
        out.push(json!({"clip": a.id, "score": score["score"], "exact": score["exact"], "says": score["says"]}));
    }
    Ok(out)
}

/// The `gp corpus` command a folded capture command is.
pub(crate) fn corpus_score(
    paths: &Paths,
    game: &Path,
    run: &str,
    a: &ScoreArgs,
    reuse: Option<PathBuf>,
    impact: bool,
) -> Result<Outcome> {
    crate::corpus_score::run(
        &crate::corpus_score::CorpusCmd::Score {
            game: game.to_owned(),
            run: run.to_owned(),
            module: a.module.clone(),
            model: None,
            assets: None,
            rom: a.rom.clone(),
            only: Vec::new(),
            reuse,
            impact,
            write: None,
            remeasure: None,
        },
        paths,
    )
}

pub(crate) fn corpus_verify(paths: &Paths, game: &Path, a: &ScoreArgs) -> Result<Outcome> {
    crate::corpus_score::run(
        &crate::corpus_score::CorpusCmd::Verify {
            dir: game.to_owned(),
            run: a.run.clone(),
            accept: false,
            module: a.module.clone(),
            assets: None,
            rom: a.rom.clone(),
        },
        paths,
    )
}
