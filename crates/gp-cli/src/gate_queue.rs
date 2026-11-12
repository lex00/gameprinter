//! The game gate's queue (docs/gate.md, "Batches"). Gates on one home run
//! one at a time, holding `gate.lock`. A gate that waits leaves a ticket in
//! the home's `queue/`: what it asks and a copy of its patch. The gate whose
//! turn comes takes the waiting tickets that agree with it on the repo, the
//! game, the branch and the run into its batch (a `.claim` with its pid),
//! gates them together, and writes each one's outcome beside its ticket
//! (`.result`) before it lets go of the lock. A waiting gate that gets the
//! lock reads its result and ends; one whose claimer died with no result
//! gates its patch itself. A ticket whose process is gone is cleared.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

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
    pid == std::process::id()
        || Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
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
            for ext in ["json", "patch", "claim", "result"] {
                let _ = fs::remove_file(side(queue, n, ext));
            }
        }
        keep
    });
    names
}

impl Ticket {
    /// Takes a ticket for `req` in `home`'s queue, with a copy of `patch`.
    pub(crate) fn enter(home: &Path, mut req: Request, patch: &Path) -> Result<Self> {
        let queue = home.join("queue");
        fs::create_dir_all(&queue)?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let name = format!("{now:024}-{}", std::process::id());
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
                    .and_then(|mut f| {
                        use std::io::Write as _;
                        write!(f, "{}", std::process::id())
                    })
                    .is_ok()
            {
                out.push((n, r));
            }
        }
        out
    }

    /// Writes `o` as the outcome of the claimed ticket `name`.
    pub(crate) fn answer(&self, name: &str, o: &Outcome) {
        let v = json!({"json": o.json, "text": o.text, "exit": o.exit});
        let _ = write_new(&side(&self.queue, name, "result"), v.to_string().as_bytes());
    }
}

impl Drop for Ticket {
    fn drop(&mut self) {
        for ext in ["json", "patch", "claim", "result"] {
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
        }
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
        let me = Ticket::enter(home, req(home, "dk", "main", "gate"), &patch).unwrap();
        let same = Ticket::enter(home, req(home, "dk", "main", "gate"), &patch).unwrap();
        let game = Ticket::enter(home, req(home, "other", "main", "gate"), &patch).unwrap();
        let branch = Ticket::enter(home, req(home, "dk", "dev", "gate"), &patch).unwrap();
        let run = Ticket::enter(home, req(home, "dk", "main", "canary"), &patch).unwrap();
        // A ticket left by a process that is gone is cleared, not taken.
        let dead = home.join("queue/000000000000000000000001-999999999.json");
        fs::write(
            &dead,
            serde_json::to_vec(&req(home, "dk", "main", "gate")).unwrap(),
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
    }
}
