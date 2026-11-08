//! Stacking: which themes are on, in what order, and what each pixel,
//! effect and restyle resolves to (docs/formats/theme.md, "Stacking").
//!
//! The rules:
//!
//! - Only enabled layers count. Baked layers come first, in their order,
//!   then personal layers, so a player's own themes sit on top of the look a
//!   template ships with.
//! - A baked layer's theme must be in the template's `themes/`; a personal
//!   one is looked up in the player's themes first, then the template's.
//! - A theme may appear once. A stack naming a model, or a theme listing
//!   models in `for`, must name the model it's applied to.
//! - Palette rules compose bottom to top. In each layer only the most
//!   specific rule for a pixel's role applies: its role, then its group,
//!   then `*`.
//! - Effects of the same kind and target: the highest layer's wins.
//!   Particles with different targets all draw.
//! - Restyles: the highest layer with one for a role or slot wins, and lower
//!   layers fill what it lacks. A restyled pixel takes palette rules only
//!   from the layers above the one that restyled it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use gp_model::validate::role_group;

use crate::color::{Rgb, byte};
use crate::format::{Effect, Layer, PaletteRule, ParticleStyle, Restyle, Scope, Stack, Theme};
use crate::particles::{Emitter, Motion};
use crate::pix::Rgba;

/// Where themes are found.
#[derive(Debug, Clone, Default)]
pub struct Places {
    /// The template's `themes/` directory: baked themes.
    pub baked: Option<PathBuf>,
    /// The player's `$GP_HOME/themes/` directory: personal themes.
    pub personal: Option<PathBuf>,
}

/// The stack file of a template's baked look, in its `themes/`.
pub const STACK_FILE: &str = "stack.toml";

impl Places {
    /// The template's baked themes and the player's own: `MODEL/themes/`
    /// and `$GP_HOME/themes/`.
    pub fn for_model(model_dir: &Path, gp_home: Option<&Path>) -> Self {
        let baked = model_dir.join("themes");
        Self {
            baked: baked.is_dir().then_some(baked),
            personal: gp_home.map(|h| h.join("themes")),
        }
    }

    /// The default stack for `model`: the template's baked stack
    /// (`themes/stack.toml`), then the player's personal stack for the
    /// model (`$GP_HOME/themes/stacks/<model>.toml`). Either may be missing.
    pub fn default_stack(&self, model: &str) -> Result<Stack, String> {
        let mut layers: Vec<Layer> = Vec::new();
        let read = |p: PathBuf| -> Result<Option<Stack>, String> {
            match std::fs::read_to_string(&p) {
                Ok(t) => Stack::parse(&t)
                    .map(Some)
                    .map_err(|e| format!("{}: {e}", p.display())),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(format!("{}: {e}", p.display())),
            }
        };
        if let Some(b) = &self.baked
            && let Some(s) = read(b.join(STACK_FILE))?
        {
            if let Some(l) = s.layer.iter().find(|l| l.scope != Scope::Baked) {
                return Err(format!(
                    "{}: the template's stack holds baked layers only, not `{}`",
                    b.join(STACK_FILE).display(),
                    l.theme
                ));
            }
            layers.extend(s.layer);
        }
        if let Some(p) = &self.personal
            && let Some(s) = read(personal_stack(p, model))?
        {
            if let Some(l) = s.layer.iter().find(|l| l.scope != Scope::Personal) {
                return Err(format!(
                    "{}: a player's stack holds personal layers only, not `{}`",
                    personal_stack(p, model).display(),
                    l.theme
                ));
            }
            layers.extend(s.layer);
        }
        Ok(Stack {
            format: crate::FORMAT_VERSION.into(),
            model: Some(model.into()),
            layer: layers,
        })
    }
}

/// The player's stack file for a model.
pub fn personal_stack(personal: &Path, model: &str) -> PathBuf {
    personal.join("stacks").join(format!("{model}.toml"))
}

/// A theme in a stack, read and checked.
#[derive(Debug, Clone)]
pub struct Loaded {
    pub theme: Theme,
    pub dir: PathBuf,
    pub scope: Scope,
    rules: Vec<Rule>,
}

/// A palette rule with its colors read.
#[derive(Debug, Clone)]
struct Rule {
    target: String,
    replace: Vec<(Rgb, Rgb)>,
    ramp: Vec<Rgb>,
    hue: Option<f64>,
    saturation: Option<f64>,
    lightness: Option<f64>,
    tint: Option<(Rgb, f64)>,
}

impl Rule {
    fn new(r: &PaletteRule) -> Result<Self, String> {
        let c = |s: &String| Rgb::parse(s);
        Ok(Self {
            target: r.target.clone(),
            replace: r
                .replace
                .iter()
                .map(|[a, b]| Ok((c(a)?, c(b)?)))
                .collect::<Result<_, String>>()?,
            ramp: r.ramp.iter().map(c).collect::<Result<_, _>>()?,
            hue: r.hue,
            saturation: r.saturation,
            lightness: r.lightness,
            tint: r
                .tint
                .as_ref()
                .map(|t| Ok::<_, String>((c(t)?, r.tint_amount.unwrap_or(0.5))))
                .transpose()?,
        })
    }

    fn apply(&self, c: Rgb) -> Rgb {
        if let Some((_, to)) = self.replace.iter().find(|(from, _)| *from == c) {
            return *to;
        }
        let mut c = c;
        if self.ramp.len() >= 2 {
            let l = (c.lab()[0] / 100.0).clamp(0.0, 1.0);
            let pos = l * (self.ramp.len() - 1) as f64;
            let i = (pos.floor() as usize).min(self.ramp.len() - 2);
            c = self.ramp[i].mix(self.ramp[i + 1], pos - i as f64);
        }
        if self.hue.is_some() || self.saturation.is_some() || self.lightness.is_some() {
            let [h, s, l] = c.hsl();
            c = Rgb::from_hsl([
                h + self.hue.unwrap_or(0.0),
                s * self.saturation.unwrap_or(1.0),
                l * self.lightness.unwrap_or(1.0),
            ]);
        }
        if let Some((t, a)) = self.tint {
            c = c.mix(t, a);
        }
        c
    }
}

/// How well `target` names `role`: 3 the role itself, 2 its group, 1 `*`,
/// 0 not at all. A pixel with no role matches only `*`.
pub fn specificity(target: &str, role: Option<&str>) -> u8 {
    match role {
        Some(r) if target == r => 3,
        Some(r) if role_group(r) == Some(target) => 2,
        _ if target == "*" => 1,
        _ => 0,
    }
}

/// A stack resolved against a model: its layers, bottom first.
#[derive(Debug, Clone, Default)]
pub struct Resolved {
    pub layers: Vec<Loaded>,
}

/// Reads and checks one theme directory as a layer. A folder holding
/// `same_as.toml` is read from the folder it points at, which becomes the
/// layer's `dir` ([`crate::same_as`]).
pub fn load(dir: &Path, scope: Scope) -> Result<Loaded, String> {
    let dir = crate::same_as::target(dir)?;
    load_with(&dir, scope, Theme::load(&dir)?)
}

/// Refuses a theme whose `for` list names models but not `model`.
pub fn check_for(theme: &Theme, model: &str) -> Result<(), String> {
    if theme.for_models.is_empty() || theme.for_models.iter().any(|m| m == model) {
        return Ok(());
    }
    Err(format!(
        "theme `{}` is for {}, not `{model}`",
        theme.id,
        theme.for_models.join(", ")
    ))
}

/// Checks `theme` as a layer whose files are in `dir`, such as an edited
/// theme not yet saved over the one in `dir`.
pub fn load_with(dir: &Path, scope: Scope, theme: Theme) -> Result<Loaded, String> {
    let problems = theme.check(dir);
    if !problems.is_empty() {
        let list: Vec<String> = problems.iter().map(|(a, m)| format!("{a}: {m}")).collect();
        return Err(format!("{}: {}", dir.display(), list.join("; ")));
    }
    let rules = theme
        .palette
        .iter()
        .map(Rule::new)
        .collect::<Result<_, _>>()?;
    Ok(Loaded {
        theme,
        dir: dir.to_owned(),
        scope,
        rules,
    })
}

/// Resolves a stack for the model named `model`.
pub fn resolve(stack: &Stack, places: &Places, model: &str) -> Result<Resolved, String> {
    if let Some(m) = &stack.model
        && m != model
    {
        return Err(format!("the stack is for `{m}`, not `{model}`"));
    }
    let on: Vec<&Layer> = stack.layer.iter().filter(|l| l.enabled).collect();
    let ordered = on
        .iter()
        .filter(|l| l.scope == Scope::Baked)
        .chain(on.iter().filter(|l| l.scope == Scope::Personal));
    let mut layers = Vec::new();
    for l in ordered {
        if layers.iter().any(|x: &Loaded| x.theme.id == l.theme) {
            return Err(format!("theme `{}` is in the stack twice", l.theme));
        }
        let candidates: Vec<&PathBuf> = match l.scope {
            Scope::Baked => places.baked.iter().collect(),
            Scope::Personal => places.personal.iter().chain(places.baked.iter()).collect(),
        };
        let dir = candidates
            .iter()
            .map(|d| d.join(&l.theme))
            .find(|d| crate::same_as::is_theme_dir(d))
            .ok_or_else(|| {
                format!(
                    "no {} theme `{}` in {}",
                    match l.scope {
                        Scope::Baked => "baked",
                        Scope::Personal => "personal",
                    },
                    l.theme,
                    if candidates.is_empty() {
                        "any themes directory".into()
                    } else {
                        candidates
                            .iter()
                            .map(|d| d.display().to_string())
                            .collect::<Vec<_>>()
                            .join(" or ")
                    }
                )
            })?;
        let loaded = load(&dir, l.scope)?;
        if loaded.theme.id != l.theme {
            return Err(format!(
                "{}: its id is `{}`, not `{}`",
                dir.display(),
                loaded.theme.id,
                l.theme
            ));
        }
        check_for(&loaded.theme, model)?;
        layers.push(loaded);
    }
    Ok(Resolved { layers })
}

/// The particle motion for a style.
fn motion(s: ParticleStyle) -> Motion {
    match s {
        ParticleStyle::Fireflies => Motion::Fireflies,
        ParticleStyle::Snow => Motion::Snow,
        ParticleStyle::Rain => Motion::Rain,
        ParticleStyle::Sparks => Motion::Sparks,
    }
}

/// A picture whose pixels carry roles, as the presentation draws it before
/// a theme: each pixel's color, its role (an index into [`crate::ROLES`],
/// or [`crate::NO_ROLE`]) and the layer its art came from (0 for the
/// model's own art, the layer above a restyle's for restyled art).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleImage {
    pub image: Rgba,
    pub roles: Vec<u8>,
    pub floor: Vec<u8>,
}

impl RoleImage {
    pub fn new(width: u32, height: u32) -> Self {
        let n = (width * height) as usize;
        Self {
            image: Rgba::new(width, height),
            roles: vec![crate::NO_ROLE; n],
            floor: vec![0; n],
        }
    }

    pub fn set(&mut self, x: u32, y: u32, c: [u8; 4], role: u8, floor: u8) {
        let (w, h) = (self.image.width, self.image.height);
        if x < w && y < h {
            let i = (y * w + x) as usize;
            self.image.pixels[i] = c;
            self.roles[i] = role;
            self.floor[i] = floor;
        }
    }

    pub fn role_at(&self, i: usize) -> Option<&'static str> {
        crate::ROLES.get(usize::from(self.roles[i])).copied()
    }
}

impl Resolved {
    /// No themes: everything draws as the model has it.
    pub fn none() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.layers.is_empty()
    }

    /// The color a pixel of `role` gets, through the layers from `floor` up.
    pub fn color(&self, role: Option<&str>, floor: usize, c: Rgb) -> Rgb {
        let mut c = c;
        for l in self.layers.iter().skip(floor) {
            let best = l
                .rules
                .iter()
                .map(|r| (specificity(&r.target, role), r))
                .filter(|(s, _)| *s > 0)
                .max_by_key(|(s, _)| *s);
            // Ties: the later rule in the file, as max_by_key keeps the last.
            if let Some((_, r)) = best {
                c = r.apply(c);
            }
        }
        c
    }

    /// The effects in force, each (kind, target) once: the highest layer's.
    pub fn effects(&self) -> Vec<&Effect> {
        let mut by: BTreeMap<(&str, String), &Effect> = BTreeMap::new();
        for l in &self.layers {
            for e in &l.theme.effect {
                let key = match e {
                    Effect::Particles { target, style, .. } => format!("{target}/{style:?}"),
                    _ => e.target().to_owned(),
                };
                by.insert((e.kind(), key), e);
            }
        }
        by.into_values().collect()
    }

    /// The highest layer's restyle for entities of `role`, with its layer.
    pub fn restyle_role(&self, role: &str) -> Option<(usize, PathBuf, &Restyle)> {
        self.restyle(|r| r.slot.is_none() && r.target.as_deref() == Some(role))
    }

    /// The highest layer's restyle for items of `role` in `slot`.
    pub fn restyle_item(&self, slot: &str, role: &str) -> Option<(usize, PathBuf, &Restyle)> {
        self.restyle(|r| r.slot.as_deref() == Some(slot) && r.target.as_deref() == Some(role))
    }

    /// The highest layer's restyle of a whole slot.
    pub fn restyle_slot(&self, slot: &str) -> Option<(usize, PathBuf, &Restyle)> {
        self.restyle(|r| r.slot.as_deref() == Some(slot) && r.target.is_none() && r.entry.is_none())
    }

    /// The highest layer's picture for one entry of a layout slot (format
    /// 0.1.3).
    pub fn restyle_entry(&self, slot: &str, entry: &str) -> Option<(usize, PathBuf, &Restyle)> {
        self.restyle(|r| r.slot.as_deref() == Some(slot) && r.entry.as_deref() == Some(entry))
    }

    fn restyle(&self, f: impl Fn(&Restyle) -> bool) -> Option<(usize, PathBuf, &Restyle)> {
        self.layers.iter().enumerate().rev().find_map(|(i, l)| {
            l.theme
                .restyle
                .iter()
                .find(|r| f(r))
                .map(|r| (i, l.dir.join(&r.file), r))
        })
    }

    /// The picture with the stack's palette rules and picture effects
    /// (gradients, glows) applied.
    pub fn apply(&self, img: &RoleImage) -> Rgba {
        let mut out = img.image.clone();
        let (w, h) = (out.width, out.height);
        let effects = self.effects();
        let gradients: Vec<(&str, Vec<Rgb>)> = effects
            .iter()
            .filter_map(|e| match e {
                Effect::Gradient { target, colors } => Some((
                    target.as_str(),
                    colors.iter().filter_map(|c| Rgb::parse(c).ok()).collect(),
                )),
                _ => None,
            })
            .collect();
        for (i, px) in out.pixels.iter_mut().enumerate() {
            if px[3] == 0 {
                continue;
            }
            let role = img.role_at(i);
            let floor = usize::from(img.floor[i]);
            let c = self.color(role, floor, Rgb([px[0], px[1], px[2]]));
            let y = i as u32 / w;
            let c = match gradients
                .iter()
                .filter(|(t, _)| specificity(t, role) > 0)
                .max_by_key(|(t, _)| specificity(t, role))
            {
                Some((_, colors)) if colors.len() >= 2 => {
                    let pos = f64::from(y) / f64::from(h.max(2) - 1) * (colors.len() - 1) as f64;
                    let k = (pos.floor() as usize).min(colors.len() - 2);
                    colors[k].mix(colors[k + 1], pos - k as f64)
                }
                _ => c,
            };
            *px = [c.0[0], c.0[1], c.0[2], px[3]];
        }
        for e in &effects {
            let Effect::Glow {
                target,
                color,
                radius,
            } = e
            else {
                continue;
            };
            let Ok(Rgb([r, g, b])) = Rgb::parse(color) else {
                continue;
            };
            let rad = *radius as i64;
            let src = out.clone();
            for y in 0..h as i64 {
                for x in 0..w as i64 {
                    if src.get(x as u32, y as u32)[3] != 0 {
                        continue;
                    }
                    let near = (-rad..=rad).any(|dy| {
                        (-rad..=rad).any(|dx| {
                            let (nx, ny) = (x + dx, y + dy);
                            nx >= 0
                                && ny >= 0
                                && nx < w as i64
                                && ny < h as i64
                                && src.get(nx as u32, ny as u32)[3] != 0
                                && specificity(target, img.role_at((ny * w as i64 + nx) as usize))
                                    > 0
                        })
                    });
                    if near {
                        out.set(x as u32, y as u32, [r, g, b, 160]);
                    }
                }
            }
        }
        out
    }

    /// The screen overlay: tint, then vignette, then scanlines, then a
    /// CRT's or a grid's look, as one RGBA picture of the view's size drawn
    /// over everything. None without them. At this size a CRT is its line
    /// gaps on every other row and its corners, and a grid isn't drawn;
    /// [`Resolved::overlay_fine`] draws both whole.
    pub fn overlay(&self, w: u32, h: u32) -> Option<Rgba> {
        self.overlay_at(w, h, 1)
    }

    /// The overlay at [`Resolved::overlay_scale`] overlay pixels to a game
    /// pixel each way, for a runtime that draws it over the view's size:
    /// with a CRT or a grid, 3, and 1 without.
    pub fn overlay_fine(&self, w: u32, h: u32) -> Option<Rgba> {
        self.overlay_at(w, h, self.overlay_scale())
    }

    /// Overlay pixels to a game pixel in [`Resolved::overlay_fine`].
    pub fn overlay_scale(&self) -> u32 {
        let fine = self
            .effects()
            .iter()
            .any(|e| matches!(e, Effect::Crt { .. } | Effect::Grid { .. }));
        if fine { 3 } else { 1 }
    }

    fn overlay_at(&self, w: u32, h: u32, k: u32) -> Option<Rgba> {
        let effects = self.effects();
        let screen: Vec<&&Effect> = effects
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    Effect::Tint { .. }
                        | Effect::Vignette { .. }
                        | Effect::Scanlines { .. }
                        | Effect::Crt { .. }
                        | Effect::Grid { .. }
                )
            })
            .collect();
        if screen.is_empty() {
            return None;
        }
        let k = k.max(1);
        let (fw, fh) = (w * k, h * k);
        let mut img = Rgba::new(fw, fh);
        let over = |dst: [u8; 4], src: [f64; 3], a: f64| -> [u8; 4] {
            let da = f64::from(dst[3]) / 255.0;
            let oa = a + da * (1.0 - a);
            if oa <= 0.0 {
                return [0; 4];
            }
            let ch = |i: usize| (src[i] * a + f64::from(dst[i]) * da * (1.0 - a)) / oa;
            [byte(ch(0)), byte(ch(1)), byte(ch(2)), byte(oa * 255.0)]
        };
        let order = |e: &Effect| match e {
            Effect::Tint { .. } => 0,
            Effect::Vignette { .. } => 1,
            Effect::Scanlines { .. } => 2,
            _ => 3,
        };
        // How dark a corner is, 0 to 1, with a smooth falloff from half way
        // out.
        let corner = |x: u32, y: u32| {
            let dx = (f64::from(x) + 0.5) / f64::from(fw) * 2.0 - 1.0;
            let dy = (f64::from(y) + 0.5) / f64::from(fh) * 2.0 - 1.0;
            let d = ((dx * dx + dy * dy).sqrt() / 2f64.sqrt()).clamp(0.0, 1.0);
            let s = ((d - 0.5) / 0.5).clamp(0.0, 1.0);
            s * s * (3.0 - 2.0 * s)
        };
        let mut sorted = screen.clone();
        sorted.sort_by_key(|e| order(e));
        for e in sorted {
            for y in 0..fh {
                for x in 0..fw {
                    // The game pixel's row, and the place inside it.
                    let (gy, sx, sy) = (y / k, x % k, y % k);
                    let mut layers: [([f64; 3], f64); 2] = [([0.0; 3], 0.0); 2];
                    match e {
                        Effect::Tint { color, amount } => {
                            let Rgb(c) = Rgb::parse(color).unwrap_or_default();
                            layers[0] = (c.map(f64::from), *amount);
                        }
                        Effect::Vignette { strength } => {
                            layers[0] = ([0.0; 3], strength * corner(x, y));
                        }
                        Effect::Scanlines { strength } => {
                            layers[0] = ([0.0; 3], if gy % 2 == 1 { *strength } else { 0.0 });
                        }
                        Effect::Crt { strength, mask } => {
                            // The gap under a line: the last overlay row of
                            // each game row, or every other row at scale 1.
                            let gap = if k >= 3 { sy == k - 1 } else { gy % 2 == 1 };
                            let dark = if gap { 0.6 } else { 0.0 } + 0.5 * corner(x, y);
                            layers[0] = ([0.0; 3], (strength * dark).min(1.0));
                            if *mask && k >= 3 {
                                let mut c = [0.0; 3];
                                c[(sx % 3) as usize] = 255.0;
                                layers[1] = (c, strength * 0.2);
                            }
                        }
                        Effect::Grid { strength } => {
                            let line = k >= 3 && (sx == k - 1 || sy == k - 1);
                            layers[0] = ([0.0; 3], if line { *strength } else { 0.0 });
                        }
                        _ => continue,
                    }
                    for (c, a) in layers {
                        if a > 0.0 {
                            let p = img.get(x, y);
                            img.set(x, y, over(p, c, a));
                        }
                    }
                }
            }
        }
        Some(img)
    }

    /// The particle emitters in force, with the entity role each follows
    /// (None: the screen).
    pub fn emitters(&self) -> Vec<(Option<String>, Emitter)> {
        self.effects()
            .into_iter()
            .filter_map(|e| match e {
                Effect::Particles {
                    target,
                    style,
                    color,
                    count,
                    seed,
                } => {
                    let Rgb(c) = Rgb::parse(color).ok()?;
                    let follows = target != "screen";
                    Some((
                        follows.then(|| target.clone()),
                        Emitter {
                            motion: motion(*style),
                            color: c,
                            count: *count,
                            seed: *seed,
                            follows,
                        },
                    ))
                }
                _ => None,
            })
            .collect()
    }

    /// The color after the screen tint only, for the readability check:
    /// tint changes every color, while scanlines and vignettes darken
    /// player and background alike.
    pub fn screen_color(&self, c: Rgb) -> Rgb {
        self.effects()
            .iter()
            .find_map(|e| match e {
                Effect::Tint { color, amount } => Rgb::parse(color).ok().map(|t| c.mix(t, *amount)),
                _ => None,
            })
            .unwrap_or(c)
    }

    /// One line per layer, bottom first: id, scope and directory.
    pub fn describe(&self) -> Vec<serde_json::Value> {
        self.layers
            .iter()
            .map(|l| {
                serde_json::json!({
                    "theme": l.theme.id,
                    "title": l.theme.title,
                    "scope": l.scope,
                    "dir": l.dir,
                    "license": l.theme.license,
                })
            })
            .collect()
    }
}

#[cfg(test)]
#[path = "stack_tests.rs"]
mod tests;

impl Resolved {
    /// The file a sound role plays from (format 0.1.3's `sound`): the
    /// highest layer's replacement for the role.
    pub fn sound_file(&self, role: &str) -> Option<PathBuf> {
        self.layers.iter().rev().find_map(|l| {
            l.theme
                .sound
                .iter()
                .find(|s| s.role == role)
                .and_then(|s| crate::format::restyle_path(&l.dir, &s.file).ok())
        })
    }

    /// Whether any layer replaces a sound role: the model's own sound is
    /// silent then, and the replacements play on their cues.
    pub fn replaces_sound(&self) -> bool {
        self.layers.iter().any(|l| !l.theme.sound.is_empty())
    }

    /// The sound bank mapping a slot of `role` plays through: the highest
    /// layer's audio restyle that applies, as (mapping file, theme
    /// directory). None plays the APU.
    pub fn audio_mapping(&self, role: &str) -> Option<(PathBuf, PathBuf)> {
        self.layers.iter().rev().find_map(|l| {
            l.theme
                .audio
                .iter()
                .find(|a| a.applies(role))
                .and_then(|a| crate::format::restyle_path(&l.dir, &a.banks).ok())
                .map(|p| (p, l.dir.clone()))
        })
    }
}
