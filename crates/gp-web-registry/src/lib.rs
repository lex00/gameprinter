//! What the page asks of a registry, answered by the code `gp` uses.
//!
//! [`resolve`] runs over the files fetched so far. When it reaches a key it
//! doesn't have it answers `need`; the page fetches that key and asks again.
//! A key the source doesn't hold is passed as null. So no check is written
//! twice, and no check depends on the order the page fetched in.

use std::cell::RefCell;
use std::collections::BTreeMap;

use gp_package::Manifest;
use gp_package::registry::{ByGameLine, Reader, Recheck, Target, Yank};
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
    /// A recheck of this version that failed, from its by-game line: the
    /// page still offers the theme and says why the recheck failed. A
    /// recheck that passed isn't passed on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recheck: Option<Recheck>,
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

/// A theme the source lists for the game that the page leaves out of
/// Settings, Theme, with why: `reason` in words a player reads, written to
/// follow "because" (the page shows "Themes not offered: TITLE, because
/// REASON."), and `detail` for the theme's maker.
#[derive(Serialize, Debug, PartialEq, Clone)]
pub struct LeftOut {
    pub name: String,
    pub version: String,
    pub title: String,
    pub reason: String,
    pub detail: String,
}

/// What the page makes of a game's list: the themes it offers, in order,
/// and the ones it leaves out with why.
#[derive(Serialize, Debug, PartialEq, Default)]
pub struct Offer {
    pub offered: Vec<Offered>,
    pub left_out: Vec<LeftOut>,
}

impl Offer {
    /// Whether `name` is offered, or the reason it is left out (None when
    /// the game's list doesn't name it at all).
    pub fn verdict(&self, name: &str) -> Option<Result<&Offered, &LeftOut>> {
        if let Some(o) = self.offered.iter().find(|o| o.name == name) {
            return Some(Ok(o));
        }
        self.left_out.iter().find(|l| l.name == name).map(Err)
    }
}

/// The name a content package's theme takes on the page: the part of its
/// package name after the namespace (`lex00/turnips` is `turnips`). The
/// page keeps its files under that name and Settings, Theme lists one theme
/// of each name.
pub fn theme_id(name: &str) -> &str {
    name.split_once('/').map_or(name, |(_, n)| n)
}

/// Why a package name can't name a theme on the page, when it can't.
pub fn theme_id_problem(name: &str) -> Option<(String, String)> {
    let id = theme_id(name);
    if id == "original" {
        return Some((
            "its name is the one Settings, Theme gives the game's own art".into(),
            format!(
                "{name}: the page names a theme by the part of its package name after the namespace, and `original` is taken; publish it under another name"
            ),
        ));
    }
    if !gp_package::theme_id_ok(id) {
        return Some((
            "its name isn't one this page can list".into(),
            format!(
                "{name}: the page names a theme by the part of its package name after the namespace, and that part must start with a lowercase letter and hold only lowercase letters, digits, - and _; publish it under such a name"
            ),
        ));
    }
    None
}

/// The page's choice from a game's by-game list and the yanks: the latest
/// version of each package, leaving out one withdrawn for safety, one
/// whose name can't name a theme here ([`theme_id_problem`]), and all but
/// one of the packages whose names end the same (the one from the game's
/// own namespace, else the first by title). Offered in order by title.
pub fn choose(game: &str, lines: &[ByGameLine], yanks: &[Yank]) -> Offer {
    let mut out = Offer::default();
    let mut latest: Vec<&ByGameLine> = Vec::new();
    for l in lines {
        let newer = lines
            .iter()
            .any(|o| o.package == l.package && version_cmp(&o.version, &l.version).is_gt());
        if !newer && !latest.iter().any(|o| o.package == l.package) {
            latest.push(l);
        }
    }
    let left = |l: &ByGameLine, reason: String, detail: String| LeftOut {
        name: l.package.clone(),
        version: l.version.clone(),
        title: l.title.clone(),
        reason,
        detail,
    };
    let mut keep: Vec<&ByGameLine> = Vec::new();
    for l in latest {
        if let Some(y) = yanks
            .iter()
            .find(|y| y.id == l.id && y.severity == "security")
        {
            out.left_out.push(left(
                l,
                format!("it was withdrawn for safety: {}", y.reason),
                format!(
                    "{}@{} is in yanks.toml with severity security",
                    l.package, l.version
                ),
            ));
            continue;
        }
        if let Some((reason, detail)) = theme_id_problem(&l.package) {
            out.left_out.push(left(l, reason, detail));
            continue;
        }
        keep.push(l);
    }
    keep.sort_by(|a, b| a.title.cmp(&b.title).then(a.package.cmp(&b.package)));
    let game_ns = game.split_once('/').map_or(game, |(ns, _)| ns);
    let own = |l: &ByGameLine| {
        l.package
            .split_once('/')
            .is_some_and(|(ns, _)| ns == game_ns)
    };
    for l in &keep {
        let id = theme_id(&l.package);
        let same: Vec<&&ByGameLine> = keep.iter().filter(|o| theme_id(&o.package) == id).collect();
        let winner = same.iter().find(|o| own(o)).unwrap_or(&same[0]);
        if winner.package != l.package {
            out.left_out.push(left(
                l,
                format!(
                    "another theme, {}, has the same name here, and Settings, Theme lists one of each name",
                    winner.title
                ),
                format!(
                    "{} and {} both end in `{id}`; the page keeps {} ({}) and leaves this one out. Publish it under another name",
                    l.package,
                    winner.package,
                    winner.package,
                    if own(winner) { "the game's own namespace" } else { "first by title" }
                ),
            ));
            continue;
        }
        out.offered.push(Offered {
            name: l.package.clone(),
            version: l.version.clone(),
            id: l.id.clone(),
            title: l.title.clone(),
            recheck: l.recheck.clone().filter(Recheck::failed),
        });
    }
    out
}

/// What a source offers for the game ([`choose`] on its by-game list and
/// yanks), read through any [`Target`]: the page's question, and the one
/// `gp registry check --for` asks of a registry.
pub fn offer_on(
    t: &dyn Target,
    root: Option<&str>,
    game: &str,
    now: &str,
) -> Result<Offer, String> {
    let r = match root {
        Some(root) => Reader::open(t, root)?,
        None => Reader::unsigned(t)?,
    };
    let yanks = r.yanks(now)?;
    let lines = r.by_game(game, now)?.line;
    Ok(choose(game, &lines, &yanks.yank))
}

fn offered(ask: &Ask, t: &Held<'_>) -> Result<Offer, String> {
    let game = ask
        .for_game
        .as_deref()
        .ok_or("a list asks what's offered for a game: give for_game")?;
    offer_on(t, ask.root.as_deref(), game, &ask.now)
}

/// Installs a package as the page does ([`resolve`] for one package),
/// through any [`Target`]: what `gp registry check --for` asks of each
/// theme it offers, so a theme that is listed and then fails to install is
/// named with the page's own reason.
pub fn install_on(
    t: &dyn Target,
    root: Option<&str>,
    (name, version): (&str, Option<&str>),
    for_game: Option<&str>,
    now: &str,
) -> Result<Resolved, String> {
    let empty = BTreeMap::new();
    let ask = Ask {
        root: root.map(str::to_owned),
        name: name.to_owned(),
        version: version.map(str::to_owned),
        now: now.to_owned(),
        for_game: for_game.map(str::to_owned),
        files: empty,
        list: false,
    };
    run(&ask, t)
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

fn run(ask: &Ask, t: &dyn Target) -> Result<Resolved, String> {
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
            Ok(o) => json!({ "ok": o.offered, "left_out": o.left_out }).to_string(),
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

    /// Each theme the list names and the page doesn't offer comes back
    /// with its reason: withdrawn for safety, a name that can't name a
    /// theme here, and one of two themes whose names end the same (the
    /// game's own namespace keeps its theme, whatever the titles say).
    #[test]
    fn a_theme_left_out_of_the_list_is_named_with_why() {
        let lines = by_game(&[
            ("a/turnips", "1.0.0", "t1", "Turnips"),
            ("b/turnips", "2.0.0", "u2", "A Turnips copy"),
            ("b/8bit", "1.0.0", "e1", "Eight"),
            ("b/night.v2", "1.0.0", "n1", "Night"),
            ("b/original", "1.0.0", "o1", "Plain"),
            ("b/clay", "1.0.0", "c1", "Clay"),
            ("b/bad", "1.0.0", "b1", "Bad"),
        ]);
        let y = yanks(&[("b1", "security")]);
        let v = list(&[
            ("registry.toml", REGISTRY),
            ("yanks.toml", &y),
            ("by-game/a/game.toml", &lines),
        ]);
        let names = |k: &str| -> Vec<String> {
            v[k].as_array()
                .unwrap()
                .iter()
                .map(|o| o["name"].as_str().unwrap().to_owned())
                .collect()
        };
        assert_eq!(names("ok"), ["b/clay", "a/turnips"], "{v}");
        let why = |n: &str| -> String {
            v["left_out"]
                .as_array()
                .unwrap()
                .iter()
                .find(|l| l["name"] == n)
                .unwrap_or_else(|| panic!("{n} not left out: {v}"))["reason"]
                .as_str()
                .unwrap()
                .to_owned()
        };
        assert_eq!(why("b/bad"), "it was withdrawn for safety: r");
        assert_eq!(why("b/8bit"), "its name isn't one this page can list");
        assert_eq!(why("b/night.v2"), "its name isn't one this page can list");
        assert_eq!(
            why("b/original"),
            "its name is the one Settings, Theme gives the game's own art"
        );
        assert_eq!(
            why("b/turnips"),
            "another theme, Turnips, has the same name here, and Settings, Theme lists one of each name"
        );
        let detail = |n: &str| -> String {
            v["left_out"]
                .as_array()
                .unwrap()
                .iter()
                .find(|l| l["name"] == n)
                .unwrap()["detail"]
                .as_str()
                .unwrap()
                .to_owned()
        };
        assert!(
            detail("b/8bit").contains("start with a lowercase letter"),
            "{v}"
        );
        assert!(
            detail("b/turnips").contains("the game's own namespace"),
            "{v}"
        );
        // An empty list offers nothing and leaves nothing out.
        let o = choose("a/game", &[], &[]);
        assert!(o.offered.is_empty() && o.left_out.is_empty());
    }

    /// One package's verdict: offered, left out with why, or not listed.
    #[test]
    fn a_verdict_names_one_package() {
        let l = |p: &str, id: &str, t: &str| ByGameLine {
            package: p.into(),
            version: "1.0.0".into(),
            id: id.into(),
            title: t.into(),
            recheck: None,
        };
        let o = choose(
            "a/game",
            &[l("x/clay", "c", "Clay"), l("y/clay", "d", "Clay")],
            &[],
        );
        assert_eq!(o.verdict("x/clay").unwrap().unwrap().id, "c");
        let left = o.verdict("y/clay").unwrap().unwrap_err();
        assert!(left.detail.contains("first by title"), "{left:?}");
        assert!(o.verdict("z/none").is_none());
    }

    /// A theme whose recheck failed is still offered, with the recheck's
    /// date and reason; one that passed carries nothing.
    #[test]
    fn a_failed_recheck_comes_with_the_offer() {
        let lines = by_game(&[("a/clay", "2.0.0", "c2", "Clay"), ("a/moss", "1.0.0", "m1", "Moss")])
            .replace(
                "title = \"Clay\"\n",
                "title = \"Clay\"\n\n[line.recheck]\ndate = \"2026-11-14\"\nresult = \"fail\"\nreason = \"made up: the lift leaves it out\"\n",
            )
            .replace(
                "title = \"Moss\"\n",
                "title = \"Moss\"\n\n[line.recheck]\ndate = \"2026-11-14\"\nresult = \"pass\"\nreason = \"made up\"\n",
            );
        let v = list(&[
            ("registry.toml", REGISTRY),
            ("yanks.toml", &yanks(&[])),
            ("by-game/a/game.toml", &lines),
        ]);
        let ok = v["ok"].as_array().unwrap();
        assert_eq!(ok.len(), 2, "{v}");
        assert_eq!(ok[0]["name"], "a/clay");
        assert_eq!(ok[0]["recheck"]["result"], "fail");
        assert_eq!(ok[0]["recheck"]["date"], "2026-11-14");
        assert_eq!(
            ok[0]["recheck"]["reason"],
            "made up: the lift leaves it out"
        );
        assert!(ok[1].get("recheck").is_none(), "{v}");
    }

    #[test]
    fn a_list_needs_a_game() {
        let ask = json!({"name": "", "now": "2026-01-02T00:00:00Z", "list": true, "files": {}});
        let v: serde_json::Value = serde_json::from_str(&resolve(&ask.to_string())).unwrap();
        assert!(v["error"].as_str().unwrap().contains("for_game"));
    }

    /// A registry held in memory, for gp-package's writers.
    #[derive(Default)]
    struct Mem(RefCell<BTreeMap<String, Vec<u8>>>);

    impl Target for Mem {
        fn read(&self, key: &str) -> Result<Option<Vec<u8>>, String> {
            Ok(self.0.borrow().get(key).cloned())
        }
        fn write_once(&self, key: &str, bytes: &[u8]) -> Result<bool, String> {
            let mut m = self.0.borrow_mut();
            if m.contains_key(key) {
                return Ok(false);
            }
            m.insert(key.to_owned(), bytes.to_vec());
            Ok(true)
        }
        fn replace(&self, key: &str, bytes: &[u8]) -> Result<(), String> {
            self.0.borrow_mut().insert(key.to_owned(), bytes.to_vec());
            Ok(())
        }
    }

    /// What `gp registry yank` writes is what the page reads: a security
    /// yank leaves the version out of the list and stops its install; a
    /// notice yank keeps it offered, with the reason.
    #[test]
    fn a_yank_gp_writes_is_read_by_the_page() {
        use gp_package::registry::{Publish, Signer, init, publish, reindex, yank};
        let (t, none) = (Mem::default(), Signer::none());
        let (now, later) = ("2026-01-01T00:00:00Z", "2026-01-08T00:00:00Z");
        init(
            &t,
            "t",
            &none,
            &[],
            &[("a".into(), Vec::new())],
            "2026-01-01",
        )
        .unwrap();
        let games = ["a/game".to_owned()];
        for (name, title) in [("a/clay", "Clay"), ("a/night", "Night")] {
            let p = Publish {
                name,
                version: "1.0.0",
                kind: "content",
                title,
                for_games: &games,
                published: now,
            };
            let files = BTreeMap::from([("a.txt".to_owned(), b"made up\n".to_vec())]);
            publish(&t, &p, &files, &none).unwrap();
        }
        let v = [
            ("a/clay".into(), "1.0.0".into()),
            ("a/night".into(), "1.0.0".into()),
        ];
        reindex(&t, &v, &none, now, later).unwrap();
        yank(
            &t, "a/clay", "1.0.0", "unsafe", "security", &none, now, later,
        )
        .unwrap();
        yank(&t, "a/night", "1.0.0", "old", "notice", &none, now, later).unwrap();
        let files: Vec<(String, String)> =
            t.0.borrow()
                .iter()
                .map(|(k, v)| (k.clone(), String::from_utf8_lossy(v).into_owned()))
                .collect();
        let held: Vec<(&str, &str)> = files
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let names: Vec<String> = list(&held)["ok"]
            .as_array()
            .unwrap()
            .iter()
            .map(|o| o["name"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(names, ["a/night"]);
        let one = |name: &str| {
            let mut ask = json!({"name": name, "version": "1.0.0", "now": "2026-01-02T00:00:00Z", "files": {}});
            for (k, v) in &held {
                ask["files"][*k] = json!(v);
            }
            serde_json::from_str::<serde_json::Value>(&resolve(&ask.to_string())).unwrap()
        };
        assert_eq!(one("a/clay")["ok"]["yank"]["severity"], "security");
        assert_eq!(one("a/night")["ok"]["yank"]["reason"], "old");
    }

    #[test]
    fn versions_compare_by_their_numbers() {
        assert!(version_cmp("1.0.10", "1.0.9").is_gt());
        assert!(version_cmp("2.0.0", "10.0.0").is_lt());
        assert!(version_cmp("1.0.0", "1.0.0").is_eq());
    }
}
