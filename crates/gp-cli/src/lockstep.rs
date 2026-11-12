//! A lockstep of the original in gp-nes and the model, with its report: both
//! games on the same input, measured apart on every mapped path, with the
//! splits grouped by the mechanic that owns each path (gp_interp::lockstep).
//! The model runs through a game's module, or the built-in interpreter for
//! a model with no hooks (`crate::hosted`).

use std::path::Path;

use anyhow::{Result, bail};
use serde_json::{Value as Json, json};

use gp_contract::{CONTRACT_FILE, Contract, Level, Tolerance};
use gp_evidence::manifest::PowerOn;

use crate::original::OriginalRuntime;
use gp_interp::lockstep::{self, Lockstep, Mode, Options};
use gp_interp::mapping::{Mapping, show};
use gp_model::path::Step;
use gp_runtime::LoadNote;
use gp_sim::Fx;

use crate::hosted::Source;

/// Each mapped path's tolerance and the contract level it's held to, from
/// the model directory's fidelity contract: a metric rule over the path's
/// part with a tolerance named for its field (`x = 2`, in the field's
/// units) gives that tolerance; anything else is exact. A free part is
/// still measured, and marked free.
pub(crate) fn tolerances(
    model_dir: &Path,
    model: &gp_model::Model,
    mapping: &Mapping,
) -> (Vec<Fx>, Vec<&'static str>) {
    let contract = std::fs::read_to_string(model_dir.join(CONTRACT_FILE))
        .ok()
        .and_then(|t| Contract::parse(&t).ok());
    let mut tol = Vec::new();
    let mut levels = Vec::new();
    for f in mapping.fields.iter().filter(|f| !f.presentation) {
        let Some(c) = &contract else {
            tol.push(Fx::ZERO);
            levels.push("exact");
            continue;
        };
        let Ok(p) = gp_model::Path::parse(&f.path) else {
            tol.push(Fx::ZERO);
            levels.push("exact");
            continue;
        };
        let fact: Vec<Step> = p
            .segments
            .iter()
            .map(|s| Step::Key(s.name.clone()))
            .collect();
        let field = p.segments.last().map(|s| s.name.as_str());
        let r = c.bind(model).resolve(&fact, field);
        let t = match (r.level, field.and_then(|k| r.tolerance.get(k))) {
            (Level::Metric, Some(Tolerance::Absolute(a))) => Fx::from_raw(a.raw()),
            _ => Fx::ZERO,
        };
        tol.push(t);
        levels.push(match r.level {
            Level::Free => "free",
            Level::Metric if t > Fx::ZERO => "metric",
            _ => "exact",
        });
    }
    (tol, levels)
}

fn fx_json(f: Fx) -> Json {
    // Floats only in presentation: the report is read by people and the page.
    json!(f.raw() as f64 / 65536.0)
}

pub(crate) fn notes_json(notes: &[LoadNote]) -> Json {
    json!(
        notes
            .iter()
            .map(|n| json!({"path": n.path, "how": n.how.name(), "why": n.why}))
            .collect::<Vec<_>>()
    )
}

/// [`crate::hosted::Source`] and the original in lockstep on `inputs`, going
/// on from a start snapshot when `resume` gives one. With `assets` (the
/// model's lifted assets folder), the two games' pictures are compared too;
/// if the model's pictures can't be drawn, the report says why and measures
/// the rest.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_resumed(
    source: &Source,
    model_dir: &Path,
    mapping: &Mapping,
    rom: &[u8],
    inputs: &[gp_runtime::InputFrame],
    mode: Mode,
    from: Option<u64>,
    values: bool,
    assets: Option<&Path>,
    draws: Option<gp_sim::DrawScript>,
    resume: Option<(&crate::clip::cache::Saved, lockstep::Prefix)>,
    keep: Option<crate::screen::Keep>,
) -> Result<(Lockstep, Json, Vec<String>)> {
    let model = source.parse_model()?;
    let mut original = match resume {
        Some((saved, _)) => OriginalRuntime::from_saved(rom, saved)?,
        None => OriginalRuntime::of_rom(rom)?,
    };
    // The hooks' APU writes, for the sound measure.
    let mut interp = source.open()?.with_apu_events()?;
    if let Some(mut d) = draws {
        d.base = from.unwrap_or(0);
        interp = interp.with_draws(d)?;
    }
    let spec = source.spec.clone();
    let mut no_screen = None;
    let mut probe = match assets.map(|a| crate::screen::art(&model, &spec, a)) {
        Some(Ok(art)) => {
            let mut p = crate::screen::Probe::new(art, crate::screen::nes_options());
            // Drawn from the frame before the start: the mapping's start
            // can come later, which only draws a few frames more.
            p.video_from = from.unwrap_or(0).saturating_sub(1);
            p.keep = keep;
            Some(p)
        }
        Some(Err(e)) => {
            no_screen = Some(format!("{e:#}"));
            None
        }
        None => None,
    };
    let (tolerance, levels) = tolerances(model_dir, &model, mapping);
    let opts = Options {
        mode,
        tolerance,
        from,
        start_state: None,
        values,
        until_start: false,
        sound: true,
        stop_at_split: false,
    };
    let screens = probe
        .as_mut()
        .map(|p| p as &mut dyn lockstep::Screens<_, _>);
    let ls = match resume {
        Some((_, at)) => lockstep::lockstep_resumed(
            &mut original,
            &mut interp,
            &spec,
            mapping,
            inputs,
            &opts,
            screens,
            at,
        ),
        None => lockstep::lockstep_with(
            &mut original,
            &mut interp,
            &spec,
            mapping,
            inputs,
            &opts,
            screens,
        ),
    }
    .map_err(anyhow::Error::msg)?;
    let catalog = gp_interp::mechanics::catalog(&model, &source.exts);
    let groups = lockstep::by_mechanic(&catalog, &ls.splits);
    let (mut j, mut lines) = report(&ls, &levels, &catalog, &groups);
    if let Some(sc) = &ls.screen {
        let from = ls.frames.first().map_or(0, |f| f.1);
        lines.extend(crate::screen::lines(sc, &ls.frames, from));
        j["screen"] = crate::screen::json(sc, &ls.frames);
    } else if let Some(e) = no_screen {
        lines.push(format!(
            "screen: not compared, the model's pictures can't be drawn: {e}"
        ));
        j["screen_error"] = Json::from(e);
    }
    if let Some((sj, sl)) = sound_report(&ls) {
        j["sound"] = sj;
        lines.extend(sl);
    }
    let (sj, sl) = stretches_report(&ls, mapping);
    j["stretches"] = sj;
    lines.extend(sl);
    if let Some(d) = interp.draws_report()? {
        let (line, desync) = match &d.desync {
            None => (
                format!(
                    "random draws: took {} of the script's {}, no desync",
                    d.taken, d.script
                ),
                Json::Null,
            ),
            Some((f, why)) => (
                format!(
                    "random draws: DESYNC at the original's frame {f}, after {} of {}: {why}",
                    d.taken, d.script
                ),
                json!({"original_frame": f, "why": why}),
            ),
        };
        lines.push(line);
        let all: Vec<Json> = d
            .desyncs
            .iter()
            .map(|(f, site, why)| json!({"original_frame": f, "site": site, "why": why}))
            .collect();
        j["draws"] =
            json!({"taken": d.taken, "script": d.script, "desync": desync, "desyncs": all});
    }
    Ok((ls, j, lines))
}

/// The random draws the original makes on `inputs` from power-on, at the
/// draw sites the mapping names, from the original's frame `from` on.
pub(crate) fn record_draws(
    rom: &[u8],
    mapping: &Mapping,
    inputs: &[gp_evidence::input_log::InputFrame],
    from: u64,
) -> Result<gp_sim::DrawScript> {
    if mapping.draws.is_empty() {
        bail!("the state mapping names no `[[draws]]` sites");
    }
    crate::original::nes_only(
        crate::original::Platform::of_file(rom),
        "a random draw (a mapping's `[[draws]]`)",
    )?;
    let nes = gp_nes::Nes::power_on(rom, PowerOn::default()).map_err(anyhow::Error::msg)?;
    record_draws_on(nes, mapping, inputs, from)
}

/// [`record_draws`] on a console already running: its frames are counted
/// on from the console's own count, so a console standing at frame `F`
/// gives frame `F + 1` to the first input.
pub(crate) fn record_draws_on(
    mut nes: gp_nes::Nes,
    mapping: &Mapping,
    inputs: &[gp_evidence::input_log::InputFrame],
    from: u64,
) -> Result<gp_sim::DrawScript> {
    if mapping.draws.is_empty() {
        bail!("the state mapping names no `[[draws]]` sites");
    }
    let first = nes.frame();
    let mut script = gp_sim::DrawScript {
        soft: mapping
            .draws
            .iter()
            .filter(|d| d.soft)
            .map(|d| d.site.clone())
            .collect(),
        ..gp_sim::DrawScript::default()
    };
    for (i, input) in inputs.iter().enumerate() {
        let frame = first + i as u64 + 1;
        let frame_in = *input;
        if frame <= from {
            nes.step(frame_in).map_err(|e| anyhow::anyhow!("{e}"))?;
            continue;
        }
        nes.step_with(frame_in, |bus| {
            let pc = bus.cpu.pc;
            if let Some(d) = mapping.draws.iter().find(|d| d.pc == pc) {
                // The byte the instruction reads: its operand's address
                // with any index, so `AND $19,X` reads each slot's byte.
                let at = gp_nes::footprint::operand_read(bus, pc).unwrap_or(d.ram);
                script.draws.push(gp_sim::draws::Draw {
                    frame,
                    site: d.site.clone(),
                    value: bus.peek(at),
                });
            }
        })
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    }
    Ok(script)
}

/// The sound measure (gp_sound::compare): the two games' APU writes on
/// each compared frame, when the model's hooks write any (a game's sound
/// driver kept as state, gp_sim::Ext::apu). None for a model that makes no
/// sound of its own.
pub(crate) fn sound_report(ls: &Lockstep) -> Option<(Json, Vec<String>)> {
    if !ls.sound.iter().any(|f| !f.1.is_empty()) {
        return None;
    }
    let a = gp_sound::compare::compare(gp_sound::apu::Region::Ntsc, &ls.sound);
    let at = |i: usize| ls.frames.get(i).copied().unwrap_or_default();
    let name = |i: usize| {
        let (m, o) = at(i);
        format!("model frame {m} (original frame {o})")
    };
    let lines = gp_sound::compare::lines(&a, &name);
    let first = |i: Option<usize>| {
        i.map(|i| {
            let (m, o) = at(i);
            json!({"frame": m, "original_frame": o})
        })
    };
    let j = json!({
        "frames": a.frames,
        "writes": a.writes,
        "channels": a.channels,
        "longest": a.longest,
        "by_channel": gp_sound::apu::Channel::ALL
            .iter()
            .zip(a.by_channel)
            .map(|(c, n)| (c.name().to_string(), json!(n)))
            .collect::<serde_json::Map<_, _>>(),
        "first_writes": first(a.first_writes.as_ref().map(|f| f.0)),
        "first_channel": first(a.first_channel.as_ref().map(|f| f.0)),
        // Each frame whose writes differ, with both sides' writes, so a
        // write the model makes that the original doesn't can be found
        // from the score alone (the first 100).
        "writes_apart": a
            .writes_apart
            .iter()
            .take(100)
            .map(|&i| {
                let (m, o) = at(i);
                let text = |w: &[(u8, u8)]| {
                    w.iter()
                        .map(|(r, v)| format!("40{r:02X}={v:02X}"))
                        .collect::<Vec<_>>()
                        .join(" ")
                };
                let (ow, mw) = ls.sound.get(i).cloned().unwrap_or_default();
                json!({"frame": m, "original_frame": o, "original": text(&ow), "model": text(&mw)})
            })
            .collect::<Vec<_>>(),
        // The original's frames whose channels differ, as runs.
        "apart": gp_sound::compare::apart_spans(&a)
            .iter()
            .map(|(x, y, chs)| json!({
                "from": at(*x).1,
                "to": at(*y).1,
                "channels": chs.iter().map(|c| c.name()).collect::<Vec<_>>(),
            }))
            .collect::<Vec<_>>(),
    });
    Some((j, lines))
}

/// A run's exact stretches and its splits by cause (docs/benchmarks.md,
/// "Long runs"), as JSON and lines for people.
pub(crate) fn stretches_report(ls: &Lockstep, mapping: &Mapping) -> (Json, Vec<String>) {
    use gp_interp::lockstep::{seconds, split_causes, stretches};
    let st = stretches(ls);
    let names: Vec<String> = mapping
        .stretches
        .as_ref()
        .map(|s| s.state.iter().map(|n| n.name.clone()).collect())
        .unwrap_or_default();
    let (by_state, by_measure) = split_causes(ls, &names);
    let pct = |a: u64, b: u64| (a * 1000).checked_div(b).unwrap_or(0);
    let j = json!({
        "frames": st.frames,
        "together": st.together,
        "together_permille": pct(st.together, st.frames),
        "play_frames": st.play,
        "split_events": st.events,
        "whole": st.whole,
        "open_end": st.open_end,
        "cut_by_scene": st.cut_by_scene,
        "shortest": st.whole.first().copied().unwrap_or(0),
        "p10": st.percentile(10),
        "low_tenth_mean": st.low_mean(),
        "median": st.median(),
        "longest": st.longest(),
        "longest_any": st.longest_any(),
        "at_least_5s": st.at_least(5),
        "at_least_10s": st.at_least(10),
        "at_least_30s": st.at_least(30),
        "play_from_mapping": mapping.stretches.as_ref().is_some_and(|s| s.play.is_some()),
        "splits_by_state": by_state.iter().map(|(k, n)| json!({"state": k, "splits": n})).collect::<Vec<_>>(),
        "splits_by_measure": by_measure.iter().map(|(k, n)| json!({"path": k, "splits": n})).collect::<Vec<_>>(),
    });
    let s = |f: u64| format!("{} s", seconds(f));
    let mut lines = vec![format!(
        "stretches ({} mode): {}/{} frames together ({}.{}%), {} play frames, {} split events",
        ls.mode.name(),
        st.together,
        st.frames,
        pct(st.together, st.frames) / 10,
        pct(st.together, st.frames) % 10,
        st.play,
        st.events
    )];
    if st.whole.is_empty() {
        lines.push(format!(
            "  no whole stretch; the run's end cut one of {}",
            s(st.open_end)
        ));
    } else {
        lines.push(format!(
            "  {} whole: shortest {}, 10th pct {}, mean of lowest tenth {}, median {}, longest {}; the end cut one of {}; >=5/10/30 s: {}/{}/{}",
            st.whole.len(),
            s(st.whole[0]),
            s(st.percentile(10)),
            s(st.low_mean()),
            s(st.median()),
            s(st.longest()),
            s(st.open_end),
            st.at_least(5),
            st.at_least(10),
            st.at_least(30)
        ));
    }
    if !ls.splits.is_empty() {
        let join = |v: &[(String, u64)]| {
            v.iter()
                .map(|(k, n)| format!("{k} {n}"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        if !names.is_empty() {
            lines.push(format!(
                "  split frames by the original's state: {}",
                join(&by_state)
            ));
        }
        lines.push(format!("  split frames by measure: {}", join(&by_measure)));
    }
    (j, lines)
}

fn report(
    ls: &Lockstep,
    levels: &[&'static str],
    catalog: &gp_interp::mechanics::Catalog,
    groups: &[(String, Vec<usize>, Vec<String>)],
) -> (Json, Vec<String>) {
    let n = ls.frames.len();
    let mut lines = vec![format!(
        "lockstep, {} mode: {n} frames measured, {} splits",
        ls.mode.name(),
        ls.splits.len()
    )];
    // How the frames pair, so no one pairs them by hand (a split's model
    // frame and the original's are both given below).
    let pairing = lockstep::pairing(&ls.frames);
    if let Some(p) = &pairing {
        lines.push(format!("pairing: {}", p.says()));
    }
    if let Some(f) = ls.scope_end {
        lines.push(format!(
            "out of scope from the original's frame {f}: the mapping's [scope] ends there, so the run stopped, {} frames left out",
            ls.out_of_scope
        ));
    }
    if let Some(s) = &ls.start {
        lines.push(format!(
            "started at the original's frame {} (model frame {}) from its state: {} loaded, {} approximated, {} missing",
            s.original_frame,
            s.frame,
            count(&s.notes, gp_runtime::LoadHow::Loaded),
            count(&s.notes, gp_runtime::LoadHow::Approximated),
            count(&s.notes, gp_runtime::LoadHow::Missing),
        ));
    }
    let width = ls.paths.iter().map(|p| p.path.len()).max().unwrap_or(0);
    let names: Vec<String> = ls.paths.iter().map(|p| p.path.clone()).collect();
    let tol: Vec<Fx> = ls.paths.iter().map(|p| p.tolerance).collect();
    let drift = lockstep::drift_thresholds(&names, &tol, &[]);
    let mut paths = Vec::new();
    for (i, p) in ls.paths.iter().enumerate() {
        let first = p.first_split.map(|at| ls.frames[at]);
        lines.push(format!(
            "{:width$}  within {} of {n} frames (tolerance {}{}), mean {}, max {}{}",
            p.path,
            p.within,
            p.tolerance,
            if levels.get(i) == Some(&"free") {
                ", free in the contract"
            } else {
                ""
            },
            p.mean(),
            p.max,
            match first {
                Some((m, o)) => format!("; first split at model frame {m} (original frame {o})"),
                None => String::new(),
            }
        ));
        paths.push(json!({
            "path": p.path,
            "level": levels.get(i).copied().unwrap_or("exact"),
            "tolerance": fx_json(p.tolerance),
            "within": p.within,
            "compared": n,
            "mean": fx_json(p.mean()),
            "max": fx_json(p.max),
            "first_split": first.map(|(m, o)| json!({"frame": m, "original_frame": o})),
            "owners": lockstep::owners(catalog, &p.path),
            "drift": fx_json(drift[i]),
            "distance": p.distance.iter().map(|d| fx_json(*d)).collect::<Vec<_>>(),
        }));
    }
    let mechanics: Vec<Json> = groups
        .iter()
        .map(|(m, splits, ps)| {
            let first = &ls.splits[splits[0]];
            json!({"mechanic": m, "splits": splits.len(), "paths": ps,
                   "first": {"frame": first.frame, "original_frame": first.original_frame}})
        })
        .collect();
    if !groups.is_empty() {
        lines.push("splits by mechanic:".into());
        for (m, splits, ps) in groups {
            let first = &ls.splits[splits[0]];
            lines.push(format!(
                "  {m}: {} splits, first at original frame {} ({})",
                splits.len(),
                first.original_frame,
                ps.join(", ")
            ));
        }
    }
    let splits: Vec<Json> = ls
        .splits
        .iter()
        .map(|s| {
            json!({"at": s.at, "frame": s.frame, "original_frame": s.original_frame,
                   "paths": s.paths.iter().map(|p| json!({"path": p.path, "original": show(&p.original),
                       "model": show(&p.model), "distance": fx_json(p.distance)})).collect::<Vec<_>>()})
        })
        .collect();
    let missing: Vec<&LoadNote> = ls
        .resync_notes
        .iter()
        .filter(|n| n.how == gp_runtime::LoadHow::Missing)
        .collect();
    if ls.mode == Mode::Resync && !missing.is_empty() {
        lines.push(format!(
            "a resync can't put back {} parts the mapping doesn't hold (gp interp translate lists them)",
            missing.len()
        ));
    }
    let j = json!({
        "mode": ls.mode.name(),
        "measured": n,
        "frames": ls.frames.iter().map(|(m, o)| json!([m, o])).collect::<Vec<_>>(),
        "paths": paths,
        "splits": splits,
        "mechanics": mechanics,
        "start": ls.start.as_ref().map(|s| json!({"frame": s.frame, "original_frame": s.original_frame,
            "notes": notes_json(&s.notes)})),
        "resync_notes": notes_json(&ls.resync_notes),
        "pairing": pairing,
        "scope": ls.scope_end.map(|f| json!({"original_frame": f, "frames": ls.out_of_scope})),
    });
    (j, lines)
}

fn count(notes: &[LoadNote], how: gp_runtime::LoadHow) -> usize {
    notes.iter().filter(|n| n.how == how).count()
}

#[cfg(test)]
mod arcade_tests {
    use std::fs;

    /// The seam end to end, with the owner's rbtapper romset: the skeleton
    /// `gp game new` writes for it (an arcade mapping with no fields yet)
    /// in lockstep with Tapper on gp-arcade, a coin and a start on the way.
    /// The model is a placeholder, so this shows only that the lockstep
    /// runs on an arcade original and lines its frames up; scoring waits
    /// for a model. Skipped without the romset.
    #[test]
    fn a_lockstep_runs_on_an_arcade_original() {
        let Some(zip) = gp_arcade::romset::local_romset("rbtapper") else {
            eprintln!("skipped: no rbtapper romset");
            return;
        };
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("home");
        let paths = gp_evidence::Paths {
            home: home.clone(),
            store: home.join("store"),
            roms: home.join("roms"),
        };
        fs::create_dir_all(paths.roms.join("arcade")).unwrap();
        let rom_path = paths.roms.join("arcade/rbtapper.zip");
        fs::copy(&zip, &rom_path).unwrap();
        let games = d.path().join("games");
        fs::create_dir_all(&games).unwrap();
        crate::game_new::run(
            &crate::game_new::New {
                game: "tapper",
                rom: &rom_path,
                genre: "other",
                title: None,
                namespace: Some("someone"),
                games: Some(&games),
                date: "2026-11-11",
            },
            &paths,
        )
        .unwrap();
        let dir = games.join("tapper");
        // No arcade assets are lifted yet: an empty lift, as a map with
        // no assets would give.
        let assets = d.path().join("assets");
        fs::create_dir_all(&assets).unwrap();
        fs::write(
            assets.join(gp_assets::MAP_FILE),
            format!(
                "schema = 1\nsource = \"tapper\"\nsha256 = \"{}\"\nasset = []\n",
                gp_arcade::romset::set("rbtapper").unwrap().identity()
            ),
        )
        .unwrap();
        let (source, _) =
            crate::hosted::Source::open_dir(&dir, Some(&assets), &paths, None).unwrap();
        let mapping =
            super::Mapping::parse(&fs::read_to_string(dir.join("mapping.toml")).unwrap()).unwrap();
        let rom = crate::clip_score::rom_of(&paths, &dir, None).unwrap();
        let inputs: Vec<gp_runtime::InputFrame> = (0..400u32)
            .map(|i| gp_runtime::InputFrame {
                buttons: match i {
                    100..106 => 1,
                    160..166 => 2,
                    _ => 0,
                },
                ..Default::default()
            })
            .collect();
        let (ls, _, lines) = super::run_resumed(
            &source,
            &dir,
            &mapping,
            &rom,
            &inputs,
            super::Mode::Free,
            None,
            false,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        eprintln!("{}", lines.join("\n"));
        assert!(ls.original_frames > 0, "{lines:?}");
        assert!(!ls.frames.is_empty(), "{lines:?}");
    }
}
