//! The checks of an arcade game against its own ROM, for `gp oracle
//! arcade` to run from a game's folder (docs/oracle-mame.md, "Checks of a
//! game against its ROM"). Each takes a set the caller loaded and what
//! the game's folder holds, and returns a line saying what it checked, or
//! why it failed. None reads a ROM or a game's folder itself, so each is
//! tested here on gp_arcade::testrom's made-up sets.
//!
//! - [`replay`]: a MAME capture's input log played on the board matches
//!   the capture on every frame (its AY writes and, with `sound.wav`, its
//!   speakers too), and a log whose watchdog runs out resets the board,
//!   which comes back.
//! - [`snapshots`]: a snapshot taken mid-run restores exactly, and two runs
//!   with the same presses end the same.
//! - [`polls`]: the runtime protocol's frame event says the game read its
//!   inputs on most frames.
//! - [`sound_board_alone`]: the sound board run alone, fed the latch writes
//!   the whole machine made, gives the same AY writes and samples.
//! - [`dasm`]: every instruction either CPU runs is followed by the one the
//!   disassembler's length and flow say.
//! - [`harvest`]: the static harvest of the main CPU finds what the game's
//!   harvest.toml says was found by hand.

use crate::dasm::{Flow, Op, decode};
use crate::harvest::{Board, Harvest, Value};
use crate::oracle::{Capture, InputLog, compare};
use crate::ssio::SoundBoard;
use crate::{Controls, Mcr, Romset};

/// A check's outcome: Ok with what it checked, Err with why it failed.
pub type Checked = Result<String, String>;

/// The capture's input log played on the board from power-on, held to the
/// capture on every frame (RAM, ports, picture and, when the capture has
/// ay.tsv, the AY writes). A log that asks for the 4-way filter must have
/// the filter change some frame. With `sound.wav` in the capture's folder
/// the speakers' samples must all but equal MAME's ([`samples`]); then the
/// watchdog ([`watchdog`]).
pub fn replay(rom: &Romset, cap: &Capture, log: &InputLog) -> Checked {
    if log.four_way && log.played == log.frames {
        return Err("the log asks for the 4-way filter, and it changes no frame".into());
    }
    let mut t = Mcr::power_on(rom);
    for n in 0..cap.rows.len() {
        if n > 0 {
            t.step(log.controls(n));
        }
        let d = compare(&t, cap, n);
        if !d.is_empty() {
            return Err(format!("frame {n} differs from MAME:\n{}", d.describe(16)));
        }
    }
    let mut said = vec![format!(
        "all {} frames match MAME{}",
        cap.rows.len(),
        if cap.ay.is_some() {
            ", AY writes included"
        } else {
            ""
        }
    )];
    match std::fs::read(cap.dir.join("sound.wav")) {
        Ok(wav) => said.push(samples(rom, cap, log, &wav)?),
        Err(_) => said.push("no sound.wav, so no samples compared".into()),
    }
    said.push(watchdog(rom, log)?);
    Ok(said.join("; "))
}

/// The speakers' samples, the log played with sound on, against a MAME
/// 16-bit stereo WAV's: as many, none more than one step apart, and fewer
/// than one in 100000 differing (float rounding in the mixing).
pub fn samples(rom: &Romset, cap: &Capture, log: &InputLog, wav: &[u8]) -> Checked {
    let at = wav
        .windows(4)
        .position(|w| w == b"data")
        .ok_or("sound.wav has no data chunk")?
        + 8;
    let theirs: Vec<i16> = wav
        .get(at..)
        .unwrap_or_default()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| i16::from_le_bytes(*b))
        .collect();
    let mut t = Mcr::power_on(rom);
    t.set_audio(true);
    let mut ours = Vec::new();
    for n in 1..cap.rows.len() {
        t.step(log.controls(n));
        for s in t.take_audio() {
            ours.push(crate::sound::to_i16(s[0]));
            ours.push(crate::sound::to_i16(s[1]));
        }
    }
    if ours.len() != theirs.len() {
        return Err(format!(
            "{} samples, MAME's sound.wav {}",
            ours.len(),
            theirs.len()
        ));
    }
    let worst = ours
        .iter()
        .zip(&theirs)
        .map(|(a, b)| (i32::from(*a) - i32::from(*b)).abs())
        .max()
        .unwrap_or(0);
    let differ = ours.iter().zip(&theirs).filter(|(a, b)| a != b).count();
    if worst > 1 {
        return Err(format!("a sample differs from MAME's by {worst}"));
    }
    if differ * 100_000 >= ours.len().max(1) {
        return Err(format!(
            "{differ} of {} samples differ from MAME's",
            ours.len()
        ));
    }
    Ok(format!(
        "{} samples as MAME's, {differ} one step apart",
        ours.len()
    ))
}

/// When the log's watchdog runs out (a tilt hangs the game), the board
/// resets: snapshots over the 20 frames after restore exactly, and the
/// game reads its inputs on over 300 frames after. A log whose watchdog
/// never runs out passes.
pub fn watchdog(rom: &Romset, log: &InputLog) -> Checked {
    let mut t = Mcr::power_on(rom);
    let mut fired_at = None;
    let mut polled_after = 0;
    for n in 1..=log.frames.len() {
        t.step(log.controls(n));
        if fired_at.is_none() && t.watchdog_fired() {
            fired_at = Some(n);
        }
        if fired_at.is_some() && t.polled() {
            polled_after += 1;
        }
        if fired_at.is_some_and(|f| n < f + 20) {
            let mut u = t.clone();
            u.restore(&t.snapshot())
                .map_err(|e| format!("frame {n}, after the watchdog: {e}"))?;
            if u.snapshot() != t.snapshot() {
                return Err(format!(
                    "frame {n}, after the watchdog: a snapshot doesn't restore exactly"
                ));
            }
        }
    }
    match fired_at {
        None => Ok("the watchdog never ran out".into()),
        Some(at) if polled_after > 300 => Ok(format!(
            "the watchdog ran out at frame {at} and the game came back ({polled_after} frames read their inputs after)"
        )),
        Some(at) => Err(format!(
            "the watchdog ran out at frame {at}, and the game read its inputs on only {polled_after} frames after (over 300 wanted)"
        )),
    }
}

/// A coin at frames 300 to 305; a snapshot at frame 320, put back into a
/// machine that ran 40 frames on, and a fresh machine run the same way,
/// end at frame 410 with the same state hash, picture hash and snapshot.
pub fn snapshots(rom: &Romset) -> Checked {
    let coin_at = |n: u64| Controls {
        coin: (300..306).contains(&n),
        ..Default::default()
    };
    let run = |t: &mut Mcr, frames: u32| {
        for _ in 0..frames {
            let n = t.frame() + 1;
            t.step(coin_at(n));
        }
    };
    let mut a = Mcr::power_on(rom);
    run(&mut a, 320);
    let snap = a.snapshot();
    let mut b = a.clone();
    run(&mut a, 90);
    run(&mut b, 40);
    b.restore(&snap).map_err(|e| format!("restoring: {e}"))?;
    run(&mut b, 90);
    if b.state_hash() != a.state_hash() || b.picture_hash() != a.picture_hash() {
        return Err("a snapshot from frame 320, restored, ends at frame 410 elsewhere".into());
    }
    if b.snapshot() != a.snapshot() {
        return Err("a snapshot from frame 320, restored, gives another snapshot at 410".into());
    }
    let mut c = Mcr::power_on(rom);
    run(&mut c, 410);
    if c.state_hash() != a.state_hash() {
        return Err("two runs with the same presses end at frame 410 apart".into());
    }
    Ok("a snapshot restores exactly and two runs end the same at frame 410".into())
}

/// Over `frames` frames of no input from power-on, the runtime protocol's
/// `frame` event says the game read its inputs on more than half.
pub fn polls(rom: &Romset, frames: usize) -> Checked {
    use gp_runtime::Runtime;
    let mut rt = crate::ArcadeRuntime::load(rom);
    let ev = rt
        .step(&vec![gp_runtime::InputFrame::default(); frames])
        .map_err(|e| format!("the runtime: {e:?}"))?;
    let polled = ev
        .iter()
        .filter(|e| e.data[0].value == gp_runtime::Value::Flag(true))
        .count();
    if polled * 2 > frames {
        Ok(format!(
            "the game read its inputs on {polled} of {frames} frames"
        ))
    } else {
        Err(format!(
            "the frame event says the game read its inputs on {polled} of {frames} frames (over half wanted)"
        ))
    }
}

/// The log played on the whole board for up to `frames` frames, and the
/// latch writes that reached its sound board fed to a sound board of its
/// own: the same AY writes and speaker samples every frame, with some
/// latch writes, AY writes and loud samples among them.
pub fn sound_board_alone(rom: &Romset, log: &InputLog, frames: usize) -> Checked {
    let mut whole = Mcr::power_on(rom);
    whole.set_audio(true);
    whole.set_latch_log(true);
    let mut alone = SoundBoard::power_on(rom);
    alone.set_audio(true);
    let (mut latches, mut ay, mut loud) = (0, 0, 0);
    let frames = log.frames.len().min(frames);
    for n in 1..frames {
        whole.step(log.controls(n));
        alone.clear_writes();
        for (at, latch, value) in whole.take_latch_log() {
            alone.latch(at, latch, value);
            latches += 1;
        }
        alone.run_to(whole.time());
        if alone.ay_writes() != whole.ay_writes() {
            return Err(format!(
                "frame {n}: the sound board alone wrote the AYs [{}], the whole machine [{}]",
                crate::oracle::ay_text(alone.ay_writes()),
                crate::oracle::ay_text(whole.ay_writes())
            ));
        }
        ay += whole.ay_writes().len();
        let (a, w) = (alone.take_audio(), whole.take_audio());
        loud += w.iter().filter(|s| s[0].abs() + s[1].abs() > 0.01).count();
        if a != w {
            return Err(format!("frame {n}: the speaker samples differ"));
        }
    }
    if latches == 0 || ay == 0 || loud == 0 {
        return Err(format!(
            "nothing to compare: {latches} latch writes, {ay} AY writes, {loud} loud samples in {frames} frames"
        ));
    }
    Ok(format!(
        "{frames} frames: {latches} latch writes, {ay} AY writes and every sample the same"
    ))
}

/// `frames` frames from power-on (a coin two thirds in, Start 66 frames
/// later), traced every 30th frame and the last 30: every instruction
/// either CPU runs is followed by the one its decoded length and flow say,
/// unless an interrupt comes between. Fails under `min_pairs` pairs.
pub fn dasm(rom: &Romset, frames: u32, min_pairs: u64) -> Checked {
    let coin = frames * 2 / 3;
    let start = coin + 66;
    let mut t = Mcr::power_on(rom);
    let mut checked = 0u64;
    for frame in 1..=frames {
        t.set_trace(frame % 30 == 0 || frame + 30 > frames);
        t.step(Controls {
            coin: (coin..coin + 6).contains(&frame),
            start: (start..start + 6).contains(&frame),
            ..Controls::default()
        });
        let all = t.take_trace();
        for cpu in 0..2u8 {
            let entries: Vec<_> = all.iter().filter(|e| e.cpu == cpu).collect();
            for w in entries.windows(2) {
                let (a, b) = (w[0], w[1]);
                if a.interrupt.is_some() || b.interrupt.is_some() {
                    continue;
                }
                let i = decode(&a.bytes, a.pc);
                let next = a.pc.wrapping_add(u16::from(i.len));
                // ldir and the other repeating block instructions run
                // again from their own address until BC is done.
                let repeats = i.op == Op::Block && i.text.ends_with('r');
                let ok = match i.flow {
                    Flow::Next if repeats => b.pc == next || b.pc == a.pc,
                    Flow::Next => b.pc == next,
                    Flow::Jump(t) => b.pc == t,
                    Flow::JumpIf(t) | Flow::Djnz(t) => b.pc == t || b.pc == next,
                    Flow::Call(t) | Flow::Rst(t) => b.pc == t,
                    Flow::CallIf(t) => b.pc == t || b.pc == next,
                    Flow::Halt => b.pc == a.pc || b.pc == next,
                    // Returns and jp (hl) go where the stack or a register says.
                    Flow::Ret | Flow::RetIf | Flow::Indirect(_) => true,
                };
                if !ok {
                    return Err(format!(
                        "cpu {cpu} frame {frame}: {:04x} {:02x?} `{}` ({} bytes, {:?}) was followed by {:04x}",
                        a.pc, a.bytes, i.text, i.len, i.flow, b.pc
                    ));
                }
                checked += 1;
            }
        }
    }
    if checked < min_pairs {
        return Err(format!(
            "only {checked} pairs of instructions traced in {frames} frames ({min_pairs} wanted)"
        ));
    }
    Ok(format!(
        "{checked} pairs of instructions in {frames} frames follow as their lengths and flow say"
    ))
}

/// What a game's harvest.toml says the static harvest must find, on the
/// set it names.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HarvestWants {
    /// MAME's name for the set whose main CPU is harvested.
    pub set: String,
    #[serde(default)]
    fact: Vec<WantFact>,
    #[serde(default)]
    list: Vec<WantList>,
    #[serde(default)]
    dispatch: Vec<WantDispatch>,
    #[serde(default)]
    machine: Vec<WantMachine>,
}

/// A fact at a path, with this value when one is given.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct WantFact {
    path: String,
    value: Option<toml::Value>,
}

/// A list fact with an item starting with each string.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct WantList {
    path: String,
    starts_with: Vec<String>,
}

/// A jump table at an address, on a subject, with so many cases.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct WantDispatch {
    at: String,
    subject: String,
    cases: usize,
}

/// A state machine on a byte with these (from, to, routine) transitions.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct WantMachine {
    byte: String,
    transitions: Vec<[String; 3]>,
}

impl HarvestWants {
    pub fn parse(text: &str) -> Result<Self, String> {
        toml::from_str(text).map_err(|e| format!("harvest.toml: {e}"))
    }

    /// How many things it asks the harvest to find.
    pub fn len(&self) -> usize {
        self.fact.len() + self.list.len() + self.dispatch.len() + self.machine.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

fn same(v: &Value, want: &toml::Value) -> bool {
    match (v, want) {
        (Value::Int(a), toml::Value::Integer(b)) => a == b,
        (Value::Str(a), toml::Value::String(b)) => a == b,
        (Value::Bool(a), toml::Value::Boolean(b)) => a == b,
        _ => false,
    }
}

/// The static harvest of `maincpu` (the MCR main board's) held to `want`:
/// each thing it names must be found. Err lists every one that wasn't.
pub fn harvest(maincpu: &[u8], want: &HarvestWants) -> Checked {
    harvest_found(&crate::harvest::harvest(&Board::mcr_main(maincpu)), want)
}

/// [`harvest`] on a harvest already made.
pub fn harvest_found(h: &Harvest, want: &HarvestWants) -> Checked {
    let mut missing = Vec::new();
    for f in &want.fact {
        let found: Vec<&Value> = h
            .facts
            .iter()
            .filter(|x| x.path == f.path)
            .map(|x| &x.value)
            .collect();
        match &f.value {
            None if found.is_empty() => missing.push(format!("no fact {}", f.path)),
            Some(w) if !found.iter().any(|v| same(v, w)) => {
                missing.push(format!("fact {}: {found:?}, not {w}", f.path));
            }
            _ => {}
        }
    }
    for l in &want.list {
        let Some(Value::List(items)) = h.facts.iter().find(|f| f.path == l.path).map(|f| &f.value)
        else {
            missing.push(format!("no list {}", l.path));
            continue;
        };
        for start in &l.starts_with {
            if !items
                .iter()
                .any(|c| matches!(c, Value::Str(x) if x.starts_with(start.as_str())))
            {
                missing.push(format!("list {}: no item starts {start}", l.path));
            }
        }
    }
    for d in &want.dispatch {
        match h
            .dispatch
            .iter()
            .find(|t| t.at == d.at && t.subject.starts_with(d.subject.as_str()))
        {
            None => missing.push(format!("no jump table at {} on {}", d.at, d.subject)),
            Some(t) if t.cases.len() != d.cases => missing.push(format!(
                "the jump table at {} has {} cases, not {}",
                d.at,
                t.cases.len(),
                d.cases
            )),
            Some(_) => {}
        }
    }
    for m in &want.machine {
        let Some(found) = h.machines.iter().find(|x| x.byte == m.byte) else {
            missing.push(format!("no state machine on {}", m.byte));
            continue;
        };
        for w in &m.transitions {
            if !found
                .transitions
                .iter()
                .any(|t| t.from == w[0] && t.to == w[1] && t.routine == w[2])
            {
                missing.push(format!(
                    "the state machine on {} has no {} to {} in {}",
                    m.byte, w[0], w[1], w[2]
                ));
            }
        }
    }
    if missing.is_empty() {
        Ok(format!(
            "the harvest found all {} things harvest.toml names",
            want.len()
        ))
    } else {
        Err(format!(
            "the harvest missed {} of {}: {}",
            missing.len(),
            want.len(),
            missing.join("; ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oracle::{FrameRow, ay_text};
    use crate::testrom;

    /// An input log of `frames` idle frames on a made-up set.
    fn idle(frames: usize) -> InputLog {
        InputLog::parse(&format!(
            "gp-arcade-input 0\nsystem madeup\ncolumns CSRLDUT\n{frames} .......\n"
        ))
        .unwrap()
    }

    /// A capture of our own board's run of `log`, as MAME's would read.
    fn own_capture(rom: &Romset, log: &InputLog, frames: usize) -> Capture {
        let mut t = Mcr::power_on(rom);
        let (mut rows, mut ram, mut ay) = (Vec::new(), Vec::new(), Vec::new());
        for n in 0..frames {
            if n > 0 {
                t.step(log.controls(n));
            }
            rows.push(FrameRow {
                frame: n as u64,
                time: String::new(),
                ports: t.ports(),
                picture: t.picture_hash(),
                main_pc: 0,
                ssio_pc: 0,
                shot: None,
            });
            ram.extend_from_slice(&t.ram_record());
            ay.push(ay_text(t.ay_writes()));
        }
        Capture {
            dir: std::path::PathBuf::from("/nonexistent/made-up-capture"),
            rows,
            ram,
            ay: Some(ay),
        }
    }

    #[test]
    fn a_capture_of_the_boards_own_run_replays_and_one_byte_off_fails() {
        let rom = testrom::sounding();
        let log = idle(12);
        let mut cap = own_capture(&rom, &log, 10);
        let ok = replay(&rom, &cap, &log).unwrap();
        assert!(ok.contains("all 10 frames match"), "{ok}");
        assert!(ok.contains("watchdog never ran out"), "{ok}");
        let at = 7 * crate::mcr::RAM_RECORD + 3;
        cap.ram[at] ^= 0xff;
        let err = replay(&rom, &cap, &log).unwrap_err();
        assert!(err.starts_with("frame 7 differs"), "{err}");
        cap.ram[at] ^= 0xff;
        cap.ay.as_mut().unwrap()[4] = "0d00".into();
        assert!(replay(&rom, &cap, &log).unwrap_err().contains("AY writes"));
    }

    #[test]
    fn a_four_way_log_whose_filter_changes_nothing_fails() {
        let rom = testrom::looping();
        let log = InputLog::parse(
            "gp-arcade-input 0\nsystem madeup\njoystick 4way\ncolumns CSRLDUT\n4 .......\n",
        )
        .unwrap();
        let cap = own_capture(&rom, &log, 3);
        assert!(replay(&rom, &cap, &log).unwrap_err().contains("4-way"));
    }

    #[test]
    fn samples_are_held_to_a_wav_of_the_same_run() {
        let rom = testrom::sounding();
        let log = idle(6);
        let cap = own_capture(&rom, &log, 5);
        let mut t = Mcr::power_on(&rom);
        t.set_audio(true);
        let mut wav = b"RIFF\0\0\0\0WAVEdata\0\0\0\0".to_vec();
        for n in 1..5 {
            t.step(log.controls(n));
            for s in t.take_audio() {
                wav.extend_from_slice(&crate::sound::to_i16(s[0]).to_le_bytes());
                wav.extend_from_slice(&crate::sound::to_i16(s[1]).to_le_bytes());
            }
        }
        assert!(samples(&rom, &cap, &log, &wav).is_ok());
        let last = wav.len() - 1;
        wav[last] ^= 0x40;
        let err = samples(&rom, &cap, &log, &wav).unwrap_err();
        assert!(err.contains("differs from MAME's by"), "{err}");
        wav.truncate(wav.len() - 4);
        assert!(
            samples(&rom, &cap, &log, &wav)
                .unwrap_err()
                .contains("samples, MAME's")
        );
    }

    #[test]
    fn snapshots_restore_on_a_made_up_set() {
        let ok = snapshots(&testrom::busy()).unwrap();
        assert!(ok.contains("restores exactly"), "{ok}");
    }

    #[test]
    fn a_set_that_never_reads_its_inputs_fails_the_poll_check() {
        let err = polls(&testrom::looping(), 20).unwrap_err();
        assert!(err.contains("of 20 frames"), "{err}");
    }

    #[test]
    fn the_sound_board_alone_sounds_as_the_whole_machine_on_a_made_up_set() {
        let rom = testrom::sounding();
        let ok = sound_board_alone(&rom, &idle(30), 1500).unwrap();
        assert!(ok.contains("every sample the same"), "{ok}");
        // A set whose sound board is never written has nothing to compare.
        let err = sound_board_alone(&testrom::looping(), &idle(10), 1500).unwrap_err();
        assert!(err.contains("nothing to compare"), "{err}");
    }

    #[test]
    fn the_disassembler_follows_a_made_up_sets_run() {
        let ok = dasm(&testrom::busy(), 31, 100).unwrap();
        assert!(ok.contains("follow as their lengths and flow say"), "{ok}");
        let err = dasm(&testrom::busy(), 31, u64::MAX).unwrap_err();
        assert!(err.contains("wanted"), "{err}");
    }

    #[test]
    fn the_harvest_is_held_to_what_a_harvest_toml_names() {
        let rom = testrom::busy();
        let h = crate::harvest::harvest(&Board::mcr_main(&rom.maincpu));
        let f = h.facts.first().expect("the harvest finds some fact");
        let want = HarvestWants::parse(&format!(
            "set = \"madeup\"\n[[fact]]\npath = {:?}\n",
            f.path
        ))
        .unwrap();
        assert_eq!(want.set, "madeup");
        let ok = harvest_found(&h, &want).unwrap();
        assert!(ok.contains("all 1 things"), "{ok}");
        let want = HarvestWants::parse(
            "set = \"madeup\"\n[[fact]]\npath = \"no.such.fact\"\n\n[[dispatch]]\nat = \"ffff\"\nsubject = \"x\"\ncases = 3\n\n[[machine]]\nbyte = \"e0ff\"\ntransitions = [[\"a\", \"b\", \"c\"]]\n",
        )
        .unwrap();
        let err = harvest(&rom.maincpu, &want).unwrap_err();
        assert!(err.starts_with("the harvest missed 3 of 3"), "{err}");
        assert!(err.contains("no fact no.such.fact"), "{err}");
        assert!(err.contains("no jump table at ffff"), "{err}");
        assert!(HarvestWants::parse("set = \"x\"\nwrong = 1\n").is_err());
    }
}
