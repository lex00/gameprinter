//! `gp capture dense` (docs/research/dense-clips.md): a few short clips
//! that each run much more of the original's code. The owner, 2026-10-30:
//! "i want data sets that are more dense and offer a better outcome
//! without nearly so many clips to study".
//!
//! A clip's value is the original's code it reaches that the clips chosen
//! before it don't: the CPU addresses it runs, the way each branch goes,
//! and the values the mapped measures and the enemy slots take
//! ([`gp_nes::coverage`]). It is read from the original alone, so it needs
//! no model, and it is the same on every machine. Candidates come from a
//! coverage-guided input search, as a fuzzer does (AFL, libFuzzer): from
//! each start of the game's clips, press sequences are mutated on
//! gp-explore's loop ([`gp_explore::drive`]), and one is kept when it
//! reaches a feature no kept one has. Then greedy weighted set cover
//! (Chvatal 1979) picks the clips, a clip's price its frames. The search
//! runs from a seed on one thread's random numbers and scores candidates
//! in batches whose results are read in order, so the same seed, ROM and
//! game give the same clips for any `--jobs`.
//!
//! Brought from game-printer's serve/dense.rs. The search, its seeds, the
//! measure features, the cover and the report are the old ones. What
//! changed: it reads a game's package ([`super::Inputs`]) where the old one
//! read a capture run, so its starts are the starts of the game's clips
//! and its pool is the game's clips; and it writes a clip set into a
//! folder (`--into`) where the old one wrote a scratch capture run. The
//! recording windows (`tas`), `--pick`, `--write-set` and `--repick` read
//! a capture run's recordings and its prune, so they wait for wave 7 and
//! are refused in words.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

use anyhow::Context as _;
use clap::Args;
use gp_evidence::input_log::button;
use gp_explore::presses::{Rng, inputs_of, spans};
use gp_interp::mapping::Mapping;
use gp_nes::coverage::{CodeMap, EXTRA, SIZE, TAKEN};
use gp_nes::protocol::NesRuntime;
use gp_runtime::{InputFrame, Runtime, Value};
use serde::Serialize;
use serde_json::{Value as Json, json};

use super::{Inputs, StartPoint};

#[derive(Args, Debug, Clone)]
pub struct DenseArgs {
    /// The game's package folder (games/donkey-kong): its scored ROM,
    /// mapping.toml and clips. In game-printer this was a capture run.
    pub run: PathBuf,
    /// The folder to write the clip set into, refused when it exists and
    /// this command didn't make it. Default: $GP_HOME/work/dense/GAME-dense-SEED.
    #[arg(long)]
    pub into: Option<PathBuf>,
    /// The ROM file, instead of finding it in $GP_ROM_DIR by hash.
    #[arg(long)]
    pub rom: Option<PathBuf>,
    /// Press sequences the search runs, at most.
    #[arg(long, default_value_t = 4000)]
    pub budget: usize,
    #[arg(long, default_value_t = 1)]
    pub seed: u64,
    /// Clips to choose.
    #[arg(long, default_value_t = 15)]
    pub keep: usize,
    /// Clips to write from each method's order, for scoring.
    #[arg(long, default_value_t = 30)]
    pub write: usize,
    #[arg(long, default_value_t = 60)]
    pub min_frames: u64,
    #[arg(long, default_value_t = 180)]
    pub max_frames: u64,
    /// Leave out the search.
    #[arg(long)]
    pub no_search: bool,
    /// Leave out the recordings' windows (always, until capture runs come
    /// back: a game's package holds no recordings).
    #[arg(long)]
    pub no_tas: bool,
    /// Pick the set from a `gp capture prune --json --mutants` instead of
    /// searching. Needs a capture run's prune: wave 7.
    #[arg(long)]
    pub pick: Option<PathBuf>,
    /// With `--pick`: write the picked dense clips into a corpus's clip
    /// set. Wave 7.
    #[arg(long, value_name = "DIR", requires = "pick")]
    pub write_set: Option<PathBuf>,
    /// Pick again in a capture run itself. Wave 7.
    #[arg(long, conflicts_with_all = ["pick", "into", "no_search", "no_tas"])]
    pub repick: bool,
}

/// Candidates scored in one batch: the search reads their results in
/// order, so the batch size is part of what a seed gives.
const BATCH: usize = 64;
/// Frames a clip runs past the last frame that found something, so what
/// it found settles.
pub(crate) const SETTLE: u64 = 12;
/// The report this command writes, which marks a folder it made.
const MARK: &str = "dense.json";

/// A value's bucket for a measure feature: small values each their own,
/// larger ones by their power of two, with the sign, as AFL buckets hit
/// counts.
fn bucket(v: &Value) -> i64 {
    let n = match v {
        Value::Fixed(r) => r >> 16,
        Value::Int(n) => *n,
        Value::Flag(b) => i64::from(*b),
        Value::Entity(e) => *e as i64,
        _ => return 0,
    };
    let a = n.unsigned_abs();
    let b = if a <= 8 {
        a as i64
    } else {
        8 + i64::from(64 - a.leading_zeros())
    };
    if n < 0 { -b } else { b }
}

/// Reads the measure features of the original's frame into `map`: each
/// mapped field's bucket, and each slot object's kind and state byte.
pub(crate) struct Measures<'a> {
    mapping: &'a Mapping,
    /// Each slot table.
    tables: Vec<&'a gp_interp::mapping::Slots>,
}

impl<'a> Measures<'a> {
    pub(crate) fn new(mapping: &'a Mapping) -> Self {
        let mut tables = Vec::new();
        if let Some(s) = &mapping.slots {
            tables.push(s);
            tables.extend(&s.groups);
        }
        Self { mapping, tables }
    }

    pub(crate) fn read(&self, o: &NesRuntime, map: &mut CodeMap) {
        if let Ok(vals) = gp_interp::translate::read_original(o, self.mapping) {
            for (i, v) in vals.iter().enumerate() {
                map.extra((i as u64) << 32 | (bucket(v) as u64 & 0xFFFF_FFFF));
            }
        }
        let nes = o.nes();
        for (t, sl) in self.tables.iter().enumerate() {
            for i in 0..sl.count {
                if sl.flag_of(i, |a| Ok(nes.peek(a))).unwrap_or(0) == 0 {
                    continue;
                }
                let k = u64::from(sl.id.map_or(0, |at| nes.peek(at + i)));
                let s = sl.state.map_or(0, |a| u64::from(nes.peek(a + i)));
                map.extra(1 << 62 | (t as u64) << 24 | k << 8 | s);
            }
        }
    }
}

/// What one run of presses from a start reached.
pub(crate) struct Reach {
    pub map: CodeMap,
    /// The last frame, from 1, on which something not in `base` was seen.
    pub last_new: u64,
    pub new: usize,
}

/// Runs `inputs` from `o` (at the start), adding each frame's code and
/// measures to a map, and noting the last frame that found a feature
/// `base` lacks.
pub(crate) fn reach(
    o: &mut NesRuntime,
    inputs: &[InputFrame],
    m: &Measures,
    base: &CodeMap,
) -> Reach {
    let mut map = CodeMap::new();
    let (mut last_new, mut new) = (0, 0);
    for (k, i) in inputs.iter().enumerate() {
        if o.step_covered(i, &mut map).is_err() {
            break;
        }
        m.read(o, &mut map);
        let n = map.new_over(base);
        if n > new {
            new = n;
            last_new = k as u64 + 1;
        }
    }
    Reach { map, last_new, new }
}

/// A candidate clip.
#[derive(Debug, Clone)]
struct Cand {
    start: usize,
    pads: Vec<u8>,
    map: CodeMap,
}

pub(crate) fn restore(rom: &[u8], snap: &[u8]) -> anyhow::Result<NesRuntime> {
    let mut o = NesRuntime::load(rom, gp_evidence::manifest::PowerOn::default())
        .map_err(|e| anyhow::anyhow!("the ROM: {e}"))?;
    o.restore(snap).map_err(anyhow::Error::msg)?;
    Ok(o)
}

/// The search's plan on gp-explore's loop: candidates kept for code and
/// measure features none kept before had, parents drawn a little more
/// often from later finds (AFL's queue favours new entries too), mates
/// from the same start.
struct DensePlan<'a> {
    global: CodeMap,
    kept: Vec<Cand>,
    min: usize,
    progress: &'a mut dyn FnMut(usize, usize, usize),
}

impl gp_explore::Plan for DensePlan<'_> {
    type Out = (CodeMap, u64, usize);

    fn has_parents(&self) -> bool {
        !self.kept.is_empty()
    }

    fn parents(&mut self, rng: &mut Rng) -> (usize, Vec<u8>, Option<Vec<u8>>) {
        let n = self.kept.len();
        let k = if rng.below(2) == 0 {
            rng.below(n)
        } else {
            n - 1 - rng.below(n.min(16))
        };
        let parent = &self.kept[k];
        let mates: Vec<&Cand> = self
            .kept
            .iter()
            .filter(|c| c.start == parent.start)
            .collect();
        let mate = mates[rng.below(mates.len())];
        (parent.start, parent.pads.clone(), Some(mate.pads.clone()))
    }

    fn judge(&mut self, s: usize, p: Vec<u8>, (map, last_new, _): Self::Out) -> bool {
        // New over the kept ones so far, in this batch's order.
        if map.new_over(&self.global) == 0 {
            return false;
        }
        self.global.union(&map);
        let len = ((last_new + SETTLE) as usize).clamp(self.min, p.len().max(self.min));
        let mut pads = p;
        pads.truncate(len);
        self.kept.push(Cand {
            start: s,
            pads,
            map,
        });
        false
    }

    fn after_batch(&mut self, ran: usize, budget: usize) {
        (self.progress)(ran, budget, self.kept.len());
    }
}

/// The coverage-guided search: from every start, the seeds, then batches
/// of mutants of kept sequences, a mutant kept when it reaches a feature
/// none kept before has. Returns the kept candidates, each cut to its
/// last new frame and a settle, in the order found.
fn search(
    rom: &[u8],
    starts: &[StartPoint],
    m: &Measures,
    o: &DenseArgs,
    progress: &mut dyn FnMut(usize, usize, usize),
) -> Vec<Cand> {
    let (min, max) = (o.min_frames as usize, o.max_frames as usize);
    let mut rng = Rng::new(o.seed);
    // The seeds: nothing, a walk, a run, a run with jumps, a walk back.
    let mut todo: Vec<(usize, Vec<u8>)> = Vec::new();
    for s in 0..starts.len() {
        let run_jumps: Vec<u8> = (0..max)
            .map(|k| button::RIGHT | button::B | if k % 40 < 20 { button::A } else { 0 })
            .collect();
        for p in [
            vec![0; min],
            vec![button::RIGHT; max],
            vec![button::RIGHT | button::B; max],
            run_jumps,
            vec![button::LEFT; min],
        ] {
            todo.push((s, p));
        }
    }
    let mut plan = DensePlan {
        global: CodeMap::new(),
        kept: Vec::new(),
        min,
        progress,
    };
    let d = gp_explore::Drive {
        budget: o.budget,
        batch: BATCH,
        min_frames: min,
        max_frames: max,
    };
    gp_explore::drive(&mut plan, &mut rng, todo, &d, &mut |plan, batch| {
        let base = plan.global.clone();
        crate::pool::map(
            batch,
            || (),
            |_, _, (s, p)| -> Option<(CodeMap, u64, usize)> {
                let mut or = restore(rom, &starts[*s].snapshot).ok()?;
                let r = reach(&mut or, &inputs_of(p), m, &base);
                Some((r.map, r.last_new, r.new))
            },
            &mut |_| {},
        )
    });
    let mut kept = plan.kept;
    // The maps again on the cut sequences.
    let maps = crate::pool::map(
        &kept,
        || (),
        |_, _, c| {
            restore(rom, &starts[c.start].snapshot)
                .map(|mut or| reach(&mut or, &inputs_of(&c.pads), m, &CodeMap::new()).map)
                .unwrap_or_default()
        },
        &mut |_| {},
    );
    for (c, map) in kept.iter_mut().zip(maps) {
        c.map = map;
    }
    kept
}

/// Greedy weighted set cover over the candidates' features, a clip's
/// price its frames: the clips in the order taken, each with the features
/// it added, until `n` are taken or none adds anything.
fn greedy(maps: &[(&CodeMap, u64)], n: usize) -> Vec<(usize, usize)> {
    let mut held = CodeMap::new();
    let mut out = Vec::new();
    let mut taken = BTreeSet::new();
    while out.len() < n {
        let mut best: Option<(f64, usize, usize)> = None;
        for (i, (m, frames)) in maps.iter().enumerate() {
            if taken.contains(&i) {
                continue;
            }
            let gain = m.new_over(&held);
            if gain == 0 {
                continue;
            }
            let r = gain as f64 / (*frames).max(1) as f64;
            // Ties to the first, so the choice is the same every time.
            if best.is_none_or(|(b, _, _)| r > b) {
                best = Some((r, i, gain));
            }
        }
        let Some((_, i, gain)) = best else { break };
        taken.insert(i);
        held.union(maps[i].0);
        out.push((i, gain));
    }
    out
}

/// What a set of maps covers, in the report's terms.
pub(crate) fn covered(maps: &[&CodeMap]) -> Json {
    let mut u = CodeMap::new();
    for m in maps {
        u.union(m);
    }
    json!({
        "features": u.count(),
        "rom_code": u.count_in(0x8000..0x1_0000),
        "branches": u.count_in(TAKEN..EXTRA),
        "measures": u.count_in(EXTRA..SIZE),
    })
}

pub(crate) fn hash_of(bytes: &[u8]) -> String {
    gp_evidence::Hash::of(bytes).to_string()
}

/// A code map's hash: the features in order, each as 4 bytes. It is what
/// a dense clip's `coverage` holds.
pub(crate) fn coverage_hash(map: &CodeMap) -> String {
    hash_of(
        &map.ids()
            .iter()
            .flat_map(|i| i.to_le_bytes())
            .collect::<Vec<u8>>(),
    )
}

/// One written clip, as the report lists it.
#[derive(Serialize)]
struct Recipe {
    id: String,
    name: String,
    method: &'static str,
    start: Json,
    presses: Vec<String>,
    frames: u64,
    coverage: String,
    rom_code: usize,
    features: usize,
}

pub(crate) fn cli(paths: &gp_evidence::Paths, o: &DenseArgs) -> anyhow::Result<crate::Outcome> {
    if o.pick.is_some() || o.repick {
        anyhow::bail!(
            "--pick and --repick read a capture run's prune and its mutants, which gameprinter doesn't have until the capture runs come back (wave 7 of the port)"
        );
    }
    anyhow::ensure!(
        (1..=o.max_frames).contains(&o.min_frames),
        "--min-frames must be at least 1 and at most --max-frames"
    );
    let inp = Inputs::of_game(paths, &o.run, o.rom.as_deref())?;
    let out_dir = o.into.clone().unwrap_or_else(|| {
        paths
            .home
            .join("work")
            .join("dense")
            .join(format!("{}-dense-{}", inp.game, o.seed))
    });
    if out_dir.exists() {
        anyhow::ensure!(
            out_dir.join(MARK).is_file(),
            "{} exists and wasn't made by gp capture dense; name another --into",
            out_dir.display()
        );
    }
    if !o.no_tas {
        eprintln!(
            "tas: a game's package holds no recordings, so no recording windows are read (capture runs, wave 7)"
        );
    }
    let m = Measures::new(&inp.mapping);
    let rom = inp.rom.clone();
    let starts = &inp.starts;
    let t0 = std::time::Instant::now();

    // The game's own clips, for the comparison.
    let pool_maps = crate::pool::map(
        &inp.pool,
        || (),
        |_, _, (_, s, pads)| -> Option<CodeMap> {
            let mut or = restore(&rom, &starts[*s].snapshot).ok()?;
            Some(reach(&mut or, &inputs_of(pads), &m, &CodeMap::new()).map)
        },
        &mut |_| {},
    );
    eprintln!(
        "the game's {} clips read on the original in {:.1} s",
        inp.pool.len(),
        t0.elapsed().as_secs_f64()
    );

    let mut cands: Vec<Cand> = Vec::new();
    if !o.no_search {
        let found = search(&rom, starts, &m, o, &mut |ran, budget, kept| {
            if ran % (BATCH * 8) < BATCH {
                eprintln!("search: {ran} of {budget} run, {kept} kept");
            }
        });
        eprintln!(
            "search: {} kept, {:.1} s in",
            found.len(),
            t0.elapsed().as_secs_f64()
        );
        cands.extend(found);
    }

    let search_maps: Vec<(&CodeMap, u64)> = cands
        .iter()
        .map(|c| (&c.map, c.pads.len() as u64))
        .collect();
    let pool_pairs: Vec<(&CodeMap, u64)> = pool_maps
        .iter()
        .zip(&inp.pool)
        .filter_map(|(m, (_, _, pads))| Some((m.as_ref()?, pads.len() as u64)))
        .collect();
    let n = o.write.max(o.keep);
    let g_search = greedy(&search_maps, n);

    // The clips to write: the search's first `write`.
    let mut picks: Vec<(String, usize)> = Vec::new();
    let mut seen_ids = BTreeSet::new();
    for &(i, _) in g_search.iter().take(o.write) {
        let start_key = &starts[cands[i].start].id;
        let mut h = start_key.clone().into_bytes();
        h.extend_from_slice(&cands[i].pads);
        let id = format!(
            "dense.search.{}.{}",
            crate::clipset::file_key(start_key),
            &hash_of(&h)[..8]
        );
        if seen_ids.insert(id.clone()) {
            picks.push((id, i));
        }
    }
    let mut sets: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    sets.insert("search", picks.iter().map(|p| p.0.clone()).collect());

    // The clip set: clips.toml, the starts' presses files beside it.
    fs::create_dir_all(&out_dir)?;
    let mut set = crate::clipset::ClipSet {
        clipset: crate::clipset::Header {
            format: crate::clipset::FORMAT.into(),
            game: inp.game.clone(),
            rom: crate::roms::sha256_of(&rom)?,
            generator: format!("gp capture dense, seed {}", o.seed),
            ..Default::default()
        },
        ..Default::default()
    };
    let mut recipes: Vec<Recipe> = Vec::new();
    let mut copied = BTreeSet::new();
    for (id, i) in &picks {
        let c = &cands[*i];
        let s = &starts[c.start];
        if copied.insert(s.presses_file.clone()) {
            let to = out_dir.join(&s.presses_file);
            if let Some(p) = to.parent() {
                fs::create_dir_all(p)?;
            }
            fs::copy(inp.dir.join(&s.presses_file), &to)
                .with_context(|| format!("copying {}", s.presses_file))?;
        }
        let presses = spans(&c.pads);
        let frames = c.pads.len() as u64;
        let name = format!("searched presses, {frames} frames, from frame {}", s.frame);
        set.clips.push(crate::clipset::Recipe {
            id: id.clone(),
            name: name.clone(),
            source: "dense".into(),
            presses: presses.clone(),
            frames,
            coverage: coverage_hash(&c.map),
            why: format!(
                "runs {} ROM addresses and {} features of the original's code",
                c.map.count_in(0x8000..0x1_0000),
                c.map.count()
            ),
            start: crate::clipset::StartRecipe {
                presses_from_power_on: Some(s.presses_file.clone()),
                frame: s.frame,
                ..Default::default()
            },
            ..Default::default()
        });
        recipes.push(Recipe {
            id: id.clone(),
            name,
            method: "search",
            start: json!({"presses_from_power_on": s.presses_file, "frame": s.frame}),
            presses,
            frames,
            coverage: coverage_hash(&c.map),
            rom_code: c.map.count_in(0x8000..0x1_0000),
            features: c.map.count(),
        });
    }
    fs::write(
        out_dir.join(crate::clipset::FILE),
        format!(
            "# Dense clips of {} chosen by gp capture dense (seed {}): searched\n# presses from the game's clip starts.\n{}",
            inp.game,
            o.seed,
            toml::to_string(&set)?
        ),
    )?;

    // The report.
    let maps_of = |order: &[(usize, usize)], all: &[(&CodeMap, u64)], k: usize| -> Json {
        let ms: Vec<&CodeMap> = order.iter().take(k).map(|&(i, _)| all[i].0).collect();
        let frames: u64 = order.iter().take(k).map(|&(i, _)| all[i].1).sum();
        let mut c = covered(&ms);
        c["clips"] = json!(ms.len());
        c["frames"] = json!(frames);
        c
    };
    let pool_all: Vec<&CodeMap> = pool_pairs.iter().map(|p| p.0).collect();
    let mut pool_cov = covered(&pool_all);
    pool_cov["clips"] = json!(pool_all.len());
    pool_cov["frames"] = json!(pool_pairs.iter().map(|p| p.1).sum::<u64>());
    let curve = |order: &[(usize, usize)], all: &[(&CodeMap, u64)]| -> Vec<Json> {
        [1usize, 3, 5, 8, 10, 12, 15, 20, 30, 40, 60]
            .iter()
            .filter(|&&k| k <= order.len())
            .map(|&k| maps_of(order, all, k))
            .collect()
    };
    let secs = t0.elapsed().as_secs_f64();
    let report = json!({
        "run": o.run, "into": out_dir, "seed": o.seed, "budget": o.budget,
        "min_frames": o.min_frames, "max_frames": o.max_frames,
        "rom": hash_of(&rom), "seconds": secs,
        "starts": starts.len(), "search_kept": cands.len(), "windows": 0,
        "pool": pool_cov,
        "kept": {
            "search": maps_of(&g_search, &search_maps, o.keep),
        },
        "curve": {
            "search": curve(&g_search, &search_maps),
        },
        "sets": sets,
        "clips": recipes,
    });
    fs::write(out_dir.join(MARK), serde_json::to_vec_pretty(&report)?)?;

    let line = |label: &str, v: &Json| -> String {
        format!(
            "{label}: {} clips, {} frames, {} ROM addresses run, {} branch ways, {} measure values",
            v["clips"], v["frames"], v["rom_code"], v["branches"], v["measures"]
        )
    };
    let text = [
        format!(
            "{} starts, {} search candidates run, {} kept. {:.1} s.",
            starts.len(),
            o.budget,
            cands.len(),
            secs
        ),
        line("the game's clips", &report["pool"]),
        line("search, greedy", &report["kept"]["search"]),
        format!(
            "Wrote {} clips into {} (clips.toml, dense.json); the game read from is unchanged.",
            picks.len(),
            out_dir.display()
        ),
    ]
    .join("\n");
    Ok(crate::Outcome::ok(report, text))
}
