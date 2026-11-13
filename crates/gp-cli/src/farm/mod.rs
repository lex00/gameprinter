//! `gp farm`: a decision model plays a level, by search. The emulator is
//! deterministic and saves its state for free, so this is Go-Explore
//! (docs/research/laya-level-play.md): an archive of cells (where the
//! player is, whether it stands, how fast it goes), each with a savestate
//! and the input log that reached it. Each round picks a cell near the
//! frontier (less visited ones more often), asks the decision model which
//! move to try first, simulates its most likely moves, and keeps any that
//! reach a new cell or an old one sooner. A death or a stall is dropped.
//! The model only orders the moves; the emulator says what works.
//!
//! Everything about the game comes from its package in the games checkout
//! (the player's bytes from its state mapping, the level key in
//! observe.toml, the level grid in mapping.toml [cells]): no game's
//! addresses are in this file.
//!
//! Ported from game-printer's farm.rs, blind.rs and blind_topdown.rs. What
//! gameprinter's layout changed: game-printer read the player's bytes from
//! the game's ROM dataset (`gp dataset rom`, not here yet) and its template
//! under templates/; here they come from the package's mapping.toml
//! (`entities.player[0].x` and `.y`, as `gp mechanic demo` reads them) and
//! observe.toml, and the ROM from its roms.toml. The decision model's
//! plan and question named Super Mario Bros.' hero; they say "the player"
//! now. The search over a package reads the NES's RAM and sprite table, so
//! an arcade game is refused in words; the blind search (`--blind`, and
//! `gp model blind`) plays an arcade romset too, on gp-arcade (machine.rs).

pub(crate) mod blind;
pub(crate) mod lanes;
pub(crate) mod machine;
mod support;
pub(crate) mod topdown;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Read as _, Write as _};
use std::net::TcpStream;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use clap::Args;
use serde_json::{Value, json};

use crate::Outcome;
use gp_evidence::Paths;
use gp_evidence::input_log::{InputFrame, InputLog};
use support as m;

#[derive(Args, Debug)]
pub(crate) struct FarmArgs {
    /// The game, as its template is named (smb1-ntsc-prg0); or `bench` to
    /// run the blind bench (bench/farm-blind.toml). Not needed with --blind.
    #[arg(required_unless_present = "blind")]
    pub game: Option<String>,
    /// Play this ROM blind: no dataset, template or mapping; everything is
    /// learned from the running game (crates/gp-cli/src/blind.rs).
    #[arg(long)]
    pub blind: Option<PathBuf>,
    /// Blind: the most seconds of search.
    #[arg(long, default_value_t = 600.0)]
    pub seconds: f64,
    /// Blind: stop after this many levels finished.
    #[arg(long, default_value_t = 3)]
    pub levels: usize,
    /// Blind: show the search in a mechanic session, GAME/MECHANIC: its
    /// queue row and status line every 50 rounds, and its best run so far
    /// as a clip the page loops (at most every 30 s), then a result turn.
    #[arg(long)]
    pub session: Option<String>,
    /// Bench: run the held-out games instead of the train games. The
    /// improvement loop never passes this.
    #[arg(long)]
    pub held_out: bool,
    /// Blind: search for each goal in this probes file (a JSON list; see
    /// blind.rs `Probe`) instead of progress, one after another.
    #[arg(long, requires = "blind")]
    pub probe: Option<PathBuf>,
    /// Bench: only the game with this name.
    #[arg(long)]
    pub only: Option<String>,
    /// Bench: a ledger line's change, to append the score to
    /// docs/research/farm-ledger.md.
    #[arg(long)]
    pub ledger: Option<String>,
    /// The level to play, by its name in the template's observe.toml
    /// [level] names (w1_1). It must be the level play starts in.
    #[arg(long, default_value = "w1_1")]
    pub level: String,
    /// The decision server; "uniform" tries moves in a seeded random order
    /// instead, as the baseline.
    #[arg(long, default_value = "http://127.0.0.1:7720")]
    pub decide: String,
    /// The most rounds before it stops.
    #[arg(long, default_value_t = 2000)]
    pub rounds: usize,
    /// Moves simulated a round when the model is sure enough.
    #[arg(long, default_value_t = 3)]
    pub top: usize,
    /// Below this confidence every move is simulated.
    #[arg(long, default_value_t = 0.4)]
    pub unsure: f64,
    /// Cells this many pixels behind the furthest x are the frontier,
    /// picked 20 times as often.
    #[arg(long, default_value_t = 256)]
    pub frontier: i64,
    /// The baseline's seed.
    #[arg(long, default_value_t = 1)]
    pub seed: u64,
    /// A recording in the store (a TAS) to check against, held out: only
    /// the furthest x it reaches in the level is read, never its moves.
    #[arg(long)]
    pub check: Option<String>,
    /// Write the winning path as a clip in the session farm-LEVEL, measure
    /// it, and set the queue row.
    #[arg(long)]
    pub save: bool,
    /// The games checkout the game's package is in (default $GP_GAMES_DIR,
    /// else a `games` folder beside the current one).
    #[arg(long)]
    pub games: Option<PathBuf>,
}

// Pad bits, as the input log writes them.
pub(crate) const A: u8 = 0x01;
pub(crate) const B: u8 = 0x02;
pub(crate) const UP: u8 = 0x10;
pub(crate) const DOWN: u8 = 0x20;
pub(crate) const LEFT: u8 = 0x40;
pub(crate) const RIGHT: u8 = 0x80;

/// A move: buttons held for some frames, then others until the player is
/// back on the ground (or a limit).
#[derive(Clone)]
pub(crate) struct Move {
    pub(crate) name: &'static str,
    pub(crate) about: &'static str,
    pub(crate) press: Vec<(u8, usize)>,
    /// Held after `press` until it lands; 0 frames means no settling.
    pub(crate) then: u8,
    pub(crate) settle: usize,
}

pub(crate) fn moves(hop: usize, mid: usize, full: usize) -> Vec<Move> {
    let mv = |name, about, press: Vec<(u8, usize)>, then, settle| Move {
        name,
        about,
        press,
        then,
        settle,
    };
    vec![
        mv("walk", "walk right a short way", vec![(RIGHT, 16)], 0, 0),
        mv("run", "run right a short way", vec![(RIGHT | B, 16)], 0, 0),
        mv(
            "hop",
            "a small hop while walking right",
            vec![(RIGHT, 1), (RIGHT | A, hop)],
            RIGHT,
            90,
        ),
        mv(
            "mid_jump",
            "a medium jump while walking right",
            vec![(RIGHT, 1), (RIGHT | A, mid)],
            RIGHT,
            90,
        ),
        mv(
            "full_jump",
            "a full-height jump while walking right",
            vec![(RIGHT, 1), (RIGHT | A, full)],
            RIGHT,
            90,
        ),
        mv(
            "run_full_jump",
            "a full-height jump while running right, the longest jump",
            vec![(RIGHT | B, 1), (RIGHT | B | A, full)],
            RIGHT | B,
            90,
        ),
        mv(
            "jump_in_place",
            "a full-height jump straight up, to land on what comes close",
            vec![(0, 1), (A, full)],
            0,
            90,
        ),
        mv(
            "wait",
            "stand still and let things move",
            vec![(0, 16)],
            0,
            0,
        ),
        mv("back", "walk left a short way", vec![(LEFT, 16)], 0, 0),
        // Ladders and vines: up and down held, for games whose levels go
        // up as well as across.
        mv(
            "climb_up",
            "hold Up: climb a ladder above",
            vec![(UP, 40)],
            0,
            0,
        ),
        mv(
            "climb_down",
            "hold Down: climb down a ladder below",
            vec![(DOWN, 40)],
            0,
            0,
        ),
        // A ladder is taken only from right in front of it (a game snaps
        // the climber onto its middle from a pixel or so away): creep a
        // couple of frames at a time with Up held between, so one of the
        // stops lands on the ladder, then keep climbing. Left and Right do
        // nothing on a ladder, so the creeping stops once it climbs.
        mv(
            "seek_climb_right",
            "creep right, trying Up at each step, then climb",
            [[(RIGHT, 2), (UP, 10)].repeat(10), vec![(UP, 60)]].concat(),
            0,
            0,
        ),
        mv(
            "seek_climb_left",
            "creep left, trying Up at each step, then climb",
            [[(LEFT, 2), (UP, 10)].repeat(10), vec![(UP, 60)]].concat(),
            0,
            0,
        ),
        mv(
            "jump_back",
            "a full jump that steers back left in the air",
            vec![(RIGHT, 1), (RIGHT | A, full / 2), (LEFT | A, full / 2)],
            LEFT,
            90,
        ),
    ]
}

/// Aimed moves, for the last stretch to a goal once travel has brought the
/// player near it: a short step (0 to `steps` frames each way) and then the
/// approach the goal needs. "below": a full jump straight up (into a block
/// overhead); "above": a full jump forward that comes down on it; "side":
/// walk into it. Tried all of them, not in a model's order.
// Not used by the blind farm now: aimed moves were measured in loop pass 2
// and didn't help (docs/research/farm-ledger.md). Kept for other searches.
#[allow(dead_code)]
pub(crate) fn aimed_moves(approach: &str, steps: usize, full: usize) -> Vec<Move> {
    let mut v = Vec::new();
    let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
    for k in 0..=steps {
        for (dir, side) in [(RIGHT, "r"), (LEFT, "l")] {
            if k == 0 && side == "l" {
                continue;
            }
            let step = if k == 0 { vec![] } else { vec![(dir, k)] };
            let (press, then): (Vec<(u8, usize)>, u8) = match approach {
                "below" => ([step, vec![(0, 2), (A, full)]].concat(), 0),
                "above" => ([step, vec![(0, 2), (RIGHT | A, full)]].concat(), RIGHT),
                _ => ([step, vec![(RIGHT, 48)]].concat(), 0),
            };
            v.push(Move {
                name: leak(format!("aim_{approach}_{side}{k}")),
                about: leak(format!("step {k} frames {side}, then {approach}")),
                press,
                then,
                settle: if approach == "side" { 0 } else { 90 },
            });
        }
    }
    v
}

/// The level key and its names, from the template's observe.toml [level].
pub(crate) struct LevelKey {
    ram: Vec<u16>,
    when: Option<gp_runtime::select::Condition>,
    names: BTreeMap<Vec<u8>, String>,
}

impl LevelKey {
    pub(crate) fn load(game_dir: &Path) -> Result<LevelKey> {
        let file = game_dir.join("observe.toml");
        let t: toml::Value = toml::from_str(
            &fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?,
        )?;
        let l = t
            .get("level")
            .context("the game's observe.toml has no [level]")?;
        let ram = l
            .get("ram")
            .and_then(|v| v.as_array())
            .context("[level] ram")?
            .iter()
            .filter_map(|v| v.as_integer().map(|n| n as u16))
            .collect();
        let when = l
            .get("when")
            .and_then(|w| w.as_str())
            .and_then(|w| gp_runtime::select::Condition::parse(w).ok());
        let mut names = BTreeMap::new();
        for n in l
            .get("names")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
        {
            let Some(a) = n.as_array() else { continue };
            let (Some(k), Some(v)) = (
                a.first().and_then(|k| k.as_str()),
                a.get(1).and_then(|v| v.as_str()),
            ) else {
                continue;
            };
            let key: Vec<u8> = k
                .split_whitespace()
                .filter_map(|h| u8::from_str_radix(h.trim_start_matches("0x"), 16).ok())
                .collect();
            names.insert(key, v.to_owned());
        }
        Ok(LevelKey { ram, when, names })
    }

    /// The level the RAM is in, or None between levels.
    pub(crate) fn of(&self, w: &[u8]) -> Option<String> {
        if let Some(c) = &self.when {
            let ok = c
                .eval(&mut |sel| match (sel.name().as_str(), sel.last_index()?) {
                    ("ram", Some(gp_runtime::select::Index::At(n))) if (0..0x800).contains(&n) => {
                        Ok(i64::from(w[n as usize]))
                    }
                    _ => Err("only ram[N]".into()),
                })
                .unwrap_or(false);
            if !ok {
                return None;
            }
        }
        let key: Vec<u8> = self.ram.iter().map(|a| w[usize::from(*a)]).collect();
        Some(self.names.get(&key).cloned().unwrap_or_else(|| {
            key.iter()
                .map(|b| format!("{b:02x}"))
                .collect::<Vec<_>>()
                .join("")
        }))
    }
}

/// The level grid the template's mapping.toml [cells] reads: pages of
/// columns of rows of tile bytes.
struct Grid {
    pages: Vec<u16>,
    cols: usize,
    rows: usize,
    top: usize,
}

impl Grid {
    fn load(game_dir: &Path) -> Option<Grid> {
        let t: toml::Value =
            toml::from_str(&fs::read_to_string(game_dir.join("mapping.toml")).ok()?).ok()?;
        let c = t.get("cells")?;
        Some(Grid {
            pages: c
                .get("pages")?
                .as_array()?
                .iter()
                .filter_map(|v| v.as_integer().map(|n| n as u16))
                .collect(),
            cols: c.get("page_columns")?.as_integer()? as usize,
            rows: c.get("rows")?.as_integer()? as usize,
            top: c.get("top").and_then(|v| v.as_integer()).unwrap_or(0) as usize,
        })
    }

    /// The tile at a level column and a grid row; None off the grid.
    fn at(&self, w: &[u8], col: i64, row: i64) -> Option<u8> {
        if col < 0 || row < 0 || row as usize >= self.rows || self.pages.is_empty() {
            return None;
        }
        let col = col as usize;
        let page = (col / self.cols) % self.pages.len();
        let a = usize::from(self.pages[page]) + row as usize * self.cols + col % self.cols;
        w.get(a).copied()
    }
}

/// The player's bytes.
struct Player {
    x_hi: Option<u16>,
    x_lo: u16,
    y: u16,
}

/// The player's bytes from the game's state mapping: the first `ram` byte
/// of `entities.player[0].x` and `.y`. game-printer read them (and an x
/// high byte) from the ROM dataset's ram table; a mapping field names no
/// high byte, so a level wider than 256 pixels needs the dataset back.
fn player_of(game_dir: &Path) -> Result<Player> {
    let file = game_dir.join(gp_interp::mapping::MAPPING_FILE);
    let text = fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
    let m: toml::Table = toml::from_str(&text).with_context(|| file.display().to_string())?;
    let field = |path: &str| -> Option<u16> {
        m.get("field")?.as_array()?.iter().find_map(|f| {
            (f.get("path")?.as_str()? == path)
                .then(|| f.get("ram")?.as_array()?.first()?.as_integer())
                .flatten()
                .and_then(|a| u16::try_from(a).ok())
        })
    };
    Ok(Player {
        x_hi: None,
        x_lo: field("entities.player[0].x").with_context(|| {
            format!(
                "{} has no entities.player[0].x with a ram byte",
                file.display()
            )
        })?,
        y: field("entities.player[0].y").with_context(|| {
            format!(
                "{} has no entities.player[0].y with a ram byte",
                file.display()
            )
        })?,
    })
}

/// Where a run is, read from the console.
#[derive(Clone, Copy, Debug)]
struct Pos {
    x: i64,
    y: i64,
    dx: i64,
    ground: bool,
}

/// A place in the archive.
struct Cell {
    snap: Vec<u8>,
    frame: u64,
    held: [u8; 4],
    log: Vec<InputFrame>,
    pos: Pos,
    visits: u32,
    failed: BTreeSet<&'static str>,
}

type Key = (i64, i64, bool, i64);

fn key_of(p: &Pos) -> Key {
    (p.x / 16, p.y / 16, p.ground, p.dx.abs().min(2))
}

/// What a move ended in.
enum End {
    At(Pos),
    Died,
    /// Left for another level: its name.
    Won(String),
    Stalled,
}

struct World<'a> {
    nes: gp_nes::Nes,
    player: &'a Player,
    level: &'a LevelKey,
    grid: Option<&'a Grid>,
    name: String,
    /// The tile row offset: feet row = (y + k) / 16 - top.
    k: i64,
}

impl World<'_> {
    fn wram(&self) -> &[u8] {
        &self.nes.wram()[..0x800]
    }

    fn x(&self) -> i64 {
        let w = self.wram();
        let hi = self.player.x_hi.map_or(0, |h| i64::from(w[usize::from(h)]));
        hi * 256 + i64::from(w[usize::from(self.player.x_lo)])
    }

    fn y(&self) -> i64 {
        i64::from(self.wram()[usize::from(self.player.y)])
    }

    fn step(&mut self, pad: u8, log: &mut Vec<InputFrame>) -> Result<()> {
        let f = InputFrame {
            polled: true,
            pads: [pad, 0, 0, 0],
            reset: false,
        };
        self.nes.step(f)?;
        log.push(f);
        Ok(())
    }

    /// After the level stops being played: run on with nothing held until
    /// a level is played again (up to 15 s). Another level is a win, the
    /// same one again a death.
    fn settle_end(&mut self, log: &mut Vec<InputFrame>) -> Result<End> {
        for _ in 0..900 {
            self.step(0, log)?;
            if let Some(l) = self.level.of(self.wram()) {
                return Ok(if l == self.name {
                    End::Died
                } else {
                    End::Won(l)
                });
            }
        }
        Ok(End::Died)
    }

    /// Runs a move from the current state; the inputs go on `log`.
    fn play(&mut self, mv: &Move, log: &mut Vec<InputFrame>) -> Result<End> {
        let (x0, y0) = (self.x(), self.y());
        let (mut px, mut py) = (x0, y0);
        let mut dx = 0;
        let mut still = 0;
        let mut frames = 0;
        let check = |w: &mut Self, log: &mut Vec<InputFrame>| -> Result<Option<End>> {
            match w.level.of(w.wram()) {
                Some(l) if l == w.name => Ok(None),
                Some(l) => Ok(Some(End::Won(l))),
                None => w.settle_end(log).map(Some),
            }
        };
        for &(pad, n) in &mv.press {
            for _ in 0..n {
                self.step(pad, log)?;
                frames += 1;
                if let Some(e) = check(self, log)? {
                    return Ok(e);
                }
                let (x, y) = (self.x(), self.y());
                dx = x - px;
                still = if y == py { still + 1 } else { 0 };
                (px, py) = (x, y);
            }
        }
        if mv.settle > 0 {
            // Until it's been on something for 2 frames after leaving it.
            let mut left_ground = still == 0;
            for _ in 0..mv.settle {
                self.step(mv.then, log)?;
                frames += 1;
                if let Some(e) = check(self, log)? {
                    return Ok(e);
                }
                let (x, y) = (self.x(), self.y());
                if y > 232 || (py > 200 && y < 32) {
                    return Ok(End::Died);
                }
                dx = x - px;
                still = if y == py { still + 1 } else { 0 };
                if still == 0 {
                    left_ground = true;
                }
                (px, py) = (x, y);
                if left_ground && still >= 2 {
                    break;
                }
            }
        }
        // Off the bottom: below the screen, or wrapped round past it.
        if self.y() > 232 {
            return Ok(End::Died);
        }
        let p = Pos {
            x: self.x(),
            y: self.y(),
            dx,
            ground: still >= 1,
        };
        if frames > 0 && p.x == x0 && p.y == y0 && mv.name != "wait" {
            return Ok(End::Stalled);
        }
        // A player the pad no longer moves is dying, at the end of the
        // level, or in a scene: play on with nothing held to see which.
        // Falling off the bottom is a death; another level is the end.
        if !self.responsive()? {
            let mut py = self.y();
            for i in 0..600 {
                self.step(0, log)?;
                let y = self.y();
                if y > 232 || (py > 200 && y < 32) {
                    return Ok(End::Died);
                }
                py = y;
                match self.level.of(self.wram()) {
                    Some(l) if l == self.name => {
                        if i % 30 == 29 && self.responsive()? {
                            let p = Pos {
                                x: self.x(),
                                y: self.y(),
                                dx: 0,
                                ground: true,
                            };
                            return Ok(End::At(p));
                        }
                    }
                    Some(l) => return Ok(End::Won(l)),
                    None => return self.settle_end(log),
                }
            }
            return Ok(End::Died);
        }
        Ok(End::At(p))
    }

    /// Whether 8 frames of Right and 8 of Left end in different places;
    /// the console is put back as it was.
    fn responsive(&mut self) -> Result<bool> {
        let (snap, frame, held) = (self.nes.snapshot()?, self.nes.frame(), self.nes.held());
        let mut ends = Vec::new();
        for pad in [RIGHT, LEFT] {
            self.nes.restore(&snap, frame, held)?;
            let mut scratch = Vec::new();
            for _ in 0..8 {
                self.step(pad, &mut scratch)?;
            }
            ends.push((self.x(), self.y()));
        }
        self.nes.restore(&snap, frame, held)?;
        Ok(ends[0] != ends[1])
    }

    /// The feet row for a y.
    fn feet_row(&self, y: i64) -> i64 {
        let top = self.grid.map_or(0, |g| g.top as i64);
        (y + self.k).div_euclid(16) - top
    }

    fn solid(&self, col: i64, row: i64) -> bool {
        self.grid
            .and_then(|g| g.at(self.wram(), col, row))
            .is_some_and(|t| t != 0)
    }

    /// The offset that puts a standing player's feet on a solid tile with
    /// an empty one above it.
    fn calibrate(&mut self) {
        let col = self.x() / 16;
        let y = self.y();
        for k in 0..64 {
            self.k = k;
            let r = self.feet_row(y);
            if self.solid(col, r) && !self.solid(col, r - 1) {
                return;
            }
        }
        self.k = 16;
    }

    /// What the decision model is told, deciding things first.
    fn state(&self, p: &Pos, plan: &str, failed: &BTreeSet<&'static str>) -> Value {
        let col = p.x / 16;
        let feet = self.feet_row(p.y);
        let into = p.x % 16;
        let rows = self.grid.map_or(13, |g| g.rows as i64);
        // A gap: a column with nothing from the feet row down.
        let mut gap = None;
        for dc in 1..=12 {
            if (feet..rows).all(|r| !self.solid(col + dc, r)) {
                let mut w = 1;
                while w < 8 && (feet..rows).all(|r| !self.solid(col + dc + w, r)) {
                    w += 1;
                }
                gap = Some((dc * 16 - into, w * 16));
                break;
            }
        }
        // A wall: something solid at body height ahead, and how tall.
        let mut wall = None;
        for dc in 1..=8 {
            if self.solid(col + dc, feet - 1) {
                let mut h = 1;
                while h < 8 && self.solid(col + dc, feet - 1 - h) {
                    h += 1;
                }
                wall = Some((dc * 16 - into, h * 16));
                break;
            }
        }
        let ceiling = (1..=6)
            .find(|d| self.solid(col, feet - 1 - d))
            .map(|d| d * 16);
        // The nearest sprite box ahead that isn't the player.
        let oam = self.nes.oam();
        let boxes = m::sprite_boxes(&oam, if self.nes.tall_sprites() { 16 } else { 8 });
        let sc = &self.nes.bus().ppu.scroll;
        let scroll =
            i64::from((sc.t & 31) * 8 + u16::from(sc.fine_x as u8 & 7) + ((sc.t >> 10) & 1) * 256);
        let sx = (p.x - scroll).rem_euclid(512);
        let mut enemy: Option<(i64, i64)> = None;
        for b in &boxes {
            let (bx, by) = (i64::from(b[0]), i64::from(b[1]));
            let ddx = bx - sx;
            let ddy = by - p.y;
            if (ddx.abs() < 12 && ddy.abs() < 24) || ddx <= 8 || ddx > 160 || ddy.abs() > 64 {
                continue;
            }
            if enemy.is_none_or(|(e, _)| ddx < e) {
                enemy = Some((ddx, ddy));
            }
        }
        let speed = p.dx.abs().max(1);
        json!({
            "plan": plan,
            "on_ground": p.ground,
            "speed_px_a_frame": p.dx,
            "gap_ahead": gap.map_or(json!("none"), |(d, w)| json!(format!("{d} px ahead, {w} px wide"))),
            "wall_ahead": wall.map_or(json!("none"), |(d, h)| json!(format!("{d} px ahead, {h} px tall"))),
            "enemy_ahead": enemy.map_or(json!("none"), |(d, dy)| json!(format!("{d} px ahead, {} , {} frames away", if dy < -8 { "higher up" } else if dy > 8 { "lower down" } else { "level with Mario" }, d / speed))),
            "ceiling": ceiling.map_or(json!("none"), |d| json!(format!("{d} px above"))),
            "failed_here": failed.iter().collect::<Vec<_>>(),
        })
    }
}

/// A little seeded random source for the baseline and the frontier pick.
pub(crate) struct Rng(pub(crate) u64);

impl Rng {
    pub(crate) fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    pub(crate) fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

pub(crate) fn post(url: &str, body: &Value) -> Result<Value> {
    let rest = url
        .strip_prefix("http://")
        .context("the decide URL is http://HOST:PORT")?;
    let (host, path) = rest
        .split_once('/')
        .map_or((rest, "/".to_owned()), |(h, p)| (h, format!("/{p}")));
    let mut conn = TcpStream::connect(host)
        .with_context(|| format!("no decision server at {url}: start one with `just laya`"))?;
    let text = body.to_string();
    write!(
        conn,
        "POST {path} HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
        text.len()
    )?;
    let mut out = String::new();
    conn.read_to_string(&mut out)?;
    let (head, body) = out.split_once("\r\n\r\n").context("no HTTP answer")?;
    if !head.starts_with("HTTP/1.1 200") {
        bail!(
            "the decision server answered {}: {body}",
            head.lines().next().unwrap_or("")
        );
    }
    Ok(serde_json::from_str(body)?)
}

/// A-hold lengths from the person's clips: runs of A held, short to long.
pub(crate) fn a_holds(dir: &Path) -> Vec<usize> {
    let mut out = Vec::new();
    for d in fs::read_dir(dir).into_iter().flatten().flatten() {
        let clips = d.path().join("clips");
        for c in fs::read_dir(&clips).into_iter().flatten().flatten() {
            let p = c.path();
            if p.extension().is_none_or(|e| e != "toml") {
                continue;
            }
            let Ok(meta) = fs::read_to_string(&p).map(|t| t.parse::<toml::Table>()) else {
                continue;
            };
            let Ok(meta) = meta else { continue };
            if meta.get("by").and_then(|v| v.as_str()) != Some("player") {
                continue;
            }
            // A clip saved as a path and a frame on it holds only its own
            // buttons in its .log; an older clip, its whole log.
            let start = if meta.contains_key("path") {
                0
            } else {
                meta.get("start").and_then(|v| v.as_integer()).unwrap_or(0) as usize
            };
            let Ok(text) = fs::read_to_string(p.with_extension("log")) else {
                continue;
            };
            let Ok(log) = InputLog::parse(&text) else {
                continue;
            };
            let mut run = 0;
            for f in log.frames.iter().skip(start) {
                if f.pads[0] & A != 0 {
                    run += 1;
                } else if run > 0 {
                    out.push(run);
                    run = 0;
                }
            }
        }
    }
    out.sort_unstable();
    out
}

pub(crate) fn run(a: &FarmArgs, paths: &Paths) -> Result<Outcome> {
    let opts = || blind::Opts {
        decide: a.decide.clone(),
        rounds: a.rounds,
        top: a.top,
        unsure: a.unsure,
        frontier: a.frontier,
        seed: a.seed,
        seconds: a.seconds,
        levels: a.levels,
        session: a.session.clone(),
    };
    if let Some(rom) = &a.blind {
        blind::check_rom(rom)?;
        // A held-out game is played only by `gp farm bench --held-out`.
        blind::refuse_held_out(rom, &m::repo_root()?.join("bench/farm-blind.toml"))?;
        let name = rom
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("rom")
            .replace(' ', "_");
        let game_dir = paths.home.join("work/farm/blind").join(name);
        if let Some(file) = &a.probe {
            let v = blind::probes(rom, &opts(), paths, &game_dir, file)?;
            let mut text = String::new();
            for p in v["probes"].as_array().into_iter().flatten() {
                text.push_str(&format!(
                    "{}: {}\n",
                    p["probe"].as_str().unwrap_or("?"),
                    match (p["reached"].as_bool(), p["error"].as_str()) {
                        (_, Some(e)) => format!("failed: {e}"),
                        (Some(true), _) => format!(
                            "reached in {} s ({} rounds), {}",
                            p["seconds"], p["round"], p["seen"]
                        ),
                        _ => format!(
                            "not reached in {} rounds; closest {}",
                            p["rounds_run"], p["closest"]
                        ),
                    }
                ));
            }
            return Ok(Outcome::ok(v, text.trim_end().to_owned()));
        }
        let v = blind::farm_run(rom, &opts(), paths, &game_dir)?;
        let text = blind_line(&v);
        return Ok(Outcome::ok(v, text));
    }
    let game = a.game.clone().unwrap_or_default();
    if game == "bench" {
        let repo = m::repo_root()?;
        let v = blind::bench(
            &repo.join("bench/farm-blind.toml"),
            a.held_out,
            &opts(),
            paths,
            a.only.as_deref(),
        )?;
        let mut text = format!(
            "{} ({})",
            v["score"].as_str().unwrap_or(""),
            v["set"].as_str().unwrap_or("")
        );
        for g in v["games"].as_array().into_iter().flatten() {
            text.push_str(&format!(
                "\n{}: {}",
                g["name"].as_str().unwrap_or(""),
                match g["skipped"].as_str() {
                    Some(w) => format!("skipped: {w}"),
                    None => blind_line(g),
                }
            ));
        }
        if let Some(change) = &a.ledger {
            let f = repo.join("docs/research/farm-ledger.md");
            let head = std::process::Command::new("git")
                .args(["rev-parse", "--short", "HEAD"])
                .current_dir(&repo)
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
                .unwrap_or_default();
            let date = std::process::Command::new("date")
                .arg("+%Y-%m-%d")
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
                .unwrap_or_default();
            let mut t = fs::read_to_string(&f).unwrap_or_default();
            let row = format!(
                "| {date} | {head} | {change} | {} | |\n",
                v["score"].as_str().unwrap_or("")
            );
            // Rows go at the end of the first table: after its last row.
            let head = t.find("| Date | Commit |").unwrap_or(0);
            let mut end = head;
            for (i, line) in t[head..].split_inclusive('\n').scan(head, |at, l| {
                let i = *at;
                *at += l.len();
                Some((i, l))
            }) {
                if !line.starts_with('|') && i > head {
                    break;
                }
                end = i + line.len();
            }
            t.insert_str(end, &row);
            fs::write(&f, t)?;
        }
        return Ok(Outcome::ok(v, text));
    }
    let game = game.as_str();
    let t0 = std::time::Instant::now();
    if !m::name_ok(game) {
        bail!(
            "a game name is lower-case letters, digits and dashes: {}",
            game
        );
    }
    let (game_dir, bytes) = m::game_and_rom(paths, game, a.games.as_deref())?;
    let player = player_of(&game_dir)?;
    let level = LevelKey::load(&game_dir)?;
    let grid = Grid::load(&game_dir);
    let root = paths.home.join("mechanics").join(game);

    // Hold lengths for the jumps, from the person's clips.
    let holds: Vec<usize> = a_holds(&root).into_iter().filter(|h| *h <= 40).collect();
    let pick = |q: f64, or: usize| {
        if holds.is_empty() {
            or
        } else {
            holds[((holds.len() - 1) as f64 * q) as usize]
        }
    };
    let (hop, full) = (pick(0.0, 3).max(2), pick(1.0, 32).max(8));
    let mid = ((hop + full) / 2).max(hop + 1);
    let moves = moves(hop, mid, full);

    // Where play starts, as stage 0 finds it, then standing still.
    let starts = crate::reach::Starts {
        runs: 4,
        seed: 1,
        start_frames: gp_explore::presses::START_FRAMES,
        no_starts: false,
    };
    let clips = starts.with(&[], paths)?;
    let (ci, t_start) = m::play_start(&bytes, &clips)?.context("no start run reaches play")?;
    let mut log: Vec<InputFrame> = clips[ci].inputs[..t_start].to_vec();
    let mut nes = gp_nes::Nes::power_on(&bytes, clips[ci].power_on)?;
    // The picture isn't needed: the search reads RAM and OAM only.
    nes.set_video(false);
    for f in &log {
        nes.step(*f)?;
    }
    let mut w = World {
        nes,
        player: &player,
        level: &level,
        grid: grid.as_ref(),
        name: String::new(),
        k: 16,
    };
    let (mut still, mut py) = (0, w.y());
    for _ in 0..240 {
        if still >= 3 && w.level.of(w.wram()).is_some() {
            break;
        }
        w.step(0, &mut log)?;
        still = if w.y() == py { still + 1 } else { 0 };
        py = w.y();
    }
    let name = w
        .level
        .of(w.wram())
        .context("play start isn't in a level the template names")?;
    if name != a.level {
        bail!(
            "play starts in {name}, not {}: only the first level can be farmed for now",
            a.level
        );
    }
    w.name = name.clone();
    w.calibrate();
    let play_frame = log.len();
    let start_pos = Pos {
        x: w.x(),
        y: w.y(),
        dx: 0,
        ground: true,
    };

    let mut cells: BTreeMap<Key, Cell> = BTreeMap::new();
    cells.insert(
        key_of(&start_pos),
        Cell {
            snap: w.nes.snapshot()?,
            frame: w.nes.frame(),
            held: w.nes.held(),
            log: log.clone(),
            pos: start_pos,
            visits: 0,
            failed: BTreeSet::new(),
        },
    );
    let uniform = a.decide == "uniform";
    let mut rng = Rng(a.seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let plan = "Get the player as far right as possible and reach the end of the level. Jump over gaps and obstacles. Jump over enemies; never walk into one.";
    let criteria: serde_json::Map<String, Value> = moves
        .iter()
        .map(|mv| (mv.name.to_owned(), json!(mv.about)))
        .collect();
    let out_dir = paths.home.join("work/farm").join(game).join(&a.level);
    fs::create_dir_all(&out_dir)?;
    let log_name = if uniform {
        "log-uniform.ndjson"
    } else {
        "log.ndjson"
    };
    let mut flog = fs::File::create(out_dir.join(log_name))?;
    let mut best_x = start_pos.x;
    let (mut calls, mut sims, mut deaths) = (0usize, 0usize, 0usize);
    let mut won: Option<Vec<InputFrame>> = None;
    let mut left_to: Option<String> = None;
    let mut rounds = 0;
    let mut picks: BTreeMap<&'static str, usize> = BTreeMap::new();
    for round in 0..a.rounds {
        rounds = round + 1;
        // The frontier: less-visited cells, and the cells within the last
        // screen of the furthest x far more often.
        let weights: Vec<(Key, f64)> = cells
            .iter()
            .map(|(k, c)| {
                let near = if c.pos.x + a.frontier >= best_x {
                    20.0
                } else {
                    1.0
                };
                (*k, near / f64::from(c.visits + 1).sqrt())
            })
            .collect();
        let total: f64 = weights.iter().map(|(_, w)| w).sum();
        let mut t = rng.unit() * total;
        let mut key = weights[0].0;
        for (k, wt) in &weights {
            t -= wt;
            if t <= 0.0 {
                key = *k;
                break;
            }
        }
        let (snap, frame, held, base_log, pos, failed) = {
            let c = cells.get_mut(&key).expect("picked");
            c.visits += 1;
            (
                c.snap.clone(),
                c.frame,
                c.held,
                c.log.clone(),
                c.pos,
                c.failed.clone(),
            )
        };
        w.nes.restore(&snap, frame, held)?;
        // The order to try the moves in.
        let (order, conf, probs): (Vec<&'static str>, f64, Value) = if uniform {
            let mut o: Vec<&'static str> = moves.iter().map(|mv| mv.name).collect();
            for i in (1..o.len()).rev() {
                o.swap(i, (rng.next() % (i as u64 + 1)) as usize);
            }
            (o, 1.0, Value::Null)
        } else {
            let st = w.state(&pos, plan, &failed);
            let ask = json!({"model": "laya", "state": st, "questions": {"move": {
                "type": "choice",
                "instructions": "Which move should the player try first from this moment, to get further right without dying?",
                "criteria": criteria}}});
            let ans = post(
                &format!("{}/v1/systemone", a.decide.trim_end_matches('/')),
                &ask,
            )?;
            calls += 1;
            let q = &ans["answers"]["move"];
            let pr = q["probabilities"].clone();
            let mut o: Vec<(&'static str, f64)> = moves
                .iter()
                .map(|mv| (mv.name, pr[mv.name].as_f64().unwrap_or(0.0)))
                .collect();
            o.sort_by(|a, b| b.1.total_cmp(&a.1));
            (
                o.into_iter().map(|(n, _)| n).collect(),
                q["answer_confidence"].as_f64().unwrap_or(0.0),
                pr,
            )
        };
        let n = if conf < a.unsure {
            moves.len()
        } else {
            a.top.min(moves.len())
        };
        let mut outcome = Vec::new();
        for name in order.iter().take(n) {
            let mv = moves.iter().find(|mv| mv.name == *name).expect("a move");
            w.nes.restore(&snap, frame, held)?;
            let mut l = base_log.clone();
            sims += 1;
            let end = w.play(mv, &mut l)?;
            match end {
                End::Won(next) => {
                    outcome.push(json!({"move": name, "end": "left", "to": next}));
                    won = Some(l);
                    left_to = Some(next);
                    break;
                }
                End::Died => {
                    deaths += 1;
                    outcome.push(json!({"move": name, "end": "died"}));
                    if let Some(c) = cells.get_mut(&key) {
                        c.failed.insert(mv.name);
                    }
                }
                End::Stalled => {
                    outcome.push(json!({"move": name, "end": "stalled"}));
                    if let Some(c) = cells.get_mut(&key) {
                        c.failed.insert(mv.name);
                    }
                }
                End::At(p) => {
                    let k = key_of(&p);
                    let better = cells.get(&k).is_none_or(|c| l.len() < c.log.len());
                    outcome.push(json!({"move": name, "x": p.x, "new": better}));
                    if better {
                        *picks.entry(mv.name).or_default() += 1;
                        let visits = cells.get(&k).map_or(0, |c| c.visits);
                        cells.insert(
                            k,
                            Cell {
                                snap: w.nes.snapshot()?,
                                frame: w.nes.frame(),
                                held: w.nes.held(),
                                log: l,
                                pos: p,
                                visits,
                                failed: BTreeSet::new(),
                            },
                        );
                        best_x = best_x.max(p.x);
                    }
                }
            }
        }
        writeln!(
            flog,
            "{}",
            json!({"round": round, "from_x": pos.x, "best_x": best_x, "cells": cells.len(),
                   "confidence": conf, "probabilities": probs, "tried": outcome})
        )?;
        if won.is_some() {
            break;
        }
    }
    let secs = t0.elapsed().as_secs_f64();

    // The held-out check: the furthest x the recording reaches in the level.
    let tas_x = match &a.check {
        Some(rec) => Some(tas_furthest(paths, &bytes, &player, &level, rec, &a.level)?),
        None => None,
    };
    let path_frames = won.as_ref().map(|l| l.len() - play_frame);
    let mut v = json!({
        "game": game, "level": a.level, "prior": if uniform { "uniform" } else { "laya" },
        "won": won.is_some(), "left_to": left_to, "rounds": rounds, "seconds": (secs * 10.0).round() / 10.0,
        "decision_calls": calls, "moves_simulated": sims, "deaths": deaths,
        "cells": cells.len(), "best_x": best_x, "start_x": start_pos.x,
        "path_frames": path_frames, "tas_furthest_x": tas_x,
        "holds": {"hop": hop, "mid": mid, "full": full}, "kept_moves": picks,
    });
    // Saved: the winning path, or the furthest cell's, as a clip.
    if a.save {
        let (l, how) = match &won {
            Some(l) => (l.clone(), "left the level"),
            None => {
                let c = cells.values().max_by_key(|c| c.pos.x).expect("a cell");
                (c.log.clone(), "got furthest")
            }
        };
        let sess = format!("farm-{}", a.level.trim_start_matches('w').replace('_', "-"));
        let note = format!(
            "{} ({}): {how} in {rounds} rounds, x {best_x}",
            if uniform { "random order" } else { "Laya" },
            a.level
        );
        let n = save_clip(paths, game, &sess, &l, play_frame, &note, &bytes)?;
        v["session"] = json!(sess);
        v["clip"] = json!(n);
    }
    fs::write(
        out_dir.join(if uniform {
            "result-uniform.json"
        } else {
            "result.json"
        }),
        serde_json::to_string_pretty(&v)?,
    )?;
    let text = format!(
        "{} on {}: {} after {rounds} rounds in {secs:.0} s; furthest x {best_x} (from {}){}; {calls} decision calls, {sims} moves simulated, {deaths} deaths, {} cells",
        if uniform { "random order" } else { "Laya" },
        a.level,
        match &left_to {
            Some(n) => format!("left the level for {n}"),
            None => "didn't leave the level".to_owned(),
        },
        start_pos.x,
        tas_x
            .map(|t| format!("; the recording's furthest x in the level is {t}"))
            .unwrap_or_default(),
        cells.len()
    );
    Ok(Outcome::ok(v, text))
}

/// One line for a blind result.
fn blind_line(v: &Value) -> String {
    if let Some(e) = v["error"].as_str() {
        return format!(
            "failed at {}: {e}",
            v["stage"].as_str().unwrap_or("the search")
        );
    }
    format!(
        "{} level(s) in {} s, {} rounds, {} decision calls, {} deaths; furthest {} scene(s) on the path, x {}; {} scenes seen; player x {} y {}, forward {}",
        v["levels"],
        v["seconds"],
        v["rounds"],
        v["decision_calls"],
        v["deaths"],
        v["furthest"]["scenes_on_path"],
        v["furthest"]["x"],
        v["scenes_seen"],
        v["learned"]["x"].as_str().unwrap_or("?"),
        v["learned"]["y"].as_str().unwrap_or("?"),
        v["learned"]["forward"].as_str().unwrap_or("?")
    )
}

/// A clip in a mechanic session, in the person's clip format.
pub(crate) fn save_clip(
    paths: &Paths,
    game: &str,
    sess: &str,
    frames: &[InputFrame],
    start: usize,
    note: &str,
    rom: &[u8],
) -> Result<u64> {
    let dir: PathBuf = paths.home.join("mechanics").join(game).join(sess);
    fs::create_dir_all(dir.join("clips"))?;
    let st = dir.join("session.toml");
    if !st.is_file() {
        let img = gp_evidence::source::identify_nes(rom)?;
        let mut t = toml::Table::new();
        t.insert("game".into(), game.into());
        t.insert("mechanic".into(), sess.into());
        t.insert(
            "prompt".into(),
            "a level played by the decision model, by search".into(),
        );
        t.insert("rom".into(), img.sha256.into());
        fs::write(&st, toml::to_string(&t)?)?;
    }
    let n = fs::read_dir(dir.join("clips"))?
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "toml"))
        .count() as u64
        + 1;
    let mut log = InputLog::new(1);
    log.frames = frames.to_vec();
    fs::write(dir.join(format!("clips/{n:03}.log")), log.to_text())?;
    let mut t = toml::Table::new();
    t.insert("by".into(), "laya".into());
    t.insert("frames".into(), ((frames.len() - start) as i64).into());
    t.insert("note".into(), note.into());
    t.insert("start".into(), (start as i64).into());
    fs::write(dir.join(format!("clips/{n:03}.toml")), toml::to_string(&t)?)?;
    Ok(n)
}

/// The furthest player x a store recording reaches while in a level.
fn tas_furthest(
    paths: &Paths,
    rom: &[u8],
    player: &Player,
    level: &LevelKey,
    rec: &str,
    name: &str,
) -> Result<i64> {
    let store = gp_evidence::store::Store::open(&paths.store)?;
    let entry = gp_evidence::recordings::resolve(&store, rec)?;
    let manifest = gp_evidence::recordings::load_manifest(&store, &entry.id)?;
    let log = gp_evidence::input_log::InputLog::parse(
        &String::from_utf8(store.get(&manifest.input_log)?)
            .context("the recording's input log isn't text")?,
    )
    .map_err(|e| anyhow::anyhow!("the recording's input log: {e}"))?;
    let mut nes = gp_nes::Nes::power_on(rom, manifest.power_on)?;
    nes.set_video(false);
    let mut best = 0;
    let mut seen = false;
    for f in &log.frames {
        nes.step(*f)?;
        let w = &nes.wram()[..0x800];
        match level.of(w) {
            Some(l) if l == name => {
                seen = true;
                let hi = player.x_hi.map_or(0, |h| i64::from(w[usize::from(h)]));
                best = best.max(hi * 256 + i64::from(w[usize::from(player.x_lo)]));
            }
            Some(_) if seen => break,
            _ => {}
        }
    }
    Ok(best)
}
