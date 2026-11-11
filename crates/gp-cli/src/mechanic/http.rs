//! The little HTTP/1.1 the mechanic page needs: one request per
//! connection, answered with a Content-Length and `Connection: close`.
//! Requests are GET and HEAD, and POST with a body of known length (no
//! chunked bodies). Ported from game-printer's serve/http.rs, less the
//! streamed responses no route here uses. It's written here rather than
//! taken from a crate because the server is local and its requests are
//! simple, and this keeps its whole surface in one file.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

/// The longest request head accepted: request line and headers.
const MAX_HEAD: usize = 16 * 1024;

/// How long a client may take to send its whole request head.
const HEAD_DEADLINE: Duration = Duration::from_secs(2);

/// How long a client may take to send its body, once the head is in. A
/// clip's play from power-on can be megabytes, so this is generous.
const BODY_DEADLINE: Duration = Duration::from_secs(20);

/// A parsed request.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Request {
    pub method: String,
    /// The path, percent-decoded segment by segment, without the query.
    pub path: String,
    /// Query parameters, percent-decoded. A repeated name keeps the last.
    pub query: BTreeMap<String, String>,
    /// Header names in lower case.
    pub headers: BTreeMap<String, String>,
    /// The body, empty unless a Content-Length came with one.
    pub body: Vec<u8>,
}

impl Request {
    pub fn param(&self, name: &str) -> Option<&str> {
        self.query.get(name).map(String::as_str)
    }
}

/// A response: a status, a content type and a body.
#[derive(Debug, Clone)]
pub(crate) struct Response {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
}

impl Response {
    pub fn json(value: &serde_json::Value) -> Self {
        Self {
            status: 200,
            content_type: "application/json",
            body: serde_json::to_vec(value).expect("json"),
        }
    }

    pub fn bytes(content_type: &'static str, body: Vec<u8>) -> Self {
        Self {
            status: 200,
            content_type,
            body,
        }
    }

    /// An error as `{"error": message}`.
    pub fn error(status: u16, message: &str) -> Self {
        Self {
            status,
            content_type: "application/json",
            body: serde_json::to_vec(&serde_json::json!({ "error": message })).expect("json"),
        }
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        411 => "Length Required",
        413 => "Content Too Large",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        _ => "",
    }
}

/// Why a request couldn't be read: the status to answer with, if any.
#[derive(Debug)]
pub(crate) enum ReadError {
    /// The connection closed or timed out before a whole request arrived.
    Gone,
    /// A malformed or oversized request, answered with this status.
    Bad(u16, &'static str),
}

/// Reads one request from `stream` with a body of at most `max` bytes. The
/// head must arrive within [`HEAD_DEADLINE`] in all, however slowly its
/// bytes trickle in.
pub(crate) fn read_request_up_to(stream: &TcpStream, max: usize) -> Result<Request, ReadError> {
    let deadline = std::time::Instant::now() + HEAD_DEADLINE;
    let mut reader = BufReader::new(stream);
    let mut lines = Vec::new();
    let mut total = 0usize;
    loop {
        let mut line = Vec::new();
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                return Err(ReadError::Gone);
            }
            stream
                .set_read_timeout(Some(left))
                .map_err(|_| ReadError::Gone)?;
            let buf = reader.fill_buf().map_err(|_| ReadError::Gone)?;
            if buf.is_empty() {
                return Err(ReadError::Gone);
            }
            let take = buf
                .iter()
                .position(|&b| b == b'\n')
                .map_or(buf.len(), |i| i + 1);
            line.extend_from_slice(&buf[..take]);
            reader.consume(take);
            total += take;
            if total > MAX_HEAD {
                return Err(ReadError::Bad(431, "the request head is too long"));
            }
            if line.ends_with(b"\n") {
                break;
            }
        }
        let line = String::from_utf8(line)
            .map_err(|_| ReadError::Bad(400, "the request head isn't UTF-8"))?;
        let line = line.trim_end_matches(['\r', '\n']).to_owned();
        if line.is_empty() {
            break;
        }
        lines.push(line);
    }
    let mut req = parse_head(&lines)?;
    read_body(&mut reader, stream, &mut req, max)?;
    Ok(req)
}

/// Reads the body a Content-Length announces, at most `max` bytes within
/// [`BODY_DEADLINE`]. A chunked body is refused.
fn read_body(
    reader: &mut BufReader<&TcpStream>,
    stream: &TcpStream,
    req: &mut Request,
    max: usize,
) -> Result<(), ReadError> {
    if req.headers.contains_key("transfer-encoding") {
        return Err(ReadError::Bad(411, "send the body with a Content-Length"));
    }
    let Some(len) = req.headers.get("content-length") else {
        return Ok(());
    };
    if len.is_empty() || !len.bytes().all(|b| b.is_ascii_digit()) {
        return Err(ReadError::Bad(400, "malformed Content-Length"));
    }
    let len: usize = len
        .parse()
        .map_err(|_| ReadError::Bad(413, "the request body is too large"))?;
    if len > max {
        return Err(ReadError::Bad(413, "the request body is too large"));
    }
    let deadline = std::time::Instant::now() + BODY_DEADLINE;
    let mut body = vec![0u8; len];
    let mut got = 0;
    while got < len {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            return Err(ReadError::Gone);
        }
        stream
            .set_read_timeout(Some(left))
            .map_err(|_| ReadError::Gone)?;
        match reader.read(&mut body[got..]) {
            Ok(0) | Err(_) => return Err(ReadError::Gone),
            Ok(n) => got += n,
        }
    }
    req.body = body;
    Ok(())
}

/// Parses a request line and headers.
pub(crate) fn parse_head(lines: &[String]) -> Result<Request, ReadError> {
    let bad = ReadError::Bad(400, "malformed request");
    let (first, rest) = lines
        .split_first()
        .ok_or(ReadError::Bad(400, "empty request"))?;
    let mut parts = first.split(' ');
    let (Some(method), Some(target), Some(version), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(bad);
    };
    if !version.starts_with("HTTP/1.") {
        return Err(bad);
    }
    let mut headers = BTreeMap::new();
    for line in rest {
        let (name, value) = line
            .split_once(':')
            .ok_or(ReadError::Bad(400, "malformed header"))?;
        headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
    }
    let (raw_path, raw_query) = target.split_once('?').unwrap_or((target, ""));
    if !raw_path.starts_with('/') {
        return Err(bad);
    }
    let mut path = String::new();
    for segment in raw_path.split('/').skip(1) {
        path.push('/');
        path.push_str(&percent_decode(segment).ok_or(ReadError::Bad(400, "bad escape in path"))?);
    }
    let mut query = BTreeMap::new();
    for pair in raw_query.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let k = percent_decode(&k.replace('+', " ")).ok_or(ReadError::Bad(400, "bad escape"))?;
        let v = percent_decode(&v.replace('+', " ")).ok_or(ReadError::Bad(400, "bad escape"))?;
        query.insert(k, v);
    }
    Ok(Request {
        method: method.to_owned(),
        path,
        query,
        headers,
        body: Vec::new(),
    })
}

/// Decodes `%XX` escapes. `None` for a broken escape or text that isn't
/// UTF-8 once decoded.
pub(crate) fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Writes `response`, leaving out the body for HEAD.
pub(crate) fn write_response(
    mut stream: &TcpStream,
    response: &Response,
    head_only: bool,
) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
        response.status,
        reason(response.status),
        response.content_type,
        response.body.len()
    );
    stream.write_all(head.as_bytes())?;
    if !head_only {
        stream.write_all(&response.body)?;
    }
    stream.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn head(lines: &[&str]) -> Result<Request, ReadError> {
        parse_head(&lines.iter().map(|l| (*l).to_owned()).collect::<Vec<_>>())
    }

    #[test]
    fn parses_path_query_and_headers() {
        let r = head(&[
            "GET /api/clips/3.log?mechanic=fire%20ball&kind=a+b HTTP/1.1",
            "Host: 127.0.0.1:7710",
            "X-Thing:  spaced ",
        ])
        .unwrap();
        assert_eq!(r.method, "GET");
        assert_eq!(r.path, "/api/clips/3.log");
        assert_eq!(r.param("mechanic"), Some("fire ball"));
        assert_eq!(r.param("kind"), Some("a b"));
        assert_eq!(r.headers["host"], "127.0.0.1:7710");
        assert_eq!(r.headers["x-thing"], "spaced");
    }

    #[test]
    fn refuses_malformed_heads() {
        for lines in [
            &[][..],
            &["GET"][..],
            &["GET /x HTTP/1.1 extra"][..],
            &["GET x HTTP/1.1"][..],
            &["GET /x SPDY/3"][..],
            &["GET /%zz HTTP/1.1"][..],
            &["GET /x HTTP/1.1", "no colon"][..],
        ] {
            assert!(
                matches!(head(lines), Err(ReadError::Bad(..))),
                "{lines:?} was accepted"
            );
        }
    }

    #[test]
    fn decodes_escapes() {
        assert_eq!(percent_decode("a%2Fb").as_deref(), Some("a/b"));
        assert_eq!(percent_decode("%e2%9c%93").as_deref(), Some("\u{2713}"));
        assert_eq!(percent_decode("%"), None);
        assert_eq!(percent_decode("%4"), None);
        assert_eq!(percent_decode("%ff"), None);
    }
}
