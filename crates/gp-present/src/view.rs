//! Picture options: how a frame is drawn, with nothing in them read by the
//! game (docs/generator.md, "Picture options"). Phases D and E of the
//! standard feature set (docs/research/reimplementation-features.md).
//!
//! - Draw-only widescreen: the game keeps its own view (who appears, who
//!   goes and where the left wall is all stay as the original's), and
//!   [`window`] draws more of the level either side of it. A body that
//!   appears or goes outside the game's view fades in or out instead of
//!   popping, and the part behind a left wall is dimmed.
//! - The console's sprite limit, when the model draws one
//!   ([`crate::SpriteLimit`]), or no limit.
//! - Flash dimming: the screen's large colors (the background and each
//!   cycling color) change by at most [`DIM_STEP`] of brightness a frame.
//! - Smooth motion: between two ticks, each body and the camera are drawn
//!   part way from where they were to where they are ([`SUB`] parts a
//!   pixel). A move bigger than the body's speed allows (a pipe, a
//!   respawn, a level change) is drawn at once.
//!
//! [`Viewer`] holds what these need to remember. Step it once after each
//! tick, after the [`crate::Animator`], and ask it for each drawn frame.
//! It keeps to whole numbers and no clock, as the rest of the crate does;
//! the caller gives the place between two ticks as a whole number.

use gp_sim::{Spec, World};

use crate::{Draw, Present, Scene, Screen};

/// Parts of a pixel the smooth motion places things in.
pub const SUB: i64 = 256;

/// Frames a body takes to fade in or out at the edge of the game's view.
pub const FADE: u32 = 8;

/// The most a large color's brightness (0 to 255) changes in a frame with
/// flash dimming on.
pub const DIM_STEP: i32 = 16;

/// The aspects widescreen offers, width over height.
pub const ASPECTS: [[u32; 2]; 2] = [[16, 10], [16, 9]];

/// Where the extra width goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Side {
    /// A quarter behind the game's view and the rest ahead (the default).
    #[default]
    Ahead,
    /// Half each side.
    Center,
    /// All ahead: the game's view at the window's left.
    Front,
}

impl Side {
    pub const ALL: [Side; 3] = [Side::Ahead, Side::Center, Side::Front];

    pub fn name(self) -> &'static str {
        match self {
            Side::Ahead => "ahead",
            Side::Center => "center",
            Side::Front => "front",
        }
    }

    pub fn parse(s: &str) -> Option<Side> {
        Side::ALL.into_iter().find(|x| x.name() == s)
    }

    /// The extra pixels drawn behind the game's view.
    pub fn behind(self, extra: u32) -> u32 {
        match self {
            Side::Ahead => extra / 4,
            Side::Center => extra / 2,
            Side::Front => 0,
        }
    }
}

/// Draw-only widescreen: the window's aspect and where the extra goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Wide {
    pub aspect: [u32; 2],
    pub side: Side,
}

impl Screen {
    /// The width in game pixels a window of `aspect` shows at the visible
    /// height, with the pixels' shape: never less than the visible width,
    /// and even, so the window's middle is a whole pixel.
    pub fn wide_width(&self, aspect: [u32; 2]) -> u32 {
        let [_, _, w, h] = self.visible();
        let [an, ad] = aspect.map(|v| u64::from(v.max(1)));
        let [pn, pd] = self.aspect.map(|v| u64::from(v.max(1)));
        let g = u64::from(h) * an * pd / (ad * pn);
        ((g as u32) & !1).max(w)
    }

    /// The visible width with `wide`, or the visible width without.
    pub fn drawn_width(&self, wide: Option<Wide>) -> u32 {
        match wide {
            Some(x) => self.wide_width(x.aspect),
            None => self.visible()[2],
        }
    }

    /// The window's size at scale 1 for `width` game pixels across, with
    /// the pixels' shape (`window` for another width).
    pub fn window_for(&self, width: u32) -> [u32; 2] {
        let [_, _, _, h] = self.visible();
        let [n, d] = self.aspect;
        [
            (u64::from(width) * u64::from(n.max(1)) / u64::from(d.max(1))) as u32,
            h,
        ]
    }
}

/// The part of the world a frame draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Window {
    /// The top left corner in world pixels, and the size.
    pub x: i64,
    pub y: i64,
    pub w: u32,
    pub h: u32,
    /// The game's own view's left edge, in pixels from the window's left.
    pub view_x: i64,
    /// Pixels from the window's left that lie behind the game's left wall
    /// (a camera that never scrolls back): drawn dimmed. 0 for none.
    pub wall: i64,
    /// The screen's top left for the HUD, in world pixels: the HUD (in
    /// screen positions) is drawn from here, in the window's middle.
    pub hud: [i64; 2],
}

impl Window {
    /// Whether a picture `w` wide at world x `x` lies wholly outside the
    /// game's own view, which is `view_w` wide.
    pub fn outside_view(&self, x: i64, w: u32, view_w: u32) -> bool {
        let left = self.x + self.view_x;
        x + i64::from(w) <= left || x >= left + i64::from(view_w)
    }
}

/// The window around the game's view. `camera` is the scene's (the
/// screen's top left in world pixels), `level_w` the level's width in
/// pixels, and `wall` whether the camera never scrolls back. The window
/// stays in the level when the level is wide enough: at its start the
/// extra goes ahead, at its end behind. The game's view is in the window
/// always, since the game keeps its camera in the level.
pub fn window(
    screen: &Screen,
    camera: [i64; 2],
    wide: Option<Wide>,
    level_w: Option<i64>,
    wall: bool,
) -> Window {
    let [vl, vt, vw, vh] = screen.visible().map(i64::from);
    let view_left = camera[0] + vl;
    let w = i64::from(screen.drawn_width(wide));
    let extra = (w - vw).max(0);
    let behind = wide.map_or(0, |x| i64::from(x.side.behind(extra as u32)));
    let mut x = view_left - behind;
    if let Some(lw) = level_w.filter(|_| extra > 0) {
        if lw >= w {
            x = x.clamp(0, lw - w);
        } else {
            // A level narrower than the window: centered on it.
            x = (lw - w) / 2;
        }
        // The game's view stays in the window.
        x = x.clamp(view_left + vw - w, view_left);
    }
    let view_x = view_left - x;
    Window {
        x,
        y: camera[1] + vt,
        w: w as u32,
        h: vh as u32,
        view_x,
        wall: if wall { view_x.max(0) } else { 0 },
        hud: [x + (w - vw) / 2 - vl, camera[1]],
    }
}

/// What a level's camera tells the window: the level's width in pixels,
/// and whether the camera follows across and never scrolls back.
pub fn level_edges(spec: &Spec, w: &World) -> (Option<i64>, bool) {
    let Some(level) = spec.levels.get(w.level) else {
        return (None, false);
    };
    let width = level.size[0] * level.tile_size;
    let wall = spec.cameras.first().is_some_and(|c| {
        let across = level
            .camera
            .as_ref()
            .and_then(|lc| lc.axis)
            .map_or(c.axis_x, |a| a[0]);
        across && !c.scroll_back
    });
    (Some(width).filter(|w| *w > 0), wall)
}

/// The parts of each body the console's sprite limit hides this frame, by
/// body: rectangles from the body's picture's top left (left, top, width,
/// height, on the screen, before any flip), as the NES evaluates its
/// sprites. Each body is as many hardware sprites as its picture is wide,
/// each covering the picture's lines; on each line of the window the
/// first `per_line` sprites there are drawn and the rest are not, the
/// HUD's sprites first, then the bodies by their place in the console's
/// sprite table (`Scene::rank`, lower first; bodies without one after, in
/// body order), each body's sprites left to right. A body the game draws
/// into the name table isn't counted. When the limit flickers the bodies'
/// order is turned by the frame number. A body may lose a few lines of one
/// column: Original Edition's 50m drops a moving ladder's lower sprites on
/// the lines its pies crowd, as the ladders come last in the table
/// ($02C0).
pub fn sprite_cuts(p: &Present, s: &Scene, win: &Window, frame: u64) -> Vec<Vec<[i64; 4]>> {
    let mut cuts = vec![Vec::new(); s.bodies.len()];
    let Some(limit) = p.screen.sprite_limit else {
        return cuts;
    };
    let h = win.h as usize;
    let mut lines = vec![0u32; h];
    let wide = i64::from(limit.width.max(1));
    let per = |pic: u32| -> Option<(u32, u32)> {
        let size = p.pictures.get(pic as usize)?.size;
        Some((size[0].div_ceil(limit.width.max(1)), size[1]))
    };
    let [_, vt, _, _] = p.screen.visible().map(i64::from);
    for d in &s.hud[s.hud.len() - s.hud_sprites..] {
        if let Some((n, rows)) = per(d.pic) {
            let from = (d.y - vt).max(0) as usize;
            let to = (d.y - vt + i64::from(rows)).clamp(0, h as i64) as usize;
            for c in lines.iter_mut().take(to).skip(from) {
                *c += n;
            }
        }
    }
    let mut drawn: Vec<usize> = (0..s.bodies.len())
        .filter(|&i| {
            !s.name_table.get(i).copied().unwrap_or(false)
                && s.bodies[i].is_some_and(|d| {
                    let w = per(d.pic).map_or(0, |x| x.0 * limit.width);
                    d.x + i64::from(w) > win.x && d.x < win.x + i64::from(win.w)
                })
        })
        .collect();
    if drawn.is_empty() {
        return cuts;
    }
    drawn.sort_by_key(|&i| s.rank.get(i).copied().unwrap_or(u32::MAX));
    let turn = if limit.flicker {
        (frame % drawn.len() as u64) as usize
    } else {
        0
    };
    for k in 0..drawn.len() {
        let i = drawn[(k + turn) % drawn.len()];
        let Some(d) = s.bodies[i] else { continue };
        let Some((n, rows)) = per(d.pic) else {
            continue;
        };
        let top = d.y - win.y;
        for (row, line) in (top..top + i64::from(rows)).enumerate() {
            if line < 0 || line >= h as i64 {
                continue;
            }
            let c = &mut lines[line as usize];
            for col in 0..i64::from(n) {
                if *c < limit.per_line {
                    *c += 1;
                    continue;
                }
                // One line of this column is past the limit: join it to
                // the cut of the line above when that one is the same.
                let r = [col * wide, row as i64, wide, 1];
                match cuts[i]
                    .iter_mut()
                    .find(|x: &&mut [i64; 4]| x[0] == r[0] && x[1] + x[3] == r[1])
                {
                    Some(x) => x[3] += 1,
                    None => cuts[i].push(r),
                }
            }
        }
    }
    cuts
}

/// The bodies the console's sprite limit hides whole this frame, by body:
/// those [`sprite_cuts`] cuts on every line of every column.
pub fn over_limit(p: &Present, s: &Scene, win: &Window, frame: u64) -> Vec<bool> {
    let cuts = sprite_cuts(p, s, win, frame);
    let Some(limit) = p.screen.sprite_limit else {
        return vec![false; s.bodies.len()];
    };
    (0..s.bodies.len())
        .map(|i| {
            let Some(size) = s.bodies[i]
                .and_then(|d| p.pictures.get(d.pic as usize))
                .map(|x| x.size)
            else {
                return false;
            };
            let n = size[0].div_ceil(limit.width.max(1));
            let cut: i64 = cuts[i].iter().map(|c| c[3]).sum();
            !cuts[i].is_empty() && cut == i64::from(n) * i64::from(size[1])
        })
        .collect()
}

/// Applies the console's sprite limit to a scene: the bodies it hides
/// whole are not drawn, and the rest keep their cuts (`Scene::cut`).
pub fn limit(p: &Present, s: &mut Scene, win: &Window, frame: u64) {
    let hidden = over_limit(p, s, win, frame);
    s.cut = sprite_cuts(p, s, win, frame);
    for (i, hide) in hidden.into_iter().enumerate() {
        if hide {
            s.bodies[i] = None;
            s.cut[i].clear();
        }
    }
}

/// A large color eased toward `target` by at most [`DIM_STEP`] of
/// brightness (Rec. 709 weights on the 0 to 255 values).
pub fn ease(shown: [u8; 3], target: [u8; 3]) -> [u8; 3] {
    let y = |c: [u8; 3]| {
        (2126 * i32::from(c[0]) + 7152 * i32::from(c[1]) + 722 * i32::from(c[2])) / 10_000
    };
    let d = (y(target) - y(shown)).abs();
    if d <= DIM_STEP {
        return target;
    }
    let mix = |a: u8, b: u8| (i32::from(a) + (i32::from(b) - i32::from(a)) * DIM_STEP / d) as u8;
    [
        mix(shown[0], target[0]),
        mix(shown[1], target[1]),
        mix(shown[2], target[2]),
    ]
}

/// How a frame is drawn. None of it is read by the game.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    /// Draw-only widescreen, or the game's own width.
    pub wide: Option<Wide>,
    /// Draw the console's sprite limit, when the model has one.
    pub sprite_limit: bool,
    pub dim_flashes: bool,
    pub smooth: bool,
}

impl Default for Options {
    /// The original's picture: its width, its sprite limit, no dimming and
    /// no smoothing.
    fn default() -> Self {
        Options {
            wide: None,
            sprite_limit: true,
            dim_flashes: false,
            smooth: false,
        }
    }
}

impl Options {
    /// Every option away from the original's: widescreen at the widest
    /// aspect, no sprite limit, dimming and smooth motion.
    pub fn all() -> Self {
        Options {
            wide: Some(Wide {
                aspect: ASPECTS[ASPECTS.len() - 1],
                side: Side::Ahead,
            }),
            sprite_limit: false,
            dim_flashes: true,
            smooth: true,
        }
    }
}

/// A frame as drawn with the options.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Framed {
    /// The scene, with the colors dimmed and the bodies past the sprite
    /// limit taken out.
    pub scene: Scene,
    pub window: Window,
    /// Each body's opacity, 0 to 255, by body.
    pub alpha: Vec<u8>,
    /// Bodies gone outside the game's view, still fading: the picture and
    /// its opacity.
    pub ghosts: Vec<(Draw, u8)>,
    /// What each body is moved by between two ticks, by body, in
    /// [`SUB`] parts of a pixel, from its place in the scene.
    pub nudge: Vec<[i64; 2]>,
    /// The camera's, likewise; the window and the HUD move with it.
    pub camera_nudge: [i64; 2],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct BodyAt {
    active: bool,
    kind: usize,
    pos: [i64; 2],
    speed: [i64; 2],
}

/// Where things were after a tick, in raw fixed-point units.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Snap {
    frame: u64,
    level: usize,
    camera: [i64; 2],
    bodies: Vec<BodyAt>,
}

impl Snap {
    fn of(w: &World) -> Snap {
        let cam = w.cameras.first().copied().unwrap_or_default();
        Snap {
            frame: w.frame,
            level: w.level,
            camera: [cam[0].raw(), cam[1].raw()],
            bodies: w
                .bodies
                .iter()
                .map(|b| BodyAt {
                    active: b.active(),
                    kind: b.kind,
                    pos: [b.x.raw(), b.y.raw()],
                    speed: [b.vx.raw(), b.vy.raw()],
                })
                .collect(),
        }
    }

    /// Whether `next` is the tick right after this one in the same level
    /// with the same bodies.
    fn followed_by(&self, next: &Snap) -> bool {
        self.frame + 1 == next.frame
            && self.level == next.level
            && self.bodies.len() == next.bodies.len()
    }
}

/// The raw units a whole pixel is.
const ONE: i64 = gp_sim::Fx::ONE.0;

/// What a viewer remembers from tick to tick: the last two ticks' places
/// for smooth motion, each body's fade, and the large colors shown.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Viewer {
    pub options: Options,
    last: Option<Snap>,
    now: Option<Snap>,
    /// Ticks since each body appeared outside the game's view, while it
    /// fades in.
    fade_in: Vec<u32>,
    /// Each body's picture after the last tick.
    draws: Vec<Option<Draw>>,
    ghosts: Vec<(Draw, u32)>,
    /// The background and each cycling color as shown, with dimming.
    colors: Vec<[u8; 3]>,
}

fn colors_of(s: &Scene) -> Vec<[u8; 3]> {
    std::iter::once(s.clear)
        .chain(s.tints.iter().map(|t| t.1))
        .chain(s.hud_tints.iter().map(|t| t.1))
        .collect()
}

impl Viewer {
    pub fn new(options: Options) -> Self {
        Viewer {
            options,
            ..Viewer::default()
        }
    }

    /// Forgets the ticks before, as after a state is put back.
    pub fn reset(&mut self) {
        *self = Viewer::new(self.options);
    }

    /// Steps after a tick, with the tick's scene (`Animator::scene` after
    /// `Animator::step`).
    pub fn step(&mut self, p: &Present, w: &World, scene: &Scene) {
        let snap = Snap::of(w);
        let follows = self.now.as_ref().is_some_and(|n| n.followed_by(&snap));
        let view_w = p.screen.visible()[2];
        let win = window(&p.screen, scene.camera, None, None, false);
        let width = |d: &Draw| p.pictures.get(d.pic as usize).map_or(0, |x| x.size[0]);
        if follows {
            let was = self
                .now
                .as_ref()
                .map(|n| n.bodies.clone())
                .unwrap_or_default();
            self.fade_in.resize(snap.bodies.len(), FADE);
            for t in &mut self.fade_in {
                *t = (*t + 1).min(FADE);
            }
            for g in &mut self.ghosts {
                g.1 += 1;
            }
            self.ghosts.retain(|g| g.1 < FADE);
            for (i, b) in snap.bodies.iter().enumerate() {
                let before = was.get(i).is_some_and(|x| x.active);
                if b.active
                    && !before
                    && scene
                        .bodies
                        .get(i)
                        .copied()
                        .flatten()
                        .is_some_and(|d| win.outside_view(d.x, width(&d), view_w))
                {
                    self.fade_in[i] = 0;
                }
                if !b.active
                    && before
                    && let Some(Some(d)) = self.draws.get(i)
                    && win.outside_view(d.x, width(d), view_w)
                {
                    self.ghosts.push((*d, 0));
                }
            }
            let target = colors_of(scene);
            if self.colors.len() == target.len() {
                for (c, t) in self.colors.iter_mut().zip(&target) {
                    *c = ease(*c, *t);
                }
            } else {
                self.colors = target;
            }
        } else {
            self.fade_in = vec![FADE; snap.bodies.len()];
            self.ghosts.clear();
            self.colors = colors_of(scene);
        }
        self.draws = scene.bodies.clone();
        self.last = self.now.take();
        self.now = Some(snap);
    }

    /// The frame to draw at `t` parts of [`SUB`] from the last tick to this
    /// one (`SUB` for this tick's place, which is what a held game or one
    /// without smooth motion draws). `scene` is this tick's.
    pub fn frame(&self, p: &Present, spec: &Spec, w: &World, scene: &Scene, t: i64) -> Framed {
        let mut s = scene.clone();
        if self.options.dim_flashes && self.colors.len() == colors_of(&s).len() {
            let mut c = self.colors.iter().copied();
            s.clear = c.next().unwrap_or(s.clear);
            for x in &mut s.tints {
                x.1 = c.next().unwrap_or(x.1);
            }
            for x in &mut s.hud_tints {
                x.1 = c.next().unwrap_or(x.1);
            }
        }
        let (level_w, wall) = level_edges(spec, w);
        let win = window(&p.screen, s.camera, self.options.wide, level_w, wall);
        if self.options.sprite_limit {
            limit(p, &mut s, &win, w.frame);
        }
        let alpha = (0..s.bodies.len())
            .map(|i| {
                let t = self.fade_in.get(i).copied().unwrap_or(FADE);
                (255 * t / FADE) as u8
            })
            .collect();
        let ghosts = self
            .ghosts
            .iter()
            .map(|(d, age)| (*d, (255 * (FADE - age) / FADE) as u8))
            .collect();
        let mut nudge = vec![[0, 0]; s.bodies.len()];
        let mut camera_nudge = [0, 0];
        let back = (SUB - t.clamp(0, SUB)) * ONE / SUB;
        if let (true, Some(last), Some(now)) =
            (self.options.smooth && back > 0, &self.last, &self.now)
            && last.followed_by(now)
            && now.frame == w.frame
        {
            let part = |from: i64, to: i64| -(to - from) * back / ONE * SUB / ONE;
            for (i, (a, b)) in last.bodies.iter().zip(&now.bodies).enumerate() {
                if !(a.active && b.active && a.kind == b.kind) {
                    continue;
                }
                let fits = (0..2).all(|k| {
                    let most = a.speed[k].abs().max(b.speed[k].abs()) + ONE;
                    (b.pos[k] - a.pos[k]).abs() <= most
                });
                if fits && i < nudge.len() {
                    nudge[i] = [part(a.pos[0], b.pos[0]), part(a.pos[1], b.pos[1])];
                }
            }
            // A camera moves at most its level's step, or a sixteenth of the
            // screen, in a frame; more is a cut.
            let step = spec
                .levels
                .get(w.level)
                .and_then(|l| l.camera.as_ref())
                .and_then(|c| c.step)
                .map_or(i64::from(p.screen.size[0]) / 16 * ONE, |s| s.raw() + ONE);
            if (0..2).all(|k| (now.camera[k] - last.camera[k]).abs() <= step) {
                camera_nudge = [
                    part(last.camera[0], now.camera[0]),
                    part(last.camera[1], now.camera[1]),
                ];
            }
        }
        Framed {
            scene: s,
            window: win,
            alpha,
            ghosts,
            nudge,
            camera_nudge,
        }
    }
}

#[cfg(test)]
#[path = "view_tests.rs"]
mod tests;
