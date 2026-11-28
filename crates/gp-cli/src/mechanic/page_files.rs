//! What the page's files load (row older-page-leftovers-removed).
//!
//! With the older page gone (row older-mechanic-page-removed), the files
//! only it loaded are left under mechanic/ with nothing loading them. The
//! test here follows what the pages the servers serve load, from their
//! HTML through every script, import and style they name, and lists each
//! file under mechanic/ none of them reaches, and each route that still
//! serves a file or folder that is gone.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

/// The pages a server serves as a page: the screen page (gp mechanic start
/// and gp serve's /mechanic/GAME/), the first screen (gp serve's /start/),
/// the watch page (gp mechanic watch) and the refinement view (/refine).
const PAGES: [&str; 4] = ["screen.html", "start.html", "watch.html", "refine.html"];

/// The file kinds a page loads.
const KINDS: [&str; 4] = [".js", ".mjs", ".css", ".html"];

/// Every file under `dir`, but the node tests (`*.test.mjs`), which the
/// cargo tests load and no page does.
fn files(dir: &Path, out: &mut BTreeSet<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            files(&p, out);
        } else if !p.to_string_lossy().ends_with(".test.mjs") {
            out.insert(p);
        }
    }
}

/// Each quoted name of a file a text names (`"./fold.js"`,
/// `'themes/pane.css'`, `` `screen.html?game=` ``), its query cut off.
/// A quote is closed on its own line by the same quote, and what is
/// between is a path: letters, digits and `./_-`.
fn quoted_files(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let b = line.as_bytes();
        let mut i = 0;
        while i < b.len() {
            let q = b[i];
            if !matches!(q, b'"' | b'\'' | b'`') {
                i += 1;
                continue;
            }
            let Some(len) = b[i + 1..].iter().position(|&c| c == q) else {
                i += 1;
                continue;
            };
            let inner = &line[i + 1..i + 1 + len];
            let name = inner.split(['?', '#']).next().unwrap_or("");
            let path = !name.is_empty()
                && name
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"./_-".contains(&c))
                && KINDS.iter().any(|k| name.ends_with(k));
            if path {
                out.push(name.to_owned());
                i += len + 2;
            } else {
                i += 1;
            }
        }
    }
    out
}

/// `a/b/../c` as `a/c`.
fn tidy(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            c => out.push(c),
        }
    }
    out
}

/// The files under `page` a name in `from` can be: beside `from`, or from
/// the page's root as the browser takes a name relative to the page or
/// from the server's root (`/style.css`, gp serve's `/start/style.css`).
fn resolve(page: &Path, from: &Path, name: &str) -> Vec<PathBuf> {
    let beside = from.parent().unwrap_or(page).join(name);
    let rooted = name.trim_start_matches('/');
    let start = rooted.strip_prefix("start/").unwrap_or(rooted);
    [beside, page.join(rooted), page.join(start)]
        .iter()
        .map(|p| tidy(p))
        .filter(|p| p.starts_with(page) && p.is_file())
        .collect()
}

/// The files under `page` its pages load, from `PAGES` on.
fn loaded(page: &Path) -> BTreeSet<PathBuf> {
    let mut seen = BTreeSet::new();
    let mut todo: Vec<PathBuf> = PAGES.iter().map(|p| tidy(&page.join(p))).collect();
    todo.retain(|p| p.is_file());
    while let Some(f) = todo.pop() {
        if !seen.insert(f.clone()) {
            continue;
        }
        let Ok(text) = fs::read_to_string(&f) else {
            continue;
        };
        for name in quoted_files(&text) {
            for p in resolve(page, &f, &name) {
                if !seen.contains(&p) {
                    todo.push(p);
                }
            }
        }
    }
    seen
}

/// What is left of the older page's files and routes under `root` (a
/// checkout): each file under mechanic/ nothing loads, each route that
/// serves a mechanic file that isn't there, and each folder route whose
/// folder is gone.
fn leftovers(root: &Path) -> Vec<String> {
    let page = tidy(&root.join("mechanic"));
    let mut all = BTreeSet::new();
    files(&page, &mut all);
    let reached = loaded(&page);
    let mut left: Vec<String> = all
        .difference(&reached)
        .map(|f| {
            let at = f.strip_prefix(root).unwrap_or(f).display().to_string();
            format!("{at}: nothing loads it")
        })
        .collect();
    let read = |f: &str| fs::read_to_string(root.join(f)).unwrap_or_default();
    for src in [
        "crates/gp-cli/src/mechanic/server.rs",
        "crates/gp-cli/src/mechanic/watch_page.rs",
    ] {
        let text = read(src);
        for part in text.split("page(\"").skip(1) {
            let name = part.split('"').next().unwrap_or("");
            if KINDS.iter().any(|k| name.ends_with(k)) && !page.join(name).is_file() {
                left.push(format!("{src} serves {name}, which isn't in mechanic/"));
            }
        }
    }
    let journeys = read("crates/gp-cli/src/serve/journeys.rs");
    if let Some(list) = journeys
        .split("const FILES")
        .nth(1)
        .and_then(|s| s.split("];").next())
    {
        for name in quoted_files(list) {
            if !page.join(&name).is_file() {
                left.push(format!(
                    "serve/journeys.rs serves /start/{name}, which isn't in mechanic/"
                ));
            }
        }
    }
    for (src, folder, call) in [
        (
            "crates/gp-cli/src/mechanic/server.rs",
            "details",
            "s.page.join(\"details\")",
        ),
        (
            "crates/gp-cli/src/mechanic/server.rs",
            "refine",
            "s.page.join(\"refine\")",
        ),
        (
            "crates/gp-cli/src/theme_serve.rs",
            "themes",
            "at.page.join(\"themes\")",
        ),
    ] {
        if read(src).contains(call) && !page.join(folder).is_dir() {
            left.push(format!(
                "{src} serves files from mechanic/{folder}/, which is gone"
            ));
        }
    }
    for name in quoted_files(&read("crates/gp-cli/src/self_check.rs")) {
        if let Some(rest) = name.strip_prefix("mechanic/")
            && !page.join(rest).is_file()
        {
            left.push(format!(
                "self_check.rs wants {name}, which isn't in the checkout"
            ));
        }
    }
    left
}

/// The row's measure: no file under mechanic/ is left that nothing loads,
/// and no route serves one that is gone. A file added under mechanic/ that
/// no page loads, or a page file taken off while a route still serves it,
/// fails it.
#[test]
fn no_mechanic_file_is_left_unloaded() {
    let root = tidy(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let left = leftovers(&root);
    assert!(
        left.is_empty(),
        "files under mechanic/ nothing loads, or routes to what is gone:\n{}",
        left.join("\n")
    );
}

/// The walk on a made-up page: what a page names in its HTML, its
/// imports (beside the file or up a folder) and the styles a script adds
/// from the page's root are loaded; a file only a file nothing loads names
/// isn't, nor is one named only outside quotes; a node test isn't listed.
#[test]
fn the_walk_follows_what_the_pages_load() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path();
    let m = root.join("mechanic");
    for dir in ["parts", "old"] {
        fs::create_dir_all(m.join(dir)).unwrap();
    }
    let put = |f: &str, s: &str| fs::write(m.join(f), s).unwrap();
    put(
        "screen.html",
        "<link href=\"screen.css\"><script src=\"screen.js?v=2\"></script>",
    );
    put("screen.css", "body {}");
    put(
        "screen.js",
        "import { a } from './parts/a.js';\n// the older app.js's way\n",
    );
    put(
        "parts/a.js",
        "import '../card.js';\nlink.href = `parts/a.css`;\n",
    );
    put("parts/a.css", "");
    put("card.js", "");
    put("watch.html", "<script src=\"/watch.js\"></script>");
    put("watch.js", "");
    put("app.js", "import './old/pane.js';");
    put("old/pane.js", "");
    put("screen.test.mjs", "import './screen.js';");
    let crates = root.join("crates/gp-cli/src/mechanic");
    fs::create_dir_all(&crates).unwrap();
    fs::write(
        crates.join("server.rs"),
        "page(\"screen.html\", \"text/html\") page(\"index.html\", \"text/html\") s.page.join(\"details\")",
    )
    .unwrap();
    let left = leftovers(root);
    assert_eq!(
        left,
        [
            "mechanic/app.js: nothing loads it",
            "mechanic/old/pane.js: nothing loads it",
            "crates/gp-cli/src/mechanic/server.rs serves index.html, which isn't in mechanic/",
            "crates/gp-cli/src/mechanic/server.rs serves files from mechanic/details/, which is gone",
        ]
    );
}
