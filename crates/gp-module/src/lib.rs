//! A game's module as a [`Runtime`]: wasmtime with NaN canonicalization on,
//! relaxed SIMD off and no imports, a deadline on every call and a cap on
//! memory, so a bad module stalls nothing but itself.
//!
//! One Wasm instance holds a game's sessions. [`ModuleRuntime::fork`] makes
//! another inside it, so forks of one runtime share that instance and take
//! turns running.

use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use gp_runtime::wire::{self, Reader, Writer};
use gp_runtime::{
    Capabilities, Entry, Event, InputFrame, LoadNote, PatchResult, Rect, Runtime, Until, Value,
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
}

impl Host {
    fn open(wasm: &[u8]) -> Result<Self, String> {
        let module =
            Module::new(engine(), wasm).map_err(|e| format!("the module doesn't compile: {e}"))?;
        if module.imports().len() != 0 {
            return Err("the module imports something, so gp won't run it".into());
        }
        let limits = StoreLimitsBuilder::new()
            .memory_size(MAX_MEMORY)
            .trap_on_grow_failure(true)
            .build();
        let mut store = Store::new(engine(), limits);
        store.limiter(|l| l);
        store.set_epoch_deadline(CALL_TICKS);
        let instance = Instance::new(&mut store, &module, &[])
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
            memory,
            store,
        })
    }

    /// `ticks` is the call's deadline. An error says what limit stopped it.
    fn call(&mut self, op: u32, input: &[u8], ticks: u64) -> Result<Vec<u8>, String> {
        self.store.set_epoch_deadline(ticks);
        let stopped = |e: wasmtime::Error| {
            if matches!(
                e.downcast_ref::<wasmtime::Trap>(),
                Some(wasmtime::Trap::Interrupt)
            ) {
                format!(
                    "the game's module stopped: it ran past its deadline of {} ms",
                    ticks * TICK_MS
                )
            } else {
                format!("the game's module stopped: {e:#}")
            }
        };
        let len = u32::try_from(input.len()).map_err(|_| "a call of more than 4 GB")?;
        let p = self.alloc.call(&mut self.store, len).map_err(stopped)?;
        self.memory
            .write(&mut self.store, p as usize, input)
            .map_err(|e| e.to_string())?;
        let code = self
            .rt
            .call(&mut self.store, (op, p, len))
            .map_err(stopped)?;
        let ptr = self.out_ptr.call(&mut self.store, ()).map_err(stopped)? as usize;
        let n = self.out_len.call(&mut self.store, ()).map_err(stopped)? as usize;
        let mut out = vec![0; n];
        self.memory
            .read(&self.store, ptr, &mut out)
            .map_err(|e| e.to_string())?;
        if code != 0 {
            return Err(String::from_utf8_lossy(&out).into_owned());
        }
        Ok(out)
    }
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
    /// The hooks' label, as a recording stores it.
    pub label: String,
    /// The hooks the module has.
    pub hooks: Vec<String>,
}

impl ModuleRuntime {
    /// Opens a module and loads a model bundle into it.
    pub fn load(wasm: &[u8], bundle: &[u8]) -> Result<Self, String> {
        let mut host = Host::open(wasm)?;
        let out = host.call(wire::LOAD, bundle, CALL_TICKS)?;
        let mut r = Reader::new(&out);
        Ok(Self {
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

    fn frames_deadline(frames: usize) -> u64 {
        CALL_TICKS.max(FRAME_TICKS.saturating_mul(frames as u64))
    }

    /// A frame's deadline when it's one frame: the design's ten frame-times.
    fn step_deadline(frames: usize) -> u64 {
        if frames <= 1 {
            FRAME_TICKS
        } else {
            Self::frames_deadline(frames)
        }
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
        let desyncs = (0..r.len()?)
            .map(|_| Ok((r.u64()?, r.str()?, r.str()?)))
            .collect::<Result<_, String>>()?;
        Ok(Some(DrawsReport {
            taken,
            script,
            desync,
            desyncs,
        }))
    }

    /// Makes `step` return the hooks' sound-driver writes as events.
    pub fn set_apu_events(&mut self) -> Result<(), String> {
        self.call(wire::SET_APU_EVENTS, CALL_TICKS, |_| {})
            .map(|_| ())
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
        let out = self.call(wire::STEP, Self::step_deadline(inputs.len()), |w| {
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

    fn restore(&mut self, snap: &[u8]) -> Result<(), String> {
        self.load_state(snap).map(|_| ())
    }

    fn fork(&self) -> Self {
        let out = must(self.call(wire::FORK, CALL_TICKS, |_| {}));
        Self {
            host: Arc::clone(&self.host),
            id: must(Reader::new(&out).u32()),
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
        (0..r.len()?).map(|_| r.entries()).collect()
    }

    fn entities_at(&self, area: Rect) -> Vec<u64> {
        let out = must(self.call(wire::ENTITIES_AT, CALL_TICKS, |w| w.rect(area)));
        let mut r = Reader::new(&out);
        must((0..must(r.len())).map(|_| r.u64()).collect())
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

    #[test]
    fn a_failing_call_gives_its_words() {
        let wasm = module("i32.const 1", "1");
        let mut h = Host::open(&wasm).unwrap();
        assert_eq!(h.call(wire::STEP, &[], CALL_TICKS).unwrap_err(), "");
    }
}
