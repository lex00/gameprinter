//! Watch sessions over HTTP (`gp watch`, docs/watch.md), from the mechanic
//! servers: the sessions, a session's events after a `seq` (as a page, or
//! pushed as server-sent events that tail the file by byte offset), the
//! files its snapshots name, the clips it kept, its inputs, and L3 detail
//! for a window of any recipe it ran, made again by running it. Nothing
//! here writes a session's events: `gp watch start` is their one writer.
//!
//! The routes and their answers are game-printer's `gp serve` ones
//! (serve/watch.rs), with one change: the session list is at
//! `/api/watch/sessions`, since `/api/watch` is the mechanic page's own
//! watch panel. Both mechanic servers (`gp mechanic start`'s and `gp
//! mechanic watch`'s) answer them, after their host and origin checks,
//! through [`handle`] and [`stream`]:
//!
//! | Route | What |
//! |---|---|
//! | `GET /api/watch/sessions` | The sessions: name, ROM, events and whether a writer is writing |
//! | `GET /api/watch/{name}/events` | A session's events after `?after=SEQ` (default 0), at most 2000 a request |
//! | `GET /api/watch/{name}/stream` | Its events as server-sent events (`text/event-stream`), `id:` the seq, from `?after=SEQ` or `Last-Event-ID` |
//! | `GET /api/watch/{name}/file/{file}` | session.json, or a harvest a snapshot names (static.json, unread.json, static-0.json, unread-0.json) |
//! | `GET /api/watch/{name}/clips/{file}` | The clips the session's search kept: clips.toml and its start presses |
//! | `GET, POST /api/watch/{name}/input` | The session's inputs; POST one (`{v, actor, kind: view, point or note, data}`); a point is answered at once |
//! | `GET /api/watch/{name}/recipes/{hash}` | A recipe the session ran, as an input log from power-on |
//! | `GET /api/watch/{name}/rom` | The session's ROM, when it is in `$GP_ROM_DIR` by hash |
//! | `GET /api/watch/{name}/detail` | L3 for `?recipe=HASH&frames=A..B` of a recipe the session ran |

use std::io::Write;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::Result;
use gp_evidence::Paths;
use serde_json::{Value, json};

use crate::mechanic::http::{Request, Response};

/// The most events one request returns; a client asks again from the last.
const PAGE: usize = 2000;
/// How long one stream lasts before the browser reconnects with
/// `Last-Event-ID`.
const STREAM_FOR: Duration = Duration::from_secs(600);
/// How often a stream looks at the file for new lines.
const STREAM_POLL: Duration = Duration::from_millis(200);
/// How long a quiet stream waits before a comment that keeps it open.
const KEEP_ALIVE: Duration = Duration::from_secs(15);

/// The session list's route.
const SESSIONS: &str = "/api/watch/sessions";

/// A route's session name and the rest of its path, for a path under
/// `/api/watch/` that isn't the mechanic page's own (`files/...`).
fn split(path: &str) -> Option<(&str, &str)> {
    let rest = path.strip_prefix("/api/watch/")?;
    let (name, tail) = rest.split_once('/')?;
    if name == "files" {
        return None;
    }
    Some((name, tail))
}

/// The answer to a watch route, or `None` for a path that isn't one. The
/// caller has checked the host, and the origin of a POST.
pub(crate) fn handle(req: &Request) -> Option<Result<Response>> {
    if req.path == SESSIONS {
        return Some((|| {
            let p = Paths::from_env()?;
            Ok(Response::json(
                &json!({"sessions": crate::watch::sessions(&p)?}),
            ))
        })());
    }
    let (name, tail) = split(&req.path)?;
    if tail == "stream" {
        return None;
    }
    Some(match Paths::from_env() {
        Ok(paths) => route(&paths, req, name, tail),
        Err(e) => Err(e.into()),
    })
}

fn not_found(why: &str) -> Response {
    Response::error(404, why)
}

fn bad_request(why: &str) -> Response {
    Response::error(400, why)
}

/// The session's folder, or the answer that refuses it.
fn session(paths: &Paths, name: &str) -> std::result::Result<PathBuf, Response> {
    let dir =
        crate::watch::session_dir(&paths.home, name).map_err(|e| bad_request(&e.to_string()))?;
    if !dir.join("session.json").is_file() {
        return Err(not_found(&format!("no watch session {name}")));
    }
    Ok(dir)
}

fn route(paths: &Paths, req: &Request, name: &str, tail: &str) -> Result<Response> {
    let get = req.method == "GET" || req.method == "HEAD";
    let dir = match session(paths, name) {
        Ok(d) => d,
        Err(r) => return Ok(r),
    };
    if tail == "input" {
        return input(&dir, req, name);
    }
    if !get {
        return Ok(Response::error(405, "this route answers GET"));
    }
    match tail.split_once('/') {
        None if tail == "events" => events(&dir, req, name),
        None if tail == "rom" => rom(paths, &dir),
        None if tail == "detail" => detail(&dir, req),
        Some(("file", f)) => file(&dir, f),
        Some(("clips", f)) => clips(&dir, f),
        Some(("recipes", h)) => recipe(&dir, h),
        _ => Ok(not_found("no such watch route")),
    }
}

fn events(dir: &Path, req: &Request, name: &str) -> Result<Response> {
    let after = match req.param("after") {
        None => 0,
        Some(a) => match a.parse::<u64>() {
            Ok(n) => n,
            Err(_) => return Ok(bad_request("after is a sequence number")),
        },
    };
    let evs = gp_capture::events_after(dir, after, PAGE)?;
    Ok(Response::json(&json!({"name": name, "events": evs})))
}

/// A plain file name: no folder, nothing hidden.
fn plain(name: &str) -> Option<&str> {
    (!name.is_empty() && !name.starts_with('.') && !name.contains(['/', '\\', '\0']))
        .then_some(name)
}

fn file(dir: &Path, name: &str) -> Result<Response> {
    let Some(name) = plain(name) else {
        return Ok(not_found("no such file"));
    };
    if !matches!(
        name,
        "session.json" | "static.json" | "unread.json" | "static-0.json" | "unread-0.json"
    ) {
        return Ok(not_found("not a session file"));
    }
    Ok(match std::fs::read(dir.join(name)) {
        Ok(body) => Response::bytes("application/json", body),
        Err(_) => not_found("the session has no such file yet"),
    })
}

/// A served file's type, by its extension.
fn content_type(name: &str) -> &'static str {
    match name.rsplit_once('.').map(|(_, e)| e) {
        Some("json") => "application/json",
        Some("toml" | "presses" | "txt" | "input") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn clips(dir: &Path, name: &str) -> Result<Response> {
    let Some(name) = plain(name) else {
        return Ok(not_found("no such file"));
    };
    Ok(match std::fs::read(dir.join("clips").join(name)) {
        Ok(body) => Response::bytes(content_type(name), body),
        Err(_) => not_found("the session kept no such file"),
    })
}

fn input(dir: &Path, req: &Request, name: &str) -> Result<Response> {
    if req.method == "GET" || req.method == "HEAD" {
        let inputs = crate::watch::read_inputs(dir)?;
        return Ok(Response::json(&json!({"name": name, "inputs": inputs})));
    }
    if req.method != "POST" {
        return Ok(Response::error(405, "POST an input, or GET the list"));
    }
    let mut input: gp_events::watch::WatchInput = match serde_json::from_slice(&req.body) {
        Ok(i) => i,
        Err(e) => return Ok(bad_request(&format!("not an input: {e}"))),
    };
    if input.kind == "tool" {
        return Ok(Response::error(
            422,
            "a tool input comes from an agent's tool, not over HTTP",
        ));
    }
    if input.kind == "point" {
        let d = &input.data;
        let (Some(recipe), Some(frame)) = (d["recipe"].as_str(), d["frame"].as_u64()) else {
            return Ok(bad_request("a point names its recipe and frame"));
        };
        let r: Vec<u16> = d["rect"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_u64().map(|n| n as u16))
                    .collect()
            })
            .unwrap_or_default();
        if r.len() != 4 {
            return Ok(bad_request("a point's rect is [x, y, w, h]"));
        }
        match crate::watch::point(dir, recipe, frame, (r[0], r[1], r[2], r[3])) {
            Ok(answer) => input.data["answer"] = answer,
            Err(e) => return Ok(bad_request(&e.to_string())),
        }
    }
    match crate::watch::append_input(dir, &input) {
        Ok(n) => Ok(Response::json(
            &json!({"name": name, "inputs": n, "input": input}),
        )),
        Err(e) => Ok(Response::error(422, &e.to_string())),
    }
}

fn recipe(dir: &Path, hash: &str) -> Result<Response> {
    let clip = match crate::watch::recipe(dir, hash) {
        Ok(c) => c,
        Err(e) => return Ok(not_found(&e.to_string())),
    };
    let log = gp_evidence::input_log::InputLog {
        controllers: 1,
        frames: clip.inputs,
        ..gp_evidence::input_log::InputLog::new(1)
    };
    Ok(Response::bytes(
        "text/plain; charset=utf-8",
        log.to_text().into_bytes(),
    ))
}

/// The ROM path a session was started on.
fn rom_path(dir: &Path) -> Result<std::result::Result<String, Response>> {
    let s: Value = serde_json::from_slice(&std::fs::read(dir.join("session.json"))?)?;
    Ok(s["rom"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| not_found("the session names no ROM")))
}

/// The session's ROM, served only when a file with its hash is in
/// `$GP_ROM_DIR`, as the mechanic page serves the ROMs a person owns.
fn rom(paths: &Paths, dir: &Path) -> Result<Response> {
    let path = match rom_path(dir)? {
        Ok(p) => p,
        Err(r) => return Ok(r),
    };
    let Ok(bytes) = std::fs::read(&path) else {
        return Ok(not_found(&format!(
            "the session's ROM {path} can't be read"
        )));
    };
    let sha = gp_evidence::source::identify_nes(&bytes)?.sha256;
    Ok(
        match crate::open::RomFolder::new(paths).read(&sha).ok().flatten() {
            Some(owned) => Response::bytes("application/octet-stream", owned),
            None => not_found("the session's ROM isn't in $GP_ROM_DIR"),
        },
    )
}

fn detail(dir: &Path, req: &Request) -> Result<Response> {
    let (Some(hash), Some(frames)) = (req.param("recipe"), req.param("frames")) else {
        return Ok(bad_request("detail needs ?recipe=HASH&frames=A..B"));
    };
    let (a, last) = match crate::reach::frame_range(frames) {
        Ok(r) => r,
        Err(e) => return Ok(bad_request(&e.to_string())),
    };
    let clip = match crate::watch::recipe(dir, hash) {
        Ok(c) => c,
        Err(e) => return Ok(not_found(&e.to_string())),
    };
    let path = match rom_path(dir)? {
        Ok(p) => p,
        Err(r) => return Ok(r),
    };
    let Ok(rom) = std::fs::read(&path) else {
        return Ok(not_found(&format!(
            "the session's ROM {path} can't be read"
        )));
    };
    let (v, _) = crate::reach::detail_of(&rom, &clip, a, last)?;
    Ok(Response::json(&v))
}

/// Where a stream starts: `Last-Event-ID` (a reconnect), else `?after=`.
fn stream_start(req: &Request) -> std::result::Result<u64, ()> {
    let raw = req
        .headers
        .get("last-event-id")
        .map(String::as_str)
        .or(req.param("after"));
    match raw {
        None | Some("") => Ok(0),
        Some(a) => a.trim().parse::<u64>().map_err(|_| ()),
    }
}

/// `GET /api/watch/{name}/stream`, answered on `conn` itself: `None` for
/// any other request. The caller has checked the host. The answer has no
/// length: it lasts ten minutes or until the client goes, and the browser's
/// EventSource reconnects with `Last-Event-ID`.
pub(crate) fn stream(req: &Request, conn: &TcpStream) -> Option<std::io::Result<()>> {
    let (name, "stream") = split(&req.path)? else {
        return None;
    };
    if req.method != "GET" {
        return None;
    }
    let refuse = |status: u16, why: &str| {
        crate::mechanic::http::write_response(conn, &Response::error(status, why), false)
    };
    let paths = match Paths::from_env() {
        Ok(p) => p,
        Err(e) => return Some(refuse(500, &format!("{e:#}"))),
    };
    let dir = match session(&paths, name) {
        Ok(d) => d,
        Err(r) => return Some(crate::mechanic::http::write_response(conn, &r, false)),
    };
    let Ok(after) = stream_start(req) else {
        return Some(refuse(400, "after and Last-Event-ID are sequence numbers"));
    };
    let mut w = conn;
    Some((|| {
        w.write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
        )?;
        sse(&dir, after, &mut w, Instant::now() + STREAM_FOR)
    })())
}

/// Writes the events in `dir`'s `events.ndjson` after `after` as
/// server-sent events until `until` or the client goes: `id:` is the
/// event's seq, `data:` the event as one JSON line, so `Last-Event-ID`
/// resumes where the stream left off. It tails the file
/// (`gp_capture::Tail`), reading only the bytes added since its last look.
pub(crate) fn sse(
    dir: &Path,
    after: u64,
    w: &mut dyn Write,
    until: Instant,
) -> std::io::Result<()> {
    let file: PathBuf = dir.join("events.ndjson");
    w.write_all(b"retry: 1000\n\n")?;
    w.flush()?;
    let mut tail = gp_capture::Tail::default();
    let mut quiet_since = Instant::now();
    while Instant::now() < until {
        let mut sent = false;
        for (e, line) in tail.read(&file).unwrap_or_default() {
            let Some(s) = e["seq"].as_u64() else { continue };
            if s > after {
                write!(w, "id: {s}\ndata: {line}\n\n")?;
                sent = true;
            }
        }
        if sent {
            w.flush()?;
            quiet_since = Instant::now();
        } else {
            if quiet_since.elapsed() >= KEEP_ALIVE {
                w.write_all(b": keep-alive\n\n")?;
                quiet_since = Instant::now();
            }
            w.flush()?;
            std::thread::sleep(STREAM_POLL);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stream sends every event after the one asked for, with its seq as
    /// the id, and a session's other routes answer by name.
    #[test]
    fn a_session_streams_its_events_after_a_seq() {
        let d = tempfile::tempdir().unwrap();
        let s = d.path().join("watch/dk");
        std::fs::create_dir_all(&s).unwrap();
        std::fs::write(
            s.join("events.ndjson"),
            "{\"seq\":1,\"kind\":\"a\"}\n{\"seq\":2,\"kind\":\"b\"}\n{\"seq\":3,\"kind\":\"c\"}\n",
        )
        .unwrap();
        let mut out = Vec::new();
        sse(&s, 1, &mut out, Instant::now() + Duration::from_millis(50)).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.starts_with("retry: 1000\n\n"), "{text}");
        assert!(text.contains("id: 2\ndata: {\"seq\":2,\"kind\":\"b\"}\n\n"));
        assert!(text.contains("id: 3\n"));
        assert!(!text.contains("id: 1\n"));
        assert_eq!(split("/api/watch/dk/stream"), Some(("dk", "stream")));
        assert_eq!(split("/api/watch/files/log.txt"), None);
        assert_eq!(split("/api/watch"), None);
    }
}
