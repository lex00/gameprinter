//! The package scan on made-up ROMs and packages: no game's bytes.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use sha2::{Digest, Sha256};

use super::rom_match::{self, Kind, RomIndex};
use super::{Bucket, Package, Source, fields, roms, scan, transcribe};

/// A made-up NROM image: 16 KB of PRG and 8 KB of CHR of xorshift noise,
/// with a routine at PRG 0x0100 ($8100) and two tables.
fn made_up_rom() -> Vec<u8> {
    let mut x: u32 = 0x1234_5678;
    let mut data: Vec<u8> = (0..16384 + 8192)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (x >> 24) as u8
        })
        .collect();
    // LDA $5E; CMP #$10; BCC +4; LDA #$20; STA $68; LDA $0233; AND #$80;
    // BEQ +2; INC $8A; LDA #$FF; STA $72; CMP #$30; RTS
    let routine = [
        0xA5, 0x5E, 0xC9, 0x10, 0x90, 0x04, 0xA9, 0x20, 0x85, 0x68, 0xAD, 0x33, 0x02, 0x29, 0x80,
        0xF0, 0x02, 0xE6, 0x8A, 0xA9, 0xFF, 0x85, 0x72, 0xC9, 0x30, 0x60,
    ];
    data[0x100..0x100 + routine.len()].copy_from_slice(&routine);
    data[0x800..0x800 + TABLE.len()].copy_from_slice(&TABLE);
    // A pointer table: $8123, $8456, $879A, $8BCD.
    data[0x880..0x888].copy_from_slice(&[0x23, 0x81, 0x56, 0x84, 0x9A, 0x87, 0xCD, 0x8B]);
    let mut rom = b"NES\x1a\x01\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00".to_vec();
    rom.extend(data);
    rom
}

const TABLE: [u8; 12] = [11, 12, 13, 21, 22, 23, 24, 25, 26, 30, 31, 156];

fn data(rom: &[u8]) -> &[u8] {
    &rom[16..]
}

fn matches(text: &str, rom: &[u8]) -> Vec<rom_match::Match> {
    let nums = rom_match::numbers(text);
    let lexed = fields::lex(text);
    let groups = super::groups_of(&nums, &lexed.groups);
    let idx = RomIndex::new("rom", data(rom), 16384);
    rom_match::scan_text(&nums, &groups, &idx, (nums.len() * 3).max(1) as f64)
}

#[test]
fn numbers_reads_literals_not_words_or_floats() {
    let n: Vec<i64> = rom_match::numbers(
        "a = 0x1F, b = $20, c = -3, d = 7u8, speed = \"0.333\", bounce_right1 = 9, v = 1.5, 0b101",
    )
    .iter()
    .map(|n| n.value)
    .collect();
    assert_eq!(n, vec![0x1F, 0x20, -3, 7, 9, 5]);
}

#[test]
fn a_table_split_over_fields_is_found_exactly() {
    let rom = made_up_rom();
    let mut text = String::from("[entities.barrel]\nspeed = 2\n");
    for (k, v) in TABLE.iter().enumerate() {
        text.push_str(&format!("bounce{} = {v}\n", k + 1));
    }
    let m = matches(&text, &rom);
    assert_eq!(m.len(), 1, "{m:?}");
    assert_eq!(m[0].kind, Kind::Exact);
    assert_eq!(m[0].rom_offset, 0x800);
    assert_eq!(m[0].len, 12);
}

#[test]
fn a_short_run_counts_and_small_facts_do_not() {
    let rom = made_up_rom();
    // Six values of the table in order: rare enough by the ROM's own
    // frequencies to count.
    let m = matches("t = [21, 22, 23, 24, 25, 26]\n", &rom);
    assert!(m.is_empty(), "a ramp of one step is too regular: {m:?}");
    let m = matches("t = [12, 13, 21, 22, 23]\n", &rom);
    assert_eq!(m.len(), 1, "{m:?}");
    // Facts: sizes, a speed, a cycle of 0s and 1s.
    let m = matches(
        "size = [16, 16]\nspeed = 2\ncycle = [1, 0, 0, 1, 0, 0, 1]\nhitbox = { x = 2, y = 10, w = 14, h = 4 }\n",
        &rom,
    );
    assert!(m.is_empty(), "{m:?}");
}

#[test]
fn a_reordered_table_is_found() {
    let rom = made_up_rom();
    let m = matches(
        "t = [31, 11, 25, 13, 156, 22, 12, 30, 21, 26, 23, 24]\n",
        &rom,
    );
    assert!(
        m.iter()
            .any(|m| m.kind == Kind::Reordered && m.rom_offset == 0x800),
        "{m:?}"
    );
}

#[test]
fn a_scaled_table_is_found() {
    let rom = made_up_rom();
    let vals: Vec<String> = TABLE
        .iter()
        .map(|v| (i64::from(*v) * 4 + 3).to_string())
        .collect();
    let m = matches(&format!("t = [{}]\n", vals.join(", ")), &rom);
    assert_eq!(m.len(), 1, "{m:?}");
    assert_eq!(m[0].kind, Kind::Scaled);
    assert_eq!((m[0].scale, m[0].offset, m[0].rom_offset), (4, 3, 0x800));
}

#[test]
fn a_pointer_table_is_found_as_words() {
    let rom = made_up_rom();
    let m = matches("ptrs = [0x8123, 0x8456, 0x879A, 0x8BCD]\n", &rom);
    assert_eq!(m.len(), 1, "{m:?}");
    assert_eq!((m[0].stream, m[0].rom_offset), ("words", 0x880));
}

#[test]
fn fields_have_paths_lines_and_groups() {
    let l = fields::lex(
        "[a]\nx = 1\ny = { p = 2, q = [3, 4] }\n[[b]]\nz = \"s # not a comment\"\n[[b]]\nz = 5\n",
    );
    let got: Vec<(&str, usize)> = l.fields.iter().map(|f| (f.path.as_str(), f.line)).collect();
    assert_eq!(
        got,
        vec![
            ("a.x", 2),
            ("a.y.p", 3),
            ("a.y.q", 3),
            ("b[0].z", 5),
            ("b[1].z", 7)
        ]
    );
}

const HOOKS: &str = r#"
/// $8100's body, branch by branch.
fn r8100(ram: &mut [u8; 2048]) {
    if ram[0x5E] >= 0x10 {
        ram[0x68] = 0x20;
    }
    if ram[0x233] & 0x80 != 0 {
        ram[0x8A] = ram[0x8A].wrapping_add(1);
    }
    ram[0x72] = 0xFF;
    let _ = 0x30;
}

/// What $8100 does: a slot past its threshold turns.
fn turn(p: &Params, s: &mut Slot) {
    if s.x >= p.threshold {
        s.turned = true;
    }
}

/// $8100 from its JSR.
fn cost(a: u8) -> i64 {
    let mut c = 6;
    c += 3;
    if a < 16 {
        c += 2;
    } else {
        c += 7;
    }
    c += 4;
    c
}
"#;

#[test]
fn a_transcription_is_found_and_a_description_is_not() {
    let rom = made_up_rom();
    let image = gp_nes::codemap::cpu_image(&data(&rom)[..16384]);
    let items = transcribe::items(HOOKS);
    let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, vec!["r8100", "turn", "cost"]);
    let v: Vec<Vec<&str>> = items
        .iter()
        .map(|i| transcribe::judge(HOOKS, i, &image).map_or(vec![], |v| v.signals))
        .collect();
    assert_eq!(v, vec![vec!["same-order"], vec![], vec!["cycle-count"]]);
}

/// An index stepped by 1 isn't a cycle count: a table walk with `i += 1`
/// four times and one stride is a description, and the same walk adding
/// instruction times is a transcription.
#[test]
fn an_index_step_is_not_a_cycle_count() {
    let src = "/// $8100: the girder by y.\nfn girder(t: &[u8], y: u8) -> u8 {\n    let mut i = 0;\n    let mut k = 1;\n    while t[i] > y {\n        i += 1;\n        k += 1;\n    }\n    i += 1;\n    k += 1;\n    i += 3;\n    k\n}\n\n/// $8100: the same, counted.\nfn girder_cost(t: &[u8], y: u8) -> u8 {\n    let mut c = 6;\n    let mut i = 0;\n    while t[i] > y {\n        c += 7;\n        i += 1;\n    }\n    c += 2;\n    c += 3;\n    c += 6;\n    c\n}\n";
    let items = transcribe::items(src);
    let adds: Vec<usize> = items
        .iter()
        .map(|i| transcribe::fn_operands(src, i).1)
        .collect();
    assert_eq!(adds, vec![1, 4]);
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// A made-up package: a model with the table typed out when `table_inline`,
/// a hook that transcribes the routine, and an asset map lifting other bytes.
fn made_up_package(root: &Path, pin: Option<&str>, table_inline: bool) -> std::path::PathBuf {
    let rom = made_up_rom();
    fs::create_dir_all(root.join("roms")).unwrap();
    fs::write(root.join("roms/made-up.nes"), &rom).unwrap();
    let sha = hex::encode(Sha256::digest(data(&rom)));
    let dir = root.join("package");
    let map = format!(
        "schema = 1\nsource = \"made-up\"\ntitle = \"Made Up\"\nsha256 = \"{sha}\"\n\n[[asset]]\nname = \"rows\"\nformat = \"byte-rows\"\nregion = \"prg\"\noffset = 0x0900\ncount = 4\nwidth = 3\n"
    );
    write(&dir.join("assets/made-up.toml"), &map);
    let map_sha = hex::encode(Sha256::digest(map.as_bytes()));
    write(
        &dir.join("package.toml"),
        &format!(
            "[package]\nname = \"test/made-up\"\nversion = \"1.0.0\"\nkind = \"game\"\ntier = \"commercial\"\n\n[[rom.entry]]\nid = \"made-up\"\nrole = \"scored\"\nsha256 = \"{sha}\"\nasset_map = \"assets/made-up.toml\"\nasset_map_sha256 = \"{map_sha}\"\n{}",
            pin.map(|p| format!("lifted_sha256 = \"{p}\"\n"))
                .unwrap_or_default()
        ),
    );
    write(
        &dir.join("roms.toml"),
        &format!("[[source]]\nid = \"made-up\"\nsha256 = \"{sha}\"\n"),
    );
    let mut model = String::from(
        "[model]\nname = \"made_up\"\n\n[assets.rows]\nkind = \"table\"\n\n[assets.rows.from]\nsource = \"made-up\"\nasset = \"rows\"\n\n[entities.barrel]\nspeed = 2\nsize = [16, 16]\nroutine = \"$8100\"\n",
    );
    if table_inline {
        model.push_str("ext_params = { ");
        let parts: Vec<String> = TABLE
            .iter()
            .enumerate()
            .map(|(k, v)| format!("bounce{} = {v}", k + 1))
            .collect();
        model.push_str(&parts.join(", "));
        model.push_str(" }\n");
    }
    write(&dir.join("model.toml"), &model);
    write(
        &dir.join("mapping.toml"),
        "schema = 22\n[[field]]\npath = \"entities.player[0].x\"\nram = [0x203]\n",
    );
    if table_inline {
        write(&dir.join("ext/src/lib.rs"), HOOKS);
    }
    write(&dir.join("clips/a.presses"), "0 a\n60 right\n");
    dir
}

fn found(root: &Path, pkg: &Package) -> BTreeMap<String, roms::Rom> {
    roms::find(&pkg.rom_hashes(), &[], &root.join("roms")).unwrap()
}

#[test]
fn the_scan_buckets_lists_and_rederives() {
    let d = tempfile::tempdir().unwrap();
    let dir = made_up_package(d.path(), None, true);
    let pkg = Package::load(&dir).unwrap();
    let r = scan(&pkg, &found(d.path(), &pkg)).unwrap();
    let p = &r.parts[0];
    let c = super::counts(p);
    // The table (one inline table's twelve fields), the transcribing hook
    // and the counting hook.
    assert_eq!(c[Bucket::Rewrite as usize], 12 + 2, "{:?}", p.rewrite);
    assert!(
        p.rewrite
            .iter()
            .any(|x| x.at.starts_with("entities.barrel.ext_params.bounce1"))
    );
    assert!(p.rewrite.iter().any(|x| x.at == "fn r8100"));
    // The mapping row and the routine field are references.
    assert!(c[Bucket::Reference as usize] >= 3, "{c:?}");
    // The asset map's one recipe and the model's field naming it.
    assert_eq!(c[Bucket::FromRom as usize], 2, "{c:?}");
    // The rebuild works but has no pin to check against.
    let (_, d2) = &p.rederive[0];
    assert_eq!(d2.roundtrip_differences, 0);
    assert!(d2.pin.is_none());
    // No line of the report holds the table's bytes.
    let out = r.outcome();
    assert!(!out.text.contains("11, 12, 13"));
    assert_eq!(out.exit, 1);
}

#[test]
fn a_clean_pinned_package_is_complete() {
    let d = tempfile::tempdir().unwrap();
    let dir = made_up_package(d.path(), None, false);
    let pkg = Package::load(&dir).unwrap();
    let digest = scan(&pkg, &found(d.path(), &pkg)).unwrap().parts[0].rederive[0]
        .1
        .digest
        .clone();
    let dir = made_up_package(d.path(), Some(&digest), false);
    let pkg = Package::load(&dir).unwrap();
    let r = scan(&pkg, &found(d.path(), &pkg)).unwrap();
    assert!(r.parts[0].rewrite.is_empty(), "{:?}", r.parts[0].rewrite);
    let out = r.outcome();
    assert_eq!(out.exit, 0, "{}", out.text);
    assert!(out.text.contains("complete: yes"), "{}", out.text);
}

#[test]
fn with_no_rom_the_report_says_nothing_was_compared() {
    let d = tempfile::tempdir().unwrap();
    // The typed-out table would be found if a ROM were there to compare.
    let dir = made_up_package(d.path(), None, true);
    let pkg = Package::load(&dir).unwrap();
    let r = scan(&pkg, &BTreeMap::new()).unwrap();
    let out = r.outcome();
    assert_eq!(out.exit, 0, "{}", out.text);
    assert!(out.text.contains("NOT RUN"), "{}", out.text);
    assert!(out.text.contains("compared with no ROM"), "{}", out.text);
    assert!(out.text.contains("complete: not known"), "{}", out.text);
    assert!(!out.text.contains("complete: yes"), "{}", out.text);
    assert_eq!(out.json["rom_compared"], false);
}

#[test]
fn a_disagreement_between_package_and_roms_toml_needs_rewrite() {
    let d = tempfile::tempdir().unwrap();
    let dir = made_up_package(d.path(), None, false);
    write(
        &dir.join("roms.toml"),
        &format!(
            "[[source]]\nid = \"made-up\"\nsha256 = \"{}\"\n",
            "00".repeat(32)
        ),
    );
    let pkg = Package::load(&dir).unwrap();
    let r = scan(&pkg, &BTreeMap::new()).unwrap();
    assert_eq!(r.rewrite_count(), 1);
    assert_eq!(r.outcome().exit, 1);
}

/// A place in a romset is named by MAME's region and the chip, not PRG.
#[test]
fn a_romset_place_names_its_region_and_chip() {
    let set = gp_arcade::romset::set("tapper").unwrap();
    let total: usize = set.chips.iter().map(|c| c.size).sum();
    let s = Source::of_romset("t".into(), "tapper", &vec![0; total], Vec::new());
    let first = &set.chips[0];
    assert_eq!(
        s.place(0x10),
        format!(
            "{} {:#07x} ({} +0x0010)",
            first.region.name(),
            first.offset + 0x10,
            first.name
        )
    );
    let last = set.chips.last().unwrap();
    assert!(
        s.place(total - 1).starts_with(last.region.name()),
        "{}",
        s.place(total - 1)
    );
    assert!(!s.place(total - 1).contains("PRG"));
}

/// With a baseline, the items an earlier scan listed are known and only a
/// new one fails (game-printer's `--baseline`).
#[test]
fn a_baseline_excuses_the_items_it_lists() {
    let d = tempfile::tempdir().unwrap();
    let dir = made_up_package(d.path(), None, true);
    let paths = gp_evidence::Paths {
        home: d.path().join("home"),
        store: d.path().join("home/store"),
        roms: d.path().join("roms"),
    };
    let first = super::run(&dir, &[], &[], &paths).unwrap();
    assert_eq!(first.exit, 1);
    let base = d.path().join("base.json");
    fs::write(&base, first.json.to_string()).unwrap();
    let again = super::run_against(&dir, &[], &[], &base, &paths).unwrap();
    assert_eq!(again.exit, 0, "{}", again.text);
    assert!(
        again
            .text
            .contains("against the baseline: 0 new needs-rewrite items, 3 known"),
        "{}",
        again.text
    );
    // A baseline that lists only some of them: the rest are new.
    let mut v = first.json.clone();
    v["parts"][0]["rewrite"]
        .as_array_mut()
        .unwrap()
        .retain(|r| r["at"] != "fn r8100");
    fs::write(&base, v.to_string()).unwrap();
    let again = super::run_against(&dir, &[], &[], &base, &paths).unwrap();
    assert_eq!(again.exit, 1);
    assert_eq!(again.json["new_rewrite"], 1);
    // game-printer's scan JSON, its items at the top level, reads the same.
    let old = serde_json::json!({"rewrite": first.json["parts"][0]["rewrite"]});
    assert_eq!(
        super::known_keys(&old.to_string()).unwrap().len(),
        super::known_keys(&first.json.to_string()).unwrap().len()
    );
}

/// A clip's press timings may equal a short rising run of ROM bytes by
/// chance; such a match is allowed, a longer one, one on other numbers or
/// one in another kind of file isn't.
#[test]
fn press_timings_have_an_allowance() {
    let text = "gp-presses 0\nframes 335\narcade tapper CSRLDUT\ntap@9-41\ndown@51-58\ntap@68-100\nstart1@110-117\n";
    let x = super::Text {
        rel: "inputs/a.presses".into(),
        role: super::Role::Whole("clip recipe"),
        text: text.into(),
        nums: rom_match::numbers(text),
        groups: Vec::new(),
        lexed: None,
    };
    let timings = super::press_timings(&x);
    // 335, then 9 41 51 58 68 100 110 117: the 1 of start1 isn't one.
    let values: Vec<i64> = timings.iter().map(|&i| x.nums[i].value).collect();
    assert_eq!(values, [335, 9, 41, 51, 58, 68, 100, 110, 117]);
    let m = |first: usize, last: usize| rom_match::Match {
        kind: Kind::Exact,
        stream: "bytes",
        rom: "rom".into(),
        rom_offset: 0,
        len: last + 1 - first,
        first,
        last,
        scale: 1,
        offset: 0,
        ln_expected: -10.0,
    };
    let first_span = x.nums.iter().position(|n| n.value == 9).unwrap();
    assert!(super::press_allowance(
        &m(first_span, first_span + 6),
        &x,
        &timings
    ));
    // The frame count before them doesn't rise into them.
    assert!(!super::press_allowance(
        &m(first_span - 1, first_span + 5),
        &x,
        &timings
    ));
    let other = super::Text {
        rel: "notes.txt".into(),
        role: super::Role::Whole("doc"),
        ..x
    };
    assert!(super::press_timings(&other).is_empty());
}
