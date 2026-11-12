//! The ROM-blocking hook guards every chip of every set this crate knows:
//! `.githooks/guarded-roms.toml`'s `[[chip]]` rows hold the same sets,
//! names, regions, offsets, sizes and SHA-1s as [`gp_arcade::romset::SETS`]
//! and [`gp_arcade::romset::SSIO_PROM`].

use std::collections::BTreeSet;

use gp_arcade::romset::{Region, SETS, SSIO_PROM};

type Row = (String, String, String, usize, usize, String);

/// The `[[chip]]` rows of the hook's list, read line by line: the file is
/// written one `key = value` a line.
fn guarded() -> BTreeSet<Row> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../.githooks/guarded-roms.toml"
    );
    let text = std::fs::read_to_string(path).expect("the hook's guarded-roms.toml");
    let mut rows = BTreeSet::new();
    let mut cur: Option<Vec<(String, String)>> = None;
    let mut flush = |cur: &mut Option<Vec<(String, String)>>| {
        if let Some(kv) = cur.take() {
            let get = |k: &str| {
                kv.iter()
                    .find(|(a, _)| a == k)
                    .map(|(_, v)| v.trim_matches('"').to_owned())
                    .unwrap_or_else(|| panic!("a [[chip]] row without {k}"))
            };
            rows.insert((
                get("set"),
                get("name"),
                get("region"),
                get("offset").parse().expect("an offset"),
                get("size").parse().expect("a size"),
                get("sha1"),
            ));
        }
    };
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("[[") || line.starts_with('[') {
            flush(&mut cur);
            if line == "[[chip]]" {
                cur = Some(Vec::new());
            }
        } else if let (Some(kv), Some((k, v))) = (cur.as_mut(), line.split_once(" = ")) {
            kv.push((k.to_owned(), v.to_owned()));
        }
    }
    flush(&mut cur);
    rows
}

fn region(r: Region) -> &'static str {
    match r {
        Region::MainCpu => "main",
        Region::SsioCpu => "ssio",
        Region::Gfx1 => "gfx1",
        Region::Gfx2 => "gfx2",
    }
}

#[test]
fn the_hook_guards_every_chip_of_every_set() {
    let mut want: BTreeSet<Row> = SETS
        .iter()
        .flat_map(|s| {
            s.chips.iter().map(|c| {
                (
                    s.name.to_owned(),
                    c.name.to_owned(),
                    region(c.region).to_owned(),
                    c.offset,
                    c.size,
                    c.sha1.to_owned(),
                )
            })
        })
        .collect();
    want.insert((
        "ssio".into(),
        SSIO_PROM.name.into(),
        "ssio-prom".into(),
        SSIO_PROM.offset,
        SSIO_PROM.size,
        SSIO_PROM.sha1.into(),
    ));
    let have = guarded();
    let missing: Vec<_> = want.difference(&have).collect();
    let extra: Vec<_> = have.difference(&want).collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        ".githooks/guarded-roms.toml and romset.rs differ.\nmissing from the hook's list: {missing:#?}\nnot in romset.rs: {extra:#?}"
    );
}
