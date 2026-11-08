//! The interpreter as a runtime: `gp_runtime::Runtime` over a model, so
//! every tool that drives the original game through the runtime protocol
//! drives the model the same way.

use std::sync::Arc;

use gp_model::Model;
use gp_runtime::select::Condition;
use gp_runtime::{
    Capabilities, Entry, Event, InputFrame, PatchResult, Rect, Runtime, Until, Value, input_at,
};
use gp_sim::{Exts, Fx, Spec, World};

use crate::lifted::{Bundle, Lifted};
use crate::state::View;

/// The kind of the event that ends each frame, as in every runtime.
pub const FRAME_KIND: &str = "frame";

/// The path a lockstep `set`s, as it takes its start, to the original's
/// frame there, so a random draws script lines its frames up from that
/// frame and not from the frame the start was asked for: a run from
/// power-on (`from` 0) starts at the original's first polled frame, 3 for
/// Donkey Kong, and its draws came 3 frames off ("the rebuild drew
/// `ladder` on frame 781; the original's next draw is `ladder` on frame
/// 784"). A runtime without draws needn't know the path.
pub const DRAWS_BASE: &str = "draws.base";

/// The path a lockstep `set`s on each of the original's lag frames past
/// its start, which the model doesn't step: the original's frames run one
/// ahead of the model's from then on, so the draws script's do too. With
/// a lag frame not counted, Donkey Kong's game B clip 1 drew `fire_way` a
/// frame early after its lag at 1510 ("the rebuild drew `fire_way` on
/// frame 1520; the original's next draw is `sound_buzz` on frame 1521").
pub const DRAWS_LAG: &str = "draws.lag";

/// A running model.
#[derive(Clone)]
pub struct Interp {
    spec: Arc<Spec>,
    world: World,
    exts: Arc<Exts>,
    /// This frame's state by model path, built on the first `query` after
    /// the world last changed: building it walks every body, and a
    /// comparison queries a frame many times. Every `&mut self` method
    /// clears it.
    view: std::sync::OnceLock<View>,
    /// A random draws script the hooks draw from ([`gp_sim::draws`]), kept
    /// apart from the world so a loaded state doesn't drop it.
    draws: Option<gp_sim::DrawScript>,
    /// Whether each step also gives the APU writes the hooks make on it
    /// ([`APU_KIND`] events), for the lockstep's sound measure.
    apu_events: bool,
}

/// The event kind of an APU write a hook makes ([`Interp::with_apu_events`]),
/// with `reg` (the register's offset from $4000) and `value`, in order. The
/// original's are gp-nes's `apu-write` events, with `addr` and `value`.
pub const APU_KIND: &str = "apu-write";

impl std::fmt::Debug for Interp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Interp")
            .field("model", &self.spec.name)
            .field("frame", &self.world.frame)
            .field("exts", &self.exts.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl Interp {
    /// Compiles the model with its lifted assets and powers it on.
    pub fn load(model: &Model, assets: &Lifted) -> Result<Self, String> {
        Ok(Self::from_spec(crate::compile::compile(model, assets)?))
    }

    pub fn from_bundle(bundle: &Bundle) -> Result<Self, String> {
        Self::load(&bundle.parse_model()?, &bundle.assets)
    }

    pub fn from_spec(spec: Spec) -> Self {
        let world = World::power_on(&spec);
        Self {
            spec: Arc::new(spec),
            world,
            exts: Arc::new(Exts::new()),
            view: std::sync::OnceLock::new(),
            draws: None,
            apu_events: false,
        }
    }

    /// Runs these `ext` hooks from now on, by the names the model gives.
    pub fn with_exts(mut self, exts: Arc<Exts>) -> Self {
        self.exts = exts;
        self
    }

    /// Draws from this random draws script once a state is loaded: each
    /// load is the start of a lockstep from the original's frame `base`, so
    /// a script is for a run with one start, as the free mode has. Before
    /// a load the hooks draw from the generator.
    pub fn with_draws(mut self, draws: gp_sim::DrawScript) -> Self {
        self.draws = Some(draws);
        self
    }

    /// Gives each frame's APU writes as events from now on: what
    /// [`gp_sim::ext::hook_apu`] says the hooks write on the frame, worked
    /// out before it runs, as a runtime that plays them takes them.
    pub fn with_apu_events(mut self) -> Self {
        self.apu_events = true;
        self
    }

    /// The random draws script, with how far the run took it and its first
    /// desync.
    pub fn draws(&self) -> Option<&gp_sim::DrawScript> {
        self.draws.as_ref()
    }

    /// Sets the original's frame the next load starts the draws script at
    /// (also `set` on [`DRAWS_BASE`], which a lockstep does as it takes its
    /// start)
    /// (its `base`), when the start's frame is known only as the run
    /// reaches it: the first compared frame of a run from power-on.
    pub fn set_draws_base(&mut self, base: u64) {
        if let Some(d) = &mut self.draws {
            d.base = base;
        }
    }

    /// The `ext` hooks the runtime steps the model with.
    pub fn exts(&self) -> &Exts {
        &self.exts
    }

    pub fn spec(&self) -> &Spec {
        &self.spec
    }

    pub fn world(&self) -> &World {
        &self.world
    }

    /// The `ext` hooks the model names that the interpreter has no hook
    /// for: those entities do only what their components say.
    pub fn ext_hooks(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for k in &self.spec.kinds {
            let states = k
                .machine
                .iter()
                .flat_map(|m| m.states.iter().map(|s| &s.kind));
            for kind in std::iter::once(k).chain(states) {
                if let Some(e) = &kind.ext
                    && !self.exts.contains_key(e)
                    && !out.contains(&e.as_str())
                {
                    out.push(e);
                }
            }
        }
        out
    }

    /// The state at this frame, by model path.
    pub fn view(&self) -> View {
        View::new(&self.spec, &self.world)
    }

    /// Runs one frame. Controllers 1 and 2 drive a model (controller 2 only
    /// one with core `input.turn`). A reset starts the game over from its
    /// power-on state, but the frame count goes on.
    pub fn advance(&mut self, input: &InputFrame) -> Result<Vec<gp_sim::Event>, String> {
        self.view.take();
        if !input.axes.is_empty() {
            return Err("the model's input device has no analog axes".into());
        }
        if input.buttons > 0xFFFF {
            return Err("a model reads controllers 1 and 2 only".into());
        }
        if input.reset {
            let frame = self.world.frame;
            self.world = World::power_on(&self.spec);
            self.world.frame = frame;
        }
        let [p1, p2, ..] = input.buttons.to_le_bytes();
        Ok(gp_sim::step_pads(
            &self.spec,
            &mut self.world,
            [p1, p2],
            &self.exts,
            &mut self.draws,
        ))
    }

    fn step_frame(&mut self, input: &InputFrame) -> Result<Vec<Event>, String> {
        self.view.take();
        let apu = if self.apu_events {
            // The soft draws the step will take, from the script.
            let next = self.world.frame + 1;
            let draws = self.draws.as_ref();
            let peek = |site: &str| draws.and_then(|d| d.peek(next, site));
            gp_sim::ext::hook_apu_peek(&self.spec, &self.world, &self.exts, &peek)
        } else {
            Vec::new()
        };
        let events = self.advance(input)?;
        let frame = self.world.frame;
        let apu = apu.into_iter().map(|(reg, value)| Event {
            frame,
            kind: APU_KIND.into(),
            entity: None,
            data: vec![
                Entry {
                    path: "reg".into(),
                    value: Value::Int(reg.into()),
                },
                Entry {
                    path: "value".into(),
                    value: Value::Int(value.into()),
                },
            ],
        });
        let mut out: Vec<Event> = apu.collect();
        out.extend(events.into_iter().map(|e| Event {
            frame: e.frame,
            kind: self.spec.events[e.kind.0].clone(),
            entity: e.entity.map(|i| i as u64),
            data: Vec::new(),
        }));
        out.push(Event {
            frame,
            kind: FRAME_KIND.into(),
            entity: None,
            data: vec![Entry {
                path: "polled".into(),
                value: Value::Flag(true),
            }],
        });
        Ok(out)
    }

    fn holds(&self, cond: &Condition) -> Result<bool, String> {
        let view = self.view();
        cond.eval(&mut |sel| view.int(&sel.to_string()))
    }

    /// The part of the world a value at `p` lives in, for `set`.
    fn body_field(&self, p: &str) -> Result<(usize, String), String> {
        let path = gp_model::Path::parse(p)?;
        let seg = &path.segments;
        let bad = || {
            format!(
                "`{p}` isn't a settable value; set takes entities.<kind>[i].<field> or counters.<name>"
            )
        };
        if seg.len() != 3 || seg[0].name != "entities" || seg[0].index.is_some() {
            return Err(bad());
        }
        let kind = self
            .spec
            .kinds
            .iter()
            .position(|k| k.name == seg[1].name)
            .ok_or_else(bad)?;
        let Some(gp_model::path::Index::At(n)) = seg[1].index else {
            return Err(bad());
        };
        let (id, _) = self
            .world
            .of_kind(kind)
            .nth(usize::try_from(n).map_err(|_| bad())?)
            .ok_or_else(|| format!("`{p}`: there's no such entity"))?;
        Ok((id, seg[2].name.clone()))
    }
}

impl Runtime for Interp {
    fn capabilities(&self) -> Capabilities {
        Capabilities::SNAPSHOT
            .union(Capabilities::FORK)
            .union(Capabilities::FULL_MODEL_QUERY)
            .union(Capabilities::EXPLAIN_MODEL)
            .union(Capabilities::LOAD_STATE)
    }

    fn step(&mut self, inputs: &[InputFrame]) -> Result<Vec<Event>, String> {
        self.view.take();
        let mut out = Vec::new();
        for input in inputs {
            out.extend(self.step_frame(input)?);
        }
        Ok(out)
    }

    fn run_until(
        &mut self,
        condition: &str,
        inputs: &[InputFrame],
        max_frames: u32,
    ) -> Result<Until, String> {
        self.view.take();
        let cond = Condition::parse(condition)?;
        let mut events = Vec::new();
        if self.holds(&cond)? {
            return Ok(Until { met: true, events });
        }
        for i in 0..max_frames as usize {
            events.extend(self.step_frame(&input_at(inputs, i))?);
            if self.holds(&cond)? {
                return Ok(Until { met: true, events });
            }
        }
        Ok(Until { met: false, events })
    }

    fn frame(&self) -> u64 {
        self.world.frame
    }

    /// The canonical state, which holds everything that changes.
    fn snapshot(&self) -> Vec<u8> {
        self.world.canonical()
    }

    fn restore(&mut self, snap: &[u8]) -> Result<(), String> {
        self.view.take();
        self.world = World::from_canonical(&self.spec, snap)?;
        Ok(())
    }

    /// The canonical state is the interpreter's snapshot, so it loads in
    /// full: no notes. One of another model, or another state version, is
    /// refused.
    fn load_state(&mut self, state: &[u8]) -> Result<Vec<gp_runtime::LoadNote>, String> {
        self.restore(state)?;
        // The first loaded state is the start a lockstep's `from` names,
        // the script's `base`, whatever the model ran before it; a later
        // one is a resync, from which the script goes on.
        if let Some(d) = &mut self.draws {
            d.resume(self.world.frame);
        }
        Ok(Vec::new())
    }

    fn fork(&self) -> Self {
        self.clone()
    }

    fn canonical_state(&self) -> Vec<u8> {
        self.world.canonical()
    }

    /// The first 8 bytes, little-endian, of the BLAKE3 hash recordings store
    /// for each frame.
    fn state_hash(&self) -> u64 {
        u64::from_le_bytes(self.world.hash()[..8].try_into().expect("8 bytes"))
    }

    fn query(&self, selector: &str) -> Result<Vec<Entry>, String> {
        self.view.get_or_init(|| self.view()).query(selector)
    }

    fn sample(
        &mut self,
        selectors: &[String],
        inputs: &[InputFrame],
        frames: u32,
    ) -> Result<Vec<Vec<Entry>>, String> {
        self.view.take();
        for s in selectors {
            gp_model::Path::parse(s)?;
        }
        let mut rows = Vec::with_capacity(frames as usize);
        for i in 0..frames as usize {
            self.step_frame(&input_at(inputs, i))?;
            let view = self.view();
            let mut row = Vec::new();
            for s in selectors {
                row.extend(view.query(s)?);
            }
            rows.push(row);
        }
        Ok(rows)
    }

    /// Active entities whose hitbox (as their state makes it) overlaps the
    /// area, by ID.
    fn entities_at(&self, area: Rect) -> Vec<u64> {
        let (ax0, ay0) = (area.x, area.y);
        let (ax1, ay1) = (
            area.x.saturating_add(area.w as i64),
            area.y.saturating_add(area.h as i64),
        );
        self.world
            .bodies
            .iter()
            .enumerate()
            .filter(|(_, b)| b.active())
            .filter(|(_, b)| {
                let h = self.spec.kind(b).hitbox;
                let x0 = b.x.add(h.x).floor();
                let y0 = b.y.add(h.y).floor();
                let (x1, y1) = (x0 + h.w.floor(), y0 + h.h.floor());
                x0 < ax1 && ax0 < x1 && y0 < ay1 && ay0 < y1
            })
            .map(|(i, _)| i as u64)
            .collect()
    }

    fn patch(&mut self, _model_diff: &str) -> PatchResult {
        self.view.take();
        PatchResult::NeedsReload
    }

    /// Sets an entity's `x`, `y`, `vx`, `vy`, `on_ground` or `alive`, or a
    /// counter.
    fn set(&mut self, p: &str, v: Value) -> Result<(), String> {
        self.view.take();
        if p == DRAWS_BASE {
            self.set_draws_base(
                u64::try_from(v.as_int().ok_or("a frame is a number")?).unwrap_or(0),
            );
            return Ok(());
        }
        if p == DRAWS_LAG {
            if let Some(d) = &mut self.draws {
                d.base += 1;
            }
            return Ok(());
        }
        if let Some(name) = p.strip_prefix("counters.") {
            let i = self
                .spec
                .counters
                .index_of(name)
                .ok_or(format!("there's no counter `{name}`"))?;
            self.world.counters[i] = v.as_int().ok_or("a counter is a number")?;
            return Ok(());
        }
        let (id, field) = self.body_field(p)?;
        let b = &mut self.world.bodies[id];
        let fixed = |v: &Value| match v {
            Value::Fixed(raw) => Ok(Fx::from_raw(*raw)),
            Value::Int(n) => Ok(Fx::from_int(*n)),
            _ => Err(format!("`{p}` takes a number")),
        };
        let flag = |v: &Value| match v {
            Value::Flag(f) => Ok(*f),
            _ => Err(format!("`{p}` takes a flag")),
        };
        match field.as_str() {
            "x" => b.x = fixed(&v)?,
            "y" => b.y = fixed(&v)?,
            "vx" => b.vx = fixed(&v)?,
            "vy" => b.vy = fixed(&v)?,
            "on_ground" => b.on_ground = flag(&v)?,
            "alive" => b.alive = flag(&v)?,
            _ => return Err(format!("`{p}` isn't a settable value")),
        }
        Ok(())
    }

    /// The model parts behind a state value: an entity field's components,
    /// a rule or counter's definition.
    fn explain(&self, p: &str) -> Vec<String> {
        let Ok(path) = gp_model::Path::parse(p) else {
            return Vec::new();
        };
        let seg = &path.segments;
        let name = |i: usize| seg.get(i).map(|s| s.name.as_str());
        match (name(0), name(1), name(2)) {
            (Some("entities"), Some(kind), field) => {
                let Some(k) = self.spec.kinds.iter().find(|k| k.name == kind) else {
                    return Vec::new();
                };
                // The entity as each of its states makes it.
                let forms: Vec<&gp_sim::spec::Kind> = match &k.machine {
                    Some(m) => m.states.iter().map(|s| &s.kind).collect(),
                    None => vec![k],
                };
                let has = |part: &str, f: &dyn Fn(&gp_sim::spec::Kind) -> bool| {
                    forms
                        .iter()
                        .any(|k| f(k))
                        .then(|| format!("entities.{kind}.{part}"))
                };
                let mut out = Vec::new();
                match field {
                    Some("x") | Some("vx") | Some("facing") => {
                        out.extend(has("run", &|k| k.run.is_some()));
                        out.extend(has("walk", &|k| k.walk.is_some()));
                        out.extend(has("patrol", &|k| k.patrol.is_some()));
                    }
                    Some("y") | Some("vy") | Some("on_ground") => {
                        out.extend(has("jump", &|k| k.jump.is_some()));
                        out.extend(has("fall", &|k| k.fall.is_some()));
                    }
                    Some("alive") => out.extend(has("contact", &|k| k.contact.is_some())),
                    Some("state") | Some("time") => out.extend(
                        k.machine
                            .is_some()
                            .then(|| format!("entities.{kind}.machine")),
                    ),
                    _ => {}
                }
                out.push(format!("entities.{kind}"));
                out
            }
            (Some("cameras"), Some(c), _) => vec![format!("cameras.{c}")],
            (Some("rules"), Some(r), _) => vec![format!("rules.{r}")],
            (Some("counters" | "ticks"), Some(c), _) => vec![format!("counters.{c}")],
            _ => Vec::new(),
        }
    }
}
