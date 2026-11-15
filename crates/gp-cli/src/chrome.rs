//! Headless Chrome for `gp theme look play --shots`, with nothing but gp:
//! the browser fetched by gp itself, and driven over the DevTools protocol
//! from here, so a released gp needs no checkout, no Node and no
//! playwright-core (issue #6).
//!
//! The browser is Chrome Headless Shell, pinned by version and SHA-256 as
//! scripts/inspector/fetch-browser.sh pins it (a test keeps the two the
//! same), and unpacked to the same place, `$GP_HOME/work/chrome-headless-shell/<version>/`,
//! so either one's download serves both.
//!
//! The driver starts the browser with `--remote-debugging-port=0`, reads
//! the port it took from `DevToolsActivePort` in its own profile folder,
//! and speaks the protocol over a WebSocket on 127.0.0.1 (a small client
//! here: text frames, masked as a client must, with pings answered). One
//! page is opened and attached to, and each command waits for its answer;
//! the events read on the way are the console lines and failed loads.

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead as _, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use sha2::Digest as _;

/// The pinned build: the one playwright-core 1.64.0 is made for.
pub(crate) const VERSION: &str = "156.0.8078.4";
const CDN: &str = "https://cdn.playwright.dev/builds/cft";

/// This machine's pinned download: (platform, SHA-256 of the zip, the
/// executable inside it).
pub(crate) fn pin() -> Option<(&'static str, &'static str, &'static str)> {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some((
            "mac-arm64",
            "94e4337b70cbc5638f0935fbc46b9f242bc7416f657e3fbe924baf348404b53d",
            "chrome-headless-shell-mac-arm64/chrome-headless-shell",
        ))
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some((
            "linux64",
            "c3e50596aef52c58f8fda3c6b3411c33d34c88b177b3ed1c2243359fd3b8ba20",
            "chrome-headless-shell-linux64/chrome-headless-shell",
        ))
    } else {
        None
    }
}

/// The browser in the home, when one was unpacked there.
pub(crate) fn installed(home: &Path) -> Option<PathBuf> {
    let base = home.join("work").join("chrome-headless-shell");
    let mut found: Vec<PathBuf> = Vec::new();
    for v in fs::read_dir(&base).into_iter().flatten().flatten() {
        for d in fs::read_dir(v.path()).into_iter().flatten().flatten() {
            let exe = d.path().join("chrome-headless-shell");
            if exe.is_file() {
                found.push(exe);
            }
        }
    }
    found.sort();
    found.pop()
}

/// The browser to drive: given, else the one in the home, else the pinned
/// build downloaded now (about 100 MB, once) and checked against its pin
/// before anything is unpacked.
pub(crate) fn browser(given: Option<&Path>, home: &Path) -> Result<PathBuf> {
    if let Some(b) = given {
        return Ok(b.to_owned());
    }
    if let Some(b) = installed(home) {
        return Ok(b);
    }
    fetch(home)
}

fn fetch(home: &Path) -> Result<PathBuf> {
    let Some((platform, sha256, exe)) = pin() else {
        bail!(
            "--shots needs Chrome Headless Shell, and gp has no pinned download of it for this machine ({} {}); install one and give it with --browser",
            std::env::consts::OS,
            std::env::consts::ARCH
        );
    };
    let dir = home
        .join("work")
        .join("chrome-headless-shell")
        .join(VERSION);
    fs::create_dir_all(&dir).with_context(|| format!("making {}", dir.display()))?;
    let zip = dir.join(format!("chrome-headless-shell-{platform}.zip"));
    if !zip.is_file() {
        let url = format!("{CDN}/{VERSION}/{platform}/chrome-headless-shell-{platform}.zip");
        eprintln!(
            "--shots: downloading Chrome Headless Shell {VERSION} for {platform} (about 100 MB, once) into {}",
            dir.display()
        );
        let partial = zip.with_extension("zip.partial");
        let st = Command::new("curl")
            .args(["-sSfL", "--retry", "2", "-o"])
            .arg(&partial)
            .arg(&url)
            .status()
            .context("running curl (--shots fetches its browser with curl)")?;
        if !st.success() {
            bail!("the download of Chrome Headless Shell failed from {url}");
        }
        fs::rename(&partial, &zip)?;
    }
    let bytes = fs::read(&zip).with_context(|| format!("reading {}", zip.display()))?;
    let actual = hex::encode(sha2::Sha256::digest(&bytes));
    if actual != sha256 {
        bail!(
            "{} has SHA-256 {actual}, not the pinned {sha256}: move it aside and run again; if it still differs, the build changed upstream and nothing was unpacked",
            zip.display()
        );
    }
    zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .and_then(|mut z| z.extract(&dir))
        .with_context(|| format!("unpacking {}", zip.display()))?;
    let exe = dir.join(exe);
    if !exe.is_file() {
        bail!("{} isn't in {}", exe.display(), zip.display());
    }
    fs::write(dir.join(".sha256"), format!("{sha256}\n"))?;
    Ok(exe)
}

// ---------------------------------------------------------------------
// The WebSocket.

struct Socket {
    r: BufReader<TcpStream>,
    w: TcpStream,
}

impl Socket {
    fn open(port: u16, path: &str) -> Result<Socket> {
        let w = TcpStream::connect(("127.0.0.1", port))
            .with_context(|| format!("connecting to the browser on port {port}"))?;
        w.set_read_timeout(Some(Duration::from_secs(300)))?;
        w.set_nodelay(true)?;
        let mut r = BufReader::new(w.try_clone()?);
        let mut w = w;
        write!(
            w,
            "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: Z3AtdGhlbWUtc2hvdHM=\r\nSec-WebSocket-Version: 13\r\n\r\n"
        )?;
        let mut status = String::new();
        r.read_line(&mut status)?;
        if !status.contains(" 101 ") {
            bail!(
                "the browser refused the DevTools connection: {}",
                status.trim()
            );
        }
        loop {
            let mut l = String::new();
            if r.read_line(&mut l)? == 0 || l == "\r\n" {
                break;
            }
        }
        Ok(Socket { r, w })
    }

    fn frame(&mut self, op: u8, data: &[u8]) -> Result<()> {
        let mut f = vec![0x80 | op];
        let n = data.len();
        if n < 126 {
            f.push(0x80 | n as u8);
        } else if n <= 0xffff {
            f.push(0x80 | 126);
            f.extend_from_slice(&(n as u16).to_be_bytes());
        } else {
            f.push(0x80 | 127);
            f.extend_from_slice(&(n as u64).to_be_bytes());
        }
        let mask = [0x67, 0x70, 0x73, 0x68];
        f.extend_from_slice(&mask);
        f.extend(data.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
        self.w.write_all(&f)?;
        Ok(())
    }

    fn send(&mut self, text: &str) -> Result<()> {
        self.frame(0x1, text.as_bytes())
    }

    /// The next whole text message.
    fn recv(&mut self) -> Result<String> {
        let mut msg = Vec::new();
        loop {
            let mut h = [0u8; 2];
            self.r.read_exact(&mut h)?;
            let (fin, op) = (h[0] & 0x80 != 0, h[0] & 0x0f);
            let mut n = u64::from(h[1] & 0x7f);
            if n == 126 {
                let mut b = [0u8; 2];
                self.r.read_exact(&mut b)?;
                n = u64::from(u16::from_be_bytes(b));
            } else if n == 127 {
                let mut b = [0u8; 8];
                self.r.read_exact(&mut b)?;
                n = u64::from_be_bytes(b);
            }
            let mask = if h[1] & 0x80 != 0 {
                let mut m = [0u8; 4];
                self.r.read_exact(&mut m)?;
                Some(m)
            } else {
                None
            };
            let mut data = vec![0u8; usize::try_from(n)?];
            self.r.read_exact(&mut data)?;
            if let Some(m) = mask {
                for (i, b) in data.iter_mut().enumerate() {
                    *b ^= m[i % 4];
                }
            }
            match op {
                0x0..=0x2 => {
                    msg.extend_from_slice(&data);
                    if fin {
                        return Ok(String::from_utf8(msg)?);
                    }
                }
                0x8 => bail!("the browser closed the DevTools connection"),
                0x9 => self.frame(0xA, &data)?,
                _ => {}
            }
        }
    }
}

// ---------------------------------------------------------------------
// The browser.

/// A page in a headless browser this process started, and what the page
/// said on its console.
pub(crate) struct Page {
    child: Child,
    ws: Socket,
    session: String,
    next: u64,
    /// The page's console lines, its uncaught errors and its failed loads.
    pub(crate) console: Vec<String>,
    /// How many of those count as errors.
    pub(crate) errors: u64,
    urls: BTreeMap<String, String>,
    loaded: bool,
    _profile: tempfile::TempDir,
}

/// A load the page asks for and may lack without harm: a game with no
/// hooks has no module.wasm, and the player runs its own interpreter.
fn optional(url: &str) -> bool {
    let path = url.split('?').next().unwrap_or(url);
    path.ends_with("/module.wasm")
}

/// A console argument as text, as a browser's console prints it.
fn arg_text(a: &Value) -> String {
    match &a["value"] {
        Value::String(s) => s.clone(),
        Value::Null => a["unserializableValue"]
            .as_str()
            .or(a["description"].as_str())
            .unwrap_or(a["type"].as_str().unwrap_or(""))
            .to_owned(),
        v => v.to_string(),
    }
}

impl Page {
    /// Start the browser with a profile in `work` and open a page of
    /// `width` by `height`.
    pub(crate) fn launch(browser: &Path, work: &Path, width: u32, height: u32) -> Result<Page> {
        let profile = tempfile::Builder::new()
            .prefix("chrome-profile-")
            .tempdir_in(work)?;
        let log = work.join("chrome-stderr.txt");
        let mut child = Command::new(browser)
            .arg("--remote-debugging-port=0")
            .arg(format!("--user-data-dir={}", profile.path().display()))
            .args([
                "--no-sandbox",
                "--no-first-run",
                "--no-default-browser-check",
                "--autoplay-policy=no-user-gesture-required",
                "--disable-background-timer-throttling",
                "--disable-backgrounding-occluded-windows",
                "--disable-renderer-backgrounding",
                "--hide-scrollbars",
                "--mute-audio",
                "about:blank",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(fs::File::create(&log)?)
            .spawn()
            .with_context(|| format!("starting {}", browser.display()))?;
        let file = profile.path().join("DevToolsActivePort");
        let t0 = Instant::now();
        let (port, path) = loop {
            if let Ok(text) = fs::read_to_string(&file) {
                let mut l = text.lines();
                if let (Some(p), Some(path)) = (l.next(), l.next())
                    && let Ok(p) = p.trim().parse::<u16>()
                {
                    break (p, path.trim().to_owned());
                }
            }
            if let Some(st) = child.try_wait()? {
                bail!(
                    "{} stopped at once ({st}): {}",
                    browser.display(),
                    fs::read_to_string(&log)
                        .unwrap_or_default()
                        .lines()
                        .rev()
                        .take(6)
                        .collect::<Vec<_>>()
                        .join(" / ")
                );
            }
            if t0.elapsed() > Duration::from_secs(60) {
                let _ = child.kill();
                bail!(
                    "{} didn't open its DevTools port in a minute",
                    browser.display()
                );
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        let ws = match Socket::open(port, &path) {
            Ok(ws) => ws,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e);
            }
        };
        let mut p = Page {
            child,
            ws,
            session: String::new(),
            next: 0,
            console: Vec::new(),
            errors: 0,
            urls: BTreeMap::new(),
            loaded: false,
            _profile: profile,
        };
        let target = p.browser_call("Target.createTarget", json!({ "url": "about:blank" }))?;
        let id = target["targetId"].as_str().unwrap_or_default().to_owned();
        let at = p.browser_call(
            "Target.attachToTarget",
            json!({ "targetId": id, "flatten": true }),
        )?;
        p.session = at["sessionId"]
            .as_str()
            .ok_or_else(|| anyhow!("the browser gave no session for the page"))?
            .to_owned();
        for m in [
            "Page.enable",
            "Runtime.enable",
            "Network.enable",
            "Log.enable",
            "DOM.enable",
        ] {
            p.call(m, json!({}))?;
        }
        p.call(
            "Emulation.setDeviceMetricsOverride",
            json!({ "width": width, "height": height, "deviceScaleFactor": 1, "mobile": false }),
        )?;
        Ok(p)
    }

    fn note(&mut self, line: String, bad: bool) {
        self.console.push(line);
        if bad {
            self.errors += 1;
        }
    }

    fn event(&mut self, v: &Value) {
        let p = &v["params"];
        match v["method"].as_str().unwrap_or("") {
            "Runtime.consoleAPICalled" => {
                let kind = p["type"].as_str().unwrap_or("log").to_owned();
                let text: Vec<String> = p["args"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(arg_text)
                    .collect();
                let text = text.join(" ");
                let bad = kind == "error" && !text.starts_with("Failed to load resource");
                self.note(format!("[{kind}] {text}"), bad);
            }
            "Log.entryAdded" => {
                let e = &p["entry"];
                let kind = match e["level"].as_str().unwrap_or("info") {
                    "verbose" => "debug",
                    l => l,
                }
                .to_owned();
                let text = e["text"].as_str().unwrap_or("").to_owned();
                let bad = kind == "error" && !text.starts_with("Failed to load resource");
                self.note(format!("[{kind}] {text}"), bad);
            }
            "Runtime.exceptionThrown" => {
                let d = &p["exceptionDetails"];
                let text = d["exception"]["description"]
                    .as_str()
                    .or(d["text"].as_str())
                    .unwrap_or("")
                    .lines()
                    .next()
                    .unwrap_or("")
                    .to_owned();
                self.note(format!("[pageerror] {text}"), true);
            }
            "Network.requestWillBeSent" => {
                if let (Some(id), Some(url)) =
                    (p["requestId"].as_str(), p["request"]["url"].as_str())
                {
                    self.urls.insert(id.to_owned(), url.to_owned());
                }
            }
            "Network.responseReceived" => {
                let status = p["response"]["status"].as_u64().unwrap_or(0);
                if status >= 400 {
                    let url = p["response"]["url"].as_str().unwrap_or("").to_owned();
                    let bad = !(status == 404 && optional(&url));
                    self.note(format!("[http {status}] {url}"), bad);
                }
            }
            "Network.loadingFailed" => {
                let url = p["requestId"]
                    .as_str()
                    .and_then(|id| self.urls.get(id))
                    .cloned()
                    .unwrap_or_default();
                let why = p["errorText"].as_str().unwrap_or("").to_owned();
                let bad = !optional(&url);
                self.note(format!("[requestfailed] {url} {why}"), bad);
            }
            "Page.loadEventFired" => self.loaded = true,
            _ => {}
        }
    }

    fn send(&mut self, session: Option<&str>, method: &str, params: Value) -> Result<Value> {
        self.next += 1;
        let id = self.next;
        let mut m = json!({ "id": id, "method": method, "params": params });
        if let Some(s) = session {
            m["sessionId"] = json!(s);
        }
        self.ws.send(&m.to_string())?;
        loop {
            let v: Value = serde_json::from_str(&self.ws.recv()?)?;
            if v["id"].as_u64() == Some(id) {
                if let Some(e) = v.get("error") {
                    bail!("{method}: {}", e["message"].as_str().unwrap_or("failed"));
                }
                return Ok(v["result"].clone());
            }
            if v.get("method").is_some() {
                self.event(&v);
            }
        }
    }

    fn browser_call(&mut self, method: &str, params: Value) -> Result<Value> {
        self.send(None, method, params)
    }

    fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        let s = self.session.clone();
        self.send(Some(&s), method, params)
    }

    /// Go to `url` and wait for its load event (two minutes at most).
    pub(crate) fn goto(&mut self, url: &str) -> Result<()> {
        self.loaded = false;
        let r = self.call("Page.navigate", json!({ "url": url }))?;
        if let Some(e) = r["errorText"].as_str() {
            bail!("loading {url}: {e}");
        }
        let t0 = Instant::now();
        while !self.loaded {
            if t0.elapsed() > Duration::from_secs(120) {
                bail!("{url} didn't finish loading in two minutes");
            }
            let v: Value = serde_json::from_str(&self.ws.recv()?)?;
            if v.get("method").is_some() {
                self.event(&v);
            }
        }
        Ok(())
    }

    /// The value of a JavaScript expression in the page, its promise
    /// awaited.
    pub(crate) fn eval(&mut self, expr: &str) -> Result<Value> {
        let r = self.call(
            "Runtime.evaluate",
            json!({ "expression": expr, "returnByValue": true, "awaitPromise": true }),
        )?;
        if let Some(d) = r.get("exceptionDetails") {
            bail!(
                "{}",
                d["exception"]["description"]
                    .as_str()
                    .or(d["text"].as_str())
                    .unwrap_or("the script failed")
            );
        }
        Ok(r["result"]["value"].clone())
    }

    /// Poll `expr` every 100 ms until it gives something other than null,
    /// false or an error, for `secs` at most: None when it never does.
    pub(crate) fn wait_for(&mut self, expr: &str, secs: u64) -> Option<Value> {
        let t0 = Instant::now();
        loop {
            match self.eval(expr) {
                Ok(Value::Null | Value::Bool(false)) | Err(_) => {}
                Ok(v) => return Some(v),
            }
            if t0.elapsed() > Duration::from_secs(secs) {
                return None;
            }
            self.pause(100);
        }
    }

    /// Wait `ms` milliseconds (what the page says meanwhile is read with
    /// the next command's answer).
    pub(crate) fn pause(&mut self, ms: u64) {
        std::thread::sleep(Duration::from_millis(ms));
    }

    /// Choose `file` in the page's file input that `selector` names.
    pub(crate) fn set_file(&mut self, selector: &str, file: &Path) -> Result<()> {
        let r = self.call(
            "Runtime.evaluate",
            json!({ "expression": format!("document.querySelector({})", json!(selector)) }),
        )?;
        let Some(obj) = r["result"]["objectId"].as_str() else {
            bail!("the page has no {selector}");
        };
        let obj = obj.to_owned();
        let file = file.canonicalize()?;
        self.call(
            "DOM.setFileInputFiles",
            json!({ "files": [file], "objectId": obj }),
        )?;
        Ok(())
    }

    /// Close the browser.
    pub(crate) fn close(mut self) {
        let _ = self.browser_call("Browser.close", json!({}));
        let t0 = Instant::now();
        while t0.elapsed() < Duration::from_secs(5) {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Page {
    fn drop(&mut self) {
        if let Ok(None) = self.child.try_wait() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pins here are fetch-browser.sh's, so both fetch the same build
    /// into the same folder.
    #[test]
    fn pins_match_fetch_browser() {
        let script = fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/inspector/fetch-browser.sh"),
        )
        .unwrap();
        assert!(script.contains(&format!("version=\"{VERSION}\"")));
        assert!(script.contains(&format!("cdn=\"{CDN}/$version\"")));
        for (platform, sha, exe) in [
            (
                "mac-arm64",
                "94e4337b70cbc5638f0935fbc46b9f242bc7416f657e3fbe924baf348404b53d",
                "chrome-headless-shell-mac-arm64/chrome-headless-shell",
            ),
            (
                "linux64",
                "c3e50596aef52c58f8fda3c6b3411c33d34c88b177b3ed1c2243359fd3b8ba20",
                "chrome-headless-shell-linux64/chrome-headless-shell",
            ),
        ] {
            assert!(script.contains(&format!("platform=\"{platform}\"")));
            assert!(script.contains(&format!("sha256=\"{sha}\"")));
            assert!(script.contains(&format!("exe=\"{exe}\"")));
        }
        if let Some(p) = pin() {
            assert!(script.contains(&format!("sha256=\"{}\"", p.1)));
        }
    }

    #[test]
    fn optional_loads() {
        assert!(optional("http://127.0.0.1:1/game/module.wasm"));
        assert!(optional("http://127.0.0.1:1/game/module.wasm?v=2"));
        assert!(!optional("http://127.0.0.1:1/game/lift/gp-web-lift.wasm"));
    }

    /// A WebSocket frame written by the client reads back the same through
    /// the reader (the reader unmasks what is masked).
    #[test]
    fn frames_round_trip() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (s, _) = l.accept().unwrap();
            let mut r = BufReader::new(s.try_clone().unwrap());
            let mut w = s;
            let mut line = String::new();
            loop {
                line.clear();
                r.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
            }
            w.write_all(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n\r\n")
                .unwrap();
            let mut sock = Socket { r, w };
            for _ in 0..3 {
                let m = sock.recv().unwrap();
                sock.send(&m).unwrap();
            }
        });
        let mut c = Socket::open(port, "/devtools/browser/x").unwrap();
        for m in ["short", &"x".repeat(300), &"y".repeat(70_000)] {
            c.send(m).unwrap();
            assert_eq!(c.recv().unwrap(), m);
        }
        server.join().unwrap();
    }
}
