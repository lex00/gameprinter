//! Large-art looks: a look whose pictures are drawn bigger than the game's
//! own, at the place and size of the picture each stands for, with a few
//! drawing rules (docs/formats/theme.md, "Large-art rules"). The game never sees
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
//! Lines added since `gp-art 1` began start with `#+`, which a reader from
//! before them takes for a note: a page built before a line draws the look
//! without it rather than refusing the look. A `#+` line this reader can't
//! read is passed over the same way ([`ArtRules::skipped`]).
//!
//! - `#+ hud_over [ROW...]`: the HUD's layer and the text in its background
//!   are drawn after the bodies (after their ghosts, before the dimmed wall
//!   and the props), in those rows of 8 pixels from the screen's top, or
//!   all of them with no row named. Without it they are drawn under the
//!   bodies, as before. The HUD's sprites are drawn last either way.
//! - `#+ in REGION pic ...` and `#+ in REGION timed ...`: a `pic` or
//!   `timed` rule that holds only while the body's picture has the middle
//!   of its bottom row in the named region of the stage. In a region, its
//!   `in` rules come first (its timed ones that hold, then its picture);
//!   elsewhere the picture's other rules, as before. The regions are named
//!   in the game's kit and given with [`Art::set_regions`]; until then no
//!   `in` rule holds.
//! - `#+ room SCREEN...`, `#+ room_cells`, `#+ room_fill`, `#+ room_run`,
//!   `#+ room_box` and `#+ room_at`: an arcade game's screens built from the stage's
//!   pieces on the cells of the screen the player's lift gives
//!   ([`room`]). The runtime gives each screen's cells with
//!   [`Art::build_room`].
//!
//! A [`Painter`] steps once after each game frame, keeping what the rules
//! need from frame to frame. [`Painter::frame`] turns a framed scene into
//! an [`ArtFrame`]: every picture to paint, in order, placed in the window.
//! Every runtime paints that list and keeps no rules of its own, so the web
//! page and the native player show the same frame. Whole numbers only.

use crate::view::Framed;
use crate::{Draw, Present, Scene};

pub mod room;
pub mod stage;
use room::RoomRules;
use stage::{Stage, StageRules, StageSrc};

/// The rules file's first line.
pub const ART_HEADER: &str = "gp-art 1";

/// What starts a line an older reader passes over as a note (`#+
/// hud_over`, `#+ in REGION pic ...`): a rule added since, which a page
/// built before it draws without.
pub const ADDED: &str = "#+";

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
    /// `#+ hud_over [ROW...]`: the HUD's layer and the text in its
    /// background drawn after the bodies, in those rows of 8 pixels from
    /// the screen's top (all of them with none named). None, the default,
    /// draws them under the bodies.
    pub hud_over: Option<Vec<i64>>,
    /// `#+ in REGION pic ...`: a picture drawn only while the body is in a
    /// named region of the stage.
    pub region_pics: Vec<(String, ArtPic)>,
    /// `#+ in REGION timed ...`, likewise.
    pub region_timed: Vec<(String, Timed)>,
    /// `#+ room SCREEN...` and the lines after it: screens built from the
    /// stage's pieces ([`room`]).
    pub rooms: Vec<RoomRules>,
    /// The `#+` lines this reader can't read, by line number, passed over
    /// as an older reader passes over every `#+` line.
    pub skipped: Vec<(usize, String)>,
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
            hud_over: None,
            region_pics: Vec::new(),
            region_timed: Vec::new(),
            rooms: Vec::new(),
            skipped: Vec::new(),
        }
    }
}

fn inside(f: &str) -> bool {
    !f.is_empty() && !f.starts_with('/') && !f.split('/').any(|p| p == "..")
}

/// A region's name as an `in` line gives it: lowercase letters, digits,
/// `-` and `_`.
#[must_use]
pub fn region_name(n: &str) -> bool {
    !n.is_empty()
        && n.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
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
            // A line an older reader passes over as a note: one this reader
            // can't read is passed over too, so the look still draws.
            if let Some(rest) = line.strip_prefix(ADDED) {
                let w: Vec<&str> = rest.split_whitespace().collect();
                if r.added(&w, n + 2).is_err() {
                    r.skipped.push((n + 2, line.to_owned()));
                }
                continue;
            }
            let w: Vec<&str> = line.split_whitespace().collect();
            r.rule(&w)
                .map_err(|()| format!("art rules line {}: `{line}`", n + 2))?;
        }
        Ok(r)
    }

    /// A `#+` line's words: `hud_over [ROW...]`, `in REGION` and a `pic`
    /// or `timed` line, or a room's line; `line` is its number.
    fn added(&mut self, w: &[&str], line: usize) -> Result<(), ()> {
        if RoomRules::line(&mut self.rooms, self.stage.as_ref(), w, line)? {
            return Ok(());
        }
        match w {
            ["hud_over"] => self.hud_over = Some(Vec::new()),
            ["hud_over", rows @ ..] => {
                let mut v = Vec::new();
                for x in rows {
                    v.push(
                        x.parse::<i64>()
                            .ok()
                            .filter(|r| (0..64).contains(r))
                            .ok_or(())?,
                    );
                }
                match &mut self.hud_over {
                    Some(all) if all.is_empty() => {}
                    Some(had) => had.extend(v),
                    None => self.hud_over = Some(v),
                }
                if let Some(rows) = &mut self.hud_over {
                    rows.sort_unstable();
                    rows.dedup();
                }
            }
            ["in", region, rest @ ..] if region_name(region) => {
                let mut one = ArtRules::default();
                match rest.first() {
                    Some(&"pic") => {
                        one.rule(rest)?;
                        let p = one.pics.pop().ok_or(())?;
                        self.region_pics.push(((*region).to_owned(), p));
                    }
                    Some(&"timed") => {
                        one.rule(rest)?;
                        let t = one.timed.pop().ok_or(())?;
                        self.region_timed.push(((*region).to_owned(), t));
                    }
                    _ => return Err(()),
                }
            }
            _ => return Err(()),
        }
        Ok(())
    }

    /// One line's words, read into the rules.
    fn rule(&mut self, w: &[&str]) -> Result<(), ()> {
        let r = self;
        let int = |s: &str| s.parse::<i64>().map_err(|_| ());
        let count = |s: &str| s.parse::<u64>().map_err(|_| ());
        let art = |a: &str, w: &str, h: &str, dx: &str, dy: &str| -> Result<Sized, ()> {
            let size = [int(w)?, int(h)?];
            if size[0] <= 0 || size[1] <= 0 || !inside(a) {
                return Err(());
            }
            Ok(Sized {
                art: a.to_owned(),
                size,
                offset: [int(dx)?, int(dy)?],
            })
        };
        match w {
            [] => {}
            [c, ..] if c.starts_with('#') => {}
            ["scale", s] => {
                r.scale = s.parse().ok().filter(|s| (1..=16).contains(s)).ok_or(())?;
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
                        _ => return Err(()),
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
            ["near_when", b @ ..] => r.near.as_mut().ok_or(())?.when.extend(names(b)),
            ["near_other", b @ ..] => r.near.as_mut().ok_or(())?.other.extend(names(b)),
            ["elsewhere", level, dx, dy, b @ ..] => r.elsewhere.push(Elsewhere {
                level: (*level).to_owned(),
                offset: [int(dx)?, int(dy)?],
                builds: names(b),
            }),
            ["prop", level, a, x, y, ww, h] => {
                if !inside(a) {
                    return Err(());
                }
                r.props.push(Prop {
                    level: (*level).to_owned(),
                    art: (*a).to_owned(),
                    rect: [int(x)?, int(y)?, int(ww)?, int(h)?],
                    over: Vec::new(),
                    burst: Vec::new(),
                });
            }
            ["prop_over", b @ ..] => r.props.last_mut().ok_or(())?.over.extend(names(b)),
            ["prop_burst", b @ ..] => r.props.last_mut().ok_or(())?.burst.extend(names(b)),
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
                    return Err(());
                }
                r.drift
                    .as_mut()
                    .ok_or(())?
                    .arts
                    .push(((*a).to_owned(), [int(ww)?, int(h)?]));
            }
            ["drift_at", x, phase] => r.drift.as_mut().ok_or(())?.at.push((int(x)?, int(phase)?)),
            w => {
                if !StageRules::line(&mut r.stage, w)? {
                    return Err(());
                }
            }
        }
        Ok(())
    }

    /// The regions the `in` lines name, each once, in the order given.
    #[must_use]
    pub fn regions(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        let names = self.region_pics.iter().map(|(r, _)| r.as_str());
        for r in names.chain(self.region_timed.iter().map(|(r, _)| r.as_str())) {
            if !out.contains(&r) {
                out.push(r);
            }
        }
        out
    }

    /// Every name of a game's picture or level the rules give, each once,
    /// in the order the rules give them: what [`Art::new`] looks up in the
    /// build's pictures.
    #[must_use]
    pub fn game_names(&self) -> Vec<&str> {
        let mut named: Vec<&str> = Vec::new();
        named.extend(self.pics.iter().map(|p| p.build.as_str()));
        named.extend(self.region_pics.iter().map(|(_, p)| p.build.as_str()));
        for t in self
            .timed
            .iter()
            .chain(self.region_timed.iter().map(|(_, t)| t))
        {
            named.push(&t.build);
            if let Some(a) = &t.after {
                named.push(a);
            }
        }
        named.extend(self.held.iter().map(String::as_str));
        named.extend(self.face.iter().map(String::as_str));
        if let Some(n) = &self.near {
            named.extend(n.when.iter().map(String::as_str));
            named.extend(n.other.iter().map(String::as_str));
        }
        for e in &self.elsewhere {
            named.push(&e.level);
            named.extend(e.builds.iter().map(String::as_str));
        }
        for p in &self.props {
            named.push(&p.level);
            named.extend(p.over.iter().map(String::as_str));
            named.extend(p.burst.iter().map(String::as_str));
        }
        if let Some(d) = &self.drift {
            named.push(&d.level);
        }
        if let Some(st) = &self.stage {
            named.extend(st.levels.iter().map(|l| l.level.as_str()));
        }
        for r in &self.rooms {
            named.extend(r.screens.iter().map(String::as_str));
        }
        let mut seen = std::collections::BTreeSet::new();
        named.retain(|n| seen.insert(*n));
        named
    }

    /// The names the rules give that a build whose pictures are `pictures`
    /// hasn't got: [`Art::new`] passes over each rule naming one, and the
    /// look is drawn without it.
    #[must_use]
    pub fn passed_over<'a>(&'a self, pictures: &[&str]) -> Vec<&'a str> {
        self.game_names()
            .into_iter()
            .filter(|n| !pictures.contains(n))
            .collect()
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
        // Last, so a look without them numbers its files as before.
        self.region_pics.iter().for_each(|(_, p)| add(&p.art.art));
        self.region_timed.iter().for_each(|(_, t)| add(&t.art.art));
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

/// A region of a stage a `#+ in` line names: its level, by the level
/// picture's file name, and its box in the level's pixels. The game's kit
/// names it; the runtime gets it with [`Art::set_regions`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Region {
    pub name: String,
    pub level: String,
    /// x, y, w, h.
    pub rect: [i64; 4],
}

/// A region's level picture and box, once the kit's regions are given.
type RegionAt = Option<(Option<u32>, [i64; 4])>;

/// A band of the HUD's rows, its top and bottom in screen pixels.
type Band = (i64, i64);

/// A rule that holds only in a region, by its number in [`Art::regions`].
#[derive(Debug, Clone, PartialEq, Eq)]
enum InRegion {
    Pic(Placed),
    Timed(TimedAt),
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
    /// By the build's picture: a level's HUD layer.
    hud_layers: Vec<bool>,
    /// By the build's picture: its size in pixels.
    sizes: Vec<[i64; 2]>,
    /// By the build's picture: its `#+ in` rules, each with its region's
    /// number, in the order given.
    in_region: Vec<Vec<(usize, InRegion)>>,
    /// The regions the rules name, by number: each one's level and box
    /// once [`Art::set_regions`] gives it, else None (its rules never
    /// hold).
    regions: Vec<(String, RegionAt)>,
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
        let regions: Vec<(String, RegionAt)> = rules
            .regions()
            .into_iter()
            .map(|r| (r.to_owned(), None))
            .collect();
        let region = |r: &str| regions.iter().position(|x| x.0 == r).unwrap_or(0);
        let mut in_region = vec![Vec::new(); n];
        for (r, t) in &rules.region_timed {
            let Some(i) = pic(&t.build) else { continue };
            let after = t.after.as_ref().map(|a| pic(a).unwrap_or(u32::MAX));
            in_region[i as usize].push((
                region(r),
                InRegion::Timed(TimedAt {
                    from: t.from,
                    to: t.to,
                    every: t.every,
                    after,
                    wait: t.wait,
                    hide: t.hide,
                    art: placed(&t.art),
                }),
            ));
        }
        for (r, a) in &rules.region_pics {
            if let Some(i) = pic(&a.build) {
                in_region[i as usize].push((region(r), InRegion::Pic(placed(&a.art))));
            }
        }
        let mut hud_layers = vec![false; n];
        for l in p.levels.iter().filter_map(|l| l.hud_layer) {
            if let Some(v) = hud_layers.get_mut(l as usize) {
                *v = true;
            }
        }
        let sizes = p
            .pictures
            .iter()
            .map(|x| [i64::from(x.size[0]), i64::from(x.size[1])])
            .collect();
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
            hud_layers,
            sizes,
            in_region,
            regions,
            rules,
        })
    }

    /// Gives the regions the game's kit names: a `#+ in` line whose region
    /// isn't among them never holds, so the body shows its other pictures.
    pub fn set_regions(&mut self, p: &Present, regions: &[Region]) {
        for (name, at) in &mut self.regions {
            *at = regions.iter().find(|r| r.name == *name).map(|r| {
                let level = p
                    .pictures
                    .iter()
                    .position(|x| x.file == r.level)
                    .map(|i| i as u32);
                (level, r.rect)
            });
        }
    }

    /// Whether body `d` is in region `k` on level `level`: the middle of
    /// its picture's bottom row inside the region's box.
    fn in_region(&self, k: usize, d: &Draw, level: Option<u32>) -> bool {
        let Some((Some(l), [x, y, w, h])) = self.regions.get(k).and_then(|r| r.1) else {
            return false;
        };
        let [pw, ph] = self.sizes.get(d.pic as usize).copied().unwrap_or([16, 16]);
        let (cx, by) = (d.x + pw / 2, d.y + ph - 1);
        level == Some(l) && cx >= x && cx < x + w && by >= y && by < y + h
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

    /// The screens the look builds rooms of (`#+ room`), by picture
    /// number, each with its room's number in [`ArtRules::rooms`]: a
    /// runtime gives [`Art::build_room`] each one's cells. A screen two
    /// rooms name is the first's.
    #[must_use]
    pub fn room_screens(&self, p: &Present) -> Vec<(u32, usize)> {
        if self.rules.stage.is_none() {
            return Vec::new();
        }
        let mut out: Vec<(u32, usize)> = Vec::new();
        for (k, r) in self.rules.rooms.iter().enumerate() {
            for s in &r.screens {
                if let Some(i) = p.pictures.iter().position(|x| x.file == *s)
                    && !out.iter().any(|(pic, _)| *pic == i as u32)
                {
                    out.push((i as u32, k));
                }
            }
        }
        out
    }

    /// Builds the room over screen picture `pic` from its cells (the
    /// player's lift's) and keeps it for the frames, drawn as the stage
    /// over that picture. Gives the cells each of the room's rules found,
    /// by its place in [`room::RoomRules::places`].
    pub fn build_room(
        &mut self,
        p: &Present,
        pic: u32,
        cells: &room::Cells,
    ) -> Result<Vec<usize>, String> {
        let st = self
            .rules
            .stage
            .as_ref()
            .ok_or("the look builds no rooms (no `stage` line)")?;
        let k = self
            .room_screens(p)
            .into_iter()
            .find(|(x, _)| *x == pic)
            .map(|(_, k)| k)
            .ok_or("the look builds no room over that picture")?;
        if cells.cells.len() != cells.cols * cells.rows {
            return Err(format!(
                "{} cells for a screen of {} by {}",
                cells.cells.len(),
                cells.cols,
                cells.rows
            ));
        }
        let mut built = room::build(st, &self.rules.rooms[k], cells);
        for d in &mut built.stage.draws {
            if let StageSrc::Piece(i) = d.src {
                let art = &st.pieces[i as usize].1;
                d.src =
                    StageSrc::Piece(self.files.iter().position(|f| f == art).unwrap_or(0) as u32);
            }
        }
        self.stages[pic as usize] = Some(built.stage);
        Ok(built.cells)
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
    /// What of the scene the draw stands for: a runtime that draws the
    /// game's own pictures itself (the board's, gp_look::board) leaves out
    /// those the look draws as art, or not at all.
    pub of: Of,
}

/// What of the scene an [`ArtDraw`] stands for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Of {
    /// Anything else: a tint, the HUD, a prop, the drift, the wall.
    #[default]
    Other,
    /// The level's picture, or the stage built over it.
    Level,
    /// The body of that number in the scene, as it shows this frame.
    Body(usize),
    /// The changed cell of that number in the scene's `cells`.
    Cell(usize),
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
            of: Of::Other,
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

    /// The picture body `i` shows this frame, `d` on level `level`: in a
    /// region, the first of its `#+ in` rules there that holds (a timed
    /// one that holds now, or a picture); else a timed one, or its own.
    fn shown(&self, art: &Art, i: usize, d: &Draw, level: Option<u32>) -> (Option<Placed>, bool) {
        let pic = d.pic;
        let base = art.pics.get(pic as usize).copied().flatten();
        let timed = art.timed.get(pic as usize).filter(|r| !r.is_empty());
        let here = art.in_region.get(pic as usize).filter(|r| !r.is_empty());
        if timed.is_none() && here.is_none() {
            return (base, false);
        }
        let a = self.ages.get(i).filter(|a| a.pic == Some(pic));
        let age = a.map_or(0, |a| self.clock - a.since);
        let before = a.and_then(|a| a.before.iter().rev().nth(1).copied().flatten());
        let holds = |r: &TimedAt| {
            if (r.wait > 0 && age < r.wait) || (r.after.is_some() && r.after != before) {
                return false;
            }
            let g = if r.every > 0 {
                self.clock % r.every
            } else {
                age
            };
            g >= r.from && g < r.to
        };
        for (k, rule) in here.into_iter().flatten() {
            if !art.in_region(*k, d, level) {
                continue;
            }
            match rule {
                InRegion::Timed(r) if holds(r) => return (Some(r.art), r.hide),
                InRegion::Pic(a) => return (Some(*a), false),
                InRegion::Timed(_) => {}
            }
        }
        for r in timed.into_iter().flatten() {
            if holds(r) {
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
                && self.shown(art, i, d, s.level).1
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
            let (mut src, _) = self.shown(art, i, d, s.level);
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
            a.of = Of::Body(i);
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
            out.draws.push(ArtDraw { of: Of::Level, ..d });
        }
        self.drift(art, s, ox, oy, &mut out);
        for &(m, c) in &s.tints {
            let [w, h] = own(m);
            out.draws
                .push(ArtDraw::new(Src::Tint(m, c), ox * u, oy * u, w * u, h * u));
        }
        for (k, d) in s.cells.iter().enumerate() {
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
            out.draws.last_mut().expect("just drawn").of = Of::Cell(k);
        }
        let hud_bg = s.hud.len() - s.hud_sprites.min(s.hud.len());
        // The HUD's layer and background text: under the bodies, or (`#+
        // hud_over`) after them, whole or in rows. A layer and its tints
        // are cut into bands of rows by clips; text goes by its row.
        let (top, rest) = hud_bands(art.rules.hud_over.as_deref());
        let on_top = |d: &Draw| match art.rules.hud_over.as_deref() {
            None => false,
            Some([]) => true,
            Some(rows) => rows.contains(&d.y.div_euclid(8)),
        };
        let band = |(y0, y1): (i64, i64)| {
            (y0 > -FAR || y1 < FAR).then(|| [-FAR * u, (hy + y0) * u, 2 * FAR * u, (y1 - y0) * u])
        };
        let hud_bg_part = |out: &mut ArtFrame, over: bool| {
            let bands = if over { &top } else { &rest };
            for (j, d) in s.hud[..hud_bg].iter().enumerate() {
                if Art::is(&art.hud_layers, d.pic) {
                    for &b in bands {
                        plain(out, d, d.x + hx, d.y + hy, 255);
                        out.draws.last_mut().expect("just drawn").clip = band(b);
                    }
                } else if on_top(d) == over {
                    plain(out, d, d.x + hx, d.y + hy, 255);
                }
                if j == 0 {
                    for &(m, c) in &s.hud_tints {
                        let [w, h] = own(m);
                        for &b in bands {
                            let mut t = ArtDraw::new(Src::Tint(m, c), hx * u, hy * u, w * u, h * u);
                            t.clip = band(b);
                            out.draws.push(t);
                        }
                    }
                }
            }
        };
        // The HUD's sprites, last. With no layer or background text, the
        // tints follow the first, as they always have.
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
        hud_bg_part(&mut out, false);
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
        hud_bg_part(&mut out, true);
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

/// Far past any screen, in pixels: a band of the HUD open at an end.
const FAR: i64 = 4096;

/// The bands of the HUD's rows (top and bottom, in screen pixels) drawn
/// over the bodies, and those drawn under them, for `hud_over`'s rows.
fn hud_bands(over: Option<&[i64]>) -> (Vec<Band>, Vec<Band>) {
    let whole = vec![(-FAR, FAR)];
    let rows = match over {
        None => return (Vec::new(), whole),
        Some([]) => return (whole, Vec::new()),
        Some(rows) => rows,
    };
    let mut top: Vec<Band> = Vec::new();
    for &r in rows {
        match top.last_mut() {
            Some(b) if b.1 == r * 8 => b.1 = r * 8 + 8,
            _ => top.push((r * 8, r * 8 + 8)),
        }
    }
    let mut rest = Vec::new();
    let mut from = -FAR;
    for &(y0, y1) in &top {
        if y0 > from {
            rest.push((from, y0));
        }
        from = y1;
    }
    rest.push((from, FAR));
    (top, rest)
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
