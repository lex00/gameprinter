//! The long-lived runtime session (docs/runtime-protocol.md, "Sessions"):
//! a recording stays open between queries, with its ROM, its checkpoints
//! restored and its consoles where the last query left them, so an agent's
//! loop of `gp query` calls doesn't open the recording and replay to the
//! frame every time.
//!
//! `gp mcp` and `gp serve` are long-lived, so they keep the sessions
//! ([`serve_in_background`]): a thread answers on a local socket. Each `gp
//! query` is a process of its own; it asks a running server first
//! ([`ask`]) and opens the recording itself when none answers, declines,
//! or the socket fails, with no error. `GP_SESSION=off` skips the ask.
//!
//! The answer is the same either way, byte for byte: both go through
//! [`crate::recordings::query_on`] and `query_frames_on`, and a session's
//! console for a frame is the cold path's own, the recording's nearest
//! checkpoint restored and replayed to the frame. A session keeps, per
//! checkpoint it has used, the console as restored and the one furthest
//! along; a query at or after the second continues it, an earlier one
//! starts again from the first. The checkpoint, the frames replayed and
//! every note are what the cold path gives, since the console followed the
//! same frames from the same checkpoint.
//!
//! A server answers only a gp binary that is itself (the same file, size and
//! time) and only for its own home, since the sealed-run guard is the home's;
//! the client's store and ROM folder are its own (`gp mcp` runs its children
//! on a store of its own). A `--rom` given, or a `clip:` play, goes the cold
//! way. A recording the sealed-run guard a new process would make refuses
//! (crate::sealed::FreshGuard, with the client's games checkout) is declined, and so is
//! a query that fails, so the cold path gives the refusal or the error.
//!
//! Limits ([`Limits`]): at most 4 recordings open, an estimated 256 MiB of
//! consoles, and a recording unused for 10 minutes is dropped.

use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead as _, BufReader, Write as _};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use gp_evidence::Paths;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Outcome;
use crate::open::Opened;
use crate::recordings::Seek;

/// How many recordings, and how many bytes of consoles, a server keeps.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Limits {
    pub recordings: usize,
    pub bytes: usize,
    pub idle: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            recordings: 4,
            bytes: 256 << 20,
            idle: Duration::from_secs(600),
        }
    }
}

/// The consoles kept for one checkpoint: as restored, and furthest along.
struct Slot<C> {
    base: C,
    head: C,
    used: u64,
}

/// One open recording.
struct Session {
    rom: Option<Vec<u8>>,
    set: Option<gp_arcade::Romset>,
    /// By the checkpoint the cold path would restore (`None`: power-on).
    nes: BTreeMap<Option<u64>, Slot<gp_nes::seek::Cursor>>,
    arcade: BTreeMap<Option<u64>, Slot<crate::arcade_rec::Cursor>>,
    /// The estimated size of one slot, once one is made.
    slot_bytes: usize,
    used: Instant,
}

impl Session {
    fn new() -> Self {
        Self {
            rom: None,
            set: None,
            nes: BTreeMap::new(),
            arcade: BTreeMap::new(),
            slot_bytes: 0,
            used: Instant::now(),
        }
    }

    fn slots(&self) -> usize {
        self.nes.len() + self.arcade.len()
    }

    fn bytes(&self) -> usize {
        self.slots() * self.slot_bytes
    }

    /// The least recently used slot's tick.
    fn oldest(&self) -> Option<u64> {
        self.nes
            .values()
            .map(|s| s.used)
            .chain(self.arcade.values().map(|s| s.used))
            .min()
    }

    fn drop_oldest(&mut self) {
        let Some(t) = self.oldest() else { return };
        self.nes.retain(|_, s| s.used != t);
        self.arcade.retain(|_, s| s.used != t);
    }
}

/// What a session is for: the store, the ROM folder and the recording.
type Key = (PathBuf, PathBuf, String);

/// Every open recording of a server.
pub(crate) struct Sessions {
    map: HashMap<Key, Session>,
    limits: Limits,
    tick: u64,
}

impl Sessions {
    pub fn new(limits: Limits) -> Self {
        Self {
            map: HashMap::new(),
            limits,
            tick: 0,
        }
    }

    /// Recordings open, and consoles kept.
    #[cfg(test)]
    pub fn counts(&self) -> (usize, usize) {
        (self.map.len(), self.map.values().map(Session::slots).sum())
    }

    /// Answers one query, as `gp query` would with no `--rom`.
    pub fn query(&mut self, paths: &Paths, q: &Query) -> Result<Outcome> {
        self.tick += 1;
        let opened = Opened::open(paths, &q.id)?;
        let key = (
            paths.store.clone(),
            paths.roms.clone(),
            opened.entry.id.to_hex(),
        );
        let session = self.map.entry(key.clone()).or_insert_with(Session::new);
        session.used = Instant::now();
        let mut warm = Warm {
            opened,
            session,
            paths,
            tick: self.tick,
        };
        let out = match &q.frames {
            Some(spec) => crate::recordings::query_frames_on(
                &mut warm,
                spec,
                &q.selectors,
                q.changed,
                q.replay,
            ),
            None => match q.frame {
                Some(frame) => {
                    crate::recordings::query_on(&mut warm, frame, &q.selectors, q.replay)
                }
                None => Err(anyhow::anyhow!("gp query needs --frame or --frames")),
            },
        };
        self.trim(&key);
        out
    }

    /// Drops idle recordings, then the least recently used consoles and
    /// recordings until the limits hold. `keep` is the recording just used.
    fn trim(&mut self, keep: &Key) {
        let idle = self.limits.idle;
        self.map.retain(|k, s| k == keep || s.used.elapsed() < idle);
        while self.map.len() > self.limits.recordings {
            let oldest = self
                .map
                .iter()
                .filter(|(k, _)| *k != keep)
                .min_by_key(|(_, s)| s.used)
                .map(|(k, _)| k.clone());
            match oldest {
                Some(k) => {
                    self.map.remove(&k);
                }
                None => break,
            }
        }
        while self.map.values().map(Session::bytes).sum::<usize>() > self.limits.bytes {
            let oldest = self
                .map
                .values_mut()
                .filter(|s| s.slots() > 0)
                .min_by_key(|s| s.oldest().unwrap_or(u64::MAX));
            match oldest {
                Some(s) => s.drop_oldest(),
                None => break,
            }
        }
    }

    /// Drops the recordings unused for longer than the idle limit.
    fn drop_idle(&mut self) {
        let idle = self.limits.idle;
        self.map.retain(|_, s| s.used.elapsed() < idle);
    }
}

/// A recording opened for one query, on its session.
struct Warm<'a> {
    opened: Opened,
    session: &'a mut Session,
    paths: &'a Paths,
    tick: u64,
}

impl Warm<'_> {
    fn rom(&mut self) -> Result<Vec<u8>> {
        if let Some(r) = &self.session.rom {
            return Ok(r.clone());
        }
        let bytes = self.opened.rom(None, self.paths)?.bytes;
        self.session.rom = Some(bytes.clone());
        Ok(bytes)
    }

    fn set(&mut self) -> Result<&gp_arcade::Romset> {
        if self.session.set.is_none() {
            self.session.set = Some(self.opened.romset(None, self.paths)?);
        }
        Ok(self.session.set.as_ref().expect("just set"))
    }

    /// The checkpoint the cold path restores for `frame` (`usable`: this
    /// runtime may restore the recording's checkpoints at all).
    fn checkpoint(&self, frame: u64, usable: bool) -> Option<u64> {
        self.opened
            .manifest
            .checkpoints
            .iter()
            .filter(|c| usable && c.frame <= frame)
            .map(|c| c.frame)
            .max()
    }
}

/// The estimated bytes of a slot: two consoles, each with its own input
/// log and frame hashes.
fn slot_bytes(state: usize, frames: usize, rom: usize) -> usize {
    let per = state
        + rom
        + frames
            * (std::mem::size_of::<gp_evidence::frame_hashes::FrameHash>()
                + std::mem::size_of::<gp_evidence::input_log::InputFrame>());
    2 * per
}

impl Seek for Warm<'_> {
    fn opened(&self) -> &Opened {
        &self.opened
    }

    fn check_given(&mut self) -> Result<()> {
        Ok(())
    }

    fn nes(&mut self, frame: u64) -> Result<gp_nes::seek::Cursor> {
        let rom = self.rom()?;
        if frame > self.opened.manifest.frames {
            // The cold path's own refusal.
            return Ok(gp_nes::seek::open(
                &self.opened.store,
                &self.opened.manifest,
                &rom,
                frame,
            )?);
        }
        // gp_nes::seek::open restores the nearest checkpoint whatever the
        // runtime, and falls back to power-on when it may not.
        let cp = self.checkpoint(frame, true);
        let tick = self.tick;
        if !self.session.nes.contains_key(&cp) {
            // Another slot's input log and frame hashes, which are the
            // recording's, so they're read once.
            let loaded = self
                .session
                .nes
                .values()
                .next()
                .map(|s| (s.base.input_log().clone(), s.base.hashes().to_vec()));
            let (store, manifest, at) =
                (&self.opened.store, &self.opened.manifest, cp.unwrap_or(0));
            let base = match loaded {
                Some((input, hashes)) => {
                    gp_nes::seek::open_with(store, manifest, &rom, at, input, hashes)?
                }
                None => gp_nes::seek::open(store, manifest, &rom, at)?,
            };
            if self.session.slot_bytes == 0 {
                let state = base.nes().snapshot().map(|s| s.len()).unwrap_or(0);
                self.session.slot_bytes = slot_bytes(state, base.hashes().len(), rom.len());
            }
            self.session.nes.insert(
                cp,
                Slot {
                    head: base.clone(),
                    base,
                    used: tick,
                },
            );
        }
        let slot = self.session.nes.get_mut(&cp).expect("just made");
        slot.used = tick;
        if slot.head.frame() > frame {
            slot.head = slot.base.clone();
        }
        slot.head.advance_to(frame)?;
        Ok(slot.head.clone())
    }

    fn arcade(&mut self, frame: u64) -> Result<crate::arcade_rec::Cursor> {
        let usable = crate::arcade_rec::made_by_this_runtime(&self.opened.manifest);
        let cp = self.checkpoint(frame, usable);
        let tick = self.tick;
        self.set()?;
        let set = self.session.set.as_ref().expect("loaded");
        if frame > self.opened.manifest.frames {
            return crate::arcade_rec::open(&self.opened.store, &self.opened.manifest, set, frame);
        }
        if !self.session.arcade.contains_key(&cp) {
            // Another slot's input log and frame hashes, which are the
            // recording's, so they're read once.
            let loaded = self
                .session
                .arcade
                .values()
                .next()
                .map(|s| (s.base.input_log().clone(), s.base.hashes().to_vec()));
            let (store, manifest, at) =
                (&self.opened.store, &self.opened.manifest, cp.unwrap_or(0));
            let base = match loaded {
                Some((input, hashes)) => {
                    crate::arcade_rec::open_with(store, manifest, set, at, input, hashes)?
                }
                None => crate::arcade_rec::open(store, manifest, set, at)?,
            };
            if self.session.slot_bytes == 0 {
                let state = base.tapper().snapshot().len();
                let rom = set.maincpu.len() + set.ssiocpu.len() + set.gfx1.len() + set.gfx2.len();
                self.session.slot_bytes = slot_bytes(state, base.hashes().len(), rom);
            }
            self.session.arcade.insert(
                cp,
                Slot {
                    head: base.clone(),
                    base,
                    used: tick,
                },
            );
        }
        let slot = self.session.arcade.get_mut(&cp).expect("just made");
        slot.used = tick;
        if slot.head.frame() > frame {
            slot.head = slot.base.clone();
        }
        slot.head.advance_to(frame)?;
        Ok(slot.head.clone())
    }
}

/// A `gp query` as the client sends it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Query {
    pub id: String,
    pub frame: Option<u64>,
    pub frames: Option<String>,
    pub selectors: Vec<String>,
    pub changed: bool,
    pub replay: bool,
}

/// The wire's version: a server answers only its own.
const VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct Request {
    v: u32,
    /// The client's binary ([`exe_identity`]).
    exe: String,
    home: PathBuf,
    store: PathBuf,
    roms: PathBuf,
    /// The games checkout the client's registry is read from.
    games: Option<PathBuf>,
    query: Query,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Reply {
    Outcome { json: Value, text: String, exit: u8 },
    Error(String),
    Declined(String),
}

/// This gp binary: its path, size and modification time. A server answers
/// only the same binary, so an answer is this build's.
fn exe_identity() -> Option<String> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    let meta = std::fs::metadata(&exe).ok()?;
    let mtime = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some(format!("{}:{}:{mtime}", exe.display(), meta.len()))
}

/// The folder the servers' sockets are in. A socket's path must be short
/// (104 bytes on macOS), so it is under the system's temporary folder, not
/// the home.
fn socket_dir() -> PathBuf {
    std::env::temp_dir().join("gp-sessions")
}

/// The start of the names of a home's sockets.
fn home_tag(home: &Path) -> String {
    blake3::hash(home.as_os_str().as_encoded_bytes()).to_hex()[..16].to_owned()
}

/// A running server's socket, removed when it is dropped.
pub(crate) struct Served {
    pub path: PathBuf,
}

impl Drop for Served {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// The end of a server's file in [`socket_dir`]: the socket itself on
/// unix; on Windows, where std has no local socket, a file holding the
/// loopback port the server listens on and the key a client must send.
#[cfg(unix)]
const SERVED_EXT: &str = ".sock";
#[cfg(not(unix))]
const SERVED_EXT: &str = ".port";

/// A connection to a server, by the path of its file in [`socket_dir`].
#[cfg(unix)]
fn connect(p: &Path) -> Option<std::os::unix::net::UnixStream> {
    std::os::unix::net::UnixStream::connect(p).ok()
}

/// On Windows: the port file's port, connected on 127.0.0.1, with its key
/// sent first. The file is in the user's own temporary folder, so only
/// this user's processes can read the key; a process of another user that
/// connects to the port without it is answered nothing.
#[cfg(not(unix))]
fn connect(p: &Path) -> Option<std::net::TcpStream> {
    let text = std::fs::read_to_string(p).ok()?;
    let (port, key) = text.trim().split_once(' ')?;
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port.parse().ok()?));
    let mut s = std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(2)).ok()?;
    s.write_all(format!("{key}\n").as_bytes()).ok()?;
    Some(s)
}

/// Starts answering queries on a local socket for `paths`' home, on a
/// thread of its own, until the process ends. `None` (and nothing served)
/// when the socket can't be made; the server works without it.
pub(crate) fn serve_in_background(paths: &Paths) -> Option<Served> {
    let dir = socket_dir();
    std::fs::create_dir_all(&dir).ok()?;
    let tag = home_tag(&paths.home);
    // Servers that are gone: nothing answers on their files.
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            let p = e.path();
            let ours = p
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(&tag) && n.ends_with(SERVED_EXT));
            if ours && connect(&p).is_none() {
                let _ = std::fs::remove_file(&p);
            }
        }
    }
    let path = dir.join(format!("{tag}-{}{SERVED_EXT}", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let incoming = listen(&path)?;
    let home = paths.home.clone();
    let exe = exe_identity()?;
    let sessions = Arc::new(Mutex::new(Sessions::new(Limits::default())));
    let guard: Arc<GuardSlot> = Arc::new(Mutex::new(None));
    let reaper = Arc::downgrade(&sessions);
    std::thread::spawn(move || {
        while let Some(s) = reaper.upgrade() {
            s.lock().unwrap_or_else(|e| e.into_inner()).drop_idle();
            drop(s);
            std::thread::sleep(Duration::from_secs(30));
        }
    });
    std::thread::spawn(move || {
        for stream in incoming {
            let sessions = sessions.clone();
            let guard = guard.clone();
            let home = home.clone();
            let exe = exe.clone();
            std::thread::spawn(move || answer(stream, &sessions, &guard, &home, &exe));
        }
    });
    Some(Served { path })
}

/// The connections to a server listening at `path`.
#[cfg(unix)]
fn listen(path: &Path) -> Option<impl Iterator<Item = Conn> + Send + 'static> {
    let listener = std::os::unix::net::UnixListener::bind(path).ok()?;
    Some(
        std::iter::repeat_with(move || listener.accept())
            .filter_map(|r| r.ok())
            .map(|(s, _)| Conn(s)),
    )
}

/// On Windows: a loopback port of the system's choosing, written with a
/// fresh key into `path`. A connection that doesn't send the key first is
/// closed unanswered.
#[cfg(not(unix))]
fn listen(path: &Path) -> Option<impl Iterator<Item = Conn> + Send + 'static> {
    use std::hash::{BuildHasher as _, Hasher as _};
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).ok()?;
    let port = listener.local_addr().ok()?.port();
    // RandomState is seeded from the system's random source.
    let word = || {
        std::collections::hash_map::RandomState::new()
            .build_hasher()
            .finish()
    };
    let key = format!("{:016x}{:016x}", word(), word());
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, format!("{port} {key}\n")).ok()?;
    std::fs::rename(&tmp, path).ok()?;
    let accepted = std::iter::repeat_with(move || listener.accept()).filter_map(|r| r.ok());
    Some(accepted.filter_map(move |(mut s, _)| {
        let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
        // A byte at a time up to the newline, so nothing of the request
        // that follows the key is read here.
        let mut line = Vec::new();
        let mut b = [0u8; 1];
        while line.len() <= key.len() {
            match std::io::Read::read(&mut s, &mut b) {
                Ok(1) if b[0] == b'\n' => break,
                Ok(1) => line.push(b[0]),
                _ => return None,
            }
        }
        let _ = s.set_read_timeout(None);
        (line == key.as_bytes()).then_some(Conn(s))
    }))
}

/// A connection a request comes on.
#[cfg(unix)]
struct Conn(std::os::unix::net::UnixStream);
#[cfg(not(unix))]
struct Conn(std::net::TcpStream);

/// Reads one request from `stream` and writes its reply.
fn answer(stream: Conn, sessions: &Mutex<Sessions>, guard: &GuardSlot, home: &Path, exe: &str) {
    let mut stream = stream.0;
    let mut line = String::new();
    let Ok(read) = stream.try_clone() else { return };
    if BufReader::new(read).read_line(&mut line).is_err() {
        return;
    }
    let reply = match serde_json::from_str::<Request>(&line) {
        Ok(r) => reply_to(r, sessions, guard, home, exe),
        Err(e) => Reply::Declined(format!("not a request: {e}")),
    };
    if let Ok(mut text) = serde_json::to_string(&reply) {
        text.push('\n');
        let _ = stream.write_all(text.as_bytes());
    }
}

/// The sealed-run guard a server asks before each answer.
type GuardSlot = Mutex<Option<crate::sealed::FreshGuard>>;

fn reply_to(
    r: Request,
    sessions: &Mutex<Sessions>,
    guard: &GuardSlot,
    home: &Path,
    exe: &str,
) -> Reply {
    if r.v != VERSION {
        return Reply::Declined(format!("wire version {}, not {VERSION}", r.v));
    }
    if r.exe != exe {
        return Reply::Declined("another gp binary".into());
    }
    if r.home != home {
        return Reply::Declined("another home".into());
    }
    let paths = Paths {
        home: r.home,
        store: r.store,
        roms: r.roms,
    };
    let refused = {
        let mut g = guard.lock().unwrap_or_else(|e| e.into_inner());
        if !g
            .as_ref()
            .is_some_and(|g| g.is_for(&paths.home, r.games.as_deref()))
        {
            *g = Some(crate::sealed::FreshGuard::new(&paths.home, r.games.clone()));
        }
        g.as_mut().expect("just made").refuses(&paths, &r.query.id)
    };
    if refused {
        return Reply::Declined("the sealed-run guard has something to say".into());
    }
    let mut s = sessions.lock().unwrap_or_else(|e| e.into_inner());
    match s.query(&paths, &r.query) {
        Ok(o) => Reply::Outcome {
            json: o.json,
            text: o.text,
            exit: o.exit,
        },
        // A failed query goes the cold way, which gives its own error:
        // this process's guard and ROM folder were read when it started.
        Err(e) => Reply::Declined(format!("{e:#}")),
    }
}

/// Asks a running server for `q`'s answer: `None` when none answers it, and
/// the caller opens the recording itself.
pub(crate) fn ask(paths: &Paths, q: &Query) -> Option<Result<Outcome>> {
    if std::env::var("GP_SESSION").is_ok_and(|v| v == "off") {
        return None;
    }
    let tag = home_tag(&paths.home);
    let mut socks: Vec<PathBuf> = std::fs::read_dir(socket_dir())
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(&tag) && n.ends_with(SERVED_EXT))
        })
        .collect();
    if socks.is_empty() {
        return None;
    }
    socks.sort();
    let request = Request {
        v: VERSION,
        exe: exe_identity()?,
        home: paths.home.clone(),
        store: paths.store.clone(),
        roms: paths.roms.clone(),
        games: crate::roms_cmd::games_dir(None),
        query: q.clone(),
    };
    let mut text = serde_json::to_string(&request).ok()?;
    text.push('\n');
    for sock in socks {
        let Some(mut stream) = connect(&sock) else {
            continue;
        };
        let _ = stream.set_read_timeout(Some(Duration::from_secs(600)));
        if stream.write_all(text.as_bytes()).is_err() {
            continue;
        }
        let mut line = String::new();
        if BufReader::new(&stream).read_line(&mut line).is_err() {
            continue;
        }
        match serde_json::from_str::<Reply>(&line) {
            Ok(Reply::Outcome { json, text, exit }) => {
                return Some(Ok(Outcome { json, text, exit }));
            }
            Ok(Reply::Error(e)) => return Some(Err(anyhow::Error::msg(e))),
            Ok(Reply::Declined(_)) | Err(_) => continue,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recordings::{Cold, query_frames_on, query_on};

    /// One query both ways: (frame or a range, selectors, --replay).
    struct Q {
        frame: Option<u64>,
        frames: Option<&'static str>,
        selectors: &'static [&'static str],
        replay: bool,
    }

    fn q(frame: u64, selectors: &'static [&'static str], replay: bool) -> Q {
        Q {
            frame: Some(frame),
            frames: None,
            selectors,
            replay,
        }
    }

    fn range(spec: &'static str, selectors: &'static [&'static str]) -> Q {
        Q {
            frame: None,
            frames: Some(spec),
            selectors,
            replay: true,
        }
    }

    impl Q {
        fn query(&self, id: &str) -> Query {
            Query {
                id: id.to_owned(),
                frame: self.frame,
                frames: self.frames.map(str::to_owned),
                selectors: self.selectors.iter().map(|s| (*s).to_owned()).collect(),
                changed: false,
                replay: self.replay,
            }
        }
    }

    /// What gp prints for an outcome or an error, with --json and without.
    fn printed(r: Result<Outcome>) -> String {
        match r {
            Ok(o) => format!(
                "{}\n{}\nexit {}",
                serde_json::to_string_pretty(&o.json).unwrap(),
                o.text,
                o.exit
            ),
            Err(e) => format!("gp: {e:#}"),
        }
    }

    fn on(s: &mut impl Seek, q: &Query) -> Result<Outcome> {
        match &q.frames {
            Some(spec) => query_frames_on(s, spec, &q.selectors, q.changed, q.replay),
            None => query_on(s, q.frame.unwrap(), &q.selectors, q.replay),
        }
    }

    /// The made-up NES ROM recorded with checkpoints every 10 frames, in a
    /// home of its own: the recording's ID.
    fn nes_home(home: &Path) -> String {
        std::fs::create_dir_all(home.join("roms")).unwrap();
        let rom = gp_nes::testrom::build();
        std::fs::write(home.join("roms/test.nes"), &rom).unwrap();
        let input = gp_evidence::input_log::generate(120, &["a@10-30", "right@40-"]).unwrap();
        let store = gp_evidence::Store::open(home.join("store")).unwrap();
        let image = gp_evidence::source::identify_nes(&rom).unwrap();
        let rec = gp_nes::recording::record_with(
            &store,
            gp_nes::recording::RecordRequest {
                rom: &rom,
                source: gp_evidence::manifest::SourceRef::NesRom {
                    sha256: image.sha256.clone(),
                    mapper: image.header.mapper,
                },
                source_id: None,
                power_on: Default::default(),
                input: &input,
                checkpoint_interval: 10,
                provenance: gp_evidence::manifest::Provenance::Scripted,
                name: "walk".into(),
                created_at: "2026-11-14T00:00:00Z".into(),
                notes: None,
            },
            gp_nes::recording::Capture::ALL,
        )
        .unwrap();
        rec.id.to_hex()
    }

    /// Scattered frames, back and forth inside one checkpoint's frames,
    /// past the end, from the trace and by replay, single frames and
    /// ranges: a session answers each byte for byte as the recording
    /// opened again would.
    #[test]
    fn a_session_answers_as_the_cold_path_on_nes() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let id = nes_home(home);
        let paths = crate::test_paths(home);
        let ram: &[&str] = &["ram[0x0000..0x0010]", "cpu.pc"];
        let queries = [
            q(57, ram, true),
            q(55, ram, true),
            q(59, ram, true),
            q(3, &["cpu", "oam[0..8]"], true),
            q(118, ram, true),
            q(60, &["bus[0xfffc]", "frame"], false),
            q(91, &["ram[0x0010]"], false),
            q(92, &["ram[0x0010]"], true),
            q(120, ram, true),
            q(121, ram, true),
            q(44, &["nothing[1]"], true),
            range("48..=63", &["ram[0x0000..0x0004]"]),
            range("50..53", &["bus[0xfffc]"]),
            q(51, ram, true),
            q(0, ram, true),
        ];
        let mut sessions = Sessions::new(Limits::default());
        for (n, one) in queries.iter().enumerate() {
            let query = one.query(&id[..12]);
            let cold =
                printed(Cold::open(&query.id, None, &paths).and_then(|mut c| on(&mut c, &query)));
            let warm = printed(sessions.query(&paths, &query));
            assert_eq!(warm, cold, "query {n}");
        }
        let (open, slots) = sessions.counts();
        assert_eq!(open, 1);
        assert!(slots > 3, "{slots} consoles kept");
    }

    /// An arcade recording's cold path, with the made-up set given rather
    /// than found in a ROM folder: what [`Cold`]'s own `arcade` does.
    struct ColdSet {
        opened: Opened,
        set: gp_arcade::Romset,
    }

    impl Seek for ColdSet {
        fn opened(&self) -> &Opened {
            &self.opened
        }
        fn check_given(&mut self) -> Result<()> {
            Ok(())
        }
        fn nes(&mut self, _: u64) -> Result<gp_nes::seek::Cursor> {
            unreachable!("an arcade recording")
        }
        fn arcade(&mut self, frame: u64) -> Result<crate::arcade_rec::Cursor> {
            crate::arcade_rec::open(&self.opened.store, &self.opened.manifest, &self.set, frame)
        }
    }

    #[test]
    fn a_session_answers_as_the_cold_path_on_arcade() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let paths = crate::test_paths(home);
        let store = gp_evidence::Store::open(&paths.store).unwrap();
        let set = gp_arcade::testrom::busy();
        let mut text = String::from("gp-arcade-input 0\nsystem test\ncolumns CSRLDUT\n");
        text.push_str("22 .......\n22 ......T\n1 C......\n");
        let input = gp_evidence::InputLog::parse(&text).unwrap();
        let rec = crate::arcade_rec::record(
            &store,
            crate::arcade_rec::Request {
                set: &set,
                source_id: None,
                input: &input,
                checkpoint_interval: 10,
                capture: gp_nes::recording::Capture::ALL,
                provenance: gp_evidence::manifest::Provenance::Scripted,
                name: "busy".into(),
                created_at: "2026-11-14T00:00:00Z".into(),
                notes: None,
            },
        )
        .unwrap();
        let id = rec.id.to_hex();
        let ram: &[&str] = &["ram[0xe000..0xe004]", "cpu.pc", "frame"];
        let queries = [
            q(33, ram, false),
            q(31, ram, false),
            q(38, ram, true),
            q(7, &["ram[0]"], false),
            q(45, ram, false),
            q(46, ram, false),
            q(12, &["nothing"], false),
            range("18..=24", &["ram[0xe000]"]),
            q(20, ram, false),
        ];
        let mut sessions = Sessions::new(Limits::default());
        let key = (paths.store.clone(), paths.roms.clone(), id.clone());
        let mut preloaded = Session::new();
        preloaded.set = Some(gp_arcade::testrom::busy());
        sessions.map.insert(key, preloaded);
        for (n, one) in queries.iter().enumerate() {
            let query = one.query(&id[..12]);
            let cold = printed(
                Opened::open(&paths, &query.id)
                    .map(|opened| ColdSet {
                        opened,
                        set: gp_arcade::testrom::busy(),
                    })
                    .and_then(|mut c| on(&mut c, &query)),
            );
            let warm = printed(sessions.query(&paths, &query));
            assert_eq!(warm, cold, "query {n}");
        }
    }

    /// The limits hold: one recording kept when one is allowed, and its
    /// consoles cut to what the bytes allow.
    #[test]
    fn the_limits_drop_the_least_recently_used() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let id = nes_home(home);
        let paths = crate::test_paths(home);
        let mut sessions = Sessions::new(Limits {
            recordings: 1,
            bytes: 1,
            idle: Duration::from_secs(600),
        });
        for f in [5, 25, 45, 65] {
            sessions
                .query(&paths, &q(f, &["ram[0]"], true).query(&id))
                .unwrap();
        }
        // Over the byte limit every console but none goes; the recording
        // stays open with its ROM.
        assert_eq!(sessions.counts(), (1, 0));
        sessions.limits.idle = Duration::ZERO;
        sessions.drop_idle();
        assert_eq!(sessions.counts(), (0, 0));
    }

    /// Over the socket: a server for the home answers what the cold path
    /// would, and a client for another home finds none and is told nothing.
    #[test]
    fn a_query_asks_the_server_for_its_home() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let id = nes_home(home);
        let paths = crate::test_paths(home);
        let served = serve_in_background(&paths).expect("a socket");
        let query = q(57, &["ram[0x0000..0x0010]", "cpu.pc"], true).query(&id[..12]);
        let cold =
            printed(Cold::open(&query.id, None, &paths).and_then(|mut c| on(&mut c, &query)));
        let asked = ask(&paths, &query).expect("the server answers");
        assert_eq!(printed(asked), cold);
        // A failing query is declined, and the caller opens it itself.
        let past = q(500, &["ram[0]"], true).query(&id[..12]);
        assert!(ask(&paths, &past).is_none());
        let other = tempfile::tempdir().unwrap();
        assert!(ask(&crate::test_paths(other.path()), &query).is_none());
        let path = served.path.clone();
        drop(served);
        assert!(!path.exists());
    }
}
