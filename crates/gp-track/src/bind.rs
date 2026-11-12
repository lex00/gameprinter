//! The first two of the tracker's passes: sprites into tracks, then each
//! track bound to the RAM bytes that hold its position (docs/formats/
//! tracks.md, "Binding").
//!
//! A byte holds an object's x if, on almost every frame the object is
//! drawn, the byte minus the object's screen x minus the scroll is the same
//! number mod 256 (the distance from the object's position to its box's
//! edge). Twelve frames spread over the track pick the candidates; a full
//! pass counts how many of the track's frames each one holds on.

use serde::Serialize;

use crate::sprites::{Obs, Track, Tracker, clusters, group};
use crate::{Frame, Options, RAM, Source};

/// What the first pass keeps: the tracks, and per frame the scroll, the
/// controller and which tracks were drawn.
pub struct Seen {
    pub first: u64,
    pub frames: usize,
    pub tracks: Vec<Track>,
    pub scroll: Vec<u16>,
    pub pad: Vec<u8>,
    /// For each frame (by index from `first`), the tracks drawn in it and
    /// the index of that frame's observation in each.
    pub by_frame: Vec<Vec<(u32, u32)>>,
    /// For each address, how many frames it went up by exactly 1.
    pub plus_one: Vec<u32>,
    /// RAM pages (0 to 7) that equal the sprite table on most frames: the
    /// buffer the game copies to the PPU, whose bytes are screen positions.
    pub sprite_pages: Vec<u16>,
}

impl Seen {
    pub fn index(&self, frame: u64) -> usize {
        (frame - self.first) as usize
    }
}

pub fn pass_sprites(source: &mut dyn Source, opts: &Options) -> Result<Seen, String> {
    let mut tracker = Tracker::new(opts.max_gap);
    let mut first = None;
    let mut next = 0u64;
    let mut scroll = Vec::new();
    let mut pad = Vec::new();
    let mut plus_one = vec![0u32; RAM];
    let mut prev: Option<Vec<u8>> = None;
    let mut broken = None;
    let mut page_equal = [0u32; 8];
    source.scan(&mut |f: &Frame<'_>| {
        if first.is_none() {
            first = Some(f.frame);
            next = f.frame;
        }
        if f.frame != next && broken.is_none() {
            broken = Some(f.frame);
        }
        next = f.frame + 1;
        match f.sprites {
            Some(list) => tracker.push(f.frame, group(list)),
            None => tracker.push(f.frame, clusters(f.oam, f.tall)),
        }
        // The page the game copies its sprite table from, when the table is
        // the NES's.
        if f.oam.len() >= 256 {
            for (p, n) in page_equal.iter_mut().enumerate() {
                *n += u32::from(f.ram.get(p * 256..p * 256 + 256) == Some(&f.oam[..256]));
            }
        }
        scroll.push(f.scroll_x);
        pad.push(f.pad);
        if let Some(p) = &prev {
            for (n, (a, b)) in plus_one.iter_mut().zip(p.iter().zip(f.ram)) {
                *n += u32::from(b.wrapping_sub(*a) == 1);
            }
        }
        prev = Some(f.ram[..RAM].to_vec());
    })?;
    if let Some(frame) = broken {
        return Err(format!(
            "the frames skip to {frame}; the tracker needs every frame in order"
        ));
    }
    let first = first.unwrap_or(0);
    let frames = scroll.len();
    let sprite_pages = (0..8u16)
        .filter(|p| frames > 0 && page_equal[usize::from(*p)] as usize * 2 > frames)
        .collect();
    let tracks = tracker.finish();
    let mut by_frame = vec![Vec::new(); frames];
    for (t, track) in tracks.iter().enumerate() {
        for (o, obs) in track.obs.iter().enumerate() {
            by_frame[(obs.frame - first) as usize].push((t as u32, o as u32));
        }
    }
    Ok(Seen {
        first,
        frames,
        tracks,
        scroll,
        pad,
        by_frame,
        plus_one,
        sprite_pages,
    })
}

/// The scroll an object's x is read against: none, the PPU's scroll at the
/// same frame, or at the next frame (a game that writes the scroll in the
/// frame after it moves the camera).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Camera {
    Ppu,
    PpuNext,
    None,
}

/// Which edge of the object's box the byte follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Anchor {
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Opt {
    anchor: Anchor,
    camera: Camera,
}

const X_OPTS: [Opt; 6] = [
    Opt {
        anchor: Anchor::Left,
        camera: Camera::Ppu,
    },
    Opt {
        anchor: Anchor::Right,
        camera: Camera::Ppu,
    },
    Opt {
        anchor: Anchor::Left,
        camera: Camera::PpuNext,
    },
    Opt {
        anchor: Anchor::Right,
        camera: Camera::PpuNext,
    },
    Opt {
        anchor: Anchor::Left,
        camera: Camera::None,
    },
    Opt {
        anchor: Anchor::Right,
        camera: Camera::None,
    },
];
const Y_OPTS: [Opt; 2] = [
    Opt {
        anchor: Anchor::Top,
        camera: Camera::None,
    },
    Opt {
        anchor: Anchor::Bottom,
        camera: Camera::None,
    },
];

impl Opt {
    /// The screen position plus the scroll, mod 256.
    fn value(self, obs: &Obs, seen: &Seen) -> u8 {
        let edge = match self.anchor {
            Anchor::Left => obs.x0,
            Anchor::Right => obs.x1,
            Anchor::Top => obs.y0,
            Anchor::Bottom => obs.y1,
        };
        (i32::from(edge) + camera_at(self.camera, obs.frame, seen)) as u8
    }
}

/// The scroll the camera option reads at `frame`, 0 to 511.
pub fn camera_at(camera: Camera, frame: u64, seen: &Seen) -> i32 {
    let i = seen.index(frame);
    match camera {
        Camera::None => 0,
        Camera::Ppu => i32::from(seen.scroll[i]),
        Camera::PpuNext => i32::from(seen.scroll[(i + 1).min(seen.frames - 1)]),
    }
}

/// A track's position byte.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Binding {
    pub ram: u16,
    pub anchor: Anchor,
    /// The byte minus the anchor (minus the scroll, for x), mod 256.
    pub residual: u8,
    /// The share of the track's frames on which the residual holds.
    pub score: f64,
    /// Other bytes that hold as well, within 0.02.
    pub ties: Vec<u16>,
    /// For x: of the frames the controller held Left or Right, the share
    /// the byte moved the way it says.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<f64>,
    /// For x: of the frames the byte changed while one of them was held,
    /// the share it moved the way the controller says.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_agree: Option<f64>,
}

/// A track's bindings, and the camera they were read against.
pub struct Bound {
    pub camera: Camera,
    pub x: Vec<Option<Binding>>,
    pub y: Vec<Option<Binding>>,
    /// Frames sampled across the whole range (by index), with their RAM,
    /// for the slot search.
    pub samples: Vec<(usize, Vec<u8>)>,
    /// Each track's twelve sampled frames' RAM (empty for short tracks).
    pub track_samples: Vec<Vec<Vec<u8>>>,
}

/// Frames sampled from each track to find candidate bytes.
const TRACK_SAMPLES: usize = 12;
/// A candidate must hold on this many of them.
const SAMPLE_HOLDS: usize = 10;
/// Candidates kept per track and axis option, best first.
const MAX_CANDIDATES: usize = 24;
/// A binding must hold on this share of a track's frames.
pub const MIN_SCORE: f64 = 0.9;
/// Frames sampled across the range for the slot search.
const RANGE_SAMPLES: usize = 4096;

struct Cand {
    addr: u16,
    opt: Opt,
    residual: u8,
    holds: u32,
    input_agree: u32,
    input_total: u32,
    input_held: u32,
}

pub fn bind(source: &mut dyn Source, opts: &Options, seen: &Seen) -> Result<Bound, String> {
    // Which frames to sample: twelve per long track, and a spread over
    // the range.
    let mut wanted: Vec<Vec<(u32, u8)>> = vec![Vec::new(); seen.frames];
    for (t, track) in seen.tracks.iter().enumerate() {
        if track.obs.len() < opts.min_frames {
            continue;
        }
        let n = track.obs.len();
        for s in 0..TRACK_SAMPLES {
            let o = &track.obs[s * (n - 1) / (TRACK_SAMPLES - 1)];
            wanted[seen.index(o.frame)].push((t as u32, s as u8));
        }
    }
    let step = seen.frames.div_ceil(RANGE_SAMPLES).max(1);
    let mut track_samples: Vec<Vec<Vec<u8>>> = seen
        .tracks
        .iter()
        .map(|t| {
            if t.obs.len() < opts.min_frames {
                Vec::new()
            } else {
                vec![Vec::new(); TRACK_SAMPLES]
            }
        })
        .collect();
    let mut samples = Vec::new();
    let mut i = 0usize;
    source.scan(&mut |f: &Frame<'_>| {
        for &(t, s) in &wanted[i] {
            track_samples[t as usize][s as usize] = f.ram[..RAM].to_vec();
        }
        if i.is_multiple_of(step) {
            samples.push((i, f.ram[..RAM].to_vec()));
        }
        i += 1;
    })?;

    // Candidates from the samples.
    let mut cands: Vec<Vec<Cand>> = Vec::with_capacity(seen.tracks.len());
    for (t, track) in seen.tracks.iter().enumerate() {
        let mut out = Vec::new();
        if !track_samples[t].is_empty() {
            let n = track.obs.len();
            let obs: Vec<&Obs> = (0..TRACK_SAMPLES)
                .map(|s| &track.obs[s * (n - 1) / (TRACK_SAMPLES - 1)])
                .collect();
            // A track that keeps its place on the screen while the view
            // scrolls is drawn over the view, as a HUD is (Moss Hop's
            // counters): its x is no byte's but the scroll's, so it isn't
            // bound across.
            let fixed = obs.iter().all(|o| (o.x0, o.x1) == (obs[0].x0, obs[0].x1))
                && obs.iter().any(|o| {
                    camera_at(Camera::Ppu, o.frame, seen)
                        != camera_at(Camera::Ppu, obs[0].frame, seen)
                });
            for opt in X_OPTS.iter().chain(Y_OPTS.iter()) {
                if fixed && is_x(*opt) {
                    continue;
                }
                let values: Vec<u8> = obs.iter().map(|o| opt.value(o, seen)).collect();
                if distinct(&values) < 3 {
                    continue;
                }
                let mut found = Vec::new();
                for addr in 0..RAM {
                    if opts.excluded(addr) {
                        continue;
                    }
                    let mut hist = [0u8; 256];
                    for (row, v) in track_samples[t].iter().zip(&values) {
                        hist[usize::from(row[addr].wrapping_sub(*v))] += 1;
                    }
                    let (residual, holds) = hist
                        .iter()
                        .enumerate()
                        .max_by_key(|(r, n)| (**n, std::cmp::Reverse(*r)))
                        .map(|(r, n)| (r as u8, usize::from(*n)))
                        .unwrap();
                    if holds >= SAMPLE_HOLDS {
                        found.push((holds, addr, residual));
                    }
                }
                found.sort_by_key(|&(holds, addr, _)| (std::cmp::Reverse(holds), addr));
                out.extend(
                    found
                        .into_iter()
                        .take(MAX_CANDIDATES)
                        .map(|(_, addr, residual)| Cand {
                            addr: addr as u16,
                            opt: *opt,
                            residual,
                            holds: 0,
                            input_agree: 0,
                            input_total: 0,
                            input_held: 0,
                        }),
                );
            }
        }
        cands.push(out);
    }

    // The full pass: how many frames each candidate holds on, and for x,
    // whether it moves the way the controller says.
    let mut prev: Vec<u8> = Vec::new();
    let mut i = 0usize;
    source.scan(&mut |f: &Frame<'_>| {
        let pad = seen.pad[i];
        let dir: i32 = match (pad & 0x80 != 0, pad & 0x40 != 0) {
            (true, false) => 1,
            (false, true) => -1,
            _ => 0,
        };
        for &(t, o) in &seen.by_frame[i] {
            let track = &seen.tracks[t as usize];
            let obs = &track.obs[o as usize];
            let follows = o > 0 && track.obs[o as usize - 1].frame + 1 == obs.frame;
            for c in &mut cands[t as usize] {
                let a = usize::from(c.addr);
                if f.ram[a].wrapping_sub(c.opt.value(obs, seen)) == c.residual {
                    c.holds += 1;
                }
                if follows && dir != 0 && !prev.is_empty() && is_x(c.opt) {
                    let d = f.ram[a].wrapping_sub(prev[a]) as i8;
                    c.input_held += 1;
                    if d != 0 {
                        c.input_total += 1;
                        c.input_agree += u32::from(i32::from(d).signum() == dir);
                    }
                }
            }
        }
        prev.clear();
        prev.extend_from_slice(&f.ram[..RAM]);
        i += 1;
    })?;

    // One camera for the whole range: the PPU's scroll of this frame or the
    // next, whichever binds more tracked frames. A byte that holds the
    // screen position itself (a game keeps those for its renderer) binds
    // with no scroll at all, so that camera is used only when neither
    // scroll binds anything.
    let frames_under = |cam: Camera| -> usize {
        seen.tracks
            .iter()
            .zip(&cands)
            .filter(|(track, cs)| {
                cs.iter().any(|c| {
                    is_x(c.opt)
                        && c.opt.camera == cam
                        && f64::from(c.holds) / track.obs.len() as f64 >= MIN_SCORE
                })
            })
            .map(|(track, _)| track.obs.len())
            .sum()
    };
    let (ppu, next) = (frames_under(Camera::Ppu), frames_under(Camera::PpuNext));
    let camera = if ppu == 0 && next == 0 {
        (Camera::None, 0)
    } else if next > ppu {
        (Camera::PpuNext, next)
    } else {
        (Camera::Ppu, ppu)
    };
    let camera = camera.0;

    let pick = |track: &Track, cs: &[Cand], x: bool| -> Option<Binding> {
        let n = track.obs.len() as f64;
        let ok: Vec<&Cand> = cs
            .iter()
            .filter(|c| is_x(c.opt) == x && (!x || c.opt.camera == camera))
            .filter(|c| f64::from(c.holds) / n >= MIN_SCORE)
            .collect();
        let best = ok
            .iter()
            .max_by(|a, b| a.holds.cmp(&b.holds).then(b.addr.cmp(&a.addr)))?;
        let score = f64::from(best.holds) / n;
        let mut ties: Vec<u16> = ok
            .iter()
            .filter(|c| c.addr != best.addr && f64::from(c.holds) / n >= score - 0.02)
            .map(|c| c.addr)
            .collect();
        ties.sort_unstable();
        ties.dedup();
        Some(Binding {
            ram: best.addr,
            anchor: best.opt.anchor,
            residual: best.residual,
            score,
            ties,
            input: (x && best.input_held > 0)
                .then(|| f64::from(best.input_agree) / f64::from(best.input_held)),
            input_agree: (x && best.input_total > 0)
                .then(|| f64::from(best.input_agree) / f64::from(best.input_total)),
        })
    };
    let x = seen
        .tracks
        .iter()
        .zip(&cands)
        .map(|(t, cs)| pick(t, cs, true))
        .collect();
    let y = seen
        .tracks
        .iter()
        .zip(&cands)
        .map(|(t, cs)| pick(t, cs, false))
        .collect();
    Ok(Bound {
        camera,
        x,
        y,
        samples,
        track_samples,
    })
}

fn is_x(opt: Opt) -> bool {
    matches!(opt.anchor, Anchor::Left | Anchor::Right)
}

fn distinct(values: &[u8]) -> usize {
    let mut seen = [false; 256];
    values
        .iter()
        .filter(|v| !std::mem::replace(&mut seen[usize::from(**v)], true))
        .count()
}

/// The value an anchor reads from an observation, for the slot pass.
pub fn edge(obs: &Obs, anchor: Anchor) -> i32 {
    i32::from(match anchor {
        Anchor::Left => obs.x0,
        Anchor::Right => obs.x1,
        Anchor::Top => obs.y0,
        Anchor::Bottom => obs.y1,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth;

    #[test]
    fn tracks_bind_to_their_position_bytes() {
        let mut src = synth::game(1200);
        let opts = Options::default();
        let seen = pass_sprites(&mut src, &opts).unwrap();
        let bound = bind(&mut src, &opts, &seen).unwrap();
        assert_eq!(bound.camera, Camera::Ppu);
        let xs: std::collections::BTreeSet<u16> = bound.x.iter().flatten().map(|b| b.ram).collect();
        assert!(xs.contains(&(synth::PLAYER_X as u16)), "{xs:?}");
        assert!(xs.contains(&(synth::X as u16)), "{xs:?}");
        assert!(xs.iter().all(|a| *a == synth::PLAYER_X as u16
            || (synth::X..synth::X + synth::SLOTS).contains(&usize::from(*a))));
        let player = bound
            .x
            .iter()
            .flatten()
            .find(|b| b.ram == synth::PLAYER_X as u16)
            .unwrap();
        assert!(player.input.unwrap() > 0.9);
    }
}
