//! The host's HTTP face, on 127.0.0.1 only: a JSON API under `/api/` and the
//! playable links under `/play/<id>/`. There is no authentication, so the
//! listener refuses any address but loopback, as `gp serve` does.
//!
//! | Request | Does |
//! |---|---|
//! | `GET /api/health` | `{ok}` |
//! | `POST /api/sessions` `{copy}` | Opens a session |
//! | `GET /api/sessions` | Lists them |
//! | `POST /api/sessions/ID/step` | A [`StepRequest`] |
//! | `POST /api/sessions/ID/query` `{paths}` | Reads state |
//! | `DELETE /api/sessions/ID` | Closes it |
//! | `POST /api/scenarios/run` | A [`scenario::Request`], run now |
//! | `POST /api/scenarios/record` | A [`scenario::Record`] |
//! | `POST /api/scenarios/search` | A [`crate::search::Search`] for holds that reach a goal |
//! | `POST /api/jobs` | A [`JobSpec`], queued; answers the job |
//! | `GET /api/jobs`, `GET /api/jobs/ID` | Status |
//! | `POST /api/links` `{dir}`, `GET /api/links` | Playable links |
//! | `GET /api/speed`, `POST /api/speed` `{name, seconds}` | Speed metrics |
//! | `GET /play/ID/...` | The build's files |

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::local::LocalHost;
use crate::{Host, JobSpec, StepRequest, links, scenario};

const MAX_BODY: usize = 1 << 20;
const MAX_CONNECTIONS: usize = 64;

pub struct Response {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
}

impl Response {
    fn json(status: u16, v: &Value) -> Self {
        Self {
            status,
            content_type: "application/json",
            body: serde_json::to_vec_pretty(v).expect("JSON serializes"),
        }
    }
    fn err(status: u16, msg: impl Into<String>) -> Self {
        Self::json(status, &json!({ "error": msg.into() }))
    }
}

fn parse<T: for<'de> Deserialize<'de>>(body: &[u8]) -> Result<T, Response> {
    let body = if body.is_empty() { b"{}" } else { body };
    serde_json::from_slice(body)
        .map_err(|e| Response::err(400, format!("the body isn't the right JSON: {e}")))
}

fn reply(r: Result<Value, String>) -> Response {
    match r {
        Ok(v) => Response::json(200, &v),
        Err(e) => Response::err(422, e),
    }
}

/// Answers one request. `host` is concrete only for link lookup; every API
/// call goes through the [`Host`] trait.
pub fn handle(host: &LocalHost, method: &str, target: &str, body: &[u8]) -> Response {
    let path = target.split('?').next().unwrap_or("");
    if let Some(rest) = path.strip_prefix("/play/") {
        if method != "GET" && method != "HEAD" {
            return Response::err(405, "links are read-only");
        }
        let (id, file) = rest.split_once('/').unwrap_or((rest, ""));
        let Some(link) = host.link(id) else {
            return Response::err(404, format!("there's no link `{id}`"));
        };
        if !rest.contains('/') {
            // `/play/ID` needs the slash for relative paths to resolve.
            return Response {
                status: 301,
                content_type: "text/plain",
                body: format!("/play/{id}/").into_bytes(),
            };
        }
        return match links::resolve(&link, file)
            .and_then(|p| std::fs::read(&p).ok().map(|b| (p, b)))
        {
            Some((p, bytes)) => Response {
                status: 200,
                content_type: links::content_type(&p),
                body: bytes,
            },
            None => Response::err(404, "no such file in that build"),
        };
    }
    let Some(api) = path.strip_prefix("/api/") else {
        if path == "/" {
            return index(host);
        }
        return Response::err(404, "not found");
    };
    let parts: Vec<&str> = api.split('/').collect();
    match (method, parts.as_slice()) {
        ("GET", ["health"]) => Response::json(200, &json!({ "ok": true })),
        ("POST", ["sessions"]) => {
            #[derive(Deserialize)]
            struct A {
                copy: String,
            }
            match parse::<A>(body) {
                Ok(a) => reply(host.open_session(&a.copy)),
                Err(r) => r,
            }
        }
        ("GET", ["sessions"]) => Response::json(200, &json!({ "sessions": host.sessions() })),
        ("POST", ["sessions", id, "step"]) => match parse::<StepRequest>(body) {
            Ok(s) => reply(host.step_session(id, &s)),
            Err(r) => r,
        },
        ("POST", ["sessions", id, "query"]) => {
            #[derive(Deserialize)]
            struct A {
                #[serde(default)]
                paths: Vec<String>,
            }
            match parse::<A>(body) {
                Ok(a) => reply(host.query_session(id, &a.paths)),
                Err(r) => r,
            }
        }
        ("DELETE", ["sessions", id]) => {
            reply(host.close_session(id).map(|()| json!({ "closed": id })))
        }
        ("POST", ["scenarios", "run"]) => match parse::<scenario::Request>(body) {
            Ok(r) => reply(host.run_scenarios(&r)),
            Err(r) => r,
        },
        ("POST", ["scenarios", "record"]) => match parse::<scenario::Record>(body) {
            Ok(r) => reply(host.record_scenario(&r)),
            Err(r) => r,
        },
        ("POST", ["scenarios", "search"]) => match parse::<crate::search::Search>(body) {
            Ok(r) => reply(host.search_inputs(&r)),
            Err(r) => r,
        },
        ("POST", ["jobs"]) => match parse::<JobSpec>(body) {
            Ok(spec) => reply(host.submit(spec).map(|id| json!({ "job": host.job(&id) }))),
            Err(r) => r,
        },
        ("GET", ["jobs"]) => Response::json(200, &json!({ "jobs": host.jobs() })),
        ("GET", ["jobs", id]) => match host.job(id) {
            Some(j) => Response::json(200, &json!({ "job": j })),
            None => Response::err(404, format!("there's no job `{id}`")),
        },
        ("POST", ["links"]) => {
            #[derive(Deserialize)]
            struct A {
                dir: std::path::PathBuf,
            }
            match parse::<A>(body) {
                Ok(a) => reply(host.register_link(&a.dir).map(|l| json!({ "link": l }))),
                Err(r) => r,
            }
        }
        ("GET", ["links"]) => Response::json(200, &json!({ "links": host.links() })),
        ("GET", ["speed"]) => Response::json(200, &host.speed()),
        ("POST", ["speed"]) => {
            #[derive(Deserialize)]
            struct A {
                name: String,
                seconds: f64,
            }
            match parse::<A>(body) {
                Ok(a) => reply(host.record_speed(&a.name, a.seconds)),
                Err(r) => r,
            }
        }
        _ => Response::err(404, "no such endpoint"),
    }
}

fn index(host: &LocalHost) -> Response {
    let mut html = String::from(
        "<!doctype html><meta charset=utf-8><title>Game Printer host</title><h1>Playable builds</h1><ul>",
    );
    for l in host.links() {
        html.push_str(&format!(
            "<li><a href=\"{}\">{}</a> ({} {})</li>",
            l.path,
            l.copy.as_deref().unwrap_or(&l.id),
            l.template.as_deref().unwrap_or("?"),
            l.target.as_deref().unwrap_or("?")
        ));
    }
    html.push_str("</ul>");
    Response {
        status: 200,
        content_type: "text/html; charset=utf-8",
        body: html.into_bytes(),
    }
}

fn read_request(stream: &mut TcpStream) -> Result<(String, String, Vec<u8>), String> {
    let mut r = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
    let mut line = String::new();
    r.read_line(&mut line).map_err(|e| e.to_string())?;
    let mut it = line.split_whitespace();
    let (method, target) = (
        it.next().ok_or("empty request")?.to_owned(),
        it.next().ok_or("no target")?.to_owned(),
    );
    let mut len = 0usize;
    loop {
        let mut h = String::new();
        r.read_line(&mut h).map_err(|e| e.to_string())?;
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':')
            && k.eq_ignore_ascii_case("content-length")
        {
            len = v.trim().parse().map_err(|_| "bad content-length")?;
        }
    }
    if len > MAX_BODY {
        return Err(format!("a body over {MAX_BODY} bytes"));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body).map_err(|e| e.to_string())?;
    Ok((method, target, body))
}

fn write_response(stream: &mut TcpStream, r: &Response, head_only: bool) {
    let reason = match r.status {
        200 => "OK",
        301 => "Moved Permanently",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Unprocessable Entity",
    };
    let location = if r.status == 301 {
        format!("Location: {}\r\n", String::from_utf8_lossy(&r.body))
    } else {
        String::new()
    };
    let head = format!(
        "HTTP/1.1 {} {reason}\r\nContent-Type: {}\r\nContent-Length: {}\r\n{location}Cache-Control: no-store\r\nConnection: close\r\n\r\n",
        r.status,
        r.content_type,
        r.body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    if !head_only {
        let _ = stream.write_all(&r.body);
    }
    let _ = stream.flush();
}

/// Listens on a loopback address and serves until the process ends. Returns
/// the bound address through `on_listening` before accepting.
pub fn serve(
    host: Arc<LocalHost>,
    port: u16,
    on_listening: impl FnOnce(SocketAddr),
) -> Result<(), String> {
    let listener = TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port))
        .map_err(|e| format!("binding 127.0.0.1:{port}: {e}"))?;
    let addr = listener.local_addr().map_err(|e| e.to_string())?;
    host.set_base(&format!("http://{addr}"));
    on_listening(addr);
    let active = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    for conn in listener.incoming() {
        let Ok(mut stream) = conn else { continue };
        if active.load(std::sync::atomic::Ordering::Relaxed) >= MAX_CONNECTIONS {
            write_response(&mut stream, &Response::err(503, "busy"), false);
            continue;
        }
        active.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let (host, active) = (Arc::clone(&host), Arc::clone(&active));
        std::thread::spawn(move || {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
            match read_request(&mut stream) {
                Ok((method, target, body)) => {
                    let r = handle(&host, &method, &target, &body);
                    write_response(&mut stream, &r, method == "HEAD");
                }
                Err(e) => write_response(&mut stream, &Response::err(400, e), false),
            }
            active.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        });
    }
    Ok(())
}
