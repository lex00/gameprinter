//! Laya, the decision server, started by `gp serve` when a tool needs it.
//!
//! `gp mechanic demo`, `gp mechanic say --judge` and the triage's demo step
//! ask a Jev-compatible decision server at 127.0.0.1:7720 which buttons to
//! hold. Before, the person started it by hand with `just laya`. Now:
//!
//! - A server already answering at the address is used as it is, whoever
//!   started it, and nobody here stops it.
//! - Else, at the default address, the tool asks `gp serve` (its
//!   `POST /api/laya/start`, at $GP_SERVE_URL or http://127.0.0.1:7710) to
//!   start it. gp serve runs scripts/laya/serve.mjs on an install already
//!   there ($LAYA_DIR, $GP_HOME/work/laya, else ~/.game-printer/work/laya,
//!   whose weights are cached in ~/.cache/receptron-laya), waits until it
//!   answers, and keeps it while it runs. The child holds a pipe from gp
//!   serve as its standard input and exits when that closes, so it stops
//!   with gp serve however gp serve ends.
//! - With no gp serve to ask, the tool starts one for itself the same way,
//!   and it stops when the tool ends.
//! - Nothing is installed here. With no install, the error says how
//!   (`just laya` installs @receptron/laya and fetches the weights, about
//!   1.7 GB, on first use).

use std::io::{Read as _, Write as _};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use serde_json::{Value, json};

/// Where the tools ask for decisions unless told otherwise.
pub(crate) const DEFAULT_URL: &str = "http://127.0.0.1:7720";

/// The address gp serve keeps Laya at: $GP_LAYA_URL (another port on this
/// machine, for a test beside a Laya already running), else [`DEFAULT_URL`].
pub(crate) fn managed_url() -> String {
    std::env::var("GP_LAYA_URL")
        .ok()
        .filter(|u| u.starts_with("http://127.0.0.1:"))
        .map(|u| u.trim_end_matches('/').to_owned())
        .unwrap_or_else(|| DEFAULT_URL.to_owned())
}

/// The port of [`managed_url`].
fn port() -> u16 {
    managed_url()
        .rsplit(':')
        .next()
        .and_then(|p| p.parse().ok())
        .unwrap_or(7720)
}

/// How long a started Laya may take to load its model and answer.
const LOAD_DEADLINE: Duration = Duration::from_secs(180);

/// gp serve's address when $GP_SERVE_URL doesn't say.
const SERVE_URL: &str = "http://127.0.0.1:7710";

/// What answers at an address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Answer {
    /// A decision server: it knows /v1/systemone.
    Decides,
    /// Something else holds the port; its first line.
    Other(String),
    /// Nothing listens.
    Nothing,
}

/// One plain HTTP exchange with a server on this machine: the status line
/// and the body. `host` is HOST:PORT.
fn exchange(
    host: &str,
    method: &str,
    path: &str,
    body: &str,
    wait: Duration,
) -> Result<(String, String)> {
    let addr: SocketAddr = host
        .parse()
        .with_context(|| format!("{host} isn't HOST:PORT"))?;
    let mut conn = TcpStream::connect_timeout(&addr, Duration::from_millis(500))?;
    conn.set_read_timeout(Some(wait))?;
    conn.set_write_timeout(Some(Duration::from_secs(5)))?;
    write!(
        conn,
        "{method} {path} HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    let mut out = String::new();
    conn.read_to_string(&mut out)?;
    let (head, rest) = out.split_once("\r\n\r\n").context("no HTTP answer")?;
    Ok((
        head.lines().next().unwrap_or("").to_owned(),
        rest.to_owned(),
    ))
}

/// HOST:PORT of an `http://HOST:PORT[/...]` URL.
fn host_of(url: &str) -> Result<&str> {
    let rest = url
        .strip_prefix("http://")
        .context("the decide URL is http://HOST:PORT")?;
    Ok(rest.split('/').next().unwrap_or(rest))
}

/// What answers at `url`'s host. A decision server answers anything but a
/// POST to /v1/systemone with an error that names that path (serve.mjs's
/// "POST /v1/systemone only"), so a GET tells it from another server.
pub(crate) fn probe(url: &str) -> Answer {
    let Ok(host) = host_of(url) else {
        return Answer::Nothing;
    };
    match exchange(host, "GET", "/v1/systemone", "", Duration::from_secs(2)) {
        Ok((_, body)) if body.contains("/v1/systemone") => Answer::Decides,
        Ok((line, _)) => Answer::Other(line),
        Err(e) => match e.downcast_ref::<std::io::Error>().map(std::io::Error::kind) {
            Some(std::io::ErrorKind::ConnectionRefused) | None => Answer::Nothing,
            // A port that takes the connection and says nothing in time is
            // held by something.
            Some(_) => Answer::Other(format!("no answer in time ({e})")),
        },
    }
}

/// The installs looked for, in order: $LAYA_DIR, $GP_HOME/work/laya, and
/// game-printer's ~/.game-printer/work/laya.
fn candidates(home: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(d) = std::env::var_os("LAYA_DIR").filter(|d| !d.is_empty()) {
        out.push(PathBuf::from(d));
    }
    out.push(home.join("work").join("laya"));
    if let Some(h) = std::env::var_os("HOME") {
        out.push(Path::new(&h).join(".game-printer/work/laya"));
    }
    out
}

/// The first install with @receptron/laya in it.
pub(crate) fn install_dir(home: &Path) -> Option<PathBuf> {
    candidates(home).into_iter().find(|d| {
        d.join("node_modules/@receptron/laya/package.json")
            .is_file()
    })
}

/// Why there's no Laya to start, and how to get one.
fn not_installed(home: &Path) -> String {
    let looked: Vec<String> = candidates(home)
        .iter()
        .map(|d| d.display().to_string())
        .collect();
    format!(
        "Laya isn't installed here (looked in {}). To install it: in the gameprinter checkout, run `just laya`; it installs @receptron/laya with npm under $GP_HOME/work/laya (or $LAYA_DIR), downloads the model's weights (about 1.7 GB) into ~/.cache/receptron-laya the first time, and serves it on 127.0.0.1:7720. After that gp serve starts it by itself when a tool needs it",
        looked.join(", ")
    )
}

/// A Laya this process started.
struct Owned {
    child: Child,
    dir: PathBuf,
    log: PathBuf,
    since: std::time::SystemTime,
}

static OWNED: Mutex<Option<Owned>> = Mutex::new(None);

/// Whether a Laya this process started is still running.
fn owned_running(o: &mut Owned) -> bool {
    matches!(o.child.try_wait(), Ok(None))
}

/// What's at 7720 now, for gp serve's `GET /api/laya` and the page.
pub(crate) fn status(home: &Path) -> Value {
    let url = managed_url();
    let mut owned = OWNED.lock().unwrap_or_else(|e| e.into_inner());
    if owned.as_mut().is_some_and(|o| !owned_running(o)) {
        *owned = None;
    }
    let answer = probe(&managed_url());
    let install = install_dir(home);
    let by = match (&answer, owned.as_ref()) {
        (Answer::Decides, Some(_)) => "gp serve",
        (Answer::Decides, None) => "another process",
        (_, Some(_)) => "gp serve (loading)",
        _ => "nobody",
    };
    json!({
        "url": url,
        "answering": answer == Answer::Decides,
        "held_by_other": match &answer { Answer::Other(l) => Some(l.clone()), _ => None },
        "started_by": by,
        "pid": owned.as_ref().map(|o| o.child.id()),
        "since": owned.as_ref().and_then(|o| o.since.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()),
        "install": install.as_ref().map(|d| d.display().to_string()),
        "log": owned.as_ref().map(|o| o.log.display().to_string()),
        "says": match (&answer, owned.as_ref(), &install) {
            (Answer::Decides, Some(o), _) => format!("Laya answers at {url}, started by this gp (pid {}) from {}", std::process::id(), o.dir.display()),
            (Answer::Decides, None, _) => format!("Laya answers at {url}, started by another process; gp serve uses it and leaves it running"),
            (Answer::Other(l), _, _) => format!("port {} is held by something that isn't a decision server ({l})", port()),
            (Answer::Nothing, Some(_), _) => "Laya is loading its model".into(),
            (Answer::Nothing, None, Some(d)) => format!("Laya isn't running; gp serve starts it from {} when a tool needs it", d.display()),
            (Answer::Nothing, None, None) => not_installed(home),
        },
    })
}

/// A Laya answering at 7720: the one running, else one started from the
/// install and waited for. Returns [`status`]. `repo` holds
/// scripts/laya/serve.mjs; `logs` is where its output goes.
pub(crate) fn ensure(home: &Path, repo: &Path, logs: &Path) -> Result<Value> {
    {
        let mut owned = OWNED.lock().unwrap_or_else(|e| e.into_inner());
        if owned.as_mut().is_some_and(|o| !owned_running(o)) {
            *owned = None;
        }
        match probe(&managed_url()) {
            Answer::Decides => {
                drop(owned);
                return Ok(status(home));
            }
            Answer::Other(l) => bail!(
                "port {} is held by something that isn't a decision server ({l}); gp serve won't start Laya over it",
                port()
            ),
            Answer::Nothing if owned.is_some() => {}
            Answer::Nothing => {
                let Some(dir) = install_dir(home) else {
                    bail!("{}", not_installed(home));
                };
                let script = repo.join("scripts/laya/serve.mjs");
                if !script.is_file() {
                    bail!(
                        "{} isn't there: gp serve starts Laya with gameprinter's scripts/laya/serve.mjs",
                        script.display()
                    );
                }
                std::fs::create_dir_all(logs)?;
                let log = logs.join("laya.log");
                let out = std::fs::File::create(&log)
                    .with_context(|| format!("writing {}", log.display()))?;
                let child = Command::new("node")
                    .arg(&script)
                    .arg(&dir)
                    .arg(port().to_string())
                    // serve.mjs exits when this pipe closes: when this process
                    // ends, however it ends.
                    .env("GP_LAYA_EXIT_WITH_STDIN", "1")
                    .stdin(Stdio::piped())
                    .stdout(out.try_clone()?)
                    .stderr(out)
                    .spawn()
                    .context(
                        "starting node for Laya (scripts/laya/serve.mjs needs node on PATH)",
                    )?;
                *owned = Some(Owned {
                    child,
                    dir,
                    log,
                    since: std::time::SystemTime::now(),
                });
            }
        }
    }
    let t = Instant::now();
    loop {
        if probe(&managed_url()) == Answer::Decides {
            return Ok(status(home));
        }
        {
            let mut owned = OWNED.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(o) = owned.as_mut()
                && !owned_running(o)
            {
                let log = std::fs::read_to_string(&o.log).unwrap_or_default();
                let tail: Vec<&str> = log.lines().rev().take(6).collect();
                let tail: Vec<&str> = tail.into_iter().rev().collect();
                let log_path = o.log.display().to_string();
                *owned = None;
                bail!(
                    "Laya stopped while loading ({log_path}): {}",
                    tail.join(" / ")
                );
            }
        }
        if t.elapsed() > LOAD_DEADLINE {
            bail!(
                "Laya didn't answer at {} within {} s of starting; it's still loading or stuck (see {})",
                managed_url(),
                LOAD_DEADLINE.as_secs(),
                logs.join("laya.log").display()
            );
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// Stops the Laya this process started, if any; one started by another
/// process is left alone.
pub(crate) fn stop() -> bool {
    let mut owned = OWNED.lock().unwrap_or_else(|e| e.into_inner());
    let Some(mut o) = owned.take() else {
        return false;
    };
    let _ = o.child.kill();
    let _ = o.child.wait();
    true
}

/// gp serve's address for a tool: $GP_SERVE_URL, else 127.0.0.1:7710.
fn serve_url() -> String {
    std::env::var("GP_SERVE_URL")
        .ok()
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| SERVE_URL.to_owned())
}

/// Before a tool asks `decide` for decisions: a decision server answering
/// there. At the default address one is started when none answers, by gp
/// serve if it runs, else by this process for as long as it runs. Says in
/// words what it did, for the tool's output.
pub(crate) fn for_tool(home: &Path, decide: &str) -> Result<String> {
    match probe(decide) {
        Answer::Decides => return Ok(format!("Laya answers at {decide}")),
        Answer::Other(l) => bail!("{decide} answers, but not as a decision server ({l})"),
        Answer::Nothing => {}
    }
    let managed = managed_url();
    if decide.trim_end_matches('/') != managed {
        bail!(
            "no decision server at {decide}; gp serve starts Laya only at {managed}, so leave --decide at its default or start the server at {decide} yourself"
        );
    }
    let serve = serve_url();
    let host = host_of(&serve)?;
    match exchange(
        host,
        "POST",
        "/api/laya/start",
        "{}",
        LOAD_DEADLINE + Duration::from_secs(10),
    ) {
        Ok((line, body)) if line.contains(" 200") => {
            let says = serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|v| v["says"].as_str().map(str::to_owned))
                .unwrap_or_default();
            return Ok(format!("gp serve at {serve}: {says}"));
        }
        // A gp serve from before this route: it can't start one, so this
        // command does.
        Ok((line, _)) if line.contains(" 404") => {}
        Ok((_, body)) => {
            let why = serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|v| v["error"].as_str().map(str::to_owned))
                .unwrap_or(body);
            bail!("gp serve at {serve} couldn't start Laya: {why}");
        }
        // No gp serve: start one here, for this command.
        Err(_) => {}
    }
    let logs = home.join("work").join("laya-runs");
    let st = ensure(home, &crate::serve::repo_root(), &logs)?;
    Ok(format!(
        "no gp serve at {serve}, so this command started Laya itself and stops it when it ends: {}",
        st["says"].as_str().unwrap_or("")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads a request's head, so closing doesn't reset the connection.
    fn read_head(c: &mut TcpStream) {
        let mut got = Vec::new();
        let mut buf = [0u8; 256];
        while !got.windows(4).any(|w| w == b"\r\n\r\n") {
            match c.read(&mut buf) {
                Ok(0) | Err(_) => return,
                Ok(n) => got.extend_from_slice(&buf[..n]),
            }
        }
    }

    #[test]
    fn nothing_listening_is_nothing() {
        // Port 9 on this machine: nothing listens there.
        assert_eq!(probe("http://127.0.0.1:9"), Answer::Nothing);
    }

    #[test]
    fn another_server_is_told_apart() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut c, _)) = l.accept() {
                read_head(&mut c);
                let _ = c.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nhi",
                );
            }
        });
        assert!(matches!(
            probe(&format!("http://127.0.0.1:{port}")),
            Answer::Other(_)
        ));
    }

    #[test]
    fn a_decision_server_is_known_by_its_path() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut c, _)) = l.accept() {
                read_head(&mut c);
                let b = br#"{"error":"POST /v1/systemone only"}"#;
                let _ = write!(
                    c,
                    "HTTP/1.1 404 Not Found\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    b.len()
                );
                let _ = c.write_all(b);
            }
        });
        assert_eq!(probe(&format!("http://127.0.0.1:{port}")), Answer::Decides);
    }

    #[test]
    fn no_install_says_how_to_get_one() {
        let d = tempfile::tempdir().unwrap();
        let w = not_installed(d.path());
        assert!(w.contains("just laya") && w.contains("1.7 GB"), "{w}");
    }

    #[test]
    fn another_address_is_never_started() {
        let d = tempfile::tempdir().unwrap();
        let e = for_tool(d.path(), "http://127.0.0.1:9")
            .unwrap_err()
            .to_string();
        assert!(e.contains("only at"), "{e}");
    }
}
