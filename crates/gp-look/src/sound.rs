//! The model's sound for a generated game (docs/sound.md, "Playback"):
//! `present.sound`'s bindings compiled into cues (`assets/sound.txt`,
//! `gp_present::sound`) and each bound song and effect rendered ahead of
//! play into `assets/sound/<slot>/<name>.wav`, through the APU synthesizer
//! or, when a theme's restyled-audio layer maps the slot's role to a sound
//! bank, through the bank (gp-sound-bank). The render crate plays them
//! with Bevy's audio when built with its `audio` feature. Songs are game
//! content: they come from the slots' captures outside the repo, and the
//! project is written outside it too.

use std::collections::BTreeMap;
use std::path::PathBuf;

use gp_model::Model;
use gp_present::sound::Cues;
use gp_sim::Spec;
use gp_sound::apu::Channel;
use gp_sound::banks::Mapping;
use gp_sound::song::SongSet;

/// What the sound of a project is: the cues, and files by project path.
#[derive(Debug, Clone, Default)]
pub struct SoundArt {
    pub cues: Cues,
    pub files: Vec<(String, Vec<u8>)>,
    /// The song sets by slot, as the web preview loads them.
    pub sets: BTreeMap<String, String>,
    /// What couldn't be bound or rendered.
    pub notes: Vec<String>,
    /// The channel stems rendered, as `<slot>/<name>.<channel>`: each bound
    /// song and effect with only that channel sounding, for per-channel
    /// mute, at `assets/sound/<slot>/<name>.<channel>.wav`. A channel the
    /// sound never uses has none.
    pub stems: Vec<String>,
    /// The sound roles a theme replaces, each rendered to
    /// `assets/sound/roles/<role>.wav` ([`role_files`]).
    pub roles: Vec<String>,
}

/// Loops rendered after a song's first pass, so its file loops cleanly
/// enough when played on repeat.
pub const LOOPS: u32 = 1;

/// Compiles and renders the model's sound. `sets` are the song sets by
/// slot; `theme` gives a slot role's bank mapping; `folders` are where
/// banks are found.
pub fn art(
    model: &Model,
    spec: &Spec,
    sets: &BTreeMap<String, SongSet>,
    theme: &gp_theme::Resolved,
    folders: &[PathBuf],
) -> Result<SoundArt, String> {
    let mut out = SoundArt::default();
    let Some(snd) = model.present.as_ref().and_then(|p| p.sound.as_ref()) else {
        return Ok(out);
    };
    let mut want: Vec<(String, String)> = Vec::new();
    let key = |slot: &str, name: &str| format!("{slot}/{name}");
    for m in &snd.music {
        let k = key(&m.song.slot, &m.song.song);
        if let Some(l) = &m.level {
            match spec.levels.iter().position(|x| &x.name == l) {
                Some(i) => out.cues.level.push((i, k.clone())),
                None => out
                    .notes
                    .push(format!("level `{l}` isn't in the compiled spec")),
            }
        }
        if let Some(st) = &m.state {
            let found = st.split_once('.').and_then(|(r, s)| {
                let mi = spec.machines.iter().position(|x| x.name == r)?;
                let si = spec.machines[mi].states.iter().position(|x| x.name == s)?;
                Some((mi, si))
            });
            match found {
                Some((mi, si)) => out.cues.state.push((mi, si, k.clone())),
                None => out
                    .notes
                    .push(format!("rule state `{st}` isn't in the compiled spec")),
            }
        }
        want.push((m.song.slot.clone(), m.song.song.clone()));
    }
    for e in &snd.sfx {
        match spec.events.iter().position(|x| x == &e.event) {
            Some(i) => out.cues.sfx.push((i, key(&e.slot, &e.effect))),
            None => out
                .notes
                .push(format!("event `{}` isn't in the compiled spec", e.event)),
        }
        want.push((e.slot.clone(), e.effect.clone()));
    }
    out.cues.roles = role_cues(snd, spec, &mut out.notes);
    out.cues.silent = silent_cues(snd, spec, &mut out.notes);
    for (role, wav) in role_files(&out.cues, theme, &mut out.notes) {
        out.files
            .push((format!("assets/{}", role_path(&role)), wav));
        out.roles.push(role);
    }
    // While a theme replaces a role, the model's own sound is silent, so
    // its songs aren't bound or rendered and its sound data isn't read.
    if theme.replaces_sound() {
        out.cues.level.clear();
        out.cues.state.clear();
        out.cues.sfx.clear();
        return Ok(out);
    }
    want.sort();
    want.dedup();
    for (slot, name) in want {
        let Some(set) = sets.get(&slot) else {
            out.notes.push(format!(
                "slot `{slot}` has no songs (capture them with gp sound capture)"
            ));
            continue;
        };
        let Some(song) = set.song(&name) else {
            out.notes
                .push(format!("slot `{slot}` has no song `{name}`"));
            continue;
        };
        let role = model
            .assets
            .get(&slot)
            .map(|s| s.role.as_str())
            .unwrap_or("");
        let loops = if song.kind == gp_sound::song::Kind::Music {
            LOOPS
        } else {
            0
        };
        let bank = match theme.audio_mapping(role) {
            Some((file, dir)) => {
                let text = std::fs::read_to_string(&file)
                    .map_err(|e| format!("{}: {e}", file.display()))?;
                let map = Mapping::parse(&text)?;
                let mut f = vec![dir];
                f.extend(folders.iter().cloned());
                Some((map, f))
            }
            None => None,
        };
        // The sound with only `solo` sounding, or all of it.
        let render = |solo: Option<Channel>, notes: &mut Vec<String>| -> Result<Vec<i16>, String> {
            Ok(match &bank {
                Some((map, f)) => {
                    let mut map = map.clone();
                    if let Some(c) = solo {
                        map.mix.solo = vec![c.name().to_owned()];
                    }
                    let r = gp_sound_bank::render(set, &name, &map, f, loops)?;
                    if solo.is_none() {
                        notes.extend(r.fallbacks);
                    }
                    r.samples
                }
                None => {
                    let mut p = gp_sound::player::Player::new(set.clone());
                    p.play_music(Some(&name));
                    let lf = song.loop_frame.unwrap_or(0);
                    let frames = song.frames + u64::from(loops) * (song.frames - lf);
                    let mut synth = gp_sound::synth::Synth::new(
                        set.region,
                        gp_sound::synth::SAMPLE_RATE,
                        set.samples.clone(),
                    );
                    if let Some(c) = solo {
                        synth.gains = [0.0; 5];
                        synth.gains[c.index()] = 1.0;
                    }
                    let mut v = Vec::new();
                    for _ in 0..frames {
                        let w = p.frame();
                        v.extend(synth.frame(&w));
                    }
                    v
                }
            })
        };
        let samples = render(None, &mut out.notes)?;
        out.files.push((
            format!("assets/sound/{slot}/{name}.wav"),
            gp_sound::synth::wav(&samples, gp_sound::synth::SAMPLE_RATE),
        ));
        for c in Channel::ALL {
            let stem = render(Some(c), &mut out.notes)?;
            if stem.iter().all(|v| *v == 0) {
                continue;
            }
            out.stems.push(format!("{slot}/{name}.{}", c.name()));
            out.files.push((
                format!("assets/sound/{slot}/{name}.{}.wav", c.name()),
                gp_sound::synth::wav(&stem, gp_sound::synth::SAMPLE_RATE),
            ));
        }
    }
    for slot in out
        .cues
        .level
        .iter()
        .map(|x| &x.1)
        .chain(out.cues.state.iter().map(|x| &x.2))
        .chain(out.cues.sfx.iter().map(|x| &x.1))
        .filter_map(|k| k.split_once('/').map(|x| x.0.to_owned()))
        .collect::<std::collections::BTreeSet<_>>()
    {
        if let Some(s) = sets.get(&slot) {
            out.sets.insert(slot, s.to_text());
        }
    }
    Ok(out)
}

/// The captured song sets of a model's sound slots, by slot: each slot's
/// `capture` recipe names a recording and a capture name, found under
/// `<home>/work/sound/<recording>/<name>/songs.json`. Slots whose capture
/// isn't there are left out (the generator says so).
pub fn slot_sets(model: &Model, home: &std::path::Path) -> BTreeMap<String, SongSet> {
    let mut out = BTreeMap::new();
    let root = home.join("work/sound");
    for (slot, s) in &model.assets {
        let Some(c) = &s.capture else { continue };
        let name = gp_sound::capture::slug(&c.name);
        let Ok(rd) = std::fs::read_dir(&root) else {
            continue;
        };
        let mut dirs: Vec<PathBuf> = rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(&c.recording) || c.recording.starts_with(n))
            })
            .collect();
        dirs.sort();
        for d in dirs {
            let f = d.join(&name).join("songs.json");
            if let Ok(text) = std::fs::read_to_string(&f)
                && let Ok(set) = SongSet::parse(&text)
            {
                out.insert(slot.clone(), set);
                break;
            }
        }
    }
    out
}

/// What the web preview loads (`assets/sound.json`): the cues, the song
/// sets they name, by slot, and the sound roles a theme replaces, whose
/// files are `assets/sound/roles/<role>.wav`.
pub fn web_bundle(art: &SoundArt) -> String {
    let mut s = String::from("{\"cues\":");
    s.push_str(&json_string(&art.cues.write()));
    s.push_str(",\"sets\":{");
    for (i, (slot, text)) in art.sets.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&json_string(slot));
        s.push(':');
        s.push_str(text.trim_end());
    }
    s.push_str("},\"roles\":[");
    for (i, r) in art.roles.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&json_string(r));
    }
    s.push_str("]}");
    s
}

fn json_string(v: &str) -> String {
    let mut s = String::from("\"");
    for c in v.chars() {
        match c {
            '"' => s.push_str("\\\""),
            '\\' => s.push_str("\\\\"),
            '\n' => s.push_str("\\n"),
            c if (c as u32) < 0x20 => s.push_str(&format!("\\u{:04x}", c as u32)),
            c => s.push(c),
        }
    }
    s.push('"');
    s
}

/// Where a sound role's file is, under `assets/` or a look's folder.
pub fn role_path(role: &str) -> String {
    format!("sound/roles/{role}.wav")
}

/// Each sound role the theme stack replaces (format 0.1.3's `sound`),
/// rendered to a 16-bit mono WAV at [`gp_present::sound::ROLE_RATE`], the
/// files every runtime's [`gp_present::sound::RolePlayer`] plays. A role
/// whose file can't be read or decoded is noted and left silent.
pub fn role_files(
    cues: &Cues,
    theme: &gp_theme::Resolved,
    notes: &mut Vec<String>,
) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    for r in &cues.roles {
        let Some(file) = theme.sound_file(&r.name) else {
            continue;
        };
        let samples = std::fs::read(&file)
            .map_err(|e| format!("{}: {e}", file.display()))
            .and_then(|b| gp_theme::tune::samples_of(&file.to_string_lossy(), &b));
        match samples {
            Ok(s) => {
                let s = gp_theme::tune::resample(
                    &s,
                    gp_theme::tune::RATE,
                    gp_present::sound::ROLE_RATE,
                );
                out.push((
                    r.name.clone(),
                    gp_theme::tune::wav(&s, gp_present::sound::ROLE_RATE),
                ));
            }
            Err(e) => notes.push(format!("sound role `{}`: {e}", r.name)),
        }
    }
    out
}

/// The tests while which the game makes no sound (model feature
/// `present.sound_silent_while`), each counter by its index in the spec.
pub fn silent_cues(
    snd: &gp_model::model::Sound,
    spec: &Spec,
    notes: &mut Vec<String>,
) -> Vec<gp_present::sound::RoleCue> {
    let mut out = Vec::new();
    for t in &snd.silent_while {
        let Some(counter) = spec.counters.iter().position(|c| c.name == t.counter) else {
            notes.push(format!(
                "silent_while: counter `{}` isn't in the compiled spec",
                t.counter
            ));
            continue;
        };
        out.push(gp_present::sound::RoleCue {
            name: String::new(),
            music: false,
            counter,
            mask: t.mask.unwrap_or(-1),
            equals: t.equals,
            held: false,
            loops: false,
        });
    }
    out
}

/// The sound roles' cues (model feature `present.sound_roles`), each
/// counter by its index in the compiled spec.
pub fn role_cues(
    snd: &gp_model::model::Sound,
    spec: &Spec,
    notes: &mut Vec<String>,
) -> Vec<gp_present::sound::RoleCue> {
    let mut out = Vec::new();
    for r in &snd.roles {
        let Some(counter) = spec.counters.iter().position(|c| c.name == r.counter) else {
            notes.push(format!(
                "sound role `{}`: counter `{}` isn't in the compiled spec",
                r.name, r.counter
            ));
            continue;
        };
        out.push(gp_present::sound::RoleCue {
            name: r.name.clone(),
            music: r.kind == gp_model::model::SoundRoleKind::Music,
            counter,
            mask: r.mask.unwrap_or(-1),
            equals: r.equals,
            held: r.held,
            loops: r.loops,
        });
    }
    out
}
