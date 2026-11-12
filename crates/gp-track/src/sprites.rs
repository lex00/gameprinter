//! Sprites grouped into objects within a frame, and objects followed from
//! frame to frame (docs/formats/tracks.md, "Tracks").

use serde::Serialize;

/// Sprite memory hides a sprite by putting it below the screen: a Y of
/// 0xEF or more is never drawn on a 240-line picture.
const HIDDEN_Y: u8 = 0xEF;

/// Two sprites of one palette are one object when their boxes overlap or
/// are at most this many pixels apart on both axes.
const JOIN_GAP: i16 = 1;

/// One object's sprites in one frame: their bounding box on screen, in
/// pixels (`x1` and `y1` exclusive), their palette and their tiles.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cluster {
    pub x0: i16,
    pub y0: i16,
    pub x1: i16,
    pub y1: i16,
    pub palette: u8,
    pub tiles: Vec<u8>,
}

impl Cluster {
    fn center2(&self) -> (i32, i32) {
        (
            i32::from(self.x0) + i32::from(self.x1),
            i32::from(self.y0) + i32::from(self.y1),
        )
    }
}

/// Where a track's object was in one frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Obs {
    pub frame: u64,
    pub x0: i16,
    pub y0: i16,
    pub x1: i16,
    pub y1: i16,
}

/// One object followed through the frames it was drawn in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Track {
    pub palette: u8,
    /// Every tile the object was drawn with, sorted.
    pub tiles: Vec<u8>,
    pub obs: Vec<Obs>,
}

impl Track {
    pub fn first(&self) -> u64 {
        self.obs[0].frame
    }

    pub fn last(&self) -> u64 {
        self.obs[self.obs.len() - 1].frame
    }
}

/// The visible sprites of a 256-byte sprite table, grouped into objects:
/// sprites of one palette whose boxes touch are one object. `tall` is
/// PPUCTRL bit 5 (8x16 sprites).
pub fn clusters(oam: &[u8], tall: bool) -> Vec<Cluster> {
    let h: i16 = if tall { 16 } else { 8 };
    let sprites: Vec<(i16, i16, u8, u8)> = oam
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|s| s[0] < HIDDEN_Y)
        .map(|s| (i16::from(s[3]), i16::from(s[0]) + 1, s[1], s[2] & 3))
        .collect();
    let n = sprites.len();
    let mut parent: Vec<usize> = (0..n).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for i in 0..n {
        for j in i + 1..n {
            let (ax, ay, _, ap) = sprites[i];
            let (bx, by, _, bp) = sprites[j];
            if ap == bp
                && ax <= bx + 8 + JOIN_GAP
                && bx <= ax + 8 + JOIN_GAP
                && ay <= by + h + JOIN_GAP
                && by <= ay + h + JOIN_GAP
            {
                let (ri, rj) = (root(&mut parent, i), root(&mut parent, j));
                parent[ri.max(rj)] = ri.min(rj);
            }
        }
    }
    let mut out: Vec<(usize, Cluster)> = Vec::new();
    for (i, &(x, y, tile, palette)) in sprites.iter().enumerate() {
        let r = root(&mut parent, i);
        match out.iter_mut().find(|(k, _)| *k == r) {
            Some((_, c)) => {
                c.x0 = c.x0.min(x);
                c.y0 = c.y0.min(y);
                c.x1 = c.x1.max(x + 8);
                c.y1 = c.y1.max(y + h);
                if !c.tiles.contains(&tile) {
                    c.tiles.push(tile);
                }
            }
            None => out.push((
                r,
                Cluster {
                    x0: x,
                    y0: y,
                    x1: x + 8,
                    y1: y + h,
                    palette,
                    tiles: vec![tile],
                },
            )),
        }
    }
    out.into_iter()
        .map(|(_, mut c)| {
            c.tiles.sort_unstable();
            c
        })
        .collect()
}

/// How far an object may move from where its track predicts it, in pixels
/// (the sum of the horizontal and vertical distance of the box centers),
/// plus [`GAP_SLACK`] for each frame it went unseen.
const MAX_JUMP: i32 = 16;
const GAP_SLACK: i32 = 4;

/// Follows objects through frames. Feed it each frame's clusters in order
/// with [`Tracker::push`], then [`Tracker::finish`].
pub struct Tracker {
    max_gap: u64,
    live: Vec<Live>,
    done: Vec<Track>,
}

struct Live {
    track: Track,
    /// The last frame's box center, doubled, and its change per frame.
    center2: (i32, i32),
    velocity2: (i32, i32),
}

impl Tracker {
    /// `max_gap` is how many frames in a row an object may go undrawn (by
    /// flicker, say) and keep its track.
    pub fn new(max_gap: u64) -> Self {
        Self {
            max_gap,
            live: Vec::new(),
            done: Vec::new(),
        }
    }

    pub fn push(&mut self, frame: u64, clusters: Vec<Cluster>) {
        // Tracks unseen for too long end here.
        let max_gap = self.max_gap;
        let (keep, ended): (Vec<Live>, Vec<Live>) = std::mem::take(&mut self.live)
            .into_iter()
            .partition(|l| frame - l.track.last() <= max_gap + 1);
        self.done.extend(ended.into_iter().map(|l| l.track));
        self.live = keep;

        // Every pairing a track's prediction allows, cheapest first.
        let mut pairs: Vec<(i32, usize, usize)> = Vec::new();
        for (t, l) in self.live.iter().enumerate() {
            let gap = (frame - l.track.last()) as i32;
            let px = l.center2.0 + l.velocity2.0 * gap;
            let py = l.center2.1 + l.velocity2.1 * gap;
            let limit = 2 * (MAX_JUMP + GAP_SLACK * (gap - 1));
            for (c, cl) in clusters.iter().enumerate() {
                if cl.palette != l.track.palette {
                    continue;
                }
                let (cx, cy) = cl.center2();
                let mut cost = (cx - px).abs() + (cy - py).abs();
                if cost > limit {
                    continue;
                }
                if cl
                    .tiles
                    .iter()
                    .any(|t| l.track.tiles.binary_search(t).is_ok())
                {
                    cost = (cost - 4).max(0);
                }
                pairs.push((cost, t, c));
            }
        }
        pairs.sort_unstable();
        let mut track_used = vec![false; self.live.len()];
        let mut cluster_used = vec![false; clusters.len()];
        for (_, t, c) in pairs {
            if track_used[t] || cluster_used[c] {
                continue;
            }
            track_used[t] = true;
            cluster_used[c] = true;
            let l = &mut self.live[t];
            let cl = &clusters[c];
            let gap = (frame - l.track.last()) as i32;
            let center = cl.center2();
            l.velocity2 = (
                (center.0 - l.center2.0) / gap,
                (center.1 - l.center2.1) / gap,
            );
            l.center2 = center;
            observe(&mut l.track, frame, cl);
        }
        for (c, cl) in clusters.into_iter().enumerate() {
            if cluster_used[c] {
                continue;
            }
            let mut track = Track {
                palette: cl.palette,
                tiles: Vec::new(),
                obs: Vec::new(),
            };
            observe(&mut track, frame, &cl);
            self.live.push(Live {
                track,
                center2: cl.center2(),
                velocity2: (0, 0),
            });
        }
    }

    /// Every track, ordered by the frame it starts on, then where.
    pub fn finish(mut self) -> Vec<Track> {
        self.done.extend(self.live.into_iter().map(|l| l.track));
        self.done
            .sort_by_key(|t| (t.first(), t.obs[0].x0, t.obs[0].y0, t.palette));
        self.done
    }
}

fn observe(track: &mut Track, frame: u64, cl: &Cluster) {
    track.obs.push(Obs {
        frame,
        x0: cl.x0,
        y0: cl.y0,
        x1: cl.x1,
        y1: cl.y1,
    });
    for t in &cl.tiles {
        if let Err(i) = track.tiles.binary_search(t) {
            track.tiles.insert(i, *t);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oam(sprites: &[(u8, u8, u8, u8)]) -> Vec<u8> {
        let mut o = vec![0xF8; 256];
        for (i, &(x, y, tile, attr)) in sprites.iter().enumerate() {
            o[i * 4..i * 4 + 4].copy_from_slice(&[y, tile, attr, x]);
        }
        o
    }

    #[test]
    fn touching_sprites_of_one_palette_are_one_object() {
        // A 2x2 object at (40, 100), another of palette 1 beside it, and a
        // third, of palette 0, 20 pixels away.
        let o = oam(&[
            (40, 99, 1, 0),
            (48, 99, 2, 0),
            (40, 107, 3, 0),
            (48, 107, 4, 0),
            (56, 99, 9, 1),
            (76, 99, 1, 0),
        ]);
        let c = clusters(&o, false);
        assert_eq!(c.len(), 3);
        assert_eq!((c[0].x0, c[0].y0, c[0].x1, c[0].y1), (40, 100, 56, 116));
        assert_eq!(c[0].tiles, vec![1, 2, 3, 4]);
        assert_eq!(c[1].palette, 1);
        assert_eq!(c[2].x0, 76);
    }

    #[test]
    fn hidden_sprites_are_left_out_and_tall_sprites_are_16_high() {
        let o = oam(&[(10, 0xEF, 1, 0), (10, 49, 2, 0)]);
        let c = clusters(&o, true);
        assert_eq!(c.len(), 1);
        assert_eq!((c[0].y0, c[0].y1), (50, 66));
    }

    #[test]
    fn objects_keep_their_tracks_through_motion_and_a_short_gap() {
        let mut t = Tracker::new(2);
        let at = |x: i16, p: u8| Cluster {
            x0: x,
            y0: 100,
            x1: x + 16,
            y1: 116,
            palette: p,
            tiles: vec![p],
        };
        for f in 1..=10u64 {
            let x = 10 + 2 * f as i16;
            let mut cs = vec![at(200 - x, 1)];
            // The first object isn't drawn on frames 5 and 6.
            if f != 5 && f != 6 {
                cs.push(at(x, 0));
            }
            t.push(f, cs);
        }
        let tracks = t.finish();
        assert_eq!(tracks.len(), 2, "{tracks:?}");
        assert_eq!(tracks.iter().map(|t| t.obs.len()).sum::<usize>(), 18);
        // Unseen for longer than the gap allows, an object starts a new track.
        let mut t = Tracker::new(1);
        t.push(1, vec![at(10, 0)]);
        t.push(4, vec![at(10, 0)]);
        assert_eq!(t.finish().len(), 2);
    }
}
