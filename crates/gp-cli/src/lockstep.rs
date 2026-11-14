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
            let mut p =
                crate::screen::Probe::new(art, crate::screen::options_for(original.platform()));
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
    let (sj, sl) = stretches_report(&ls, mapping, model.model.frame_rate.milli());
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
    if crate::original::Platform::of_file(rom) == crate::original::Platform::Arcade {
        let set = gp_arcade::Romset::from_zip(rom)?;
        return record_draws_arcade(gp_arcade::Tapper::power_on(&set), mapping, inputs, from);
    }
    let nes = gp_nes::Nes::power_on(rom, PowerOn::default()).map_err(anyhow::Error::msg)?;
    record_draws_on(nes, mapping, inputs, from)
}

/// [`record_draws`] from a start the cache kept, on either platform.
pub(crate) fn record_draws_saved(
    rom: &[u8],
    saved: &crate::clip::cache::Saved,
    mapping: &Mapping,
    inputs: &[gp_evidence::input_log::InputFrame],
    from: u64,
) -> Result<gp_sim::DrawScript> {
    if crate::original::Platform::of_file(rom) == crate::original::Platform::Arcade {
        let set = gp_arcade::Romset::from_zip(rom)?;
        let mut t = gp_arcade::Tapper::power_on(&set);
        t.restore(&saved.snap).map_err(anyhow::Error::msg)?;
        return record_draws_arcade(t, mapping, inputs, from);
    }
    record_draws_on(saved.nes(rom, PowerOn::default())?, mapping, inputs, from)
}

/// The random draws of an arcade original (port wave 6), from gp-arcade's
/// instruction trace and access log on the main Z80. A Z80 game's random
/// routine returns its number in A (Tapper's `call $5fac`), so a site can
/// name either the call (what `gp interp draws --find` proposes) or the
/// instruction after it (where A holds the draw): either way the draw is A
/// when control comes back from the routine, which may be a frame later.
/// Any other site draws the first byte its instruction reads, as the NES
/// path takes its operand's byte (the mapping's `ram` when it reads none).
/// A draw is in the frame it completes in. Frames are counted on from the
/// machine's own count.
pub(crate) fn record_draws_arcade(
    mut t: gp_arcade::Tapper,
    mapping: &Mapping,
    inputs: &[gp_evidence::input_log::InputFrame],
    from: u64,
) -> Result<gp_sim::DrawScript> {
    use gp_arcade::dasm::{self, Flow};
    use gp_arcade::mcr::AccessKind;
    if mapping.draws.is_empty() {
        bail!("the state mapping names no `[[draws]]` sites");
    }
    let first = t.frame();
    let mut script = gp_sim::DrawScript {
        soft: mapping
            .draws
            .iter()
            .filter(|d| d.soft)
            .map(|d| d.site.clone())
            .collect(),
        ..gp_sim::DrawScript::default()
    };
    // Calls waiting for their routine to return: site, return address, SP.
    let mut pending: Vec<(String, u16, u16)> = Vec::new();
    // Sites just after a call: the call's address.
    let after_call: std::collections::BTreeMap<u16, u16> = mapping
        .draws
        .iter()
        .filter_map(|d| {
            let decode = |at: u16| {
                let bytes: Vec<u8> = (0..4).map(|k| t.peek(at.wrapping_add(k))).collect();
                dasm::decode(&bytes, at)
            };
            // A site that is itself a call draws by the call; otherwise a
            // `call nn` just before it makes it the place the number lands.
            let calls = |i: &dasm::Insn| matches!(i.flow, Flow::Call(_) | Flow::CallIf(_));
            let at = d.pc.wrapping_sub(3);
            let before = decode(at);
            (!calls(&decode(d.pc)) && before.len == 3 && calls(&before)).then_some((d.pc, at))
        })
        .collect();
    // The last main-CPU instruction run, across frames.
    let mut prev_pc: Option<u16> = None;
    for (i, input) in inputs.iter().enumerate() {
        let frame = first + i as u64 + 1;
        let bits = input.buttons_word();
        if bits > 0xff || input.reset {
            bail!("an arcade input frame holds bits 0 to 7 and no reset; this one holds {bits:#x}");
        }
        let c = gp_arcade::Controls::from_bits(bits);
        if frame <= from {
            t.step(c);
            continue;
        }
        t.set_trace(true);
        t.set_accesses(true);
        t.step(c);
        let mut trace: Vec<_> = t.take_trace().into_iter().filter(|e| e.cpu == 0).collect();
        trace.sort_by_key(|e| e.time);
        let acc: Vec<_> = t
            .take_accesses()
            .into_iter()
            .filter(|a| a.cpu == 0 && a.kind == AccessKind::Read)
            .collect();
        t.set_trace(false);
        t.set_accesses(false);
        let mut j = 0;
        for (n, e) in trace.iter().enumerate() {
            let end = trace.get(n + 1).map(|x| x.time);
            let from_j = j;
            while j < acc.len() && end.is_none_or(|end| acc[j].time < end) {
                j += 1;
            }
            if e.interrupt.is_some() {
                continue;
            }
            let came_from = prev_pc.replace(e.pc);
            if let Some(k) = pending.iter().position(|p| p.1 == e.pc && p.2 == e.sp) {
                let (site, _, _) = pending.remove(k);
                script.draws.push(gp_sim::draws::Draw {
                    frame,
                    site,
                    value: (e.af >> 8) as u8,
                });
            }
            let Some(d) = mapping.draws.iter().find(|d| d.pc == e.pc) else {
                continue;
            };
            if let Some(&call) = after_call.get(&e.pc) {
                // Back from the routine, not a conditional call that fell
                // through without calling.
                if came_from != Some(call) {
                    script.draws.push(gp_sim::draws::Draw {
                        frame,
                        site: d.site.clone(),
                        value: (e.af >> 8) as u8,
                    });
                }
                continue;
            }
            let insn = dasm::decode(&e.bytes, e.pc);
            match insn.flow {
                Flow::Call(_) | Flow::CallIf(_) | Flow::Rst(_) => {
                    let back = e.pc.wrapping_add(u16::from(insn.len));
                    // A call not taken comes straight back with SP as is.
                    pending.push((d.site.clone(), back, e.sp));
                }
                _ => {
                    let value = acc[from_j..j]
                        .first()
                        .map_or_else(|| t.peek(d.ram), |a| a.value);
                    script.draws.push(gp_sim::draws::Draw {
                        frame,
                        site: d.site.clone(),
                        value,
                    });
                }
            }
        }
    }
    Ok(script)
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
/// Seconds are the game's own, at `fps_milli` frames a second times 1000
/// (the model's `frame_rate`).
pub(crate) fn stretches_report(
    ls: &Lockstep,
    mapping: &Mapping,
    fps_milli: u64,
) -> (Json, Vec<String>) {
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
        "at_least_5s": st.at_least(5, fps_milli),
        "at_least_10s": st.at_least(10, fps_milli),
        "at_least_30s": st.at_least(30, fps_milli),
        "play_from_mapping": mapping.stretches.as_ref().is_some_and(|s| s.play.is_some()),
        "splits_by_state": by_state.iter().map(|(k, n)| json!({"state": k, "splits": n})).collect::<Vec<_>>(),
        "splits_by_measure": by_measure.iter().map(|(k, n)| json!({"path": k, "splits": n})).collect::<Vec<_>>(),
    });
    let s = |f: u64| format!("{} s", seconds(f, fps_milli));
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
            st.at_least(5, fps_milli),
            st.at_least(10, fps_milli),
            st.at_least(30, fps_milli)
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
        if gp_arcade::skip_slow_test() {
            return;
        }
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
                source: None,
                date: "2026-11-11",
                draws: true,
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

    /// A made-up generator routine at 0010 returns its number in A; the
    /// draw site is its call at 0003. Each draw is A at 0006, where the
    /// call comes back, and a site that reads memory draws the byte read.
    #[test]
    fn arcade_draws_are_what_the_routine_returns() {
        let main: &[u8] = &[
            // 0000: ld sp,e7f0 ; call 0010 ; and 01 ; jr z,+3 ; ld (e010),a ; jr 0003
            0x31, 0xf0, 0xe7, 0xcd, 0x10, 0x00, 0xe6, 0x01, 0x28, 0x03, 0x32, 0x10, 0xe0, 0x18,
            0xf4, 0, //
            // 0010: ld a,(e002) ; rlca ; jr nc,+2 ; xor 1c ; ld c,a ; ld a,r ; xor c
            0x3a, 0x02, 0xe0, 0x07, 0x30, 0x02, 0xee, 0x1c, 0x4f, 0xed, 0x5f, 0xa9,
            // 001c: ld (e002),a ; ret
            0x32, 0x02, 0xe0, 0xc9,
        ];
        let set = gp_arcade::testrom::romset(main, &[0x18, 0xfe]);
        let mapping = gp_interp::mapping::Mapping::parse(
            "schema = 1\nmodel = \"m\"\nsource = \"s\"\nalign = \"polled\"\n\n[[field]]\npath = \"entities.hero[0].x\"\nram = [0xE010]\n\n[[draws]]\nsite = \"pick\"\npc = 0x0003\nram = 0xE002\n\n[[draws]]\nsite = \"peek\"\npc = 0x0010\nram = 0xE002\n\n[[draws]]\nsite = \"back\"\npc = 0x0006\nram = 0xE002\n",
        )
        .unwrap();
        let inputs = vec![gp_evidence::input_log::InputFrame::default(); 3];
        let script = crate::lockstep::record_draws_arcade(
            gp_arcade::Tapper::power_on(&set),
            &mapping,
            &inputs,
            1,
        )
        .unwrap();
        // What A was at 0006 and what 0010 read, from the trace itself.
        let mut t = gp_arcade::Tapper::power_on(&set);
        t.step(gp_arcade::Controls::default());
        let mut want = Vec::new();
        // A call made before the first frame watched isn't a draw of it.
        let mut called = false;
        for f in 2..=3 {
            t.set_trace(true);
            t.step(gp_arcade::Controls::default());
            let mut tr: Vec<_> = t.take_trace().into_iter().filter(|e| e.cpu == 0).collect();
            tr.sort_by_key(|e| e.time);
            for e in &tr {
                if e.pc == 0x0003 {
                    called = true;
                }
                if e.pc == 0x0006 && called {
                    want.push((f, "pick".to_owned(), (e.af >> 8) as u8));
                }
                if e.pc == 0x0006 {
                    // The site after the call draws on every return.
                    want.push((f, "back".to_owned(), (e.af >> 8) as u8));
                }
                if e.pc == 0x0013 {
                    // After `ld a,(e002)` at 0010, A is the byte it read.
                    want.push((f, "peek".to_owned(), (e.af >> 8) as u8));
                }
            }
        }
        let mut have: Vec<_> = script
            .draws
            .iter()
            .map(|d| (d.frame, d.site.clone(), d.value))
            .collect();
        have.sort();
        want.sort();
        assert!(have.len() > 100, "{}", have.len());
        assert_eq!(have, want);
    }
}
