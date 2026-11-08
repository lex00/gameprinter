//! The carry rule on made-up states: a small model's pacer (a patrol)
//! in four slots of a RAM image, with one mapping changed at a time.

use super::*;
use crate::mapping::Mapping;
use gp_runtime::{Entry, InputFrame};

/// An original that is only RAM.
#[derive(Clone)]
struct Ram(Vec<u8>);

impl Runtime for Ram {
    fn capabilities(&self) -> gp_runtime::Capabilities {
        gp_runtime::Capabilities::FORK
    }
    fn step(&mut self, _: &[InputFrame]) -> Result<Vec<gp_runtime::Event>, String> {
        Ok(Vec::new())
    }
    fn run_until(
        &mut self,
        _: &str,
        _: &[InputFrame],
        _: u32,
    ) -> Result<gp_runtime::Until, String> {
        Err("no".into())
    }
    fn frame(&self) -> u64 {
        0
    }
    fn snapshot(&self) -> Vec<u8> {
        self.0.clone()
    }
    fn restore(&mut self, s: &[u8]) -> Result<(), String> {
        self.0 = s.to_vec();
        Ok(())
    }
    fn load_state(&mut self, _: &[u8]) -> Result<Vec<gp_runtime::LoadNote>, String> {
        Err("an original has no model state".into())
    }
    fn fork(&self) -> Self {
        self.clone()
    }
    fn canonical_state(&self) -> Vec<u8> {
        self.0.clone()
    }
    fn state_hash(&self) -> u64 {
        0
    }
    fn query(&self, sel: &str) -> Result<Vec<Entry>, String> {
        let a = sel
            .strip_prefix("ram[0x")
            .and_then(|s| s.strip_suffix(']'))
            .and_then(|h| usize::from_str_radix(h, 16).ok())
            .ok_or(format!("{sel}?"))?;
        Ok(vec![Entry {
            path: sel.into(),
            value: Value::Int(i64::from(self.0[a])),
        }])
    }
    fn sample(
        &mut self,
        _: &[String],
        _: &[InputFrame],
        _: u32,
    ) -> Result<Vec<Vec<Entry>>, String> {
        Err("no".into())
    }
    fn entities_at(&self, _: gp_runtime::Rect) -> Vec<u64> {
        Vec::new()
    }
    fn patch(&mut self, _: &str) -> gp_runtime::PatchResult {
        gp_runtime::PatchResult::NeedsReload
    }
    fn set(&mut self, _: &str, _: Value) -> Result<(), String> {
        Err("no".into())
    }
    fn explain(&self, _: &str) -> Vec<String> {
        Vec::new()
    }
}

/// Slots at 0x10 (flag), 0x18 (ID), 0x1C (state), x at 0x20 and 0x28, y
/// at 0x30, facing at 0x38; `kind` is the pacer's kind table, `extra` more
/// slot fields.
fn mapping(kind: &str, extra: &str) -> Mapping {
    Mapping::parse(&format!(
        r#"
schema = 17
model = "tiny"
source = "tiny"
align = "polled"

[[field]]
path = "entities.hero[0].x"
ram = [0x40, 0x41]

[slots]
count = 4
active = 0x10
id = 0x18
clock = 0x14
state = 0x1C
ends = 0x20

[[slots.field]]
path = "x"
ram = [0x20, 0x28]

[[slots.field]]
path = "y"
ram = [0x30]
{extra}
[[slots.kind]]
entity = "pacer"
ids = [7]
{kind}
"#
    ))
    .unwrap()
}

const FACING: &str = r#"
[[slots.field]]
path = "facing"
ram = [0x38]
values = ["right", "left"]
"#;

/// One pacer loaded in slot 0, at x 100, y 120, facing left.
fn one() -> Ram {
    let mut r = vec![0u8; 0x80];
    r[0x10] = 1;
    r[0x18] = 7;
    r[0x20] = 100;
    r[0x30] = 120;
    r[0x38] = 1;
    Ram(r)
}

fn spec() -> Spec {
    crate::tests::pacers(&[(100, 40)]).spec().clone()
}

fn pacer(s: &Spec) -> usize {
    s.kinds.iter().position(|k| k.name == "pacer").unwrap()
}

#[test]
fn a_patrol_needs_its_place_and_facing() {
    let s = spec();
    let n: Vec<String> = needs(&s, pacer(&s)).into_iter().map(|n| n.path).collect();
    assert_eq!(n, ["x", "y", "facing"]);
}

#[test]
fn a_field_the_mapping_does_not_read_is_a_gap() {
    let s = spec();
    let g = gaps(&one(), &s, &mapping("", "")).unwrap();
    assert_eq!(g.len(), 1);
    assert_eq!(g[0].cause, Cause::MissingField);
    assert_eq!(g[0].field.as_deref(), Some("facing"));
    assert_eq!(g[0].entity.as_deref(), Some("pacer"));
    let why = says(&g).unwrap();
    assert!(
        why.contains("pacer") && why.contains("facing") && why.contains("can't carry"),
        "{why}"
    );
}

#[test]
fn a_kind_whose_fields_are_all_read_carries() {
    let s = spec();
    assert!(gaps(&one(), &s, &mapping("", FACING)).unwrap().is_empty());
    // The kind's own field does as well as the slots'.
    let own = FACING.replace("[[slots.field]]", "[[slots.kind.field]]");
    assert!(gaps(&one(), &s, &mapping(&own, "")).unwrap().is_empty());
}

#[test]
fn an_id_no_kind_names_is_an_unmapped_kind() {
    let s = spec();
    let mut r = one();
    r.0[0x18] = 9;
    let g = gaps(&r, &s, &mapping("", FACING)).unwrap();
    assert_eq!(g.len(), 1);
    assert_eq!(g[0].cause, Cause::UnmappedKind);
    assert_eq!(g[0].entity, None);
    assert_eq!(g[0].what(), "object 0x09");
}

#[test]
fn a_value_the_mapping_names_nothing_is_a_state_not_named() {
    let s = spec();
    let mut r = one();
    r.0[0x38] = 5;
    let g = gaps(&r, &s, &mapping("", FACING)).unwrap();
    assert_eq!(g.len(), 1);
    assert_eq!(g[0].cause, Cause::StateNotNamed);
}

#[test]
fn an_ended_object_is_gone_and_no_gap() {
    let s = spec();
    let mut r = one();
    r.0[0x1C] = 0x22;
    assert!(gaps(&r, &s, &mapping("", "")).unwrap().is_empty());
}

#[test]
fn refuse_and_leave_say_so_or_say_nothing() {
    let s = spec();
    let g = gaps(
        &one(),
        &s,
        &mapping("refuse = \"it moves in a way the model can't\"", FACING),
    )
    .unwrap();
    assert_eq!(g.len(), 1);
    assert_eq!(g[0].cause, Cause::Refused);
    assert_eq!(g[0].why, "it moves in a way the model can't");
    let left = gaps(&one(), &s, &mapping("leave = \"only drawn\"", "")).unwrap();
    assert!(left.is_empty());
}

#[test]
fn a_second_part_under_another_flag_is_carried_with_its_kind() {
    let s = spec();
    let mut r = one();
    r.0[0x10] = 2;
    assert!(
        gaps(&r, &s, &mapping("flags = [1]", ""))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn an_empty_slot_and_no_slots_have_no_gaps() {
    let s = spec();
    let mut r = one();
    r.0[0x10] = 0;
    assert!(gaps(&r, &s, &mapping("", "")).unwrap().is_empty());
    let mut m = mapping("", "");
    m.slots = None;
    assert!(gaps(&one(), &s, &m).unwrap().is_empty());
}

#[test]
fn the_carry_is_driven_by_the_mapping_alone() {
    // The same pacer under another ID and other addresses carries the same
    // once its rows say where it is: no code names it.
    let s = spec();
    let mut r = one();
    r.0[0x18] = 0x33;
    r.0[0x50] = 1;
    let m = mapping("", FACING).slots.unwrap();
    let mut m2 = mapping("", FACING);
    let sl = m2.slots.as_mut().unwrap();
    sl.kinds[0].ids = vec![0x33];
    sl.fields[2].ram = vec![0x50];
    assert_eq!(m.kinds[0].entity, sl.kinds[0].entity);
    let g = gaps(&r, &s, &m2).unwrap();
    assert!(g.is_empty(), "{g:?}");
    // Read at 0x50, the facing loads left.
    r.0[0x50] = 7;
    assert_eq!(gaps(&r, &s, &m2).unwrap()[0].cause, Cause::StateNotNamed);
}

#[test]
fn sparse_names_read_by_number() {
    let f: crate::mapping::Field = toml::from_str(
        r#"
path = "state"
ram = [0x1C]
names = [[0, "walking"], [1, "walking"], [4, "shell"], [0x84, "sliding"]]
"#,
    )
    .unwrap();
    assert_eq!(f.value(&[1]), Value::Text("walking".into()));
    assert_eq!(f.value(&[0x84]), Value::Text("sliding".into()));
    assert_eq!(f.value(&[2]), Value::Text("(unnamed value 2)".into()));
    assert!(f.named());
}

#[test]
fn the_new_keys_need_schema_17() {
    for (kind, extra) in [
        ("refuse = \"no\"", ""),
        ("leave = \"no\"", ""),
        ("", "[[slots.field]]\npath = \"time\"\nram = [0x60]\n"),
        (
            "",
            "[[slots.field]]\npath = \"facing\"\nram = [0x38]\nnames = [[0, \"right\"]]\n",
        ),
    ] {
        let text = format!(
            "schema = 16\nmodel = \"tiny\"\nsource = \"tiny\"\nalign = \"polled\"\n[[field]]\npath = \"entities.hero[0].x\"\nram = [0x40]\n[slots]\ncount = 1\nactive = 0x10\nid = 0x18\nclock = 0x14\n[[slots.field]]\npath = \"x\"\nram = [0x20]\n{extra}[[slots.kind]]\nentity = \"pacer\"\nids = [7]\n{kind}\n"
        );
        let e = Mapping::parse(&text).unwrap_err();
        assert!(e.contains("schema 17"), "{e}");
        assert!(Mapping::parse(&text.replace("schema = 16", "schema = 17")).is_ok());
    }
    let both = "refuse = \"a\"\nleave = \"b\"";
    let text = format!(
        "schema = 17\nmodel = \"tiny\"\nsource = \"tiny\"\nalign = \"polled\"\n[[field]]\npath = \"entities.hero[0].x\"\nram = [0x40]\n[slots]\ncount = 1\nactive = 0x10\nid = 0x18\nclock = 0x14\n[[slots.field]]\npath = \"x\"\nram = [0x20]\n[[slots.kind]]\nentity = \"pacer\"\nids = [7]\n{both}\n"
    );
    assert!(Mapping::parse(&text).is_err());
}

/// A refused or left-out kind takes no slot life and no body.
#[test]
fn refused_and_left_kinds_are_not_bodies() {
    let m = mapping("refuse = \"no\"", FACING);
    let g = m.slots.as_ref().unwrap();
    assert!(crate::bodies::slot_objects(&one(), g).unwrap().is_empty());
    let m = mapping("", FACING);
    assert_eq!(
        crate::bodies::slot_objects(&one(), m.slots.as_ref().unwrap())
            .unwrap()
            .len(),
        1
    );
}

/// While the slots' `vacant` holds, every slot reads free.
#[test]
fn vacant_slots_hold_no_objects() {
    let m = mapping("", FACING);
    let mut g = m.slots.clone().unwrap();
    g.vacant = Some("ram[0x0038] == 1".into());
    assert!(crate::bodies::slot_objects(&one(), &g).unwrap().is_empty());
    g.vacant = Some("ram[0x0038] == 0".into());
    assert_eq!(crate::bodies::slot_objects(&one(), &g).unwrap().len(), 1);
}
