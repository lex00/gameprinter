//! The theme format and the stack format, version [`FORMAT_VERSION`]
//! (docs/formats/theme.md), as Rust types. The JSON Schemas in `schema/`
//! are generated from them.
//!
//! A theme is a directory with `theme.toml` and the files its restyles
//! name. A stack is a `stack.toml` listing themes in order, each baked
//! (shipped with the template) or personal (kept with the player).

use std::path::{Component, Path, PathBuf};

use gp_model::validate::{ENTITY_ROLES, ROLE_GROUPS, WORLD_ROLES};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::color::Rgb;

/// The theme and stack format version.
pub const FORMAT_VERSION: &str = "0.1.3";

/// A theme's file in its directory.
pub const THEME_FILE: &str = "theme.toml";

/// A theme: live effects keyed by role, and restyled assets by slot or role.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "Game Printer theme 0.1.0")]
pub struct Theme {
    /// The format version, "0.1.3" (0.1.0 to 0.1.2 themes read as before).
    pub format: String,
    /// A short name: lowercase letters, digits, `-` and `_`.
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// The theme's own license, an SPDX expression.
    pub license: String,
    /// Models it was made for, by `model.name`. Empty: any model.
    #[serde(default, rename = "for", skip_serializing_if = "Vec::is_empty")]
    pub for_models: Vec<String>,
    /// Live effects: color changes for the pixels of a role.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub palette: Vec<PaletteRule>,
    /// Live effects: picture, particle and screen effects.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effect: Vec<Effect>,
    /// Restyled assets: replacement art by role or slot.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub restyle: Vec<Restyle>,
    /// What Style checks measure art against while the theme is on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<StyleGuide>,
    /// Restyled audio: the same notes through other sound banks, by sound
    /// role (since format 0.1.1).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub audio: Vec<AudioRestyle>,
    /// Sound replacements (since format 0.1.3, the feature `sound`): a
    /// file played on a sound role's cue in place of the model's own sound
    /// (model feature `present.sound_roles`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sound: Vec<SoundReplace>,
}

/// A sound role's replacement (format 0.1.3): `file` plays on the role's
/// cue, and while a theme replaces any role the model's own sound (its
/// songs, or a driver's APU writes) is silent, so a theme with a file for
/// every role needs none of the game's sound data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SoundReplace {
    /// The model's sound role (`present.sound.roles`), such as `jump`.
    pub role: String,
    /// The file, relative to the theme's directory: `.tune` (text notes,
    /// rendered by every runtime the same way), `.wav` (16-bit PCM) or
    /// `.ogg` (Vorbis).
    pub file: String,
    /// `generated`, `uploaded` or `drawn` (made by hand).
    pub origin: String,
    /// An SPDX expression; never `LicenseRef-own-copy`.
    pub license: String,
    /// The file's SHA-256, in hex.
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generator: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// A remaster: every song or effect whose slot has the target role plays
/// its notes through a sound bank mapping (docs/formats/sound-banks.md)
/// instead of the APU. The notes, and so the simulation, don't change.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AudioRestyle {
    /// `sound` (every sound slot), `*`, or a slot role (`music.level`,
    /// `sfx.game`) or its prefix (`music`, `sfx`).
    pub target: String,
    /// The mapping file, relative to the theme's directory.
    pub banks: String,
    /// The mapping's (and its banks') license, an SPDX expression.
    pub license: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl AudioRestyle {
    /// Whether it restyles a slot of `role`.
    pub fn applies(&self, role: &str) -> bool {
        let t = self.target.as_str();
        t == "*"
            || t == "sound"
            || t == role
            || role.strip_prefix(t).is_some_and(|r| r.starts_with('.'))
    }
}

/// A color change for every pixel of a target. The changes apply in the
/// order of the fields below; a pixel `replace` matches keeps its new color.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PaletteRule {
    /// A role, a role group or `*`.
    pub target: String,
    /// Exact swaps, `[from, to]`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub replace: Vec<[String; 2]>,
    /// Map each pixel's lightness onto these colors, darkest first (two or
    /// more), as a gradient map.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ramp: Vec<String>,
    /// Degrees added to the hue.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hue: Option<f64>,
    /// Saturation multiplier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saturation: Option<f64>,
    /// Lightness multiplier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lightness: Option<f64>,
    /// A color mixed in, `tint_amount` (0 to 1, default 0.5) of the way.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tint_amount: Option<f64>,
}

/// A live effect.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Effect {
    /// The target's pixels drawn as a vertical gradient, top to bottom
    /// (a gradient sky).
    Gradient { target: String, colors: Vec<String> },
    /// An outline of `color`, `radius` pixels wide (1 to 3), around the
    /// target's pixels.
    Glow {
        target: String,
        color: String,
        #[serde(default = "one")]
        radius: u32,
    },
    /// Particles: around each body of an entity role, or over the whole
    /// screen with target `screen`.
    Particles {
        #[serde(default = "screen")]
        target: String,
        style: ParticleStyle,
        color: String,
        /// How many, 1 to 256 (per body for a role).
        count: u32,
        #[serde(default)]
        seed: u32,
    },
    /// Every other row darkened, `strength` 0 to 1.
    Scanlines { strength: f64 },
    /// The corners darkened, `strength` 0 to 1.
    Vignette { strength: f64 },
    /// The whole screen mixed with `color`, `amount` 0 to 1.
    Tint { color: String, amount: f64 },
    /// A television's picture (since format 0.1.2), drawn at three overlay
    /// pixels to a game pixel where a runtime can: a dark gap under each
    /// line, the corners darkened, and with `mask` (default true) the
    /// columns tinted red, green and blue in turn, as an aperture grille.
    /// `strength` 0 to 1.
    Crt {
        strength: f64,
        #[serde(default = "yes")]
        mask: bool,
    },
    /// A handheld's pixel grid (since format 0.1.2): a dark line between
    /// pixels at three overlay pixels to a game pixel, `strength` 0 to 1.
    Grid { strength: f64 },
}

fn one() -> u32 {
    1
}

fn screen() -> String {
    "screen".into()
}

/// How particles move (docs/formats/theme.md gives each one's formula).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ParticleStyle {
    Fireflies,
    Snow,
    Rain,
    Sparks,
}

impl Effect {
    /// The effect's kind, as written.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Gradient { .. } => "gradient",
            Self::Glow { .. } => "glow",
            Self::Particles { .. } => "particles",
            Self::Scanlines { .. } => "scanlines",
            Self::Vignette { .. } => "vignette",
            Self::Tint { .. } => "tint",
            Self::Crt { .. } => "crt",
            Self::Grid { .. } => "grid",
        }
    }

    /// What it applies to: a role, group, `*`, `screen`, or "" for the
    /// screen effects.
    pub fn target(&self) -> &str {
        match self {
            Self::Gradient { target, .. }
            | Self::Glow { target, .. }
            | Self::Particles { target, .. } => target,
            _ => "",
        }
    }
}

/// Replacement art. With `target` and no `slot`: every entity of that role
/// draws `file` instead of its sprite. With `slot` and `target`: every item
/// of that role in a metatiles slot draws `file` (16 by 16). With `slot`
/// alone: the slot's whole file (a tiles slot's indexed PNG, same size).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Restyle {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<String>,
    /// With `slot` alone (since format 0.1.3, the feature `entry`): one
    /// entry of a layout slot (update strings, such as a stage's screen,
    /// the HUD's layer or a title) drawn as this picture, the screen's
    /// size, in place of the tiles it writes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
    /// The file, relative to the theme's directory: `.pix` (text pixel art,
    /// [`crate::pix`]) or `.png`.
    pub file: String,
    /// `generated`, `uploaded` or `drawn`.
    pub origin: String,
    /// An SPDX expression; never `LicenseRef-own-copy`.
    pub license: String,
    /// The file's SHA-256, in hex.
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generator: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// The theme's style, which Style checks measure art against.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StyleGuide {
    /// The colors the theme's art keeps to.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub palette: Vec<String>,
    /// The most a pixel may be from the nearest palette color, on average,
    /// in CIE76 delta E (default 12).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_palette_distance: Option<f64>,
    /// The share of a sprite's pixels on an edge, `[min, max]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge_density: Option<[f64; 2]>,
    /// How far a sprite's drawn box may be from its entity's size, as a
    /// share (default 0.25).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_tolerance: Option<f64>,
}

/// A stack: themes in order, bottom first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "Game Printer theme stack 0.1.0")]
pub struct Stack {
    pub format: String,
    /// The model it's for, by `model.name`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layer: Vec<Layer>,
}

/// One layer of a stack.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Layer {
    /// The theme's id, found in the template's `themes/` (baked) or the
    /// player's `$GP_HOME/themes/` (personal).
    pub theme: String,
    pub scope: Scope,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

fn is_true(b: &bool) -> bool {
    *b
}

/// Where a layer lives and who sees it.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum Scope {
    /// Shipped with the template or fork as its default look.
    Baked,
    /// Only on this player's screen; never shipped.
    Personal,
}

impl Stack {
    pub fn parse(text: &str) -> Result<Self, String> {
        let s: Stack = toml::from_str(text).map_err(|e| e.to_string())?;
        if !crate::version_supported(&s.format) {
            return Err(format!(
                "stack format {} isn't supported; this build reads {FORMAT_VERSION}",
                s.format
            ));
        }
        Ok(s)
    }

    pub fn to_text(&self) -> String {
        toml::to_string(self).expect("a stack serializes")
    }
}

/// True for a role, a group or `*`. `screen` too when `screen_ok`.
pub fn is_target(t: &str, screen_ok: bool) -> bool {
    t == "*"
        || (screen_ok && t == "screen")
        || ENTITY_ROLES.contains(&t)
        || WORLD_ROLES.contains(&t)
        || ROLE_GROUPS.iter().any(|(g, _)| *g == t)
}

impl Theme {
    pub fn parse(text: &str) -> Result<Self, String> {
        let t: Theme = toml::from_str(text).map_err(|e| e.to_string())?;
        if !crate::version_supported(&t.format) {
            return Err(format!(
                "theme format {} isn't supported; this build reads {FORMAT_VERSION}",
                t.format
            ));
        }
        Ok(t)
    }

    /// Reads `dir/theme.toml`.
    pub fn load(dir: &Path) -> Result<Self, String> {
        let file = dir.join(THEME_FILE);
        let text =
            std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        Self::parse(&text).map_err(|e| format!("{}: {e}", file.display()))
    }

    pub fn to_text(&self) -> String {
        toml::to_string(self).expect("a theme serializes")
    }

    /// Every problem with the theme, as (where, what). `dir` is its
    /// directory, for the restyles' files.
    pub fn check(&self, dir: &Path) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let mut add = |at: String, msg: String| out.push((at, msg));
        if !is_name(&self.id) {
            add("id".into(), format!("`{}` isn't a theme id", self.id));
        }
        if !is_spdx(&self.license) {
            add(
                "license".into(),
                format!("`{}` isn't an SPDX expression", self.license),
            );
        }
        let color = |at: &str, c: &str, add: &mut dyn FnMut(String, String)| {
            if let Err(e) = Rgb::parse(c) {
                add(at.into(), e);
            }
        };
        let unit = |at: &str, v: f64, add: &mut dyn FnMut(String, String)| {
            if !(0.0..=1.0).contains(&v) {
                add(at.into(), format!("{v} isn't between 0 and 1"));
            }
        };
        for (i, r) in self.palette.iter().enumerate() {
            let at = format!("palette[{i}]");
            if !is_target(&r.target, false) {
                add(
                    format!("{at}.target"),
                    format!("`{}` isn't a role, a group or *", r.target),
                );
            }
            for (j, [a, b]) in r.replace.iter().enumerate() {
                color(&format!("{at}.replace[{j}]"), a, &mut add);
                color(&format!("{at}.replace[{j}]"), b, &mut add);
            }
            if r.ramp.len() == 1 {
                add(format!("{at}.ramp"), "a ramp has two colors or more".into());
            }
            for c in &r.ramp {
                color(&format!("{at}.ramp"), c, &mut add);
            }
            if let Some(c) = &r.tint {
                color(&format!("{at}.tint"), c, &mut add);
            }
            if let Some(a) = r.tint_amount {
                unit(&format!("{at}.tint_amount"), a, &mut add);
            }
            for (name, v) in [("saturation", r.saturation), ("lightness", r.lightness)] {
                if let Some(v) = v
                    && !(0.0..=4.0).contains(&v)
                {
                    add(format!("{at}.{name}"), format!("{v} isn't between 0 and 4"));
                }
            }
        }
        for (i, e) in self.effect.iter().enumerate() {
            let at = format!("effect[{i}]");
            match e {
                Effect::Gradient { target, colors } => {
                    if !is_target(target, false) {
                        add(format!("{at}.target"), format!("`{target}` isn't a role"));
                    }
                    if colors.len() < 2 {
                        add(
                            format!("{at}.colors"),
                            "a gradient has two colors or more".into(),
                        );
                    }
                    for c in colors {
                        color(&format!("{at}.colors"), c, &mut add);
                    }
                }
                Effect::Glow {
                    target,
                    color: c,
                    radius,
                } => {
                    if !is_target(target, false) {
                        add(format!("{at}.target"), format!("`{target}` isn't a role"));
                    }
                    color(&format!("{at}.color"), c, &mut add);
                    if !(1..=3).contains(radius) {
                        add(format!("{at}.radius"), format!("{radius} isn't 1 to 3"));
                    }
                }
                Effect::Particles {
                    target,
                    color: c,
                    count,
                    ..
                } => {
                    if target != "screen" && !ENTITY_ROLES.contains(&target.as_str()) {
                        add(
                            format!("{at}.target"),
                            format!("`{target}` isn't an entity role or `screen`"),
                        );
                    }
                    color(&format!("{at}.color"), c, &mut add);
                    if !(1..=256).contains(count) {
                        add(format!("{at}.count"), format!("{count} isn't 1 to 256"));
                    }
                }
                Effect::Scanlines { strength }
                | Effect::Vignette { strength }
                | Effect::Crt { strength, .. }
                | Effect::Grid { strength } => {
                    unit(&format!("{at}.strength"), *strength, &mut add);
                }
                Effect::Tint { color: c, amount } => {
                    color(&format!("{at}.color"), c, &mut add);
                    unit(&format!("{at}.amount"), *amount, &mut add);
                }
            }
        }
        for (i, a) in self.audio.iter().enumerate() {
            let at = format!("audio[{i}]");
            let t = a.target.as_str();
            let ok = t == "*"
                || t == "sound"
                || t == "music"
                || t == "sfx"
                || t.strip_prefix("music.")
                    .or_else(|| t.strip_prefix("sfx."))
                    .is_some_and(gp_model::path::is_name);
            if !ok {
                add(
                    format!("{at}.target"),
                    format!("`{t}` isn't sound, *, or a music.<mood> or sfx.<event> role"),
                );
            }
            if !is_spdx(&a.license) || a.license.contains("LicenseRef-own-copy") {
                add(
                    format!("{at}.license"),
                    format!("`{}` isn't a license a theme can carry", a.license),
                );
            }
            match restyle_path(dir, &a.banks) {
                Err(e) => add(format!("{at}.banks"), e),
                Ok(p) => match std::fs::read_to_string(&p) {
                    Err(e) => add(format!("{at}.banks"), format!("{}: {e}", p.display())),
                    Ok(text) => {
                        if let Err(e) = gp_sound::banks::Mapping::parse(&text) {
                            add(format!("{at}.banks"), e);
                        }
                    }
                },
            }
        }
        let mut seen = std::collections::BTreeSet::new();
        for (i, r) in self.sound.iter().enumerate() {
            let at = format!("sound[{i}]");
            if !gp_model::path::is_name(&r.role) {
                add(
                    format!("{at}.role"),
                    format!("`{}` isn't a sound role", r.role),
                );
            }
            if !seen.insert(r.role.as_str()) {
                add(
                    format!("{at}.role"),
                    format!("`{}` is replaced twice", r.role),
                );
            }
            if !matches!(r.origin.as_str(), "generated" | "uploaded" | "drawn") {
                add(
                    format!("{at}.origin"),
                    format!("`{}` isn't generated, uploaded or drawn", r.origin),
                );
            }
            if !is_spdx(&r.license) || r.license.contains("LicenseRef-own-copy") {
                add(
                    format!("{at}.license"),
                    format!("`{}` isn't a license a theme can carry", r.license),
                );
            }
            match restyle_path(dir, &r.file) {
                Err(e) => add(format!("{at}.file"), e),
                Ok(p) => match std::fs::read(&p) {
                    Err(e) => add(format!("{at}.file"), format!("{}: {e}", p.display())),
                    Ok(bytes) => {
                        let got = sha256_hex(&bytes);
                        if got != r.sha256 {
                            add(
                                format!("{at}.sha256"),
                                format!("{} has SHA-256 {got}, not {}", r.file, r.sha256),
                            );
                        }
                        if let Err(e) = sound_kind(&r.file, &bytes) {
                            add(format!("{at}.file"), e);
                        }
                    }
                },
            }
        }
        for (i, r) in self.restyle.iter().enumerate() {
            let at = format!("restyle[{i}]");
            if r.entry.is_some() && (r.slot.is_none() || r.target.is_some()) {
                add(
                    format!("{at}.entry"),
                    "an entry restyle names a slot and its entry, and no target".into(),
                );
            }
            match (&r.target, &r.slot) {
                (None, None) => add(
                    at.clone(),
                    "a restyle names a target, a slot or both".into(),
                ),
                (Some(t), None) if !ENTITY_ROLES.contains(&t.as_str()) => add(
                    format!("{at}.target"),
                    format!("`{t}` isn't an entity role; name a slot to restyle its items"),
                ),
                (Some(t), Some(_))
                    if !WORLD_ROLES.contains(&t.as_str())
                        && !ENTITY_ROLES.contains(&t.as_str()) =>
                {
                    add(format!("{at}.target"), format!("`{t}` isn't an item role"))
                }
                _ => {}
            }
            if !matches!(r.origin.as_str(), "generated" | "uploaded" | "drawn") {
                add(
                    format!("{at}.origin"),
                    format!("`{}` isn't generated, uploaded or drawn", r.origin),
                );
            }
            if !is_spdx(&r.license) || r.license.contains("LicenseRef-own-copy") {
                add(
                    format!("{at}.license"),
                    format!(
                        "`{}` isn't a license a theme can carry (an SPDX expression, never LicenseRef-own-copy)",
                        r.license
                    ),
                );
            }
            match restyle_path(dir, &r.file) {
                Err(e) => add(format!("{at}.file"), e),
                Ok(p) => match std::fs::read(&p) {
                    Err(e) => add(format!("{at}.file"), format!("{}: {e}", p.display())),
                    Ok(bytes) => {
                        let got = sha256_hex(&bytes);
                        if got != r.sha256 {
                            add(
                                format!("{at}.sha256"),
                                format!("{} has SHA-256 {got}, not {}", r.file, r.sha256),
                            );
                        }
                    }
                },
            }
        }
        if let Some(s) = &self.style {
            for c in &s.palette {
                color("style.palette", c, &mut add);
            }
            if let Some([lo, hi]) = s.edge_density
                && !(0.0 <= lo && lo <= hi && hi <= 1.0)
            {
                add(
                    "style.edge_density".into(),
                    format!("[{lo}, {hi}] isn't a range within 0 to 1"),
                );
            }
        }
        out
    }
}

/// A restyle's file under the theme's directory. Absolute paths and `..`
/// are refused, so a theme can't name a file outside itself.
pub fn restyle_path(dir: &Path, file: &str) -> Result<PathBuf, String> {
    let p = Path::new(file);
    if p.components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err(format!(
            "`{file}` must be a relative path inside the theme's directory"
        ));
    }
    Ok(dir.join(p))
}

/// A sound replacement's file kind by its name and bytes: `tune` (text
/// notes, [`crate::tune`], which a template's theme uses since a template
/// holds only text), `wav` (a RIFF WAVE file) or `ogg` (an Ogg stream).
pub fn sound_kind(file: &str, bytes: &[u8]) -> Result<&'static str, String> {
    let lower = file.to_ascii_lowercase();
    if lower.ends_with(".tune") {
        let text = std::str::from_utf8(bytes).map_err(|_| format!("{file} isn't UTF-8 text"))?;
        crate::tune::Tune::parse(text).map_err(|e| format!("{file}: {e}"))?;
        return Ok("tune");
    }
    if lower.ends_with(".wav") {
        if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
            return Ok("wav");
        }
        return Err(format!("{file} isn't a RIFF WAVE file"));
    }
    if lower.ends_with(".ogg") {
        if bytes.starts_with(b"OggS") {
            return Ok("ogg");
        }
        return Err(format!("{file} isn't an Ogg file"));
    }
    Err(format!(
        "{file}: a sound replacement is .tune, .wav or .ogg"
    ))
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let d = Sha256::digest(bytes);
    d.iter().map(|b| format!("{b:02x}")).collect()
}

/// A theme id: lowercase letters, digits, `-` and `_`, starting with a letter.
pub fn is_name(s: &str) -> bool {
    s.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// SPDX-style: identifiers, `LicenseRef-...`, `AND`, `OR`, `WITH` and
/// parentheses. Not a full parser, as `assets_place` does it.
pub fn is_spdx(s: &str) -> bool {
    !s.trim().is_empty()
        && s.split(|c: char| c.is_whitespace() || c == '(' || c == ')')
            .filter(|w| !w.is_empty())
            .all(|w| {
                matches!(w, "AND" | "OR" | "WITH")
                    || w.chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '+'))
            })
}

/// The theme format's JSON Schema.
pub fn theme_schema() -> String {
    finish(
        schemars::schema_for!(Theme),
        &format!("game-printer:theme@{FORMAT_VERSION}"),
    )
}

/// The stack format's JSON Schema.
pub fn stack_schema() -> String {
    finish(
        schemars::schema_for!(Stack),
        &format!("game-printer:theme-stack@{FORMAT_VERSION}"),
    )
}

fn finish(mut schema: schemars::Schema, id: &str) -> String {
    schema.insert("$id".into(), id.into());
    let mut text = serde_json::to_string_pretty(&schema).expect("schemas serialize");
    text.push('\n');
    text
}
