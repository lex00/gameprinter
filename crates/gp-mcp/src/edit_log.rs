//! A working copy's art history (docs/sprite-editing.md): every edit
//! `assets_edit` made or proposed, by sequence number, and its status.
//!
//! | File | What it holds |
//! |---|---|
//! | `asset-edits.jsonl` | Placed edits, oldest first: undo takes from here |
//! | `asset-redo.jsonl` | Edits undone, newest last: redo takes from here, and a new edit empties it |
//! | `asset-proposals/<seq>.json` | Edits proposed (`place: false`) and edits rejected, with who decided |
//!
//! An edit is kept as what it changed: for a tiles slot, each 8x8 tile it
//! changed, before and after, as 64 color digits; for a palettes slot, the
//! slot's palettes before and after. So undo, redo and accept can each
//! check that what they would overwrite is what the edit left (or found),
//! and an older edit can be undone when no later one changed its tiles.

use std::fs;
use std::io::Write as _;
use std::path::Path;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::SlotState;
use crate::sprite_ops::Op;

/// Placed edits, one JSON object a line.
pub const HISTORY_FILE: &str = "asset-edits.jsonl";
/// Undone edits, for redo.
pub const REDO_FILE: &str = "asset-redo.jsonl";
/// Proposals and rejected edits, one file each.
pub const PROPOSALS_DIR: &str = "asset-proposals";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum EditStatus {
    /// Made with `place: false`: shown, not written; accept places it.
    Proposed,
    /// Written by the agent; a person may still accept or reject it.
    Placed,
    /// A person (or the agent) kept it.
    Accepted,
    /// A person turned it down: a proposal never placed, or a placed edit
    /// undone.
    Rejected,
    /// Undone; redo puts it back.
    Undone,
}

impl EditStatus {
    pub fn name(self) -> &'static str {
        match self {
            EditStatus::Proposed => "proposed",
            EditStatus::Placed => "placed",
            EditStatus::Accepted => "accepted",
            EditStatus::Rejected => "rejected",
            EditStatus::Undone => "undone",
        }
    }
}

/// One tile an edit changed, as 64 color digits row by row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TileEdit {
    pub tile: u32,
    pub before: String,
    pub after: String,
}

/// A palettes slot before and after an edit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaletteChange {
    pub before: Vec<[u8; 4]>,
    pub after: Vec<[u8; 4]>,
}

/// One palette operation on an entity's palette (docs/sprite-editing.md).
/// Index 0 is the backdrop every sprite palette shares, so only 1 to 3
/// change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum PaletteOp {
    /// Set one color: palette index 1 to 3 to an NES color number 0 to 63.
    Color { index: u8, nes: u8 },
    /// Swap two colors of the palette.
    Swap { a: u8, b: u8 },
    /// Turn the hue of colors 1 to 3 by `by` steps of the NES's 12 hues;
    /// grays stay gray.
    Hue { by: i8 },
    /// Move colors 1 to 3 by `by` of the NES's four brightness rows.
    Brightness { by: i8 },
    /// Take colors 1 to 3 from another entity's palette.
    CopyFrom { entity: String },
}

impl PaletteOp {
    pub fn name(&self) -> &'static str {
        match self {
            PaletteOp::Color { .. } => "palette.color",
            PaletteOp::Swap { .. } => "palette.swap",
            PaletteOp::Hue { .. } => "palette.hue",
            PaletteOp::Brightness { .. } => "palette.brightness",
            PaletteOp::CopyFrom { .. } => "palette.copy_from",
        }
    }
}

fn index_ok(i: u8) -> Result<(), String> {
    if !(1..=3).contains(&i) {
        return Err(format!(
            "palette index {i}: only 1 to 3 change (0 is the shared backdrop)"
        ));
    }
    Ok(())
}

/// Runs one palette op on four NES colors. `other` is the palette a
/// `copy_from` resolved to.
pub fn apply_palette(
    p: [u8; 4],
    op: &PaletteOp,
    other: Option<[u8; 4]>,
) -> Result<[u8; 4], String> {
    let mut out = p;
    match op {
        PaletteOp::Color { index, nes } => {
            index_ok(*index)?;
            if *nes > 0x3F {
                return Err(format!("{nes} is past the last NES color number 63"));
            }
            out[*index as usize] = *nes;
        }
        PaletteOp::Swap { a, b } => {
            index_ok(*a)?;
            index_ok(*b)?;
            out.swap(*a as usize, *b as usize);
        }
        PaletteOp::Hue { by } => {
            for c in &mut out[1..] {
                let hue = *c & 0x0F;
                if (1..=12).contains(&hue) {
                    let h = (i16::from(hue) - 1 + i16::from(*by)).rem_euclid(12) as u8 + 1;
                    *c = (*c & 0x30) | h;
                }
            }
        }
        PaletteOp::Brightness { by } => {
            for c in &mut out[1..] {
                let row = i16::from(*c >> 4) + i16::from(*by);
                let v = ((row.clamp(0, 3) as u8) << 4) | (*c & 0x0F);
                // $0D is "blacker than black", which upsets some TVs.
                *c = if v == 0x0D { 0x0F } else { v };
            }
        }
        PaletteOp::CopyFrom { .. } => {
            let o = other.ok_or("copy_from has no palette")?;
            out[1..].copy_from_slice(&o[1..]);
        }
    }
    Ok(out)
}

/// One edit, placed or proposed, as the history keeps it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub seq: u64,
    pub slot: String,
    pub entity: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ops: Vec<Op>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub palette_ops: Vec<PaletteOp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    /// The first frame edited, before and after, as rows of color indices
    /// (empty for a palette edit).
    #[serde(default)]
    pub before: Vec<String>,
    #[serde(default)]
    pub after: Vec<String>,
    /// The slot's state before the edit, put back by undo of the newest.
    pub state_before: SlotState,
    /// The frames of the sprite set it edited.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub frames: Vec<String>,
    /// Each tile it changed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tiles: Vec<TileEdit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub palettes: Option<PaletteChange>,
    /// Frames whose whole picture was given (`pictures`, an editor's save).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pictures: Vec<String>,
    /// The external editor a save came from; the slot's origin is then
    /// `uploaded`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub editor: Option<String>,
    /// A resize: the sprite's tile list and width before and after.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sprite: Option<crate::resize::SpriteChange>,
    #[serde(default = "placed")]
    pub status: EditStatus,
    /// Who accepted or rejected it: `person` or `agent`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decided_by: Option<String>,
    /// A level tile kind's look instead of an entity's sprite: the kind's
    /// name (`crate::tile_edit`); `entity` is then empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tile: Option<String>,
}

fn placed() -> EditStatus {
    EditStatus::Placed
}

impl Entry {
    /// The edit's name, `<copy>:<seq>`, which assets_history and accept and reject use.
    pub fn item(&self, copy: &str) -> String {
        format!("{copy}:{}", self.seq)
    }

    /// The entity whose sprite it edited; None for a tile kind's look.
    pub fn entity_ref(&self) -> Option<&str> {
        (!self.entity.is_empty()).then_some(self.entity.as_str())
    }

    /// What it changed, in words: the entity, or the tile kind.
    pub fn subject(&self) -> &str {
        self.tile.as_deref().unwrap_or(&self.entity)
    }

    pub fn op_names(&self) -> Vec<&'static str> {
        self.sprite
            .iter()
            .map(|_| "resize")
            .chain(self.pictures.iter().take(1).map(|_| "picture"))
            .chain(self.ops.iter().map(Op::name))
            .chain(self.palette_ops.iter().map(PaletteOp::name))
            .collect()
    }

    /// Whether this edit and `other` changed a tile (or palette) in common.
    pub fn overlaps(&self, other: &Entry) -> bool {
        if self.slot != other.slot {
            return false;
        }
        if let (Some(a), Some(b)) = (&self.palettes, &other.palettes) {
            return (0..a.before.len().min(b.before.len()))
                .any(|i| a.before[i] != a.after[i] && b.before[i] != b.after[i]);
        }
        self.tiles
            .iter()
            .any(|t| other.tiles.iter().any(|o| o.tile == t.tile))
    }

    /// A short line for logs.
    pub fn summary(&self) -> serde_json::Value {
        serde_json::json!({
            "seq": self.seq,
            "status": self.status.name(),
            "entity": self.entity,
            "tile": self.tile,
            "slot": self.slot,
            "frames": self.frames,
            "ops": self.op_names(),
            "tiles": self.tiles.iter().map(|t| t.tile).collect::<Vec<_>>(),
            "palettes": self.palettes.as_ref().map(|p| (0..p.before.len()).filter(|&i| p.before[i] != p.after[i]).collect::<Vec<_>>()),
            "prompt": self.prompt,
            "decided_by": self.decided_by,
        })
    }
}

pub fn read_lines(dir: &Path, file: &str) -> Result<Vec<Entry>, String> {
    let Ok(text) = fs::read_to_string(dir.join(file)) else {
        return Ok(Vec::new());
    };
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .enumerate()
        .map(|(i, l)| serde_json::from_str(l).map_err(|e| format!("{file} line {}: {e}", i + 1)))
        .collect()
}

fn write_atomic(dir: &Path, file: &str, text: &str) -> Result<(), String> {
    let path = dir.join(file);
    if let Some(p) = path.parent() {
        fs::create_dir_all(p).map_err(|e| format!("making {}: {e}", p.display()))?;
    }
    let tmp = path.with_extension("tmp-edit");
    let mut f = fs::File::create(&tmp).map_err(|e| format!("writing {file}: {e}"))?;
    f.write_all(text.as_bytes())
        .map_err(|e| format!("writing {file}: {e}"))?;
    fs::rename(&tmp, &path).map_err(|e| format!("writing {file}: {e}"))
}

pub fn write_lines(dir: &Path, file: &str, entries: &[Entry]) -> Result<(), String> {
    let mut text = String::new();
    for e in entries {
        text.push_str(&serde_json::to_string(e).map_err(|e| e.to_string())?);
        text.push('\n');
    }
    write_atomic(dir, file, &text)
}

pub fn read_history(dir: &Path) -> Result<Vec<Entry>, String> {
    read_lines(dir, HISTORY_FILE)
}

pub fn write_history(dir: &Path, entries: &[Entry]) -> Result<(), String> {
    write_lines(dir, HISTORY_FILE, entries)
}

pub fn read_redo(dir: &Path) -> Result<Vec<Entry>, String> {
    read_lines(dir, REDO_FILE)
}

pub fn write_redo(dir: &Path, entries: &[Entry]) -> Result<(), String> {
    write_lines(dir, REDO_FILE, entries)
}

fn proposal_file(seq: u64) -> String {
    format!("{PROPOSALS_DIR}/{seq}.json")
}

pub fn read_proposal(dir: &Path, seq: u64) -> Result<Option<Entry>, String> {
    let path = dir.join(proposal_file(seq));
    let Ok(text) = fs::read_to_string(&path) else {
        return Ok(None);
    };
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|e| format!("{}: {e}", path.display()))
}

pub fn write_proposal(dir: &Path, e: &Entry) -> Result<(), String> {
    let text = serde_json::to_string_pretty(e).map_err(|e| e.to_string())?;
    write_atomic(dir, &proposal_file(e.seq), &text)
}

pub fn proposals(dir: &Path) -> Result<Vec<Entry>, String> {
    let Ok(rd) = fs::read_dir(dir.join(PROPOSALS_DIR)) else {
        return Ok(Vec::new());
    };
    let mut seqs: Vec<u64> = rd
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().to_str()?.strip_suffix(".json")?.parse().ok())
        .collect();
    seqs.sort_unstable();
    seqs.into_iter()
        .filter_map(|s| read_proposal(dir, s).transpose())
        .collect()
}

/// The next sequence number: one past every edit the copy has seen.
pub fn next_seq(dir: &Path) -> Result<u64, String> {
    let max = read_history(dir)?
        .iter()
        .chain(read_redo(dir)?.iter())
        .chain(proposals(dir)?.iter())
        .map(|e| e.seq)
        .max()
        .unwrap_or(0);
    Ok(max + 1)
}

/// An edit by sequence number, wherever it is, with its status now.
pub fn find(dir: &Path, seq: u64) -> Result<Option<Entry>, String> {
    if let Some(e) = read_history(dir)?.into_iter().find(|e| e.seq == seq) {
        return Ok(Some(e));
    }
    if let Some(e) = read_redo(dir)?.into_iter().find(|e| e.seq == seq) {
        return Ok(Some(e));
    }
    read_proposal(dir, seq)
}

/// Parses `<copy>:<seq>`.
pub fn parse_item(item: &str) -> Result<(&str, u64), String> {
    let (copy, seq) = item
        .rsplit_once(':')
        .ok_or(format!("`{item}` isn't an edit item (<copy>:<seq>)"))?;
    let seq = seq
        .parse()
        .map_err(|_| format!("`{item}`: the part after `:` is an edit number"))?;
    Ok((copy, seq))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{Origin, Status};

    fn entry(seq: u64, tiles: &[u32]) -> Entry {
        Entry {
            seq,
            slot: "tiles".into(),
            entity: "hero".into(),
            ops: vec![],
            palette_ops: vec![],
            prompt: None,
            before: vec![],
            after: vec![],
            state_before: SlotState {
                status: Status::Original,
                origin: Origin::Source,
                license: "CC0-1.0".into(),
                sha256: "0".repeat(64),
                generator: None,
                author: None,
                note: None,
            },
            frames: vec!["base".into()],
            tiles: tiles
                .iter()
                .map(|&t| TileEdit {
                    tile: t,
                    before: "0".repeat(64),
                    after: "1".repeat(64),
                })
                .collect(),
            palettes: None,
            pictures: vec![],
            editor: None,
            sprite: None,
            status: EditStatus::Placed,
            decided_by: None,
            tile: None,
        }
    }

    #[test]
    fn the_log_finds_edits_wherever_they_are() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        assert_eq!(next_seq(d).unwrap(), 1);
        write_history(d, &[entry(1, &[16]), entry(2, &[17])]).unwrap();
        let mut p = entry(3, &[18]);
        p.status = EditStatus::Proposed;
        write_proposal(d, &p).unwrap();
        let mut u = entry(4, &[19]);
        u.status = EditStatus::Undone;
        write_redo(d, &[u]).unwrap();
        assert_eq!(next_seq(d).unwrap(), 5);
        assert_eq!(find(d, 3).unwrap().unwrap().status, EditStatus::Proposed);
        assert_eq!(find(d, 4).unwrap().unwrap().status, EditStatus::Undone);
        assert!(find(d, 9).unwrap().is_none());
        assert!(entry(1, &[16, 17]).overlaps(&entry(2, &[17])));
        assert!(!entry(1, &[16]).overlaps(&entry(2, &[17])));
        assert_eq!(parse_item("edit:12").unwrap(), ("edit", 12));
        assert!(parse_item("edit").is_err());
        // An entry written before this module (one frame, no tiles) still
        // reads, as placed.
        let old = r#"{"seq":1,"slot":"tiles","entity":"hero","ops":[],"before":["0"],"after":["1"],"state_before":{"status":"original","origin":"source","license":"CC0-1.0","sha256":"00"}}"#;
        let e: Entry = serde_json::from_str(old).unwrap();
        assert_eq!(e.status, EditStatus::Placed);
    }

    #[test]
    fn palette_ops_keep_the_backdrop_and_the_grays() {
        let p = [0x0F, 0x16, 0x27, 0x30];
        assert_eq!(
            apply_palette(
                p,
                &PaletteOp::Color {
                    index: 2,
                    nes: 0x2A
                },
                None
            )
            .unwrap(),
            [0x0F, 0x16, 0x2A, 0x30]
        );
        assert!(apply_palette(p, &PaletteOp::Color { index: 0, nes: 1 }, None).is_err());
        assert_eq!(
            apply_palette(p, &PaletteOp::Swap { a: 1, b: 3 }, None).unwrap(),
            [0x0F, 0x30, 0x27, 0x16]
        );
        // $16 (hue 6) -> hue 8; $27 -> hue 9; $30 is gray and stays.
        assert_eq!(
            apply_palette(p, &PaletteOp::Hue { by: 2 }, None).unwrap(),
            [0x0F, 0x18, 0x29, 0x30]
        );
        // Wraps from hue 12 to hue 1.
        assert_eq!(
            apply_palette([0x0F, 0x1C, 0x0C, 0x2C], &PaletteOp::Hue { by: 1 }, None).unwrap(),
            [0x0F, 0x11, 0x01, 0x21]
        );
        assert_eq!(
            apply_palette(p, &PaletteOp::Brightness { by: 1 }, None).unwrap(),
            [0x0F, 0x26, 0x37, 0x30]
        );
        assert_eq!(
            apply_palette(
                [0x0F, 0x1D, 0x2D, 0x3D],
                &PaletteOp::Brightness { by: -1 },
                None
            )
            .unwrap(),
            [0x0F, 0x0F, 0x1D, 0x2D]
        );
        let c = PaletteOp::CopyFrom {
            entity: "slug".into(),
        };
        assert_eq!(
            apply_palette(p, &c, Some([0x0F, 1, 2, 3])).unwrap(),
            [0x0F, 1, 2, 3]
        );
    }
}
