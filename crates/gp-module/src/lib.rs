//! A game's module as a [`Runtime`]: wasmtime with NaN canonicalization on,
//! relaxed SIMD off and no imports, a deadline on every call and a cap on
//! memory, so a bad module stalls nothing but itself.
//!
//! One Wasm instance holds a game's sessions. [`ModuleRuntime::fork`] makes
//! another inside it, so forks of one runtime share that instance and take
//! turns running.
//!
//! A compiled module is kept on disk ([`cache`]), so a gp call opens a
//! module it has compiled before without compiling it again.

mod cache;

use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use gp_runtime::wire::{self, Reader, Writer};
use gp_runtime::{
    Capabilities, Entry, Event, InputFrame, LoadNote, PatchResult, QueryValues, Rect, Runtime,
    Until, Value,
};
use wasmtime::{
    Config, Engine, Instance, Memory, Module, Store, StoreLimits, StoreLimitsBuilder, TypedFunc,
};

/// The largest memory a module may reach: 256 MB.
pub const MAX_MEMORY: usize = 256 * 1024 * 1024;
const TICK_MS: u64 = 10;
/// Ten frame-times at 60 fps, in ticks.
const FRAME_TICKS: u64 = 17;
/// A call that isn't frames, such as loading a model: 30 s.
const CALL_TICKS: u64 = 3000;

fn engine() -> &'static Engine {
    static ENGINE: OnceLock<Engine> = OnceLock::new();
    ENGINE.get_or_init(|| {
        let mut c = Config::new();
        c.cranelift_nan_canonicalization(true)
            .wasm_relaxed_simd(false)
            .epoch_interruption(true);
        let engine = Engine::new(&c).expect("a wasmtime engine");
        let ticking = engine.clone();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_millis(TICK_MS));
                ticking.increment_epoch();
            }
        });
        engine
    })
}

struct Host {
    store: Store<StoreLimits>,
    memory: Memory,
    alloc: TypedFunc<u32, u32>,
    out_ptr: TypedFunc<(), u32>,
    out_len: TypedFunc<(), u32>,
    rt: TypedFunc<(u32, u32, u32), i32>,
    /// The words of the module's last panic (`gp_panic`), in a module
    /// built from 2026-11-23 on.
    panic: Option<TypedFunc<(), i32>>,
}

/// A game's module, compiled once: each [`ModuleCode::load`] is a fresh
/// instance of it, so a tool that scores many clips compiles it one time.
#[derive(Clone)]
pub struct ModuleCode {
    module: Module,
}

impl ModuleCode {
    /// The module compiled, read from the machine's cache of compiled
    /// modules when it is there and sound, else compiled and kept there.
    pub fn compile(wasm: &[u8]) -> Result<Self, String> {
        Self::compile_in(wasm, cache::dir().as_deref())
    }

    /// [`Self::compile`] with the cache in `dir`, or none.
    pub fn compile_in(wasm: &[u8], dir: Option<&Path>) -> Result<Self, String> {
        let key = dir.map(|d| (d, cache::key(engine(), wasm)));
        let kept = key
            .as_ref()
            .and_then(|(d, k)| cache::read(engine(), &cache::file(d, k), k));
        let module = match kept {
            Some(m) => m,
            None => {
                let m = Module::new(engine(), wasm)
                    .map_err(|e| format!("the module doesn't compile: {e}"))?;
                if let Some((d, k)) = &key {
                    cache::write(d, k, &m);
                }
                m
            }
        };
        if module.imports().len() != 0 {
            return Err("the module imports something, so gp won't run it".into());
        }
        Ok(Self { module })
    }

    /// A fresh instance of the module with a model bundle loaded.
    pub fn load(&self, bundle: &[u8]) -> Result<ModuleRuntime, String> {
        ModuleRuntime::start(Host::start(&self.module)?, bundle)
    }
}

impl Host {
    /// The module's first instance, compiling it first.
    #[cfg(test)]
    fn open(wasm: &[u8]) -> Result<Self, String> {
        Self::start(&ModuleCode::compile_in(wasm, None)?.module)
    }

    fn start(module: &Module) -> Result<Self, String> {
        let limits = StoreLimitsBuilder::new()
            .memory_size(MAX_MEMORY)
            .trap_on_grow_failure(true)
            .build();
        let mut store = Store::new(engine(), limits);
        store.limiter(|l| l);
        store.set_epoch_deadline(CALL_TICKS);
        let instance = Instance::new(&mut store, module, &[])
            .map_err(|e| format!("the module wouldn't start: {e}"))?;
        let missing = |what: &str| format!("the module has no {what}");
        let memory = instance
            .get_memory(&mut store, "memory")
            .ok_or_else(|| missing("memory"))?;
        macro_rules! func {
            ($name:literal) => {
                instance
                    .get_typed_func(&mut store, $name)
                    .map_err(|_| missing($name))?
            };
        }
        Ok(Self {
            alloc: func!("gp_alloc"),
            out_ptr: func!("gp_out_ptr"),
            out_len: func!("gp_out_len"),
            rt: func!("gp_rt"),
            panic: instance.get_typed_func(&mut store, "gp_panic").ok(),
            memory,
            store,
        })
    }

    /// `ticks` is the call's deadline. An error says what limit stopped it.
    fn call(&mut self, op: u32, input: &[u8], ticks: u64) -> Result<Vec<u8>, String> {
        self.store.set_epoch_deadline(ticks);
        let len = u32::try_from(input.len()).map_err(|_| "a call of more than 4 GB")?;
        let p = match self.alloc.call(&mut self.store, len) {
            Ok(p) => p,
            Err(e) => return Err(self.stopped(&e, ticks)),
        };
        self.memory
            .write(&mut self.store, p as usize, input)
            .map_err(|e| e.to_string())?;
        let code = match self.rt.call(&mut self.store, (op, p, len)) {
            Ok(c) => c,
            Err(e) => return Err(self.stopped(&e, ticks)),
        };
        let out = match self.out() {
            Ok(o) => o,
            Err(e) => return Err(self.stopped(&e, ticks)),
        };
        if code != 0 {
            return Err(String::from_utf8_lossy(&out).into_owned());
        }
        Ok(out)
    }

    /// The output buffer's bytes.
    fn out(&mut self) -> wasmtime::Result<Vec<u8>> {
        let ptr = self.out_ptr.call(&mut self.store, ())? as usize;
        let n = self.out_len.call(&mut self.store, ())? as usize;
        let mut out = vec![0; n];
        self.memory.read(&self.store, ptr, &mut out)?;
        Ok(out)
    }

    /// What stopped a call, its cause first: the deadline, the module's
    /// panic in its own words (`gp_panic`), else the trap. The wasm
    /// backtrace comes last: the page shows a message's first 160
    /// characters, and until 2026-11-23 those were the backtrace's, of a
    /// stripped module's unnamed functions, so the cause never showed.
    fn stopped(&mut self, e: &wasmtime::Error, ticks: u64) -> String {
        let trap = e.downcast_ref::<wasmtime::Trap>();
        if matches!(trap, Some(wasmtime::Trap::Interrupt)) {
            return format!(
                "the game's module stopped: it ran past its deadline of {} ms",
                ticks * TICK_MS
            );
        }
        let said = self.panic_words();
        let cause = match (&said, trap) {
            (Some(p), _) => p.clone(),
            (None, Some(t)) => t.to_string(),
            (None, None) => e.root_cause().to_string(),
        };
        match e.downcast_ref::<wasmtime::WasmBacktrace>() {
            Some(bt) => format!("the game's module stopped: {cause}\n{bt}"),
            None => format!("the game's module stopped: {cause}"),
        }
    }

    /// The module's last panic's words, when it has `gp_panic` and panicked.
    fn panic_words(&mut self) -> Option<String> {
        self.store.set_epoch_deadline(CALL_TICKS);
        let f = self.panic.clone()?;
        let n = f.call(&mut self.store, ()).ok()?;
        if n <= 0 {
            return None;
        }
        let out = self.out().ok()?;
        Some(String::from_utf8_lossy(&out).trim().to_owned()).filter(|s| !s.is_empty())
    }
}

/// A draws script as [`ModuleRuntime::set_draw_script`] sends it.
pub struct DrawScriptCall<'a> {
    /// The script's text form (gp_sim::DrawScript::text).
    pub text: &'a str,
    pub base: u64,
    pub origin: Option<u64>,
    pub lenient: bool,
    pub keep_calls: bool,
}

/// What the module reported of a run's random draws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrawsReport {
    pub taken: u32,
    pub script: u32,
    pub desync: Option<(u64, String)>,
    pub desyncs: Vec<(u64, String, String)>,
}

/// One session of a game's module.
pub struct ModuleRuntime {
    host: Arc<Mutex<Host>>,
    id: u32,
    /// Whether the module has `query-values` (game-printer:game@0.2.0 on):
    /// [`QueryValues::query_values`] is then one call, else a `query` for
    /// each path.
    batch: bool,
    /// The hooks' label, as a recording stores it.
    pub label: String,
    /// The hooks the module has.
    pub hooks: Vec<String>,
}

impl ModuleRuntime {
    /// Opens a module and loads a model bundle into it.
    pub fn load(wasm: &[u8], bundle: &[u8]) -> Result<Self, String> {
        ModuleCode::compile(wasm)?.load(bundle)
    }

    fn start(mut host: Host, bundle: &[u8]) -> Result<Self, String> {
        // A module built before game-printer:game@0.2.0 says only its
        // runtime protocol.
        let batch = host.call(wire::INFO, &[], CALL_TICKS).is_ok_and(|info| {
            let mut r = Reader::new(&info);
            r.str().is_ok() && r.str().is_ok_and(|game| answers_query_values(&game))
        });
        let out = host.call(wire::LOAD, bundle, CALL_TICKS)?;
        let mut r = Reader::new(&out);
        Ok(Self {
            batch,
            id: r.u32()?,
            label: r.str()?,
            hooks: r.strs()?,
            host: Arc::new(Mutex::new(host)),
        })
    }

    fn call(&self, op: u32, ticks: u64, args: impl FnOnce(&mut Writer)) -> Result<Vec<u8>, String> {
        let mut w = Writer::default();
        w.u32(self.id);
        args(&mut w);
        self.host
            .lock()
            .expect("the module isn't poisoned")
            .call(op, &w.0, ticks)
    }

    /// The deadline of a call that runs `frames` frames: a call's, or ten
    /// frame-times a frame when that is longer. Every frame here is one the
    /// tools run and no player waits on (the page holds a frame to ten
    /// frame-times itself, docs/modules.md). A single frame gets a call's
    /// deadline too: on a loaded machine one frame of a long clip can wait
    /// more than ten frame-times for the CPU, and the gate failed on it.
    fn frames_deadline(frames: usize) -> u64 {
        CALL_TICKS.max(FRAME_TICKS.saturating_mul(frames as u64))
    }

    /// Whether [`QueryValues::query_values`] reads with one call across
    /// (`query-values`), true for a module built from a kit of
    /// game-printer:game@0.2.0 on; false for one built before, read a
    /// `query` for each path.
    pub fn batch_reads(&self) -> bool {
        self.batch
    }

    /// The hash recordings store for this frame.
    pub fn world_hash(&self) -> Result<[u8; 32], String> {
        let out = self.call(wire::WORLD_HASH, CALL_TICKS, |_| {})?;
        out.try_into()
            .map_err(|_| "a world hash isn't 32 bytes".to_owned())
    }

    /// The digest of the spec the module compiled.
    pub fn spec_digest(&self) -> Result<String, String> {
        let out = self.call(wire::SPEC_DIGEST, CALL_TICKS, |_| {})?;
        Reader::new(&out).str()
    }

    /// Gives the run its script of random draws, in `DrawScript`'s text form.
    pub fn set_draws(&mut self, script: &str, base: u64) -> Result<(), String> {
        self.call(wire::SET_DRAWS, CALL_TICKS, |w| {
            w.str(script);
            w.u64(base);
        })
        .map(|_| ())
    }

    /// How the run took its script; None when it has none.
    pub fn report_draws(&self) -> Result<Option<DrawsReport>, String> {
        let out = self.call(wire::REPORT_DRAWS, CALL_TICKS, |_| {})?;
        let mut r = Reader::new(&out);
        if !r.bool()? {
            return Ok(None);
        }
        let taken = r.u32()?;
        let script = r.u32()?;
        let desync = if r.bool()? {
            Some((r.u64()?, r.str()?))
        } else {
            None
        };
        let desyncs = (0..r.count()?)
            .map(|_| Ok((r.u64()?, r.str()?, r.str()?)))
            .collect::<Result<_, String>>()?;
        Ok(Some(DrawsReport {
            taken,
            script,
            desync,
            desyncs,
        }))
    }

    /// Starts noting what the session reads of its spec and hooks.
    pub fn start_reads(&mut self) -> Result<(), String> {
        self.call(wire::START_READS, CALL_TICKS, |_| {}).map(|_| ())
    }

    /// The bytes of what was noted since [`Self::start_reads`], which
    /// `gp_interp::reads::read_reads` reads.
    pub fn take_reads(&mut self) -> Result<Vec<u8>, String> {
        self.call(wire::TAKE_READS, CALL_TICKS, |_| {})
    }

    /// The bytes of every hook's mechanic records, which
    /// `gp_interp::describe::read_exts` reads.
    pub fn describe(&self) -> Result<Vec<u8>, String> {
        self.call(wire::DESCRIBE, CALL_TICKS, |_| {})
    }

    /// Puts the session's level on the flat floor of
    /// `gp_interp::variant::flat_floor`: `flat-level` in `wit/game/game.wit`.
    pub fn flat_level(&mut self, level: u32) -> Result<(), String> {
        self.call(wire::FLAT_LEVEL, CALL_TICKS, |w| w.u32(level))
            .map(|_| ())
    }

    /// Makes `step` return the hooks' sound-driver writes as events.
    pub fn set_apu_events(&mut self) -> Result<(), String> {
        self.call(wire::SET_APU_EVENTS, CALL_TICKS, |_| {})
            .map(|_| ())
    }

    /// Makes `step` return the sound requests the hooks made as events
    /// (`set-sound-events`, game-printer:game@0.3.0). A module built before
    /// has no such call and refuses it: its runs make no requests.
    pub fn set_sound_events(&mut self) -> Result<(), String> {
        self.call(wire::SET_SOUND_EVENTS, CALL_TICKS, |_| {})
            .map(|_| ())
    }

    /// Gives the run a draws script whole: its text and base as
    /// [`Self::set_draws`] does, and its origin, whether it is lenient and
    /// whether it keeps the run's draw calls ([`Self::draw_calls`]).
    pub fn set_draw_script(&mut self, d: &DrawScriptCall<'_>) -> Result<(), String> {
        self.call(wire::SET_DRAW_SCRIPT, CALL_TICKS, |w| {
            w.str(d.text);
            w.u64(d.base);
            w.opt_u64(d.origin);
            w.bool(d.lenient);
            w.bool(d.keep_calls);
        })
        .map(|_| ())
    }

    /// The draw calls the run's script has kept, frame and site each; None
    /// when it keeps none.
    pub fn draw_calls(&self) -> Result<Option<Vec<(u64, String)>>, String> {
        let out = self.call(wire::DRAW_CALLS, CALL_TICKS, |_| {})?;
        let mut r = Reader::new(&out);
        if !r.bool()? {
            return Ok(None);
        }
        (0..r.count()?)
            .map(|_| Ok((r.u64()?, r.str()?)))
            .collect::<Result<_, String>>()
            .map(Some)
    }

    /// Steps every body of the world `state` (a canonical model state) but
    /// the player once with this session's spec and hooks, as the enemies
    /// system does: whether a hook drew a random byte, and the new state.
    pub fn step_hooks(&self, state: &[u8]) -> Result<(bool, Vec<u8>), String> {
        let out = self.call(wire::STEP_HOOKS, CALL_TICKS, |w| w.bytes(state))?;
        let mut r = Reader::new(&out);
        let drew = r.bool()?;
        Ok((drew, out[1..].to_vec()))
    }
}

/// Whether a module whose game package is `game` (`game-printer:game@X.Y.Z`)
/// has `query-values`: 0.2.0 on.
fn answers_query_values(game: &str) -> bool {
    let Some(v) = game.strip_prefix("game-printer:game@") else {
        return false;
    };
    let mut parts = v.split('.').map(str::parse::<u32>);
    match (parts.next(), parts.next()) {
        (Some(Ok(major)), Some(Ok(minor))) => (major, minor) >= (0, 2),
        _ => false,
    }
}

impl QueryValues for ModuleRuntime {
    fn query_values(&self, selectors: &[String]) -> Vec<Result<Vec<Value>, String>> {
        if !self.batch {
            return selectors
                .iter()
                .map(|s| Ok(self.query(s)?.into_iter().map(|e| e.value).collect()))
                .collect();
        }
        match self
            .call(wire::QUERY_VALUES, CALL_TICKS, |w| w.strs(selectors))
            .and_then(|out| Reader::new(&out).value_rows())
        {
            Ok(rows) if rows.len() == selectors.len() => rows,
            Ok(rows) => vec![
                Err(format!(
                    "the module answered {} of {} paths",
                    rows.len(),
                    selectors.len()
                ));
                selectors.len()
            ],
            Err(e) => vec![Err(e); selectors.len()],
        }
    }
}

impl Drop for ModuleRuntime {
    fn drop(&mut self) {
        let _ = self.call(wire::DROP, CALL_TICKS, |_| {});
    }
}

fn must<T>(r: Result<T, String>) -> T {
    r.unwrap_or_else(|e| panic!("{e}"))
}

impl Runtime for ModuleRuntime {
    fn capabilities(&self) -> Capabilities {
        Capabilities::SNAPSHOT
            .union(Capabilities::FORK)
            .union(Capabilities::FULL_MODEL_QUERY)
            .union(Capabilities::EXPLAIN_MODEL)
            .union(Capabilities::LOAD_STATE)
    }

    fn step(&mut self, inputs: &[InputFrame]) -> Result<Vec<Event>, String> {
        let out = self.call(wire::STEP, Self::frames_deadline(inputs.len()), |w| {
            w.inputs(inputs)
        })?;
        Reader::new(&out).events()
    }

    fn run_until(
        &mut self,
        condition: &str,
        inputs: &[InputFrame],
        max_frames: u32,
    ) -> Result<Until, String> {
        let out = self.call(
            wire::RUN_UNTIL,
            Self::frames_deadline(max_frames as usize),
            |w| {
                w.str(condition);
                w.inputs(inputs);
                w.u32(max_frames);
            },
        )?;
        Reader::new(&out).until()
    }

    fn frame(&self) -> u64 {
        let out = must(self.call(wire::FRAME, CALL_TICKS, |_| {}));
        must(Reader::new(&out).u64())
    }

    fn snapshot(&self) -> Vec<u8> {
        self.canonical_state()
    }

    /// Puts the session back to a snapshot: a restore is not a load, so a
    /// script of random draws doesn't start or resume on it.
    fn restore(&mut self, snap: &[u8]) -> Result<(), String> {
        self.call(wire::RESTORE, CALL_TICKS, |w| w.bytes(snap))
            .map(|_| ())
    }

    fn fork(&self) -> Self {
        let out = must(self.call(wire::FORK, CALL_TICKS, |_| {}));
        Self {
            host: Arc::clone(&self.host),
            id: must(Reader::new(&out).u32()),
            batch: self.batch,
            label: self.label.clone(),
            hooks: self.hooks.clone(),
        }
    }

    fn canonical_state(&self) -> Vec<u8> {
        must(self.call(wire::CANONICAL_STATE, CALL_TICKS, |_| {}))
    }

    fn state_hash(&self) -> u64 {
        let out = must(self.call(wire::STATE_HASH, CALL_TICKS, |_| {}));
        must(Reader::new(&out).u64())
    }

    fn load_state(&mut self, state: &[u8]) -> Result<Vec<LoadNote>, String> {
        let out = self.call(wire::LOAD_STATE, CALL_TICKS, |w| w.bytes(state))?;
        Reader::new(&out).notes()
    }

    fn query(&self, selector: &str) -> Result<Vec<Entry>, String> {
        let out = self.call(wire::QUERY, CALL_TICKS, |w| w.str(selector))?;
        Reader::new(&out).entries()
    }

    fn sample(
        &mut self,
        selectors: &[String],
        inputs: &[InputFrame],
        frames: u32,
    ) -> Result<Vec<Vec<Entry>>, String> {
        let out = self.call(wire::SAMPLE, Self::frames_deadline(frames as usize), |w| {
            w.strs(selectors);
            w.inputs(inputs);
            w.u32(frames);
        })?;
        let mut r = Reader::new(&out);
        (0..r.count()?).map(|_| r.entries()).collect()
    }

    fn entities_at(&self, area: Rect) -> Vec<u64> {
        let out = must(self.call(wire::ENTITIES_AT, CALL_TICKS, |w| w.rect(area)));
        let mut r = Reader::new(&out);
        must((0..must(r.count())).map(|_| r.u64()).collect())
    }

    /// Not carried: a changed model is loaded again.
    fn patch(&mut self, _model_diff: &str) -> PatchResult {
        PatchResult::NeedsReload
    }

    fn set(&mut self, p: &str, v: Value) -> Result<(), String> {
        self.call(wire::SET, CALL_TICKS, |w| {
            w.str(p);
            w.value(&v);
        })
        .map(|_| ())
    }

    fn explain(&self, p: &str) -> Vec<String> {
        let out = must(self.call(wire::EXPLAIN, CALL_TICKS, |w| w.str(p)));
        must(Reader::new(&out).strs())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A module with the host's calls, `gp_rt` being `body`.
    fn module(body: &str, memory: &str) -> Vec<u8> {
        wat::parse_str(format!(
            r#"(module
              (memory (export "memory") {memory})
              (func (export "gp_alloc") (param i32) (result i32) i32.const 0)
              (func (export "gp_out_ptr") (result i32) i32.const 0)
              (func (export "gp_out_len") (result i32) i32.const 0)
              (func (export "gp_rt") (param i32 i32 i32) (result i32) {body}))"#
        ))
        .unwrap()
    }

    #[test]
    fn a_module_that_loops_forever_stops_at_its_deadline() {
        let wasm = module("(loop (br 0)) i32.const 0", "1");
        let mut h = Host::open(&wasm).unwrap();
        let t0 = std::time::Instant::now();
        let e = h.call(wire::STEP, &[], FRAME_TICKS).unwrap_err();
        assert!(e.contains("deadline"), "{e}");
        assert!(t0.elapsed() < Duration::from_secs(5));
    }

    /// A step of one frame has a call's deadline, so a tool's frame that
    /// waits on a loaded machine isn't stopped; a long run has ten
    /// frame-times a frame.
    #[test]
    fn one_frame_has_a_calls_deadline_and_a_long_run_more() {
        assert_eq!(ModuleRuntime::frames_deadline(0), CALL_TICKS);
        assert_eq!(ModuleRuntime::frames_deadline(1), CALL_TICKS);
        assert_eq!(ModuleRuntime::frames_deadline(24_650), 24_650 * FRAME_TICKS);
    }

    #[test]
    fn a_module_that_grows_without_end_stops_at_the_cap() {
        let wasm = module(
            "(loop (drop (memory.grow (i32.const 256))) (br 0)) i32.const 0",
            "1",
        );
        let mut h = Host::open(&wasm).unwrap();
        let e = h.call(wire::STEP, &[], CALL_TICKS).unwrap_err();
        assert!(e.contains("stopped"), "{e}");
        assert!(h.memory.data_size(&h.store) <= MAX_MEMORY);
    }

    #[test]
    fn a_module_with_an_import_is_refused() {
        let wasm = wat::parse_str(r#"(module (import "x" "y" (func)))"#).unwrap();
        assert!(Host::open(&wasm).err().unwrap().contains("imports"));
    }

    /// A module compiled once is read back from the cache and runs the
    /// same; an entry that is cut short, has a byte changed, is of another
    /// key or of another module is never used, and the module is compiled
    /// and its entry written again.
    #[test]
    fn the_cache_reads_back_sound_entries_and_compiles_past_any_other() {
        let d = tempfile::tempdir().unwrap();
        let wasm = module("i32.const 1", "1");
        let key = cache::key(engine(), &wasm);
        let file = cache::file(d.path(), &key);
        let run = |code: &ModuleCode| {
            let mut h = Host::start(&code.module).unwrap();
            h.call(wire::STEP, &[], CALL_TICKS).unwrap_err()
        };
        let first = ModuleCode::compile_in(&wasm, Some(d.path())).unwrap();
        let sound = std::fs::read(&file).unwrap();
        assert!(cache::read(engine(), &file, &key).is_some());
        assert_eq!(
            run(&ModuleCode::compile_in(&wasm, Some(d.path())).unwrap()),
            run(&first)
        );

        let mut flipped = sound.clone();
        let last = flipped.len() - 1;
        flipped[last] ^= 1;
        let mut wrong_key = sound.clone();
        wrong_key[MAGIC_LEN] ^= 1;
        for bad in [
            sound[..sound.len() / 2].to_vec(),
            flipped,
            wrong_key,
            Vec::new(),
            b"not a module".to_vec(),
        ] {
            std::fs::write(&file, &bad).unwrap();
            assert!(cache::read(engine(), &file, &key).is_none());
            let again = ModuleCode::compile_in(&wasm, Some(d.path())).unwrap();
            assert_eq!(run(&again), run(&first));
            assert!(cache::read(engine(), &file, &key).is_some());
        }

        // Another module's sound entry, put under this key, is refused.
        let other = module("i32.const 2", "1");
        let other_key = cache::key(engine(), &other);
        ModuleCode::compile_in(&other, Some(d.path())).unwrap();
        std::fs::copy(cache::file(d.path(), &other_key), &file).unwrap();
        assert!(cache::read(engine(), &file, &key).is_none());
        assert_ne!(key, other_key);

        // An import is refused on the cached path too.
        let imports = wat::parse_str(r#"(module (import "x" "y" (func)))"#).unwrap();
        for _ in 0..2 {
            let e = ModuleCode::compile_in(&imports, Some(d.path())).err();
            assert!(e.unwrap().contains("imports"));
        }
    }

    /// Past the cap the least recently used entries go first; a temporary
    /// file stays unless it is stale.
    #[test]
    fn the_cache_cap_removes_the_least_recently_used() {
        let d = tempfile::tempdir().unwrap();
        let now = std::time::SystemTime::now();
        let ago = |s| now - Duration::from_secs(s);
        for (name, at) in [
            ("old.cwasm", ago(60)),
            ("new.cwasm", now),
            (".new.cwasm.tmp-1-1", now),
            (".old.cwasm.tmp-1-1", ago(7200)),
        ] {
            let p = d.path().join(name);
            std::fs::write(&p, [0u8; 1000]).unwrap();
            std::fs::File::options()
                .append(true)
                .open(&p)
                .unwrap()
                .set_modified(at)
                .unwrap();
        }
        cache::evict(d.path(), 1500);
        assert!(!d.path().join("old.cwasm").exists());
        assert!(d.path().join("new.cwasm").exists());
        assert!(d.path().join(".new.cwasm.tmp-1-1").exists());
        assert!(!d.path().join(".old.cwasm.tmp-1-1").exists());
    }

    /// Where the key starts in an entry: after the magic line.
    const MAGIC_LEN: usize = b"gp compiled module 1\n".len();

    #[test]
    fn query_values_is_from_game_0_2_on() {
        assert!(!answers_query_values("game-printer:game@0.1.0"));
        assert!(answers_query_values("game-printer:game@0.2.0"));
        assert!(answers_query_values("game-printer:game@1.0.0"));
        assert!(answers_query_values(gp_runtime::GAME_WIT_PACKAGE));
        assert!(!answers_query_values("game-printer:runtime@0.4.0"));
        assert!(!answers_query_values(""));
    }

    /// A trap's message starts with its cause, not the wasm backtrace: the
    /// page shows a message's first 160 characters.
    #[test]
    fn a_trap_says_its_cause_first() {
        let wasm = module("unreachable", "1");
        let mut h = Host::open(&wasm).unwrap();
        let e = h.call(wire::STEP, &[], CALL_TICKS).unwrap_err();
        assert!(
            e.starts_with("the game's module stopped: wasm trap: wasm `unreachable`"),
            "{e}"
        );
    }

    /// A module that panicked says so in its own words (`gp_panic`), once:
    /// the words are taken, so a later trap gives its own cause.
    #[test]
    fn a_panic_gives_its_words() {
        let words = "panicked at hooks/src/lib.rs:7:9: index out of bounds";
        let wasm = wat::parse_str(format!(
            r#"(module
              (memory (export "memory") 1)
              (data (i32.const 0) "{words}")
              (global $len (mut i32) (i32.const 0))
              (global $said (mut i32) (i32.const 0))
              (func (export "gp_alloc") (param i32) (result i32) i32.const 64)
              (func (export "gp_out_ptr") (result i32) i32.const 0)
              (func (export "gp_out_len") (result i32) global.get $len)
              (func (export "gp_panic") (result i32)
                (if (global.get $said) (then (return (i32.const 0))))
                (global.set $said (i32.const 1))
                (global.set $len (i32.const {n}))
                global.get $len)
              (func (export "gp_rt") (param i32 i32 i32) (result i32) unreachable))"#,
            n = words.len()
        ))
        .unwrap();
        let mut h = Host::open(&wasm).unwrap();
        let e = h.call(wire::STEP, &[], CALL_TICKS).unwrap_err();
        assert!(
            e.starts_with(&format!("the game's module stopped: {words}")),
            "{e}"
        );
        let e = h.call(wire::STEP, &[], CALL_TICKS).unwrap_err();
        assert!(e.contains("wasm trap"), "{e}");
    }

    #[test]
    fn a_failing_call_gives_its_words() {
        let wasm = module("i32.const 1", "1");
        let mut h = Host::open(&wasm).unwrap();
        assert_eq!(h.call(wire::STEP, &[], CALL_TICKS).unwrap_err(), "");
    }
}
