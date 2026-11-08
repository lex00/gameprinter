//! The save slots as a player sees them, and the shell's own settings
//! (docs/shell.md, "Saves on the settings screen"). Both are written here
//! once: a target hands over its [`Storage`] and the settings screen
//! (`crate::settings`) lists what this module reads from it.
//!
//! A slot is a save record (`gp_sim::save`) under [`slot_name`], with a
//! stamp beside it under [`when_name`]: the seconds since 1970 (UTC) when
//! it was saved, as text. The stamp is the shell's, read from the host's
//! clock, and the simulation never reads it. Everything else a slot line
//! shows (its level, play time and mode) comes from the record and its
//! state.

use gp_sim::Spec;
use gp_sim::save::{OTHER_BUILD, OTHER_GAME, Record, VERSION};

use super::keys::{Binding, Needs};
use super::{AUTOSAVE_SLOT, QUICK_SLOTS, SAVES, Storage, slot_name};

/// The storage name of the stamp kept beside a slot.
pub fn when_name(slot: usize) -> String {
    format!("{SAVES}slot-{slot}.when")
}

/// Keeps a save record in a slot, with the time it was saved when the host
/// has a clock (`now`, seconds since 1970). The record goes first, so a
/// stamp is never kept without its save.
pub fn write_slot(
    store: &mut dyn Storage,
    slot: usize,
    record: &[u8],
    now: Option<u64>,
) -> Result<(), String> {
    store.write(&slot_name(slot), record)?;
    match now {
        Some(t) => store.write(&when_name(slot), t.to_string().as_bytes()),
        None => store.delete(&when_name(slot)),
    }
}

/// Removes a slot and its stamp.
pub fn delete_slot(store: &mut dyn Storage, slot: usize) -> Result<(), String> {
    store.delete(&slot_name(slot))?;
    store.delete(&when_name(slot))
}

/// A slot's record and its stamp, or None when the slot is empty.
pub fn read_slot(store: &mut dyn Storage, slot: usize) -> Option<(Vec<u8>, Option<u64>)> {
    let bytes = store.read(&slot_name(slot))?;
    let when = store
        .read(&when_name(slot))
        .and_then(|b| String::from_utf8(b).ok())
        .and_then(|s| s.trim().parse().ok());
    Some((bytes, when))
}

/// What a kept slot holds, as its line shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotSave {
    /// The frame the state is at, which gives the play time.
    pub frame: u64,
    /// The mode it was played in (the record's profile).
    pub profile: String,
    /// The level it is at, counted from 1, when the game has more than one
    /// and this build can read the state.
    pub level: Option<usize>,
    /// When it was saved, in seconds since 1970, when a stamp is kept.
    pub saved_at: Option<u64>,
    /// The sentence a load gets, when this build can tell it is refused
    /// (a save of another version or another game).
    pub refused: Option<&'static str>,
}

/// A slot's line on the settings screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotLine {
    pub slot: usize,
    /// None for an empty slot.
    pub save: Option<SlotSave>,
    /// The bytes kept are no save record at all.
    pub damaged: bool,
}

impl SlotLine {
    pub fn empty(slot: usize) -> SlotLine {
        SlotLine {
            slot,
            save: None,
            damaged: false,
        }
    }

    /// A slot's line from what is kept: the record, its stamp, and what
    /// the host knows. `model` is the game's name. `own` is the profile the
    /// game runs in and this build's digest in it: a record of that profile
    /// with another digest is from another version. `spec` reads the level
    /// from the state, when it is of this build.
    pub fn read(
        slot: usize,
        bytes: Option<&[u8]>,
        saved_at: Option<u64>,
        model: &str,
        own: Option<(&str, &str)>,
        spec: Option<&Spec>,
    ) -> SlotLine {
        let Some(bytes) = bytes else {
            return SlotLine::empty(slot);
        };
        let Ok(r) = Record::read(bytes) else {
            return SlotLine {
                slot,
                save: None,
                damaged: true,
            };
        };
        let refused = if r.model != model {
            Some(OTHER_GAME)
        } else if r.version != VERSION || own.is_some_and(|(p, d)| p == r.profile && d != r.digest)
        {
            Some(OTHER_BUILD)
        } else {
            None
        };
        let frame = r
            .state
            .get(8..16)
            .and_then(|b| b.try_into().ok())
            .map(u64::from_le_bytes)
            .unwrap_or(0);
        let level = match (spec, refused) {
            (Some(spec), None) if spec.levels.len() > 1 => {
                gp_sim::World::from_canonical(spec, &r.state)
                    .ok()
                    .map(|w| w.level + 1)
            }
            _ => None,
        };
        SlotLine {
            slot,
            save: Some(SlotSave {
                frame,
                profile: r.profile,
                level,
                saved_at,
                refused,
            }),
            damaged: false,
        }
    }

    /// The slot's name: "Resume" for the autosave, else "Slot N".
    pub fn label(&self) -> String {
        if self.slot == AUTOSAVE_SLOT {
            "Resume".to_owned()
        } else {
            format!("Slot {}", self.slot)
        }
    }
}

/// The lines the settings screen lists: the autosave and the player's
/// slots 1 to [`QUICK_SLOTS`], read from `store`.
pub fn slot_lines(
    store: &mut dyn Storage,
    model: &str,
    own: Option<(&str, &str)>,
    spec: Option<&Spec>,
) -> Vec<SlotLine> {
    (AUTOSAVE_SLOT..=QUICK_SLOTS)
        .map(|slot| {
            let kept = read_slot(store, slot);
            let when = kept.as_ref().and_then(|(_, w)| *w);
            SlotLine::read(
                slot,
                kept.as_ref().map(|(b, _)| b.as_slice()),
                when,
                model,
                own,
                spec,
            )
        })
        .collect()
}

/// Play time from a frame count at `rate` frames a second, as `m:ss` (or
/// `h:mm:ss`).
pub fn play_time(frame: u64, rate: f64) -> String {
    let s = (frame as f64 / rate.max(1.0)) as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// How long ago a stamp was, in plain words: "just now", "5 min ago", "2
/// hours ago", "3 days ago".
pub fn ago(now: u64, then: u64) -> String {
    let s = now.saturating_sub(then);
    match s {
        0..60 => "just now".to_owned(),
        60..3600 => format!("{} min ago", s / 60),
        3600..7200 => "1 hour ago".to_owned(),
        7200..86400 => format!("{} hours ago", s / 3600),
        86400..172800 => "yesterday".to_owned(),
        _ => format!("{} days ago", s / 86400),
    }
}

/// A mode's name as a player reads it: `normal` is "Normal", and a set of
/// switches joined by `+` is "Custom".
pub fn mode_title(profile: &str) -> String {
    if profile.is_empty() {
        return "Retro".to_owned();
    }
    if profile.contains('+') {
        return "Custom".to_owned();
    }
    let p = profile.replace('_', " ");
    let mut c = p.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// Where the shell's settings are kept, as a [`Storage`] name.
pub const SETTINGS_NAME: &str = "settings/shell.txt";

/// The shell's own settings: switches of the shell, never of the model,
/// so changing one never restarts the game. Kept through [`Storage`] as
/// [`SETTINGS_NAME`], beside the theme (docs/shell.md).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Settings {
    /// "Console only": no saves, loads or rewind, as the console had none.
    pub console_only: bool,
    /// "Quick save buttons": quick save and quick load on two more buttons
    /// (the bumpers), off by default since a stray press would replace a
    /// slot.
    pub quick_buttons: bool,
}

impl Settings {
    /// The settings as their file holds them: a `name = true|false` line
    /// each.
    pub fn to_text(self) -> String {
        format!(
            "console_only = {}\nquick_buttons = {}\n",
            self.console_only, self.quick_buttons
        )
    }

    /// Settings from their file. A line not understood is left out, so a
    /// file from a later build reads as what this build knows of it.
    pub fn parse(text: &str) -> Settings {
        let mut s = Settings::default();
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let on = v.trim() == "true";
            match k.trim() {
                "console_only" => s.console_only = on,
                "quick_buttons" => s.quick_buttons = on,
                _ => {}
            }
        }
        s
    }

    /// The settings kept, or the defaults when none are.
    pub fn read(store: &mut dyn Storage) -> Settings {
        store
            .read(SETTINGS_NAME)
            .and_then(|b| String::from_utf8(b).ok())
            .map(|t| Settings::parse(&t))
            .unwrap_or_default()
    }

    pub fn write(self, store: &mut dyn Storage) -> Result<(), String> {
        store.write(SETTINGS_NAME, self.to_text().as_bytes())
    }

    /// Whether a binding of the key table acts under these settings.
    pub fn allows(self, b: &Binding) -> bool {
        match b.needs {
            Needs::Always => true,
            Needs::NotConsoleOnly => !self.console_only,
            Needs::QuickButtons => self.quick_buttons && !self.console_only,
        }
    }

    /// Whether the slots can be saved to and loaded from.
    pub fn saves(self) -> bool {
        !self.console_only
    }
}
