//! `gp mechanic watch GAME TOPIC`: the watch panel on a page of its own,
//! for work with no game to play in the mechanic page (a new platform's
//! emulator being built, say). It serves the session's watch files (the
//! agent's log, pins and problem table, [`super::watch`]) and its
//! conversation, and takes the person's notes as turns the agent reads with
//! `gp mechanic show` and `wait`. On 127.0.0.1 only, with the same host and
//! origin checks as the mechanic page. Needs no ROM and no games checkout.

use std::fs;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result};
use gp_evidence::Paths;
use serde_json::{Value, json};

use super::http::{self, Request, Response};
use super::session::Session;
use crate::Outcome;

const MAX_BODY: usize = 1 << 20;

struct Watch {
    session: Session,
    page: PathBuf,
    port: u16,
}

impl Watch {
    fn own_host(&self, req: &Request) -> bool {
        req.headers.get("host").is_some_and(|h| {
            h == &format!("127.0.0.1:{}", self.port) || h == &format!("localhost:{}", self.port)
        })
    }

    fn same_origin(&self, req: &Request) -> bool {
        match req.headers.get("origin") {
            None => true,
            Some(o) => {
                o == &format!("http://127.0.0.1:{}", self.port)
                    || o == &format!("http://localhost:{}", self.port)
            }
        }
    }
}

/// The page's folder: `mechanic/` from the current folder upward, else in
/// the checkout gp was built from.
fn page_dir() -> Result<PathBuf> {
    let has = |d: &Path| d.join("mechanic").join("watch.html").is_file();
    if let Ok(cwd) = std::env::current_dir() {
        for d in cwd.ancestors() {
            if has(d) {
                return Ok(d.join("mechanic"));
            }
        }
    }
    let built = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    if has(&built) {
        return Ok(built.join("mechanic"));
    }
    anyhow::bail!(
        "no mechanic/watch.html here or above: run gp mechanic watch inside a gameprinter checkout"
    )
}

fn handle(w: &Watch, req: &Request) -> Result<Response> {
    if !w.own_host(req) {
        return Ok(Response::error(
            403,
            "this page answers only at 127.0.0.1 or localhost on its own port",
        ));
    }
    if req.method == "POST" && !w.same_origin(req) {
        return Ok(Response::error(403, "only this server's own page writes"));
    }
    let page = |name: &str, kind: &'static str| -> Result<Response> {
        Ok(Response::bytes(kind, fs::read(w.page.join(name))?))
    };
    // Watch sessions' routes (crate::watch_serve, docs/watch.md).
    if let Some(r) = crate::watch_serve::handle(req) {
        return r;
    }
    match (req.method.as_str(), req.path.as_str()) {
        ("GET" | "HEAD", "/") => page("watch.html", "text/html; charset=utf-8"),
        ("GET" | "HEAD", "/watch.js") => page("watch.js", "text/javascript; charset=utf-8"),
        ("GET" | "HEAD", "/style.css") => page("style.css", "text/css; charset=utf-8"),
        ("GET" | "HEAD", "/api/watch") => {
            let mut v = super::watch::view(&w.session.dir);
            v["talk"] = json!(w.session.turns()?);
            v["topic"] = json!(w.session.mechanic());
            v["game"] = json!(
                w.session
                    .dir
                    .parent()
                    .and_then(|p| p.file_name())
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
            );
            Ok(Response::json(&v))
        }
        ("GET" | "HEAD", p) if p.starts_with("/api/watch/files/") => {
            match super::watch::file(&w.session.dir, &p["/api/watch/files/".len()..]) {
                Some((kind, bytes)) => Ok(Response::bytes(kind, bytes)),
                None => Ok(Response::error(404, "no such pinned file")),
            }
        }
        ("POST", "/api/talk") => {
            let v: Value = serde_json::from_slice(&req.body)?;
            let text = v["text"].as_str().unwrap_or("").trim();
            if text.is_empty() {
                return Ok(Response::error(400, "say something"));
            }
            let (n, again) = w
                .session
                .turn_once(json!({"by": "player", "text": text, "clips": []}))?;
            Ok(Response::json(&json!({"turn": n, "again": again})))
        }
        ("POST", _) => Ok(Response::error(404, "no such action")),
        _ => Ok(Response::error(404, "not here")),
    }
}

fn serve_one(w: &Watch, conn: &TcpStream) -> std::io::Result<()> {
    let req = match http::read_request_up_to(conn, MAX_BODY) {
        Ok(r) => r,
        Err(http::ReadError::Bad(status, why)) => {
            return http::write_response(conn, &Response::error(status, why), false);
        }
        Err(http::ReadError::Gone) => return Ok(()),
    };
    // A watch session's event stream (crate::watch_serve) is written as it goes.
    if w.own_host(&req)
        && let Some(done) = crate::watch_serve::stream(&req, conn)
    {
        return done;
    }
    let res = handle(w, &req).unwrap_or_else(|e| Response::error(500, &format!("{e:#}")));
    http::write_response(conn, &res, req.method == "HEAD")
}

/// Opens (or makes) the session and serves its watch page until stopped.
pub(crate) fn run(
    paths: &Paths,
    game: &str,
    topic: &str,
    prompt: Option<&str>,
    port: u16,
) -> Result<Outcome> {
    let session = Session::create(paths, game, topic, prompt, "")?;
    let listener = TcpListener::bind(("127.0.0.1", port))
        .with_context(|| format!("port {port} is taken: give another with --port"))?;
    let port = listener.local_addr()?.port();
    let w = Arc::new(Watch {
        session,
        page: page_dir()?,
        port,
    });
    println!("{game} / {topic}: watch at http://127.0.0.1:{port}/");
    println!(
        "The agent writes here with gp mechanic log, pin and problem {game} {topic}, and reads your notes with gp mechanic show or wait {game} {topic}."
    );
    for stream in listener.incoming().flatten() {
        let w = w.clone();
        std::thread::spawn(move || {
            let _ = serve_one(&w, &stream);
        });
    }
    Ok(Outcome::ok(json!({}), String::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn watch(d: &Path) -> Watch {
        let paths = Paths {
            home: d.to_path_buf(),
            store: d.join("store"),
            roms: d.join("roms"),
        };
        Watch {
            session: Session::create(&paths, "tapper", "emulator", None, "").unwrap(),
            page: Path::new(env!("CARGO_MANIFEST_DIR")).join("../../mechanic"),
            port: 7711,
        }
    }

    fn req(method: &str, path: &str, host: &str, body: &str) -> Request {
        Request {
            method: method.into(),
            path: path.into(),
            headers: [("host".to_owned(), host.to_owned())].into(),
            body: body.as_bytes().to_vec(),
            ..Default::default()
        }
    }

    #[test]
    fn the_watch_page_shows_the_agents_work_and_takes_notes() {
        let d = tempfile::tempdir().unwrap();
        let w = watch(d.path());
        super::super::watch::log(&w.session.dir, "Booting the main Z80").unwrap();
        let r = handle(&w, &req("GET", "/api/watch", "127.0.0.1:7711", "")).unwrap();
        let v: Value = serde_json::from_slice(&r.body).unwrap();
        assert_eq!(v["status"]["text"], "Booting the main Z80");
        assert_eq!(
            (v["game"].as_str(), v["topic"].as_str()),
            (Some("tapper"), Some("emulator"))
        );
        let r = handle(
            &w,
            &req(
                "POST",
                "/api/talk",
                "127.0.0.1:7711",
                r#"{"text":"the bar looks shifted"}"#,
            ),
        )
        .unwrap();
        assert_eq!(r.status, 200);
        assert_eq!(
            w.session.turns().unwrap()[0]["text"],
            "the bar looks shifted"
        );
        assert_eq!(
            handle(&w, &req("GET", "/", "127.0.0.1:7711", ""))
                .unwrap()
                .status,
            200
        );
    }

    #[test]
    fn another_host_or_origin_is_refused() {
        let d = tempfile::tempdir().unwrap();
        let w = watch(d.path());
        assert_eq!(
            handle(&w, &req("GET", "/api/watch", "evil.example:7711", ""))
                .unwrap()
                .status,
            403
        );
        let mut r = req("POST", "/api/talk", "127.0.0.1:7711", r#"{"text":"x"}"#);
        r.headers
            .insert("origin".into(), "http://evil.example".into());
        assert_eq!(handle(&w, &r).unwrap().status, 403);
        assert_eq!(
            handle(
                &w,
                &req(
                    "GET",
                    "/api/watch/files/../session.toml",
                    "127.0.0.1:7711",
                    ""
                )
            )
            .unwrap()
            .status,
            404
        );
    }
}
