//! Scoring the detectors against a labelled reference set (docs/gaps.md,
//! "Scoring"). A reference set names, for one game, what the detectors
//! should find: the sound interfaces and effects, the interactive elements
//! with their handler routines and sounds, and the RAM variables that matter
//! (and the per-frame bookkeeping that doesn't). It holds names and the IDs
//! of the disassembly's constants only, never game data, and lives in
//! `crates/gp-gaps/truth/<template>.toml`.
//!
//! Each detector gets a precision (the share of what it found that the set
//! names) and a recall (the share of what the set names, and the recording
//! exercised, that it found). What the recording exercised comes from the
//! code signals, not from the detector: a handler ran in play, a queue was
//! written with the effect's ID, a play-sound routine was called with it.
//! Scoring reads the run; it never changes what the detectors find.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::Run;
use crate::changes::Report;
use crate::interactive::Catalog;

/// RAM items read from the top of the RAM place.
pub const RAM_TOP: usize = 25;

/// The reference sets kept with the crate, by template. Other games' sets
/// (Super Mario Bros., Mega Man 2, Super Mario Bros. 2) stayed in game-printer.
pub fn builtin(template: &str) -> Option<&'static str> {
    match template {
        "mosshop" => Some(include_str!("../truth/mosshop.toml")),
        _ => None,
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Truth {
    pub schema: u32,
    pub template: String,
    #[serde(default)]
    pub sound: Sound,
    #[serde(default, rename = "element")]
    pub elements: Vec<Element>,
    #[serde(default)]
    pub ram: Ram,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sound {
    #[serde(default)]
    pub interfaces: Vec<String>,
    /// Interfaces all of whose requests are music.
    #[serde(default)]
    pub music: Vec<String>,
    #[serde(default)]
    pub effects: Vec<Effect>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Effect {
    pub via: String,
    pub id: u8,
    pub name: String,
    #[serde(default)]
    pub music: bool,
    /// An engine command (stop, fade), neither music nor an effect.
    #[serde(default)]
    pub command: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Element {
    pub name: String,
    #[serde(default)]
    pub handlers: Vec<String>,
    #[serde(default)]
    pub sounds: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ram {
    #[serde(default)]
    pub matter: Vec<String>,
    #[serde(default)]
    pub bookkeeping: Vec<String>,
}

impl Truth {
    pub fn parse(text: &str) -> Result<Self, String> {
        let t: Self = toml::from_str(text).map_err(|e| e.to_string())?;
        if t.schema != 0 {
            return Err(format!("reference set schema {} (this reads 0)", t.schema));
        }
        Ok(t)
    }

    fn effect(&self, name: &str) -> Vec<&Effect> {
        self.sound
            .effects
            .iter()
            .filter(|e| e.name == name)
            .collect()
    }

    /// Whether the effect is music.
    pub fn is_music(&self, e: &Effect) -> bool {
        e.music || self.sound.music.contains(&e.via)
    }
}

/// One detector's score.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct DetectorScore {
    pub detector: String,
    pub precision: f64,
    pub recall: f64,
    /// Found items the set names, found items it doesn't, and items the
    /// set names that the recording exercised.
    pub true_positives: usize,
    pub false_positives: usize,
    pub exercised: usize,
    /// Up to 12 of each, by name.
    pub false_found: Vec<String>,
    pub missed: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Score {
    pub schema: u32,
    pub template: String,
    pub detectors: Vec<DetectorScore>,
    /// Of the effects found that the set names, the share whose name is
    /// the set's.
    pub effects_named: f64,
    /// Of the effects found that the set names, the share whose music or
    /// effect class (`EffectItem::class`) is the set's.
    pub effects_classed: f64,
    /// Bookkeeping items in the RAM place's top items.
    pub ram_bookkeeping: usize,
    /// Names in the set that no label has (a typo or another revision).
    pub unknown: Vec<String>,
}

impl Score {
    pub fn get(&self, detector: &str) -> Option<&DetectorScore> {
        self.detectors.iter().find(|d| d.detector == detector)
    }
}

fn ratio(a: usize, b: usize) -> f64 {
    if b == 0 {
        1.0
    } else {
        crate::r2(a as f64 / b as f64)
    }
}

fn keep(v: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut v: Vec<String> = v.into_iter().collect::<BTreeSet<_>>().into_iter().collect();
    v.truncate(12);
    v
}

fn detector(
    name: &str,
    found_true: usize,
    found_false: Vec<String>,
    exercised: usize,
    hit: usize,
    missed: Vec<String>,
) -> DetectorScore {
    DetectorScore {
        detector: name.into(),
        precision: ratio(found_true, found_true + found_false.len()),
        recall: ratio(hit, exercised),
        true_positives: found_true,
        false_positives: found_false.len(),
        exercised,
        false_found: keep(found_false),
        missed: keep(missed),
    }
}

/// A found routine name's PRG offset: a label, `Label+n`, or `sub_<hex>`.
pub fn resolve(run: &Run, name: &str) -> Option<u32> {
    let k = &run.known;
    if let Some(l) = k.rom.iter().find(|l| l.name == name) {
        return Some(l.prg);
    }
    if let Some(hex) = name.strip_prefix("sub_") {
        return u32::from_str_radix(hex, 16).ok();
    }
    let (base, off) = name.rsplit_once('+')?;
    let l = k.rom.iter().find(|l| l.name == base)?;
    Some(l.prg + off.parse::<u32>().ok()?)
}

/// The set's routine, as the instruction starts its code reaches.
struct Routine {
    name: String,
    body: BTreeSet<u32>,
}

/// Scores the detectors' findings on `run` against `truth`.
pub fn score(run: &Run, truth: &Truth, changes: &Report, catalog: &Catalog) -> Score {
    let k = &run.known;
    let mut unknown = BTreeSet::new();
    let entries: BTreeSet<u32> = run.calls.iter().map(|c| c.target).collect();
    let mut bodies: BTreeMap<String, Routine> = BTreeMap::new();
    for e in &truth.elements {
        for h in &e.handlers {
            if bodies.contains_key(h) {
                continue;
            }
            match k.rom.iter().find(|l| &l.name == h) {
                Some(l) => {
                    let body = crate::code::body(run, l, &entries);
                    bodies.insert(
                        h.clone(),
                        Routine {
                            name: h.clone(),
                            body,
                        },
                    );
                }
                None => {
                    unknown.insert(h.clone());
                }
            }
        }
    }
    // Which set routine a found routine (by PRG offset) belongs to.
    let owner = |prg: u32| -> Vec<&str> {
        bodies
            .values()
            .filter(|r| r.body.contains(&prg))
            .map(|r| r.name.as_str())
            .collect()
    };
    let element_of_routine = |name: &str| -> BTreeSet<usize> {
        let Some(prg) = resolve(run, name) else {
            return BTreeSet::new();
        };
        let owners = owner(prg);
        truth
            .elements
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                e.handlers
                    .iter()
                    .any(|h| h == name || owners.contains(&h.as_str()))
            })
            .map(|(i, _)| i)
            .collect()
    };
    // A found request's name: the set's effect name, or `<via> <id>`.
    let effect_key = |e: &Effect| format!("{} {:#04x}", e.via, e.id);
    let element_of_request = |req: &str| -> BTreeSet<usize> {
        truth
            .elements
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                e.sounds.iter().any(|s| {
                    req.split('/').any(|r| r == s)
                        || truth.effect(s).iter().any(|x| effect_key(x) == req)
                })
            })
            .map(|(i, _)| i)
            .collect()
    };

    // What the recording exercised, from the code signals.
    let play_entered: BTreeSet<u32> = run
        .frames
        .iter()
        .filter(|f| f.play)
        .filter_map(|f| f.code.as_ref())
        .flat_map(|c| c.entered.iter().map(|e| e.0))
        .collect();
    let ran_in_play = |r: &Routine| r.body.iter().any(|o| play_entered.contains(o));
    let queue_addr = |name: &str| k.ram.iter().find(|l| l.name == name).map(|l| l.addr);
    let mut asked: BTreeSet<(String, u8)> = BTreeSet::new();
    let mut iface_used: BTreeSet<String> = BTreeSet::new();
    for via in &truth.sound.interfaces {
        if let Some(a) = queue_addr(via) {
            for f in &run.frames {
                let Some(c) = &f.code else { continue };
                for w in c
                    .ram
                    .iter()
                    .filter(|w| w.addr == a && w.value != 0 && !w.engine)
                {
                    iface_used.insert(via.clone());
                    for e in truth.sound.effects.iter().filter(|e| &e.via == via) {
                        if w.value == e.id || (e.id.count_ones() == 1 && w.value & e.id != 0) {
                            asked.insert((via.clone(), e.id));
                        }
                    }
                }
            }
        } else if let Some(prg) = k.rom.iter().find(|l| &l.name == via).map(|l| l.prg) {
            for f in &run.frames {
                let Some(c) = &f.code else { continue };
                for &(t, _, a, engine) in &c.calls {
                    if t == prg && !engine {
                        iface_used.insert(via.clone());
                        asked.insert((via.clone(), a));
                    }
                }
            }
            if !run.watched.contains(&prg) {
                // Calls to an unwatched routine aren't listed: count it as
                // used when it ran at all.
                if run.ran(prg) {
                    iface_used.insert(via.clone());
                }
            }
        } else {
            unknown.insert(via.clone());
        }
    }

    let mut out = Score {
        schema: crate::SCHEMA,
        template: truth.template.clone(),
        ..Score::default()
    };

    // Sound interfaces.
    let ifaces: Vec<&crate::sound::Interface> = changes.sound_interfaces.iter().collect();
    let tp = ifaces
        .iter()
        .filter(|i| truth.sound.interfaces.contains(&i.name))
        .count();
    let fp: Vec<String> = ifaces
        .iter()
        .filter(|i| !truth.sound.interfaces.contains(&i.name))
        .map(|i| i.name.clone())
        .collect();
    let hit = iface_used
        .iter()
        .filter(|n| ifaces.iter().any(|i| &&i.name == n))
        .count();
    let missed = iface_used
        .iter()
        .filter(|n| !ifaces.iter().any(|i| &&i.name == n))
        .cloned()
        .collect();
    out.detectors
        .push(detector("sound", tp, fp, iface_used.len(), hit, missed));

    // Sound effects: every request the detector found, by interface and ID.
    let (_, reqs) = crate::sound::detect(run);
    let mut found: BTreeMap<(String, u8), String> = BTreeMap::new();
    for r in &reqs {
        found.entry((r.via.clone(), r.id)).or_insert(r.name.clone());
    }
    let in_set = |via: &str, id: u8| {
        truth
            .sound
            .effects
            .iter()
            .find(|e| e.via == via && e.id == id)
    };
    let mut named = 0;
    let mut tp = 0;
    let mut fp = Vec::new();
    for ((via, id), name) in &found {
        match in_set(via, *id) {
            Some(e) => {
                tp += 1;
                if name.split('/').any(|n| n == e.name) {
                    named += 1;
                }
            }
            None => fp.push(format!("{via} {id:#04x}")),
        }
    }
    out.effects_named = ratio(named, tp);
    let exercised: Vec<&(String, u8)> = asked
        .iter()
        .filter(|(v, i)| in_set(v, *i).is_some())
        .collect();
    let hit = exercised.iter().filter(|x| found.contains_key(x)).count();
    let missed = exercised
        .iter()
        .filter(|x| !found.contains_key(x))
        .map(|(v, i)| in_set(v, *i).map_or_else(|| format!("{v} {i:#04x}"), |e| e.name.clone()))
        .collect();
    out.detectors
        .push(detector("effects", tp, fp, exercised.len(), hit, missed));
    // Music or effect, as the changes report classes each effect.
    let mut classed = 0;
    let mut class_n = 0;
    for item in &changes.sound {
        if let Some(e) = in_set(&item.via, item.id) {
            class_n += 1;
            let want = if e.command {
                "command"
            } else if truth.is_music(e) {
                "music"
            } else {
                "effect"
            };
            if item.class == want {
                classed += 1;
            }
        }
    }
    out.effects_classed = ratio(classed, class_n);

    // Interactive kinds.
    let mut matched: BTreeSet<usize> = BTreeSet::new();
    let mut tp = 0;
    let mut fp = Vec::new();
    for kind in &catalog.kinds {
        let mut els = BTreeSet::new();
        if let Some(r) = &kind.trigger.routine {
            els.extend(element_of_routine(r));
        }
        if let Some(r) = &kind.trigger.request {
            els.extend(element_of_request(r));
        }
        if let Some(r) = &kind.trigger.requester {
            els.extend(element_of_routine(r));
        }
        if els.is_empty() {
            fp.push(kind.kind.clone());
        } else {
            tp += 1;
            matched.extend(els);
        }
    }
    // An element was exercised when one of its routines ran in play or the
    // game asked for one of its sounds.
    let exercised: Vec<usize> = truth
        .elements
        .iter()
        .enumerate()
        .filter(|(_, e)| {
            e.handlers
                .iter()
                .any(|h| bodies.get(h).is_some_and(ran_in_play))
                || e.sounds.iter().any(|s| {
                    truth
                        .effect(s)
                        .iter()
                        .any(|x| asked.contains(&(x.via.clone(), x.id)))
                })
        })
        .map(|(i, _)| i)
        .collect();
    let hit = exercised.iter().filter(|i| matched.contains(i)).count();
    let missed = exercised
        .iter()
        .filter(|i| !matched.contains(i))
        .map(|&i| truth.elements[i].name.clone())
        .collect();
    out.detectors.push(detector(
        "interactive",
        tp,
        fp,
        exercised.len(),
        hit,
        missed,
    ));

    // Handler routines.
    let mut tp = 0;
    let mut fp = Vec::new();
    let mut covered: BTreeSet<&str> = BTreeSet::new();
    let found_handlers: Vec<&String> = catalog
        .handlers
        .iter()
        .map(|h| &h.0)
        .chain(catalog.requesters.iter().map(|r| &r.0))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    for name in found_handlers {
        let owners = resolve(run, name).map(owner).unwrap_or_default();
        let direct = bodies.contains_key(name.as_str());
        if owners.is_empty() && !direct {
            fp.push(name.clone());
        } else {
            tp += 1;
            covered.extend(owners);
            if direct {
                covered.insert(bodies[name.as_str()].name.as_str());
            }
        }
    }
    let ran: Vec<&Routine> = bodies.values().filter(|r| ran_in_play(r)).collect();
    let hit = ran
        .iter()
        .filter(|r| covered.contains(r.name.as_str()))
        .count();
    let missed = ran
        .iter()
        .filter(|r| !covered.contains(r.name.as_str()))
        .map(|r| r.name.clone())
        .collect();
    out.detectors
        .push(detector("handlers", tp, fp, ran.len(), hit, missed));

    // RAM: the top of the RAM place against the variables that matter.
    let matter: BTreeSet<&str> = truth.ram.matter.iter().map(String::as_str).collect();
    let books: BTreeSet<&str> = truth.ram.bookkeeping.iter().map(String::as_str).collect();
    let topn: Vec<&crate::changes::RamItem> = changes.ram.iter().take(RAM_TOP).collect();
    let tp = topn
        .iter()
        .filter(|r| matter.contains(r.name.as_str()))
        .count();
    let fp: Vec<String> = topn
        .iter()
        .filter(|r| !matter.contains(r.name.as_str()))
        .map(|r| r.name.clone())
        .collect();
    out.ram_bookkeeping = topn
        .iter()
        .filter(|r| books.contains(r.name.as_str()))
        .count();
    let present: Vec<&str> = changes
        .ram
        .iter()
        .map(|r| r.name.as_str())
        .filter(|n| matter.contains(n))
        .collect();
    let want = present.len().min(RAM_TOP);
    let missed = present
        .iter()
        .filter(|n| !topn.iter().any(|r| r.name == **n))
        .map(|n| (*n).to_owned())
        .collect();
    out.detectors
        .push(detector("ram", tp, fp, want, tp.min(want), missed));

    for n in truth.ram.matter.iter().chain(&truth.ram.bookkeeping) {
        if !k.ram.iter().any(|l| &l.name == n) {
            unknown.insert(n.clone());
        }
    }
    for e in &truth.sound.effects {
        if !truth.sound.interfaces.contains(&e.via) {
            unknown.insert(format!("{} (interface of {})", e.via, e.name));
        }
    }
    for el in &truth.elements {
        for s in &el.sounds {
            if truth.effect(s).is_empty() {
                unknown.insert(format!("{s} (sound of {})", el.name));
            }
        }
    }
    out.unknown = unknown.into_iter().collect();
    out
}
