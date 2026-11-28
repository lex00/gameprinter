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
//! link `/share/p?g=local/NAME` plays it on this computer. For anyone else,
//! `--registry DIR --namespace NS --source URL --page DIR` publishes into the
//! person's own registry folder (a clone of their GitHub repo, say) and
//! writes a page whose source is the https address that registry will be
//! read at, so the two put online on two hosts play `p?g=NS/NAME`
//! (docs/host-your-own.md, "A game with its own pictures"). Nothing is
//! committed, pushed or sent anywhere.

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
pub const FOR_OTHERS: &str = "This link plays your game on this computer only: for other people to open it, the page and your registry (the two folders below) have to be put online together, the page on a web host that sends its own headers: share it again with --source, the https address your registry will be read at, and the page reads the game from there (docs/host-your-own.md, \"A game with its own pictures\", says how).";

/// What one sentence says once the page reads the registry at an address.
pub const ONLINE: &str = "Once the registry is at that address and the page on a web host that sends its own headers, the link plays your game for anyone, with nothing installed: gp has put neither online (docs/host-your-own.md, \"A game with its own pictures\").";

/// Where a share goes, as `gp game share` is given it: left out, the
/// folders under `$GP_HOME/work/share`, the namespace `local`, and a page
/// that reads the registry beside it (`.`).
#[derive(Debug, Default, Clone)]
pub struct RoadArgs {
    /// The person's registry: a folder, written as a folder (a clone of
    /// their GitHub repo is committed and pushed by them, never by gp).
    pub registry: Option<PathBuf>,
    /// The page's folder.
    pub page: Option<PathBuf>,
    /// The address the page reads the registry at once online.
    pub source: Option<String>,
    /// The namespace the game is published under.
    pub namespace: Option<String>,
}

/// A share's places, every one known.
#[derive(Debug, Clone)]
struct Road {
    registry: PathBuf,
    page: PathBuf,
    source: String,
    namespace: String,
}

impl Road {
    fn of(a: &RoadArgs, home: &Path) -> Result<Road> {
        let share = share_dir(home);
        let source = a.source.clone().unwrap_or_else(|| ".".into());
        let source = source.trim_end_matches('/').to_owned();
        let local = |s: &str| {
            ["http://127.0.0.1:", "http://localhost:"]
                .iter()
                .any(|p| s.starts_with(p))
        };
        if source != "." && !source.starts_with("https://") && !local(&source) {
            bail!(
                "--source {source}: the page reads a registry on another address only over https (its policy allows no other), so give the https address the registry will be read at, such as https://raw.githubusercontent.com/OWNER/REPO/main"
            );
        }
        let namespace = a.namespace.clone().unwrap_or_else(|| NAMESPACE.into());
        if !crate::journeys::is_name(&namespace) {
            bail!("--namespace {namespace}: a namespace is lower-case letters, digits and -");
        }
        let abs = |p: &Option<PathBuf>, d: PathBuf| -> Result<PathBuf> {
            Ok(std::path::absolute(p.clone().unwrap_or(d))?)
        };
        Ok(Road {
            registry: abs(&a.registry, share.join("registry"))?,
            page: abs(&a.page, share.join("page"))?,
            source,
            namespace,
        })
    }
}

/// The registry's name (registry.toml's), for page.json's source.
fn registry_name(reg: &Path) -> String {
    table(&reg.join("registry.toml"))
        .and_then(|t| t.get("name")?.as_str().map(str::to_owned))
        .unwrap_or_else(|| "my-registry".into())
}

/// The namespaces granted in a folder registry.
fn granted(reg: &Path) -> Vec<String> {
    gp_package::registry::grants(&gp_package::registry::Folder(reg.to_owned()))
        .unwrap_or_default()
        .into_iter()
        .filter(|(_, g)| g.as_ref().is_ok_and(|g| g.status == "granted"))
        .map(|(n, _)| n)
        .collect()
}

/// The registry made or checked: one is made, unsigned, with the namespace
/// granted, where there is none; one that is there must grant it.
fn ready_registry(road: &Road) -> Result<()> {
    let reg = &road.registry;
    let spec = reg.display().to_string();
    if !reg.join("registry.toml").is_file() {
        registry::init_cmd(
            &spec,
            "my-registry",
            None,
            None,
            std::slice::from_ref(&road.namespace),
            true,
        )?;
    } else if !granted(reg).contains(&road.namespace) {
        bail!(
            "the registry {} doesn't grant the namespace {}, so a page would list the game and then fail to install it: gp registry grant {} {} first",
            reg.display(),
            road.namespace,
            spec,
            road.namespace
        );
    }
    Ok(())
}

/// The version this share gets: one more than the registry holds of it.
fn next_version(road: &Road, short: &str) -> String {
    let versions = fs::read_dir(
        road.registry
            .join("index")
            .join(&road.namespace)
            .join(short),
    )
    .map(|d| {
        d.flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "toml"))
            .count()
    })
    .unwrap_or(0);
    format!("0.1.{}", versions + 1)
}

/// The road's last part: the staged package (its `play/` built) into the
/// registry, listed, and the built page into the page's folder, its source
/// the registry at the road's address. Returns publish's own-pictures line.
fn publish_and_place(
    road: &Road,
    stage: &Path,
    built: &Path,
    (name, version, title): (&str, &str, &str),
    paths: &Paths,
) -> Result<String> {
    let spec = road.registry.display().to_string();
    // gp registry publish compares it with every ROM here first
    // (registry::own_pictures_guard).
    let published = registry::publish_with(
        paths,
        &spec,
        stage,
        (name, version),
        ("game", title),
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
    fresh(&road.page)?;
    copy_tree(built, &road.page)?;
    let pj_path = road.page.join("page.json");
    let mut pj: Value = serde_json::from_str(&fs::read_to_string(&pj_path)?)?;
    pj["source"] = json!({"name": registry_name(&road.registry), "url": road.source});
    fs::write(&pj_path, serde_json::to_string_pretty(&pj)? + "\n")?;
    Ok(published.json["own_pictures"]
        .as_str()
        .unwrap_or("")
        .to_owned())
}

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

pub fn run(
    dir: &Path,
    title: Option<&str>,
    look: Option<&str>,
    road: &RoadArgs,
    paths: &Paths,
) -> Result<Outcome> {
    let road = Road::of(road, &paths.home)?;
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
    let name = format!("{}/{short}", road.namespace);
    let title = title.map_or_else(|| words(&copy), str::to_owned);
    let from = t
        .get("template")
        .and_then(|x| x.get("title"))
        .and_then(toml::Value::as_str)
        .unwrap_or("a template")
        .to_owned();

    let share = share_dir(&paths.home);
    let reg = road.registry.clone();
    ready_registry(&road)?;
    let version = next_version(&road, &short);

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

    // Into the person's registry, and the page, its source the registry at
    // the road's address.
    let own = publish_and_place(&road, &stage, &built, (&name, &version, &title), paths)?;
    let at = format!("{name}@{version}");
    let page = road.page.clone();

    let query = match &opens {
        Some(t) => format!("p?g={name}&look={}", t.id),
        None => format!("p?g={name}"),
    };
    let at_home = road.source == "."
        && road.registry == share.join("registry")
        && road.page == share.join("page");
    let link = if at_home {
        format!("/share/{query}")
    } else {
        format!("YOUR_PAGE/{query}")
    };
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
    let (play, others) = if road.source == "." {
        (
            if at_home {
                format!(
                    "play it on this computer: gp serve's {link}, or {serve} and http://127.0.0.1:7800/{query}"
                )
            } else {
                format!("play it on this computer: {serve} and http://127.0.0.1:7800/{query}")
            },
            FOR_OTHERS,
        )
    } else {
        (
            format!(
                "the page reads your registry at {}: put the registry there and the page on its host, and {link} plays it, YOUR_PAGE being the page's address",
                road.source
            ),
            ONLINE,
        )
    };
    let text = format!(
        "shared {title} as {at} in your registry {}, {looks}\n{own}\n{play}\n{others}\n  the page      {}\n  the registry  {}",
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
            "source": road.source,
            "namespace": road.namespace,
            "for_others": others,
            "checked": own,
        }),
        text,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serves `root` on a free port of this machine with these headers,
    /// for as long as the test runs; its address.
    fn serve_folder(root: PathBuf, headers: &'static str) -> String {
        let l = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let at = format!("http://127.0.0.1:{}", l.local_addr().unwrap().port());
        std::thread::spawn(move || {
            let roots = [root];
            for s in l.incoming().flatten() {
                crate::registry::answer(s, &roots, headers);
            }
        });
        at
    }

    /// The share-link road for a game with its own pictures: a made-up
    /// game, as build-page.sh leaves it, published into a registry folder
    /// and its page written to another, both outside the maker's home;
    /// then, with the home gone, the page on one local address and the
    /// registry on a second, the page's page.json names the second, and
    /// the page's own install code there finds the game by the link's name
    /// with every file, its play/page.json read back by its hash.
    #[test]
    fn the_road_puts_a_game_where_a_page_on_another_address_finds_it() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("home");
        let paths = Paths {
            home: home.clone(),
            store: home.join("store"),
            roms: home.join("roms"),
        };
        fs::create_dir_all(&paths.roms).unwrap();
        let share = share_dir(&home);
        // The made-up game's package, its play/ built.
        let stage = share.join("stage").join("rooftop");
        fs::create_dir_all(stage.join("play")).unwrap();
        fs::write(
            stage.join("package.toml"),
            "[package]\nname = \"acme/rooftop\"\nversion = \"0.1.1\"\nkind = \"game\"\ntitle = \"Rooftop\"\n\n[page]\nabout = \"Rooftop is made up for this test.\"\n",
        )
        .unwrap();
        fs::write(
            stage.join("play").join("page.json"),
            r#"{"format":1,"title":"Rooftop","player":"own","rom":{"kind":"none","game":"rooftop"},"howto":[]}"#,
        )
        .unwrap();
        fs::write(stage.join("play").join("bundle.json"), "{}\n").unwrap();
        // The page build-page.sh built, reading the registry beside it.
        let built = share.join("build").join("rooftop");
        fs::create_dir_all(&built).unwrap();
        fs::write(built.join("index.html"), "<!doctype html>\n").unwrap();
        fs::write(built.join("page.js"), "// the page\n").unwrap();
        fs::write(
            built.join("page.json"),
            r#"{"format":1,"source":{"name":"my-registry","url":"."}}"#,
        )
        .unwrap();

        // The registry's address is known before it is served, as GitHub's
        // raw address is before the push.
        let out = d.path().join("out");
        let reg_listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let source = format!(
            "http://127.0.0.1:{}",
            reg_listener.local_addr().unwrap().port()
        );
        let road = Road::of(
            &RoadArgs {
                registry: Some(out.join("registry")),
                page: Some(out.join("page")),
                source: Some(format!("{source}/")),
                namespace: Some("acme".into()),
            },
            &home,
        )
        .unwrap();
        assert_eq!(
            road.source, source,
            "the address is kept without its last /"
        );
        ready_registry(&road).unwrap();
        let version = next_version(&road, "rooftop");
        assert_eq!(version, "0.1.1");
        let own = publish_and_place(
            &road,
            &stage,
            &built,
            ("acme/rooftop", &version, "Rooftop"),
            &paths,
        )
        .unwrap();
        assert!(own.contains("plays from its own pictures"), "{own}");
        assert_eq!(next_version(&road, "rooftop"), "0.1.2");

        // Nothing written out names the maker's home.
        let home_s = home.display().to_string();
        let mut stack = vec![out.clone()];
        while let Some(dir) = stack.pop() {
            for e in fs::read_dir(&dir).unwrap().flatten() {
                if e.path().is_dir() {
                    stack.push(e.path());
                } else {
                    let b = fs::read(e.path()).unwrap();
                    assert!(
                        !String::from_utf8_lossy(&b).contains(&home_s),
                        "{} names the home",
                        e.path().display()
                    );
                }
            }
        }
        fs::rename(&home, d.path().join("gone")).unwrap();

        // The registry on its address, sending CORS as GitHub's raw host
        // does; the page on another.
        let reg_root = vec![out.join("registry")];
        std::thread::spawn(move || {
            for s in reg_listener.incoming().flatten() {
                crate::registry::answer(s, &reg_root, "/*\n  Access-Control-Allow-Origin: *\n");
            }
        });
        let page_at = serve_folder(out.join("page"), "");
        assert_ne!(page_at, source);

        let page = registry::open(&page_at);
        let pj: Value = serde_json::from_slice(&page.read("page.json").unwrap().unwrap()).unwrap();
        assert_eq!(pj["source"]["url"], json!(source));
        assert_eq!(pj["source"]["name"], json!("my-registry"));
        assert!(page.read("index.html").unwrap().is_some());

        let from = registry::open(pj["source"]["url"].as_str().unwrap());
        let now = crate::registry::utc(0).unwrap();
        let got =
            gp_web_registry::install_on(&*from, None, ("acme/rooftop", None), None, &now).unwrap();
        assert_eq!(
            (got.kind.as_str(), got.version.as_str(), got.title.as_str()),
            ("game", "0.1.1", "Rooftop")
        );
        let facts = got
            .files
            .iter()
            .find(|f| f.path == "play/page.json")
            .expect("the package holds play/page.json");
        let bytes = from
            .read(&gp_package::blob_key(&facts.hash))
            .unwrap()
            .expect("the registry serves the file by its hash");
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["rom"]["kind"], json!("none"));
        // The page itself holds nothing of the game: it all comes from the
        // registry.
        assert!(
            page.read("index/acme/rooftop/0.1.1.toml")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn a_source_on_another_address_is_https_and_the_namespace_a_name() {
        let home = Path::new("/nowhere");
        let road = |source: &str, ns: &str| {
            Road::of(
                &RoadArgs {
                    source: Some(source.into()),
                    namespace: Some(ns.into()),
                    ..RoadArgs::default()
                },
                home,
            )
        };
        let e = road("http://example.com/reg", "acme")
            .unwrap_err()
            .to_string();
        assert!(e.contains("only over https"), "{e}");
        assert!(road("https://raw.githubusercontent.com/acme/r/main", "acme").is_ok());
        assert!(road(".", "acme").is_ok());
        assert!(road("https://x.example", "Not A Name").is_err());
        let r = Road::of(&RoadArgs::default(), home).unwrap();
        assert_eq!((r.source.as_str(), r.namespace.as_str()), (".", "local"));
        assert_eq!(r.registry, share_dir(home).join("registry"));
    }

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
