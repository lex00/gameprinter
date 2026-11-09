//! Spec variants for the searches ([`crate::reach`], [`crate::difficulty`],
//! [`crate::stage`]).
//!
//! The searches edit a [`Spec`] before stepping it: the player alone, a level
//! made the start, every other kind's `contact` taken away. A game's hooks run
//! in its module, which compiles its own spec from the bytes `load` was given,
//! so each edit is also written on the model (or, for [`Edit::AloneFlat`], with
//! a call on the session) and loaded as a variant. The module's digest of the
//! spec it compiled has to equal the host's digest of the edited spec, or the
//! variant isn't the same run.

use std::cell::RefCell;

use gp_model::Model;
use gp_sim::spec::{Spec, State, System, Tile};
use gp_sim::{Event, Exts, Fx, World, step_with_ext};

/// How a search changes the model it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edit {
    /// The model as it is.
    Full,
    /// The model starting at this level, an index into `Spec::levels`.
    StartAt(usize),
    /// The level alone with its player, at power-on: every other spawn gone,
    /// and each game machine that starts holding the player still moved to
    /// the first of its states that doesn't.
    Alone(usize),
    /// [`Edit::Alone`] on a flat floor ([`flat_floor`]). The floor is the
    /// compiled level's cells, which model text can't name: the edit keeps the
    /// level's other compiled fields, its ground boxes and tile surfaces.
    /// The module takes it as the `flat-level` call after `load`.
    AloneFlat(usize),
    /// Nothing touches the player: every kind but this one loses its own
    /// `contact`. A kind's machine states keep the contact they compiled to.
    Contactless(usize),
}

impl Edit {
    /// The spec this edit makes of `base`.
    pub fn spec(&self, base: &Spec) -> Result<Spec, String> {
        let mut s = base.clone();
        match *self {
            Edit::Full => {}
            Edit::StartAt(level) => s.start_level = level,
            Edit::Alone(level) => alone(&mut s, level)?,
            Edit::AloneFlat(level) => {
                alone(&mut s, level)?;
                flat_floor(&mut s, level)?;
            }
            Edit::Contactless(player) => {
                for (k, kind) in s.kinds.iter_mut().enumerate() {
                    if k != player {
                        kind.contact = None;
                    }
                }
            }
        }
        Ok(s)
    }

    /// The model text this edit is, for a module's `load`. [`Edit::AloneFlat`]
    /// is the same text as [`Edit::Alone`]; its floor comes after `load`
    /// ([`flat_floor`]).
    pub fn model(&self, model: &Model, base: &Spec) -> Result<Model, String> {
        let mut m = model.clone();
        match *self {
            Edit::Full => {}
            Edit::StartAt(level) => m.model.start_level = Some(level_name(base, level)?),
            Edit::Alone(level) | Edit::AloneFlat(level) => {
                let name = level_name(base, level)?;
                m.model.start_level = Some(name.clone());
                let kind = crate::reach::player_kind(base, level)?;
                let player = base.kinds[kind].name.clone();
                let lv = m
                    .levels
                    .get_mut(&name)
                    .ok_or_else(|| format!("the model has no level `{name}`"))?;
                let spawn = lv
                    .core
                    .spawn
                    .iter()
                    .find(|s| s.entity == player)
                    .ok_or(format!("level {name}: no spawn of `{player}`"))?;
                if spawn.table.is_some() {
                    return Err(format!(
                        "level {name}: `{player}` spawns from a table, which the search can't take a single spawn of in model text"
                    ));
                }
                lv.core.spawn = vec![spawn.clone()];
                let edited = self.spec(base)?;
                for machine in &edited.machines {
                    let rule = m
                        .rules
                        .get_mut(&machine.name)
                        .ok_or(format!("the model has no rule `{}`", machine.name))?;
                    rule.initial = machine.states[machine.initial].name.clone();
                }
            }
            Edit::Contactless(player) => {
                let keep = base.kinds[player].name.clone();
                for (name, e) in &mut m.entities {
                    if *name == keep {
                        continue;
                    }
                    if e.platformer.contact.is_none() {
                        continue;
                    }
                    // A state keeps the contact it compiled to: written whole
                    // in the state's `set`, the state doesn't need the base's.
                    let whole: Vec<(String, Option<toml::Value>)> = match &e.core.machine {
                        Some(machine) => machine
                            .states
                            .keys()
                            .map(|s| {
                                let merged = e
                                    .in_state(s)
                                    .map_err(|err| format!("entities.{name}.{s}: {err}"))?;
                                let table = toml::Table::try_from(&merged)
                                    .map_err(|err| format!("entities.{name}.{s}: {err}"))?;
                                Ok::<_, String>((s.clone(), table.get("contact").cloned()))
                            })
                            .collect::<Result<_, _>>()?,
                        None => Vec::new(),
                    };
                    e.platformer.contact = None;
                    if let Some(machine) = &mut e.core.machine {
                        for (s, contact) in whole {
                            let set = &mut machine
                                .states
                                .get_mut(&s)
                                .expect("the state just listed")
                                .set;
                            match contact {
                                Some(c) => {
                                    set.insert("contact".to_owned(), c);
                                }
                                None => {
                                    set.remove("contact");
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(m)
    }
}

fn level_name(spec: &Spec, level: usize) -> Result<String, String> {
    spec.levels
        .get(level)
        .map(|l| l.name.clone())
        .ok_or(format!("there's no level {level}"))
}

/// [`Edit::Alone`] on the spec.
fn alone(s: &mut Spec, level: usize) -> Result<(), String> {
    let kind = crate::reach::player_kind(s, level)?;
    s.start_level = level;
    s.levels[level].spawns.retain(|sp| sp.kind == kind);
    s.levels[level].spawns.truncate(1);
    // The search asks what the player's own moves reach, so a game machine
    // that starts in a state holding the player still (a title screen)
    // starts in the first of its states that lets the player move.
    for m in &mut s.machines {
        let holds = |st: &State| st.freeze || st.part.stops.contains(&System::Player);
        if m.states.get(m.initial).is_some_and(holds)
            && let Some(i) = m.states.iter().position(|st| !holds(st))
        {
            m.initial = i;
        }
    }
    Ok(())
}

/// Puts a level on a flat floor for [`crate::reach::envelope`]: 512 by 16
/// cells of the level's tile size, one row of solid cells at the bottom, no
/// pit and no goal, with the first spawn four tiles in at the top.
pub fn flat_floor(s: &mut Spec, level: usize) -> Result<(), String> {
    let l = s
        .levels
        .get_mut(level)
        .ok_or(format!("there's no level {level}"))?;
    let (w, h) = (512, 16);
    let mut tiles = vec![Tile::Empty; (w * h) as usize];
    for c in 0..w {
        tiles[((h - 1) * w + c) as usize] = Tile::Solid;
    }
    let ts = l.tile_size;
    l.size = [w, h];
    l.tiles = tiles;
    l.kill_below = None;
    l.goal_x = None;
    let spawn = l
        .spawns
        .first_mut()
        .ok_or(format!("level {}: no spawn to place", l.name))?;
    spawn.x = Fx::from_int(ts * 4);
    spawn.y = Fx::from_int(0);
    Ok(())
}

/// Steps a world the search holds: the model's hooks run on it through the
/// game's module, or natively when there are none.
pub trait Stepper {
    fn step(&mut self, spec: &Spec, w: &mut World, buttons: u8) -> Vec<Event>;
}

/// A spec variant and what steps worlds of it.
pub struct Variant<'a> {
    pub spec: Spec,
    stepper: RefCell<Box<dyn Stepper + 'a>>,
}

impl<'a> Variant<'a> {
    pub fn new(spec: Spec, stepper: Box<dyn Stepper + 'a>) -> Self {
        Self {
            spec,
            stepper: RefCell::new(stepper),
        }
    }

    /// One frame of `w` with these buttons, and its events.
    pub fn step(&self, w: &mut World, buttons: u8) -> Vec<Event> {
        self.stepper.borrow_mut().step(&self.spec, w, buttons)
    }
}

impl std::ops::Deref for Variant<'_> {
    type Target = Spec;
    fn deref(&self) -> &Spec {
        &self.spec
    }
}

/// How a search opens a variant of the model it was given.
pub trait Sim {
    fn open(&self, base: &Spec, edit: &Edit) -> Result<Variant<'_>, String>;
}

/// The simulation in this process, with the hooks `exts`.
pub struct NativeSim<'a>(pub &'a Exts);

struct NativeStepper<'a>(&'a Exts);

impl Stepper for NativeStepper<'_> {
    fn step(&mut self, spec: &Spec, w: &mut World, buttons: u8) -> Vec<Event> {
        step_with_ext(spec, w, buttons, self.0)
    }
}

/// A variant stepped in this process, with the hooks `exts`.
pub fn native<'a>(exts: &'a Exts, base: &Spec, edit: &Edit) -> Result<Variant<'a>, String> {
    Ok(Variant::new(
        edit.spec(base)?,
        Box::new(NativeStepper(exts)),
    ))
}

impl Sim for NativeSim<'_> {
    fn open(&self, base: &Spec, edit: &Edit) -> Result<Variant<'_>, String> {
        native(self.0, base, edit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::compile;
    use crate::{Lifted, reach};
    use std::collections::BTreeMap;

    const MODEL: &str = include_str!("../../../templates/mosshop/model.toml");
    const MAP: &str = include_str!("../../../assets/mosshop.toml");

    fn lifted(level: String) -> Lifted {
        let mut files = BTreeMap::new();
        files.insert("assets.toml".to_owned(), MAP.to_owned());
        files.insert(
            "metatiles.toml".to_owned(),
            reach::tests::METATILES.to_owned(),
        );
        files.insert("slugs.toml".to_owned(), reach::tests::SLUGS.to_owned());
        files.insert("level.toml".to_owned(), level);
        Lifted { files }
    }

    fn digest(s: &Spec) -> String {
        gp_sim::save::spec_digest(s)
    }

    /// A variant is real only when the model text and the spec edit compile
    /// to the same spec. Every edit but the flat floor is checked here on
    /// Moss Hop, a model with no hooks and no machines.
    #[test]
    fn each_edit_of_model_text_compiles_to_the_edited_spec() {
        let model = gp_model::Model::parse(MODEL).unwrap();
        let assets = lifted(reach::tests::level(0, &[]));
        let base = compile(&model, &assets).unwrap();
        for edit in [
            Edit::Full,
            Edit::StartAt(0),
            Edit::Alone(0),
            Edit::Contactless(reach::player_kind(&base, 0).unwrap()),
        ] {
            let edited = edit.spec(&base).unwrap();
            let text = edit.model(&model, &base).unwrap();
            let round = gp_model::Model::parse(&text.to_toml()).unwrap();
            let compiled = compile(&round, &assets).unwrap();
            assert_eq!(digest(&compiled), digest(&edited), "{edit:?}");
        }
    }

    /// `Contactless` removes a kind's own contact but keeps what each of its
    /// states compiled to, as the search's spec edit does: on a model whose
    /// enemy has a machine, the text edit compiles to the same spec.
    #[test]
    fn contactless_text_keeps_a_states_contact() {
        const SHELL: &str = r#"
[entities.shell_walker]
role = "enemy"
size = [16, 16]
tile_collision = true
hitbox = { x = 2, y = 4, w = 12, h = 12 }
walk = { speed = "0.5" }
fall = { gravity = "0.25", max_fall = 4 }
contact = { stomp_depth = 7, bounce = -3, keep_on_stomp = true, otherwise = "kill-player" }

[entities.shell_walker.machine]
initial = "walking"

[[entities.shell_walker.machine.states.walking.transition]]
on = "enemy.stomped"
to = "still"

[entities.shell_walker.machine.states.still.set]
walk = { speed = 0 }
contact = { otherwise = "kick" }

[[levels.main.spawn]]
entity = "shell_walker"
x = 320
y = 176
"#;
        let model = gp_model::Model::parse(&format!("{MODEL}\n{SHELL}")).unwrap();
        let assets = lifted(reach::tests::level(2, &[]));
        let base = compile(&model, &assets).unwrap();
        let edit = Edit::Contactless(reach::player_kind(&base, 0).unwrap());
        let edited = edit.spec(&base).unwrap();
        let k = edited
            .kinds
            .iter()
            .position(|k| k.name == "shell_walker")
            .unwrap();
        assert!(edited.kinds[k].contact.is_none());
        assert!(
            edited.kinds[k].machine.as_ref().unwrap().states[0]
                .kind
                .contact
                .is_some()
        );
        let text = edit.model(&model, &base).unwrap();
        let round = gp_model::Model::parse(&text.to_toml()).unwrap();
        let compiled = compile(&round, &assets).unwrap();
        assert_eq!(digest(&compiled), digest(&edited));
    }
}
