//! High scores kept from one play to the next (docs/shell.md, "High
//! scores"): the counters a model marks `kept` (feature `counter.kept`),
//! which outlast a power-off as a high score table does. The shell keeps
//! their values through [`Storage`] whenever they change and puts them back
//! at the game's start, before the first frame. The simulation never reads
//! this: a clip, a score and a comparison start from the state they are
//! given, and only a player's own play (the preview played from the
//! keyboard, free play from power-on) loads them.
//!
//! The file is [`KEPT_DIR`]`scores-<version>.txt`, a `name = value` line a
//! counter. The version is of the kept counters as the model declares them
//! (their names, starts and bounds, [`Kept::version`]), so a model whose
//! table changed reads none of an older table's values, and every other
//! change to the model keeps them.

use gp_sim::{Spec, World};

use super::Storage;

/// Where kept values are, as a [`Storage`] name.
pub const KEPT_DIR: &str = "kept/";

/// The counters of a spec a model marks kept, by index.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Kept {
    counters: Vec<usize>,
}

impl Kept {
    /// The kept counters of `spec`, from the names the model marks. A name
    /// the spec has no counter of is refused.
    pub fn of(spec: &Spec, names: &[String]) -> Result<Kept, String> {
        let counters = names
            .iter()
            .map(|n| {
                spec.counters
                    .index_of(n)
                    .ok_or_else(|| format!("no counter named {n} to keep"))
            })
            .collect::<Result<_, _>>()?;
        Ok(Kept { counters })
    }

    /// Whether the model keeps nothing.
    pub fn is_empty(&self) -> bool {
        self.counters.is_empty()
    }

    /// The counters kept, by index into the spec's counters.
    pub fn counters(&self) -> &[usize] {
        &self.counters
    }

    /// The version of the kept counters as the model declares them: FNV-1a
    /// (64 bits, in hex) of a `name start min max` line each, in order.
    pub fn version(&self, spec: &Spec) -> String {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for &i in &self.counters {
            let c = &spec.counters[i];
            let line = format!("{} {} {:?} {:?}\n", c.name, c.start, c.min, c.max);
            for b in line.bytes() {
                h ^= u64::from(b);
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        format!("{h:016x}")
    }

    /// The storage name the values are kept under.
    pub fn file(&self, spec: &Spec) -> String {
        format!("{KEPT_DIR}scores-{}.txt", self.version(spec))
    }

    /// The kept counters' values in `world`, as the file holds them; empty
    /// when the model keeps nothing.
    pub fn text(&self, spec: &Spec, world: &World) -> String {
        self.counters
            .iter()
            .map(|&i| format!("{} = {}\n", spec.counters[i].name, world.counters[i]))
            .collect()
    }

    /// Puts kept values into `world`: each line naming a kept counter sets
    /// it, held within its `min` and `max`. A line not understood, or of a
    /// counter not kept, is left out. Returns how many were set.
    pub fn apply(&self, spec: &Spec, world: &mut World, text: &str) -> usize {
        let mut set = 0;
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let Ok(v) = v.trim().parse::<i64>() else {
                continue;
            };
            let Some(&i) = self
                .counters
                .iter()
                .find(|&&i| spec.counters[i].name == k.trim())
            else {
                continue;
            };
            let c = &spec.counters[i];
            let v = c.max.map_or(v, |m| v.min(m));
            world.counters[i] = c.min.map_or(v, |m| v.max(m));
            set += 1;
        }
        set
    }

    /// Puts every kept counter back at its `start`: the high scores cleared.
    pub fn clear(&self, spec: &Spec, world: &mut World) {
        for &i in &self.counters {
            world.counters[i] = spec.counters[i].start;
        }
    }

    /// The values kept in `store`, or None.
    pub fn read(&self, spec: &Spec, store: &mut dyn Storage) -> Option<String> {
        store
            .read(&self.file(spec))
            .and_then(|b| String::from_utf8(b).ok())
    }

    /// The game's start, before its first frame, as a player's shell does
    /// it: with Keep high scores on, the values kept in `store` are put
    /// into `world`. Returns the values the game starts with, which
    /// [`Kept::keep`] compares with after each frame.
    pub fn start(
        &self,
        spec: &Spec,
        world: &mut World,
        settings: super::Settings,
        store: &mut dyn Storage,
    ) -> String {
        if !self.is_empty()
            && settings.keep_scores
            && let Some(t) = self.read(spec, store)
        {
            self.apply(spec, world, &t);
        }
        self.text(spec, world)
    }

    /// After a frame: with Keep high scores on, `world`'s values are
    /// written to `store` when they differ from `last`, which becomes them.
    /// Off, nothing is written. True when they were written.
    pub fn keep(
        &self,
        spec: &Spec,
        world: &World,
        settings: super::Settings,
        last: &mut String,
        store: &mut dyn Storage,
    ) -> Result<bool, String> {
        if self.is_empty() || !settings.keep_scores {
            return Ok(false);
        }
        let now = self.text(spec, world);
        if now == *last {
            return Ok(false);
        }
        store.write(&self.file(spec), now.as_bytes())?;
        *last = now;
        Ok(true)
    }
}
