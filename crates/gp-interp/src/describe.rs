//! A hook's mechanic records across a game's module boundary: the module
//! writes them ([`write_exts`]) and the host reads them back into
//! [`Described`] hooks, which answer `describe` and nothing else, so the
//! host's catalog is the module's without any hook running on the host.

use std::collections::BTreeMap;

use gp_runtime::wire::{Reader, Writer};
use gp_sim::ext::{Ext, ExtCx, Exts};
use gp_sim::mechanic::{Carrier, Param, Primitive, Record, Table, TableKind};

fn strs(w: &mut Writer, list: &[&'static str]) {
    w.count(list.len());
    for s in list {
        w.str(s);
    }
}

fn pairs(w: &mut Writer, list: &[(&'static str, &'static str)]) {
    w.count(list.len());
    for (a, b) in list {
        w.str(a);
        w.str(b);
    }
}

fn write_record(r: &Record, w: &mut Writer) {
    for s in [r.name, r.version, r.carrier.name(), r.system, r.class] {
        w.str(s);
    }
    for s in [r.implementation, r.like, r.about] {
        w.str(s);
    }
    w.count(r.params.len());
    for p in r.params {
        for s in [p.name, p.ty, p.unit, p.range] {
            w.str(s);
        }
        w.bool(p.optional);
    }
    w.count(r.tables.len());
    for t in r.tables {
        w.str(t.name);
        w.str(t.kind.name());
    }
    for list in [r.state, r.reads, r.writes, r.emits, r.consumes] {
        strs(w, list);
    }
    for list in [r.spawns, r.assumes, r.after, r.excludes] {
        strs(w, list);
    }
    w.count(r.built_from.len());
    for p in r.built_from {
        w.str(p.name());
    }
    strs(w, r.invariants);
    pairs(w, r.measures);
    pairs(w, r.envelope);
}

/// Every hook of `exts` with its records, in name order.
pub fn write_exts(exts: &Exts, w: &mut Writer) {
    w.count(exts.len());
    for (hook, ext) in exts {
        w.str(hook);
        let records = ext.describe();
        w.count(records.len());
        for r in &records {
            write_record(r, w);
        }
    }
}

/// A string kept for the process's life: a [`Record`] holds only `'static`
/// strings, and a catalog is read a few times a run.
fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

fn leak_list<T>(v: Vec<T>) -> &'static [T] {
    Box::leak(v.into_boxed_slice())
}

fn list_of(r: &mut Reader) -> Result<&'static [&'static str], String> {
    let v = (0..r.count()?)
        .map(|_| Ok(leak(r.str()?)))
        .collect::<Result<Vec<_>, String>>()?;
    Ok(leak_list(v))
}

fn pairs_of(r: &mut Reader) -> Result<&'static [(&'static str, &'static str)], String> {
    let v = (0..r.count()?)
        .map(|_| Ok((leak(r.str()?), leak(r.str()?))))
        .collect::<Result<Vec<_>, String>>()?;
    Ok(leak_list(v))
}

fn read_record(r: &mut Reader) -> Result<Record, String> {
    let name = leak(r.str()?);
    let version = leak(r.str()?);
    let carrier = match r.str()?.as_str() {
        "entity" => Carrier::Entity,
        "level" => Carrier::Level,
        "game" => Carrier::Game,
        c => return Err(format!("no carrier `{c}`")),
    };
    let system = leak(r.str()?);
    let class = leak(r.str()?);
    let implementation = leak(r.str()?);
    let like = leak(r.str()?);
    let about = leak(r.str()?);
    let params = (0..r.count()?)
        .map(|_| {
            Ok(Param {
                name: leak(r.str()?),
                ty: leak(r.str()?),
                unit: leak(r.str()?),
                range: leak(r.str()?),
                optional: r.bool()?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let tables = (0..r.count()?)
        .map(|_| {
            let name = leak(r.str()?);
            let kind = match r.str()?.as_str() {
                "mechanic" => TableKind::Mechanic,
                "content" => TableKind::Content,
                k => return Err(format!("no table kind `{k}`")),
            };
            Ok(Table { name, kind })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let state = list_of(r)?;
    let reads = list_of(r)?;
    let writes = list_of(r)?;
    let emits = list_of(r)?;
    let consumes = list_of(r)?;
    let spawns = list_of(r)?;
    let assumes = list_of(r)?;
    let after = list_of(r)?;
    let excludes = list_of(r)?;
    let built_from = (0..r.count()?)
        .map(|_| {
            let n = r.str()?;
            Primitive::ALL
                .into_iter()
                .find(|p| p.name() == n)
                .ok_or_else(|| format!("no primitive `{n}`"))
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(Record {
        name,
        version,
        carrier,
        system,
        class,
        implementation,
        like,
        about,
        params: leak_list(params),
        tables: leak_list(tables),
        state,
        reads,
        writes,
        emits,
        consumes,
        spawns,
        assumes,
        after,
        excludes,
        built_from: leak_list(built_from),
        invariants: list_of(r)?,
        measures: pairs_of(r)?,
        envelope: pairs_of(r)?,
    })
}

/// A hook known only by its records, on a host that runs no hooks.
pub struct Described(pub Vec<Record>);

impl Ext for Described {
    fn step(&self, _cx: &mut ExtCx<'_>) {
        unreachable!("a described hook runs in its module");
    }

    fn describe(&self) -> Vec<Record> {
        self.0.clone()
    }
}

/// The bytes [`write_exts`] made, as hooks that only describe themselves.
pub fn read_exts(bytes: &[u8]) -> Result<Exts, String> {
    let mut r = Reader::new(bytes);
    let mut out: Exts = BTreeMap::new();
    for _ in 0..r.count()? {
        let hook = r.str()?;
        let records = (0..r.count()?)
            .map(|_| read_record(&mut r))
            .collect::<Result<Vec<_>, String>>()?;
        out.insert(hook, Box::new(Described(records)));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Plain(Vec<Record>);

    impl Ext for Plain {
        fn step(&self, _cx: &mut ExtCx<'_>) {}
        fn describe(&self) -> Vec<Record> {
            self.0.clone()
        }
    }

    #[test]
    fn a_hooks_records_cross_the_boundary_unchanged() {
        const P: &[Param] = &[Param::new("speed", "fixed", "px/frame").optional()];
        const T: &[Table] = &[Table {
            name: "ways",
            kind: TableKind::Content,
        }];
        let rec = Record {
            name: "dk.fire",
            version: "1",
            carrier: Carrier::Game,
            system: "rules",
            class: "mover",
            params: P,
            tables: T,
            reads: &["self.x"],
            writes: &["self.vx", "counters"],
            built_from: &[Primitive::Timer],
            measures: &[("speed", "px/frame")],
            ..Record::EMPTY
        };
        let mut exts: Exts = BTreeMap::new();
        exts.insert("fire".into(), Box::new(Plain(vec![rec])));
        exts.insert("quiet".into(), Box::new(Plain(Vec::new())));
        let mut w = Writer::default();
        write_exts(&exts, &mut w);
        let back = read_exts(&w.0).unwrap();
        assert_eq!(back["fire"].describe(), vec![rec]);
        assert!(back["quiet"].describe().is_empty());
        assert_eq!(back.len(), 2);
    }
}
