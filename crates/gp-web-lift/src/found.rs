//! Any dump of the game, lifted (docs/web.md, "The ROM, in the browser"):
//! each of the game's templates is compared with the ROM by its asset
//! map's content ([`gp_assets::find`]), the model is the one its program
//! matches and the assets found load from the ROM. An asset of the model's
//! the ROM hasn't is left blank only when the ROM's entry says its game
//! does without the level or part that uses it (`without` in load.json,
//! [`crate::Without`]): its entry in the template's map, lifted from zero
//! bytes. Any other asset the ROM lacks stops the lift.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use gp_assets::find::{self, Match, Report};
use gp_assets::{Asset, AssetMap, MAP_FILE};

/// Where the files of one asset map come from in a merged lift.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AssetLoad {
    pub name: String,
    /// `same`, `changed` (both from the ROM), or `blank` (not in the ROM:
    /// zero bytes of its shape).
    pub from: &'static str,
    /// For a blank asset, what the ROM's game does without that uses it,
    /// in words (`Without::what`: "50m").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub without: Option<String>,
}

/// One template's files, read from its folder.
pub struct Template {
    pub id: String,
    pub dir: std::path::PathBuf,
    pub map_text: String,
    pub map: AssetMap,
}

/// What [`pick`] chose.
pub struct Picked<'a> {
    pub template: &'a Template,
    pub report: Report,
    pub matched: Match,
    pub share: f64,
    pub reports: Vec<(String, Report)>,
}

/// Compares the ROM with each template and picks one ([`find::choose`]).
pub fn pick<'a>(rom: &[u8], templates: &'a [Template]) -> Result<Picked<'a>, String> {
    let reports: Vec<Report> = templates
        .iter()
        .map(|t| find::find(rom, &t.map).map_err(|e| format!("{}: {e}", t.id)))
        .collect::<Result<_, _>>()?;
    let refs: Vec<&Report> = reports.iter().collect();
    let (i, matched) = find::choose(&refs).ok_or("no templates to compare with")?;
    let share = reports[i].code.as_ref().map_or(0.0, |c| c.share());
    let report = reports[i].clone();
    Ok(Picked {
        template: &templates[i],
        report,
        matched,
        share,
        reports: templates
            .iter()
            .map(|t| t.id.clone())
            .zip(reports)
            .collect(),
    })
}

/// The asset each slot of a model takes (`assets.<slot>.from.asset`), by
/// slot.
fn slot_assets(model_text: &str) -> Result<BTreeMap<String, String>, String> {
    let t: toml::Table = toml::from_str(model_text).map_err(|e| format!("model.toml: {e}"))?;
    let mut out = BTreeMap::new();
    if let Some(assets) = t.get("assets").and_then(|a| a.as_table()) {
        for (slot, v) in assets {
            if let Some(a) = v
                .get("from")
                .and_then(|f| f.get("asset"))
                .and_then(|a| a.as_str())
            {
                out.insert(slot.clone(), a.to_owned());
            }
        }
    }
    Ok(out)
}

/// Offsets of blank assets are moved past any NES ROM (12MB on, under the
/// map's 16MB limit for a span's end), so they claim no byte a found asset
/// claims in the merged map.
const BLANK_BASE: i64 = 0x00C0_0000;

fn shift(v: &mut toml::Value, base: i64) {
    match v {
        toml::Value::Table(t) => {
            for (k, x) in t.iter_mut() {
                match (k.as_str(), &mut *x) {
                    ("offset" | "hi_offset", toml::Value::Integer(n)) => *n += base,
                    _ => shift(x, base),
                }
            }
        }
        toml::Value::Array(a) => a.iter_mut().for_each(|x| shift(x, base)),
        _ => {}
    }
}

/// The lifted folder `lifted` with the assets of `map` the ROM has
/// (`found`, lifted there from the ROM already), and its `assets.toml`
/// written for them and the blank ones. An asset no slot of the model takes
/// is left out.
///
/// An asset a slot takes that isn't in `found` is left blank when `blank`
/// names it: lifted from zero bytes by its entry in the first of `shapes`
/// (asset map texts: the template's own, and for a ROM loaded only to play,
/// the map of the template it plays) that has it. Any other missing asset
/// is an error, and so is an asset `blank` names that the ROM has or no
/// slot takes. Returns each asset left blank.
pub fn fill(
    model_text: &str,
    map_text: &str,
    map: &AssetMap,
    found: &[String],
    lifted: &Path,
    shapes: &[&str],
    blank: &BTreeSet<String>,
) -> Result<Vec<String>, String> {
    let ours = slot_assets(model_text)?;
    for name in blank {
        if found.contains(name) {
            return Err(format!(
                "{name} is in this ROM, but the ROM's entry says its game does without it"
            ));
        }
        if !ours.values().any(|a| a == name) {
            return Err(format!(
                "the ROM's entry leaves {name} blank, but no slot of the model takes it"
            ));
        }
    }
    // The map's assets, then any the model's slots take that the map hasn't
    // (a ROM loaded only to play, whose map lacks some of the model's).
    let mut names: Vec<&str> = map.assets.iter().map(|a| a.name()).collect();
    for a in ours.values() {
        if !names.contains(&a.as_str()) {
            names.push(a.as_str());
        }
    }
    // The entries of the assets left blank, as their map has them.
    let mut entries: Vec<toml::Value> = Vec::new();
    let mut left = Vec::new();
    for name in names {
        if found.iter().any(|f| f == name) {
            continue;
        }
        let Some(slot) = ours
            .iter()
            .find(|(_, x)| x.as_str() == name)
            .map(|(s, _)| s)
        else {
            continue;
        };
        if !blank.contains(name) {
            return Err(format!(
                "{name} (slot {slot}) isn't in this ROM, and the ROM's entry doesn't say its game does without it"
            ));
        }
        let entry = shape_of(shapes, name)?.ok_or_else(|| {
            format!("{name} isn't in this ROM, and no map gives its shape to leave it blank")
        })?;
        entries.push(blank_shape(entry));
        left.push(name.to_owned());
    }

    let mut t: toml::Table = toml::from_str(map_text).map_err(|e| format!("{MAP_FILE}: {e}"))?;
    t.remove("content");
    t.remove("code");
    let mut assets = Vec::new();
    for a in t
        .get("asset")
        .and_then(|a| a.as_array())
        .cloned()
        .unwrap_or_default()
    {
        let name = a.get("name").and_then(|n| n.as_str()).unwrap_or_default();
        if found.iter().any(|f| f == name) {
            assets.push(a);
        }
    }
    for v in &entries {
        let mut v = v.clone();
        shift(&mut v, BLANK_BASE);
        assets.push(v);
    }
    t.insert("asset".into(), toml::Value::Array(assets));
    let merged = toml::to_string(&t).map_err(|e| e.to_string())?;
    AssetMap::parse(&merged).map_err(|e| format!("the merged map: {e}"))?;

    if !entries.is_empty() {
        t.insert("asset".into(), toml::Value::Array(entries));
        let text = toml::to_string(&t).map_err(|e| e.to_string())?;
        let sub = AssetMap::parse(&text).map_err(|e| format!("the blank assets' map: {e}"))?;
        let (files, _) = gp_assets::lift_any(&zero_image(&sub), &sub, &text)
            .map_err(|e| format!("the blank assets: {e}"))?;
        for f in files.iter().filter(|f| f.name != MAP_FILE) {
            crate::write(&lifted.join(&f.name), &f.bytes)?;
        }
    }
    crate::write(&lifted.join(MAP_FILE), merged.as_bytes())?;
    Ok(left)
}

/// The entry for asset `name` in the first of the map texts `shapes` that
/// has one.
fn shape_of(shapes: &[&str], name: &str) -> Result<Option<toml::Value>, String> {
    for text in shapes {
        let t: toml::Table = toml::from_str(text).map_err(|e| format!("{MAP_FILE}: {e}"))?;
        let entry = t.get("asset").and_then(|a| a.as_array()).and_then(|a| {
            a.iter()
                .find(|x| x.get("name").and_then(|n| n.as_str()) == Some(name))
        });
        if let Some(e) = entry {
            return Ok(Some(e.clone()));
        }
    }
    Ok(None)
}

/// An asset map entry as it is left blank. Zero bytes are a blank of most
/// formats as they are; a `ppu-strings` asset's zeros would be one empty
/// string and then bytes the strings don't reach, so a blank one is an
/// empty string for each entry (a byte each, its end byte 0), with no
/// embedded assets.
fn blank_shape(mut entry: toml::Value) -> toml::Value {
    let Some(t) = entry.as_table_mut() else {
        return entry;
    };
    let strings = t.get("format").and_then(|f| f.as_str()) == Some("ppu-strings")
        && t.get("end").and_then(|e| e.as_integer()).unwrap_or(0) == 0;
    if strings {
        let mut n = 0;
        if let Some(es) = t.get_mut("entry").and_then(|e| e.as_array_mut()) {
            for e in es.iter_mut() {
                if let Some(e) = e.as_table_mut() {
                    e.remove("records");
                }
            }
            n = es.len() as i64;
        }
        t.insert("size".into(), toml::Value::Integer(n.max(1)));
        t.remove("embed");
    }
    entry
}

/// An NES image of zero bytes with room for every span of `map`.
fn zero_image(map: &AssetMap) -> Vec<u8> {
    let end = |r: gp_assets::Region| {
        map.spans()
            .iter()
            .filter(|s| s.region == r)
            .map(|s| s.offset as usize + s.len as usize)
            .max()
            .unwrap_or(0)
    };
    let prg = end(gp_assets::Region::Prg).div_ceil(0x4000).max(1);
    let chr = end(gp_assets::Region::Chr).div_ceil(0x2000);
    let mut image = b"NES\x1a".to_vec();
    image.push(prg as u8);
    image.push(chr as u8);
    image.resize(16 + prg * 0x4000 + chr * 0x2000, 0);
    image
}

/// Lifts what was found of `map` in `rom` into `lifted` (with the map's
/// own text as assets.toml), and says which assets loaded.
pub fn lift_found(
    rom: &[u8],
    map: &AssetMap,
    map_text: &str,
    report: &Report,
    lifted: &Path,
) -> Result<Vec<String>, String> {
    let (image, names) = find::assemble(rom, map, report).map_err(|e| e.to_string())?;
    let sub = AssetMap {
        assets: map
            .assets
            .iter()
            .filter(|a| names.iter().any(|n| n == a.name()))
            .cloned()
            .collect::<Vec<Asset>>(),
        content: Vec::new(),
        code: None,
        ..map.clone()
    };
    let (files, _) = gp_assets::lift_any(&image, &sub, map_text).map_err(|e| e.to_string())?;
    for f in &files {
        crate::write(&lifted.join(&f.name), &f.bytes)?;
    }
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "00000000000000000000000000000000000000000000000000000000000000aa";

    fn map(source: &str, assets: &str) -> String {
        format!("schema = 1\nsource = \"{source}\"\nsha256 = \"{SHA}\"\n{assets}")
    }

    /// A ROM loaded to play: its map has `ladders`; the model also takes
    /// `pals` and `code`, which only the played template's map has.
    const OURS: &str = "[assets.ladders.from]\nasset = \"ladders\"\n[assets.palettes.from]\nasset = \"pals\"\n[assets.code.from]\nasset = \"code\"\n";
    const ROWS: &str = "\n[[asset]]\nname = \"ladders\"\nformat = \"byte-rows\"\nregion = \"prg\"\noffset = 0x10\ncount = 1\nwidth = 3\n";
    const PLAYED: &str = "\n[[asset]]\nname = \"pals\"\nformat = \"palette\"\nregion = \"prg\"\noffset = 0x20\ncount = 1\n\n[[asset]]\nname = \"code\"\nformat = \"table\"\ncount = 4\ncolumn = [{ name = \"byte\", region = \"prg\", offset = 0x5000 }]\n";

    fn names(n: &[&str]) -> BTreeSet<String> {
        n.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn an_asset_the_roms_game_does_without_is_left_blank() {
        let dir = tempfile::tempdir().unwrap();
        let lifted = dir.path().join("lifted");
        let text = map("rom", ROWS);
        let played = map("game", PLAYED);
        let m = AssetMap::parse(&text).unwrap();
        let left = fill(
            OURS,
            &text,
            &m,
            &["ladders".into()],
            &lifted,
            &[&text, &played],
            &names(&["pals", "code"]),
        )
        .unwrap();
        assert_eq!(left, ["code", "pals"]);
        let merged =
            AssetMap::parse(&std::fs::read_to_string(lifted.join(MAP_FILE)).unwrap()).unwrap();
        assert_eq!(merged.assets.len(), 3);
        // The found asset keeps its place; a blank one is past any ROM.
        let spans = merged.asset("ladders").unwrap().spans();
        assert_eq!((spans[0].offset, spans[0].len), (0x10, 3));
        let spans = merged.asset("code").unwrap().spans();
        assert_eq!((spans[0].offset, spans[0].len), (0x00C0_5000, 4));
        for a in ["pals", "code"] {
            for f in merged.asset(a).unwrap().files() {
                assert!(lifted.join(&f).is_file(), "{f}");
            }
        }
        let code = std::fs::read_to_string(lifted.join("code.toml")).unwrap();
        assert!(!code.contains("0x01"), "{code}");
    }

    #[test]
    fn a_missing_asset_nothing_does_without_stops_the_lift() {
        let dir = tempfile::tempdir().unwrap();
        let lifted = dir.path().join("lifted");
        let text = map("rom", ROWS);
        let played = map("game", PLAYED);
        let m = AssetMap::parse(&text).unwrap();
        let shapes = [text.as_str(), played.as_str()];
        // `code` is missing too, and the entry names only `pals`.
        let e = fill(
            OURS,
            &text,
            &m,
            &["ladders".into()],
            &lifted,
            &shapes,
            &names(&["pals"]),
        )
        .unwrap_err();
        assert!(e.contains("code (slot code) isn't in this ROM"), "{e}");
        assert!(!lifted.join(MAP_FILE).exists());
        // Nothing to leave blank: every missing asset stops it.
        let e = fill(
            OURS,
            &text,
            &m,
            &["ladders".into()],
            &lifted,
            &shapes,
            &BTreeSet::new(),
        )
        .unwrap_err();
        assert!(e.contains("isn't in this ROM"), "{e}");
        // An entry that leaves blank an asset the ROM has, or one no slot
        // takes, is a mistake in the entry.
        let e = fill(
            OURS,
            &text,
            &m,
            &["ladders".into()],
            &lifted,
            &shapes,
            &names(&["pals", "code", "ladders"]),
        )
        .unwrap_err();
        assert!(e.contains("ladders is in this ROM"), "{e}");
        let e = fill(
            OURS,
            &text,
            &m,
            &["ladders".into()],
            &lifted,
            &shapes,
            &names(&["pals", "code", "tiles"]),
        )
        .unwrap_err();
        assert!(e.contains("no slot of the model takes it"), "{e}");
        // A missing asset with no map to give its shape can't be blank.
        let e = fill(
            OURS,
            &text,
            &m,
            &["ladders".into()],
            &lifted,
            &[&text],
            &names(&["pals", "code"]),
        )
        .unwrap_err();
        assert!(e.contains("no map gives its shape"), "{e}");
    }
}
