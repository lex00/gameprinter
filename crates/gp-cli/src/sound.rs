//! `gp sound`: music and sound effects as model data (docs/sound.md).
//!
//! `capture` writes a recording's APU register log (observe-only: from its
//! event stream, or a replay with the event log on) and derives songs,
//! effects and instruments from it, with the game's sound requests from
//! `gp gaps` when a template is given; `fidelity` holds songs to their logs;
//! `compare` holds one register log to another. Everything a capture writes
//! is game content and goes under `$GP_HOME/work/sound/`, never the repo.
//!
//! These are the NES APU's: its register log format, channels and synth.
//! An arcade recording (Tapper's two AY-8910s, port wave 6) is captured,
//! held to its notes and compared through crate::sound_ay instead: an
//! `ay.log` from the recording's `ay-write` events, and notes per
//! channel. `decode` reads songs with a registered driver's decoder
//! (gp_sound::drivers); Tapper's driver decodes the AY log. `render`
//! writes WAV files and `banks` lists the sound banks found; AY logs and
//! songs render through the arcade board's sound (crate::sound_ay_render).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use gp_evidence::Paths;
use gp_sound::apu::{Channel, Region};
use gp_sound::banks::Mapping;
use gp_sound::capture::{self, Claim, Req};
use gp_sound::log::{ApuLog, Write as LogWrite, cycle_in_frame};
use gp_sound::song::{Kind, Request, SongSet};
use serde_json::json;

use crate::Outcome;
use crate::open::Opened;
use crate::recordings::parse_frames;

#[derive(Subcommand)]
pub enum SoundCmd {
    /// Write a recording's APU register log and derive its songs, effects
    /// and instruments (docs/formats/apu-log.md, docs/formats/songs.md).
    Capture(CaptureArgs),
    /// Read songs from the game's music data with a registered driver's
    /// decoder, and cross-check them against a capture's songs. On arcade
    /// the driver's decoder reads an AY register log (`--ay-log`).
    Decode(DecodeArgs),
    /// Hold songs to the register log they came from: note onsets, pitch
    /// and volume per frame, and the two renders' loudness.
    Fidelity(FidelityArgs),
    /// Hold a register log to the original's, frame by frame: the frames
    /// whose writes are the same, and those whose every channel's state is
    /// (gp_sound::compare). `gp interp compare` and `gp clip score` measure
    /// the same in their lockstep. Exits 1 when a frame's channels differ.
    Compare(CompareArgs),
    /// Render a song, or a register log, to a WAV file through the APU
    /// synthesizer (or a sound bank mapping). An AY register log or AY
    /// songs play through the arcade board's sound (gp_arcade::sound).
    Render(RenderArgs),
    /// List the sound banks. The APU synthesizer comes first, then the SF2 and
    /// SFZ files found in $GP_SOUND_BANKS and $GP_HOME/sound-banks.
    Banks,
    /// The sound drivers the registry knows.
    Drivers,
}

#[derive(Args)]
pub struct CaptureArgs {
    /// The recording's ID, or a unique prefix of at least 4 characters.
    #[arg(long)]
    pub recording: String,
    /// Which frames: `N`, `A..B` (B excluded), `A..=B` or `A..`.
    #[arg(long, default_value = "1..")]
    pub frames: String,
    /// A name for the capture (and the song when there's no music request).
    #[arg(long)]
    pub name: String,
    /// The template whose model the songs bind to; with it, the game's
    /// sound requests (gp gaps, which replays with the code observer)
    /// split music from effects.
    #[arg(long)]
    pub template: Option<String>,
    /// The model level the captured music plays in.
    #[arg(long)]
    pub level: Option<String>,
    /// An ld65 debug file naming the routines (as for gp gaps), for a
    /// game that asks for sound through a play-sound routine.
    #[arg(long)]
    pub listing: Option<PathBuf>,
    /// The folder holding the templates (default: `templates`, then the
    /// games checkout, $GP_GAMES_DIR).
    #[arg(long)]
    pub dir: Option<PathBuf>,
    /// Write here instead of $GP_HOME/work/sound/<recording>/.
    #[arg(long)]
    pub out: Option<PathBuf>,
    /// The ROM, when the recording has no event stream and it isn't in
    /// $GP_ROM_DIR.
    #[arg(long)]
    pub rom: Option<PathBuf>,
}

#[derive(Args)]
pub struct DecodeArgs {
    /// The source ID, whose asset map names the music asset and its format.
    #[arg(long)]
    pub source: String,
    /// The lifted assets (default $GP_HOME/work/assets/<source>).
    #[arg(long)]
    pub assets: Option<PathBuf>,
    /// The asset maps' folder (default: $GP_ASSETS, else the tool's
    /// assets/ and each game's assets/ in the games checkout).
    #[arg(long)]
    pub maps: Option<PathBuf>,
    /// The songs to decode: `name=slot,slot,...[@loop index]`, e.g.
    /// `ground=16-48@1`. On an AY log, `name=FIRST-LAST[@LOOP]` in
    /// frames, the loop counted from FIRST. Default: every slot as its own
    /// song (on an AY log, each run of sound between silences).
    #[arg(long)]
    pub song: Vec<String>,
    /// A capture's songs to cross-check against (`gp sound capture`'s
    /// songs.json).
    #[arg(long)]
    pub against: Option<PathBuf>,
    /// An AY register log (`gp sound capture`'s ay.log, or its folder),
    /// for a driver that decodes one (midway-ssio, Tapper's).
    #[arg(long)]
    pub ay_log: Option<PathBuf>,
    /// On an AY log, the frames of silence on all six channels that end a
    /// song.
    #[arg(long, default_value_t = 60)]
    pub gap: u64,
    #[arg(long)]
    pub out: Option<PathBuf>,
}

#[derive(Args)]
pub struct RenderArgs {
    /// A song set (songs.json) or a register log (apu.log, or an AY
    /// log's ay.log).
    pub input: PathBuf,
    /// The song to render (from a song set).
    #[arg(long)]
    pub song: Option<String>,
    /// Loops of the song to render after the first pass.
    #[arg(long, default_value_t = 0)]
    pub loops: u32,
    /// A sound bank mapping (docs/formats/sound-banks.md): its mix applies;
    /// bank targets need gp-sound-bank's renderer.
    #[arg(long)]
    pub banks: Option<PathBuf>,
    /// For AY sound, the romset whose SSIO PROM sets the voices' volumes
    /// (default: tapper's from $GP_ROM_DIR; without one every voice plays
    /// at full volume).
    #[arg(long)]
    pub rom: Option<PathBuf>,
    #[arg(long)]
    pub out: PathBuf,
}

#[derive(Args)]
pub struct FidelityArgs {
    /// A capture's directory (holding apu.log, songs.json, claims.json).
    pub capture: PathBuf,
}

#[derive(Args)]
pub struct CompareArgs {
    /// The original's register log (`gp sound capture`'s apu.log).
    pub original: PathBuf,
    /// The log held to it, such as a model's (`gp interp run --apu-log`).
    pub model: PathBuf,
    /// The original's frame `f` is the model's `f - offset`, as a
    /// lockstep's pairing says ("model frame n is the original's n+3":
    /// 3).
    #[arg(long, default_value_t = 0, allow_hyphen_values = true)]
    pub offset: i64,
}

pub(crate) fn run(cmd: &SoundCmd, paths: &Paths) -> Result<Outcome> {
    match cmd {
        SoundCmd::Capture(a) => capture_cmd(a, paths),
        SoundCmd::Fidelity(a) => fidelity_cmd(&a.capture),
        SoundCmd::Compare(a) => compare_cmd(a),
        SoundCmd::Decode(a) => decode_cmd(a, paths),
        SoundCmd::Drivers => drivers_cmd(),
        SoundCmd::Render(a) => render_cmd(a, &paths.home),
        SoundCmd::Banks => banks_cmd(paths),
    }
}

fn read(p: &Path) -> Result<String> {
    std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))
}

fn write(p: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).with_context(|| d.display().to_string())?;
    }
    std::fs::write(p, bytes).with_context(|| format!("writing {}", p.display()))
}

/// The register log of frames `range` of a recording.
pub(crate) fn apu_log(
    opened: &Opened,
    range: std::ops::Range<u64>,
    rom: Option<&Path>,
    paths: &Paths,
) -> Result<(ApuLog, &'static str)> {
    use gp_evidence::events::EventKind;
    let region = match opened.manifest.power_on.region {
        gp_evidence::manifest::Region::Pal => Region::Pal,
        _ => Region::Ntsc,
    };
    let (events, from) =
        match gp_nes::recording::stored_events(&opened.store, &opened.manifest, range.clone())? {
            Some(e) => (e, "event stream"),
            None => {
                let rom = opened.rom(rom, paths)?.bytes;
                (
                    gp_nes::recording::replayed_events(
                        &opened.store,
                        &opened.manifest,
                        &rom,
                        range.clone(),
                    )?,
                    "replay",
                )
            }
        };
    let mut writes: Vec<LogWrite> = events
        .iter()
        .filter(|e| e.kind == EventKind::ApuWrite)
        .filter(|e| (0x4000..=0x4017).contains(&e.addr) && e.addr != 0x4014 && e.addr != 0x4016)
        .map(|e| LogWrite {
            frame: e.frame,
            cycle: cycle_in_frame(region, e.scanline, e.dot),
            reg: (e.addr - 0x4000) as u8,
            value: e.value,
        })
        .collect();
    // Keep the console's order; a cycle that seems to go back (an event
    // at the frame's edge) takes the one before it.
    for i in 1..writes.len() {
        if writes[i].frame == writes[i - 1].frame && writes[i].cycle < writes[i - 1].cycle {
            writes[i].cycle = writes[i - 1].cycle;
        }
    }
    Ok((
        ApuLog {
            region,
            source: Some(opened.sha256().to_owned()),
            recording: Some(opened.entry.id.to_hex()),
            first: range.start,
            end: range.end,
            timed: true,
            writes,
            samples: Vec::new(),
        },
        from,
    ))
}

/// The events and rule states a template's model can name.
fn model_names(t: &crate::gaps::Template) -> (Vec<String>, Vec<String>, Vec<String>) {
    let m = &t.model;
    let mut events: Vec<String> = gp_model::validate::CORE_EVENTS
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    if m.model.extensions.contains_key(gp_model::platformer::NAME) {
        events.extend(
            gp_model::platformer::EXTENSION
                .events
                .iter()
                .map(|s| (*s).to_owned()),
        );
    }
    events.extend(m.model.events.iter().cloned());
    let states: Vec<String> = m
        .rules
        .iter()
        .flat_map(|(r, mach)| mach.states.keys().map(move |s| format!("{r}.{s}")))
        .collect();
    let levels: Vec<String> = m.levels.keys().cloned().collect();
    (events, states, levels)
}

/// `gp sound capture` of an arcade recording: its AY register log and
/// notes (crate::sound_ay).
fn capture_ay(a: &CaptureArgs, opened: &Opened, paths: &Paths) -> Result<Outcome> {
    if a.template.is_some() {
        bail!(
            "--template binds songs to a model through gp gaps' NES sound requests; an arcade capture writes the AY log and its notes only"
        );
    }
    let range = parse_frames(&a.frames, opened.manifest.frames)?;
    let range = range.start.max(1)..range.end.min(opened.manifest.frames + 1);
    if range.is_empty() {
        bail!("--frames {} leaves no frame", a.frames);
    }
    let (events, from) =
        match crate::arcade_rec::stored_events(&opened.store, &opened.manifest, range.clone())? {
            Some(e) => (e, "event stream"),
            None => {
                let set = opened.romset(a.rom.as_deref(), paths)?;
                (
                    crate::arcade_rec::replayed_events(
                        &opened.store,
                        &opened.manifest,
                        &set,
                        range.clone(),
                    )?,
                    "replay",
                )
            }
        };
    let log = crate::sound_ay::AyLog::of_events(&events, range.start, range.end - 1);
    let songs = crate::sound_ay::notes(&log);
    let name = capture::slug(&a.name);
    let dir = a.out.clone().unwrap_or_else(|| {
        paths
            .home
            .join("work/sound")
            .join(&opened.entry.id.to_hex()[..12])
            .join(&name)
    });
    write(&dir.join("ay.log"), log.text().as_bytes())?;
    write(
        &dir.join("songs.json"),
        serde_json::to_string_pretty(&songs)?.as_bytes(),
    )?;
    let writes: usize = log.frames.values().map(Vec::len).sum();
    let notes: Vec<String> = songs["channels"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| {
            format!(
                "{} {}",
                c["channel"].as_str().unwrap_or("?"),
                c["notes"].as_array().map_or(0, Vec::len)
            )
        })
        .collect();
    Ok(Outcome::ok(
        json!({"dir": dir, "from": from, "writes": writes, "frames": [range.start, range.end - 1], "songs": songs}),
        format!(
            "{} AY writes in frames {} to {} of {} (from the {from}); notes by channel: {}\nwritten to {}",
            writes,
            range.start,
            range.end - 1,
            opened.short(),
            notes.join(", "),
            dir.display()
        ),
    ))
}

fn capture_cmd(a: &CaptureArgs, paths: &Paths) -> Result<Outcome> {
    let opened = Opened::open(paths, &a.recording)?;
    if opened.is_arcade() {
        return capture_ay(a, &opened, paths);
    }
    crate::track::nes_recording(&opened, "gp sound capture")?;
    let range = parse_frames(&a.frames, opened.manifest.frames)?;
    let range = range.start.max(1)..range.end.min(opened.manifest.frames + 1);
    if range.is_empty() {
        bail!("--frames {} leaves no frame", a.frames);
    }
    let (log, from) = apu_log(&opened, range.clone(), a.rom.as_deref(), paths)?;
    let name = capture::slug(&a.name);
    let dir = a.out.clone().unwrap_or_else(|| {
        paths
            .home
            .join("work/sound")
            .join(&opened.entry.id.to_hex()[..12])
            .join(&name)
    });
    write(&dir.join("apu.log"), log.to_text().as_bytes())?;

    // Requests, from gp gaps' detector, when there's a template.
    let mut reqs: Vec<Req> = Vec::new();
    let mut names = (Vec::new(), Vec::new(), Vec::new());
    // Frames the player is in control (the mapping's start condition).
    let mut play: std::collections::BTreeSet<u64> = Default::default();
    let mut notes = Vec::new();
    if let Some(tn) = &a.template {
        let t = crate::gaps::Template::open(a.dir.as_deref(), tn)?;
        names = model_names(&t);
        // The detector reads the whole recording: an interface is known by
        // how many places use it, which one level may not show.
        let ga = crate::gaps::GapsArgs::for_sound(
            tn,
            &a.recording,
            "1..",
            a.dir.as_deref(),
            a.listing.clone(),
        );
        match crate::gaps::build_run(&ga, &t, paths) {
            Ok((run, why)) => {
                if !why.is_empty() {
                    notes.push(format!("no code signals: {why}"));
                }
                play = run
                    .frames
                    .iter()
                    .filter(|f| f.play)
                    .map(|f| f.frame)
                    .collect();
                let (_, found) = gp_gaps::sound::detect(&run);
                for r in found {
                    if r.frame < range.start || r.frame >= range.end {
                        continue;
                    }
                    let kind = if capture::is_music(&r.via, &r.name) {
                        Kind::Music
                    } else {
                        Kind::Sfx
                    };
                    reqs.push(Req {
                        frame: r.frame - range.start,
                        request: Request {
                            via: r.via,
                            id: r.id,
                            name: r.name,
                        },
                        kind,
                    });
                }
            }
            Err(e) => notes.push(format!("no sound requests: {e:#}")),
        }
    }
    let states = log.states();
    let claims = capture::claims(&states, &reqs);
    let mut set = capture::capture(
        log.region,
        &states,
        &reqs,
        &capture::Options {
            name: a.name.clone(),
            recording: Some(opened.entry.id.to_hex()),
            first_frame: range.start,
            level: a.level.clone(),
        },
    );
    // The level goes to the music that plays while the player is in
    // control most (the longest, without a template); the rest is event
    // or menu music.
    let in_play = |s: &gp_sound::song::Song| -> u64 {
        let Some(c) = &s.captured else { return 0 };
        if play.is_empty() {
            return s.frames;
        }
        play.range(c.frames[0]..c.frames[1]).count() as u64
    };
    let main = set
        .songs
        .iter()
        .filter(|s| s.kind == Kind::Music)
        .max_by_key(|s| (in_play(s), s.frames))
        .map(|s| s.name.clone());
    for s in &mut set.songs {
        if s.kind == Kind::Music && Some(&s.name) != main.as_ref() {
            s.level = None;
        }
    }
    // Bind effects to events and music to rule states by name.
    let (events, rstates, levels) = &names;
    if let Some(l) = &a.level
        && !levels.is_empty()
        && !levels.contains(l)
    {
        notes.push(format!("level `{l}` isn't one of the model's levels"));
    }
    for s in &mut set.songs {
        let n = s
            .request
            .as_ref()
            .map_or(s.name.clone(), |r| r.name.clone());
        match s.kind {
            Kind::Sfx => s.event = capture::bind(&n, events),
            Kind::Music => {
                if let Some(st) = capture::bind(&n, rstates) {
                    s.state = Some(st);
                    s.level = None;
                }
            }
        }
    }
    write(&dir.join("songs.json"), set.to_text().as_bytes())?;
    let claims_json: Vec<_> = claims
        .iter()
        .map(|c| json!({"request": c.request, "channel": c.channel.name(), "from": c.from, "to": c.to}))
        .collect();
    let reqs_json: Vec<_> = reqs
        .iter()
        .map(|r| json!({"frame": r.frame, "via": r.request.via, "id": r.request.id, "name": r.request.name, "kind": r.kind}))
        .collect();
    write(
        &dir.join("claims.json"),
        serde_json::to_string_pretty(&json!({"requests": reqs_json, "claims": claims_json}))?
            .as_bytes(),
    )?;
    let fid = fidelity_of(&dir, &log, &set, &claims)?;
    let music = set.songs.iter().filter(|s| s.kind == Kind::Music).count();
    let sfx = set.songs.len() - music;
    let bound = set
        .songs
        .iter()
        .filter(|s| s.event.is_some() || s.state.is_some() || s.level.is_some())
        .count();
    let mut text = format!(
        "{} frames {}..{} from the {from}: {} APU writes; {} songs, {} effects, {} instruments, {} requests ({} bound to the model)\nwrote {}",
        opened.short(),
        range.start,
        range.end,
        log.writes.len(),
        music,
        sfx,
        set.instruments.len(),
        reqs.len(),
        bound,
        dir.display()
    );
    for s in &set.songs {
        let ag = &fid["songs"][&s.name];
        let _ = write!(
            text,
            "\n  {:<5} {:<28} {:>5} frames{} {:>4} notes, {:.0}% sure, onset F1 {}, pitch {}, loudness {}{}",
            match s.kind {
                Kind::Music => "music",
                Kind::Sfx => "sfx",
            },
            s.name,
            s.frames,
            s.loop_frame
                .map_or(String::new(), |l| format!(" (loop at {l})")),
            s.note_count(),
            s.confidence * 100.0,
            ag["f1"],
            ag["pitch_frames"],
            ag["loudness"],
            s.event
                .as_ref()
                .or(s.state.as_ref())
                .or(s.level.as_ref())
                .map_or(String::new(), |b| format!(" -> {b}"))
        );
    }
    for n in &notes {
        let _ = write!(text, "\nnote: {n}");
    }
    Ok(Outcome::ok(
        json!({"dir": dir, "frames": [range.start, range.end], "from": from, "writes": log.writes.len(),
               "songs": music, "sfx": sfx, "instruments": set.instruments.len(), "requests": reqs.len(),
               "bound": bound, "fidelity": fid, "notes": notes}),
        text,
    ))
}

/// Each song against the log over its frames, written as fidelity.json:
/// music with its effects' channels left out, an effect on its own
/// channels only.
fn fidelity_of(
    dir: &Path,
    log: &ApuLog,
    set: &SongSet,
    claims: &[Claim],
) -> Result<serde_json::Value> {
    let mut songs = serde_json::Map::new();
    let mut sums: BTreeMap<Kind, (usize, usize, usize)> = BTreeMap::new();
    // The whole log's states and render, so each song's reference keeps
    // the frame counter's phase.
    let states = log.states();
    let audio = gp_sound::synth::render(
        log.region,
        &log.timeline(),
        log.region.frame_start(log.frames()),
        &log.samples,
        gp_sound::synth::UNITY,
    );
    for s in &set.songs {
        let Some(c) = &s.captured else { continue };
        let a = c.frames[0];
        let off = a - log.first;
        let n = (s.frames as usize).min(states.len().saturating_sub(off as usize));
        let reference = &states[off as usize..off as usize + n];
        // An effect's reference sound is its own channels only.
        let own;
        let ref_audio = if s.kind == Kind::Sfx {
            let chans = s.channels();
            let gains = std::array::from_fn(|i| {
                if chans.contains(&Channel::ALL[i]) {
                    1.0
                } else {
                    0.0
                }
            });
            let end = off + n as u64;
            let tl: Vec<_> = log
                .timeline()
                .into_iter()
                .filter(|w| w.0 < log.region.frame_start(end))
                .collect();
            own = gp_sound::synth::render(
                log.region,
                &tl,
                log.region.frame_start(end),
                &log.samples,
                gains,
            );
            gp_sound::frames_of(log.region, &own, off, end)
        } else {
            gp_sound::frames_of(log.region, &audio, off, off + n as u64)
        };
        let mask: [Vec<bool>; 5] = std::array::from_fn(|ci| {
            (0..n)
                .map(|f| {
                    let fr = off + f as u64;
                    let claimed =
                        |cl: &&Claim| cl.channel.index() == ci && fr >= cl.from && fr < cl.to;
                    match s.kind {
                        Kind::Music => claims.iter().any(|cl| claimed(&cl)),
                        Kind::Sfx => !s.channels().contains(&Channel::ALL[ci]),
                    }
                })
                .collect()
        });
        let Some(ag) = gp_sound::agreement(set, &s.name, reference, Some(ref_audio), Some(&mask))
        else {
            continue;
        };
        let e = sums.entry(s.kind).or_default();
        e.0 += ag.reference_notes;
        e.1 += ag.candidate_notes;
        e.2 += ag.matched;
        songs.insert(s.name.clone(), serde_json::to_value(&ag)?);
    }
    let f1 = |(r, c, m): (usize, usize, usize)| {
        if r + c == 0 {
            1.0
        } else {
            (2.0 * m as f64 / (r + c) as f64 * 1000.0).round() / 1000.0
        }
    };
    let v = json!({
        "songs": songs,
        "music_f1": sums.get(&Kind::Music).map(|x| f1(*x)),
        "sfx_f1": sums.get(&Kind::Sfx).map(|x| f1(*x)),
    });
    write(
        &dir.join("fidelity.json"),
        serde_json::to_string_pretty(&v)?.as_bytes(),
    )?;
    Ok(v)
}

fn fidelity_cmd(dir: &Path) -> Result<Outcome> {
    if dir.join("ay.log").is_file() {
        let (text, v) = crate::sound_ay::fidelity(dir)?;
        return Ok(Outcome::ok(v, text));
    }
    let log = ApuLog::parse(&read(&dir.join("apu.log"))?).map_err(anyhow::Error::msg)?;
    let set = SongSet::parse(&read(&dir.join("songs.json"))?).map_err(anyhow::Error::msg)?;
    let cj: serde_json::Value = serde_json::from_str(&read(&dir.join("claims.json"))?)?;
    let claims: Vec<Claim> = cj["claims"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| {
            Some(Claim {
                request: c["request"].as_u64()? as usize,
                channel: Channel::parse(c["channel"].as_str()?)?,
                from: c["from"].as_u64()?,
                to: c["to"].as_u64()?,
            })
        })
        .collect();
    let v = fidelity_of(dir, &log, &set, &claims)?;
    let text = format!(
        "music onset F1 {}, effects {} ({} songs)",
        v["music_f1"],
        v["sfx_f1"],
        set.songs.len()
    );
    Ok(Outcome::ok(v, text))
}

fn compare_cmd(a: &CompareArgs) -> Result<Outcome> {
    let first = read(&a.original)?;
    if first.starts_with(crate::sound_ay::HEADER) {
        let x = crate::sound_ay::AyLog::parse(&first)?;
        let y = crate::sound_ay::AyLog::parse(&read(&a.model)?)?;
        let (text, json, exit) = crate::sound_ay::compare(&x, &y, a.offset);
        return Ok(Outcome { json, text, exit });
    }
    let original = ApuLog::parse(&read(&a.original)?).map_err(anyhow::Error::msg)?;
    let model = ApuLog::parse(&read(&a.model)?).map_err(anyhow::Error::msg)?;
    let (first, frames) = gp_sound::compare::pair_logs(&original, &model, a.offset);
    if frames.is_empty() {
        bail!("the two logs share no frame at offset {}", a.offset);
    }
    let r = gp_sound::compare::compare(original.region, &frames);
    let name = |i: usize| format!("the original's frame {}", first + i as u64);
    let text = gp_sound::compare::lines(&r, &name).join("\n");
    let at = |i: Option<usize>| i.map(|i| first + i as u64);
    let mut out = Outcome::ok(
        json!({
            "first": first,
            "frames": r.frames,
            "writes": r.writes,
            "channels": r.channels,
            "longest": r.longest,
            "by_channel": r.by_channel,
            "first_writes": at(r.first_writes.as_ref().map(|f| f.0)),
            "first_channel": at(r.first_channel.as_ref().map(|f| f.0)),
        }),
        text,
    );
    if r.channels < r.frames {
        out.exit = 1;
    }
    Ok(out)
}

fn drivers_cmd() -> Result<Outcome> {
    let mut text = String::new();
    let mut v = Vec::new();
    for d in gp_sound::drivers::REGISTRY {
        let _ = writeln!(
            text,
            "{:<11} {:<11} decoder {:<3} {}: {}",
            d.name,
            d.format,
            if d.decoder { "yes" } else { "no" },
            d.games.join(", "),
            d.about
        );
        v.push(json!({"name": d.name, "format": d.format, "decoder": d.decoder, "games": d.games}));
    }
    Ok(Outcome::ok(json!(v), text.trim_end().to_owned()))
}

/// `name=a-b,c@k`: slots and the loop's index into them.
fn song_spec(s: &str) -> Result<(String, Vec<usize>, Option<usize>)> {
    let (name, rest) = s.split_once('=').context("a song is name=slots[@loop]")?;
    let (list, lp) = match rest.split_once('@') {
        Some((l, k)) => (l, Some(k.parse::<usize>()?)),
        None => (rest, None),
    };
    let mut slots = Vec::new();
    for part in list.split(',') {
        match part.split_once('-') {
            Some((a, b)) => slots.extend(a.parse::<usize>()?..=b.parse::<usize>()?),
            None => slots.push(part.parse()?),
        }
    }
    Ok((name.to_owned(), slots, lp))
}

/// The asset map `<source>.toml`: in `--maps`, else the maps folders.
fn source_map(a: &DecodeArgs) -> Option<(PathBuf, String)> {
    let dirs = match &a.maps {
        Some(d) => vec![d.clone()],
        None => crate::assets::maps_dirs(),
    };
    dirs.iter().find_map(|d| {
        let p = d.join(format!("{}.toml", a.source));
        std::fs::read_to_string(&p).ok().map(|t| (p, t))
    })
}

fn decode_cmd(a: &DecodeArgs, paths: &Paths) -> Result<Outcome> {
    use gp_sound::drivers;
    // The driver: by the format of an asset the NES map lifts, else the
    // one the registry knows the game by.
    let map = source_map(a);
    let by_asset = map
        .as_ref()
        .filter(|(_, t)| !gp_assets::arcade::is_arcade(t))
        .and_then(|(_, t)| gp_assets::map::AssetMap::parse(t).ok())
        .and_then(|m| {
            m.assets
                .iter()
                .find_map(|x| drivers::by_format(x.format()).map(|d| (d, x.name().to_owned())))
        });
    let driver = by_asset
        .as_ref()
        .map(|(d, _)| *d)
        .or_else(|| drivers::by_game(&a.source))
        .with_context(|| {
            format!(
                "no asset of {}'s map has a registered driver's format, and no driver lists the game (gp sound drivers)",
                a.source
            )
        })?;
    if !driver.decoder {
        bail!(
            "{} uses driver `{}`, which has no decoder yet: {}",
            a.source,
            driver.name,
            driver.about
        );
    }
    if driver.format != drivers::AY_LOG {
        // Every NES decoder game-printer had was another game's (Super
        // Mario Bros.'s), cut in the port.
        bail!(
            "driver `{}` decodes `{}` music data, which gameprinter has no decoder for",
            driver.name,
            driver.format
        );
    }
    decode_ay(a, driver, paths)
}

/// The midway-ssio decoder over an AY register log.
fn decode_ay(a: &DecodeArgs, driver: &gp_sound::drivers::Driver, paths: &Paths) -> Result<Outcome> {
    let mut log_path = a.ay_log.clone().with_context(|| {
        format!(
            "driver `{}` decodes an AY register log: give --ay-log (gp sound capture's ay.log of a {} recording)",
            driver.name, a.source
        )
    })?;
    if log_path.is_dir() {
        log_path = log_path.join("ay.log");
    }
    let log = crate::sound_ay::AyLog::parse(&read(&log_path)?)?;
    let mut specs = Vec::new();
    for s in &a.song {
        let (name, frames, lp) = song_spec(s)?;
        let (first, last) = (
            *frames.iter().min().context("a song names no frame")? as u64,
            *frames.iter().max().context("a song names no frame")? as u64,
        );
        specs.push(crate::sound_ay::SongSpec {
            name,
            first,
            last,
            loop_at: lp.map(|k| k as u64),
        });
    }
    let set = crate::sound_ay::decode(&log, &specs, a.gap);
    let songs = set["songs"].as_array().cloned().unwrap_or_default();
    let out = a.out.clone().unwrap_or_else(|| {
        paths
            .home
            .join("work/sound/decoded")
            .join(&a.source)
            .join("songs.json")
    });
    write(&out, serde_json::to_string_pretty(&set)?.as_bytes())?;
    let mut text = format!(
        "{}: driver {}, {} songs decoded from {} (frames {} to {})\nwrote {}",
        a.source,
        driver.name,
        songs.len(),
        log_path.display(),
        log.first,
        log.last,
        out.display()
    );
    let mut cross = serde_json::Map::new();
    if let Some(p) = &a.against {
        let cap: serde_json::Value = serde_json::from_str(&read(p)?)?;
        let caps = crate::sound_ay::songs_of(&cap, "captured");
        for d in &songs {
            let best = caps
                .iter()
                .map(|c| (c, crate::sound_ay::cross_check(d, c)))
                .max_by(|x, y| x.1["f1"].as_f64().partial_cmp(&y.1["f1"].as_f64()).unwrap());
            if let Some((c, o)) = best {
                let name = d["name"].as_str().unwrap_or("?");
                let _ = write!(
                    text,
                    "\n  {name} vs captured {}: onset F1 {} at offset {} ({} of {} captured notes matched)",
                    c["name"].as_str().unwrap_or("?"),
                    o["f1"],
                    o["offset"],
                    o["matched"],
                    o["reference"]
                );
                cross.insert(name.to_owned(), json!({"against": c["name"], "onsets": o}));
            }
        }
    }
    for s in &songs {
        let notes: usize = s["channels"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|c| c["notes"].as_array().map_or(0, Vec::len))
            .sum();
        let _ = write!(
            text,
            "\n  {:<8} frames {} to {}, {notes} notes",
            s["name"].as_str().unwrap_or("?"),
            s["first"],
            s["last"]
        );
    }
    let failed: Vec<String> = Vec::new();
    Ok(Outcome::ok(
        json!({"out": out, "songs": songs.len(), "driver": driver.name, "cross_check": cross, "failed": failed}),
        text,
    ))
}

fn mapping(p: Option<&Path>) -> Result<Mapping> {
    match p {
        Some(p) => Mapping::parse(&read(p)?).map_err(anyhow::Error::msg),
        None => Ok(Mapping::default()),
    }
}

/// A song's samples through the APU: its first pass and `loops` more.
pub(crate) fn render_song(
    set: &SongSet,
    song: &str,
    loops: u32,
    gains: gp_sound::synth::Gains,
) -> Result<Vec<i16>> {
    let s = set
        .song(song)
        .with_context(|| format!("no song `{song}`"))?;
    let mut p = gp_sound::player::Player::new(set.clone());
    p.play_music(Some(song));
    let lf = s.loop_frame.unwrap_or(0);
    let frames = s.frames + u64::from(loops) * (s.frames - lf);
    let mut synth = gp_sound::synth::Synth::new(
        set.region,
        gp_sound::synth::SAMPLE_RATE,
        set.samples.clone(),
    );
    synth.gains = gains;
    let mut out = Vec::new();
    for _ in 0..frames {
        let w = p.frame();
        out.extend(synth.frame(&w));
    }
    Ok(out)
}

/// Whether a file's text is an AY song file (`gp-ay-songs 0`).
fn ay_songs(text: &str) -> Option<serde_json::Value> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    (v["format"] == "gp-ay-songs 0").then_some(v)
}

/// The AY song asked for, or the first.
fn ay_song(v: &serde_json::Value, input: &Path, want: Option<&str>) -> Result<serde_json::Value> {
    let name = input
        .parent()
        .and_then(|d| d.file_name())
        .map_or("capture".into(), |n| n.to_string_lossy().into_owned());
    let songs = crate::sound_ay::songs_of(v, &name);
    match want {
        Some(w) => songs
            .into_iter()
            .find(|s| s["name"] == w)
            .with_context(|| format!("no song `{w}`")),
        None => songs.into_iter().next().context("the set has no song"),
    }
}

/// `gp sound render` of an AY log or AY songs, through the board's sound.
fn render_ay(a: &RenderArgs, text: &str) -> Result<Outcome> {
    if a.banks.is_some() {
        bail!("--banks maps the NES's instruments; AY sound plays through the board's own voices");
    }
    let (log, what) = if text.starts_with(crate::sound_ay::HEADER) {
        (
            crate::sound_ay::AyLog::parse(text)?,
            "the AY log".to_owned(),
        )
    } else {
        let v = ay_songs(text).context("not AY songs")?;
        let song = ay_song(&v, &a.input, a.song.as_deref())?;
        let name = song["name"].as_str().unwrap_or("?").to_owned();
        (
            crate::sound_ay_render::song_log(&song, a.loops),
            format!("song {name}"),
        )
    };
    let zip = match &a.rom {
        Some(p) => Some(p.clone()),
        None => gp_arcade::romset::local_romset("tapper"),
    };
    let prom = zip
        .as_deref()
        .and_then(|z| gp_arcade::Romset::load_zip(z).ok())
        .and_then(|s| s.ssio_prom);
    let samples = crate::sound_ay_render::render_log(&log, prom.as_deref());
    write(&a.out, &crate::sound_ay_render::wav_stereo(&samples))?;
    let secs = samples.len() as f64 / f64::from(gp_arcade::sound::OUT_RATE);
    Ok(Outcome::ok(
        json!({"out": a.out, "samples": samples.len(), "seconds": secs, "channels": 2, "prom": prom.is_some()}),
        format!(
            "wrote {} ({secs:.1} s, stereo, {what} through the AY-8910s{})",
            a.out.display(),
            if prom.is_some() {
                ", the SSIO PROM's volumes"
            } else {
                "; no romset with the SSIO PROM, so every voice at full volume"
            }
        ),
    ))
}

fn render_cmd(a: &RenderArgs, home: &Path) -> Result<Outcome> {
    let text = read(&a.input)?;
    if text.starts_with(crate::sound_ay::HEADER) || ay_songs(&text).is_some() {
        return render_ay(a, &text);
    }
    let map = mapping(a.banks.as_deref())?;
    let samples = if text.starts_with("gp-apu-log") {
        let log = ApuLog::parse(&text).map_err(anyhow::Error::msg)?;
        let end = log.region.frame_start(log.frames());
        gp_sound::synth::render(
            log.region,
            &log.timeline(),
            end,
            &log.samples,
            map.mix.gains(),
        )
    } else {
        let set = SongSet::parse(&text).map_err(anyhow::Error::msg)?;
        let song = a
            .song
            .clone()
            .or_else(|| set.songs.first().map(|s| s.name.clone()));
        let song = song.context("the set has no song")?;
        if map.uses_banks(&set, &song) {
            let mut folders = gp_sound::banks::folders(home);
            if let Some(d) = a.banks.as_ref().and_then(|b| b.parent()) {
                folders.insert(0, d.to_path_buf());
            }
            let r = gp_sound_bank::render(&set, &song, &map, &folders, a.loops)
                .map_err(anyhow::Error::msg)?;
            for (c, b, p) in &r.banked {
                tracing::info!("{} plays through {b} `{p}`", c.name());
            }
            for f in &r.fallbacks {
                tracing::warn!("{f}");
            }
            r.samples
        } else {
            render_song(&set, &song, a.loops, map.mix.gains())?
        }
    };
    write(
        &a.out,
        &gp_sound::synth::wav(&samples, gp_sound::synth::SAMPLE_RATE),
    )?;
    let secs = samples.len() as f64 / f64::from(gp_sound::synth::SAMPLE_RATE);
    Ok(Outcome::ok(
        json!({"out": a.out, "samples": samples.len(), "seconds": secs}),
        format!("wrote {} ({secs:.1} s)", a.out.display()),
    ))
}

fn banks_cmd(paths: &Paths) -> Result<Outcome> {
    let folders = gp_sound::banks::folders(&paths.home);
    let found = gp_sound::banks::discover(&folders);
    let mut text = format!(
        "{} banks (looked in {})",
        found.len(),
        folders
            .iter()
            .map(|f| f.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
    for b in &found {
        let _ = write!(
            text,
            "\n  {:<20} {:?} {} patches, licence {}",
            b.name,
            b.kind,
            b.patches.len(),
            b.license.as_deref().unwrap_or("unknown")
        );
    }
    Ok(Outcome::ok(serde_json::to_value(&found)?, text))
}
