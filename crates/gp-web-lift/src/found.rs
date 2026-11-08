//! Any dump of the game, lifted (docs/web.md, "The ROM, in the browser"):
//! each of the game's templates is compared with the ROM by its asset
//! map's content ([`gp_assets::find`]), the model is the one its program
//! matches, the assets found load from the ROM, and the rest are filled
//! from the paper version's own (the fallback folder: Rivet Rodeo's
//! lifted files and model), by the slot each fills. One the paper version
//! has no slot for is left blank: its entry in the template's map, lifted
//! from zero bytes.

use std::collections::BTreeMap;
use std::path::Path;

use gp_assets::find::{self, Match, Report};
use gp_assets::{Asset, AssetMap, MAP_FILE};

/// Where the files of one asset map come from in a merged lift.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AssetLoad {
    pub name: String,
    /// `same`, `changed` (both from the ROM), `filled` (from the paper
    /// version), or `blank` (in neither: zero bytes of its shape).
    pub from: &'static str,
    /// For a filled asset, the paper version's asset it was filled with.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub with: Option<String>,
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

/// Offsets of filled assets are moved past any NES ROM (8MB on, under the
/// map's 16MB limit for a span's end), so they claim no
/// byte a found asset claims in the merged map.
const FILLED_BASE: i64 = 0x0080_0000;
/// Offsets of blank assets, past the filled ones (12MB on).
const BLANK_BASE: i64 = 0x00C0_0000;

fn shift(v: &mut toml::Value, rename: &BTreeMap<String, String>, base: i64) {
    match v {
        toml::Value::Table(t) => {
            for (k, x) in t.iter_mut() {
                match (k.as_str(), &mut *x) {
                    ("offset" | "hi_offset", toml::Value::Integer(n)) => *n += base,
                    ("asset" | "targets" | "metatiles", toml::Value::String(s)) => {
                        if let Some(r) = rename.get(s.as_str()) {
                            *s = r.clone();
                        }
                    }
                    _ => shift(x, rename, base),
                }
            }
        }
        toml::Value::Array(a) => a.iter_mut().for_each(|x| shift(x, rename, base)),
        _ => {}
    }
}

/// The lifted folder `lifted` with the assets of `map`, and those the
/// model's slots take that `map` hasn't, it lacks (those not in `found`)
/// filled from the fallback's lifted folder, by slot, and
/// its `assets.toml` written for the mix. `found` were lifted there from
/// the ROM already. An asset no slot of the model takes is left out.
///
/// The paper version is derived from a template and may lag it: a slot
/// the template gained since has no asset there. Such an asset is left
/// blank, lifted from zero bytes by its entry in the first of `shapes`
/// (asset map texts: the template's own, and for a ROM loaded only to
/// play, the map of the template it plays) that has it. Returns each
/// asset it filled, with `filled` or `blank`.
pub fn fill(
    model_text: &str,
    map_text: &str,
    map: &AssetMap,
    found: &[String],
    fallback: &Path,
    lifted: &Path,
    shapes: &[&str],
) -> Result<Vec<(String, &'static str)>, String> {
    let read = |p: &Path| std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()));
    let ours = slot_assets(model_text)?;
    let theirs = slot_assets(&read(&fallback.join(gp_model::MODEL_FILE))?)?;
    let fb_file = |name: &str| fallback_file(fallback, name);
    let fb_text = String::from_utf8(fb_file(MAP_FILE)?).map_err(|e| e.to_string())?;
    let fb: toml::Table =
        toml::from_str(&fb_text).map_err(|e| format!("fallback {MAP_FILE}: {e}"))?;
    let fb_map = AssetMap::parse(&fb_text).map_err(|e| format!("fallback {MAP_FILE}: {e}"))?;
    let fb_assets = fb
        .get("asset")
        .and_then(|a| a.as_array())
        .cloned()
        .unwrap_or_default();

    // Each missing asset a slot takes, and the fallback's asset for it.
    let mut rename: BTreeMap<String, String> = BTreeMap::new();
    let mut filled = Vec::new();
    // The entries of the assets left blank, as their map has them.
    let mut blank: Vec<toml::Value> = Vec::new();
    // The map's assets, then any the model's slots take that the map hasn't
    // (a ROM loaded only to play, whose map lacks some of the model's).
    let mut names: Vec<&str> = map.assets.iter().map(|a| a.name()).collect();
    for a in ours.values() {
        if !names.contains(&a.as_str()) {
            names.push(a.as_str());
        }
    }
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
        if let Some(g) = theirs.get(slot) {
            rename.insert(g.clone(), name.to_owned());
            filled.push((name.to_owned(), "filled"));
            continue;
        }
        let entry = shape_of(shapes, name)?.ok_or_else(|| {
            format!(
                "{name} wasn't found, the paper version has no slot {slot} to fill it from, \
                 and no map gives its shape to leave it blank"
            )
        })?;
        blank.push(entry);
        filled.push((name.to_owned(), "blank"));
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
        let name = a
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or_default()
            .to_owned();
        if found.contains(&name) {
            assets.push(a);
        }
    }
    for (g, name) in &rename {
        let mut v = fb_assets
            .iter()
            .find(|x| x.get("name").and_then(|n| n.as_str()) == Some(g.as_str()))
            .cloned()
            .ok_or_else(|| format!("the paper version's map has no asset {g}"))?;
        shift(&mut v, &rename, FILLED_BASE);
        if let Some(tab) = v.as_table_mut() {
            tab.insert("name".into(), toml::Value::String(name.clone()));
        }
        assets.push(v);
    }
    for v in &blank {
        let mut v = v.clone();
        shift(&mut v, &BTreeMap::new(), BLANK_BASE);
        assets.push(v);
    }
    t.insert("asset".into(), toml::Value::Array(assets));
    let merged = toml::to_string(&t).map_err(|e| e.to_string())?;
    let merged_map = AssetMap::parse(&merged).map_err(|e| format!("the merged map: {e}"))?;

    // The fallback's files, renamed.
    for (g, name) in &rename {
        let (Some(from), Some(to)) = (fb_map.asset(g), merged_map.asset(name)) else {
            continue;
        };
        for (src, dst) in from.files().iter().zip(to.files()) {
            let bytes = fb_file(src)?;
            let bytes = if src.ends_with(".toml") {
                let mut s = String::from_utf8(bytes).map_err(|e| format!("{src}: {e}"))?;
                for (a, b) in &rename {
                    s = s
                        .replace(&format!("asset = \"{a}\""), &format!("asset = \"{b}\""))
                        .replace(&format!("data = \"{a}\""), &format!("data = \"{b}\""));
                }
                s.into_bytes()
            } else {
                bytes
            };
            crate::write(&lifted.join(&dst), &bytes)?;
        }
    }
    if !blank.is_empty() {
        t.insert("asset".into(), toml::Value::Array(blank));
        let text = toml::to_string(&t).map_err(|e| e.to_string())?;
        let sub = AssetMap::parse(&text).map_err(|e| format!("the blank assets' map: {e}"))?;
        let (files, _) = gp_assets::lift_any(&zero_image(&sub), &sub, &text)
            .map_err(|e| format!("the blank assets: {e}"))?;
        for f in files.iter().filter(|f| f.name != MAP_FILE) {
            crate::write(&lifted.join(&f.name), &f.bytes)?;
        }
    }
    crate::write(&lifted.join(MAP_FILE), merged.as_bytes())?;
    Ok(filled)
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

/// The file of the paper version's lifted folder: the one in `fallback`,
/// else the text the paper version's bundle (`fallback/bundle.json`, the
/// page's own `game/bundle.json`) holds for it, so the page ships each
/// file once.
fn fallback_file(fallback: &Path, name: &str) -> Result<Vec<u8>, String> {
    if let Ok(b) = std::fs::read(fallback.join(name)) {
        return Ok(b);
    }
    let bundle = fallback.join("bundle.json");
    let text =
        std::fs::read_to_string(&bundle).map_err(|e| format!("the paper version's {name}: {e}"))?;
    let v: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", bundle.display()))?;
    v.get("assets")
        .and_then(|a| a.get("files"))
        .and_then(|f| f.get(name))
        .and_then(|t| t.as_str())
        .map(|t| t.as_bytes().to_vec())
        .ok_or_else(|| format!("the paper version has no {name}"))
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

    #[test]
    fn a_missing_asset_is_filled_from_the_fallback_by_its_slot() {
        let dir = tempfile::tempdir().unwrap();
        let fb = dir.path().join("fallback");
        let lifted = dir.path().join("lifted");
        std::fs::create_dir_all(&fb).unwrap();
        // The fallback's model takes slot `ladders` from its asset
        // `ladders`; ours takes it from `ladders_25m`.
        std::fs::write(
            fb.join("model.toml"),
            "[assets.ladders.from]\nasset = \"ladders\"\n",
        )
        .unwrap();
        let rows = "\n[[asset]]\nname = \"ladders\"\nformat = \"byte-rows\"\nregion = \"prg\"\noffset = 0x10\ncount = 1\nwidth = 3\n";
        std::fs::write(fb.join("assets.toml"), map("paper", rows)).unwrap();
        std::fs::write(
            fb.join("ladders.toml"),
            "schema = 1\nasset = \"ladders\"\nrows = [\n  [0x01, 0x02, 0x03],\n]\n",
        )
        .unwrap();
        let ours = "[assets.ladders.from]\nasset = \"ladders_25m\"\n[assets.palettes.from]\nasset = \"pals\"\n";
        let text = map(
            "game",
            "\n[[asset]]\nname = \"ladders_25m\"\nformat = \"byte-rows\"\nregion = \"prg\"\noffset = 0x10\ncount = 2\nwidth = 3\n\n[[asset]]\nname = \"pals\"\nformat = \"palette\"\nregion = \"prg\"\noffset = 0x20\ncount = 1\n",
        );
        let m = AssetMap::parse(&text).unwrap();
        let filled = fill(ours, &text, &m, &["pals".into()], &fb, &lifted, &[]).unwrap();
        assert_eq!(filled, [("ladders_25m".to_owned(), "filled")]);
        let merged =
            AssetMap::parse(&std::fs::read_to_string(lifted.join(MAP_FILE)).unwrap()).unwrap();
        // The found asset keeps its place; the filled one is the
        // fallback's, renamed, past any ROM.
        assert_eq!(merged.assets.len(), 2);
        let spans = merged.asset("ladders_25m").unwrap().spans();
        assert_eq!((spans[0].offset, spans[0].len), (0x0080_0010, 3));
        let file = std::fs::read_to_string(lifted.join("ladders_25m.toml")).unwrap();
        assert!(file.contains("asset = \"ladders_25m\""), "{file}");
        // A slot the fallback has no asset for can't be filled, and with
        // no map to give its shape it can't be left blank either.
        let e = fill(ours, &text, &m, &[], &fb, &lifted, &[]).unwrap_err();
        assert!(e.contains("no slot palettes"), "{e}");
    }

    #[test]
    fn an_asset_the_paper_version_has_no_slot_for_is_left_blank() {
        let dir = tempfile::tempdir().unwrap();
        let fb = dir.path().join("fallback");
        let lifted = dir.path().join("lifted");
        std::fs::create_dir_all(&fb).unwrap();
        // The paper version, derived before the model took `pals` and `code`.
        std::fs::write(
            fb.join("model.toml"),
            "[assets.ladders.from]\nasset = \"ladders\"\n",
        )
        .unwrap();
        let rows = "\n[[asset]]\nname = \"ladders\"\nformat = \"byte-rows\"\nregion = \"prg\"\noffset = 0x10\ncount = 1\nwidth = 3\n";
        std::fs::write(fb.join("assets.toml"), map("paper", rows)).unwrap();
        // A ROM loaded to play: its map has `ladders`, the model also takes
        // `pals` and `code`, which only the played template's map has.
        let ours = "[assets.ladders.from]\nasset = \"ladders\"\n[assets.palettes.from]\nasset = \"pals\"\n[assets.code.from]\nasset = \"code\"\n";
        let text = map(
            "rom",
            "\n[[asset]]\nname = \"ladders\"\nformat = \"byte-rows\"\nregion = \"prg\"\noffset = 0x10\ncount = 1\nwidth = 3\n",
        );
        let played = map(
            "game",
            "\n[[asset]]\nname = \"pals\"\nformat = \"palette\"\nregion = \"prg\"\noffset = 0x20\ncount = 1\n\n[[asset]]\nname = \"code\"\nformat = \"table\"\ncount = 4\ncolumn = [{ name = \"byte\", region = \"prg\", offset = 0x5000 }]\n",
        );
        let m = AssetMap::parse(&text).unwrap();
        let filled = fill(
            ours,
            &text,
            &m,
            &["ladders".into()],
            &fb,
            &lifted,
            &[&text, &played],
        )
        .unwrap();
        assert_eq!(
            filled,
            [("code".to_owned(), "blank"), ("pals".to_owned(), "blank")]
        );
        let merged =
            AssetMap::parse(&std::fs::read_to_string(lifted.join(MAP_FILE)).unwrap()).unwrap();
        assert_eq!(merged.assets.len(), 3);
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
}
