//! gp serve moves itself onto a newly published gp (quest log
//! tapper/serve-reloads-itself; docs/loop.md, "The runner"). Every few
//! seconds it looks at `$GP_HOME/bin/gp`; when the gate has published
//! another build since this one started, nothing new starts (the runner's
//! workers and its clip keeping wait; the person's yeses still run, since
//! the workers can take half an hour), and once the
//! workers have finished their step, the yeses written their result, the
//! background jobs and the preview's build ended and no request is being
//! answered, the process becomes the new build (exec) with the arguments
//! it was started with, on the port it listens on. The page's own polls
//! reconnect a moment later. `--rom` is left out, so the set the page's
//! picker chose last stays chosen.
//!
//! Each start writes `work/loop/serve/PID.json` (the process's start, its
//! build and, while it waits, what for), which `gp loop health` reads: the
//! process keeps its pid and start time across the exec, so `ps` alone
//! can't tell the build it runs. `GP_SERVE_NO_RELOAD=1` keeps a server on
//! the build it started on.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::Ctx;

/// How often the published gp is looked at.
const LOOK: Duration = Duration::from_secs(5);
/// How long requests being answered are waited for, once everything else
/// is done, before the exec cuts them.
const REQUESTS_FOR: Duration = Duration::from_secs(120);

/// A newer gp is published: nothing new starts.
static DRAINING: AtomicBool = AtomicBool::new(false);
/// Requests are refused from now on (the exec is next).
static CLOSING: AtomicBool = AtomicBool::new(false);
/// No yes starts from now on: set just before the last look at what is at
/// work, so a yes the runner took in between is seen (runner::run_yeses
/// counts a yes before it looks at this).
static SEALED: AtomicBool = AtomicBool::new(false);
/// Requests being answered now.
static ANSWERING: AtomicUsize = AtomicUsize::new(0);
/// What it waits for, in words, while it drains.
static WAITING: Mutex<Option<String>> = Mutex::new(None);

/// Whether the server is moving onto a newer gp, so nothing new starts.
pub(crate) fn draining() -> bool {
    DRAINING.load(Ordering::SeqCst)
}

/// Whether the move onto a newer gp is under way, so no yes starts.
pub(crate) fn sealed() -> bool {
    SEALED.load(Ordering::SeqCst)
}

/// The runner's words while it moves onto a newer gp.
pub(crate) fn waiting_words() -> Option<String> {
    WAITING.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// A request being answered: counted until dropped. None once the server
/// is closing for the exec; the request is then refused with a retry.
pub(crate) struct Answering(());

impl Answering {
    pub(crate) fn begin() -> Option<Self> {
        ANSWERING.fetch_add(1, Ordering::SeqCst);
        if CLOSING.load(Ordering::SeqCst) {
            ANSWERING.fetch_sub(1, Ordering::SeqCst);
            return None;
        }
        Some(Self(()))
    }
}

impl Drop for Answering {
    fn drop(&mut self) {
        ANSWERING.fetch_sub(1, Ordering::SeqCst);
    }
}

/// This server's own file, which `gp loop health` reads.
pub(crate) fn file(home: &Path, pid: u32) -> PathBuf {
    gp_mcp::steps::loop_dir(home)
        .join("serve")
        .join(format!("{pid}.json"))
}

fn write(home: &Path, v: &Value) {
    let f = file(home, std::process::id());
    if let Some(d) = f.parent() {
        let _ = fs::create_dir_all(d);
        // Files of servers no longer running.
        for e in fs::read_dir(d).into_iter().flatten().flatten() {
            let pid = e
                .file_name()
                .to_str()
                .and_then(|n| n.strip_suffix(".json"))
                .and_then(|n| n.parse::<u64>().ok());
            if pid.is_some_and(|p| !gp_mcp::proc::alive(p)) {
                let _ = fs::remove_file(e.path());
            }
        }
    }
    let tmp = f.with_extension("json.tmp");
    if fs::write(&tmp, serde_json::to_vec_pretty(v).unwrap_or_default()).is_ok() {
        let _ = fs::rename(&tmp, &f);
    }
}

/// The arguments the new build is started with: this one's, without
/// `--rom` (the set the page chose last is kept in the home) and with the
/// port it listens on (a `--port 0` picked one).
pub(crate) fn again_args(args: &[OsString], port: u16) -> Vec<OsString> {
    let mut out = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let s = a.to_string_lossy();
        if s == "--rom" || s == "--port" {
            it.next();
            continue;
        }
        if s.starts_with("--rom=") || s.starts_with("--port=") {
            continue;
        }
        out.push(a.clone());
    }
    out.push("--port".into());
    out.push(port.to_string().into());
    out
}

/// What is still at work, in words; None when nothing is.
fn still_at_work() -> Option<String> {
    let (workers, yeses, keeping) = super::runner::at_work();
    let jobs = super::background::jobs_in_flight();
    let building = crate::mechanic::live::building();
    let mut parts = Vec::new();
    let n = |k: usize, one: &str, many: &str| {
        if k == 1 {
            format!("1 {one}")
        } else {
            format!("{k} {many}")
        }
    };
    if workers > 0 {
        parts.push(format!(
            "{} {} {} step",
            n(workers, "worker", "workers"),
            if workers == 1 { "finishes" } else { "finish" },
            if workers == 1 { "its" } else { "their" }
        ));
    }
    if yeses > 0 {
        parts.push(format!(
            "{} {} running",
            n(yeses, "yes", "yeses"),
            if yeses == 1 { "is" } else { "are" }
        ));
    }
    if keeping {
        parts.push("the clip keeping's gate ends".into());
    }
    if jobs > 0 {
        parts.push(format!(
            "{} {}",
            n(jobs, "background job", "background jobs"),
            if jobs == 1 { "ends" } else { "end" }
        ));
    }
    if building {
        parts.push("the preview's build ends".into());
    }
    (!parts.is_empty()).then(|| parts.join(", "))
}

/// Starts the watch, once per process.
pub(crate) fn start(ctx: Arc<Ctx>) {
    let home = ctx.paths.home.clone();
    let exe = std::env::current_exe().ok();
    let started_ms = gp_mcp::asks::now_ms();
    write(
        &home,
        &json!({
            "pid": std::process::id(),
            "started_ms": started_ms,
            "build": exe,
            "moving": null,
        }),
    );
    if std::env::var_os("GP_SERVE_NO_RELOAD").is_some_and(|v| v != "0") || !cfg!(unix) {
        return;
    }
    let link = home.join("bin").join("gp");
    let mut seen = crate::mechanic::server::published_id(&link);
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(LOOK);
            let now = crate::mechanic::server::published_id(&link);
            let Some((bin, _)) = now.clone().filter(|_| now != seen) else {
                continue;
            };
            if !DRAINING.swap(true, Ordering::SeqCst) {
                eprintln!(
                    "gp serve: {} is published; moving onto it once nothing is at work",
                    bin.display()
                );
            }
            let words = match still_at_work() {
                Some(w) => format!(
                    "A new gp is published: gp serve moves onto it once {w}. Nothing new starts until then."
                ),
                None => "A new gp is published: gp serve is moving onto it now.".into(),
            };
            *WAITING.lock().unwrap_or_else(|e| e.into_inner()) = Some(words.clone());
            write(
                &home,
                &json!({
                    "pid": std::process::id(),
                    "started_ms": started_ms,
                    "build": exe,
                    "moving": {"to": bin, "says": words},
                }),
            );
            if still_at_work().is_some() {
                continue;
            }
            // The yeses still start while it drains; now none does, and a
            // yes taken since the look above is waited for.
            SEALED.store(true, Ordering::SeqCst);
            if still_at_work().is_some() {
                SEALED.store(false, Ordering::SeqCst);
                continue;
            }
            if !crate::mechanic::server::runs(&bin) {
                SEALED.store(false, Ordering::SeqCst);
                eprintln!(
                    "gp serve: {} doesn't run `--version`; staying on this build",
                    bin.display()
                );
                // Until another is published; gp loop health says it's old.
                seen = now;
                DRAINING.store(false, Ordering::SeqCst);
                *WAITING.lock().unwrap_or_else(|e| e.into_inner()) = None;
                write(
                    &home,
                    &json!({"pid": std::process::id(), "started_ms": started_ms, "build": exe, "moving": null}),
                );
                continue;
            }
            // Requests already being answered finish; new ones are told
            // to try again.
            CLOSING.store(true, Ordering::SeqCst);
            let t = Instant::now();
            while ANSWERING.load(Ordering::SeqCst) > 0 && t.elapsed() < REQUESTS_FOR {
                std::thread::sleep(Duration::from_millis(20));
            }
            let args: Vec<OsString> = std::env::args_os().skip(1).collect();
            let mut c = std::process::Command::new(&bin);
            c.args(again_args(&args, ctx.port))
                .env("GP_SERVE_RELOADED", "1");
            eprintln!("gp serve: moving onto {}", bin.display());
            let e = crate::mechanic::server::exec(c);
            eprintln!("gp serve: couldn't move onto {}: {e:#}", bin.display());
            seen = now;
            CLOSING.store(false, Ordering::SeqCst);
            SEALED.store(false, Ordering::SeqCst);
            DRAINING.store(false, Ordering::SeqCst);
            *WAITING.lock().unwrap_or_else(|e| e.into_inner()) = None;
            write(
                &home,
                &json!({"pid": std::process::id(), "started_ms": started_ms, "build": exe, "moving": null}),
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(v: &[&str]) -> Vec<OsString> {
        v.iter().map(OsString::from).collect()
    }

    #[test]
    fn the_new_build_gets_the_same_arguments_without_the_set_and_with_the_port() {
        let a = os(&[
            "serve",
            "--port",
            "0",
            "--games",
            "/g",
            "--rom",
            "tapper=tapperg",
            "--rom=rooftop=rooftop2",
            "--clip-agent",
            "claude --model sonnet",
        ]);
        assert_eq!(
            again_args(&a, 7796),
            os(&[
                "serve",
                "--games",
                "/g",
                "--clip-agent",
                "claude --model sonnet",
                "--port",
                "7796"
            ])
        );
        assert_eq!(
            again_args(&os(&["serve", "--port=7710"]), 7710),
            os(&["serve", "--port", "7710"])
        );
    }

    #[test]
    fn a_request_begun_while_closing_is_refused() {
        let before = ANSWERING.load(Ordering::SeqCst);
        let a = Answering::begin();
        assert!(a.is_some());
        CLOSING.store(true, Ordering::SeqCst);
        assert!(Answering::begin().is_none());
        CLOSING.store(false, Ordering::SeqCst);
        drop(a);
        assert_eq!(ANSWERING.load(Ordering::SeqCst), before);
    }
}
