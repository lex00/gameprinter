//! A run's reads by name ([`ReadSet`]): which kinds, levels, machines,
//! counters and cameras of the spec it read, and which `ext` hooks it
//! called, from what `gp_sim::reads` noted by index. A clip's score keeps
//! it, so a scorer can tell which clips a change to the model or a hook
//! can move (docs/gate.md, "Affected clips").

use std::collections::{BTreeMap, BTreeSet};

use gp_sim::reads::{List, Reads};
use serde::{Deserialize, Serialize};

/// What a run read, by list (`kinds`, `levels`, `machines`, `counters`,
/// `cameras`, and `hooks`) and name; a list read whole has the one name
/// [`ALL`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ReadSet(pub BTreeMap<String, BTreeSet<String>>);

/// The name that stands for every element of a list.
pub const ALL: &str = "*";

impl ReadSet {
    /// `r` by name, through the spec it was read from.
    pub fn of(r: &Reads, names: &SpecNames) -> Self {
        let mut out = BTreeMap::new();
        for l in List::ALL {
            let seen = r.part(l);
            let set: BTreeSet<String> = if seen.all {
                [ALL.to_owned()].into()
            } else {
                seen.some()
                    .into_iter()
                    .filter_map(|i| names.get(l, i).map(str::to_owned))
                    .collect()
            };
            if !set.is_empty() {
                out.insert(l.name().to_owned(), set);
            }
        }
        if !r.hooks.is_empty() {
            out.insert("hooks".to_owned(), r.hooks.clone());
        }
        Self(out)
    }

    pub fn union(&mut self, o: &ReadSet) {
        for (k, v) in &o.0 {
            let mine = self.0.entry(k.clone()).or_default();
            if mine.contains(ALL) || v.contains(ALL) {
                *mine = [ALL.to_owned()].into();
            } else {
                mine.extend(v.iter().cloned());
            }
        }
    }

    /// Whether it read any of `changed`: names by list, as here.
    pub fn meets(&self, changed: &BTreeMap<String, BTreeSet<String>>) -> bool {
        changed.iter().any(|(list, names)| {
            self.0.get(list).is_some_and(|mine| {
                !names.is_empty() && (mine.contains(ALL) || names.iter().any(|n| mine.contains(n)))
            })
        })
    }
}

/// The names of a spec's lists, by index, read without noting a read.
#[derive(Debug, Clone, Default)]
pub struct SpecNames(BTreeMap<List, Vec<String>>);

impl SpecNames {
    pub fn of(spec: &gp_sim::Spec) -> Self {
        let mut m = BTreeMap::new();
        m.insert(List::Kind, spec.kinds.names().map(str::to_owned).collect());
        m.insert(
            List::Level,
            spec.levels.names().map(str::to_owned).collect(),
        );
        m.insert(
            List::Machine,
            spec.machines.names().map(str::to_owned).collect(),
        );
        m.insert(
            List::Counter,
            spec.counters.names().map(str::to_owned).collect(),
        );
        m.insert(
            List::Camera,
            spec.cameras.names().map(str::to_owned).collect(),
        );
        Self(m)
    }

    fn get(&self, l: List, i: usize) -> Option<&str> {
        self.0.get(&l).and_then(|v| v.get(i)).map(String::as_str)
    }
}

/// `r` as bytes, as a game's module gives it across its boundary: for each
/// list whether it was read whole and which elements, then the hooks.
pub fn write_reads(r: &Reads, w: &mut gp_runtime::wire::Writer) {
    for l in List::ALL {
        let seen = r.part(l);
        w.bool(seen.all);
        let some = seen.some();
        w.count(some.len());
        for i in some {
            w.u32(i as u32);
        }
    }
    w.strs(&r.hooks.iter().cloned().collect::<Vec<_>>());
}

/// The bytes [`write_reads`] made.
pub fn read_reads(bytes: &[u8]) -> Result<Reads, String> {
    let mut r = gp_runtime::wire::Reader::new(bytes);
    let mut out = Reads::default();
    for l in List::ALL {
        let all = r.bool()?;
        let some: Vec<usize> = (0..r.count()?)
            .map(|_| Ok(r.u32()? as usize))
            .collect::<Result<_, String>>()?;
        out.parts[l as usize] = gp_sim::reads::Seen::of(all, some);
    }
    out.hooks = r.strs()?.into_iter().collect();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_cross_a_boundary_unchanged() {
        let mut r = Reads::default();
        r.parts[List::Kind as usize] = gp_sim::reads::Seen::of(false, [1, 70]);
        r.parts[List::Level as usize] = gp_sim::reads::Seen::of(true, []);
        r.hooks.insert("dk_barrel".into());
        let mut w = gp_runtime::wire::Writer::default();
        write_reads(&r, &mut w);
        let back = read_reads(&w.0).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn a_read_set_meets_a_change_to_what_it_read_and_to_a_list_it_read_whole() {
        let mut a = ReadSet::default();
        a.0.insert("kinds".into(), ["barrel".to_owned()].into());
        a.0.insert("hooks".into(), ["dk_barrel".to_owned()].into());
        let ch = |l: &str, n: &str| -> BTreeMap<String, BTreeSet<String>> {
            [(l.to_owned(), [n.to_owned()].into())].into()
        };
        assert!(a.meets(&ch("kinds", "barrel")));
        assert!(!a.meets(&ch("kinds", "spring")));
        assert!(a.meets(&ch("hooks", "dk_barrel")));
        assert!(!a.meets(&ch("levels", "m75")));
        let mut b = ReadSet::default();
        b.0.insert("levels".into(), [ALL.to_owned()].into());
        a.union(&b);
        assert!(a.meets(&ch("levels", "m75")));
        let mut c = ReadSet::default();
        c.0.insert("kinds".into(), ["spring".to_owned()].into());
        a.union(&c);
        assert!(a.meets(&ch("kinds", "spring")));
    }
}
