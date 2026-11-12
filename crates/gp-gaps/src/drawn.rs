//! The drawn-box check (`gp gaps drawn`, docs/gaps.md): where the model
//! draws each mapped object's sprite against where the original drew it.
//! The model's box is the presentation's (`gp_present::Animator::scene`):
//! a frame laid out from the position (a captured one) at its origin from
//! the body, any other with its left at the body's x and its bottom on the
//! body's bottom edge. The original's is the sprite object nearest it on
//! screen. The vertical difference is measured at the bottom edge, where
//! a sprite stands, since a game's frame may hold blank rows at its top
//! that are still in the sprite table. A difference that holds on most
//! frames is a systematic offset, such as enemies drawn a few pixels above
//! the ground. Report only.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::{Run, mean_sd, r2};

/// How the model draws an entity kind: its body's size and its picture's,
/// and the picture's top left from the body's position when the frame is
/// laid out from it (`gp_present::Present::origins`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ModelBox {
    pub body: [i32; 2],
    pub picture: [i32; 2],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<[i32; 2]>,
}

impl ModelBox {
    /// The picture's top left from the body's position.
    pub fn top_left(&self) -> [i32; 2] {
        self.origin.unwrap_or([0, self.body[1] - self.picture[1]])
    }
}

/// The farthest a sprite object may be from the model's box (the sum of
/// the left and top differences) to count as the same object.
pub const MATCH: i32 = 24;

#[derive(Debug, Clone, Serialize)]
pub struct KindReport {
    pub kind: String,
    pub frames: u64,
    pub matched: u64,
    /// Model box minus original box, pixels: left, bottom (as `dy`),
    /// width, height, each as mean and spread (standard deviation).
    pub dx: [f64; 2],
    pub dy: [f64; 2],
    pub dw: [f64; 2],
    pub dh: [f64; 2],
    /// The median of each difference (left, bottom, width, height), and the
    /// share of matched frames within a pixel of it.
    pub median: [f64; 4],
    pub at_median: [f64; 4],
    /// Differences whose median is at least a pixel and that hold within a
    /// pixel on at least 60% of the matched frames: an offset that pairing
    /// mistakes (two objects drawn as one) don't explain.
    pub systematic: Vec<String>,
    pub model: Option<ModelBox>,
    /// Frames with the largest |dy|.
    pub samples: Vec<u64>,
    /// Frames left out of the differences because the nearest sprite
    /// object was near another mapped object's box too (two objects drawn
    /// as one), or its size wasn't the kind's usual one.
    #[serde(default)]
    pub composite_frames: u64,
    /// The offset as a detected fact: model minus original, left and bottom,
    /// the medians of the frames whose object has the kind's usual size,
    /// and the share of those within a pixel of both.
    #[serde(default)]
    pub offset: [f64; 2],
    #[serde(default)]
    pub offset_share: f64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Report {
    pub schema: u32,
    pub frames: usize,
    pub kinds: Vec<KindReport>,
}

impl Report {
    /// Kinds with a systematic difference.
    pub fn off(&self) -> usize {
        self.kinds
            .iter()
            .filter(|k| !k.systematic.is_empty())
            .count()
    }
}

/// Runs the check. `boxes` are by [`Run::kinds`]; a kind without one isn't
/// drawn by the model and isn't checked.
pub fn check(run: &Run, boxes: &[Option<ModelBox>]) -> Report {
    // Per kind: frames drawn, differences of matched frames, and |dy| by
    // frame for the samples.
    type Per = (u64, Vec<[f64; 4]>, Vec<(i64, u64)>, Vec<(bool, [i16; 2])>);
    let mut per: BTreeMap<u16, Per> = BTreeMap::new();
    for f in &run.frames {
        if !f.play {
            continue;
        }
        let mut used = vec![false; f.objects.len()];
        // Every drawn model box this frame, to see which original objects
        // more than one of them is near.
        let placed: Vec<(usize, u16, i32, i32)> = f
            .ents
            .iter()
            .enumerate()
            .filter_map(|(n, e)| {
                let kind = e.kind?;
                let b = boxes.get(usize::from(kind))?.as_ref()?;
                let [ox, oy] = b.top_left();
                let mx = e.x + ox - f.camera[0];
                let my = e.y + oy - f.camera[1];
                ((-16..272).contains(&mx) && (-16..256).contains(&my)).then_some((n, kind, mx, my))
            })
            .collect();
        let claimants = |o: &crate::Object| {
            placed
                .iter()
                .filter(|(_, _, mx, my)| {
                    (mx - i32::from(o.c.x0)).abs() + (my - i32::from(o.c.y0)).abs() <= MATCH
                })
                .count()
        };
        for &(_, kind, mx, my) in &placed {
            let b = boxes[usize::from(kind)].expect("placed");
            let entry = per.entry(kind).or_default();
            entry.0 += 1;
            let best = f
                .objects
                .iter()
                .enumerate()
                .filter(|(i, _)| !used[*i])
                .map(|(i, o)| {
                    let d = (mx - i32::from(o.c.x0)).abs() + (my - i32::from(o.c.y0)).abs();
                    (d, i)
                })
                .filter(|(d, _)| *d <= MATCH)
                .min();
            if let Some((_, i)) = best {
                used[i] = true;
                let o = &f.objects[i];
                let c = &o.c;
                let d = [
                    f64::from(mx - i32::from(c.x0)),
                    f64::from(my + b.picture[1] - i32::from(c.y1)),
                    f64::from(b.picture[0] - i32::from(c.x1 - c.x0)),
                    f64::from(b.picture[1] - i32::from(c.y1 - c.y0)),
                ];
                entry.2.push(((d[1].abs() * 100.0) as i64, f.frame));
                entry.1.push(d);
                entry.3.push((claimants(o) > 1, [c.x1 - c.x0, c.y1 - c.y0]));
            }
        }
    }
    let mut kinds = Vec::new();
    for (kind, (frames, ds, mut samples, shapes)) in per {
        // The kind's usual object size, and the frames whose object has it
        // and no other mapped object near: those measure the offset.
        let mut sizes: BTreeMap<[i16; 2], u64> = BTreeMap::new();
        for (_, sz) in &shapes {
            *sizes.entry(*sz).or_default() += 1;
        }
        let usual = sizes.iter().max_by_key(|(_, n)| **n).map(|(s, _)| *s);
        let clean: Vec<&[f64; 4]> = ds
            .iter()
            .zip(&shapes)
            .filter(|(_, (composite, sz))| !composite && Some(*sz) == usual)
            .map(|(d, _)| d)
            .collect();
        let median_of = |j: usize| {
            let mut v: Vec<f64> = clean.iter().map(|d| d[j]).collect();
            v.sort_by(f64::total_cmp);
            v.get(v.len() / 2).copied().unwrap_or(0.0)
        };
        let offset = [median_of(0), median_of(1)];
        let offset_share = r2(clean
            .iter()
            .filter(|d| (d[0] - offset[0]).abs() <= 1.0 && (d[1] - offset[1]).abs() <= 1.0)
            .count() as f64
            / clean.len().max(1) as f64);
        let composite_frames = (ds.len() - clean.len()) as u64;
        let col = |j: usize| {
            let v: Vec<f64> = ds.iter().map(|d| d[j]).collect();
            let (m, s) = mean_sd(&v);
            [r2(m), r2(s)]
        };
        let (dx, dy, dw, dh) = (col(0), col(1), col(2), col(3));
        let mut median = [0.0; 4];
        let mut at_median = [0.0; 4];
        for j in 0..4 {
            let mut v: Vec<f64> = ds.iter().map(|d| d[j]).collect();
            v.sort_by(f64::total_cmp);
            let m = v.get(v.len() / 2).copied().unwrap_or(0.0);
            median[j] = m;
            at_median[j] =
                r2(v.iter().filter(|x| (*x - m).abs() <= 1.0).count() as f64
                    / v.len().max(1) as f64);
        }
        let mut systematic = Vec::new();
        if ds.len() >= 10 {
            for (j, name) in ["dx", "dy", "dw", "dh"].iter().enumerate() {
                if median[j].abs() >= 1.0 && at_median[j] >= 0.6 {
                    systematic.push(format!("{name} {:+}", median[j]));
                }
            }
        }
        samples.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        kinds.push(KindReport {
            kind: run.kinds[usize::from(kind)].clone(),
            frames,
            matched: ds.len() as u64,
            dx,
            dy,
            dw,
            dh,
            median,
            at_median,
            systematic,
            model: boxes[usize::from(kind)],
            samples: samples.iter().take(5).map(|s| s.1).collect(),
            composite_frames,
            offset,
            offset_share,
        });
    }
    kinds.sort_by(|a, b| {
        b.systematic
            .len()
            .cmp(&a.systematic.len())
            .then(b.matched.cmp(&a.matched))
    });
    Report {
        schema: crate::SCHEMA,
        frames: run.frames.len(),
        kinds,
    }
}
