//! What the page asks of a registry, answered by the code `gp` uses.
//!
//! [`resolve`] runs over the files fetched so far. When it reaches a key it
//! doesn't have it answers `need`; the page fetches that key and asks again.
//! A key the source doesn't hold is passed as null. So no check is written
//! twice, and no check depends on the order the page fetched in.

use std::cell::RefCell;
use std::collections::BTreeMap;

use gp_package::Manifest;
use gp_package::registry::{Reader, Target};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Deserialize)]
pub struct Ask {
    /// The source's pinned root key.
    pub root: String,
    /// `namespace/name`.
    pub name: String,
    /// An exact version, or none for the latest in the catalog.
    pub version: Option<String>,
    /// UTC, `YYYY-MM-DDTHH:MM:SSZ`, for the signed files' expiries.
    pub now: String,
    /// The hook hashes this page's player was built with.
    #[serde(default)]
    pub hooks: Vec<String>,
    /// For content: the game it must be offered for.
    pub for_game: Option<String>,
    /// Every key fetched so far, null when the source doesn't hold it.
    #[serde(default)]
    pub files: BTreeMap<String, Option<String>>,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct FileLine {
    pub path: String,
    pub hash: String,
    pub size: u64,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct YankLine {
    pub reason: String,
    pub severity: String,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Resolved {
    pub id: String,
    pub name: String,
    pub version: String,
    pub kind: String,
    pub title: String,
    pub files: Vec<FileLine>,
    pub yank: Option<YankLine>,
}

struct Held<'a> {
    files: &'a BTreeMap<String, Option<String>>,
    need: RefCell<Option<String>>,
}

impl Target for Held<'_> {
    fn read(&self, key: &str) -> Result<Option<Vec<u8>>, String> {
        match self.files.get(key) {
            Some(v) => Ok(v.as_ref().map(|t| t.clone().into_bytes())),
            None => {
                *self.need.borrow_mut() = Some(key.to_owned());
                Err(format!("{key} hasn't been fetched"))
            }
        }
    }

    fn write_once(&self, _: &str, _: &[u8]) -> Result<bool, String> {
        Err("the page never writes to a source".into())
    }

    fn replace(&self, _: &str, _: &[u8]) -> Result<(), String> {
        Err("the page never writes to a source".into())
    }
}

fn run(ask: &Ask, t: &Held<'_>) -> Result<Resolved, String> {
    let r = Reader::open(t, &ask.root)?;
    let yanks = r.yanks(&ask.now)?;
    let version = match &ask.version {
        Some(v) => v.clone(),
        None => {
            let c = r.catalog(&ask.now)?;
            c.package
                .iter()
                .find(|p| p.name == ask.name)
                .map(|p| p.latest.clone())
                .ok_or_else(|| format!("{} isn't in this source's catalog", ask.name))?
        }
    };
    let v = r.version(&ask.name, &version)?;
    let m: Manifest = r.manifest(&ask.name, &version, &v.id)?;
    if let Some(game) = &ask.for_game {
        let offered = r.by_game(game, &ask.now)?;
        if !offered.line.iter().any(|l| l.id == v.id) {
            return Err(format!("{}@{version} isn't offered for {game}", ask.name));
        }
    }
    if let Some(h) = m.hook_hash()
        && !ask.hooks.contains(&h)
    {
        return Err(format!(
            "{}@{version} has hook code ({h}) this player wasn't built with",
            ask.name
        ));
    }
    let yank = yanks.yank.iter().find(|y| y.id == v.id).map(|y| YankLine {
        reason: y.reason.clone(),
        severity: y.severity.clone(),
    });
    Ok(Resolved {
        id: v.id.clone(),
        name: v.name.clone(),
        version: v.version.clone(),
        kind: v.kind.clone(),
        title: v.title.clone(),
        files: m
            .files
            .into_iter()
            .map(|f| FileLine {
                path: f.path,
                hash: f.hash,
                size: f.size,
            })
            .collect(),
        yank,
    })
}

/// One step: `{"need": KEY}`, `{"error": TEXT}` or `{"ok": RESOLVED}`.
pub fn resolve(ask_json: &str) -> String {
    let ask: Ask = match serde_json::from_str(ask_json) {
        Ok(a) => a,
        Err(e) => {
            return json!({ "error": format!("the page's question didn't read: {e}") }).to_string();
        }
    };
    let t = Held {
        files: &ask.files,
        need: RefCell::new(None),
    };
    match run(&ask, &t) {
        Ok(r) => json!({ "ok": r }).to_string(),
        Err(e) => match t.need.into_inner() {
            Some(key) => json!({ "need": key }).to_string(),
            None => json!({ "error": e }).to_string(),
        },
    }
}

thread_local! {
    static OUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

fn input(ptr: *mut u8, len: usize) -> Box<[u8]> {
    // SAFETY: `ptr` and `len` come from `gr_alloc`, and the caller hands the
    // buffer over.
    unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len)) }
}

#[unsafe(no_mangle)]
pub extern "C" fn gr_alloc(len: usize) -> *mut u8 {
    Box::into_raw(vec![0u8; len].into_boxed_slice()).cast()
}

#[unsafe(no_mangle)]
pub extern "C" fn gr_out_ptr() -> *const u8 {
    OUT.with(|o| o.borrow().as_ptr())
}

#[unsafe(no_mangle)]
pub extern "C" fn gr_out_len() -> usize {
    OUT.with(|o| o.borrow().len())
}

/// [`resolve`], its answer in the output buffer.
#[unsafe(no_mangle)]
pub extern "C" fn gr_resolve(ptr: *mut u8, len: usize) {
    let b = input(ptr, len);
    let out = resolve(&String::from_utf8_lossy(&b));
    OUT.with(|o| *o.borrow_mut() = out.into_bytes());
}

/// 1 when the bytes hash to the hash, else 0. Takes both buffers.
#[unsafe(no_mangle)]
pub extern "C" fn gr_blob_ok(hash: *mut u8, hash_len: usize, ptr: *mut u8, len: usize) -> i32 {
    let want = input(hash, hash_len);
    let bytes = input(ptr, len);
    i32::from(gp_package::check(&String::from_utf8_lossy(&want), &bytes).is_ok())
}
