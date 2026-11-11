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
    /// The source's pinned root key. With none, a source's signatures and
    /// expiries aren't looked at, and hashes still are.
    pub root: Option<String>,
    /// `namespace/name`.
    pub name: String,
    /// An exact version, or none for the latest in the catalog.
    pub version: Option<String>,
    /// UTC, `YYYY-MM-DDTHH:MM:SSZ`, for the signed files' expiries.
    pub now: String,
    /// For content: the game it must be offered for.
    pub for_game: Option<String>,
    /// Every key fetched so far, null when the source doesn't hold it.
    #[serde(default)]
    pub files: BTreeMap<String, Option<String>>,
    /// Instead of one package: list what the source offers for `for_game`
    /// (`name` is then ignored). The page offers each in its theme setting.
    #[serde(default)]
    pub list: bool,
}

/// One package a source offers for a game: the latest version of it the
/// game's list names.
#[derive(Serialize, Debug, PartialEq)]
pub struct Offered {
    pub name: String,
    pub version: String,
    pub id: String,
    pub title: String,
}

/// Two versions in order by their dotted numbers (1.0.10 after 1.0.9); a
/// part that isn't a number compares as text.
fn version_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let parts = |v: &str| -> Vec<(u64, String)> {
        v.split('.')
            .map(|p| (p.parse().unwrap_or(u64::MAX), p.to_owned()))
            .collect()
    };
    parts(a).cmp(&parts(b))
}

/// What the source offers for the game: the latest version of each package
/// its by-game list names, by name, leaving out one withdrawn for safety.
fn offered(ask: &Ask, t: &Held<'_>) -> Result<Vec<Offered>, String> {
    let game = ask
        .for_game
        .as_deref()
        .ok_or("a list asks what's offered for a game: give for_game")?;
    let r = match &ask.root {
        Some(root) => Reader::open(t, root)?,
        None => Reader::unsigned(t)?,
    };
    let yanks = r.yanks(&ask.now)?;
    let lines = r.by_game(game, &ask.now)?.line;
    let latest = |name: &str, version: &str| -> Result<bool, String> {
        let newer = lines
            .iter()
            .filter(|l| l.package == name)
            .any(|l| version_cmp(&l.version, version).is_gt());
        Ok(!newer)
    };
    let mut out: Vec<Offered> = Vec::new();
    for l in &lines {
        if !latest(&l.package, &l.version)? || out.iter().any(|o| o.name == l.package) {
            continue;
        }
        if yanks
            .yank
            .iter()
            .any(|y| y.id == l.id && y.severity == "security")
        {
            continue;
        }
        out.push(Offered {
            name: l.package.clone(),
            version: l.version.clone(),
            id: l.id.clone(),
            title: l.title.clone(),
        });
    }
    out.sort_by(|a, b| a.title.cmp(&b.title).then(a.name.cmp(&b.name)));
    Ok(out)
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
    let r = match &ask.root {
        Some(root) => Reader::open(t, root)?,
        None => Reader::unsigned(t)?,
    };
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
    if m.hook_hash().is_some() && !m.under("play").any(|(p, _)| p.ends_with("module.wasm")) {
        return Err(format!(
            "{}@{version} has hook code and no module, so this player can't run it",
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
    if ask.list {
        return match offered(&ask, &t) {
            Ok(l) => json!({ "ok": l }).to_string(),
            Err(e) => match t.need.into_inner() {
                Some(key) => json!({ "need": key }).to_string(),
                None => json!({ "error": e }).to_string(),
            },
        };
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    const REGISTRY: &str = "format = \"1\"\nname = \"t\"\nroot = \"\"\nsigning = []\nnamespaces = [\"a\"]\nmirrors = []\n";

    fn yanks(ids: &[(&str, &str)]) -> String {
        let mut s = "format = \"1\"\nregistry = \"t\"\nsigned = \"2026-01-01T00:00:00Z\"\nexpires = \"2026-01-08T00:00:00Z\"\n".to_owned();
        if ids.is_empty() {
            s.push_str("yank = []\n");
        }
        for (id, sev) in ids {
            s.push_str(&format!("\n[[yank]]\nname = \"a/x\"\nversion = \"1\"\nid = \"{id}\"\nreason = \"r\"\nseverity = \"{sev}\"\n"));
        }
        s
    }

    fn by_game(lines: &[(&str, &str, &str, &str)]) -> String {
        let mut s = "format = \"1\"\nregistry = \"t\"\nsigned = \"2026-01-01T00:00:00Z\"\nexpires = \"2026-01-08T00:00:00Z\"\n".to_owned();
        for (p, v, id, t) in lines {
            s.push_str(&format!(
                "\n[[line]]\npackage = \"{p}\"\nversion = \"{v}\"\nid = \"{id}\"\ntitle = \"{t}\"\n"
            ));
        }
        s
    }

    fn list(files: &[(&str, &str)]) -> serde_json::Value {
        let mut ask = json!({"name": "", "now": "2026-01-02T00:00:00Z", "for_game": "a/game", "list": true, "files": {}});
        for (k, v) in files {
            ask["files"][*k] = json!(v);
        }
        serde_json::from_str(&resolve(&ask.to_string())).unwrap()
    }

    #[test]
    fn a_list_is_the_latest_of_each_theme_offered_for_the_game() {
        let lines = by_game(&[
            ("a/turnips", "1.0.9", "t9", "Turnips"),
            ("a/turnips", "1.0.10", "t10", "Turnips"),
            ("a/clay", "2.0.0", "c2", "Clay"),
            ("a/bad", "1.0.0", "b1", "Bad"),
        ]);
        let y = yanks(&[("b1", "security")]);
        // It asks for each file it reads, one at a time.
        assert_eq!(list(&[])["need"], "registry.toml");
        assert_eq!(list(&[("registry.toml", REGISTRY)])["need"], "yanks.toml");
        let v = list(&[
            ("registry.toml", REGISTRY),
            ("yanks.toml", &y),
            ("by-game/a/game.toml", &lines),
        ]);
        let got: Vec<(String, String)> = v["ok"]
            .as_array()
            .unwrap()
            .iter()
            .map(|o| {
                (
                    o["name"].as_str().unwrap().to_owned(),
                    o["version"].as_str().unwrap().to_owned(),
                )
            })
            .collect();
        assert_eq!(
            got,
            [
                ("a/clay".into(), "2.0.0".into()),
                ("a/turnips".into(), "1.0.10".into())
            ]
        );
    }

    #[test]
    fn a_list_needs_a_game() {
        let ask = json!({"name": "", "now": "2026-01-02T00:00:00Z", "list": true, "files": {}});
        let v: serde_json::Value = serde_json::from_str(&resolve(&ask.to_string())).unwrap();
        assert!(v["error"].as_str().unwrap().contains("for_game"));
    }

    #[test]
    fn versions_compare_by_their_numbers() {
        assert!(version_cmp("1.0.10", "1.0.9").is_gt());
        assert!(version_cmp("2.0.0", "10.0.0").is_lt());
        assert!(version_cmp("1.0.0", "1.0.0").is_eq());
    }
}
