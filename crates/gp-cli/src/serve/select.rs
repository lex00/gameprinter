//! Box selection for the workspace (docs/inspector.md, "Box selection"):
//! what's under a rectangle of a recording's picture at one frame.
//!
//! Sprites come from OAM. Entities come from the state mapping of a model
//! of the recording's game (docs/formats/state-mapping.md): the mapping
//! says which RAM bytes hold each entity's model fields, and the model
//! gives each kind's hitbox. [`MappedFrame`] reads a frame through the
//! mapping and answers two runtime protocol operations on it, `query` for
//! a model path and `entities_at` for a world-space box, the way a model
//! runtime answers them, so a selection on the original names the same
//! model paths a selection on the model would.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use gp_interp::mapping::{Field, Mapping, show};
use gp_model::Fixed;
use gp_runtime::{Rect, Value};
use serde_json::{Value as Json, json};

use super::labels::Loaded;
use super::{Ctx, Fail, Params, Route};
use crate::mechanic::http::{Request, Response};
use crate::serve::pointing::parse_rect;

pub(super) fn routes() -> Vec<Route> {
    vec![Route {
        path: "/api/recordings/{id}/select",
        about: "`?frame=N&rect=x,y,w,h`: the sprites and mapped entities under a rectangle of the picture, with their model paths and RAM",
        handler: select,
    }]
}

/// A model's hitbox for each entity kind: x, y, w, h from the entity's
/// position.
pub(crate) type Hitboxes = BTreeMap<String, [Fixed; 4]>;

/// One mapped entity instance, `entities.<kind>[<index>]`, and its fields.
#[derive(Debug)]
pub(crate) struct Instance<'m> {
    pub path: String,
    pub kind: String,
    pub fields: Vec<&'m Field>,
}

/// A frame's RAM read through a state mapping.
pub(crate) struct MappedFrame<'a> {
    pub mapping: &'a Mapping,
    pub hitboxes: &'a Hitboxes,
    pub ram: &'a [u8],
}

impl<'a> MappedFrame<'a> {
    /// The value at a mapped model path, as the protocol's `query` gives
    /// it for one path. `None` for a path the mapping doesn't hold.
    pub fn query(&self, path: &str) -> Option<Value> {
        let f = self.mapping.fields.iter().find(|f| f.path == path)?;
        Some(self.read(f))
    }

    fn read(&self, f: &Field) -> Value {
        f.read_ram(self.ram)
    }

    /// Every entity instance the mapping holds a field of, in the order
    /// their first field appears.
    pub fn instances(&self) -> Vec<Instance<'a>> {
        let mut out: Vec<Instance<'a>> = Vec::new();
        for f in &self.mapping.fields {
            let Some((inst, kind)) = instance_of(&f.path) else {
                continue;
            };
            match out.iter_mut().find(|i| i.path == inst) {
                Some(i) => i.fields.push(f),
                None => out.push(Instance {
                    path: inst,
                    kind,
                    fields: vec![f],
                }),
            }
        }
        out
    }

    /// A number at a path, in 65536ths of a pixel.
    fn raw(&self, path: &str) -> Option<i64> {
        match self.query(path)? {
            Value::Fixed(raw) => Some(raw),
            Value::Int(n) => Some(n << 16),
            _ => None,
        }
    }

    /// Whole pixels of a number at a path.
    fn pixels(&self, path: &str) -> Option<i64> {
        self.raw(path).map(|r| r >> 16)
    }

    /// The first mapped camera's position: its name and x and y, each
    /// `None` when not mapped.
    pub fn camera(&self) -> Option<(String, Option<i64>, Option<i64>)> {
        let name = self.mapping.fields.iter().find_map(|f| {
            let rest = f.path.strip_prefix("cameras.")?;
            let (name, field) = rest.rsplit_once('.')?;
            (field == "x" || field == "y").then(|| name.to_owned())
        })?;
        let x = self.pixels(&format!("cameras.{name}.x"));
        let y = self.pixels(&format!("cameras.{name}.y"));
        Some((name, x, y))
    }

    /// An instance's hitbox in world space, as x, y, w, h, rounded as the
    /// interpreter's `entities_at` rounds it: the position plus the offset,
    /// then down to a whole pixel. The y and h are `None` when the mapping
    /// has no `y` for it, and the box is `None` when it has no `x` or the
    /// model no hitbox for its kind.
    pub fn hitbox(&self, inst: &Instance) -> Option<(i64, Option<i64>, i64, Option<i64>)> {
        let [hx, hy, hw, hh] = self.hitboxes.get(&inst.kind)?;
        let whole = |f: Fixed| f.raw() >> 16;
        let x = (self.raw(&format!("{}.x", inst.path))? + hx.raw()) >> 16;
        let y = self
            .raw(&format!("{}.y", inst.path))
            .map(|y| (y + hy.raw()) >> 16);
        Some((x, y, whole(*hw), y.map(|_| whole(*hh))))
    }

    /// Whether the mapping says the instance is dead.
    fn dead(&self, inst: &Instance) -> bool {
        self.query(&format!("{}.alive", inst.path)) == Some(Value::Flag(false))
    }

    /// The protocol's `entities_at`: living instances whose hitbox
    /// overlaps the world-space area, by their index in [`Self::instances`].
    /// An instance with no mapped `y` is matched on x alone.
    pub fn entities_at(&self, area: Rect) -> Vec<usize> {
        let (ax1, ay1) = (
            area.x.saturating_add(area.w as i64),
            area.y.saturating_add(area.h as i64),
        );
        self.instances()
            .iter()
            .enumerate()
            .filter(|(_, i)| !self.dead(i))
            .filter(|(_, i)| {
                let Some((x, y, w, h)) = self.hitbox(i) else {
                    return false;
                };
                let across = x < ax1 && area.x < x + w;
                let down = match (y, h) {
                    (Some(y), Some(h)) => y < ay1 && area.y < y + h,
                    _ => true,
                };
                across && down
            })
            .map(|(n, _)| n)
            .collect()
    }
}

/// `entities.slug[1].x` -> (`entities.slug[1]`, `slug`).
fn instance_of(path: &str) -> Option<(String, String)> {
    let rest = path.strip_prefix("entities.")?;
    let open = rest.find('[')?;
    let close = rest[open..].find(']')? + open;
    let kind = &rest[..open];
    rest[open + 1..close].parse::<u32>().ok()?;
    Some((format!("entities.{}", &rest[..=close]), kind.to_owned()))
}

/// The model directory in `root` (the games checkout, or the repo's
/// `templates/`) whose state mapping is for `game`: its mapping, the
/// model's hitboxes and the mapping's file.
pub(crate) fn mapping_for(
    root: &Path,
    game: &str,
) -> Result<Option<(Mapping, Hitboxes, PathBuf)>, String> {
    let Ok(dirs) = std::fs::read_dir(root) else {
        return Ok(None);
    };
    let mut dirs: Vec<PathBuf> = dirs.flatten().map(|d| d.path()).collect();
    dirs.sort();
    for dir in dirs {
        let file = dir.join("mapping.toml");
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        // Another game's mapping that doesn't parse isn't this one's
        // problem; one that names this game and doesn't parse is.
        let mapping = match Mapping::parse(&text) {
            Ok(m) if m.source == game => m,
            Ok(_) => continue,
            Err(e) if text.contains(&format!("source = \"{game}\"")) => {
                return Err(format!("{}: {e}", file.display()));
            }
            Err(_) => continue,
        };
        let hitboxes = hitboxes_of(&dir)?;
        return Ok(Some((mapping, hitboxes, file)));
    }
    Ok(None)
}

/// The state mapping for `game` among the games checkout's games, else the
/// repo's templates (Moss Hop's).
pub(crate) fn mapping_in(
    ctx: &Ctx,
    game: &str,
) -> Result<Option<(Mapping, Hitboxes, PathBuf)>, String> {
    match mapping_for(&ctx.games, game)? {
        None => mapping_for(&ctx.repo.join("templates"), game),
        found => Ok(found),
    }
}

/// The folders models are in: each game of the games checkout and each of
/// the repo's templates.
pub(crate) fn model_dirs(ctx: &Ctx) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for root in [ctx.games.clone(), ctx.repo.join("templates")] {
        let mut dirs: Vec<PathBuf> = std::fs::read_dir(root)
            .into_iter()
            .flatten()
            .flatten()
            .map(|d| d.path())
            .collect();
        dirs.sort();
        out.extend(dirs);
    }
    out
}

/// A model run's state at `frame` as model path entries, from its trace
/// (game-printer's model_trace.rs). gameprinter records no model runs yet
/// (`gp record --model` isn't here), so this answers for a store that
/// holds one from elsewhere.
pub(crate) fn model_entries(
    opened: &crate::open::Opened,
    frame: u64,
) -> anyhow::Result<Vec<gp_runtime::Entry>> {
    let Some(state) = gp_nes::recording::trace_state(&opened.store, &opened.manifest, frame)?
    else {
        anyhow::bail!(
            "{} has no frame trace (it was recorded before model runs kept one); record it again with `gp record --model`",
            opened.short()
        );
    };
    let table = gp_nes::recording::field_table(&opened.store, &opened.manifest)?;
    gp_interp::modeltrace::entries(&table, &state).map_err(anyhow::Error::msg)
}

/// Each entity kind's hitbox in the model of `dir`.
fn hitboxes_of(dir: &Path) -> Result<Hitboxes, String> {
    let model_file = dir.join("model.toml");
    let text = std::fs::read_to_string(&model_file)
        .map_err(|e| format!("reading {}: {e}", model_file.display()))?;
    let model =
        gp_model::Model::parse(&text).map_err(|e| format!("{}: {e}", model_file.display()))?;
    Ok(model
        .entities
        .iter()
        .map(|(k, e)| {
            let h = &e.core.hitbox;
            (k.clone(), [h.x, h.y, h.w, h.h])
        })
        .collect())
}

/// The state mapping and hitboxes of the model in `dir` (a template or a
/// capture run's working copy), or `None` when it has no mapping.
pub(crate) fn mapping_for_dir(dir: &Path) -> Result<Option<(Mapping, Hitboxes)>, String> {
    let file = dir.join("mapping.toml");
    let Ok(text) = std::fs::read_to_string(&file) else {
        return Ok(None);
    };
    let mapping = Mapping::parse(&text).map_err(|e| format!("{}: {e}", file.display()))?;
    Ok(Some((mapping, hitboxes_of(dir)?)))
}

/// The sprites in OAM whose box overlaps the screen rectangle, as JSON.
fn sprites_under(oam: &[u8], tall: bool, r: Rect) -> Vec<Json> {
    let h = if tall { 16 } else { 8 };
    let (rx1, ry1) = (r.x + r.w as i64, r.y + r.h as i64);
    oam.as_chunks::<4>()
        .0
        .iter()
        .take(64)
        .enumerate()
        .filter(|(_, s)| s[0] < 0xef)
        .filter_map(|(i, s)| {
            // A sprite is drawn a line below its Y.
            let (x, y) = (i64::from(s[3]), i64::from(s[0]) + 1);
            (x < rx1 && r.x < x + 8 && y < ry1 && r.y < y + h).then(|| {
                json!({
                    "index": i,
                    "oam": i * 4,
                    "box": { "x": x, "y": y, "w": 8, "h": h },
                    "tile": s[1],
                    "attr": s[2],
                })
            })
        })
        .collect()
}

/// What's under `rect` (screen space) in this frame, as the endpoint's JSON
/// less the recording's ID and frame.
pub(crate) fn selection(
    ram: &[u8],
    oam: &[u8],
    tall: bool,
    rect: Rect,
    mapped: Option<(&Mapping, &Hitboxes)>,
    labels: &Loaded,
) -> Json {
    let sprites = sprites_under(oam, tall, rect);
    let Some((mapping, hitboxes)) = mapped else {
        return json!({
            "rect": rect_json(rect),
            "camera": null,
            "world": null,
            "sprites": sprites,
            "entities": [],
            "ram": [],
        });
    };
    let frame = MappedFrame {
        mapping,
        hitboxes,
        ram,
    };
    let camera = frame.camera();
    let (cx, cy) = camera
        .as_ref()
        .map_or((0, 0), |(_, x, y)| (x.unwrap_or(0), y.unwrap_or(0)));
    let world = Rect {
        x: rect.x + cx,
        y: rect.y + cy,
        ..rect
    };
    let instances = frame.instances();
    let mut ram_used: BTreeMap<u16, Vec<&str>> = BTreeMap::new();
    let entities: Vec<Json> = frame
        .entities_at(world)
        .into_iter()
        .map(|n| {
            let inst = &instances[n];
            let (x, y, w, h) = frame.hitbox(inst).expect("entities_at found its box");
            let fields: Vec<Json> = inst
                .fields
                .iter()
                .map(|f| {
                    let addrs: Vec<u16> = f.ram.iter().chain(&f.frac).copied().collect();
                    for a in &addrs {
                        ram_used.entry(*a).or_default().push(&f.path);
                    }
                    json!({
                        "path": f.path,
                        "value": show(&frame.read(f)),
                        "ram": addrs,
                        "note": f.note,
                    })
                })
                .collect();
            json!({
                "id": n,
                "path": inst.path,
                "kind": inst.kind,
                "box": {
                    "x": x - cx,
                    "y": y.map(|y| y - cy),
                    "w": w,
                    "h": h,
                },
                "fields": fields,
            })
        })
        .collect();
    let ram_json: Vec<Json> = ram_used
        .iter()
        .map(|(a, paths)| {
            json!({
                "addr": a,
                "value": ram.get(usize::from(*a)).copied().unwrap_or(0),
                "labels": labels.at(*a),
                "paths": paths,
            })
        })
        .collect();
    json!({
        "rect": rect_json(rect),
        "camera": camera.map(|(name, x, y)| json!({ "path": format!("cameras.{name}"), "x": x, "y": y })),
        "world": rect_json(world),
        "sprites": sprites,
        "entities": entities,
        "ram": ram_json,
    })
}

fn rect_json(r: Rect) -> Json {
    json!({ "x": r.x, "y": r.y, "w": r.w, "h": r.h })
}

fn select(ctx: &Ctx, req: &Request, params: &Params) -> Result<Response, Fail> {
    let frame_text = req.param("frame").unwrap_or("");
    let frame: u64 = frame_text.parse().map_err(|_| {
        Fail::bad_request(format!("frame must be a whole number, not {frame_text:?}"))
    })?;
    let rect = parse_rect(
        req.param("rect")
            .ok_or_else(|| Fail::bad_request("give the rectangle: ?rect=x,y,w,h"))?,
    )
    .map_err(Fail::bad_request)?;
    let opened = super::recordings::open(ctx, params.get("id").map_or("", String::as_str))?;
    let (state, from) = opened.state(frame, None, &ctx.paths)?;
    let table = crate::recordings::field_table_of(&opened.store, &opened.manifest)?;
    let fields: BTreeMap<&str, &[u8]> = table
        .split(&state)
        .map_err(anyhow::Error::from)?
        .into_iter()
        .collect();
    let get = |name: &str| fields.get(name).copied().unwrap_or(&[]);
    let tall = get("ppu.ctrl").first().is_some_and(|c| c & 0x20 != 0);
    let labels = super::labels::load(ctx, opened.sha256())?;
    let found = match &labels.game {
        None => None,
        Some(game) => Some((game, mapping_in(ctx, game).map_err(|e| anyhow::anyhow!(e))?)),
    };
    let (mapped, mapping_file, missing) = match found {
        None => (
            None,
            None,
            Some("the recording's game isn't known".to_owned()),
        ),
        Some((_, Some((m, h, file)))) => (Some((m, h)), Some(file), None),
        Some((game, None)) => (
            None,
            None,
            Some(format!(
                "no game in the games checkout has a state mapping for {game}"
            )),
        ),
    };
    let mut out = selection(
        get("ram"),
        get("oam"),
        tall,
        rect,
        mapped.as_ref().map(|(m, h)| (m, h)),
        &labels,
    );
    let o = out.as_object_mut().expect("an object");
    o.insert("id".into(), json!(opened.entry.id));
    o.insert("frame".into(), json!(frame));
    o.insert("from".into(), json!(from));
    o.insert("game".into(), json!(labels.game));
    o.insert("mapping".into(), json!(mapping_file));
    o.insert("missing".into(), json!(missing));
    Ok(Response::json(&out))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A made-up game: a hero at RAM $10 (x, two bytes) and $12 (y), two
    /// blobs with x at $20/$21 and alive flags at $26/$27 but no y, and a
    /// camera at $30.
    const MAPPING: &str = r#"
schema = 1
model = "made-up"
source = "made-up"
align = "polled"

[[field]]
path = "entities.hero[0].x"
ram = [0x10, 0x11]

[[field]]
path = "entities.hero[0].y"
ram = [0x12]
frac = 0x13

[[field]]
path = "entities.blob[0].x"
ram = [0x20]

[[field]]
path = "entities.blob[1].x"
ram = [0x21]

[[field]]
path = "entities.blob[0].alive"
ram = [0x26]
flag = true

[[field]]
path = "entities.blob[1].alive"
ram = [0x27]
flag = true

[[field]]
path = "cameras.main.x"
ram = [0x30, 0x31]

[[field]]
path = "counters.score"
ram = [0x40]
"#;

    fn setup() -> (Mapping, Hitboxes, Vec<u8>) {
        let m = Mapping::parse(MAPPING).unwrap();
        let fx = |n: i64| Fixed::from_int(n).unwrap();
        let mut h = Hitboxes::new();
        h.insert("hero".into(), [fx(2), fx(0), fx(12), fx(16)]);
        h.insert("blob".into(), [fx(0), fx(4), fx(16), fx(12)]);
        let mut ram = vec![0u8; 2048];
        // Hero at world (0x110 = 272, 100.5), camera at 256.
        ram[0x10] = 0x10;
        ram[0x11] = 0x01;
        ram[0x12] = 100;
        ram[0x13] = 0x80;
        ram[0x30] = 0x00;
        ram[0x31] = 0x01;
        // Blob 0 at x 40, alive; blob 1 at x 44, dead.
        ram[0x20] = 40;
        ram[0x21] = 44;
        ram[0x26] = 1;
        ram[0x27] = 0;
        (m, h, ram)
    }

    fn labels() -> Loaded {
        Loaded {
            game: Some("made-up".into()),
            file: None,
            missing: None,
            labels: vec![crate::labels::Label {
                name: "HERO_X".into(),
                addr: 0x10,
                size: 2,
                description: String::new(),
            }],
        }
    }

    #[test]
    fn instances_group_fields_by_entity() {
        let (m, h, ram) = setup();
        let f = MappedFrame {
            mapping: &m,
            hitboxes: &h,
            ram: &ram,
        };
        let names: Vec<_> = f.instances().iter().map(|i| i.path.clone()).collect();
        assert_eq!(
            names,
            ["entities.hero[0]", "entities.blob[0]", "entities.blob[1]"]
        );
        assert_eq!(f.query("entities.hero[0].x"), Some(Value::Fixed(272 << 16)));
        assert_eq!(
            f.query("entities.hero[0].y"),
            Some(Value::Fixed((100 << 16) + (0x80 << 8)))
        );
        assert_eq!(f.query("nothing"), None);
        assert_eq!(f.camera(), Some(("main".into(), Some(256), None)));
    }

    #[test]
    fn entities_at_matches_hitboxes_in_world_space() {
        let (m, h, ram) = setup();
        let f = MappedFrame {
            mapping: &m,
            hitboxes: &h,
            ram: &ram,
        };
        let at = |x, y, w, h| f.entities_at(Rect { x, y, w, h });
        // The hero's hitbox is x 274..286, y 100..116.
        assert_eq!(at(280, 110, 1, 1), [0]);
        assert_eq!(at(286, 110, 4, 4), Vec::<usize>::new());
        assert_eq!(at(280, 116, 4, 4), Vec::<usize>::new());
        // A blob has no mapped y, so any height matches; the dead one never.
        assert_eq!(at(40, 0, 1, 1), [1]);
        assert_eq!(at(44, 200, 20, 1), [1]);
        assert_eq!(at(57, 0, 4, 1), Vec::<usize>::new());
    }

    #[test]
    fn a_selection_names_paths_ram_and_labels_in_screen_space() {
        let (m, h, ram) = setup();
        let mut oam = vec![0xffu8; 256];
        oam[..4].copy_from_slice(&[99, 7, 1, 20]); // sprite 0 at 20,100
        oam[4..8].copy_from_slice(&[10, 8, 0, 200]); // sprite 1 elsewhere
        // The hero is at screen x 272 - 256 + 2 = 18.
        let rect = parse_rect("16,104,8,4").unwrap();
        let out = selection(&ram, &oam, false, rect, Some((&m, &h)), &labels());
        assert_eq!(out["world"], json!({ "x": 272, "y": 104, "w": 8, "h": 4 }));
        assert_eq!(out["camera"]["x"], 256);
        let e = &out["entities"][0];
        assert_eq!(e["path"], "entities.hero[0]");
        assert_eq!(e["box"], json!({ "x": 18, "y": 100, "w": 12, "h": 16 }));
        assert_eq!(e["fields"][1]["value"], "100.5");
        assert_eq!(out["entities"].as_array().unwrap().len(), 1);
        assert_eq!(out["sprites"].as_array().unwrap().len(), 1);
        assert_eq!(out["sprites"][0]["tile"], 7);
        let addrs: Vec<u64> = out["ram"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["addr"].as_u64().unwrap())
            .collect();
        assert_eq!(addrs, [0x10, 0x11, 0x12, 0x13]);
        // Fractions add before rounding, as in the interpreter: 100.5 +
        // 0.5 is 101.
        let mut half = h.clone();
        half.insert(
            "hero".into(),
            [
                Fixed::ZERO,
                Fixed::parse("0.5").unwrap(),
                Fixed::from_int(12).unwrap(),
                Fixed::from_int(16).unwrap(),
            ],
        );
        let f = MappedFrame {
            mapping: &m,
            hitboxes: &half,
            ram: &ram,
        };
        assert_eq!(
            f.hitbox(&f.instances()[0]),
            Some((272, Some(101), 12, Some(16)))
        );
        assert_eq!(out["ram"][0]["labels"], json!(["HERO_X"]));
        assert_eq!(out["ram"][1]["labels"], json!(["HERO_X"]));
        assert_eq!(out["ram"][2]["paths"], json!(["entities.hero[0].y"]));
        // Without a mapping, only sprites.
        let bare = selection(&ram, &oam, true, rect, None, &labels());
        assert!(bare["entities"].as_array().unwrap().is_empty());
        assert_eq!(bare["sprites"][0]["box"]["h"], 16);
    }

    #[test]
    fn rects_are_checked_and_clipped() {
        assert_eq!(
            parse_rect("250,230,20,20").unwrap(),
            Rect {
                x: 250,
                y: 230,
                w: 6,
                h: 10
            }
        );
        for bad in [
            "",
            "1,2,3",
            "1,2,3,4,5",
            "-1,0,1,1",
            "0,240,1,1",
            "0,0,0,1",
            "a,b,c,d",
        ] {
            assert!(parse_rect(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_committed_mosshop_mapping_is_found() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let (m, h, file) = mapping_for(&repo.join("templates"), "mosshop")
            .unwrap()
            .unwrap();
        assert_eq!(m.source, "mosshop");
        assert!(h.contains_key("player") && h.contains_key("slug"));
        assert!(file.ends_with("templates/mosshop/mapping.toml"));
        assert!(
            mapping_for(&repo.join("templates"), "no-such-game")
                .unwrap()
                .is_none()
        );
    }
}
