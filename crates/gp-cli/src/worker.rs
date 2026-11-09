//! `gp worker env NAME`: one worker's setup on a shared machine
//! (docs/workers.md). A coordinator runs several workers (agents, each in
//! its own git worktree) at once; this gives each a private output folder,
//! a cargo target folder of its worktree's own, the fast build profile and
//! the gate's published gp, a nice level and a job limit from the
//! machine's load, and the rules every brief repeats. It works the same in
//! a worktree of `gameprinter` or of `games`.
//!
//! The target folder is the worktree's, never the branch's: cargo keys its
//! fingerprints by package, not by the folder a source came from, so
//! workers in several worktrees sharing one folder got a gp mixed from
//! their checkouts (2026-11-03). They share compiled dependencies through
//! the locked cargo's compile cache instead.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use serde_json::json;

use gp_evidence::Paths;

use crate::Outcome;

#[derive(Subcommand)]
pub enum WorkerCmd {
    /// Print a worker's setup. The setup is its private output folder
    /// ($GP_HOME/work/workers/NAME, made if missing), the cargo target folder
    /// of the worktree it runs in ($GP_HOME/work/target/BRANCH/HASH, HASH from
    /// the worktree's path; $GP_TARGET_DIR replaces $GP_HOME/work/target), the
    /// build profile (fastdev) and the gate's published gp ($GP_HOME/bin/gp), a
    /// nice level and a job limit from the machine's load and core count, and
    /// the rules every brief repeats. With `--shell`, the same as lines for
    /// `eval "$(gp worker env NAME --shell)"`: exports of GP_WORKER,
    /// GP_WORKER_OUT, CARGO_TARGET_DIR, GP_PROFILE, GP_BIN, GP (the gp the
    /// games repo's `just check` runs), CARGO_BUILD_JOBS and GP_JOBS,
    /// $GP_HOME/bin put first on PATH (so `gp` is the published build)
    /// unless `--rust`, a renice of the shell, and the rules as comments.
    Env {
        /// The worker's name: letters, digits, `-` and `_`.
        name: String,
        /// The branch the target folder is filed under (default: the
        /// branch checked out here, else `detached`).
        #[arg(long)]
        branch: Option<String>,
        /// The worker changes Rust and builds its own gp: PATH is left
        /// as it is, so `gp` isn't the published build.
        #[arg(long)]
        rust: bool,
        /// Print shell lines to eval instead of the setup in words.
        #[arg(long)]
        shell: bool,
    },
}

/// The rules every worker keeps (docs/workers.md, "Rules").
pub(crate) const RULES: &[&str] = &[
    "No rm in any form, not even on a temp file or /dev/null, and no moving files to the Trash. List what should be deleted in your report.",
    "Remove a worktree with `git worktree remove`, and empty a build folder with `cargo clean --target-dir DIR`.",
    "Never commit with --no-verify, and never push.",
    "Never open $GP_HOME/sealed or a sealed run's session; those are scored only by the coordinator.",
    "ROMs stay in $GP_ROM_DIR. Nothing made from a ROM (bytes, lifted files, a built game) goes into a commit or leaves this machine.",
    "A game change lands through `gp model gate PACKAGE --patch FILE -m MSG`; a change to the tool lands through `gp tool gate --patch FILE -m MSG` (docs/gate.md). Neither repo's main branch takes a commit any other way.",
    "Not changing Rust? Run the gate's published gp ($GP_HOME/bin/gp, `gp` on PATH after --shell) and build nothing. Changing Rust? Pass --rust and build with the nice level and job limit below, in your worktree's own target folder, in the fastdev profile.",
    "Build through the locked cargo ($GP_HOME/bin/locked/cargo, first on PATH after --shell): one build at a time on the machine, jobs from free memory. At most 2 workers change Rust at once.",
    "Write outputs in your own folder below, never in a shared one.",
    "Do one task, then stop with a report: what changed, the numbers checked, and the open items.",
];

/// The machine's load: the 1-minute load average and the cores.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Load {
    pub load: f64,
    pub cores: usize,
}

impl Load {
    pub fn now() -> Load {
        Load {
            load: load_average().unwrap_or(0.0),
            cores: std::thread::available_parallelism().map_or(1, |n| n.get()),
        }
    }

    /// Build jobs: the cores the load leaves free, at least 2 and at most
    /// half the machine, so no one worker takes it all.
    pub fn jobs(self) -> usize {
        let free = (self.cores as f64 - self.load).floor().max(0.0) as usize;
        free.clamp(2, (self.cores / 2).max(2))
    }

    /// The nice level: 10 on a machine under half load, 15 under full
    /// load, 19 past it.
    pub fn nice(self) -> u8 {
        let c = self.cores as f64;
        if self.load < c / 2.0 {
            10
        } else if self.load < c {
            15
        } else {
            19
        }
    }
}

/// The 1-minute load average, from /proc/loadavg on Linux or
/// `sysctl -n vm.loadavg` on macOS.
fn load_average() -> Option<f64> {
    if let Ok(text) = fs::read_to_string("/proc/loadavg") {
        return text.split_whitespace().next()?.parse().ok();
    }
    let out = std::process::Command::new("sysctl")
        .args(["-n", "vm.loadavg"])
        .output()
        .ok()?;
    parse_sysctl_loadavg(&String::from_utf8_lossy(&out.stdout))
}

/// `{ 40.34 31.99 29.37 }` -> 40.34.
fn parse_sysctl_loadavg(text: &str) -> Option<f64> {
    text.split_whitespace()
        .find(|w| *w != "{")
        .and_then(|w| w.parse().ok())
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// The cargo profile workers build in (Cargo.toml, docs/build-times.md).
pub(crate) const PROFILE: &str = "fastdev";

/// A branch name as a folder name: `/` and anything but letters, digits,
/// `-`, `_` and `.` become `-` (`m3/stage0-coverage` -> `m3-stage0-coverage`).
fn branch_folder(branch: &str) -> String {
    let f: String = branch
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "-_.".contains(c) {
                c
            } else {
                '-'
            }
        })
        .collect();
    if f.is_empty() || f.chars().all(|c| c == '.') {
        "detached".to_owned()
    } else {
        f
    }
}

/// A worktree's folder under its branch's: the first 16 hex digits of the
/// BLAKE3 of its path, so two worktrees never share a target folder.
fn worktree_folder(worktree: &Path) -> String {
    let h = blake3::hash(worktree.as_os_str().as_encoded_bytes());
    h.to_hex()[..16].to_owned()
}

/// The worktree the command runs in: git's top level, else the working
/// folder, with links resolved.
fn current_worktree() -> Result<PathBuf> {
    let top = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()))
        .filter(|p| !p.as_os_str().is_empty());
    let dir = match top {
        Some(d) => d,
        None => std::env::current_dir().context("reading the working folder")?,
    };
    Ok(fs::canonicalize(&dir).unwrap_or(dir))
}

/// The branch checked out in the working folder, if any.
pub(crate) fn current_branch() -> Option<String> {
    let o = std::process::Command::new("git")
        .args(["symbolic-ref", "--short", "-q", "HEAD"])
        .output()
        .ok()?;
    let b = String::from_utf8_lossy(&o.stdout).trim().to_owned();
    (o.status.success() && !b.is_empty()).then_some(b)
}

/// A worker's setup, with nothing read from the machine.
pub(crate) struct Env {
    pub name: String,
    pub out: PathBuf,
    pub target: PathBuf,
    /// The worktree whose sources build into `target`.
    pub worktree: PathBuf,
    /// The gate's newest published gp (docs/gate.md, "Published builds").
    pub bin: PathBuf,
    /// The worker builds its own gp; else `gp` on PATH is `bin`.
    pub rust: bool,
    pub load: Load,
}

impl Env {
    /// The target folder is `ROOT/BRANCH/HASH`: ROOT is `target`
    /// ($GP_TARGET_DIR) when set, else $GP_HOME/work/target; HASH is the
    /// worktree's ([`worktree_folder`]). A worker's build then reads only
    /// its own worktree's sources, and a worker returning to its worktree
    /// builds incrementally.
    pub fn new(
        name: &str,
        home: &Path,
        target: Option<PathBuf>,
        branch: Option<&str>,
        worktree: &Path,
        rust: bool,
        load: Load,
    ) -> Result<Env> {
        if !valid_name(name) {
            bail!("a worker's name is letters, digits, `-` and `_`; got {name:?}");
        }
        Ok(Env {
            name: name.to_owned(),
            out: home.join("work").join("workers").join(name),
            target: target
                .unwrap_or_else(|| home.join("work").join("target"))
                .join(branch_folder(branch.unwrap_or("detached")))
                .join(worktree_folder(worktree)),
            worktree: worktree.to_owned(),
            bin: home.join("bin").join("gp"),
            rust,
            load,
        })
    }

    pub fn shell(&self) -> String {
        let q = |p: &Path| shell_quote(&p.display().to_string());
        let mut lines = vec![
            format!("export GP_WORKER={}", shell_quote(&self.name)),
            format!("export GP_WORKER_OUT={}", q(&self.out)),
            format!("export CARGO_TARGET_DIR={}", q(&self.target)),
            format!("export GP_PROFILE={PROFILE}"),
            format!("export GP_BIN={}", q(&self.bin)),
            format!("export CARGO_BUILD_JOBS={}", self.load.jobs()),
            format!("export GP_JOBS={}", self.load.jobs()),
            format!("renice -n {} -p $$ >/dev/null 2>&1", self.load.nice()),
        ];
        let bin = self.bin.parent().unwrap_or(Path::new("."));
        if !self.rust {
            // The games repo's justfile runs $GP; a worktree there has no
            // gameprinter beside it to build one from.
            lines.push(format!("export GP={}", q(&self.bin)));
            lines.push(format!("export PATH={}:\"$PATH\"", q(bin)));
        }
        // Last, so it comes first: every cargo waits for the machine's
        // build lock (scripts/cargo/locked.py).
        lines.push(format!("export PATH={}:\"$PATH\"", q(&bin.join("locked"))));
        lines.extend(RULES.iter().map(|r| format!("# {r}")));
        lines.join("\n")
    }

    pub fn words(&self) -> String {
        let mut lines = vec![
            format!("worker {}", self.name),
            format!("  output folder   {}", self.out.display()),
            format!("  worktree        {}", self.worktree.display()),
            format!(
                "  cargo target    {} (this worktree's own)",
                self.target.display()
            ),
            if self.rust {
                format!(
                    "  gp              {}/{PROFILE}/gp, built from this worktree",
                    self.target.display()
                )
            } else {
                format!(
                    "  gp              {} (the gate's newest build; --shell puts it first on PATH; pass --rust to build your own)",
                    self.bin.display()
                )
            },
            format!(
                "  nice            {} (load {:.1} on {} cores)",
                self.load.nice(),
                self.load.load,
                self.load.cores
            ),
            format!(
                "  jobs            {} (cargo build -j {0}, GP_JOBS={0})",
                self.load.jobs()
            ),
            format!(
                "  build with      nice -n {} cargo build --profile {PROFILE} -j {} --target-dir {}",
                self.load.nice(),
                self.load.jobs(),
                self.target.display()
            ),
            format!(
                "  or run          eval \"$(gp worker env {}{} --shell)\" at the start of each shell",
                self.name,
                if self.rust { " --rust" } else { "" }
            ),
            "rules:".to_owned(),
        ];
        lines.extend(RULES.iter().map(|r| format!("  - {r}")));
        lines.join("\n")
    }
}

fn shell_quote(s: &str) -> String {
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || "/-_.:+@%".contains(c))
    {
        s.to_owned()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

pub(crate) fn run(cmd: &WorkerCmd, paths: &Paths) -> Result<Outcome> {
    match cmd {
        WorkerCmd::Env {
            name,
            branch,
            rust,
            shell,
        } => {
            let target = std::env::var_os("GP_TARGET_DIR").map(PathBuf::from);
            let branch = branch.clone().or_else(current_branch);
            let worktree = current_worktree()?;
            let env = Env::new(
                name,
                &paths.home,
                target,
                branch.as_deref(),
                &worktree,
                *rust,
                Load::now(),
            )?;
            fs::create_dir_all(&env.out)
                .with_context(|| format!("making {}", env.out.display()))?;
            Ok(Outcome {
                json: json!({
                    "worker": env.name,
                    "out": env.out,
                    "target": env.target,
                    "worktree": env.worktree,
                    "branch": branch,
                    "rust": env.rust,
                    "profile": PROFILE,
                    "bin": env.bin,
                    "nice": env.load.nice(),
                    "jobs": env.load.jobs(),
                    "load": env.load.load,
                    "cores": env.load.cores,
                    "rules": RULES,
                }),
                text: if *shell { env.shell() } else { env.words() },
                exit: 0,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jobs_and_nice_follow_the_load() {
        let l = |load, cores| Load { load, cores };
        assert_eq!(l(0.0, 18).jobs(), 9, "at most half the machine");
        assert_eq!(l(12.5, 18).jobs(), 5);
        assert_eq!(l(98.0, 18).jobs(), 2, "never fewer than 2");
        assert_eq!(l(0.0, 1).jobs(), 2);
        assert_eq!(l(4.0, 18).nice(), 10);
        assert_eq!(l(12.0, 18).nice(), 15);
        assert_eq!(l(40.0, 18).nice(), 19);
    }

    #[test]
    fn reads_the_macos_load_average() {
        assert_eq!(parse_sysctl_loadavg("{ 40.34 31.99 29.37 }\n"), Some(40.34));
        assert_eq!(parse_sysctl_loadavg(""), None);
    }

    #[test]
    fn the_shell_lines_export_the_setup_and_carry_the_rules() {
        let env = Env::new(
            "w10",
            Path::new("/h/gp home"),
            None,
            Some("m3/stage0-coverage"),
            Path::new("/w/one"),
            false,
            Load {
                load: 12.0,
                cores: 18,
            },
        )
        .unwrap();
        let sh = env.shell();
        let wt = worktree_folder(Path::new("/w/one"));
        assert!(sh.contains("export GP_WORKER=w10\n"), "{sh}");
        assert!(sh.contains("export GP_WORKER_OUT='/h/gp home/work/workers/w10'"));
        assert!(sh.contains(&format!(
            "export CARGO_TARGET_DIR='/h/gp home/work/target/m3-stage0-coverage/{wt}'"
        )));
        assert!(
            sh.contains("export PATH='/h/gp home/bin':\"$PATH\""),
            "{sh}"
        );
        assert!(sh.contains("export GP_PROFILE=fastdev\n"));
        assert!(sh.contains("export GP_BIN='/h/gp home/bin/gp'"));
        assert!(sh.contains("export CARGO_BUILD_JOBS=6\n"));
        assert!(sh.contains("renice -n 15 -p $$"));
        assert!(sh.contains("# No rm in any form"));
        assert!(sh.contains("gp model gate"));
        assert!(sh.contains("gp tool gate"));
        assert!(sh.contains("export GP='/h/gp home/bin/gp'"));
        let w = Path::new("/w/one");
        let root = Env::new(
            "a",
            Path::new("/h"),
            Some("/t".into()),
            None,
            w,
            false,
            env.load,
        )
        .unwrap();
        assert_eq!(root.target, Path::new("/t/detached").join(&wt));
        let detached = Env::new("a", Path::new("/h"), None, None, w, false, env.load).unwrap();
        assert_eq!(
            detached.target,
            Path::new("/h/work/target/detached").join(&wt)
        );
        let rust = Env::new("a", Path::new("/h"), None, None, w, true, env.load).unwrap();
        let path_dirs = |sh: &str| -> Vec<String> {
            sh.lines()
                .filter_map(|l| l.strip_prefix("export PATH="))
                .map(|l| {
                    l.trim_end_matches(":\"$PATH\"")
                        .trim_matches('\'')
                        .to_owned()
                })
                .collect()
        };
        assert_eq!(
            path_dirs(&rust.shell()),
            ["/h/bin/locked"],
            "a Rust worker runs its own build, and its cargo takes the build lock"
        );
        assert_eq!(
            path_dirs(&sh),
            ["/h/gp home/bin", "/h/gp home/bin/locked"],
            "the locked cargo comes first, then the published gp"
        );
        assert!(Env::new("../x", Path::new("/h"), None, None, w, false, env.load).is_err());
        assert_eq!(branch_folder("feature/a b"), "feature-a-b");
        assert_eq!(branch_folder(".."), "detached");
    }

    /// Two worktrees of one branch build in two folders, so neither's
    /// build reads the other's sources; one worktree always gets the same.
    #[test]
    fn two_worktrees_get_different_target_folders() {
        let load = Load {
            load: 0.0,
            cores: 8,
        };
        let env = |wt: &str| {
            Env::new(
                "w",
                Path::new("/h"),
                None,
                Some("m3/x"),
                Path::new(wt),
                true,
                load,
            )
            .unwrap()
            .target
        };
        let (a, b) = (env("/scratch/wt-a"), env("/scratch/wt-b"));
        assert_ne!(a, b);
        assert_eq!(a.parent(), b.parent(), "both under the branch's folder");
        assert_eq!(a.parent().unwrap(), Path::new("/h/work/target/m3-x"));
        assert_eq!(a, env("/scratch/wt-a"));
    }
}
