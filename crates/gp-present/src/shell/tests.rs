use super::keys::{self, On, When};
use super::*;

/// A made-up game: a frame count and a value each input stirs.
#[derive(Clone)]
struct Toy {
    frame: u64,
    x: u64,
    /// Breaks the toy's load in one way, for the check's own test.
    broken: Option<&'static str>,
}

impl Toy {
    fn state(&self) -> Vec<u8> {
        let mut s = b"TOYSTATE".to_vec();
        s.extend_from_slice(&self.frame.to_le_bytes());
        s.extend_from_slice(&self.x.to_le_bytes());
        s
    }

    fn set(&mut self, s: &[u8]) -> Result<(), String> {
        let n = |at: usize| u64::from_le_bytes(s[at..at + 8].try_into().unwrap());
        if s.len() != 24 {
            return Err("not a toy state".into());
        }
        (self.frame, self.x) = (n(8), n(16));
        Ok(())
    }
}

impl Target for Toy {
    fn step(&mut self, (pads, reset): Input) -> Result<(), String> {
        self.x = self.x.wrapping_mul(31).wrapping_add(u64::from(pads)) ^ u64::from(reset);
        self.frame += 1;
        Ok(())
    }

    fn hash(&self) -> [u8; 32] {
        let mut h = [0; 32];
        h[..8].copy_from_slice(&self.frame.to_le_bytes());
        h[8..16].copy_from_slice(&self.x.to_le_bytes());
        h
    }

    fn frame(&self) -> u64 {
        self.frame
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            profile: "retro".into(),
            state: self.state(),
        }
    }

    fn put(&mut self, snap: &Snapshot) -> Result<(), String> {
        self.set(&snap.state)
    }

    fn save_record(&mut self) -> Result<Vec<u8>, String> {
        Ok(Record::new("toy", "d1", "retro", self.state()).to_bytes())
    }

    fn load_record(&mut self, bytes: &[u8]) -> Result<(), String> {
        if self.broken == Some("loads anything") {
            return Record::read(bytes).and_then(|r| self.set(&r.state));
        }
        let r = Record::load(bytes, "toy", |_| Ok("d1".into()))?;
        if self.broken == Some("loads a frame late") {
            self.set(&r.state)?;
            return self.step((0, false));
        }
        self.set(&r.state)
    }
}

#[test]
fn a_target_that_saves_loads_and_rewinds_right_passes_the_check() {
    let mut toy = Toy {
        frame: 5,
        x: 1,
        broken: None,
    };
    let mut store = Memory::default();
    let c = check(&mut toy, &mut store, &made_up_input(120)).unwrap();
    assert_eq!(
        (c.frames, c.saved_at, c.rewound_from, c.rewound_to),
        (120, 45, 85, 64)
    );
    assert_eq!(c.loaded_at, 45);
    assert_eq!(c.resumed_at, 64);
    assert_eq!(c.exported, "toy-slot1.gpsave");
    assert_eq!(c.refused.len(), 5);
    assert!(
        c.says()
            .contains("another build's save (\"Can't load a save from another version\")")
    );
    // Slot 1 was deleted at the end.
    assert!(store.files.is_empty());
    assert_eq!(toy.frame, 125);
}

#[test]
fn the_check_fails_a_target_that_loads_wrong() {
    let input = made_up_input(60);
    for (broken, says) in [
        ("loads anything", "another build's save loaded"),
        ("loads a frame late", "loading slot 1: frame"),
    ] {
        let mut toy = Toy {
            frame: 0,
            x: 0,
            broken: Some(broken),
        };
        let e = check(&mut toy, &mut Memory::default(), &input).unwrap_err();
        assert!(e.starts_with(says), "{broken}: {e}");
    }
    let mut toy = Toy {
        frame: 0,
        x: 0,
        broken: None,
    };
    assert!(check(&mut toy, &mut Memory::default(), &input[..11]).is_err());
}

#[test]
fn the_ring_keeps_its_step_and_cap_and_steps_back() {
    let mut ring = Ring::new(2, 3);
    let snap = |f: u64| {
        move || Snapshot {
            profile: String::new(),
            state: f.to_le_bytes().to_vec(),
        }
    };
    for f in 1..=10 {
        ring.note_with(f, snap(f));
    }
    // 6, 8 and 10 are kept; 2 and 4 fell off the front.
    assert_eq!(ring.len(), 3);
    assert_eq!(ring.back(10).unwrap().state, 8u64.to_le_bytes());
    assert_eq!(ring.back(8).unwrap().state, 6u64.to_le_bytes());
    assert!(ring.back(6).is_none());
    // A frame not newer than the newest kept is skipped.
    let mut r = Ring::new(2, 3);
    r.note_with(4, snap(4));
    r.note_with(4, snap(99));
    r.note_with(2, snap(2));
    assert_eq!(r.len(), 1);
    // Off.
    let mut off = Ring::new(2, 0);
    off.note_with(2, snap(2));
    assert!(off.is_empty());
    // The standard ring: 20 seconds, every 2 frames.
    assert_eq!(Ring::standard(60.0).cap, 600);
    assert_eq!(Ring::standard(60.0988).cap, 601);
    assert_eq!(Ring::standard(60.0).every, REWIND_EVERY);
    assert!(REWIND_CHOICES.contains(&REWIND_SECONDS));
}

#[test]
fn slots_have_one_name_in_every_target() {
    assert_eq!(slot_name(1), "saves/slot-1.gpsave");
    assert_eq!(slot_name(AUTOSAVE_SLOT), "saves/slot-0.gpsave");
    assert_eq!(export_name("mosshop", 2), "mosshop-slot2.gpsave");
    assert!(valid_name(&slot_name(4)));
    for bad in [
        "",
        "/saves",
        "saves//x",
        "../x",
        "saves/./x",
        "Saves/x",
        "a b",
    ] {
        assert!(!valid_name(bad), "{bad}");
    }
}

#[test]
fn memory_storage_keeps_lists_hands_over_and_takes_files() {
    let mut m = Memory::default();
    m.write("saves/slot-2.gpsave", b"two").unwrap();
    m.write("saves/slot-1.gpsave", b"one").unwrap();
    m.write("settings.txt", b"s").unwrap();
    assert!(m.write("../x", b"").is_err());
    assert_eq!(
        m.list(SAVES),
        ["saves/slot-1.gpsave", "saves/slot-2.gpsave"]
    );
    assert_eq!(m.list("").len(), 3);
    assert_eq!(m.read("saves/slot-1.gpsave").as_deref(), Some(&b"one"[..]));
    assert_eq!(
        m.export("saves/slot-1.gpsave", "g-slot1.gpsave").unwrap(),
        "g-slot1.gpsave"
    );
    assert_eq!(m.exported["g-slot1.gpsave"], b"one");
    assert!(m.export("saves/slot-9.gpsave", "x").is_err());
    m.delete("saves/slot-1.gpsave").unwrap();
    m.delete("saves/slot-1.gpsave").unwrap();
    assert!(m.read("saves/slot-1.gpsave").is_none());
    assert!(m.import().unwrap().is_none());
    m.to_import.push_back(Picked {
        file_name: "a.gpsave".into(),
        bytes: vec![1],
    });
    assert_eq!(m.import().unwrap().unwrap().file_name, "a.gpsave");
    assert!(m.persist());
}

#[test]
fn a_level_start_autosaves() {
    use gp_sim::spec::EventKind;
    let ev = |kind| gp_sim::Event {
        frame: 1,
        kind,
        entity: None,
    };
    assert!(autosaves(&[ev(EventKind::LEVEL_STARTED)]));
    assert!(!autosaves(&[]));
    assert!(!autosaves(&[ev(EventKind(7))]));
}

#[test]
fn no_key_or_button_is_on_two_actions_of_a_target() {
    assert_eq!(keys::clashes(), Vec::<String>::new());
}

#[test]
fn every_button_has_its_place_in_the_standard_layout() {
    for b in keys::BINDINGS {
        for x in b.buttons {
            assert!(keys::standard_index(x).is_some(), "{}: {x}", b.action);
        }
        assert!(b.keys.len() + b.buttons.len() > 0, "{}", b.action);
    }
    assert_eq!(
        keys::standard_index("RightThumb"),
        Some(crate::settings::OPEN_BUTTON_STANDARD)
    );
}

#[test]
fn the_table_keeps_each_target_s_keys() {
    // Both: the NES buttons, the settings key, rewind and the practice keys.
    assert_eq!(keys::keys("a", On::Web), ["KeyX", "Space"]);
    assert_eq!(
        keys::keys("settings", On::Web),
        [crate::settings::OPEN_KEY_MAIN, crate::settings::OPEN_KEY]
    );
    assert_eq!(
        keys::buttons("settings", On::Web),
        [crate::settings::OPEN_BUTTON]
    );
    assert_eq!(keys::keys("rewind", On::Web), ["Backspace"]);
    // B is either face button, and rewind is on the left trigger, in both.
    assert_eq!(keys::buttons("b", On::Web), ["West", "East"]);
    assert_eq!(keys::buttons("rewind", On::Web), ["LeftTrigger2"]);
    // Quick save and load on the bumpers, the preview's, behind their
    // switch.
    assert_eq!(keys::buttons("quick-save", On::Web), ["RightTrigger"]);
    assert_eq!(keys::buttons("quick-load", On::Web), ["LeftTrigger"]);
    // The quick slots are the preview's: Shift+1 saves, 1 loads.
    let save = keys::of(On::Web).find(|b| b.action == "save-1").unwrap();
    assert!(save.shift && save.keys == ["Digit1"]);
    assert_eq!(keys::keys("load-1", On::Web), ["Digit1"]);
    for (i, a) in keys::SCREEN.iter().enumerate() {
        assert!(
            keys::BINDINGS
                .iter()
                .any(|b| b.action == *a && b.when == When::Screen),
            "{i} {a}"
        );
    }
    for a in keys::NES.iter().chain(&keys::NES2) {
        assert!(!keys::keys(a, On::Web).is_empty(), "{a}");
    }
}

#[test]
fn a_slot_line_reads_the_record_and_its_stamp() {
    use slots::{SlotLine, ago, mode_title, play_time};
    let mut state = b"GPSM\0\0\0\0".to_vec();
    state.extend_from_slice(&3600u64.to_le_bytes());
    let rec = Record::new("toy", "d1", "normal", state).to_bytes();
    let mut m = Memory::default();
    slots::write_slot(&mut m, 2, &rec, Some(1000)).unwrap();
    slots::write_slot(&mut m, AUTOSAVE_SLOT, &rec, None).unwrap();
    m.write(&slot_name(3), b"junk").unwrap();
    let lines = slots::slot_lines(&mut m, "toy", Some(("normal", "d1")), None);
    assert_eq!(lines.len(), QUICK_SLOTS + 1);
    assert_eq!(lines[0].label(), "Resume");
    assert_eq!(lines[0].save.as_ref().unwrap().saved_at, None);
    let two = lines[2].save.as_ref().unwrap();
    assert_eq!(
        (two.frame, two.profile.as_str(), two.saved_at, two.refused),
        (3600, "normal", Some(1000), None)
    );
    assert_eq!(lines[2].label(), "Slot 2");
    assert!(lines[1].save.is_none() && !lines[1].damaged);
    assert!(lines[3].damaged);
    // Another build of the same mode: refused, by the same sentence.
    let other = SlotLine::read(1, Some(&rec), None, "toy", Some(("normal", "d2")), None);
    assert_eq!(other.save.unwrap().refused, Some(gp_sim::save::OTHER_BUILD));
    // Another mode can't be told from here; the load decides.
    let mode = SlotLine::read(1, Some(&rec), None, "toy", Some(("retro", "d2")), None);
    assert_eq!(mode.save.unwrap().refused, None);
    slots::delete_slot(&mut m, 2).unwrap();
    assert!(m.read(&slots::when_name(2)).is_none() && slots::read_slot(&mut m, 2).is_none());
    assert_eq!(play_time(3600, 60.0), "1:00");
    assert_eq!(play_time(60 * 3700, 60.0), "1:01:40");
    assert_eq!(ago(1000, 1000), "just now");
    assert_eq!(ago(1000 + 300, 1000), "5 min ago");
    assert_eq!(ago(1000 + 2 * 3600, 1000), "2 hours ago");
    assert_eq!(ago(1000 + 3 * 86400, 1000), "3 days ago");
    assert_eq!(mode_title("normal"), "Normal");
    assert_eq!(mode_title("no_lag+fix"), "Custom");
}

#[test]
fn the_shell_settings_are_kept_and_turn_keys_off() {
    let mut m = Memory::default();
    assert_eq!(Settings::read(&mut m), Settings::default());
    let s = Settings {
        console_only: true,
        quick_buttons: true,
    };
    s.write(&mut m).unwrap();
    assert_eq!(Settings::read(&mut m), s);
    assert!(Settings::parse("quick_buttons = true\nlater = 1\n").quick_buttons);
    let by = |a: &str| keys::BINDINGS.iter().find(|b| b.action == a).unwrap();
    for a in ["save-1", "load-4", "rewind", "quick-save"] {
        assert!(!s.allows(by(a)), "{a} acts under console only");
    }
    assert!(s.allows(by("a")) && s.allows(by("settings")));
    let d = Settings::default();
    assert!(d.allows(by("save-1")) && d.allows(by("rewind")));
    assert!(
        !d.allows(by("quick-load")),
        "quick buttons are off at first"
    );
}
