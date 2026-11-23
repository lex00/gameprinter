//! The gates' queues (docs/gate.md, "Batches"): the game gate's, and the
//! tool gate's, whose tickets batch only with each other. Gates of one kind
//! on one home run one at a time, holding their kind's lock
//! (crate::gate_hold::Lane: `gate.lock`, `tool.lock`). A gate that waits
//! leaves a ticket in its kind's `queue/` (the home's for the model gate,
//! `tool/queue/` for the tool gate): what it asks and a copy of its patch.
//! The gate whose
//! turn comes takes the waiting tickets that agree with it on the repo, the
//! game, the branch and the run into its batch (a `.claim` with its pid),
//! gates them together, and writes each one's outcome beside its ticket
//! (`.result`) before it lets go of the lock. A waiting gate that gets the
//! lock reads its result and ends; one whose claimer died with no result
//! gates its patch itself. A ticket whose process is gone is cleared.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde_json::{Value, json};

use crate::Outcome;

/// The most patches one gate takes into its batch.
pub(crate) const BATCH_MAX: usize = 8;

/// What a waiting gate asks, written into its ticket.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Request {
    /// The patch's copy in the queue, so the asker's file can change.
    pub patch: PathBuf,
    /// The patch as the asker named it, for the batch's lines.
    pub shown: String,
    pub message: Option<String>,
    /// The repo's git folder (shared by its worktrees), the game's folder
    /// in it, the branch and the run: gates that agree on all four can
    /// share a batch.
    pub repo: PathBuf,
    pub game: String,
    pub onto: String,
    pub run: String,
    /// The clips the patch removes on purpose (`--removes`).
    #[serde(default)]
    pub removes: Vec<String>,
    /// The clips already in the corpus the patch changes on purpose
    /// (`--replaces`).
    #[serde(default)]
    pub replaces: Vec<String>,
}

impl Request {
    /// Whether a gate asked as `self` can take `other` into its batch.
    pub(crate) fn batches_with(&self, other: &Request) -> bool {
        self.repo == other.repo
            && self.game == other.game
            && self.onto == other.onto
            && self.run == other.run
    }
}

/// This gate's place in the queue: removed when dropped, with its files.
pub(crate) struct Ticket {
    queue: PathBuf,
    name: String,
    pub req: Request,
}

fn alive(pid: u32) -> bool {
    gp_mcp::proc::alive(u64::from(pid))
}

/// The pid at the end of a ticket's name, or in a claim's text.
fn pid_of(text: &str) -> Option<u32> {
    text.trim().rsplit('-').next()?.parse().ok()
}

fn side(queue: &Path, name: &str, ext: &str) -> PathBuf {
    queue.join(format!("{name}.{ext}"))
}

/// Writes `bytes` whole or not at all.
fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// The tickets in the queue, in the order they were taken, with those of
/// processes that are gone cleared.
fn tickets(queue: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(queue)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            e.file_name()
                .to_str()?
                .strip_suffix(".json")
                .map(str::to_owned)
        })
        .collect();
    names.sort();
    names.retain(|n| {
        let keep = pid_of(n).is_some_and(alive);
        if !keep {
            for ext in SIDES {
                let _ = fs::remove_file(side(queue, n, ext));
            }
        }
        keep
    });
    names
}

/// A ticket's files: what it asks, its patch, the claim of the batch that
/// took it, that batch's outcome, and its mark while its gate waits.
const SIDES: [&str; 5] = ["json", "patch", "claim", "result", "waits"];

impl Ticket {
    /// Takes a ticket for `req` in `home`'s queue, with a copy of `patch`.
    pub(crate) fn enter(home: &Path, mut req: Request, patch: &Path) -> Result<Self> {
        let queue = home.join("queue");
        fs::create_dir_all(&queue)?;
        let name = format!("{:024}-{}", now_ns(), std::process::id());
        let copy = side(&queue, &name, "patch");
        fs::copy(patch, &copy)?;
        req.patch = copy;
        write_new(&side(&queue, &name, "json"), &serde_json::to_vec(&req)?)?;
        Ok(Self { queue, name, req })
    }

    /// The outcome a batch reached for this ticket, once this gate holds
    /// the lock: `None` when no batch took it, or the one that did died
    /// before it said.
    pub(crate) fn result(&self) -> Option<Outcome> {
        let claim = side(&self.queue, &self.name, "claim");
        let got = fs::read(side(&self.queue, &self.name, "result"))
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok());
        let _ = fs::remove_file(&claim);
        let v = got?;
        Some(Outcome {
            json: v["json"].clone(),
            text: v["text"].as_str().unwrap_or("").to_owned(),
            exit: v["exit"].as_u64().map_or(1, |e| e as u8),
        })
    }

    /// Claims the waiting tickets that can share this gate's batch, in
    /// queue order, at most [`BATCH_MAX`] with this one: their names and
    /// requests.
    pub(crate) fn claim(&self) -> Vec<(String, Request)> {
        // The gate that holds the lock waits a few seconds before it takes
        // its batch: the runner starts every approved card's gate in one
        // pass, and the first to the lock used to start alone, with the
        // rest each scoring the corpus again behind it. GP_GATE_GATHER_MS
        // sets the wait; a test waits for nothing.
        let gather = std::env::var("GP_GATE_GATHER_MS")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .unwrap_or(if cfg!(test) { 0 } else { 8000 });
        if gather > 0 {
            std::thread::sleep(std::time::Duration::from_millis(gather));
        }
        let mut out = Vec::new();
        for n in tickets(&self.queue) {
            if out.len() + 1 >= BATCH_MAX {
                break;
            }
            if n == self.name || side(&self.queue, &n, "claim").exists() {
                continue;
            }
            let Some(r) = fs::read(side(&self.queue, &n, "json"))
                .ok()
                .and_then(|b| serde_json::from_slice::<Request>(&b).ok())
            else {
                continue;
            };
            if self.req.batches_with(&r)
                && fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(side(&self.queue, &n, "claim"))
                    .and_then(|mut f| write!(f, "{}", std::process::id()))
                    .is_ok()
            {
                out.push((n, r));
            }
        }
        out
    }

    /// Gives back the claimed ticket `name` with no outcome: its own gate
    /// runs its patch itself when its turn comes.
    pub(crate) fn release(&self, name: &str) {
        let _ = fs::remove_file(side(&self.queue, name, "claim"));
    }

    /// Writes `o` as the outcome of the claimed ticket `name`.
    pub(crate) fn answer(&self, name: &str, o: &Outcome) {
        let v = json!({"json": o.json, "text": o.text, "exit": o.exit});
        let _ = write_new(&side(&self.queue, name, "result"), v.to_string().as_bytes());
    }
}

/// How many of a gate's turns one run ahead of it may hold it back: a run
/// blocked on the lock takes it in the gate's first pause, so one still
/// there after this many is not waiting for the lock (a process that
/// reused its pid, a gate stopped by hand, a landing still checking its
/// patch before its wait) and the gate goes on.
const YIELDS_MAX: u32 = 30;

/// Who comes to a lock (docs/gate.md, "Who goes first").
pub(crate) enum Comer<'a> {
    /// A landing with its ticket: it goes after the older tickets.
    Landing(&'a Ticket),
    /// A dry run (no ticket): after every landing, and after the dry runs
    /// that came before it.
    DryRun,
}

/// How a gate came to hold its lock, for the lock line of its steps file.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Turn {
    /// Seconds from coming to the lock to holding it.
    pub waited_s: f64,
    /// How many runs were ahead of it when it came.
    pub ahead: usize,
    /// How many times it got the lock and gave it back to one ahead.
    pub yields: u32,
}

impl Turn {
    /// The fields the lock line takes.
    pub(crate) fn json(&self) -> Value {
        json!({
            "waited_s": (self.waited_s * 10.0).round() / 10.0,
            "ahead": self.ahead,
            "yields": self.yields,
        })
    }
}

/// The dry runs waiting for the lock, in the order they came: a file each
/// in `queue/dry/` named as tickets are (time, pid), with those of
/// processes that are gone cleared.
fn dry_waiting(queue: &Path) -> Vec<String> {
    let dir = queue.join("dry");
    let mut names: Vec<String> = fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().to_str().map(str::to_owned))
        .filter(|n| !n.ends_with(".tmp"))
        .collect();
    names.sort();
    names.retain(|n| {
        let keep = pid_of(n).is_some_and(alive);
        if !keep {
            let _ = fs::remove_file(dir.join(n));
        }
        keep
    });
    names
}

fn now_ns() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos())
}

/// The runs ahead of `me` (named `mine`, its ticket's or dry place's name)
/// in `queue`: every landing's ticket older than a landing's, and for a dry
/// run every ticket and every dry run that came before it. The flock itself
/// is first come first served by luck: on 2026-11-23 two landings waited
/// behind three fresh dry runs, and two dry runs of 18:20 and 18:27 still
/// waited at 19:14 behind one that came at 18:45.
/// A ticket counts once its gate waits for the lock (its `.waits`): one
/// still checking its patch, or whose gate never comes to the lock, holds
/// no one back.
fn ahead_of(queue: &Path, me: &Comer, mine: &str) -> Vec<String> {
    let older = |n: &String| n.as_str() < mine;
    let landings = tickets(queue)
        .into_iter()
        .filter(|n| side(queue, n, "waits").exists());
    match me {
        Comer::Landing(_) => landings.filter(older).collect(),
        Comer::DryRun => landings
            .chain(dry_waiting(queue).into_iter().filter(older))
            .collect(),
    }
}

/// A file that is there while its gate waits for the lock.
struct Waiting(PathBuf);

impl Waiting {
    fn enter(path: PathBuf) -> std::io::Result<Self> {
        fs::write(&path, b"")?;
        Ok(Self(path))
    }
}

impl Drop for Waiting {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Takes `lock` in `queue_home`'s order (docs/gate.md, "Who goes first"):
/// landings before dry runs, and among each the one that has waited
/// longest. A gate that gets the lock while one ahead of it still waits
/// gives it back and pauses (`GP_GATE_YIELD_MS`, 2 s), so the one blocked on
/// it goes first. A landing whose batch already gave it its outcome goes at
/// once: it only reads it. Returns how it came to hold the lock.
pub(crate) fn take_turn(lock: &fs::File, queue_home: &Path, me: Comer) -> std::io::Result<Turn> {
    let pause = std::env::var("GP_GATE_YIELD_MS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(if cfg!(test) { 20 } else { 2000 });
    let started = std::time::Instant::now();
    let queue = queue_home.join("queue");
    // Its place among those waiting, there until it holds the lock.
    let (_place, mine) = match &me {
        Comer::Landing(t) => (
            Waiting::enter(side(&t.queue, &t.name, "waits"))?,
            t.name.clone(),
        ),
        Comer::DryRun => {
            let dir = queue.join("dry");
            fs::create_dir_all(&dir)?;
            let name = format!("{:024}-{}", now_ns(), std::process::id());
            (Waiting::enter(dir.join(&name))?, name)
        }
    };
    let mut turn = Turn {
        ahead: ahead_of(&queue, &me, &mine).len(),
        ..Turn::default()
    };
    let mut seen: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    loop {
        lock.lock()?;
        let answered =
            matches!(&me, Comer::Landing(t) if side(&t.queue, &t.name, "result").exists());
        let waiting: Vec<String> = if answered {
            Vec::new()
        } else {
            ahead_of(&queue, &me, &mine)
                .into_iter()
                .filter(|n| {
                    let k = seen.entry(n.clone()).or_default();
                    *k += 1;
                    *k <= YIELDS_MAX
                })
                .collect()
        };
        if waiting.is_empty() {
            turn.waited_s = started.elapsed().as_secs_f64();
            return Ok(turn);
        }
        lock.unlock()?;
        if turn.yields == 0 {
            let _ = writeln!(
                std::io::stderr(),
                "gate: {} run(s) came to the lock before this one: it goes after them",
                waiting.len()
            );
        }
        turn.yields += 1;
        std::thread::sleep(std::time::Duration::from_millis(pause));
    }
}

impl Drop for Ticket {
    fn drop(&mut self) {
        for ext in SIDES {
            let _ = fs::remove_file(side(&self.queue, &self.name, ext));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(home: &Path, game: &str, onto: &str, run: &str) -> Request {
        Request {
            patch: PathBuf::new(),
            shown: format!("{game}.patch"),
            message: None,
            repo: home.join("repo/.git"),
            game: game.into(),
            onto: onto.into(),
            run: run.into(),
            removes: Vec::new(),
            replaces: Vec::new(),
        }
    }

    /// Who gets the lock when the gate holding it lets go: every landing
    /// waiting (a ticket: a patch's landing or a corpus import's) before
    /// any dry run, and among each kind the one that came first, whatever
    /// order the flock wakes them in. A ticket that never takes the lock
    /// holds no one back. Each gate
    /// says how many were ahead of it when it came.
    #[test]
    fn the_lock_goes_to_landings_then_dry_runs_each_oldest_first() {
        // Each row: the gates in the order they came to the held lock
        // ('d' a dry run, 'l' a landing, 'i' a corpus import of another
        // game, 's' a ticket whose gate never takes the lock), and the
        // order they ran in, each as its kind, the place it came in and
        // how many were ahead of it then.
        let table = [
            ("dl", "l1:0 d0:0"),
            ("ddl", "l2:0 d0:0 d1:1"),
            ("dddll", "l3:0 l4:1 d0:0 d1:1 d2:2"),
            ("ld", "l0:0 d1:1"),
            ("dd", "d0:0 d1:1"),
            ("dddd", "d0:0 d1:1 d2:2 d3:3"),
            ("sd", "d1:0"),
            ("dsl", "l2:0 d0:0"),
            // 2026-11-23: dry runs of 18:20 and 18:27 waited behind a
            // corpus import and a dry run of 18:45.
            ("ddid", "i2:0 d0:0 d1:1 d3:3"),
            ("lidli", "l0:0 i1:1 l3:2 i4:3 d2:2"),
        ];
        for (came, want) in table {
            let t = tempfile::tempdir().unwrap();
            let home = t.path().to_owned();
            let patch = home.join("p.patch");
            fs::write(&patch, "diff").unwrap();
            let held = crate::gate_hold::open_lock(&home, crate::gate_hold::Lane::Model).unwrap();
            held.lock().unwrap();
            let ran = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
            let mut kept = Vec::new();
            let mut gates = Vec::new();
            for (k, g) in came.chars().enumerate() {
                let (home, ran) = (home.clone(), ran.clone());
                let lock =
                    crate::gate_hold::open_lock(&home, crate::gate_hold::Lane::Model).unwrap();
                // A landing's ticket is in the queue before it waits.
                let game = if g == 'i' { "hilltop" } else { "mosshop" };
                let ticket = (g != 'd').then(|| {
                    Ticket::enter(&home, req(&home, game, "main", "gate"), &patch).unwrap()
                });
                if g == 's' {
                    kept.push(ticket);
                    std::thread::sleep(std::time::Duration::from_millis(30));
                    continue;
                }
                gates.push(std::thread::spawn(move || {
                    let me = match &ticket {
                        Some(t) => Comer::Landing(t),
                        None => Comer::DryRun,
                    };
                    let turn = take_turn(&lock, &home, me).unwrap();
                    ran.lock().unwrap().push(format!("{g}{k}:{}", turn.ahead));
                    std::thread::sleep(std::time::Duration::from_millis(20));
                    drop(ticket);
                    drop(lock);
                }));
                // Each gate is waiting on the lock before the next comes.
                std::thread::sleep(std::time::Duration::from_millis(30));
            }
            held.unlock().unwrap();
            for g in gates {
                g.join().unwrap();
            }
            assert_eq!(ran.lock().unwrap().join(" "), want, "came {came}");
            // A dry run's place goes with it.
            assert_eq!(dry_waiting(&home.join("queue")), Vec::<String>::new());
        }
    }

    /// The lock line says how a gate came to hold the lock.
    #[test]
    fn a_turn_says_how_long_it_waited_and_how_many_were_ahead() {
        let t = Turn {
            waited_s: 1514.04,
            ahead: 3,
            yields: 2,
        };
        assert_eq!(
            t.json(),
            json!({"waited_s": 1514.0, "ahead": 3, "yields": 2})
        );
    }

    /// A gate takes into its batch only the waiting gates on the same
    /// repo, game, branch and run; a claimed gate reads the batch's
    /// outcome, and its ticket and files go when it ends.
    #[test]
    fn a_waiting_gate_batches_only_with_the_same_game_branch_and_run() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path();
        let patch = home.join("p.patch");
        fs::write(&patch, "diff").unwrap();
        let me = Ticket::enter(home, req(home, "mosshop", "main", "gate"), &patch).unwrap();
        let same = Ticket::enter(home, req(home, "mosshop", "main", "gate"), &patch).unwrap();
        let game = Ticket::enter(home, req(home, "other", "main", "gate"), &patch).unwrap();
        let branch = Ticket::enter(home, req(home, "mosshop", "dev", "gate"), &patch).unwrap();
        let run = Ticket::enter(home, req(home, "mosshop", "main", "trial"), &patch).unwrap();
        // A ticket left by a process that is gone is cleared, not taken.
        let dead = home.join("queue/000000000000000000000001-999999999.json");
        fs::write(
            &dead,
            serde_json::to_vec(&req(home, "mosshop", "main", "gate")).unwrap(),
        )
        .unwrap();

        let claimed = me.claim();
        assert_eq!(
            claimed.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
            [same.name.as_str()]
        );
        assert!(!dead.exists());
        assert_eq!(fs::read(&claimed[0].1.patch).unwrap(), b"diff");
        // A second claim takes nothing: it is claimed already.
        assert!(me.claim().is_empty());

        me.answer(
            &same.name,
            &Outcome::ok(json!({"verdict": "PASS"}), "PASS".into()),
        );
        let o = same.result().unwrap();
        assert_eq!(
            (o.json["verdict"].as_str(), o.text.as_str(), o.exit),
            (Some("PASS"), "PASS", 0)
        );
        // With no result, a claim is no outcome: the gate runs its own.
        assert!(game.result().is_none());
        let files = |t: &Ticket| {
            fs::read_dir(&t.queue)
                .unwrap()
                .flatten()
                .filter(|e| e.file_name().to_string_lossy().starts_with(&t.name))
                .count()
        };
        assert_eq!(files(&same), 3);
        let q = same.queue.clone();
        let name = same.name.clone();
        drop(same);
        assert!(!side(&q, &name, "json").exists() && !side(&q, &name, "patch").exists());
        drop((game, branch, run));

        // A claim given back can be taken again.
        let again = Ticket::enter(home, req(home, "mosshop", "main", "gate"), &patch).unwrap();
        let taken = me.claim();
        assert_eq!(taken.len(), 1);
        me.release(&taken[0].0);
        assert_eq!(me.claim().len(), 1);
        drop(again);
    }
}
