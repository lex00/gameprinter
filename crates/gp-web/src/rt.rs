//! The runtime protocol inside a game's module: what `gp`'s host calls, as an
//! op number and bytes (`gp_runtime::wire`). One instance holds any number
//! of sessions, so `fork` is a clone inside the module.

use std::collections::BTreeMap;

use gp_interp::Interp;
use gp_interp::lifted::Bundle;
use gp_runtime::Runtime as _;
use gp_runtime::wire::{self, Reader, Writer};

#[derive(Default)]
pub struct Sessions {
    next: u32,
    live: BTreeMap<u32, Interp>,
}

impl Sessions {
    fn add(&mut self, i: Interp) -> u32 {
        self.next += 1;
        self.live.insert(self.next, i);
        self.next
    }

    fn get(&mut self, id: u32) -> Result<&mut Interp, String> {
        self.live
            .get_mut(&id)
            .ok_or_else(|| format!("no session {id}"))
    }

    fn load(&mut self, bundle: &[u8], w: &mut Writer) -> Result<(), String> {
        let bundle = Bundle::from_bytes(bundle)?;
        let interp = Interp::from_bundle(&bundle)?;
        let (exts, label) = gp_exts::registry(&interp.spec().name);
        let mut hooks: Vec<String> = exts.keys().map(|k| k.to_string()).collect();
        hooks.sort();
        let interp = interp.with_exts(std::sync::Arc::new(exts));
        let missing = interp.ext_hooks();
        if !missing.is_empty() {
            return Err(format!(
                "the model `{}` names hooks this module doesn't have: {}",
                interp.spec().name,
                missing.join(", ")
            ));
        }
        let id = self.add(interp);
        w.u32(id);
        w.str(&label);
        w.strs(&hooks);
        Ok(())
    }

    /// Runs one call: its bytes in, its answer in `w`.
    pub fn call(&mut self, op: u32, input: &[u8], w: &mut Writer) -> Result<(), String> {
        if op == wire::LOAD {
            return self.load(input, w);
        }
        if op == wire::INFO {
            w.str(gp_runtime::WIT_PACKAGE);
            return Ok(());
        }
        let mut r = Reader::new(input);
        let id = r.u32()?;
        match op {
            wire::FORK => {
                let f = self.get(id)?.fork();
                w.u32(self.add(f));
            }
            wire::DROP => {
                self.live.remove(&id);
            }
            wire::STEP => {
                let inputs = r.inputs()?;
                w.events(&self.get(id)?.step(&inputs)?);
            }
            wire::RUN_UNTIL => {
                let condition = r.str()?;
                let inputs = r.inputs()?;
                let max = r.u32()?;
                w.until(&self.get(id)?.run_until(&condition, &inputs, max)?);
            }
            wire::FRAME => w.u64(self.get(id)?.frame()),
            wire::CANONICAL_STATE => w.0.extend(self.get(id)?.canonical_state()),
            wire::STATE_HASH => w.u64(self.get(id)?.state_hash()),
            wire::WORLD_HASH => w.0.extend(self.get(id)?.world().hash()),
            wire::LOAD_STATE => {
                let state = r.bytes()?;
                w.notes(&self.get(id)?.load_state(state)?);
            }
            wire::RESTORE => {
                let snap = r.bytes()?;
                self.get(id)?.restore(snap)?;
            }
            wire::QUERY => {
                let selector = r.str()?;
                w.entries(&self.get(id)?.query(&selector)?);
            }
            wire::SAMPLE => {
                let selectors = r.strs()?;
                let inputs = r.inputs()?;
                let frames = r.u32()?;
                let rows = self.get(id)?.sample(&selectors, &inputs, frames)?;
                w.count(rows.len());
                for row in &rows {
                    w.entries(row);
                }
            }
            wire::ENTITIES_AT => {
                let area = r.rect()?;
                let ids = self.get(id)?.entities_at(area);
                w.count(ids.len());
                for e in ids {
                    w.u64(e);
                }
            }
            wire::SET => {
                let path = r.str()?;
                let value = r.value()?;
                self.get(id)?.set(&path, value)?;
            }
            wire::EXPLAIN => {
                let path = r.str()?;
                w.strs(&self.get(id)?.explain(&path));
            }
            wire::SET_DRAWS => {
                let script = r.str()?;
                let base = r.u64()?;
                let mut d = gp_sim::DrawScript::parse(&script)?;
                d.base = base;
                let i = self.live.remove(&id).ok_or("no such session")?;
                self.live.insert(id, i.with_draws(d));
            }
            wire::REPORT_DRAWS => match self.get(id)?.draws() {
                None => w.bool(false),
                Some(d) => {
                    w.bool(true);
                    w.u32(d.next as u32);
                    w.u32(d.draws.len() as u32);
                    w.bool(d.desync.is_some());
                    if let Some((f, why)) = &d.desync {
                        w.u64(*f);
                        w.str(why);
                    }
                    w.count(d.desyncs.len());
                    for (f, site, why) in &d.desyncs {
                        w.u64(*f);
                        w.str(site);
                        w.str(why);
                    }
                }
            },
            wire::SET_APU_EVENTS => {
                let i = self.live.remove(&id).ok_or("no such session")?;
                self.live.insert(id, i.with_apu_events());
            }
            wire::DESCRIBE => gp_interp::describe::write_exts(self.get(id)?.exts(), w),
            wire::START_READS => {
                self.get(id)?;
                gp_sim::reads::start();
            }
            wire::TAKE_READS => {
                self.get(id)?;
                gp_interp::reads::write_reads(&gp_sim::reads::take(), w);
            }
            wire::FLAT_LEVEL => {
                let level = r.u32()? as usize;
                self.get(id)?
                    .edit_spec(|s| gp_interp::variant::flat_floor(s, level))?;
            }
            wire::SPEC_DIGEST => w.str(&gp_sim::save::spec_digest(self.get(id)?.spec())),
            wire::SET_DRAW_SCRIPT => {
                let mut d = gp_sim::DrawScript::parse(&r.str()?)?;
                d.base = r.u64()?;
                d.origin = r.opt_u64()?;
                d.lenient = r.bool()?;
                if r.bool()? {
                    d.calls = Some(Vec::new());
                }
                let i = self.live.remove(&id).ok_or("no such session")?;
                self.live.insert(id, i.with_draws(d));
            }
            wire::DRAW_CALLS => match self.get(id)?.draws().and_then(|d| d.calls.as_ref()) {
                None => w.bool(false),
                Some(calls) => {
                    w.bool(true);
                    w.count(calls.len());
                    for (frame, site) in calls {
                        w.u64(*frame);
                        w.str(site);
                    }
                }
            },
            wire::STEP_HOOKS => {
                let state = r.bytes()?;
                let i = self.get(id)?;
                let mut world = gp_sim::World::from_canonical(i.spec(), state)?;
                let drew = gp_interp::compare::step_hooks(i.spec(), &mut world, i.exts());
                w.bool(drew);
                w.0.extend(world.canonical());
            }
            _ => return Err(format!("this module has no call {op}")),
        }
        Ok(())
    }
}
