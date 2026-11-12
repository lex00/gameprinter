//! Object tracking from traces (Milestone 2, step 7; docs/formats/tracks.md).
//!
//! From every frame's RAM and sprite table, without a hand-written state
//! mapping, this finds the objects a game draws and where it keeps them:
//!
//! 1. Sprites are grouped into objects within each frame and followed from
//!    frame to frame by position, palette, tiles and motion ([`sprites`]).
//! 2. Each long enough track is bound to the RAM bytes that hold its
//!    position: a byte whose value minus the object's screen position (plus
//!    the scroll, for x) stays the same on almost every frame ([`bind`]).
//! 3. The bound bytes are grouped into object slots: runs of addresses at
//!    one stride that share an active flag, with a page byte, a y byte and
//!    an object ID found the same way ([`slots`]). The player is the bound
//!    object that moves the way the controller says.
//! 4. Each track gets its speed and period ([`motion`]), and the result can
//!    be scored against a hand-written mapping ([`truth`]).
//!
//! The crate reads frames through [`Source`], so it needs neither the store
//! nor a runtime: `gp track` feeds it a recording's trace.

pub mod behaviour;
pub mod bind;
pub mod motion;
pub mod slots;
pub mod sprites;
pub mod truth;

use serde::Serialize;

/// Internal RAM, the bytes the tracker searches.
pub const RAM: usize = 2048;

/// One frame as the tracker reads it.
pub struct Frame<'a> {
    pub frame: u64,
    /// Internal RAM, 2048 bytes.
    pub ram: &'a [u8],
    /// The sprite table drawn for this frame's state, 256 bytes.
    pub oam: &'a [u8],
    /// PPUCTRL bit 5: sprites are 8x16.
    pub tall: bool,
    /// The horizontal scroll as the PPU holds it: 0 to 511, the nametable
    /// bit as bit 8.
    pub scroll_x: u16,
    /// Controller 1 as the frame ran with it, A in bit 0 through Right in
    /// bit 7.
    pub pad: u8,
}

/// Frames to track, in order, with no frame missing. Each call to `scan`
/// goes through the same frames again; the tracker makes four passes.
pub trait Source {
    fn scan(&mut self, each: &mut dyn FnMut(&Frame<'_>)) -> Result<(), String>;
}

/// A source of frames held in memory, for tests and small inputs.
pub struct Frames {
    pub first: u64,
    pub ram: Vec<Vec<u8>>,
    pub oam: Vec<Vec<u8>>,
    pub tall: bool,
    pub scroll_x: Vec<u16>,
    pub pad: Vec<u8>,
}

impl Source for Frames {
    fn scan(&mut self, each: &mut dyn FnMut(&Frame<'_>)) -> Result<(), String> {
        for i in 0..self.ram.len() {
            each(&Frame {
                frame: self.first + i as u64,
                ram: &self.ram[i],
                oam: &self.oam[i],
                tall: self.tall,
                scroll_x: self.scroll_x[i],
                pad: self.pad[i],
            });
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct Options {
    /// RAM addresses never proposed: the sprite table's page, whose bytes
    /// are the screen positions themselves.
    pub exclude: Vec<std::ops::Range<u16>>,
    /// Frames an object may go undrawn and keep its track.
    pub max_gap: u64,
    /// Frames a track needs before it's bound to RAM.
    pub min_frames: usize,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            exclude: Vec::new(),
            max_gap: 4,
            min_frames: 16,
        }
    }
}

impl Options {
    pub fn excluded(&self, addr: usize) -> bool {
        self.exclude
            .iter()
            .any(|r| (r.start as usize..r.end as usize).contains(&addr))
    }
}

/// What the tracker found.
#[derive(Debug, Serialize)]
pub struct Report {
    pub first: u64,
    pub last: u64,
    /// The scroll the x positions were read against.
    pub camera: bind::Camera,
    pub tracks: Vec<TrackReport>,
    pub player: Option<slots::Player>,
    pub groups: Vec<slots::Group>,
    pub clock: Option<slots::Clock>,
    pub kinds: Vec<motion::Kind>,
}

/// One tracked object.
#[derive(Debug, Serialize)]
pub struct TrackReport {
    pub track: usize,
    /// The first and last frames it was drawn in, and how many it was drawn in.
    pub spawn: u64,
    pub despawn: u64,
    pub seen: usize,
    pub palette: u8,
    pub tiles: Vec<u8>,
    pub x: Option<bind::Binding>,
    pub y: Option<bind::Binding>,
    /// Its slot group and index, when its x byte is in one.
    pub slot: Option<(usize, usize)>,
    pub player: bool,
    /// Its object ID, read from its slot's ID byte.
    pub id: Option<u8>,
    pub motion: motion::Motion,
    /// `[frame, x, y]`: from RAM where the track is bound (x with its page
    /// when the page byte is known), otherwise the box's left and top on
    /// screen.
    pub positions: Vec<[i64; 3]>,
}

/// Runs the whole tracker over `source`.
pub fn track(source: &mut dyn Source, opts: &Options) -> Result<Report, String> {
    let seen = bind::pass_sprites(source, opts)?;
    if seen.frames == 0 {
        return Err("no frames to track".into());
    }
    let mut opts = opts.clone();
    for p in &seen.sprite_pages {
        opts.exclude.push(p * 256..p * 256 + 256);
    }
    let opts = &opts;
    let bound = bind::bind(source, opts, &seen)?;
    let found = slots::find(source, opts, &seen, &bound)?;
    let mut tracks = Vec::new();
    for (i, t) in seen.tracks.iter().enumerate() {
        let positions = found.positions[i].clone();
        tracks.push(TrackReport {
            track: i,
            spawn: t.first(),
            despawn: t.last(),
            seen: t.obs.len(),
            palette: t.palette,
            tiles: t.tiles.clone(),
            x: bound.x[i].clone(),
            y: bound.y[i].clone(),
            slot: found.slot_of[i],
            player: found.player_tracks.contains(&i),
            id: found.ids[i],
            motion: motion::measure(&positions),
            positions,
        });
    }
    let kinds = motion::kinds(&tracks);
    Ok(Report {
        first: seen.first,
        last: seen.first + seen.frames as u64 - 1,
        camera: bound.camera,
        tracks,
        player: found.player,
        groups: found.groups,
        clock: found.clock,
        kinds,
    })
}

#[cfg(test)]
pub(crate) mod synth {
    //! A made-up game for the tests: a player the controller moves, and
    //! enemy slots in a struct of arrays, drawn as 16x16 objects through a
    //! scrolling camera. No real game's layout.

    use super::*;

    pub const PLAYER_X: usize = 0x40;
    pub const PLAYER_PAGE: usize = 0x41;
    pub const PLAYER_Y: usize = 0x42;
    pub const CAMERA: usize = 0x44;
    pub const CLOCK: usize = 0x05;
    pub const SLOTS: usize = 3;
    pub const ACTIVE: usize = 0x120;
    pub const ID: usize = 0x130;
    pub const X: usize = 0x300;
    pub const PAGE: usize = 0x310;
    pub const Y: usize = 0x320;

    struct Enemy {
        x: i32,
        y: i32,
        dx: i32,
        id: u8,
        left: i32,
        right: i32,
    }

    /// `frames` frames: the player walks right for most of them, pushing the
    /// camera, and each slot holds a pacing enemy that's replaced now and
    /// then by one of another kind.
    pub fn game(frames: usize) -> Frames {
        let mut out = Frames {
            first: 1,
            ram: Vec::new(),
            oam: Vec::new(),
            tall: false,
            scroll_x: Vec::new(),
            pad: Vec::new(),
        };
        let mut ram = vec![0u8; RAM];
        let mut px: i32 = 40;
        let mut cam: i32 = 0;
        let mut enemies: Vec<Option<Enemy>> = (0..SLOTS).map(|_| None).collect();
        let mut noise: u32 = 7;
        for f in 0..frames {
            noise = noise.wrapping_mul(1_103_515_245).wrapping_add(12345);
            // Noise bytes that change all the time.
            for (a, b) in ram.iter_mut().enumerate().take(0x540).skip(0x500) {
                *b = (noise >> (a % 24)) as u8;
            }
            let right = (f / 90) % 4 != 3;
            let pad = if right { 0x80 } else { 0x40 };
            px += if right { 2 } else { -1 };
            if px - cam > 120 {
                cam = px - 120;
            }
            // Enemies spawn ahead of the camera in a free slot.
            for (i, e) in enemies.iter_mut().enumerate() {
                let gone = e.as_ref().is_some_and(|e| e.x < cam - 32 || f % 400 == 399);
                if gone {
                    *e = None;
                }
                if e.is_none() && (f + i * 37) % 60 == 0 {
                    let x = cam + 200 + 16 * i as i32;
                    *e = Some(Enemy {
                        x,
                        y: 120 + 20 * i as i32,
                        dx: -1,
                        id: [3, 7, 9][(f / 60 + i) % 3],
                        left: x - 40,
                        right: x,
                    });
                }
                if let Some(e) = e {
                    if f % 2 == 0 {
                        e.x += e.dx;
                    }
                    if e.x <= e.left || e.x >= e.right {
                        e.dx = -e.dx;
                    }
                    e.y = 120 + 20 * i as i32 + ((f as i32 / 8) % 4);
                }
            }
            ram[CLOCK] = ram[CLOCK].wrapping_add(1);
            ram[PLAYER_X] = px as u8;
            ram[PLAYER_PAGE] = (px >> 8) as u8;
            ram[PLAYER_Y] = 150;
            ram[CAMERA] = cam as u8;
            let mut oam = vec![0xF8u8; 256];
            let mut n = 0;
            let mut draw = |x: i32, y: i32, tile: u8, pal: u8, oam: &mut Vec<u8>| {
                for (dx, dy) in [(0, 0), (8, 0), (0, 8), (8, 8)] {
                    let sx = x + dx;
                    if !(0..=248).contains(&sx) || n >= 64 {
                        continue;
                    }
                    oam[n * 4] = (y + dy - 1) as u8;
                    oam[n * 4 + 1] = tile;
                    oam[n * 4 + 2] = pal;
                    oam[n * 4 + 3] = sx as u8;
                    n += 1;
                }
            };
            draw(px - cam, 150, 1, 0, &mut oam);
            for (i, e) in enemies.iter().enumerate() {
                match e {
                    Some(e) => {
                        ram[ACTIVE + i] = 1;
                        ram[ID + i] = e.id;
                        ram[X + i] = e.x as u8;
                        ram[PAGE + i] = (e.x >> 8) as u8;
                        ram[Y + i] = e.y as u8;
                        draw(e.x - cam, e.y, 0x10 + e.id, 1 + e.id % 3, &mut oam);
                    }
                    None => ram[ACTIVE + i] = 0,
                }
            }
            out.ram.push(ram.clone());
            out.oam.push(oam);
            out.scroll_x.push((cam & 511) as u16);
            out.pad.push(pad);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_made_up_game_s_slots_are_found() {
        let mut src = synth::game(2400);
        let report = track(&mut src, &Options::default()).unwrap();
        let player = report.player.as_ref().expect("a player");
        assert_eq!(player.x.ram, synth::PLAYER_X as u16);
        assert_eq!(
            player.page.as_ref().map(|p| p.ram),
            Some(synth::PLAYER_PAGE as u16)
        );
        assert_eq!(report.groups.len(), 1, "{:#?}", report.groups);
        let g = &report.groups[0];
        assert_eq!(g.count, synth::SLOTS);
        assert_eq!(g.stride, 1);
        assert_eq!(g.x.ram, synth::X as u16);
        assert_eq!(g.x_page.as_ref().map(|p| p.ram), Some(synth::PAGE as u16));
        assert_eq!(g.y.as_ref().map(|p| p.ram), Some(synth::Y as u16));
        assert_eq!(g.active.as_ref().map(|p| p.ram), Some(synth::ACTIVE as u16));
        assert_eq!(g.id.as_ref().map(|p| p.ram), Some(synth::ID as u16));
        assert_eq!(
            report.clock.as_ref().map(|c| c.ram),
            Some(synth::CLOCK as u16)
        );
        // An enemy crawls half a pixel a frame and turns every 80 frames.
        let enemy = report
            .tracks
            .iter()
            .filter(|t| t.slot.is_some())
            .max_by_key(|t| t.seen)
            .unwrap();
        let speed = enemy.motion.speed_x.unwrap();
        assert!((speed - 0.5).abs() < 0.05, "{speed}");
    }
}
