//! The agent side of music editing (docs/sound.md, "Editing by asking an
//! agent"; the pattern of docs/research/agent-sprite-editor.md): a person
//! asks for "a warmer bass" or "1-1 a little slower" and the agent answers
//! with these tools. There's no manual music editor.
//!
//! - `sound_banks`: the banks there are (the APU synthesizer and every SF2
//!   and SFZ in the bank folders), each patch with a short preview.
//! - `sound_describe`: a working copy's songs: channels, instruments with
//!   their roles and what each plays through now, where the notes came
//!   from and how sure capture is, what they're bound to, and whether each
//!   is ready to edit.
//! - `sound_edit`: deterministic ops (`gp_sound::edit`), checked, written
//!   to the copy's song sets and sound bank mapping (or a theme's
//!   restyled-audio layer), with before and after clips, history and
//!   undo, and an event in a live capture run for the person to accept or
//!   reject.
//! - `sound_preview`: the copy played in the interpreter on a baseline's
//!   input with its music and effects cued as the game would cue them,
//!   rendered to a WAV.
//!
//! A song is ready to edit when the model knows it completely
//! (`gp_sound::edit::readiness`): every channel captured with enough
//! confidence, instruments clustered and given roles, its loop known, and
//! tied to a level or state (an effect to an event).

use std::collections::BTreeMap;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use gp_model::Model;
use gp_sound::banks::{Mapping, Target};
use gp_sound::edit::Op;
use gp_sound::song::{Kind, SongSet};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{Config, Reply};

/// Where a copy keeps its sound: song sets that edits changed and the
/// mapping.
pub const SOUND_DIR: &str = "sound";
pub const BANKS_FILE: &str = "sound/banks.toml";
pub const HISTORY_FILE: &str = "sound-edits.jsonl";
/// Seconds a clip runs by default, and at most.
pub const CLIP_SECONDS: f64 = 8.0;
pub const MAX_SECONDS: f64 = 60.0;
/// Patches previewed per bank by `sound_banks`.
pub const MAX_PREVIEWS: usize = 8;

fn read_model(dir: &Path) -> Result<Model, String> {
    let (_, m) = crate::tools::load_model(dir)?;
    Ok(m)
}

/// The copy's song set for a slot: the copy's own (written by an edit), or
/// the slot's capture under `$GP_HOME/work/sound/`.
pub fn slot_set(config: &Config, dir: &Path, model: &Model, slot: &str) -> Result<SongSet, String> {
    let own = dir.join(SOUND_DIR).join(format!("{slot}.songs.json"));
    if let Ok(t) = fs::read_to_string(&own) {
        return SongSet::parse(&t);
    }
    let s = model
        .assets
        .get(slot)
        .ok_or(format!("no asset slot `{slot}`"))?;
    let c = s.capture.as_ref().ok_or(format!(
        "slot `{slot}` has no capture recipe, so it has no songs"
    ))?;
    let root = config.home.join("work/sound");
    let name = gp_sound::capture::slug(&c.name);
    let mut dirs: Vec<PathBuf> = fs::read_dir(&root)
        .map_err(|e| format!("{}: {e} (capture with gp sound capture)", root.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(&c.recording) || c.recording.starts_with(n))
        })
        .collect();
    dirs.sort();
    for d in dirs {
        if let Ok(t) = fs::read_to_string(d.join(&name).join("songs.json")) {
            return SongSet::parse(&t);
        }
    }
    Err(format!(
        "slot `{slot}`'s capture `{}` of recording {} isn't under {}; run gp sound capture --recording {} --frames {} --name {}",
        c.name,
        c.recording,
        root.display(),
        c.recording,
        c.frames,
        c.name
    ))
}

fn sound_slots(model: &Model) -> Vec<String> {
    model
        .assets
        .iter()
        .filter(|(_, s)| {
            matches!(
                s.kind,
                gp_model::model::AssetKind::Songs | gp_model::model::AssetKind::SoundEffects
            )
        })
        .map(|(n, _)| n.clone())
        .collect()
}

fn mapping(dir: &Path) -> Result<Mapping, String> {
    match fs::read_to_string(dir.join(BANKS_FILE)) {
        Ok(t) => Mapping::parse(&t),
        Err(_) => Ok(Mapping::default()),
    }
}

fn folders(config: &Config, dir: &Path) -> Vec<PathBuf> {
    let mut v = vec![dir.join(SOUND_DIR)];
    v.extend(gp_sound::banks::folders(&config.home));
    v
}

fn wav_file(path: &Path, samples: &[i16]) -> Result<(), String> {
    if let Some(d) = path.parent() {
        fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    fs::write(
        path,
        gp_sound::synth::wav(samples, gp_sound::synth::SAMPLE_RATE),
    )
    .map_err(|e| format!("{}: {e}", path.display()))
}

/// A clip of a song as the mapping plays it: its first `seconds`.
fn clip(
    set: &SongSet,
    song: &str,
    map: &Mapping,
    folders: &[PathBuf],
    seconds: f64,
) -> Result<Vec<i16>, String> {
    let s = set.song(song).ok_or(format!("no song `{song}`"))?;
    let lf = s.loop_frame.unwrap_or(0);
    let want = (seconds * set.region.frame_rate()) as u64;
    let loops = if s.frames >= want || s.frames <= lf {
        0
    } else {
        ((want - s.frames).div_ceil(s.frames - lf)) as u32
    };
    let r = gp_sound_bank::render(set, song, map, folders, loops)?;
    let n = ((seconds * f64::from(gp_sound::synth::SAMPLE_RATE)) as usize).min(r.samples.len());
    Ok(r.samples[..n].to_vec())
}

// ---------------------------------------------------------------- banks

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BanksArgs {
    /// A copy, whose `sound/` folder is searched too.
    #[serde(default)]
    pub copy: Option<String>,
    /// Render a one-second preview of each patch (the first eight of a
    /// bank). Default true.
    #[serde(default = "yes")]
    pub previews: bool,
}

fn yes() -> bool {
    true
}

pub fn banks(config: &Config, a: BanksArgs) -> Result<Reply, String> {
    let dir = a
        .copy
        .as_deref()
        .map(|c| config.existing_copy(c))
        .transpose()?;
    let mut f = gp_sound::banks::folders(&config.home);
    if let Some(d) = &dir {
        f.insert(0, d.join(SOUND_DIR));
    }
    let found = gp_sound::banks::discover(&f);
    let previews = config.work.join(".sound-previews");
    let mut out = Vec::new();
    for b in &found {
        let mut patches = Vec::new();
        for (i, p) in b.patches.iter().enumerate() {
            let mut v = json!({"name": p.name, "bank": p.bank, "program": p.program});
            if let Some(k) = p.keys {
                v["keys"] = json!(k);
            }
            if a.previews && i < MAX_PREVIEWS {
                let file = previews
                    .join(&b.name)
                    .join(format!("{}-{}.wav", p.bank, p.program));
                let samples = match b.kind {
                    gp_sound::banks::BankKind::Apu => Some(apu_preview(i)),
                    gp_sound::banks::BankKind::Sf2 => {
                        gp_sound_bank::preview(&b.path, p.bank, p.program).ok()
                    }
                    gp_sound::banks::BankKind::Sfz => None,
                };
                match samples {
                    Some(s) => {
                        wav_file(&file, &s)?;
                        v["preview"] = json!(file);
                    }
                    None => v["preview_note"] = json!("SFZ regions aren't rendered yet"),
                }
            }
            patches.push(v);
        }
        out.push(json!({
            "name": b.name,
            "kind": b.kind,
            "path": b.path,
            "license": b.license,
            "patches": patches,
        }));
    }
    Ok(Reply::ok(json!({
        "folders": f,
        "banks": out,
        "note": "Put free SF2 or SFZ banks (with their licence beside them) in $GP_SOUND_BANKS or $GP_HOME/sound-banks; map an instrument or role to a patch with sound_edit's remap op",
    })))
}

/// A C major arpeggio on the APU voice `i` (pulse, triangle, noise, DMC).
fn apu_preview(i: usize) -> Vec<i16> {
    use gp_sound::apu::Channel;
    let c = [
        Channel::Pulse1,
        Channel::Triangle,
        Channel::Noise,
        Channel::Dmc,
    ][i.min(3)];
    let mut set = SongSet::new(gp_sound::apu::Region::Ntsc);
    set.instruments.push(gp_sound::song::Instrument {
        name: "preview".into(),
        voice: gp_sound::song::Voice::of(c),
        role: None,
        volume: vec![12, 11, 10, 9, 8],
        duty: if c == Channel::Pulse1 {
            vec![2]
        } else {
            vec![]
        },
        pitch: vec![],
        sample: None,
        notes: 0,
    });
    let tempo = gp_sound::song::Tempo {
        frames_per_row: 15,
        rows_per_pattern: 4,
    };
    let placed: Vec<gp_sound::song::Placed> = [60u8, 64, 67, 72]
        .iter()
        .enumerate()
        .map(|(k, &m)| gp_sound::song::Placed {
            start: k as u64 * 15,
            note: gp_sound::song::Note {
                at: 0,
                len: 14,
                pitch: if c == Channel::Noise { 8 + k as u8 } else { m },
                fine: 0,
                instrument: 0,
            },
        })
        .collect();
    set.songs.push(gp_sound::song::Song {
        name: "preview".into(),
        kind: Kind::Music,
        origin: gp_sound::song::Origin::Edited,
        confidence: 1.0,
        driver: None,
        frames: 60,
        loop_frame: None,
        tempo,
        tracks: vec![gp_sound::song::Track::from_placed(c, &placed, tempo, 60)],
        request: None,
        event: None,
        level: None,
        state: None,
        captured: None,
        notes: vec![],
    });
    let (tl, n) = gp_sound::player::song_timeline(&set, "preview").expect("the preview song");
    gp_sound::synth::render(
        set.region,
        &tl,
        set.region.frame_start(n),
        &[],
        gp_sound::synth::UNITY,
    )
}

// ------------------------------------------------------------- describe

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DescribeArgs {
    pub copy: String,
    /// One sound slot; default every one.
    #[serde(default)]
    pub slot: Option<String>,
    /// One song; default every one.
    #[serde(default)]
    pub song: Option<String>,
}

fn target_text(t: Option<&Target>) -> Value {
    match t {
        None | Some(Target::Apu) => json!("apu"),
        Some(Target::Gm { program }) => json!({"gm": program}),
        Some(Target::Bank {
            bank,
            preset,
            program,
            ..
        }) => json!({"bank": bank, "preset": preset, "program": program}),
    }
}

pub fn describe(config: &Config, a: DescribeArgs) -> Result<Reply, String> {
    let dir = config.existing_copy(&a.copy)?;
    let model = read_model(&dir)?;
    let map = mapping(&dir)?;
    let slots: Vec<String> = match &a.slot {
        Some(s) => vec![s.clone()],
        None => sound_slots(&model),
    };
    if slots.is_empty() {
        return Err("the model has no songs or sound-effects slot".into());
    }
    let bindings = model
        .present
        .as_ref()
        .and_then(|p| p.sound.clone())
        .unwrap_or_default();
    let mut out = Vec::new();
    for slot in &slots {
        let set = match slot_set(config, &dir, &model, slot) {
            Ok(s) => s,
            Err(e) => {
                out.push(json!({"slot": slot, "error": e}));
                continue;
            }
        };
        let mut songs = Vec::new();
        for s in set
            .songs
            .iter()
            .filter(|s| a.song.as_ref().is_none_or(|n| *n == s.name))
        {
            let mut used: BTreeMap<u16, usize> = BTreeMap::new();
            let channels: Vec<Value> = s
                .tracks
                .iter()
                .map(|t| {
                    let p = t.placed(s.tempo);
                    for x in &p {
                        *used.entry(x.note.instrument).or_default() += 1;
                    }
                    json!({"channel": t.channel.name(), "notes": p.len(), "patterns": t.patterns.len()})
                })
                .collect();
            let instruments: Vec<Value> = used
                .iter()
                .map(|(&i, &n)| {
                    let ins = &set.instruments[usize::from(i)];
                    json!({
                        "name": ins.name, "voice": ins.voice, "role": ins.role, "notes": n,
                        "plays_through": target_text(map.rule_for(&s.name, ins).map(|r| &r.target)),
                    })
                })
                .collect();
            let bound: Vec<Value> = bindings
                .music
                .iter()
                .filter(|m| m.song.slot == *slot && m.song.song == s.name)
                .map(|m| json!({"level": m.level, "state": m.state}))
                .chain(
                    bindings
                        .sfx
                        .iter()
                        .filter(|e| e.slot == *slot && e.effect == s.name)
                        .map(|e| json!({"event": e.event})),
                )
                .collect();
            let missing = gp_sound::edit::readiness(&set, &s.name);
            songs.push(json!({
                "name": s.name, "kind": s.kind, "origin": s.origin, "driver": s.driver,
                "confidence": s.confidence, "frames": s.frames, "loop_frame": s.loop_frame,
                "tempo": s.tempo, "channels": channels, "instruments": instruments,
                "bound_in_model": bound, "capture_binding": {"level": s.level, "state": s.state, "event": s.event},
                "editable": missing.is_empty(), "missing": missing, "notes": s.notes,
            }));
        }
        out.push(json!({"slot": slot, "role": model.assets.get(slot).map(|x| x.role.clone()), "songs": songs}));
    }
    Ok(Reply::ok(
        json!({"slots": out, "mix": map.mix, "banks": map.banks}),
    ))
}

// ----------------------------------------------------------------- edit

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EditArgs {
    pub copy: String,
    /// The sound slot whose songs the ops change (default: the first).
    #[serde(default)]
    pub slot: Option<String>,
    /// The ops, run in order: remap (an instrument or role to a target:
    /// {kind: apu}, {kind: gm, program}, {kind: bank, bank, preset or
    /// program}), role, transpose, tempo, mute, solo, gain, swap.
    #[serde(default)]
    pub ops: Vec<Op>,
    /// Put back the last edit instead.
    #[serde(default)]
    pub undo: bool,
    /// Write the result (default true); false previews only.
    #[serde(default = "yes")]
    pub place: bool,
    /// The song the before and after clips play (default: the first song
    /// an op names, else the slot's first music).
    #[serde(default)]
    pub clip_song: Option<String>,
    /// Seconds of each clip (default 8, at most 60).
    #[serde(default)]
    pub seconds: Option<f64>,
    /// A theme of the copy (`<copy>/themes/<id>/`) whose restyled-audio
    /// layer takes the mapping, instead of the copy's `sound/banks.toml`.
    #[serde(default)]
    pub theme: Option<String>,
    /// A live capture run to post the edit to, for the person to accept
    /// or reject.
    #[serde(default)]
    pub run: Option<String>,
    /// What the person asked, kept in the history.
    #[serde(default)]
    pub prompt: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Entry {
    seq: u32,
    slot: String,
    ops: Vec<Op>,
    prompt: Option<String>,
    /// The set and mapping files before (None: there was none).
    set_before: Option<String>,
    map_file: String,
    map_before: Option<String>,
}

fn read_history(dir: &Path) -> Result<Vec<Entry>, String> {
    let Ok(t) = fs::read_to_string(dir.join(HISTORY_FILE)) else {
        return Ok(Vec::new());
    };
    t.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).map_err(|e| format!("{HISTORY_FILE}: {e}")))
        .collect()
}

fn write_history(dir: &Path, h: &[Entry]) -> Result<(), String> {
    let mut s = String::new();
    for e in h {
        s.push_str(&serde_json::to_string(e).map_err(|e| e.to_string())?);
        s.push('\n');
    }
    fs::write(dir.join(HISTORY_FILE), s).map_err(|e| format!("{HISTORY_FILE}: {e}"))
}

/// The mapping file an edit writes: the copy's, or a theme's audio layer.
fn map_file(dir: &Path, theme: Option<&str>) -> Result<PathBuf, String> {
    match theme {
        None => Ok(dir.join(BANKS_FILE)),
        Some(t) => {
            if !gp_model::path::is_name(t) {
                return Err(format!("`{t}` isn't a theme id"));
            }
            let td = dir.join("themes").join(t);
            if !td.join("theme.toml").is_file() {
                return Err(format!(
                    "the copy has no theme `{t}` (themes/{t}/theme.toml)"
                ));
            }
            Ok(td.join("audio-banks.toml"))
        }
    }
}

/// Adds an audio restyle naming `audio-banks.toml` for every sound slot
/// to a theme, when it has none.
fn theme_audio(dir: &Path, theme: &str) -> Result<(), String> {
    let f = dir.join("themes").join(theme).join("theme.toml");
    let text = fs::read_to_string(&f).map_err(|e| format!("{}: {e}", f.display()))?;
    let mut t = gp_theme::Theme::parse(&text)?;
    if t.audio.iter().any(|a| a.banks == "audio-banks.toml") {
        return Ok(());
    }
    t.audio.push(gp_theme::format::AudioRestyle {
        target: "sound".into(),
        banks: "audio-banks.toml".into(),
        license: t.license.clone(),
        note: Some("written by sound_edit".into()),
    });
    if !gp_theme::version_supported("0.1.1") || t.format == "0.1.0" {
        t.format = gp_theme::FORMAT_VERSION.into();
    }
    fs::write(&f, t.to_text()).map_err(|e| format!("{}: {e}", f.display()))
}

pub fn edit(config: &Config, a: EditArgs) -> Result<Reply, String> {
    let dir = config.existing_copy(&a.copy)?;
    if let Some(run) = &a.run {
        crate::asset_edit::run_dir(config, run)?;
    }
    if a.undo {
        return undo(config, &dir, &a);
    }
    if a.ops.is_empty() {
        return Err("give at least one op".into());
    }
    if a.ops.len() > 32 {
        return Err("at most 32 ops a call".into());
    }
    let model = read_model(&dir)?;
    let slot = match &a.slot {
        Some(s) => s.clone(),
        None => sound_slots(&model)
            .into_iter()
            .next()
            .ok_or("the model has no sound slot")?,
    };
    let set = slot_set(config, &dir, &model, &slot)?;
    let mfile = map_file(&dir, a.theme.as_deref())?;
    let map_before = fs::read_to_string(&mfile).ok();
    let map = match &map_before {
        Some(t) => Mapping::parse(t)?,
        None => Mapping::default(),
    };
    // The readiness gate: every song an op names must be known completely.
    let mut named: Vec<String> = Vec::new();
    for op in &a.ops {
        match op {
            Op::Transpose { song, .. } | Op::Tempo { song, .. } | Op::Swap { song, .. } => {
                named.push(song.clone())
            }
            Op::Remap { song: Some(s), .. } => named.push(s.clone()),
            _ => {}
        }
    }
    named.dedup();
    for s in &named {
        let missing = gp_sound::edit::readiness(&set, s);
        if !missing.is_empty() {
            return Ok(Reply::refused(json!({
                "ok": false, "stage": "readiness", "song": s, "missing": missing,
                "hint": "sound_describe says what each song lacks; capture it again over more frames, or bind it in the model",
            })));
        }
    }
    let (new_set, new_map, said) = gp_sound::edit::apply(&set, &map, &a.ops)?;
    let clip_song = a
        .clip_song
        .clone()
        .or_else(|| named.first().cloned())
        .or_else(|| {
            set.songs
                .iter()
                .find(|s| s.kind == Kind::Music)
                .map(|s| s.name.clone())
        })
        .ok_or("the slot has no song to play")?;
    let seconds = a.seconds.unwrap_or(CLIP_SECONDS).clamp(0.5, MAX_SECONDS);
    let f = folders(config, &dir);
    let before = clip(&set, &clip_song, &map, &f, seconds)?;
    let after = clip(&new_set, &clip_song, &new_map, &f, seconds)?;
    let mut out = json!({"ok": true, "said": said, "clip_song": clip_song, "seconds": seconds});
    if !a.place {
        out["written"] = json!(false);
        return Ok(Reply::ok(out));
    }
    // Write: the copy's set and the mapping, then the history and clips.
    let set_file = dir.join(SOUND_DIR).join(format!("{slot}.songs.json"));
    let set_before = fs::read_to_string(&set_file).ok();
    fs::create_dir_all(dir.join(SOUND_DIR)).map_err(|e| e.to_string())?;
    fs::write(&set_file, new_set.to_text()).map_err(|e| format!("{}: {e}", set_file.display()))?;
    if let Some(d) = mfile.parent() {
        fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    fs::write(&mfile, new_map.to_text()).map_err(|e| format!("{}: {e}", mfile.display()))?;
    if let Some(t) = &a.theme {
        theme_audio(&dir, t)?;
    }
    let mut history = read_history(&dir)?;
    let seq = history.last().map_or(1, |e| e.seq + 1);
    history.push(Entry {
        seq,
        slot: slot.clone(),
        ops: a.ops.clone(),
        prompt: a.prompt.clone(),
        set_before,
        map_file: mfile
            .strip_prefix(&dir)
            .unwrap_or(&mfile)
            .display()
            .to_string(),
        map_before,
    });
    write_history(&dir, &history)?;
    let pv = dir.join(crate::asset_edit::PREVIEW_DIR);
    let bf = pv.join(format!("sound-{seq}-before.wav"));
    let af = pv.join(format!("sound-{seq}-after.wav"));
    wav_file(&bf, &before)?;
    wav_file(&af, &after)?;
    out["written"] = json!(true);
    out["history"] = json!({"seq": seq, "depth": history.len()});
    out["before"] = json!(bf);
    out["after"] = json!(af);
    if let Some(run) = &a.run {
        let rd = crate::asset_edit::run_dir(config, run)?;
        let line = json!({
            "v": 0, "actor": "agent", "kind": "sound_edit", "item": format!("sound:{seq}"),
            "text": format!("{}: {}", clip_song, said.join("; ")),
            "data": {"edit": seq, "before": bf, "after": af},
        });
        let p = rd.join("inputs.ndjson");
        let mut fh = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&p)
            .map_err(|e| format!("{}: {e}", p.display()))?;
        fh.write_all(format!("{line}\n").as_bytes())
            .map_err(|e| e.to_string())?;
        out["live"] = json!(p);
    }
    Ok(Reply::ok(out))
}

fn undo(_config: &Config, dir: &Path, a: &EditArgs) -> Result<Reply, String> {
    if !a.ops.is_empty() {
        return Err("undo takes no ops".into());
    }
    let mut history = read_history(dir)?;
    let last = history.pop().ok_or("there's no sound edit to undo")?;
    let set_file = dir
        .join(SOUND_DIR)
        .join(format!("{}.songs.json", last.slot));
    let put = |p: &Path, v: &Option<String>| -> Result<(), String> {
        match v {
            Some(t) => fs::write(p, t).map_err(|e| format!("{}: {e}", p.display())),
            // There was no file: an empty one would differ, so the copy's
            // own file goes back to the capture's by renaming it aside.
            None if p.exists() => {
                fs::rename(p, p.with_extension("undone")).map_err(|e| e.to_string())
            }
            None => Ok(()),
        }
    };
    put(&set_file, &last.set_before)?;
    put(&dir.join(&last.map_file), &last.map_before)?;
    write_history(dir, &history)?;
    Ok(Reply::ok(
        json!({"ok": true, "undone": last.seq, "ops": last.ops, "depth": history.len()}),
    ))
}

// -------------------------------------------------------------- preview

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PreviewArgs {
    pub copy: String,
    /// A baseline of the model's source whose input plays the game, so the
    /// music and effects come as the game cues them.
    #[serde(default)]
    pub baseline: Option<String>,
    /// Frames to run without a baseline (default 600).
    #[serde(default)]
    pub frames: Option<usize>,
    /// Button holds, as in baselines (`right+b@10-`).
    #[serde(default)]
    pub holds: Vec<String>,
}

pub fn preview(config: &Config, a: PreviewArgs) -> Result<Reply, String> {
    let dir = config.existing_copy(&a.copy)?;
    let model = read_model(&dir)?;
    let pa = crate::tools::PreviewArgs {
        copy: a.copy.clone(),
        frames: a.baseline.is_none().then(|| a.frames.unwrap_or(600)),
        holds: a.holds.clone(),
        baseline: a.baseline.clone(),
        paths: Vec::new(),
        at: Vec::new(),
        picture: false,
    };
    let frames = crate::tools::input(config, &model, &pa)?;
    if frames.len() > 60 * 60 * 10 {
        return Err("at most ten minutes of frames".into());
    }
    let assets = config
        .assets_of(&dir, model.model.source.as_deref())
        .ok_or("the model names no source, so start the server with --assets")?;
    let lifted = gp_interp::Lifted::read_dir(&assets, &model).map_err(|e| e.to_string())?;
    let mut interp = gp_interp::Interp::load(&model, &lifted)?;
    // The cues, from the model's bindings and the spec's numbering.
    let snd = model
        .present
        .as_ref()
        .and_then(|p| p.sound.clone())
        .ok_or("the model binds no sound (present.sound)")?;
    let spec = interp.spec().clone();
    let mut cues = gp_present::sound::Cues::default();
    let mut sets: BTreeMap<String, SongSet> = BTreeMap::new();
    let mut notes = Vec::new();
    for m in &snd.music {
        let k = format!("{}/{}", m.song.slot, m.song.song);
        if let Some(l) = &m.level
            && let Some(i) = spec.levels.iter().position(|x| &x.name == l)
        {
            cues.level.push((i, k.clone()));
        }
        if let Some((r, s)) = m.state.as_deref().and_then(|s| s.split_once('.'))
            && let Some(mi) = spec.machines.iter().position(|x| x.name == r)
            && let Some(si) = spec.machines[mi].states.iter().position(|x| x.name == s)
        {
            cues.state.push((mi, si, k.clone()));
        }
        sets.entry(m.song.slot.clone())
            .or_insert(SongSet::new(gp_sound::apu::Region::Ntsc));
    }
    for e in &snd.sfx {
        if let Some(i) = spec.events.iter().position(|x| x == &e.event) {
            cues.sfx.push((i, format!("{}/{}", e.slot, e.effect)));
        } else {
            notes.push(format!("event `{}` isn't in the spec", e.event));
        }
        sets.entry(e.slot.clone())
            .or_insert(SongSet::new(gp_sound::apu::Region::Ntsc));
    }
    for (slot, v) in sets.iter_mut() {
        match slot_set(config, &dir, &model, slot) {
            Ok(s) => *v = s,
            Err(e) => notes.push(e),
        }
    }
    let refs: Vec<(String, &SongSet)> = sets.iter().map(|(k, v)| (k.clone(), v)).collect();
    let merged = SongSet::merged(&refs);
    let map = mapping(&dir)?;
    let mut player = gp_sound::player::Player::new(merged.clone());
    let mut synth = gp_sound::synth::Synth::new(
        merged.region,
        gp_sound::synth::SAMPLE_RATE,
        merged.samples.clone(),
    );
    synth.gains = map.mix.gains();
    let mut director = gp_present::sound::Director::new();
    let mut samples = Vec::new();
    let mut cued = Vec::new();
    for (i, f) in frames.iter().enumerate() {
        let events = interp.advance(f)?;
        let c = director.step(&cues, interp.world(), &events);
        if let Some(m) = &c.music {
            cued.push(json!({"frame": i + 1, "music": m}));
            player.play_music(m.as_deref());
        }
        for e in &c.sfx {
            cued.push(json!({"frame": i + 1, "sfx": e}));
            player.play_effect(e);
        }
        let w = player.frame();
        samples.extend(synth.frame(&w));
    }
    let n = fs::read_dir(dir.join(crate::asset_edit::PREVIEW_DIR)).map_or(0, |r| r.count());
    let file = dir
        .join(crate::asset_edit::PREVIEW_DIR)
        .join(format!("sound-preview-{}.wav", n + 1));
    wav_file(&file, &samples)?;
    if map.uses_banks(&merged, "") || !map.rules.is_empty() {
        notes.push("in-game previews play the APU with the mapping's mix; sound_edit's clips play bank targets".into());
    }
    cued.truncate(200);
    Ok(Reply::ok(json!({
        "file": file,
        "frames": frames.len(),
        "seconds": samples.len() as f64 / f64::from(gp_sound::synth::SAMPLE_RATE),
        "cued": cued,
        "notes": notes,
    })))
}
