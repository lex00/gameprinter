//! `gp theme look play` on a game that plays from its own files: a
//! template of the repo's (Moss Hop, Badge In and the rest) or a copy of
//! one (`template_fork`). There is no ROM to load and nothing in a
//! registry: the game is built for the browser as `build_publish` builds
//! it (scripts/web/build-preview.sh, the interpreter preview), from a
//! scratch copy of its folder with each theme under `themes/`, where the
//! preview finds a large-art look, and served on 127.0.0.1 with the theme
//! chosen (`?theme=ID`).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use gp_evidence::Paths;
use serde_json::Value;

use super::{
    Built, GameFacts, Pictured, Play, PlayArgs, Served, Serving, ShotDef, Side, bind, checkout,
    frames_of, moments_for, serve_on, serve_sides, theme_files,
};
use crate::Outcome;

/// The game's assets the preview is built from: the copy's own
/// (`assets/` with its map), else the ones lifted or built for its source
/// in `$GP_HOME/work/assets/`, as `build_publish` finds them.
fn assets(game_dir: &Path, facts: &GameFacts, paths: &Paths) -> Result<PathBuf> {
    let own = game_dir.join("assets");
    if own.join(gp_assets::MAP_FILE).is_file() {
        return Ok(own);
    }
    let model: toml::Table = fs::read_to_string(game_dir.join(gp_model::MODEL_FILE))
        .with_context(|| format!("reading {}'s model", facts.title))?
        .parse()
        .with_context(|| format!("parsing {}'s model", facts.title))?;
    let source = model
        .get("model")
        .and_then(|m| m.get("source"))
        .and_then(|s| s.as_str())
        .unwrap_or(&facts.id)
        .to_owned();
    let dir = paths.home.join("work").join("assets").join(&source);
    if dir.join(gp_assets::MAP_FILE).is_file() {
        return Ok(dir);
    }
    let how = if game_dir.join(crate::template_build::AUTHORED).is_file() {
        format!("build them with gp template build {}", game_dir.display())
    } else {
        "lift them from its ROM with gp assets lift ROM".to_owned()
    };
    bail!(
        "{}'s pictures aren't on this computer yet ({} holds none): {how}",
        facts.title,
        dir.display()
    )
}

/// A scratch copy of the game's folder for the build, with each side's
/// theme as `themes/ID/`.
fn stage(game_dir: &Path, sides: &[Side], to: &Path) -> Result<()> {
    // The game's own theme of a side's id gives way to the one looked at.
    let ids: Vec<String> = sides
        .iter()
        .filter(|s| !matches!(s, Side::Original))
        .map(|s| format!("themes/{}/", s.look_id()))
        .collect();
    for (rel, bytes) in theme_files(game_dir)? {
        if rel.starts_with("target/") || ids.iter().any(|i| rel.starts_with(i.as_str())) {
            continue;
        }
        let p = to.join(&rel);
        if let Some(d) = p.parent() {
            fs::create_dir_all(d)?;
        }
        fs::write(p, bytes)?;
    }
    for s in sides {
        let Side::Folder(t) = s else { continue };
        let look = crate::journeys::look_dir(&t.root).unwrap_or_else(|| t.root.join("theme"));
        let dir = to.join("themes").join(s.look_id());
        for (rel, bytes) in theme_files(&look)? {
            let p = dir.join(&rel);
            if let Some(d) = p.parent() {
                fs::create_dir_all(d)?;
            }
            fs::write(p, bytes)?;
        }
    }
    Ok(())
}

/// The preview built into `out` from the staged folder.
fn build(repo: &Path, staged: &Path, assets: &Path, out: &Path) -> Result<()> {
    eprintln!(
        "building the game for the browser into {} (scripts/web/build-preview.sh; a few minutes the first time)",
        out.display()
    );
    let o = Command::new("bash")
        .arg(repo.join("scripts/web/build-preview.sh"))
        .arg(out)
        .arg(staged)
        .arg(assets)
        .current_dir(repo)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .context("running build-preview.sh")?;
    if !o.status.success() {
        let err = String::from_utf8_lossy(&o.stderr);
        let last: Vec<&str> = err.lines().rev().take(3).collect();
        bail!(
            "scripts/web/build-preview.sh exited {}: nothing to serve ({})",
            o.status.code().unwrap_or(-1),
            last.into_iter().rev().collect::<Vec<_>>().join(" / ")
        );
    }
    Ok(())
}

/// The look ids the built preview offers (themes/index.json).
fn offered(out: &Path) -> Vec<String> {
    fs::read_to_string(out.join("themes").join("index.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v["themes"].as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|t| t["id"].as_str().map(str::to_owned))
        .collect()
}

#[allow(clippy::too_many_arguments)]
pub(super) fn run(
    a: &PlayArgs,
    paths: &Paths,
    game_dir: &Path,
    facts: &GameFacts,
    sides: &[Side],
    tags: &[String],
    work: tempfile::TempDir,
    mut text: String,
    checked: (u64, u64),
) -> Result<Outcome> {
    for s in sides {
        if let Side::Published { name, version } = s {
            bail!(
                "--against {name}@{version}: {} plays from its own files here, not from a registry, so set a theme folder beside yours",
                facts.title
            );
        }
    }
    if sides.len() == 2 && sides[0].look_id() == sides[1].look_id() {
        bail!(
            "both themes are named {}, so the game couldn't offer them side by side: give one another name in its package.toml",
            sides[0].look_id()
        );
    }
    let Some(repo) = checkout() else {
        bail!(
            "building {} for the browser needs the gameprinter checkout, which this gp can't find",
            facts.title
        );
    };
    let assets = assets(game_dir, facts, paths)?;
    let staged = work.path().join("stage").join(&facts.id);
    stage(game_dir, sides, &staged)?;
    let out = work.path().join("play");
    build(&repo, &staged, &assets, &out)?;
    // The preview draws a finished large-art look as the public page's
    // lift puts it in (gp_web_lift::add_ready_looks); gp-web-prep offers
    // only the theme stacks beside the model.
    let (mut skipped, mut partial) = (Vec::new(), Vec::new());
    // Its own pictures, not a ROM's, and the whole model.
    gp_web_lift::add_ready_looks(
        &staged.join("themes"),
        &out,
        (false, true),
        &mut skipped,
        &mut partial,
    )
    .map_err(|e| anyhow::anyhow!("putting the themes in the game: {e}"))?;
    // Served under the public page's own headers (its policy, and the
    // isolation a game's module needs for its shared buffer), which the
    // server reads from `_headers` as it does a built page's.
    fs::write(out.join("_headers"), gp_mcp::page_headers::PUBLIC)?;
    let have = offered(&out);
    for s in sides.iter().filter(|s| !matches!(s, Side::Original)) {
        if !have.iter().any(|i| i == s.look_id()) {
            let why = skipped.iter().find(|k| k.id == s.look_id()).map_or_else(
                || "its theme/look.txt didn't read as a large-art look".to_owned(),
                |k| k.detail.clone(),
            );
            bail!(
                "the game built for the browser doesn't offer {}: {why} (gp theme look check says more)",
                s.look_id()
            );
        }
    }
    let _ = writeln!(
        text,
        "The game: {} from its own files ({}), its pictures from {}; no ROM to load",
        facts.title,
        game_dir.display(),
        assets.display()
    );
    // The game's own pictures: the build's own look, whatever look a visit
    // kept (preview.js, own-look).
    let link_of = |s: &Side| match s {
        Side::Original => "/index.html?files=here&own-look".to_owned(),
        _ => format!("/index.html?files=here&theme={}", s.look_id()),
    };

    // Pictures.
    if let Some(shots_out) = &a.shots {
        let roms = vec![(facts.id.clone(), String::new())];
        let (moments, from) = moments_for(a.moments.as_deref(), game_dir, &roms)?;
        let _ = writeln!(text, "The moments: {from}");
        fs::create_dir_all(out.join("runs"))?;
        for r in &moments.run {
            fs::write(
                out.join("runs").join(format!("{}.frames", r.name)),
                frames_of(&r.holds, r.frames)?,
            )?;
        }
        let mut origins = Vec::new();
        for s in sides {
            let l = bind(0)?;
            let sv = Served {
                label: s.words(),
                registry: out.clone(),
                overlay: out.clone(),
                port: l.local_addr()?.port(),
                link: link_of(s),
            };
            serve_on(l, &out, &sv);
            origins.push(sv);
        }
        // Every run: a game with no ROM plays each the same way.
        let list: Vec<&ShotDef> = moments.shot.iter().collect();
        let plays = [Play {
            id: facts.id.clone(),
            rom: None,
            shots: list,
        }];
        let pictured = Pictured {
            sides,
            tags,
            origins: &origins,
            work: work.path(),
            moments: &from,
        };
        let r = pictured.take(paths, a.browser.as_deref(), shots_out, &plays, text);
        if let Ok(o) = &r
            && o.exit == 0
        {
            crate::journeys::note_play(
                &paths.home,
                &a.dir,
                serde_json::json!({"shots": shots_out}),
            );
        }
        return r;
    }

    // Serving.
    let mut served = Vec::new();
    let mut listeners = Vec::new();
    for (i, s) in sides.iter().enumerate() {
        let l = bind(if i == 0 { a.port } else { 0 })?;
        let port = l.local_addr()?.port();
        served.push(Served {
            label: s.words(),
            registry: out.clone(),
            overlay: out.clone(),
            port,
            link: format!("http://127.0.0.1:{port}{}", link_of(s)),
        });
        listeners.push(l);
    }
    let built: Vec<Built> = (0..sides.len())
        .map(|i| Built {
            game: facts.title.clone(),
            game_version: "from its own files".into(),
            themes: offered_titles(&out, sides, i),
        })
        .collect();
    let words = format!("{} (its own files, no registry)", game_dir.display());
    let how = Serving {
        page: &out,
        built: &built,
        checked,
        source: &words,
        hint: "The game plays from its own files: there is no ROM to load. Your theme is chosen (Settings, Theme lists the others). Nothing was published.",
    };
    serve_sides(a, paths, work, &how, served, listeners, text)
}

/// The looks the build offers, side `first`'s first, then the other side
/// by package name and the game's own by id.
fn offered_titles(out: &Path, sides: &[Side], first: usize) -> Vec<String> {
    let mut order: Vec<&Side> = vec![&sides[first]];
    order.extend(
        sides
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != first)
            .map(|(_, s)| s),
    );
    let mut v: Vec<String> = order
        .iter()
        .map(|s| match s {
            Side::Original => "the game's own look".to_owned(),
            _ => format!("{}@{}", s.name(), s.version()),
        })
        .collect();
    let ours: BTreeMap<&str, ()> = sides.iter().map(|s| (s.look_id(), ())).collect();
    v.extend(
        offered(out)
            .into_iter()
            .filter(|i| !ours.contains_key(i.as_str()))
            .map(|i| format!("{i} (the game's own)")),
    );
    v
}
