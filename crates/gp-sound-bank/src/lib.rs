//! Songs rendered through sound banks (docs/formats/sound-banks.md).
//!
//! A mapping sends each instrument to the APU synthesizer, a General MIDI
//! program or a bank's patch. This crate renders a song the way the mapping
//! says: channels whose instruments play through an SF2 bank go to
//! rustysynth (MIT; a pure Rust SoundFont synthesizer) as MIDI events, a
//! note on at each note's first frame and a note off after its last; the
//! rest play on the APU as always; the two are mixed. SFZ banks are listed
//! and mapped, but not rendered yet (docs/open-questions.md, M2.8): their
//! channels fall back to the APU, which the result says.
//!
//! Rendering is presentation, done ahead of play (the Bevy generator writes
//! WAV files) or on request (`gp sound render`, the MCP preview).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gp_sound::apu::Channel;
use gp_sound::banks::{BankKind, Mapping, Target};
use gp_sound::song::SongSet;
use gp_sound::synth::SAMPLE_RATE;
use rustysynth::{SoundFont, Synthesizer, SynthesizerSettings};

/// A channel's bank target: bank, preset, program, bank number, gain and
/// transpose.
type BankTarget = (String, Option<String>, Option<u8>, Option<u16>, f64, i8);

/// What a render did, channel by channel.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Rendered {
    pub samples: Vec<i16>,
    /// Channels played through a bank: channel, bank, preset name.
    pub banked: Vec<(Channel, String, String)>,
    /// Why a channel mapped to a bank played on the APU instead.
    pub fallbacks: Vec<String>,
}

/// A bank file's path from a mapping's `path`: absolute, or in one of the
/// folders.
pub fn locate(path: &str, folders: &[PathBuf]) -> Option<PathBuf> {
    let p = Path::new(path);
    if p.is_absolute() {
        return p.is_file().then(|| p.to_path_buf());
    }
    folders.iter().map(|d| d.join(p)).find(|x| x.is_file())
}

fn load_sf2(path: &Path) -> Result<Arc<SoundFont>, String> {
    let mut f = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    SoundFont::new(&mut f)
        .map(Arc::new)
        .map_err(|e| format!("{}: {e:?}", path.display()))
}

/// The bank and program of a target in a SoundFont: by preset name, or as
/// given.
fn preset_of(
    sf: &SoundFont,
    preset: Option<&str>,
    program: Option<u8>,
    bank: Option<u16>,
) -> Option<(i32, i32, String)> {
    let all = sf.get_presets();
    if let Some(name) = preset {
        return all
            .iter()
            .find(|p| p.get_name().eq_ignore_ascii_case(name))
            .map(|p| {
                (
                    p.get_bank_number(),
                    p.get_patch_number(),
                    p.get_name().to_owned(),
                )
            });
    }
    let program = i32::from(program?);
    let bank = i32::from(bank.unwrap_or(0));
    all.iter()
        .find(|p| p.get_patch_number() == program && p.get_bank_number() == bank)
        .map(|p| (bank, program, p.get_name().to_owned()))
}

/// Renders `song` (its first pass and `loops` more) through the mapping.
pub fn render(
    set: &SongSet,
    song: &str,
    map: &Mapping,
    folders: &[PathBuf],
    loops: u32,
) -> Result<Rendered, String> {
    let s = set.song(song).ok_or_else(|| format!("no song `{song}`"))?;
    let lf = s.loop_frame.unwrap_or(0);
    let frames = s.frames + u64::from(loops) * (s.frames - lf);
    let mut out = Rendered::default();
    // Which channels go to which bank target: the target most of the
    // channel's notes' instruments map to.
    let mut bank_of: BTreeMap<Channel, BankTarget> = BTreeMap::new();
    for t in &s.tracks {
        let mut votes: BTreeMap<String, (usize, BankTarget)> = BTreeMap::new();
        let placed = t.placed(s.tempo);
        for p in &placed {
            let ins = &set.instruments[usize::from(p.note.instrument)];
            if let Some(r) = map.rule_for(song, ins)
                && let Target::Bank {
                    bank,
                    preset,
                    program,
                    bank_number,
                } = &r.target
            {
                let key = format!("{bank}/{preset:?}/{program:?}");
                let e = votes.entry(key).or_insert((
                    0,
                    (
                        bank.clone(),
                        preset.clone(),
                        *program,
                        *bank_number,
                        r.gain.unwrap_or(1.0),
                        r.transpose.unwrap_or(0),
                    ),
                ));
                e.0 += 1;
            }
        }
        if let Some((n, v)) = votes.into_values().max_by_key(|x| x.0)
            && n * 2 >= placed.len().max(1)
        {
            bank_of.insert(t.channel, v);
        }
    }
    // Load the banks, falling back to the APU where one can't play.
    let mut fonts: BTreeMap<String, Arc<SoundFont>> = BTreeMap::new();
    let mut ready: BTreeMap<Channel, (Arc<SoundFont>, i32, i32, f64, i8)> = BTreeMap::new();
    for (c, (bank, preset, program, bank_number, gain, transpose)) in &bank_of {
        let Some(b) = map.banks.iter().find(|b| &b.name == bank) else {
            continue;
        };
        if b.kind != BankKind::Sf2 {
            out.fallbacks.push(format!(
                "{}: bank `{bank}` is {:?}; only SF2 banks render so far",
                c.name(),
                b.kind
            ));
            continue;
        }
        let Some(path) = locate(&b.path, folders) else {
            out.fallbacks.push(format!(
                "{}: bank file `{}` isn't in the bank folders",
                c.name(),
                b.path
            ));
            continue;
        };
        let sf = match fonts.get(bank) {
            Some(f) => f.clone(),
            None => {
                let f = load_sf2(&path)?;
                fonts.insert(bank.clone(), f.clone());
                f
            }
        };
        match preset_of(&sf, preset.as_deref(), *program, *bank_number) {
            Some((bk, pg, name)) => {
                out.banked.push((*c, bank.clone(), name));
                ready.insert(*c, (sf, bk, pg, *gain, *transpose));
            }
            None => out.fallbacks.push(format!(
                "{}: no preset {preset:?} / program {program:?} in `{bank}`",
                c.name()
            )),
        }
    }
    // The APU part, without the banked channels.
    let mut gains = map.mix.gains();
    for c in ready.keys() {
        gains[c.index()] = 0.0;
    }
    let mut player = gp_sound::player::Player::new(set.clone());
    player.play_music(Some(song));
    let mut synth = gp_sound::synth::Synth::new(set.region, SAMPLE_RATE, set.samples.clone());
    synth.gains = gains;
    let mut apu = Vec::new();
    for _ in 0..frames {
        let w = player.frame();
        apu.extend(synth.frame(&w));
    }
    // The bank part: one synthesizer per font, a MIDI channel per NES
    // channel (noise and DMC on the percussion channel).
    let mut mixed: Vec<f32> = apu.iter().map(|&x| f32::from(x)).collect();
    let mut by_font: BTreeMap<String, Vec<Channel>> = BTreeMap::new();
    for c in ready.keys() {
        let b = &bank_of[c].0;
        by_font.entry(b.clone()).or_default().push(*c);
    }
    for (font, chans) in by_font {
        let sf = fonts[&font].clone();
        let settings = SynthesizerSettings::new(SAMPLE_RATE as i32);
        let mut syn = Synthesizer::new(&sf, &settings).map_err(|e| format!("{font}: {e:?}"))?;
        // Note events by frame: (frame, on, midi channel, key, velocity).
        let mut ev: Vec<(u64, bool, i32, i32, i32)> = Vec::new();
        for c in &chans {
            let (_, bk, pg, _, tr) = &ready[c];
            let mch = match c {
                Channel::Noise | Channel::Dmc => 9,
                _ => c.index() as i32,
            };
            syn.process_midi_message(mch, 0xB0, 0, *bk);
            syn.process_midi_message(mch, 0xC0, *pg, 0);
            let t = s.track(*c).expect("a banked channel has a track");
            let mut passes = vec![(0u64, 0u64)];
            for k in 1..=u64::from(loops) {
                passes.push((k, s.frames + (k - 1) * (s.frames - lf)));
            }
            for p in t.placed(s.tempo) {
                for &(k, base) in &passes {
                    if k > 0 && p.start < lf {
                        continue;
                    }
                    let at = if k == 0 { p.start } else { base + p.start - lf };
                    let ins = &set.instruments[usize::from(p.note.instrument)];
                    let key = match c {
                        Channel::Noise => i32::from(gp_sound::banks::drum_key(
                            p.note.pitch,
                            p.note.len,
                            ins.duty_at(0),
                        )),
                        Channel::Dmc => 35,
                        _ => i32::from(p.note.pitch) + i32::from(*tr),
                    };
                    let vel = i32::from(ins.volume_at(0).min(15)) * 8 + 7;
                    ev.push((at, true, mch, key.clamp(0, 127), vel));
                    ev.push((
                        at + u64::from(p.note.len.max(1)),
                        false,
                        mch,
                        key.clamp(0, 127),
                        0,
                    ));
                }
            }
        }
        ev.sort_by_key(|e| (e.0, e.1));
        let gain = chans.iter().map(|c| ready[c].3).fold(0.0, f64::max) as f32;
        let mut e = ev.iter().peekable();
        let mut left = vec![0f32; 1024];
        let mut right = vec![0f32; 1024];
        let mut pos = 0usize;
        for f in 0..frames {
            while let Some(x) = e.peek() {
                if x.0 > f {
                    break;
                }
                if x.1 {
                    syn.note_on(x.2, x.3, x.4);
                } else {
                    syn.note_off(x.2, x.3);
                }
                e.next();
            }
            let n = gp_sound::frames_of(set.region, &apu, f, f + 1).len();
            if left.len() < n {
                left.resize(n, 0.0);
                right.resize(n, 0.0);
            }
            syn.render(&mut left[..n], &mut right[..n]);
            for i in 0..n {
                if let Some(m) = mixed.get_mut(pos + i) {
                    *m += (left[i] + right[i]) * 0.5 * 20_000.0 * gain;
                }
            }
            pos += n;
        }
    }
    out.samples = mixed
        .into_iter()
        .map(|x| x.round().clamp(-32_768.0, 32_767.0) as i16)
        .collect();
    Ok(out)
}

/// A one-second preview of a bank's patch: a C major arpeggio.
pub fn preview(path: &Path, bank: u16, program: u8) -> Result<Vec<i16>, String> {
    let sf = load_sf2(path)?;
    let settings = SynthesizerSettings::new(SAMPLE_RATE as i32);
    let mut syn = Synthesizer::new(&sf, &settings).map_err(|e| format!("{e:?}"))?;
    syn.process_midi_message(0, 0xB0, 0, i32::from(bank));
    syn.process_midi_message(0, 0xC0, i32::from(program), 0);
    let step = SAMPLE_RATE as usize / 4;
    let mut out = Vec::with_capacity(SAMPLE_RATE as usize);
    let mut l = vec![0f32; step];
    let mut r = vec![0f32; step];
    for key in [60, 64, 67, 72] {
        syn.note_on(0, key, 100);
        syn.render(&mut l, &mut r);
        syn.note_off(0, key);
        out.extend(l.iter().zip(&r).map(|(a, b)| {
            (((a + b) * 0.5 * 20_000.0)
                .round()
                .clamp(-32_768.0, 32_767.0)) as i16
        }));
    }
    Ok(out)
}

/// A minimal SF2 file with one preset of a looped square wave, for tests
/// (made up here; no bank's data).
pub fn test_sf2(name: &str, program: u16) -> Vec<u8> {
    fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = id.to_vec();
        v.extend_from_slice(&(body.len() as u32).to_le_bytes());
        v.extend_from_slice(body);
        if body.len() % 2 == 1 {
            v.push(0);
        }
        v
    }
    fn list(kind: &[u8; 4], parts: &[Vec<u8>]) -> Vec<u8> {
        let mut body = kind.to_vec();
        for p in parts {
            body.extend_from_slice(p);
        }
        chunk(b"LIST", &body)
    }
    fn name20(s: &str) -> Vec<u8> {
        let mut v = s.as_bytes().to_vec();
        v.resize(20, 0);
        v
    }
    let period = 100usize;
    let mut smpl = Vec::new();
    for i in 0..period * 4 {
        let v: i16 = if (i % period) < period / 2 {
            8000
        } else {
            -8000
        };
        smpl.extend_from_slice(&v.to_le_bytes());
    }
    smpl.extend(std::iter::repeat_n(0u8, 46 * 2));
    let info = list(
        b"INFO",
        &[
            chunk(b"ifil", &[2, 0, 1, 0]),
            chunk(b"isng", b"EMU8000\0"),
            chunk(b"INAM", format!("{name}\0").as_bytes()),
        ],
    );
    let sdta = list(b"sdta", &[chunk(b"smpl", &smpl)]);
    let mut phdr = Vec::new();
    for (n, prog, bag) in [(name, program, 0u16), ("EOP", 0, 1)] {
        phdr.extend(name20(n));
        phdr.extend_from_slice(&prog.to_le_bytes());
        phdr.extend_from_slice(&0u16.to_le_bytes());
        phdr.extend_from_slice(&bag.to_le_bytes());
        phdr.extend_from_slice(&[0; 12]);
    }
    let bags = |a: u16, b: u16| {
        [
            a.to_le_bytes(),
            0u16.to_le_bytes(),
            b.to_le_bytes(),
            0u16.to_le_bytes(),
        ]
        .concat()
    };
    let gen_rec = |op: u16, amount: u16| [op.to_le_bytes(), amount.to_le_bytes()].concat();
    let pgen = [gen_rec(41, 0), gen_rec(0, 0)].concat();
    let mut inst = name20("square");
    inst.extend_from_slice(&0u16.to_le_bytes());
    inst.extend(name20("EOI"));
    inst.extend_from_slice(&1u16.to_le_bytes());
    let igen = [gen_rec(54, 1), gen_rec(53, 0), gen_rec(0, 0)].concat();
    let mut shdr = name20("square");
    for v in [0u32, (period * 4) as u32, 0, (period * 4) as u32, 44_100] {
        shdr.extend_from_slice(&v.to_le_bytes());
    }
    shdr.extend_from_slice(&[60, 0]);
    shdr.extend_from_slice(&0u16.to_le_bytes());
    shdr.extend_from_slice(&1u16.to_le_bytes());
    shdr.extend(name20("EOS"));
    shdr.extend_from_slice(&[0; 26]);
    let pdta = list(
        b"pdta",
        &[
            chunk(b"phdr", &phdr),
            chunk(b"pbag", &bags(0, 1)),
            chunk(b"pmod", &[0; 10]),
            chunk(b"pgen", &pgen),
            chunk(b"inst", &inst),
            chunk(b"ibag", &bags(0, 2)),
            chunk(b"imod", &[0; 10]),
            chunk(b"igen", &igen),
            chunk(b"shdr", &shdr),
        ],
    );
    let mut body = b"sfbk".to_vec();
    body.extend(info);
    body.extend(sdta);
    body.extend(pdta);
    chunk(b"RIFF", &body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gp_sound::banks::{Bank, Rule};
    use gp_sound::song::{Instrument, Kind, Note, Origin, Placed, Song, Tempo, Track, Voice};

    fn set() -> SongSet {
        let mut set = SongSet::new(gp_sound::apu::Region::Ntsc);
        for (name, voice, role) in [
            ("lead", Voice::Pulse, "lead"),
            ("bass", Voice::Triangle, "bass"),
        ] {
            set.instruments.push(Instrument {
                name: name.into(),
                voice,
                role: Some(role.into()),
                volume: vec![12],
                duty: vec![],
                pitch: vec![],
                sample: None,
                notes: 0,
            });
        }
        let tempo = Tempo {
            frames_per_row: 8,
            rows_per_pattern: 4,
        };
        let note = |start: u64, pitch: u8, i: u16| Placed {
            start,
            note: Note {
                at: 0,
                len: 8,
                pitch,
                fine: 0,
                instrument: i,
            },
        };
        set.songs.push(Song {
            name: "tune".into(),
            kind: Kind::Music,
            origin: Origin::Edited,
            confidence: 1.0,
            driver: None,
            frames: 32,
            loop_frame: Some(0),
            tempo,
            tracks: vec![
                Track::from_placed(
                    Channel::Pulse1,
                    &[note(0, 60, 0), note(8, 64, 0), note(16, 67, 0)],
                    tempo,
                    32,
                ),
                Track::from_placed(Channel::Triangle, &[note(0, 48, 1)], tempo, 32),
            ],
            request: None,
            event: None,
            level: None,
            state: None,
            captured: None,
            notes: vec![],
        });
        set
    }

    #[test]
    fn a_role_plays_through_an_sf2_patch_and_the_rest_on_the_apu() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        std::fs::write(dir.join("made-up.sf2"), test_sf2("Square Lead", 80)).unwrap();
        let found = gp_sound::banks::discover(std::slice::from_ref(&dir));
        assert_eq!(found[1].patches[0].name, "Square Lead");
        let mut map = Mapping::default();
        map.banks.push(Bank {
            name: "made-up".into(),
            kind: BankKind::Sf2,
            path: "made-up.sf2".into(),
            license: Some("CC0-1.0".into()),
        });
        map.rules.push(Rule {
            instrument: None,
            role: Some("lead".into()),
            song: None,
            target: Target::Bank {
                bank: "made-up".into(),
                preset: Some("Square Lead".into()),
                program: None,
                bank_number: None,
            },
            gain: None,
            transpose: None,
        });
        let set = set();
        let r = render(&set, "tune", &map, std::slice::from_ref(&dir), 1).unwrap();
        assert_eq!(
            r.banked,
            vec![(
                Channel::Pulse1,
                "made-up".to_string(),
                "Square Lead".to_string()
            )]
        );
        assert!(r.fallbacks.is_empty(), "{:?}", r.fallbacks);
        assert!(r.samples.iter().any(|&x| x.unsigned_abs() > 500));
        // Deterministic, and different from the APU alone.
        let again = render(&set, "tune", &map, std::slice::from_ref(&dir), 1).unwrap();
        assert_eq!(r.samples, again.samples);
        let apu = render(&set, "tune", &Mapping::default(), &[], 1).unwrap();
        assert_ne!(apu.samples, r.samples);
        assert!(!preview(&dir.join("made-up.sf2"), 0, 80).unwrap().is_empty());
    }
}
