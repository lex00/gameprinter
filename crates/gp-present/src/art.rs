//! Large-art looks: a look whose pictures are drawn bigger than the game's
//! own, at the place and size of the picture each stands for, with a few
//! drawing rules (docs/formats/theme.md, "Large art"). The game never sees
//! any of it: bodies keep their places and boxes, and only how each is
//! drawn changes.
//!
//! A look names its rules file with a line `art <file>` in `look.txt`. The
//! rules (`gp-art 1`, one per line, `#` starts a note) name the build's
//! pictures by file name, as `present.txt` does, levels by their level
//! picture's file name, and the look's own art files by their paths in the
//! look's folder. Sizes and places are in game pixels unless a line says
//! otherwise.
//!
//! - `scale N`: canvas pixels per game pixel, so the art keeps its detail.
//! - `limit off`: drawn without the console's sprite limit (the art isn't
//!   the console's 8-pixel sprites).
//! - `pic BUILD ART W H DX DY`: the build's picture `BUILD` is drawn as
//!   `ART`, `W` by `H`, moved by `DX`, `DY` from where the build's goes.
//! - `timed BUILD FROM TO ART W H DX DY [every=N] [after=BUILD2] [wait=N]
//!   [hide]`: while a body has shown `BUILD` for `FROM` to `TO` frames
//!   (or, with `every`, at that point of a repeat of `N` frames on the
//!   look's clock), it shows `ART` instead; with `after`, only when the
//!   picture before the last was `BUILD2`; with `wait`, only once it has
//!   shown `BUILD` that long. The first rule of a picture that holds wins.
//!   `hide`: the pose holds something, and the `held` pictures by it
//!   aren't drawn.
//! - `held BUILD...`: not drawn within 48 by 38 pixels of a body in a
//!   `hide` pose (a ball in hand).
//! - `face BUILD...`: turned the way the body really moves, once it has
//!   gone `face_turn N` pixels (default 6) the new way, so the game's
//!   small back-and-forth doesn't turn it.
//! - `near ART W H DX DY`, `near_when BUILD...`, `near_other BUILD...`: a
//!   body showing a `near_when` picture shows `ART` while a body showing a
//!   `near_other` picture stands level with it, turned toward that body.
//! - `elsewhere LEVEL DX DY BUILD...`: those pictures move by `DX`, `DY`
//!   on every level but `LEVEL`.
//! - `prop LEVEL ART X Y W H`, then `prop_over BUILD...` and `prop_burst
//!   BUILD...`: on `LEVEL`, `ART` is drawn over the `prop_over` bodies
//!   where they pass behind its box; one that lands on it isn't drawn, and
//!   the `prop_burst` pictures play there, 7 frames each, growing.
//! - `drift LEVEL Y RISE EVERY`, then `drift_art ART W H` (W and H in
//!   sixteenths of a pixel) and `drift_at X PHASE` (`PHASE` in thousandths
//!   of a rise): on `LEVEL`, the arts rise `RISE` pixels from `Y` at each
//!   `X`, one rise every `EVERY` frames, fading in and out, swaying.
//!
//! A [`Painter`] steps once after each game frame, keeping what the rules
//! need from frame to frame. [`Painter::frame`] turns a framed scene into
//! an [`ArtFrame`]: every picture to paint, in order, placed in the window.
//! Every runtime paints that list and keeps no rules of its own, so the web
//! page and the native player show the same frame. Whole numbers only.

use crate::view::Framed;
use crate::{Draw, Present, Scene};

pub mod stage;
use stage::{Stage, StageRules, StageSrc};

/// The rules file's first line.
pub const ART_HEADER: &str = "gp-art 1";

/// Parts of a game pixel in an [`ArtDraw`]'s place and size.
pub const ART_SUB: i64 = 16;

/// An art file at a size and an offset, in game pixels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sized {
    pub art: String,
    pub size: [i64; 2],
    pub offset: [i64; 2],
}

/// One picture a large-art look draws in place of one of the build's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtPic {
    /// The build's picture, by file name.
    pub build: String,
    pub art: Sized,
}

/// Another picture while a body has shown one for a time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Timed {
    pub build: String,
    pub from: u64,
    pub to: u64,
    pub art: Sized,
    /// A repeat on the look's clock, 0 for none.
    pub every: u64,
    pub after: Option<String>,
    pub wait: u64,
    pub hide: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Near {
    pub art: Sized,
    pub when: Vec<String>,
    pub other: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Elsewhere {
    pub level: String,
    pub offset: [i64; 2],
    pub builds: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prop {
    pub level: String,
    pub art: String,
    /// x, y, w, h.
    pub rect: [i64; 4],
    pub over: Vec<String>,
    pub burst: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drift {
    pub level: String,
    pub y: i64,
    pub rise: i64,
    pub every: u64,
    /// Each art and its size in sixteenths of a pixel.
    pub arts: Vec<(String, [i64; 2])>,
    /// Each place's x and phase in thousandths.
    pub at: Vec<(i64, i64)>,
}

/// A large-art look's rules, as read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtRules {
    pub scale: u32,
    /// Whether the console's sprite limit is drawn.
    pub limit: bool,
    pub pics: Vec<ArtPic>,
    pub timed: Vec<Timed>,
    pub held: Vec<String>,
    pub face: Vec<String>,
    pub face_turn: i64,
    pub near: Option<Near>,
    pub elsewhere: Vec<Elsewhere>,
    pub props: Vec<Prop>,
    pub drift: Option<Drift>,
    /// The stages built from pieces (`stage`), or None.
    pub stage: Option<StageRules>,
}

impl Default for ArtRules {
    fn default() -> Self {
        ArtRules {
            scale: 1,
            limit: true,
            pics: Vec::new(),
            timed: Vec::new(),
            held: Vec::new(),
            face: Vec::new(),
            face_turn: 6,
            near: None,
            elsewhere: Vec::new(),
            props: Vec::new(),
            drift: None,
            stage: None,
        }
    }
}

fn inside(f: &str) -> bool {
    !f.is_empty() && !f.starts_with('/') && !f.split('/').any(|p| p == "..")
}

fn names(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| (*s).to_owned()).collect()
}

impl ArtRules {
    pub fn read(text: &str) -> Result<Self, String> {
        let mut lines = text.lines();
        if lines.next().map(str::trim) != Some(ART_HEADER) {
            return Err(format!("not large-art rules (no `{ART_HEADER}` line)"));
        }
        let mut r = ArtRules::default();
        for (n, line) in lines.enumerate() {
            let line = line.trim();
            let bad = || format!("art rules line {}: `{line}`", n + 2);
            let int = |s: &str| s.parse::<i64>().map_err(|_| bad());
            let count = |s: &str| s.parse::<u64>().map_err(|_| bad());
            let art = |a: &str, w: &str, h: &str, dx: &str, dy: &str| -> Result<Sized, String> {
                let size = [int(w)?, int(h)?];
                if size[0] <= 0 || size[1] <= 0 || !inside(a) {
                    return Err(bad());
                }
                Ok(Sized {
                    art: a.to_owned(),
                    size,
                    offset: [int(dx)?, int(dy)?],
                })
            };
            let w: Vec<&str> = line.split_whitespace().collect();
            match w.as_slice() {
                [] => {}
                [c, ..] if c.starts_with('#') => {}
                ["scale", s] => {
                    r.scale = s
                        .parse()
                        .ok()
                        .filter(|s| (1..=16).contains(s))
                        .ok_or_else(bad)?;
                }
                ["limit", "on"] => r.limit = true,
                ["limit", "off"] => r.limit = false,
                ["pic", b, a, ww, h, dx, dy] => r.pics.push(ArtPic {
                    build: (*b).to_owned(),
                    art: art(a, ww, h, dx, dy)?,
                }),
                ["timed", b, from, to, a, ww, h, dx, dy, rest @ ..] => {
                    let mut t = Timed {
                        build: (*b).to_owned(),
                        from: count(from)?,
                        to: count(to)?,
                        art: art(a, ww, h, dx, dy)?,
                        every: 0,
                        after: None,
                        wait: 0,
                        hide: false,
                    };
                    for x in rest {
                        match x.split_once('=') {
                            Some(("every", v)) => t.every = count(v)?,
                            Some(("after", v)) => t.after = Some(v.to_owned()),
                            Some(("wait", v)) => t.wait = count(v)?,
                            None if *x == "hide" => t.hide = true,
                            _ => return Err(bad()),
                        }
                    }
                    r.timed.push(t);
                }
                ["held", b @ ..] => r.held.extend(names(b)),
                ["face", b @ ..] => r.face.extend(names(b)),
                ["face_turn", v] => r.face_turn = int(v)?.max(1),
                ["near", a, ww, h, dx, dy] => {
                    r.near = Some(Near {
                        art: art(a, ww, h, dx, dy)?,
                        when: Vec::new(),
                        other: Vec::new(),
                    });
                }
                ["near_when", b @ ..] => r.near.as_mut().ok_or_else(bad)?.when.extend(names(b)),
                ["near_other", b @ ..] => r.near.as_mut().ok_or_else(bad)?.other.extend(names(b)),
                ["elsewhere", level, dx, dy, b @ ..] => r.elsewhere.push(Elsewhere {
                    level: (*level).to_owned(),
                    offset: [int(dx)?, int(dy)?],
                    builds: names(b),
                }),
                ["prop", level, a, x, y, ww, h] => {
                    if !inside(a) {
                        return Err(bad());
                    }
                    r.props.push(Prop {
                        level: (*level).to_owned(),
                        art: (*a).to_owned(),
                        rect: [int(x)?, int(y)?, int(ww)?, int(h)?],
                        over: Vec::new(),
                        burst: Vec::new(),
                    });
                }
                ["prop_over", b @ ..] => r.props.last_mut().ok_or_else(bad)?.over.extend(names(b)),
                ["prop_burst", b @ ..] => {
                    r.props.last_mut().ok_or_else(bad)?.burst.extend(names(b))
                }
                ["drift", level, y, rise, every] => {
                    r.drift = Some(Drift {
                        level: (*level).to_owned(),
                        y: int(y)?,
                        rise: int(rise)?,
                        every: count(every)?.max(1),
                        arts: Vec::new(),
                        at: Vec::new(),
                    });
                }
                ["drift_art", a, ww, h] => {
                    if !inside(a) {
                        return Err(bad());
                    }
                    r.drift
                        .as_mut()
                        .ok_or_else(bad)?
                        .arts
                        .push(((*a).to_owned(), [int(ww)?, int(h)?]));
                }
                ["drift_at", x, phase] => r
                    .drift
                    .as_mut()
                    .ok_or_else(bad)?
                    .at
                    .push((int(x)?, int(phase)?)),
                w => {
                    if !StageRules::line(&mut r.stage, w).map_err(|()| bad())? {
                        return Err(bad());
                    }
                }
            }
        }
        Ok(r)
    }

    /// The art files, each once, in the order [`Src::Art`] numbers them.
    #[must_use]
    pub fn files(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut add = |f: &str| {
            if !out.iter().any(|x| x == f) {
                out.push(f.to_owned());
            }
        };
        self.pics.iter().for_each(|p| add(&p.art.art));
        self.timed.iter().for_each(|t| add(&t.art.art));
        if let Some(n) = &self.near {
            add(&n.art.art);
        }
        self.props.iter().for_each(|p| add(&p.art));
        if let Some(d) = &self.drift {
            d.arts.iter().for_each(|(a, _)| add(a));
        }
        if let Some(st) = &self.stage {
            st.pieces.iter().for_each(|(_, a, _)| add(a));
        }
        out
    }
}

/// An art file by its [`Art::files`] number, its size and its offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Placed {
    file: u32,
    size: [i64; 2],
    offset: [i64; 2],
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TimedAt {
    from: u64,
    to: u64,
    every: u64,
    after: Option<u32>,
    wait: u64,
    hide: bool,
    art: Placed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PropAt {
    level: Option<u32>,
    file: u32,
    rect: [i64; 4],
    over: Vec<bool>,
    burst: Vec<u32>,
}

/// A look's rules resolved against a presentation: pictures and levels by
/// the presentation's numbers, art by [`Art::files`] numbers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Art {
    pub rules: ArtRules,
    /// The art files, numbered as [`Src::Art`] names them.
    pub files: Vec<String>,
    pics: Vec<Option<Placed>>,
    timed: Vec<Vec<TimedAt>>,
    held: Vec<bool>,
    face: Vec<bool>,
    near: Option<(Placed, Vec<bool>, Vec<bool>)>,
    elsewhere: Vec<(Option<u32>, [i64; 2], Vec<bool>)>,
    props: Vec<PropAt>,
    drift: Option<DriftAt>,
    /// By the build's picture: the stage built over that level picture
    /// ([`Art::build_stage`]), its pieces as [`Art::files`] numbers.
    stages: Vec<Option<Stage>>,
    /// By the build's picture: a changed cell drawn as the backdrop.
    holes: Vec<bool>,
}

/// The drift's level and each art's number and size.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DriftAt {
    level: Option<u32>,
    arts: Vec<(u32, [i64; 2])>,
}

impl Art {
    /// Resolves `rules` against `p`'s pictures. A rule naming a picture the
    /// build hasn't got is left out (one dump of a game can lack a glyph
    /// another has); a look with none of the build's pictures is refused,
    /// as one made for another game.
    pub fn new(rules: ArtRules, p: &Present) -> Result<Self, String> {
        let files = rules.files();
        let n = p.pictures.len();
        // A picture this build hasn't got (a glyph one dump of the game
        // lacks) has no rule here: the look is drawn without it.
        let pic = |name: &str| -> Option<u32> {
            p.pictures
                .iter()
                .position(|x| x.file == name)
                .map(|i| i as u32)
        };
        let file = |name: &str| files.iter().position(|f| f == name).unwrap_or(0) as u32;
        let placed = |s: &Sized| Placed {
            file: file(&s.art),
            size: s.size,
            offset: s.offset,
        };
        let set = |list: &[String]| -> Result<Vec<bool>, String> {
            let mut v = vec![false; n];
            for i in list.iter().filter_map(|b| pic(b)) {
                v[i as usize] = true;
            }
            Ok(v)
        };
        // A level not in this build (another game's) matches none.
        let level = |name: &str| pic(name);
        let mut pics = vec![None; n];
        for r in &rules.pics {
            if let Some(i) = pic(&r.build) {
                pics[i as usize] = Some(placed(&r.art));
            }
        }
        if !rules.pics.is_empty() && pics.iter().all(Option::is_none) {
            return Err("the build has none of the pictures the look draws".into());
        }
        let mut timed = vec![Vec::new(); n];
        for t in &rules.timed {
            let Some(i) = pic(&t.build) else { continue };
            // After a picture the build hasn't got: never.
            let after = t.after.as_ref().map(|a| pic(a).unwrap_or(u32::MAX));
            timed[i as usize].push(TimedAt {
                from: t.from,
                to: t.to,
                every: t.every,
                after,
                wait: t.wait,
                hide: t.hide,
                art: placed(&t.art),
            });
        }
        let near = match &rules.near {
            Some(x) => Some((placed(&x.art), set(&x.when)?, set(&x.other)?)),
            None => None,
        };
        let mut elsewhere = Vec::new();
        for e in &rules.elsewhere {
            elsewhere.push((level(&e.level), e.offset, set(&e.builds)?));
        }
        let mut props = Vec::new();
        for pr in &rules.props {
            props.push(PropAt {
                level: level(&pr.level),
                file: file(&pr.art),
                rect: pr.rect,
                over: set(&pr.over)?,
                burst: pr.burst.iter().filter_map(|b| pic(b)).collect(),
            });
        }
        let drift = rules.drift.as_ref().map(|d| DriftAt {
            level: level(&d.level),
            arts: d.arts.iter().map(|(a, s)| (file(a), *s)).collect(),
        });
        let holes = p
            .pictures
            .iter()
            .map(|x| {
                rules
                    .stage
                    .as_ref()
                    .and_then(|st| st.holes.as_ref())
                    .is_some_and(|h| x.file.ends_with(h.as_str()))
            })
            .collect();
        Ok(Art {
            stages: vec![None; n],
            holes,
            held: set(&rules.held)?,
            face: set(&rules.face)?,
            files,
            pics,
            timed,
            near,
            elsewhere,
            props,
            drift,
            rules,
        })
    }

    fn is(v: &[bool], pic: u32) -> bool {
        v.get(pic as usize).copied().unwrap_or(false)
    }

    /// The build's level pictures the look builds a stage over, by number:
    /// a runtime gives [`Art::build_stage`] each one's pixels.
    #[must_use]
    pub fn stage_levels(&self, p: &Present) -> Vec<u32> {
        let Some(st) = &self.rules.stage else {
            return Vec::new();
        };
        st.levels
            .iter()
            .filter_map(|l| {
                p.pictures
                    .iter()
                    .position(|x| x.file == l.level)
                    .map(|i| i as u32)
            })
            .collect()
    }

    /// Builds the stage over the level picture `pic` from its pixels
    /// (`w` by `h`, RGBA row by row), and keeps it for the frames.
    pub fn build_stage(
        &mut self,
        p: &Present,
        pic: u32,
        w: usize,
        h: usize,
        px: &[[u8; 4]],
    ) -> Result<&Stage, String> {
        let st = self
            .rules
            .stage
            .as_ref()
            .ok_or("the look builds no stages")?;
        let file = &p.pictures.get(pic as usize).ok_or("no such picture")?.file;
        let l = st
            .levels
            .iter()
            .find(|l| l.level == *file)
            .ok_or_else(|| format!("the look builds no stage over `{file}`"))?;
        if px.len() != w * h {
            return Err(format!("{} pixels for a {w} by {h} picture", px.len()));
        }
        let mut built = stage::build(st, l, w, h, px);
        for d in &mut built.draws {
            if let StageSrc::Piece(i) = d.src {
                let art = &st.pieces[i as usize].1;
                d.src =
                    StageSrc::Piece(self.files.iter().position(|f| f == art).unwrap_or(0) as u32);
            }
        }
        self.stages[pic as usize] = Some(built);
        Ok(self.stages[pic as usize].as_ref().expect("just built"))
    }

    /// The stage built over level picture `pic`, if any.
    #[must_use]
    pub fn stage(&self, pic: u32) -> Option<&Stage> {
        self.stages.get(pic as usize).and_then(Option::as_ref)
    }
}

/// Where a draw's picture comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Src {
    /// The build's picture (or the look's, under the same name), by the
    /// presentation's number.
    Own(u32),
    /// The look's art file, by [`Art::files`] number.
    Art(u32),
    /// The build's picture as a mask, filled with a color.
    Tint(u32, [u8; 3]),
    /// A box filled with a color (the dimmed wall).
    Fill([u8; 3]),
    /// The stage built over a level picture (by its number), painted once
    /// from [`Stage`]'s draws.
    Stage(u32),
    /// The stage's backdrop over the box ([`ArtFrame::backdrop`]).
    Backdrop,
}

/// One picture painted. Place and size are in [`ART_SUB`] parts of a game
/// pixel from the window's top left; the canvas has [`ArtRules::scale`]
/// pixels to each game pixel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtDraw {
    pub src: Src,
    pub x: i64,
    pub y: i64,
    pub w: i64,
    pub h: i64,
    pub flip_x: bool,
    pub flip_y: bool,
    /// 0 to 255.
    pub alpha: u8,
    /// Parts not painted (the console's sprite limit), `[x, y, w, h]` in
    /// game pixels from the picture's top left, before any flip.
    pub cut: Vec<[i64; 4]>,
    /// Painted only inside this box, `[x, y, w, h]` like the place.
    pub clip: Option<[i64; 4]>,
}

impl ArtDraw {
    fn new(src: Src, x: i64, y: i64, w: i64, h: i64) -> Self {
        ArtDraw {
            src,
            x,
            y,
            w,
            h,
            flip_x: false,
            flip_y: false,
            alpha: 255,
            cut: Vec::new(),
            clip: None,
        }
    }
}

/// A frame of a large-art look: the window's size in game pixels and the
/// clear color, then every picture in the order painted, the last on top.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArtFrame {
    pub size: [u32; 2],
    pub scale: u32,
    pub clear: [u8; 3],
    pub draws: Vec<ArtDraw>,
    /// The stage's backdrop: its two colors, from the top of the level to
    /// its bottom (y in sixteenths, like the draws).
    pub backdrop: Option<([u8; 3], [u8; 3], i64, i64)>,
}

/// What a body showed: its build picture and since when, and the pictures
/// before.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Age {
    pic: Option<u32>,
    since: u64,
    /// The last pictures before `pic`, the latest last, up to three.
    before: Vec<Option<u32>>,
}

/// Which way a `face` body looks and where it last turned.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Facing {
    seen: bool,
    at: [i64; 2],
    right: bool,
    anchor: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Burst {
    prop: usize,
    x: i64,
    y: i64,
    start: u64,
}

/// What a large-art look keeps from frame to frame: a clock, each body's
/// picture age and facing, and the props' bursts. Presentation only.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Painter {
    clock: u64,
    ages: Vec<Age>,
    facing: Vec<Facing>,
    /// By prop, by body: on it last frame.
    on_prop: Vec<Vec<bool>>,
    bursts: Vec<Burst>,
}

/// Bodies' boxes for the props and the near rule: the build's 16 by 16.
const BODY: i64 = 16;

impl Painter {
    #[must_use]
    pub fn new() -> Self {
        Painter::default()
    }

    /// Steps after a game frame, with that frame's scene.
    pub fn step(&mut self, art: &Art, s: &Scene) {
        self.clock += 1;
        let now = self.clock;
        let n = s.bodies.len();
        self.ages.resize(n, Age::default());
        self.facing.resize(n, Facing::default());
        for (i, b) in s.bodies.iter().enumerate() {
            let a = &mut self.ages[i];
            let pic = b.map(|d| d.pic);
            if b.is_some() && a.pic != pic {
                a.before.push(a.pic);
                if a.before.len() > 3 {
                    a.before.remove(0);
                }
                a.pic = pic;
                a.since = now;
            }
            if let Some(d) = b
                && Art::is(&art.face, d.pic)
            {
                let f = &mut self.facing[i];
                turn(f, [d.x, d.y], art.rules.face_turn);
            }
        }
        // The props: a body that comes onto one bursts there.
        self.on_prop.resize(art.props.len(), Vec::new());
        for (k, pr) in art.props.iter().enumerate() {
            let on = &mut self.on_prop[k];
            on.resize(n, false);
            for (i, b) in s.bodies.iter().enumerate() {
                let hit = s.level.is_some()
                    && s.level == pr.level
                    && b.is_some_and(|d| Art::is(&pr.over, d.pic) && on_prop(pr, &d));
                if hit && !on[i] {
                    let d = b.expect("hit has a body");
                    self.bursts.push(Burst {
                        prop: k,
                        x: d.x + BODY / 2,
                        y: pr.rect[1] + 2,
                        start: now,
                    });
                }
                on[i] = hit;
            }
        }
        self.bursts.retain(|b| {
            let len = art.props[b.prop].burst.len() as u64;
            (now - b.start) / 7 < len
        });
    }

    /// The picture body `i` shows this frame: a timed one, or its own.
    fn shown(&self, art: &Art, i: usize, pic: u32) -> (Option<Placed>, bool) {
        let base = art.pics.get(pic as usize).copied().flatten();
        let Some(rules) = art.timed.get(pic as usize).filter(|r| !r.is_empty()) else {
            return (base, false);
        };
        let a = self.ages.get(i).filter(|a| a.pic == Some(pic));
        let age = a.map_or(0, |a| self.clock - a.since);
        let before = a.and_then(|a| a.before.iter().rev().nth(1).copied().flatten());
        for r in rules {
            if r.wait > 0 && age < r.wait {
                continue;
            }
            if r.after.is_some() && r.after != before {
                continue;
            }
            let g = if r.every > 0 {
                self.clock % r.every
            } else {
                age
            };
            if g >= r.from && g < r.to {
                return (Some(r.art), r.hide);
            }
        }
        (base, false)
    }

    /// The frame `f` (the window `height` tall) drawn in the look: the
    /// order the page paints a scene in (behind-the-level bodies, the level
    /// and the drift over it, its tints, changed cells, the HUD's
    /// background, the bodies, those in front, fading ghosts, the dimmed
    /// wall, the props and their bursts, the HUD's sprites).
    #[must_use]
    pub fn frame(&self, art: &Art, p: &Present, f: &Framed, height: u32) -> ArtFrame {
        let s: &Scene = &f.scene;
        let win = f.window;
        let (ox, oy) = (-win.x, -win.y);
        let (hx, hy) = (win.hud[0] - win.x, win.hud[1] - win.y);
        let u = ART_SUB;
        let mut out = ArtFrame {
            size: [win.w, height],
            scale: art.rules.scale,
            clear: s.clear,
            draws: Vec::new(),
            backdrop: None,
        };
        let stage = s.level.and_then(|l| art.stage(l));
        if let Some(st) = stage {
            out.backdrop = Some((
                st.backdrop[0],
                st.backdrop[1],
                oy * u,
                (oy + st.size[1]) * u,
            ));
        }
        let own = |pic: u32| {
            p.pictures
                .get(pic as usize)
                .map_or([0, 0], |x| [i64::from(x.size[0]), i64::from(x.size[1])])
        };
        // A picture at a place in window pixels: its art, else the build's.
        let placed = |src: Option<Placed>, pic: u32, x: i64, y: i64| match src {
            Some(a) => ArtDraw::new(
                Src::Art(a.file),
                (x + a.offset[0]) * u,
                (y + a.offset[1]) * u,
                a.size[0] * u,
                a.size[1] * u,
            ),
            None => {
                let [w, h] = own(pic);
                ArtDraw::new(Src::Own(pic), x * u, y * u, w * u, h * u)
            }
        };
        let plain = |out: &mut ArtFrame, d: &Draw, x: i64, y: i64, alpha: u8| {
            let mut a = placed(art.pics.get(d.pic as usize).copied().flatten(), d.pic, x, y);
            a.flip_x = d.flip_x;
            a.flip_y = d.flip_y;
            a.alpha = alpha;
            out.draws.push(a);
        };
        // A body in a pose that holds something, and where.
        let mut hold = None;
        for (i, b) in s.bodies.iter().enumerate() {
            if let Some(d) = b
                && self.shown(art, i, d.pic).1
            {
                hold = Some([d.x, d.y]);
            }
        }
        let on_a_prop = |i: usize| {
            self.on_prop
                .iter()
                .any(|v| v.get(i).copied().unwrap_or(false))
        };
        let body = |out: &mut ArtFrame, i: usize| {
            let Some(Some(d)) = s.bodies.get(i) else {
                return;
            };
            if on_a_prop(i) {
                return;
            }
            if let Some([hx0, hy0]) = hold
                && Art::is(&art.held, d.pic)
                && d.x >= hx0
                && d.x < hx0 + 48
                && d.y >= hy0 - 4
                && d.y < hy0 + 34
            {
                return;
            }
            let (mut src, _) = self.shown(art, i, d.pic);
            let mut flip_x = d.flip_x;
            if let Some((n, when, other)) = &art.near
                && Art::is(when, d.pic)
            {
                let h = s.bodies.iter().flatten().find(|o| Art::is(other, o.pic));
                if let Some(h) = h
                    && (h.y + BODY - (d.y + 32)).abs() <= 6
                {
                    src = Some(*n);
                    flip_x = h.x + 8 < d.x + 24;
                }
            }
            if Art::is(&art.face, d.pic)
                && let Some(fc) = self.facing.get(i).filter(|f| f.seen)
            {
                flip_x = fc.right;
            }
            let mut a = placed(src, d.pic, d.x + ox, d.y + oy);
            for (level, off, builds) in &art.elsewhere {
                if s.level != *level && Art::is(builds, d.pic) {
                    a.x = (d.x + ox + off[0]) * u;
                    a.y = (d.y + oy + off[1]) * u;
                }
            }
            a.flip_x = flip_x;
            a.flip_y = d.flip_y;
            a.alpha = f.alpha.get(i).copied().unwrap_or(255);
            if a.src == Src::Own(d.pic) {
                a.cut = s.cut.get(i).cloned().unwrap_or_default();
            }
            out.draws.push(a);
        };
        let order = s.paint_order();
        let at = |v: &[bool], i: usize| v.get(i).copied().unwrap_or(false);
        for &i in &order {
            if at(&s.behind, i) {
                body(&mut out, i);
            }
        }
        if let Some(l) = s.level {
            // A stage built from pieces, else the look's picture for it (a
            // whole screen: the title), else the build's own.
            let d = match (stage, art.pics.get(l as usize).copied().flatten()) {
                (Some(_), _) => {
                    let [w, h] = own(l);
                    ArtDraw::new(Src::Stage(l), ox * u, oy * u, w * u, h * u)
                }
                (None, a) => placed(a, l, ox, oy),
            };
            out.draws.push(d);
        }
        self.drift(art, s, ox, oy, &mut out);
        for &(m, c) in &s.tints {
            let [w, h] = own(m);
            out.draws
                .push(ArtDraw::new(Src::Tint(m, c), ox * u, oy * u, w * u, h * u));
        }
        for d in &s.cells {
            // A taken rivet: the backdrop, wider beside a ladder.
            if let Some(st) = stage.filter(|_| Art::is(&art.holes, d.pic)) {
                let (x, y) = (d.x, d.y);
                let near = st
                    .ladders
                    .iter()
                    .any(|&[a, b, c, e]| x + 8 > a && x < b && y + 8 > c - 8 && y < e + 8);
                let m = if near { 3 } else { 0 };
                out.draws.push(ArtDraw::new(
                    Src::Backdrop,
                    (x - m + ox) * u,
                    (y + oy) * u,
                    (8 + 2 * m) * u,
                    8 * u,
                ));
                continue;
            }
            plain(&mut out, d, d.x + ox, d.y + oy, 255);
        }
        let hud_bg = s.hud.len() - s.hud_sprites.min(s.hud.len());
        let hud = |out: &mut ArtFrame, from: usize, to: usize| {
            for (j, d) in s.hud[from..to].iter().enumerate() {
                plain(out, d, d.x + hx, d.y + hy, 255);
                if from + j == 0 {
                    for &(m, c) in &s.hud_tints {
                        let [w, h] = own(m);
                        out.draws
                            .push(ArtDraw::new(Src::Tint(m, c), hx * u, hy * u, w * u, h * u));
                    }
                }
            }
        };
        hud(&mut out, 0, hud_bg);
        for &i in &order {
            if !at(&s.behind, i) && !at(&s.front, i) {
                body(&mut out, i);
            }
        }
        for &i in &order {
            if !at(&s.behind, i) && at(&s.front, i) {
                body(&mut out, i);
            }
        }
        for (d, a) in &f.ghosts {
            plain(&mut out, d, d.x + ox, d.y + oy, *a);
        }
        if win.wall > 0 {
            let mut d = ArtDraw::new(
                Src::Fill([0, 0, 0]),
                0,
                0,
                win.wall * u,
                i64::from(height) * u,
            );
            d.alpha = 115;
            out.draws.push(d);
        }
        self.props(art, s, ox, oy, &mut out);
        hud(&mut out, hud_bg, s.hud.len());
        out
    }

    fn drift(&self, art: &Art, s: &Scene, ox: i64, oy: i64, out: &mut ArtFrame) {
        let (Some(d), Some(DriftAt { level, arts })) = (&art.rules.drift, &art.drift) else {
            return;
        };
        if s.level.is_none() || s.level != *level || arts.is_empty() {
            return;
        }
        let now = self.clock as i64;
        let every = d.every as i64;
        for (i, &(x, phase)) in d.at.iter().enumerate() {
            // Thousandths of the way up.
            let p = (now * 1000 / every + phase).rem_euclid(1000);
            // Faded in and out: sin(pi p), at most 0.9.
            let alpha = (230 * isin(p) / 1000) as u8;
            let (file, [w, h]) = arts[i % arts.len()];
            // A sway of 1.3 pixels, about every 90 frames.
            let sway = 21 * isin(((now + i as i64 * 29) % 90) * 2000 / 90) / 1000;
            let mut a = ArtDraw::new(
                Src::Art(file),
                (x + ox) * ART_SUB + sway - w / 2,
                (d.y + oy) * ART_SUB - p * d.rise * ART_SUB / 1000 - h,
                w,
                h,
            );
            a.alpha = alpha;
            out.draws.push(a);
        }
    }

    fn props(&self, art: &Art, s: &Scene, ox: i64, oy: i64, out: &mut ArtFrame) {
        let u = ART_SUB;
        for b in &self.bursts {
            let pr = &art.props[b.prop];
            let idx = ((self.clock - b.start) / 7) as usize;
            let Some(&pic) = pr.burst.get(idx) else {
                continue;
            };
            let size = (26 + idx as i64 * 4) * u;
            let src = art
                .pics
                .get(pic as usize)
                .copied()
                .flatten()
                .map_or(Src::Own(pic), |a| Src::Art(a.file));
            out.draws.push(ArtDraw::new(
                src,
                (b.x + ox) * u - size / 2,
                (b.y + oy - 2) * u - size / 2,
                size,
                size,
            ));
        }
        for pr in &art.props {
            if s.level.is_none() || s.level != pr.level {
                continue;
            }
            let [px, py, pw, ph] = pr.rect;
            for d in s.bodies.iter().flatten() {
                if !Art::is(&pr.over, d.pic) || d.x + BODY < px || d.x > px + pw || d.y + BODY < py
                {
                    continue;
                }
                let mut a = ArtDraw::new(
                    Src::Art(pr.file),
                    (px + ox) * u,
                    (py + oy) * u,
                    pw * u,
                    ph * u,
                );
                a.clip = Some([(d.x + ox) * u, (d.y + oy) * u, BODY * u, BODY * u]);
                out.draws.push(a);
            }
        }
    }
}

/// Whether a body stands on a prop: its feet in the box's top, its middle
/// across it.
fn on_prop(pr: &PropAt, d: &Draw) -> bool {
    let [px, py, pw, _] = pr.rect;
    let cx = d.x + BODY / 2;
    d.y + BODY >= py + 4 && cx >= px && cx <= px + pw
}

/// A `face` body's turn: it looks the way it last went `turn` pixels, and
/// a jump (a new place far off) starts it over looking left.
fn turn(f: &mut Facing, at: [i64; 2], turn: i64) {
    let [x, y] = at;
    if !f.seen || (x - f.at[0]).abs() > 12 || (y - f.at[1]).abs() > 12 {
        f.right = false;
        f.anchor = x;
    }
    let dir = if f.right { 1 } else { -1 };
    if (x - f.anchor) * dir > 0 {
        f.anchor = x;
    } else if (x - f.anchor).abs() >= turn {
        f.right = !f.right;
        f.anchor = x;
    }
    f.seen = true;
    f.at = at;
}

/// sin(pi t / 1000) in thousandths, for t in thousandths of a half turn
/// (whole numbers, so every runtime sways the same).
fn isin(t: i64) -> i64 {
    const Q: [i64; 17] = [
        0, 98, 195, 290, 383, 471, 556, 634, 707, 773, 831, 882, 924, 957, 981, 995, 1000,
    ];
    let t = t.rem_euclid(2000);
    let (t, sign) = if t >= 1000 { (t - 1000, -1) } else { (t, 1) };
    // Up to the quarter, then back down.
    let q = if t > 500 { 1000 - t } else { t };
    let pos = q * 32;
    let (k, r) = ((pos / 1000) as usize, pos % 1000);
    let v = if k >= 16 {
        Q[16]
    } else {
        Q[k] + (Q[k + 1] - Q[k]) * r / 1000
    };
    sign * v
}

#[cfg(test)]
mod tests;
