//! Reading a model's levels as [`LevelGrid`]s, from its lifted assets.
//!
//! The collision class of every cell and the entities placed come from the
//! compiled model (`gp_interp::compile`), the same for every game. What a
//! cell is in the format, and the objects that drew it, come from the
//! level's layout slot by its asset kind, not by game:
//!
//! | Asset kind | Cell value | Objects |
//! |---|---|---|
//! | `level` grid with a `metatiles` tileset | the grid's metatile number | none |
//! | `ppu-strings` | the tile its string draws on the first name table | each update of the string: a run of tiles across or down |
//!
//! game-printer read Super Mario Bros.', Super Mario Bros. 2's and Mega Man
//! 2's layouts too; gameprinter cut other games' code, so those readers
//! are gone. The PPU-strings reader is new here: Donkey Kong's stages are
//! drawn that way, and game-printer refused them.

use std::collections::BTreeMap;
use std::path::Path;

use gp_assets::map::{Asset, AssetMap};
use gp_interp::Lifted;
use gp_model::Model;
use gp_sim::Spec;
use gp_sim::spec::Role;

use crate::level::{LevelGrid, Object, Placed};

/// The levels of a model, decoded, with the compiled spec.
pub struct Loaded {
    pub spec: Spec,
    pub grids: Vec<LevelGrid>,
    pub notes: Vec<String>,
}

/// Reads every level of `model` (or the ones `only` names) from the lifted
/// directory `dir`. Levels sharing one layout are read once.
pub fn load(model: &Model, dir: &Path, only: &[String]) -> Result<Loaded, String> {
    let lifted = Lifted::read_dir(dir, model)?;
    let spec = gp_interp::compile::compile(model, &lifted)?;
    let map = lifted.map()?;
    let read = |file: &str| std::fs::read(dir.join(file));
    let mut grids: Vec<LevelGrid> = Vec::new();
    let mut by_layout: BTreeMap<String, usize> = BTreeMap::new();
    let mut notes = Vec::new();
    for (i, sl) in spec.levels.iter().enumerate() {
        if !only.is_empty() && !only.contains(&sl.name) {
            continue;
        }
        let Some(l) = model.levels.get(&sl.name) else {
            continue;
        };
        let c = &l.core;
        let layout = match (&c.layout, &c.grid) {
            (Some(e), _) => format!("{}/{}", e.slot, e.entry),
            (None, Some(g)) => g.clone(),
            _ => {
                notes.push(format!(
                    "{}: the model's level names no layout or grid slot, so there are no cells to read",
                    sl.name
                ));
                continue;
            }
        };
        if let Some(&g) = by_layout.get(&layout) {
            grids[g].levels.push(sl.name.clone());
            continue;
        }
        let mut grid = LevelGrid {
            name: sl.name.clone(),
            levels: vec![sl.name.clone()],
            layout: layout.clone(),
            width: sl.size[0] as usize,
            height: sl.size[1] as usize,
            tile_size: sl.tile_size,
            class: sl.tiles.clone(),
            pits: sl.kill_below.is_some(),
            spec_level: Some(i),
            ..LevelGrid::default()
        };
        grid.spawns = sl
            .spawns
            .iter()
            .filter(|s| spec.kinds[s.kind].role != Role::Player)
            .map(|s| Placed {
                entity: spec.kinds[s.kind].name.clone(),
                x: s.x.floor(),
                y: s.y.floor(),
            })
            .collect();
        let asset_of = |slot: &str| -> Result<&Asset, String> {
            let from = model
                .assets
                .get(slot)
                .and_then(|s| s.from.as_ref())
                .ok_or(format!("{}: slot `{slot}` has no `from`", sl.name))?;
            map.asset(&from.asset)
                .ok_or(format!("{}: no asset `{}`", sl.name, from.asset))
        };
        let result = match (&c.layout, &c.grid) {
            (Some(e), _) => match asset_of(&e.slot)? {
                Asset::PpuStrings(p) => ppu_strings(&mut grid, &lifted, p, &e.entry),
                a => Err(format!(
                    "a layout from a {} asset isn't read yet",
                    a.format()
                )),
            },
            (None, Some(g)) => {
                let tileset = c.tileset.as_deref().map(asset_of).transpose()?;
                grid_level(&mut grid, &map, &lifted, &read, asset_of(g)?, tileset)
            }
            _ => continue,
        };
        if let Err(e) = result {
            notes.push(format!("{}: {e}", sl.name));
            continue;
        }
        if grid.ids.len() != grid.width * grid.height {
            notes.push(format!(
                "{}: the layout has {} cells, the model's level {} by {}",
                sl.name,
                grid.ids.len(),
                grid.width,
                grid.height
            ));
            continue;
        }
        by_layout.insert(layout, grids.len());
        grids.push(grid);
    }
    Ok(Loaded { spec, grids, notes })
}

type Read<'a> = &'a dyn Fn(&str) -> std::io::Result<Vec<u8>>;

/// The `[[metatile]]` tile numbers of a lifted metatiles file, by rows.
fn metatile_tiles(read: Read, file: &str) -> Result<Vec<[u8; 4]>, String> {
    let text = read(file).map_err(|e| format!("{file}: {e}"))?;
    let t: toml::Table =
        toml::from_str(&String::from_utf8_lossy(&text)).map_err(|e| format!("{file}: {e}"))?;
    let list = t.get("metatile").and_then(|v| v.as_array());
    Ok(list
        .into_iter()
        .flatten()
        .map(|m| {
            let mut out = [0u8; 4];
            if let Some(ts) = m.get("tiles").and_then(|v| v.as_array()) {
                for (o, v) in out.iter_mut().zip(ts) {
                    *o = v.as_integer().unwrap_or(0) as u8;
                }
            }
            out
        })
        .collect())
}

/// The names a metatiles asset gives its entries.
fn metatile_names(map: &AssetMap, name: &str) -> Vec<String> {
    match map.asset(name) {
        Some(Asset::Metatiles { names, .. }) => names.clone(),
        _ => Vec::new(),
    }
}

fn grid_level(
    grid: &mut LevelGrid,
    map: &AssetMap,
    lifted: &Lifted,
    read: Read,
    asset: &Asset,
    tileset: Option<&Asset>,
) -> Result<(), String> {
    let cells = lifted
        .decode(map, asset.name())?
        .remove(asset.name())
        .ok_or(format!("`{}` decoded to nothing", asset.name()))?;
    grid.ids = cells.iter().map(|&b| u32::from(b)).collect();
    if let Some(ts) = tileset {
        let names = metatile_names(map, ts.name());
        let tiles = match ts.files().into_iter().next() {
            Some(f) => metatile_tiles(read, &f)?,
            None => Vec::new(),
        };
        for (i, n) in names.iter().enumerate() {
            grid.names.insert(i as u32, n.clone());
        }
        for (i, t) in tiles.iter().enumerate() {
            grid.tiles.insert(i as u32, *t);
        }
    }
    Ok(())
}

/// A level drawn by a PPU string, as Donkey Kong's stages are: each cell's
/// value is the tile the string writes there on the first name table (0
/// where nothing is drawn), and each of its updates is an object, a run of
/// tiles across (`row`) or down (`column`) from a cell. The level is the
/// name table's 32 by 30 cells.
fn ppu_strings(
    grid: &mut LevelGrid,
    lifted: &Lifted,
    p: &gp_assets::ppu::PpuStrings,
    entry: &str,
) -> Result<(), String> {
    const W: usize = 32;
    const H: usize = 30;
    if grid.width != W || grid.height != H {
        return Err(format!(
            "a level drawn from a PPU string is the name table's {W} by {H} cells, not {} by {}",
            grid.width, grid.height
        ));
    }
    let file = format!("{}.toml", p.name);
    let text = lifted
        .files
        .get(&file)
        .ok_or(format!("{file} wasn't read"))?;
    let mut ids = vec![0u32; W * H];
    let mut objects = Vec::new();
    for w in p.writes(text, entry)? {
        let mut ppu = w.ppu;
        let mut cells = Vec::new();
        for b in &w.bytes {
            if (0x2000..0x23C0).contains(&ppu) {
                let i = usize::from(ppu - 0x2000);
                cells.push((i, u32::from(*b)));
                ids[i] = u32::from(*b);
            }
            ppu = ppu.wrapping_add(if w.vertical { 32 } else { 1 });
        }
        if cells.is_empty() {
            continue;
        }
        let at = usize::from(w.ppu.wrapping_sub(0x2000));
        objects.push(Object {
            kind: if w.vertical { "column" } else { "row" }.into(),
            params: vec![
                ("x".into(), (at % W) as i64),
                ("y".into(), (at / W) as i64),
                ("length".into(), w.bytes.len() as i64),
            ],
            cells,
        });
    }
    for &v in ids.iter().collect::<std::collections::BTreeSet<_>>() {
        grid.names.insert(v, format!("tile ${v:02X}"));
    }
    grid.ids = ids;
    grid.objects = objects;
    Ok(())
}
