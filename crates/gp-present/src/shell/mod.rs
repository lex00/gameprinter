//! The runtime shell's standard parts (docs/shell.md): what every Game
//! Printer target runs around its simulation, written once here so the
//! generated Bevy game and the web preview do the same thing. A target
//! supplies only its [`Storage`] (wit/shell/storage.wit), as the settings
//! screen (`crate::settings`) is drawn by one piece of code for both.
//!
//! | Part | Here |
//! |---|---|
//! | The save record | `gp_sim::save`, kept under [`slot_name`] |
//! | The rewind ring and its policy | [`Ring`], [`REWIND_EVERY`], [`REWIND_SECONDS`] |
//! | The autosave policy | [`autosaves`], into [`AUTOSAVE_SLOT`] |
//! | The slots as the settings screen lists them | [`slots::slot_lines`], [`slots::write_slot`] |
//! | The shell's own settings (console only, quick save buttons) | [`slots::Settings`] |
//! | The keys and gamepad buttons | [`keys::BINDINGS`] |
//! | Storage | [`Storage`], generated from the WIT; [`Memory`] for checks |
//! | The shell check every target passes | [`check`], on a [`Target`] |
//!
//! Every part keeps the game as its canonical state
//! (`gp_sim::World::canonical`, the bytes the runtime protocol's
//! `load-state` takes) and puts it back whole between ticks, so none of
//! them changes what a tick does: a run that saves, loads and rewinds
//! reaches the state hash a straight run of the same input reaches.

pub mod keys;
pub mod slots;
pub mod storage;

use std::collections::{BTreeMap, VecDeque};

use gp_sim::save::{NOT_A_SAVE, OTHER_BUILD, OTHER_GAME, Record};
use gp_sim::{Spec, World};

pub use slots::{Settings, SlotLine, SlotSave};
pub use storage::{Picked, Storage};

/// Frames between two states of the rewind ring.
pub const REWIND_EVERY: u64 = 2;

/// Seconds of play the ring holds by default.
pub const REWIND_SECONDS: u32 = 20;

/// The ring's lengths a player can choose, in seconds; 0 turns rewinding
/// off. The generated game's options step through them.
pub const REWIND_CHOICES: [u32; 5] = [0, 10, 20, 40, 60];

/// The slot the autosave writes; the player's slots are numbered from 1.
pub const AUTOSAVE_SLOT: usize = 0;

/// The slots the web preview's number keys reach (Shift+1 to 4 save, 1 to
/// 4 load).
pub const QUICK_SLOTS: usize = 4;

/// Where the saves are kept, as a [`Storage`] name.
pub const SAVES: &str = "saves/";

/// Where a target with no file chooser takes a file to load from (the
/// generated game's data folder's `imports/`), as a [`Storage`] name.
pub const IMPORTS: &str = "imports/";

/// The stamp the shell check keeps beside its saves, in place of a clock.
pub const CHECK_STAMP: u64 = 1_762_300_800;

/// The storage name of a save slot.
pub fn slot_name(slot: usize) -> String {
    format!("{SAVES}slot-{slot}.gpsave")
}

/// The file a slot is handed to the player as: `<game>-slotN.gpsave`.
pub fn export_name(model: &str, slot: usize) -> String {
    format!("{model}-slot{slot}.gpsave")
}

/// Whether `name` is a storage name: a relative path of lowercase
/// letters, digits, `-`, `_`, `.` and `/`, with no empty part, `.` or
/// `..`.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-_./".contains(&b))
        && name
            .split('/')
            .all(|p| !p.is_empty() && p != "." && p != "..")
}

/// Whether the tick whose events these are ends with an autosave: a level
/// started.
pub fn autosaves(events: &[gp_sim::Event]) -> bool {
    events
        .iter()
        .any(|e| e.kind == gp_sim::spec::EventKind::LEVEL_STARTED)
}

/// A game as the shell keeps it: the profile it runs in, which the
/// canonical state leaves out, and the canonical state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub profile: String,
    pub state: Vec<u8>,
}

impl Snapshot {
    pub fn of(world: &World, profile: &str) -> Self {
        Snapshot {
            profile: profile.to_owned(),
            state: world.canonical(),
        }
    }

    /// The frame the state is at, read from its header.
    pub fn frame(&self) -> Option<u64> {
        let b = self.state.get(8..16)?;
        Some(u64::from_le_bytes(b.try_into().ok()?))
    }

    /// The world the state holds, for a spec in this snapshot's profile.
    pub fn world(&self, spec: &Spec) -> Result<World, String> {
        World::from_canonical(spec, &self.state)
    }
}

/// The rewind ring: a state every `every` frames, the newest `cap` of
/// them. Held rewind steps back one state a tick, newest first.
#[derive(Debug, Clone)]
pub struct Ring {
    every: u64,
    cap: usize,
    states: VecDeque<(u64, Snapshot)>,
}

impl Ring {
    /// A ring of states `every` frames apart (at least 1) holding `cap`.
    /// A cap of 0 keeps nothing, which turns rewinding off.
    pub fn new(every: u64, cap: usize) -> Self {
        Ring {
            every: every.max(1),
            cap,
            states: VecDeque::new(),
        }
    }

    /// The ring for `seconds` of play at `rate` ticks a second.
    pub fn for_seconds(every: u64, seconds: u32, rate: f64) -> Self {
        let frames = (f64::from(seconds) * rate).ceil() as u64;
        Ring::new(every, (frames / every.max(1)) as usize)
    }

    /// The standard ring: [`REWIND_SECONDS`] of states [`REWIND_EVERY`]
    /// frames apart.
    pub fn standard(rate: f64) -> Self {
        Ring::for_seconds(REWIND_EVERY, REWIND_SECONDS, rate)
    }

    pub fn len(&self) -> usize {
        self.states.len()
    }

    pub fn is_empty(&self) -> bool {
        self.states.is_empty()
    }

    pub fn clear(&mut self) {
        self.states.clear();
    }

    /// Keeps the world's state when its frame is on the ring's step and
    /// newer than the newest kept. Called after each tick.
    pub fn note(&mut self, world: &World, profile: &str) {
        self.note_with(world.frame, || Snapshot::of(world, profile));
    }

    /// [`Ring::note`] for a game at `frame`, taking its snapshot only when
    /// the ring keeps it.
    pub fn note_with(&mut self, frame: u64, snap: impl FnOnce() -> Snapshot) {
        if self.cap == 0 || !frame.is_multiple_of(self.every) {
            return;
        }
        if self.states.back().is_some_and(|(f, _)| *f >= frame) {
            return;
        }
        if self.states.len() == self.cap {
            self.states.pop_front();
        }
        self.states.push_back((frame, snap()));
    }

    /// The newest state before `frame`, dropping every newer one. It stays
    /// in the ring, so the game can rewind to it again.
    pub fn back(&mut self, frame: u64) -> Option<Snapshot> {
        while self.states.back().is_some_and(|(f, _)| *f >= frame) {
            self.states.pop_back();
        }
        self.states.back().map(|(_, s)| s.clone())
    }
}

/// Storage held in memory: what the native shell checks run on, and a
/// target with nowhere to keep files.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Memory {
    pub files: BTreeMap<String, Vec<u8>>,
    /// The files handed to the player, by the file name they were given.
    pub exported: BTreeMap<String, Vec<u8>>,
    /// The files the player will pick, first first.
    pub to_import: VecDeque<Picked>,
}

impl Storage for Memory {
    fn list(&mut self, prefix: &str) -> Vec<String> {
        self.files
            .keys()
            .filter(|k| k.starts_with(prefix))
            .cloned()
            .collect()
    }

    fn read(&mut self, name: &str) -> Option<Vec<u8>> {
        self.files.get(name).cloned()
    }

    fn write(&mut self, name: &str, bytes: &[u8]) -> Result<(), String> {
        if !valid_name(name) {
            return Err(format!("`{name}` isn't a storage name"));
        }
        self.files.insert(name.to_owned(), bytes.to_vec());
        Ok(())
    }

    fn delete(&mut self, name: &str) -> Result<(), String> {
        self.files.remove(name);
        Ok(())
    }

    fn persist(&mut self) -> bool {
        true
    }

    fn export(&mut self, name: &str, file_name: &str) -> Result<String, String> {
        let bytes = self
            .files
            .get(name)
            .ok_or(format!("nothing is kept as `{name}`"))?;
        self.exported.insert(file_name.to_owned(), bytes.clone());
        Ok(file_name.to_owned())
    }

    /// The next file of `to_import`, else the last by name kept under
    /// `imports/`, as a data folder's import takes the newest file there.
    fn import(&mut self) -> Result<Option<Picked>, String> {
        if let Some(p) = self.to_import.pop_front() {
            return Ok(Some(p));
        }
        Ok(self
            .files
            .iter()
            .rev()
            .find(|(k, _)| k.starts_with(IMPORTS))
            .map(|(k, v)| Picked {
                file_name: k[IMPORTS.len()..].to_owned(),
                bytes: v.clone(),
            }))
    }
}

/// One frame of input as the shell check plays it: controller 1 in the
/// low byte, controller 2 in the high one, and the reset button.
pub type Input = (u16, bool);

/// A target as the shell check drives it: a game it steps, and its own
/// save and load.
pub trait Target {
    /// Runs one frame with this input.
    fn step(&mut self, input: Input) -> Result<(), String>;
    /// The canonical state's hash (`gp_sim::World::hash`).
    fn hash(&self) -> [u8; 32];
    /// The frame the game is at.
    fn frame(&self) -> u64;
    /// The game where it is.
    fn snapshot(&self) -> Snapshot;
    /// Puts a kept game back whole, as a rewind does.
    fn put(&mut self, snap: &Snapshot) -> Result<(), String>;
    /// The game as the save record this target writes (`gp_sim::save`).
    fn save_record(&mut self) -> Result<Vec<u8>, String>;
    /// Loads a save record as this target does, or says the sentence its
    /// player is shown, leaving the game as it was.
    fn load_record(&mut self, bytes: &[u8]) -> Result<(), String>;
}

/// What a passing shell check did, for its report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checked {
    pub frames: usize,
    pub saved_at: u64,
    pub rewound_from: u64,
    pub rewound_to: u64,
    pub loaded_at: u64,
    /// The saves refused, each with the sentence the player was shown.
    pub refused: Vec<(&'static str, String)>,
    /// The frame of the autosave, loaded back from its "Resume" line.
    pub resumed_at: u64,
    /// Where the export of slot 1 went, as the target said.
    pub exported: String,
}

impl Checked {
    pub fn says(&self) -> String {
        format!(
            "{} frames agree: saved at frame {} (listed on the Saves line with its stamp), rewound from {} to {}, autosaved there and resumed at {}, loaded at {} after wandering off, exported ({}) and loaded from a file; console only turns saves, loads and rewind off, quick save buttons are off until turned on; refused {}",
            self.frames,
            self.saved_at,
            self.rewound_from,
            self.rewound_to,
            self.resumed_at,
            self.loaded_at,
            self.exported,
            self.refused
                .iter()
                .map(|(what, why)| format!("{what} save (\"{why}\")"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

fn hex(h: &[u8; 32]) -> String {
    h.iter().map(|b| format!("{b:02x}")).collect()
}

/// The shell check every target passes (docs/shell.md, "The standard's
/// checks"). It plays `frames` from where the game is straight through,
/// then again with the shell at work, and every frame the second run
/// lands on must have the straight run's hash. The second run saves to
/// slot 1 through `store` a third of the way in, and the Saves line lists
/// it with its frame, mode and stamp; it rewinds through a ring from two
/// thirds back to halfway and autosaves there, listed as "Resume"; it
/// plays on, wanders off with other input, is refused another build's
/// save, another game's, a version 1 save and a damaged one (each leaving
/// the game where it was), and loads slot 1 back from `store`. Then slot 1
/// is exported, handed back as a file (under [`IMPORTS`]) and loaded from
/// it after wandering off again, a damaged file is refused, and Resume
/// loads the autosave. The shell's settings are kept and read back through
/// `store`, "Console only" turning off every save, load and rewind key and
/// the quick save buttons acting only once turned on. It plays on to the
/// end, and both slots are deleted.
pub fn check(
    t: &mut dyn Target,
    store: &mut dyn Storage,
    frames: &[Input],
) -> Result<Checked, String> {
    let n = frames.len();
    if n < 12 {
        return Err(format!(
            "the shell check needs 12 frames or more; the input has {n}"
        ));
    }
    let start = t.snapshot();
    let f0 = t.frame();
    let mut straight = vec![t.hash()];
    for f in frames {
        t.step(*f)?;
        straight.push(t.hash());
    }
    let at = |t: &dyn Target| (t.frame() - f0) as usize;
    let agree = |t: &dyn Target, what: &str| -> Result<(), String> {
        let i = at(t);
        if straight.get(i) == Some(&t.hash()) {
            Ok(())
        } else {
            Err(format!(
                "{what}: frame {} hashes {}, the straight run {}",
                f0 + i as u64,
                hex(&t.hash()),
                straight.get(i).map(hex).unwrap_or_default()
            ))
        }
    };
    t.put(&start)?;
    agree(t, "putting the start back")?;
    let mut ring = Ring::new(REWIND_EVERY, n);
    let (a, b, c) = (n / 3, 2 * n / 3, 5 * n / 6);
    let r = ((a + b) / 2) & !1;
    let play = |t: &mut dyn Target, to: usize, ring: &mut Ring| -> Result<(), String> {
        while at(t) < to {
            t.step(frames[at(t)])?;
            agree(t, "playing")?;
            let frame = t.frame();
            ring.note_with(frame, || t.snapshot());
        }
        Ok(())
    };
    play(t, a, &mut ring)?;
    let slot = slot_name(1);
    let saved = t.save_record()?;
    let record = Record::read(&saved)?;
    let model = record.model.clone();
    slots::write_slot(store, 1, &saved, Some(CHECK_STAMP))?;
    if !store.list(SAVES).contains(&slot) {
        return Err(format!("storage doesn't list {slot} after writing it"));
    }
    // The Saves line lists it with its frame, mode and stamp.
    let saved_frame = t.frame();
    let line = slots::slot_lines(store, &model, None, None)
        .into_iter()
        .find(|l| l.slot == 1)
        .and_then(|l| l.save)
        .ok_or("the Saves line doesn't list slot 1 after saving to it")?;
    let want = (saved_frame, t.snapshot().profile, Some(CHECK_STAMP), None);
    let got = (line.frame, line.profile, line.saved_at, line.refused);
    if got != want {
        return Err(format!(
            "the Saves line lists slot 1 as {got:?}, not {want:?} (frame, mode, stamp, refused)"
        ));
    }
    play(t, b, &mut ring)?;
    let rewound_from = t.frame();
    while at(t) > r {
        let back = ring
            .back(t.frame())
            .ok_or("the ring ran out before halfway")?;
        t.put(&back)?;
        agree(t, "rewinding")?;
    }
    let rewound_to = t.frame();
    // An autosave here, as a level start writes one: listed as "Resume".
    let auto = t.save_record()?;
    slots::write_slot(store, AUTOSAVE_SLOT, &auto, Some(CHECK_STAMP + 1))?;
    let resume = &slots::slot_lines(store, &model, None, None)[0];
    if resume.label() != "Resume" || resume.save.as_ref().map(|s| s.frame) != Some(rewound_to) {
        return Err(format!(
            "the autosave is listed as {} at {:?}, not Resume at frame {rewound_to}",
            resume.label(),
            resume.save.as_ref().map(|s| s.frame)
        ));
    }
    play(t, c, &mut ring)?;
    // Off another way: every button but the ones held, for 30 frames.
    let wander = |t: &mut dyn Target| -> Result<(), String> {
        for i in 0..30 {
            let (pads, _) = frames[(c + i) % n];
            t.step((!pads, false))?;
        }
        Ok(())
    };
    wander(t)?;
    let mut refused = Vec::new();
    let other_build = Record {
        digest: "0".repeat(64),
        ..record.clone()
    };
    let other_game = Record {
        model: format!("{}-other", record.model),
        ..record.clone()
    };
    let mut v1 = b"GPSV\x01\x00\x00\x00".to_vec();
    for s in [&record.model, &record.profile] {
        v1.extend_from_slice(&(s.len() as u16).to_le_bytes());
        v1.extend_from_slice(s.as_bytes());
    }
    v1.extend_from_slice(&record.state);
    for (what, bytes, want) in [
        ("another build's", other_build.to_bytes(), OTHER_BUILD),
        ("another game's", other_game.to_bytes(), OTHER_GAME),
        ("a version 1", v1, OTHER_BUILD),
        ("a damaged", saved[..10].to_vec(), NOT_A_SAVE),
    ] {
        refuses(t, what, &bytes, want)?;
        refused.push((what, want.to_owned()));
    }
    let (kept, _) = slots::read_slot(store, 1).ok_or(format!("{slot} wasn't kept"))?;
    if kept != saved {
        return Err(format!("{slot} read back isn't the save written"));
    }
    t.load_record(&kept)?;
    agree(t, "loading slot 1")?;
    let loaded_at = t.frame();
    // Downloaded as a file, and loaded back from one after wandering off.
    let exported = store.export(&slot, &export_name(&model, 1))?;
    let file = format!("{IMPORTS}{}", export_name(&model, 1));
    store.write(&file, &saved)?;
    let picked = store
        .import()?
        .ok_or("nothing to import after handing a file over")?;
    store.delete(&file)?;
    if picked.bytes != saved {
        return Err(format!(
            "the file loaded ({}) isn't slot 1 as downloaded",
            picked.file_name
        ));
    }
    wander(t)?;
    t.load_record(&picked.bytes)?;
    agree(t, "loading slot 1 from a file")?;
    let damaged = format!("{IMPORTS}{model}-damaged.gpsave");
    store.write(&damaged, &saved[..10])?;
    let picked = store
        .import()?
        .ok_or("nothing to import after handing a damaged file over")?;
    store.delete(&damaged)?;
    refuses(t, "a damaged file's", &picked.bytes, NOT_A_SAVE)?;
    refused.push(("a damaged file's", NOT_A_SAVE.to_owned()));
    // Resume puts the autosave back.
    let (auto_kept, _) =
        slots::read_slot(store, AUTOSAVE_SLOT).ok_or("the autosave wasn't kept")?;
    t.load_record(&auto_kept)?;
    agree(t, "loading Resume")?;
    let resumed_at = t.frame();
    shell_settings(store)?;
    ring.clear();
    play(t, n, &mut ring)?;
    for s in [1, AUTOSAVE_SLOT] {
        slots::delete_slot(store, s)?;
        let name = slot_name(s);
        if store.read(&name).is_some()
            || store
                .list(SAVES)
                .iter()
                .any(|x| *x == name || *x == slots::when_name(s))
        {
            return Err(format!("{name} is still kept after deleting it"));
        }
    }
    Ok(Checked {
        frames: n,
        saved_at: f0 + a as u64,
        rewound_from,
        rewound_to,
        loaded_at,
        refused,
        resumed_at,
        exported,
    })
}

/// Loads `bytes`, which must be refused with `want` and leave the game
/// where it was.
fn refuses(t: &mut dyn Target, what: &str, bytes: &[u8], want: &str) -> Result<(), String> {
    let here = t.hash();
    match t.load_record(bytes) {
        Ok(()) => return Err(format!("{what} save loaded; it must be refused")),
        Err(e) if e == want => {}
        Err(e) => {
            return Err(format!(
                "{what} save was refused with \"{e}\", not \"{want}\""
            ));
        }
    }
    if t.hash() != here {
        return Err(format!("refusing {what} save changed the game"));
    }
    Ok(())
}

/// The shell's settings through the target's storage: off by default,
/// kept and read back, "Console only" turning off every save, load and
/// rewind binding, and the quick save buttons acting only once turned on.
/// What the player kept is put back.
fn shell_settings(store: &mut dyn Storage) -> Result<(), String> {
    let before = store.read(slots::SETTINGS_NAME);
    if before.is_none() && Settings::read(store) != Settings::default() {
        return Err("with nothing kept, the shell's settings aren't the defaults".into());
    }
    let extras = |b: &&keys::Binding| {
        ["save-", "load-", "rewind", "quick-"]
            .iter()
            .any(|p| b.action.starts_with(p))
    };
    for want in [
        Settings {
            console_only: true,
            quick_buttons: true,
        },
        Settings {
            console_only: false,
            quick_buttons: true,
        },
        Settings::default(),
    ] {
        want.write(store)?;
        let got = Settings::read(store);
        if got != want {
            return Err(format!(
                "the shell's settings read back as {got:?}, not {want:?}"
            ));
        }
        for b in keys::BINDINGS.iter().filter(extras) {
            let quick = b.action.starts_with("quick-");
            let should = !got.console_only && (!quick || got.quick_buttons);
            if got.allows(b) != should {
                return Err(format!(
                    "{} {} under {got:?}; it must {}",
                    b.action,
                    if got.allows(b) { "acts" } else { "is off" },
                    if should { "act" } else { "be off" }
                ));
            }
        }
    }
    match before {
        Some(b) => store.write(slots::SETTINGS_NAME, &b),
        None => store.delete(slots::SETTINGS_NAME),
    }
}

/// A made-up input for the shell check where a game has none of its own:
/// `n` frames that walk, jump and stand still in turn, with no reset.
pub fn made_up_input(n: usize) -> Vec<Input> {
    // Right, Right with A, nothing, Left with B, A alone; 20 frames each.
    const PATTERN: [u16; 5] = [0x80, 0x81, 0x00, 0x42, 0x01];
    (0..n)
        .map(|i| (PATTERN[i / 20 % PATTERN.len()], false))
        .collect()
}

#[cfg(test)]
mod tests;
