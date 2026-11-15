//! `gp game share DIR`: a game with its own pictures (a new game, a copy
//! of a template such as Moss Hop: its template says `how = "build"` or
//! `"authored"`) made into a package and published into the person's own
//! registry on this computer, with the page that plays it by its name
//! (docs/web.md, "A game with its own pictures"):
//!
//! | Where | What |
//! |---|---|
//! | `$GP_HOME/work/share/registry` | The person's registry, a folder, unsigned, namespace `local` (made at the first share) |
//! | `$GP_HOME/work/share/page` | The page (scripts/web/build-page.sh's), its source the registry beside it (`"."`) |
//! | `$GP_HOME/work/share/stage/NAME` | The package as published: the game's folder, a package.toml and `play/` |
//!
//! Every theme the person made for the game (a large-art theme under
//! `$GP_HOME/work/themes/`, whose journey says it is of this game) goes in
//! the package's folder as `themes/ID/`, where gp-web-prep offers a
//! finished look, so the built `play/themes/` holds it and Settings offers
//! it as on any game. The link opens in the one `--look` names, else the
//! one changed last. Its files are in both folders the served check and
//! the publish's check read, so they are compared with every ROM too.
//!
//! The package is built by build-page.sh's road for such a game and
//! published by `gp registry publish`, which compares it with every ROM in
//! `$GP_ROM_DIR` first and refuses it if it holds anything of one. Each
//! share is a new version (0.1.N). gp serve serves the page and the
//! registry together at `/share/`, under the page's own headers, so the
//! link `/share/p?g=local/NAME` plays it on this computer; for anyone else
//! the two folders must be put online together. Nothing is sent anywhere.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use gp_evidence::Paths;
use serde_json::{Value, json};

use crate::Outcome;
use crate::registry::{self, Writing};

/// The namespace the person's own games are published under.
pub const NAMESPACE: &str = "local";

/// The folder the shares live in.
pub fn share_dir(home: &Path) -> PathBuf {
    home.join("work").join("share")
}

/// What one sentence says it takes for other people to open the link.
pub const FOR_OTHERS: &str = "This link plays your game on this computer only: for other people to open it, the page and your registry (the two folders below) have to be put online together, on a web host that sends the page's own headers (docs/web.md says how).";

/// A package name's last part from a copy's name: `my_hop` is `my-hop`.
fn slug(name: &str) -> String {
    let s: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let s = s.trim_matches('-').to_owned();
    if s.is_empty() { "my-game".into() } else { s }
}

/// A copy's name in words: `my_hop` is `My Hop`.
fn words(name: &str) -> String {
    name.split(['_', '-', ' '])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            c.next()
                .map(|f| f.to_uppercase().chain(c).collect::<String>())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn table(p: &Path) -> Option<toml::Table> {
    fs::read_to_string(p).ok()?.parse().ok()
}

/// Copies the game's folder into `to`, leaving out build output, git's
/// files and any ROM image.
fn copy_game(from: &Path, to: &Path) -> Result<()> {
    for e in fs::read_dir(from).with_context(|| format!("reading {}", from.display()))? {
        let e = e?;
        let (p, name) = (e.path(), e.file_name());
        if name == "target" || name == ".git" || name == "play" {
            continue;
        }
        let dst = to.join(&name);
        if e.file_type()?.is_dir() {
            fs::create_dir_all(&dst)?;
            copy_game(&p, &dst)?;
        } else if !crate::roms::is_rom_file(&p) {
            fs::copy(&p, &dst).with_context(|| format!("copying {}", p.display()))?;
        }
    }
    Ok(())
}

/// A fresh folder: what was there is taken away first.
fn fresh(dir: &Path) -> Result<()> {
    if dir.exists() {
        fs::remove_dir_all(dir).with_context(|| format!("clearing {}", dir.display()))?;
    }
    fs::create_dir_all(dir).with_context(|| format!("making {}", dir.display()))
}

/// Copies every file under `from` into `to`.
fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    for e in fs::read_dir(from).with_context(|| format!("reading {}", from.display()))? {
        let e = e?;
        let dst = to.join(e.file_name());
        if e.file_type()?.is_dir() {
            fs::create_dir_all(&dst)?;
            copy_tree(&e.path(), &dst)?;
        } else {
            fs::copy(e.path(), &dst)?;
        }
    }
    Ok(())
}

/// A theme made for the game: its look's id and the folder holding its
/// look.txt.
#[derive(Debug, Clone, PartialEq)]
struct Theme {
    name: String,
    id: String,
    look: PathBuf,
}

/// The themes the person made for the game in `dir` (their journeys say
/// they are of it), the one changed last first.
fn themes_of(dir: &Path, home: &Path) -> Vec<Theme> {
    let here = dir.canonicalize().unwrap_or_else(|_| dir.to_owned());
    let mut out: Vec<Theme> = Vec::new();
    for c in crate::journeys::list(home, None) {
        let of_this = c.kind == crate::journeys::Kind::Theme
            && c.from.as_ref().is_some_and(|f| {
                f.kind == "new"
                    && f.dir
                        .as_ref()
                        .is_some_and(|d| d.canonicalize().unwrap_or_else(|_| d.clone()) == here)
            });
        let Some(look) = of_this.then(|| crate::journeys::look_dir(&c.dir)).flatten() else {
            continue;
        };
        let id = fs::read_to_string(look.join(gp_present::looks::LOOK_FILE))
            .ok()
            .and_then(|t| {
                t.lines()
                    .find_map(|l| l.strip_prefix("id ").map(|x| x.trim().to_owned()))
            })
            .filter(|i| crate::journeys::is_name(i))
            .unwrap_or_else(|| c.name.clone());
        if out.iter().any(|t| t.id == id) {
            continue;
        }
        out.push(Theme {
            name: c.name,
            id,
            look,
        });
    }
    out
}

/// The look the link opens in: `--look` (a theme's name or id, or `own`),
/// else the theme changed last, else the game's own pictures.
fn opening<'a>(themes: &'a [Theme], look: Option<&str>) -> Result<Option<&'a Theme>> {
    match look {
        None => Ok(themes.first()),
        Some("own") => Ok(None),
        Some(l) => themes
            .iter()
            .find(|t| t.name == l || t.id == l)
            .map(Some)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "--look {l}: no theme of that name was made for this game (its themes: {}; or `own` for its own pictures)",
                    if themes.is_empty() {
                        "none".to_owned()
                    } else {
                        themes
                            .iter()
                            .map(|t| t.name.clone())
                            .collect::<Vec<_>>()
                            .join(", ")
                    }
                )
            }),
    }
}

/// Copies a theme's files (not hidden ones) into `to`, replacing what was
/// there.
fn copy_theme(from: &Path, to: &Path) -> Result<()> {
    fresh(to)?;
    for e in fs::read_dir(from).with_context(|| format!("reading {}", from.display()))? {
        let e = e?;
        if e.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let dst = to.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_theme(&e.path(), &dst)?;
        } else {
            fs::copy(e.path(), &dst)?;
        }
    }
    Ok(())
}

/// The look ids a built folder offers (themes/index.json).
fn offered(play: &Path) -> Vec<String> {
    fs::read_to_string(play.join("themes").join("index.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v["themes"].as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|t| t["id"].as_str().map(str::to_owned))
        .collect()
}

pub fn run(dir: &Path, title: Option<&str>, look: Option<&str>, paths: &Paths) -> Result<Outcome> {
    let dir = std::path::absolute(dir).with_context(|| format!("resolving {}", dir.display()))?;
    let Some(t) = table(&dir.join("template.toml")) else {
        bail!(
            "{} has no template.toml, so it isn't a game made from a template",
            dir.display()
        );
    };
    let how = t
        .get("source")
        .and_then(|s| s.get("how"))
        .and_then(toml::Value::as_str)
        .unwrap_or("-");
    if !matches!(how, "build" | "authored") {
        bail!(
            "{} plays from a ROM (its template says how = {how}), so it is shared as a port is, not from its own pictures: the page plays it from each player's own ROM",
            dir.display()
        );
    }
    let Some(repo) = crate::repo::find_repo(|d| d.join("scripts/web/build-page.sh").is_file())
    else {
        bail!("building the page needs the gameprinter checkout, which this gp can't find");
    };
    let copy = dir.file_name().map_or_else(
        || "my_game".to_owned(),
        |n| n.to_string_lossy().into_owned(),
    );
    let short = slug(&copy);
    let name = format!("{NAMESPACE}/{short}");
    let title = title.map_or_else(|| words(&copy), str::to_owned);
    let from = t
        .get("template")
        .and_then(|x| x.get("title"))
        .and_then(toml::Value::as_str)
        .unwrap_or("a template")
        .to_owned();

    let share = share_dir(&paths.home);
    let reg = share.join("registry");
    let spec = reg.display().to_string();
    if !reg.join("registry.toml").is_file() {
        registry::init_cmd(
            &spec,
            "my-registry",
            None,
            None,
            &[NAMESPACE.to_owned()],
            true,
        )?;
    }
    let versions = fs::read_dir(reg.join("index").join(NAMESPACE).join(&short))
        .map(|d| {
            d.flatten()
                .filter(|e| e.path().extension().is_some_and(|x| x == "toml"))
                .count()
        })
        .unwrap_or(0);
    let version = format!("0.1.{}", versions + 1);

    // The package: the game's folder and a package.toml naming it.
    let themes = themes_of(&dir, &paths.home);
    let opens = opening(&themes, look)?.cloned();
    let stage = share.join("stage").join(&short);
    fresh(&stage)?;
    copy_game(&dir, &stage)?;
    // The person's themes of it, beside the model, where gp-web-prep
    // offers a finished look.
    for t in &themes {
        copy_theme(&t.look, &stage.join("themes").join(&t.id))?;
    }
    let about = format!("{title} is a new game made with Game Printer, from the template {from}.");
    let mut package = toml::Table::new();
    for (k, v) in [
        ("name", &name),
        ("version", &version),
        ("kind", &"game".to_owned()),
        ("title", &title),
    ] {
        package.insert(k.into(), toml::Value::String(v.clone()));
    }
    let mut page_facts = toml::Table::new();
    page_facts.insert("about".into(), toml::Value::String(about));
    let mut pkg = toml::Table::new();
    pkg.insert("package".into(), toml::Value::Table(package));
    pkg.insert("page".into(), toml::Value::Table(page_facts));
    fs::write(stage.join("package.toml"), toml::to_string(&pkg)?)?;

    // The page and the package's play/ files, by build-page.sh's road for a
    // game with its own pictures; its served check runs on both.
    let built = share.join("build").join(&short);
    if let Some(parent) = built.parent() {
        fs::create_dir_all(parent)?;
    }
    let exe = std::env::current_exe().context("finding this gp")?;
    let o = Command::new("bash")
        .arg(repo.join("scripts/web/build-page.sh"))
        .arg(&stage)
        .arg(&built)
        .env("GP", &exe)
        .env("GP_WEB_REUSE", "1")
        .current_dir(&repo)
        .output()
        .context("running scripts/web/build-page.sh")?;
    let said = String::from_utf8_lossy(&o.stdout).into_owned();
    if !o.status.success() {
        let err = String::from_utf8_lossy(&o.stderr);
        let last: Vec<&str> = err.lines().chain(said.lines()).rev().take(4).collect();
        bail!(
            "scripts/web/build-page.sh exited {}: {} isn't shared ({})",
            o.status.code().unwrap_or(-1),
            title,
            last.into_iter().rev().collect::<Vec<_>>().join(" / ")
        );
    }
    let play = PathBuf::from(format!("{}.play", built.display()));
    let have = offered(&play);
    for t in &themes {
        if !have.contains(&t.id) {
            let err = String::from_utf8_lossy(&o.stderr);
            let why = err
                .lines()
                .find_map(|l| l.strip_prefix(&format!("gp-web-prep: the look {} ", t.id)))
                .unwrap_or("its theme/look.txt didn't read as a large-art look");
            bail!(
                "your theme {} isn't in the game built to share ({why}), so {title} isn't shared: gp theme look check says more",
                t.name
            );
        }
    }
    let stage_play = stage.join("play");
    fs::create_dir_all(&stage_play)?;
    copy_tree(&play, &stage_play)?;

    // Into the person's registry: gp registry publish compares it with
    // every ROM here first (registry::own_pictures_guard).
    let published = registry::publish_cmd(
        &spec,
        &stage,
        (&name, &version),
        ("game", &title),
        &[],
        None,
        Writing::default(),
    )?;
    let at = format!("{name}@{version}");
    registry::reindex_cmd(
        &spec,
        (std::slice::from_ref(&at), true, &[]),
        None,
        Writing::default(),
    )?;

    // The page, its source the registry it is served beside.
    let page = share.join("page");
    fresh(&page)?;
    copy_tree(&built, &page)?;
    let pj_path = page.join("page.json");
    let mut pj: Value = serde_json::from_str(&fs::read_to_string(&pj_path)?)?;
    pj["source"] = json!({"name": "my-registry", "url": "."});
    fs::write(&pj_path, serde_json::to_string_pretty(&pj)? + "\n")?;

    let query = match &opens {
        Some(t) => format!("p?g={name}&look={}", t.id),
        None => format!("p?g={name}"),
    };
    let link = format!("/share/{query}");
    let serve = format!(
        "gp registry serve {} --page {} --port 7800",
        reg.display(),
        page.display()
    );
    let looks = if themes.is_empty() {
        "its own pictures (no theme was made for it)".to_owned()
    } else {
        format!(
            "with {} ({}), offered in Settings; the link opens in {}",
            if themes.len() == 1 {
                "your theme".to_owned()
            } else {
                format!("your {} themes", themes.len())
            },
            themes
                .iter()
                .map(|t| t.name.clone())
                .collect::<Vec<_>>()
                .join(", "),
            opens
                .as_ref()
                .map_or_else(|| "its own pictures".to_owned(), |t| t.name.clone())
        )
    };
    let own = published.json["own_pictures"]
        .as_str()
        .unwrap_or("")
        .to_owned();
    let text = format!(
        "shared {title} as {at} in your registry {}, {looks}\n{own}\nplay it on this computer: gp serve's {link}, or {serve} and http://127.0.0.1:7800/{query}\n{FOR_OTHERS}\n  the page      {}\n  the registry  {}",
        reg.display(),
        page.display(),
        reg.display()
    );
    Ok(Outcome::ok(
        json!({
            "name": name,
            "version": version,
            "title": title,
            "registry": reg,
            "page": page,
            "package": stage,
            "link": link,
            "themes": themes.iter().map(|t| &t.id).collect::<Vec<_>>(),
            "look": opens.as_ref().map(|t| &t.id),
            "serve": serve,
            "for_others": FOR_OTHERS,
            "checked": own,
        }),
        text,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_copys_name_makes_a_package_name_and_a_title() {
        assert_eq!(slug("my_hop"), "my-hop");
        assert_eq!(slug("__"), "my-game");
        assert_eq!(words("my_hop"), "My Hop");
    }

    /// The themes made for a copy are found by their journeys, the one
    /// changed last first, by their look's id; a theme of another game is
    /// left out, and --look picks one by its name or id, or `own`.
    #[test]
    fn a_copys_own_themes_are_found_and_one_opens() {
        use crate::journeys::{Kind, copies_root, themes_root, write};
        let d = tempfile::tempdir().unwrap();
        let home = d.path();
        let copy = copies_root(home).join("my_hop");
        fs::create_dir_all(&copy).unwrap();
        fs::write(copy.join(gp_model::MODEL_FILE), "[model]\nname = \"x\"\n").unwrap();
        let other = copies_root(home).join("other_hop");
        fs::create_dir_all(&other).unwrap();
        fs::write(other.join(gp_model::MODEL_FILE), "[model]\nname = \"x\"\n").unwrap();
        let theme = |name: &str, id: Option<&str>, of: &str| {
            let t = themes_root(home).join(name).join("theme");
            fs::create_dir_all(t.join("art")).unwrap();
            let id_line = id.map_or_else(String::new, |i| format!("id {i}\n"));
            fs::write(
                t.join(gp_present::looks::LOOK_FILE),
                format!("gp-look 1\n{id_line}title {name}\nart art.txt\n"),
            )
            .unwrap();
            write(
                home,
                Kind::Theme,
                name,
                "journey.json",
                &json!({"journey": "theme", "name": name, "from": of, "from_kind": "new"}),
            )
            .unwrap();
        };
        theme("hop-night", Some("night"), "my_hop");
        theme("elsewhere", None, "other_hop");
        std::thread::sleep(std::time::Duration::from_millis(20));
        theme("hop-day", None, "my_hop");
        let found = themes_of(&copy, home);
        let names: Vec<(&str, &str)> = found
            .iter()
            .map(|t| (t.name.as_str(), t.id.as_str()))
            .collect();
        assert_eq!(names, [("hop-day", "hop-day"), ("hop-night", "night")]);
        assert_eq!(opening(&found, None).unwrap().unwrap().id, "hop-day");
        assert_eq!(
            opening(&found, Some("night")).unwrap().unwrap().name,
            "hop-night"
        );
        assert_eq!(
            opening(&found, Some("hop-night")).unwrap().unwrap().id,
            "night"
        );
        assert!(opening(&found, Some("own")).unwrap().is_none());
        let e = opening(&found, Some("elsewhere")).unwrap_err().to_string();
        assert!(e.contains("its themes: hop-day, hop-night"), "{e}");
        assert!(opening(&[], None).unwrap().is_none());
    }
}
