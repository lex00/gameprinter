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
    let mut original = match resume {
        Some((saved, _)) => OriginalRuntime::from_saved(rom, saved)?,
        None => OriginalRuntime::of_rom(rom)?,
    };
    run_resumed_on(
        &mut original,
        source,
        model_dir,
        mapping,
        rom,
        inputs,
        mode,
        from,
        values,
        assets,
        draws,
        resume.map(|(_, p)| p),
        keep,
        &[],
    )
}

/// [`run_resumed`] on an original the caller gives: the emulator, or the
/// clip's tape played back (`crate::tape::Original`). The original stands
/// at the start `resume` names, or at power-on without one.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_resumed_on<O: crate::screen::Shows>(
    original: &mut O,
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
    resume: Option<lockstep::Prefix>,
    keep: Option<crate::screen::Keep>,
    watch: &[String],
) -> Result<(Lockstep, Json, Vec<String>)> {
    let model = source.parse_model()?;
    // The hooks' APU writes and sound requests, for the sound measures,
    // and the original's latch writes (an arcade original; another has no
    // such switch and gives none).
    let mut interp = source.open()?.with_apu_events()?.with_sound_events();
    let _ = original.set("latch_events", gp_runtime::Value::Flag(true));
    // From the model's own power-on, a draws script starts at the first
    // compared frame: the run starts there with the model's power-on state
    // loaded as it stands, which starts the script with its base at the
    // original's frame there, so the model takes the original's draws from
    // its first frame. A script only starts on a load, so without this the
    // model drew from its own generator for the whole run.
    let power_on =
        (from.is_none() && draws.is_some()).then(|| gp_runtime::Runtime::canonical_state(&interp));
    if let Some(mut d) = draws {
        d.base = from.unwrap_or(0);
        interp = interp.with_draws(d)?;
    }
    let spec = source.spec.clone();
    let mut no_screen = None;
    let mut probe = match assets.map(|a| crate::screen::art(&model, &spec, a, mapping.board_units))
    {
        Some(Ok(art)) => {
            let mut p = crate::screen::Probe::new(
                art,
                crate::screen::options_for(crate::original::Platform::of_file(rom)),
            );
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
        from: if power_on.is_some() { Some(0) } else { from },
        start_state: power_on,
        values,
        until_start: false,
        sound: true,
        stop_at_split: false,
        watch: if values { watch.to_vec() } else { Vec::new() },
    };
    let screens = probe
        .as_mut()
        .map(|p| p as &mut dyn lockstep::Screens<_, _>);
    let ls = match resume {
        Some(at) => lockstep::lockstep_resumed(
            original,
            &mut interp,
            &spec,
            mapping,
            inputs,
            &opts,
            screens,
            at,
        ),
        None => lockstep::lockstep_with(
            original,
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
    if let Some((rj, rl)) = requests_report(&ls) {
        j["sound_requests"] = rj;
        lines.extend(rl);
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
        return record_draws_arcade(gp_arcade::Mcr::power_on(&set), mapping, inputs, from);
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
        let mut t = gp_arcade::Mcr::power_on(&set);
        t.restore(&saved.snap).map_err(anyhow::Error::msg)?;
        return record_draws_arcade(t, mapping, inputs, from);
    }
    record_draws_on(saved.nes(rom, PowerOn::default())?, mapping, inputs, from)
}

/// The random draws of an arcade original (port wave 6), recorded on the
/// main Z80 by gp-arcade's [`gp_arcade::draws::Recorder`], the one the
/// page's player records through too: a site may name the call of the
/// game's random routine or the instruction after it (the draw is A when
/// the routine returns, which may be a frame later), a site with `reg`
/// draws the register ([`gp_arcade::draws::z80_reg`]), and any other the first byte its
/// instruction reads. A draw is in the frame it completes in. Frames are
/// counted on from the machine's own count.
pub(crate) fn record_draws_arcade(
    mut t: gp_arcade::Mcr,
    mapping: &Mapping,
    inputs: &[gp_evidence::input_log::InputFrame],
    from: u64,
) -> Result<gp_sim::DrawScript> {
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
    let mut rec =
        gp_arcade::draws::Recorder::new(arcade_sites(mapping), &t).map_err(anyhow::Error::msg)?;
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
        rec.arm(&mut t);
        t.step(c);
        for (site, value) in rec.take(&mut t) {
            script
                .draws
                .push(gp_sim::draws::Draw { frame, site, value });
        }
    }
    Ok(script)
}

/// Where a forced draw at each of an arcade mapping's sites lands
/// ([`gp_arcade::mcr::Force`]): the instruction after the site's `call` of
/// the random routine (the site's own address when it is that instruction,
/// as [`record_draws_arcade`] reads it), or after an `ld a,r` or `ld a,i`
/// site, where A holds the number the caller tests. A site that reads its
/// byte from RAM has no such place and isn't listed. `peek` reads the main
/// CPU's code.
pub(crate) fn force_pcs(
    mapping: &Mapping,
    peek: impl Fn(u16) -> u8,
) -> std::collections::BTreeMap<String, u16> {
    use gp_arcade::dasm::{self, Flow};
    let decode = |at: u16| {
        let bytes: Vec<u8> = (0..4).map(|k| peek(at.wrapping_add(k))).collect();
        dasm::decode(&bytes, at)
    };
    let calls = |i: &dasm::Insn| matches!(i.flow, Flow::Call(_) | Flow::CallIf(_));
    let mut out = std::collections::BTreeMap::new();
    for d in &mapping.draws {
        let here = decode(d.pc);
        let to = match d.reg.as_deref() {
            Some("r" | "i") => {
                let (b0, b1) = (peek(d.pc), peek(d.pc.wrapping_add(1)));
                (b0 == 0xed && matches!(b1, 0x5f | 0x57)).then_some(d.pc.wrapping_add(2))
            }
            Some(_) => None,
            None if calls(&here) => Some(d.pc.wrapping_add(u16::from(here.len))),
            None => {
                let before = decode(d.pc.wrapping_sub(3));
                (before.len == 3 && calls(&before)).then_some(d.pc)
            }
        };
        if let Some(pc) = to {
            out.insert(d.site.clone(), pc);
        }
    }
    out
}

/// A mapping's draw sites as gp-arcade records them, and as the page's
/// player takes them (one [`gp_arcade::draws::Site::line`] each).
pub(crate) fn arcade_sites(mapping: &Mapping) -> Vec<gp_arcade::draws::Site> {
    mapping
        .draws
        .iter()
        .map(|d| gp_arcade::draws::Site {
            site: d.site.clone(),
            pc: d.pc,
            ram: d.ram,
            reg: d.reg.clone(),
            soft: d.soft,
        })
        .collect()
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
                let value = match d.reg.as_deref() {
                    // A register site: the register as the instruction
                    // starts (the mapping refuses any but these).
                    Some("x") => bus.cpu.x,
                    Some("y") => bus.cpu.y,
                    Some(_) => bus.cpu.acc,
                    // The byte the instruction reads: its operand's
                    // address with any index, so `AND $19,X` reads each
                    // slot's byte.
                    None => gp_nes::footprint::operand_read(bus, pc)
                        .or(d.ram)
                        .map_or(0, |at| bus.peek(at)),
                };
                script.draws.push(gp_sim::draws::Draw {
                    frame,
                    site: d.site.clone(),
                    value,
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

/// The sound request measure: each compared frame's sound requests, the
/// original's latch writes (an arcade original with `latch_events` on)
/// against the model's hooks' requests (gp_sim::Ext::sound_requests), the
/// same latches and bytes in the same order or not. None when neither side
/// made one.
pub(crate) fn requests_report(ls: &Lockstep) -> Option<(Json, Vec<String>)> {
    if !ls
        .requests
        .iter()
        .any(|f| !f.0.is_empty() || !f.1.is_empty())
    {
        return None;
    }
    let at = |i: usize| ls.frames.get(i).copied().unwrap_or_default();
    let text = |w: &[(u8, u8)]| {
        if w.is_empty() {
            return "none".to_string();
        }
        w.iter()
            .map(|(l, v)| format!("{l}:{v:02x}"))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let asked = ls
        .requests
        .iter()
        .filter(|f| !f.0.is_empty() || !f.1.is_empty())
        .count();
    let apart: Vec<usize> = (0..ls.requests.len())
        .filter(|&i| ls.requests[i].0 != ls.requests[i].1)
        .collect();
    let (original, model) = ls
        .requests
        .iter()
        .fold((0, 0), |(o, m), f| (o + f.0.len(), m + f.1.len()));
    let mut lines = vec![format!(
        "sound requests: {} of {} compared frames agree; {asked} frames with a request, the original {original} latch writes, the model {model}",
        ls.requests.len() - apart.len(),
        ls.requests.len(),
    )];
    for &i in apart.iter().take(10) {
        let (m, o) = at(i);
        let (ow, mw) = &ls.requests[i];
        lines.push(format!(
            "  differs at model frame {m} (original frame {o}): original {}, model {}",
            text(ow),
            text(mw)
        ));
    }
    if apart.len() > 10 {
        lines.push(format!("  and {} frames more", apart.len() - 10));
    }
    let j = json!({
        "frames": ls.requests.len(),
        "agree": ls.requests.len() - apart.len(),
        "frames_with_requests": asked,
        "original_writes": original,
        "model_writes": model,
        "apart": apart
            .iter()
            .take(100)
            .map(|&i| {
                let (m, o) = at(i);
                let (ow, mw) = &ls.requests[i];
                json!({"frame": m, "original_frame": o, "original": text(ow), "model": text(mw)})
            })
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

    /// The seam end to end, on a made-up game's zip
    /// (gp_arcade::testrom::made_up_game): the skeleton `gp game new`
    /// writes for it (an arcade mapping with no fields yet) in lockstep
    /// with the set on gp-arcade, a coin and a start on the way. The model
    /// is a placeholder, so this shows only that the lockstep runs on an
    /// arcade original and lines its frames up; scoring waits for a model.
    #[test]
    fn a_lockstep_runs_on_an_arcade_original() {
        let set_name = "madeuplock";
        let zip = gp_arcade::testrom::known_made_up_game(set_name);
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("home");
        let paths = gp_evidence::Paths {
            home: home.clone(),
            store: home.join("store"),
            roms: home.join("roms"),
        };
        fs::create_dir_all(paths.roms.join("arcade")).unwrap();
        let rom_path = paths.roms.join(format!("arcade/{set_name}.zip"));
        fs::write(&rom_path, &zip).unwrap();
        let games = d.path().join("games");
        fs::create_dir_all(&games).unwrap();
        crate::game_new::run(
            &crate::game_new::New {
                game: "made-up",
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
        let dir = games.join("made-up");
        // No arcade assets are lifted yet: an empty lift, as a map with
        // no assets would give.
        let assets = d.path().join("assets");
        fs::create_dir_all(&assets).unwrap();
        fs::write(
            assets.join(gp_assets::MAP_FILE),
            format!(
                "schema = 1\nsource = \"made-up\"\nsha256 = \"{}\"\nasset = []\n",
                gp_arcade::romset::set(set_name).unwrap().identity()
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
            gp_arcade::Mcr::power_on(&set),
            &mapping,
            &inputs,
            1,
        )
        .unwrap();
        // What A was at 0006 and what 0010 read, from the trace itself.
        let mut t = gp_arcade::Mcr::power_on(&set);
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

    /// Register sites (`reg`): a made-up loop reads R with `ld a,r` at
    /// 0009 and branches on its bit 0, as a game picks one of two ways
    /// (the two ways run 9 and 11 opcode fetches, so the bit changes).
    /// The draw at 0009 is the byte `ld a,r` loads, which is A at 000b
    /// where the trace sees it land; B and I are as the loop set them.
    #[test]
    fn arcade_register_sites_draw_what_the_instruction_reads() {
        let main: &[u8] = &[
            // 0000: ld sp,e7f0 ; ld a,5a ; ld i,a ; ld b,33
            0x31, 0xf0, 0xe7, 0x3e, 0x5a, 0xed, 0x47, 0x06, 0x33,
            // 0009: ld a,r ; bit 0,a ; jr z,+4 ; ld (e010),a ; nop
            0xed, 0x5f, 0xcb, 0x47, 0x28, 0x04, 0x32, 0x10, 0xe0, 0x00,
            // 0013: ld a,b ; ld a,i ; jr 0009
            0x78, 0xed, 0x57, 0x18, 0xf1,
        ];
        let set = gp_arcade::testrom::romset(main, &[0x18, 0xfe]);
        let mapping = gp_interp::mapping::Mapping::parse(
            "schema = 1\nmodel = \"m\"\nsource = \"s\"\nplatform = \"arcade\"\nalign = \"polled\"\n\n[[field]]\npath = \"entities.hero[0].x\"\nram = [0xE010]\n\n[[draws]]\nsite = \"twice\"\npc = 0x0009\nreg = \"r\"\n\n[[draws]]\nsite = \"held\"\npc = 0x000B\nreg = \"a\"\n\n[[draws]]\nsite = \"b\"\npc = 0x0013\nreg = \"b\"\n\n[[draws]]\nsite = \"vec\"\npc = 0x0014\nreg = \"i\"\n",
        )
        .unwrap();
        let inputs = vec![gp_evidence::input_log::InputFrame::default(); 3];
        let script = crate::lockstep::record_draws_arcade(
            gp_arcade::Mcr::power_on(&set),
            &mapping,
            &inputs,
            1,
        )
        .unwrap();
        let mut t = gp_arcade::Mcr::power_on(&set);
        t.step(gp_arcade::Controls::default());
        let mut want = Vec::new();
        for f in 2..=3 {
            t.set_trace(true);
            t.step(gp_arcade::Controls::default());
            let mut tr: Vec<_> = t.take_trace().into_iter().filter(|e| e.cpu == 0).collect();
            tr.sort_by_key(|e| e.time);
            for (n, e) in tr.iter().enumerate() {
                if e.interrupt.is_some() {
                    continue;
                }
                match e.pc {
                    // What `ld a,r` loaded: A at the next instruction.
                    0x0009 => {
                        if let Some(next) = tr[n + 1..].iter().find(|x| x.interrupt.is_none()) {
                            assert_eq!(next.pc, 0x000b);
                            want.push((f, "twice".to_owned(), (next.af >> 8) as u8));
                        }
                    }
                    0x000b => want.push((f, "held".to_owned(), (e.af >> 8) as u8)),
                    0x0013 => want.push((f, "b".to_owned(), 0x33)),
                    0x0014 => want.push((f, "vec".to_owned(), 0x5a)),
                    _ => {}
                }
            }
        }
        let mut have: Vec<_> = script
            .draws
            .iter()
            .map(|d| (d.frame, d.site.clone(), d.value))
            .collect();
        // A frame's last `ld a,r` lands in the next frame's trace.
        let last_frame_r = have
            .iter()
            .rposition(|d| d.1 == "twice")
            .filter(|&i| !want.contains(&have[i]));
        if let Some(i) = last_frame_r {
            have.remove(i);
        }
        have.sort();
        want.sort();
        assert!(have.len() > 100, "{}", have.len());
        assert_eq!(have, want);
        // Both ways are taken: bit 0 of R is the loop's coin.
        let twice: Vec<u8> = have
            .iter()
            .filter(|d| d.1 == "twice")
            .map(|d| d.2)
            .collect();
        assert!(twice.iter().any(|v| v & 1 == 0) && twice.iter().any(|v| v & 1 == 1));
    }

    /// Register sites on the NES: a made-up loop counts X and Y up from 7
    /// and 9 with A held at $2A; a site's draw is its register as the
    /// instruction starts.
    #[test]
    fn nes_register_sites_draw_the_register() {
        let mut rom = vec![0u8; 16 + 0x4000 + 0x2000];
        rom[..6].copy_from_slice(b"NES\x1A\x01\x01");
        #[rustfmt::skip]
        let code: &[u8] = &[
            0xA2, 0x07,       // C000 LDX #$07
            0xA0, 0x09,       // C002 LDY #$09
            0xA9, 0x2A,       // C004 LDA #$2A
            0xE8,             // C006 INX
            0xC8,             // C007 INY
            0xEA,             // C008 NOP
            0x4C, 0x06, 0xC0, // C009 JMP $C006
        ];
        rom[16..16 + code.len()].copy_from_slice(code);
        rom[16 + 0x3FFC..16 + 0x3FFE].copy_from_slice(&0xC000u16.to_le_bytes());
        let mapping = gp_interp::mapping::Mapping::parse(
            "schema = 1\nmodel = \"m\"\nsource = \"s\"\nalign = \"polled\"\n\n[[field]]\npath = \"entities.hero[0].x\"\nram = [0x10]\n\n[[draws]]\nsite = \"x\"\npc = 0xC006\nreg = \"x\"\n\n[[draws]]\nsite = \"y\"\npc = 0xC007\nreg = \"y\"\n\n[[draws]]\nsite = \"a\"\npc = 0xC008\nreg = \"a\"\n",
        )
        .unwrap();
        let inputs = vec![gp_evidence::input_log::InputFrame::default(); 2];
        let script = crate::lockstep::record_draws(&rom, &mapping, &inputs, 0).unwrap();
        let of = |site: &str| -> Vec<u8> {
            script
                .draws
                .iter()
                .filter(|d| d.site == site)
                .map(|d| d.value)
                .collect()
        };
        let (x, y, a) = (of("x"), of("y"), of("a"));
        assert!(x.len() > 100, "{}", x.len());
        // The last frame may end inside the loop.
        assert!(x.len() - a.len() <= 1, "{} {}", x.len(), a.len());
        for (n, (&x, &y)) in x.iter().zip(&y).enumerate() {
            assert_eq!(x, 7u8.wrapping_add(n as u8));
            assert_eq!(y, 9u8.wrapping_add(n as u8));
        }
        assert!(a.iter().all(|&a| a == 0x2A));
    }
}
