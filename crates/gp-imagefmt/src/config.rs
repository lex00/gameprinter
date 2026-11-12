//! The image format config (docs/formats/image-config.md): how sprite
//! frames are laid out in image files and which palette and sidecar files
//! go with them, for image editors and agents. A config is layered:
//! defaults, then `$GP_HOME/config/image.toml`, then the project's
//! `gp.toml` `[image]` table, then a template's and a slot's overrides,
//! then a file given on the command line. Each layer sets only the keys
//! it names.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The config format's version.
pub const FORMAT: &str = "0.1.0";

/// Sheet or a file per frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Layout {
    /// Every frame in one image, on a grid.
    Sheet,
    /// One image per frame.
    Frames,
}

/// How frames fill the grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Order {
    /// A row per state, its frames left to right.
    StateFrame,
    /// Frames in order, `columns` to a row.
    Frame,
}

/// Pixel storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Color {
    /// An indexed (palette) PNG: entry 4p + c is color c of sprite
    /// palette p, and entries 0, 4, 8 and 12 are transparent.
    Indexed,
    /// 8-bit RGBA.
    Rgba,
}

/// The palette file written beside the images.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum PaletteFile {
    None,
    /// GIMP palette (`.gpl`), which GIMP, Aseprite, Krita and Pixelorama read.
    Gpl,
    /// JASC-PAL (`.pal`), Paint Shop Pro's text palette, which Aseprite and
    /// Pixelorama read.
    Jasc,
    /// One `rrggbb` per line (`.hex`), as Lospec gives palettes.
    Hex,
    /// Pixelorama's own JSON palette (`.json`).
    Pixelorama,
}

impl PaletteFile {
    pub fn extension(self) -> Option<&'static str> {
        match self {
            PaletteFile::None => None,
            PaletteFile::Gpl => Some("gpl"),
            PaletteFile::Jasc => Some("pal"),
            PaletteFile::Hex => Some("hex"),
            PaletteFile::Pixelorama => Some("json"),
        }
    }
}

/// Where transparent pixels are kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Transparency {
    /// Alpha 0 (RGBA), or the transparent entries (indexed).
    Alpha,
    /// A key color, `key`, that stands for transparent (RGBA only).
    Key,
}

/// What an editor opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Editor {
    /// PNG files and a JSON sidecar: any editor.
    Png,
    /// PNG files with a Pixelorama palette and the sidecar; Pixelorama
    /// imports a sheet as frames by its grid.
    Pixelorama,
    /// PNG files with the sidecar written as Aseprite's sheet JSON
    /// (`frames`, `meta.frameTags`), which Aseprite's own exports use.
    Aseprite,
}

/// Every setting, resolved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImageConfig {
    pub format: String,
    pub layout: Layout,
    pub order: Order,
    /// Columns of a sheet in `frame` order (at least 1).
    pub columns: u32,
    /// Each cell's width and height; 0 for the largest frame's.
    pub cell: [u32; 2],
    /// Pixels between a frame and its cell's edge.
    pub padding: u32,
    /// Pixels between cells.
    pub spacing: u32,
    /// Each pixel drawn this many times across and down (at least 1).
    pub scale: u32,
    pub color: Color,
    pub palette_file: PaletteFile,
    pub transparency: Transparency,
    /// The key color for `key` transparency, as `#rrggbb`.
    pub key: String,
    /// A frame's name: `{entity}`, `{state}` and `{frame}` (its number in
    /// the set) are filled in. Also each file's name in `frames` layout.
    pub naming: String,
    /// Tag each state's frames in the sidecar.
    pub state_tags: bool,
    pub editor: Editor,
    /// Distinct tiles an import may use, after flips (0: no limit).
    pub tile_budget: u32,
}

impl Default for ImageConfig {
    fn default() -> Self {
        ImageConfig {
            format: FORMAT.into(),
            layout: Layout::Sheet,
            order: Order::StateFrame,
            columns: 8,
            cell: [0, 0],
            padding: 0,
            spacing: 1,
            scale: 1,
            color: Color::Indexed,
            palette_file: PaletteFile::Gpl,
            transparency: Transparency::Alpha,
            key: "#ff00ff".into(),
            naming: "{entity}_{state}_{frame}".into(),
            state_tags: true,
            editor: Editor::Png,
            tile_budget: 256,
        }
    }
}

/// One layer: any of the settings, plus overrides by template and slot.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "Game Printer image format config 0.1.0")]
pub struct Layer {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<Layout>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<Order>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub columns: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cell: Option<[u32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub padding: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spacing: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<Color>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub palette_file: Option<PaletteFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transparency: Option<Transparency>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub naming: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_tags: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub editor: Option<Editor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tile_budget: Option<u32>,
    /// Overrides for one template, by its folder name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub templates: BTreeMap<String, Layer>,
    /// Overrides for one slot (a model asset slot, or an entity's sprite
    /// as `<entity>`), by name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub slots: BTreeMap<String, Layer>,
}

macro_rules! take {
    ($c:ident, $l:ident, $($f:ident),*) => {
        $(if let Some(v) = &$l.$f { $c.$f = v.clone(); })*
    };
}

impl Layer {
    /// A layer from TOML: a file's top level, or `gp.toml`'s `[image]`.
    pub fn parse(text: &str) -> Result<Layer, String> {
        let t: toml::Table = toml::from_str(text).map_err(|e: toml::de::Error| e.to_string())?;
        let v = match t.get("image") {
            Some(toml::Value::Table(i)) => toml::Value::Table(i.clone()),
            _ => toml::Value::Table(t),
        };
        let l: Layer = v
            .try_into()
            .map_err(|e: toml::de::Error| e.message().to_owned())?;
        if let Some(f) = &l.format
            && f.split('.').take(2).collect::<Vec<_>>()
                != FORMAT.split('.').take(2).collect::<Vec<_>>()
        {
            return Err(format!(
                "image config format {f} isn't supported; this build reads {FORMAT}"
            ));
        }
        Ok(l)
    }

    fn apply_own(&self, c: &mut ImageConfig) {
        take!(
            c,
            self,
            layout,
            order,
            columns,
            cell,
            padding,
            spacing,
            scale,
            color,
            palette_file,
            transparency,
            key,
            naming,
            state_tags,
            editor,
            tile_budget
        );
    }

    /// This layer over `c`: its own keys, then its template's, then its
    /// slot's.
    pub fn apply(&self, c: &mut ImageConfig, template: Option<&str>, slot: Option<&str>) {
        self.apply_own(c);
        if let Some(t) = template.and_then(|t| self.templates.get(t)) {
            t.apply(c, None, slot);
        }
        if let Some(s) = slot.and_then(|s| self.slots.get(s)) {
            s.apply_own(c);
        }
    }
}

/// Layers resolved in order, the last winning, then checked.
pub fn resolve(
    layers: &[Layer],
    template: Option<&str>,
    slot: Option<&str>,
) -> Result<ImageConfig, String> {
    let mut c = ImageConfig::default();
    for l in layers {
        l.apply(&mut c, template, slot);
    }
    c.check()?;
    Ok(c)
}

impl ImageConfig {
    pub fn check(&self) -> Result<(), String> {
        if self.scale == 0 || self.scale > 16 {
            return Err("scale is 1 to 16".into());
        }
        if self.columns == 0 {
            return Err("columns is at least 1".into());
        }
        if self.transparency == Transparency::Key && self.color == Color::Indexed {
            return Err(
                "key transparency is for rgba; an indexed image keeps transparent entries".into(),
            );
        }
        crate::palette::parse_hex(&self.key).ok_or(format!("key `{}` isn't #rrggbb", self.key))?;
        Ok(())
    }

    /// A frame's name.
    pub fn name(&self, entity: &str, state: &str, frame: u32) -> String {
        let state = if state.is_empty() { "default" } else { state };
        self.naming
            .replace("{entity}", entity)
            .replace("{state}", state)
            .replace("{frame}", &frame.to_string())
    }
}

/// The JSON Schema of a config layer.
pub fn schema() -> String {
    let mut s = serde_json::to_value(schemars::schema_for!(Layer)).expect("a schema serializes");
    s["$id"] = serde_json::Value::String(format!("game-printer:image-config@{FORMAT}"));
    serde_json::to_string_pretty(&s).expect("json") + "\n"
}
