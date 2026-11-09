//! The screen measure's pictures for a lockstep of the original in gp-nes
//! and the model (gp_interp::screen). The original's picture is the
//! emulator's frame and the pixels its sprites cover; the model's is its
//! presentation drawn on the CPU (gp_present::raster), on the host, from a
//! world decoded from the module's canonical state and the pictures
//! gp-look draws. The tool's own drawing code decides the score.
//!
//! Decoding the pictures is the expensive part, so the last model's are
//! kept, by the assets folder and the model's text.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::Result;

use gp_interp::lockstep::Screens;
use gp_interp::screen::Options;
use gp_nes::protocol::NesRuntime;
use gp_present::raster::{self, Frame, Image};
use gp_present::{Animator, Present};

use crate::hosted::Model;

/// A model's presentation with its pictures decoded.
pub(crate) struct Art {
    pub(crate) present: Present,
    pub(crate) pics: Vec<Option<Image>>,
    /// Each HUD field's name: "the score".
    fields: Vec<String>,
}

/// The last model's pictures, by the assets folder and the model's text.
static ART: Mutex<Option<(String, Arc<Art>)>> = Mutex::new(None);

fn decode(png: &[u8]) -> Result<Image> {
    let mut d = png::Decoder::new(std::io::Cursor::new(png));
    d.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut r = d.read_info()?;
    let mut buf = vec![
        0;
        r.output_buffer_size()
            .ok_or_else(|| anyhow::anyhow!("a picture too big to decode"))?
    ];
    let info = r.next_frame(&mut buf)?;
    let n = info.color_type.samples();
    let px = buf[..info.buffer_size()]
        .chunks(n)
        .map(|c| match n {
            1 => [c[0], c[0], c[0], 255],
            2 => [c[0], c[0], c[0], c[1]],
            3 => [c[0], c[1], c[2], 255],
            _ => [c[0], c[1], c[2], c[3]],
        })
        .collect();
    Ok(Image {
        w: info.width,
        h: info.height,
        px,
    })
}

/// The model's art from its lifted assets in `assets`, decoded once.
pub(crate) fn art(model: &gp_model::Model, spec: &gp_sim::Spec, assets: &Path) -> Result<Arc<Art>> {
    let key = format!(
        "{}\n{}",
        assets.display(),
        gp_evidence::Hash::of(model.to_toml().as_bytes())
    );
    let lock = || ART.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((k, a)) = lock().as_ref()
        && *k == key
    {
        return Ok(a.clone());
    }
    let art = gp_look::present::art(model, spec, assets).map_err(anyhow::Error::msg)?;
    let by_file: BTreeMap<&str, &[u8]> = art
        .pictures
        .iter()
        .map(|p| (p.file.as_str(), p.png.as_slice()))
        .collect();
    let pics = art
        .present
        .pictures
        .iter()
        .map(|p| by_file.get(p.file.as_str()).map(|b| decode(b)).transpose())
        .collect::<Result<Vec<_>>>()?;
    let fields = art
        .present
        .hud
        .as_ref()
        .map(|h| {
            h.fields
                .iter()
                .map(|f| match &f.source {
                    gp_present::Source::Counter(i) => spec
                        .counters
                        .get(*i)
                        .map_or_else(String::new, |c| format!("the {}", c.name)),
                    gp_present::Source::Level => "the level's name".to_owned(),
                    gp_present::Source::Text(t) => format!("\"{}\"", t.trim()),
                })
                .collect()
        })
        .unwrap_or_default();
    let a = Arc::new(Art {
        present: art.present,
        pics,
        fields,
    });
    *lock() = Some((key, a.clone()));
    Ok(a)
}

/// The original's frame: the emulator's picture, in the palette the
/// generator draws the model's pictures with (`nes_colors`), so the same
/// NES color is the same RGB on both sides; and its sprites' pixels.
pub(crate) fn original_frame(o: &NesRuntime) -> Frame {
    use gp_nes::video::{HEIGHT, WIDTH};
    let colors = nes_palette();
    Frame {
        w: WIDTH as u32,
        h: HEIGHT as u32,
        rgb: o
            .nes()
            .frame_colors()
            .iter()
            .map(|c| colors[usize::from(*c)])
            .collect(),
        sprite: o.nes().sprite_mask(),
        visible: [0, 0, WIDTH as u32, HEIGHT as u32],
        things: Vec::new(),
    }
}

/// The NES colours by index, as the generator draws the model's pictures.
pub(crate) fn nes_palette() -> &'static [[u8; 3]; 64] {
    static COLORS: std::sync::OnceLock<[[u8; 3]; 64]> = std::sync::OnceLock::new();
    COLORS.get_or_init(gp_look::present::nes_colors)
}

/// An original that shows a picture: the emulator, or its side of a clip
/// played back from a tape (`crate::tape`).
pub(crate) trait Shows: gp_runtime::Runtime {
    /// The picture it shows now.
    fn shown(&self) -> Frame;
    /// Draws the frames it steps from now on.
    fn draw(&mut self);
}

impl Shows for NesRuntime {
    fn shown(&self) -> Frame {
        original_frame(self)
    }

    fn draw(&mut self) {
        self.set_video(true);
    }
}

impl Shows for crate::tape::Original {
    fn shown(&self) -> Frame {
        self.picture()
    }

    fn draw(&mut self) {
        crate::tape::Original::draw(self);
    }
}

/// The model's scene as a runtime draws it by default: with the console's
/// sprite limit when the model has one (`gp_present::view::limit`), so
/// the sprites the original drops on a crowded line are dropped here too.
pub(crate) fn model_scene(art: &Art, anim: &Animator, m: &Model) -> gp_present::Scene {
    use gp_present::view;
    let (spec, w) = (m.spec(), m.world());
    let mut scene = anim.scene(&art.present, spec, w);
    if art.present.screen.sprite_limit.is_some() {
        let (level_w, wall) = view::level_edges(spec, w);
        let win = view::window(&art.present.screen, scene.camera, None, level_w, wall);
        view::limit(&art.present, &mut scene, &win, w.frame);
    }
    scene
}

/// The model's frame: its scene drawn with its pictures.
pub(crate) fn model_frame(art: &Art, anim: &Animator, m: &Model) -> Frame {
    let spec = m.spec();
    let w = m.world();
    let scene = model_scene(art, anim, m);
    let names: Vec<String> = w
        .bodies
        .iter()
        .map(|b| {
            spec.kinds
                .get(b.kind)
                .map(|k| k.name.clone())
                .unwrap_or_default()
        })
        .collect();
    raster::draw(&art.present, w, &art.pics, &scene, &names, &art.fields)
}

/// The two pictures for a lockstep: the model's animation steps with it.
pub(crate) struct Probe {
    art: Arc<Art>,
    anim: Animator,
    opts: Options,
    /// The model's last pictures, newest last, for the original's lag.
    back: std::collections::VecDeque<Frame>,
    /// The original's frame from which its pictures are drawn: the frame
    /// before the first compared one. Before it the emulator runs without
    /// drawing, which is faster.
    pub(crate) video_from: u64,
}

/// Starts each body's animation where the original's is, from a
/// mapping's `present.<kind>[i].frame` and `.timer` values: the `i`th
/// body of that kind gets them ([`Animator::seed`]).
pub(crate) fn seed(
    anim: &mut Animator,
    present: &Present,
    spec: &gp_sim::Spec,
    w: &gp_sim::World,
    values: &[(String, i64)],
) {
    let mut by_body: std::collections::BTreeMap<usize, (Option<u32>, Option<u32>)> =
        std::collections::BTreeMap::new();
    for (path, v) in values {
        let Ok(p) = gp_model::Path::parse(path) else {
            continue;
        };
        let (Some(kind), Some(gp_model::path::Index::At(k)), Some(part)) = (
            p.segments.get(1).map(|s| s.name.as_str()),
            p.segments.get(1).and_then(|s| s.index.clone()),
            p.segments.get(2).map(|s| s.name.as_str()),
        ) else {
            continue;
        };
        let Some(ki) = spec.kinds.iter().position(|x| x.name == kind) else {
            continue;
        };
        let Some(body) = usize::try_from(k).ok().and_then(|k| {
            w.bodies
                .iter()
                .enumerate()
                .filter(|(_, b)| b.kind == ki)
                .nth(k)
                .map(|(i, _)| i)
        }) else {
            continue;
        };
        let v = u32::try_from((*v).max(0)).unwrap_or(u32::MAX);
        let e = by_body.entry(body).or_default();
        match part {
            "frame" => e.0 = Some(v),
            "timer" => e.1 = Some(v),
            _ => {}
        }
    }
    for (body, (frame, timer)) in by_body {
        anim.seed(present, w, body, frame, timer);
    }
}

/// The screen measure's options for the NES: exact colors, no pixels
/// allowed, and the picture one frame behind the RAM.
pub(crate) fn nes_options() -> Options {
    Options {
        lag: 1,
        ..Options::default()
    }
}

impl Probe {
    pub(crate) fn new(art: Arc<Art>, opts: Options) -> Self {
        Self {
            art,
            anim: Animator::new(),
            opts,
            back: std::collections::VecDeque::new(),
            video_from: 0,
        }
    }
}

impl<O: Shows> Screens<O, Model> for Probe {
    fn model_changed(&mut self, model: &Model, loaded: bool) {
        // As the preview does: a loaded state starts the animation over.
        if loaded {
            self.anim = Animator::new();
        }
        self.anim
            .step(&self.art.present, model.spec(), model.world());
    }

    fn model_events(&mut self, model: &Model, events: &[gp_runtime::Event]) {
        let spec = model.spec();
        let kinds: Vec<(usize, Option<usize>)> = events
            .iter()
            .filter_map(|e| {
                let k = spec.events.iter().position(|n| *n == e.kind)?;
                Some((k, e.entity.map(|i| i as usize)))
            })
            .collect();
        if !kinds.is_empty() {
            self.anim
                .fired_kinds(&self.art.present, spec, model.world(), &kinds);
        }
    }

    fn shots(&mut self, original: &O, model: &Model) -> Result<Option<(Frame, Frame)>, String> {
        let o = original.shown();
        // The model's picture `lag` compared frames ago; none at the start
        // of a run, where the original shows a frame before it.
        self.back
            .push_back(model_frame(&self.art, &self.anim, model));
        if self.back.len() <= usize::from(self.opts.lag) {
            return Ok(None);
        }
        while self.back.len() > usize::from(self.opts.lag) + 1 {
            self.back.pop_front();
        }
        let m = self.back.pop_front().expect("a picture was just kept");
        Ok(Some((o, m)))
    }

    fn options(&self) -> Options {
        self.opts
    }

    fn animation(&mut self, model: &Model, values: &[(String, i64)]) {
        seed(
            &mut self.anim,
            &self.art.present,
            model.spec(),
            model.world(),
            values,
        );
    }

    fn original_stepping(&mut self, original: &mut O) {
        // The frame about to be stepped is the next one.
        if gp_runtime::Runtime::frame(original) + 2 >= self.video_from {
            original.draw();
        }
    }
}

/// The screen measure as the lockstep report gives it: each layer's
/// counts, and each frame's distance and boxes (at most
/// [`gp_interp::screen::BOX_CAP`] a frame).
pub(crate) fn json(s: &gp_interp::screen::Screen, frames: &[(u64, u64)]) -> serde_json::Value {
    use serde_json::json;
    let layer = |l: &gp_interp::screen::Layer| {
        let at = |k: Option<usize>| {
            k.and_then(|k| frames.get(k))
                .map(|(m, o)| json!({"frame": m, "original_frame": o}))
        };
        json!({
            "name": l.name,
            "owner": l.owner,
            "tolerance": l.tolerance,
            "within": l.within,
            "skipped": l.skipped,
            "compared": l.frames.len(),
            "max": l.max,
            "first_split": at(l.first_split),
            "first_drift": at(l.first_drift),
            "first_shift": at(l.first_shift),
            "frames": l.frames.iter().map(|f| {
                let mut j = json!({
                    "distance": f.distance(&l.name),
                    "pixels": f.pixels,
                    "boxes": f.regions,
                });
                if f.more > 0 {
                    j["more"] = json!(f.more);
                }
                if let Some(sh) = f.shift {
                    j["shift"] = json!(sh);
                }
                if let Some(n) = f.shifted {
                    j["shifted"] = json!(n);
                }
                j
            }).collect::<Vec<_>>(),
        })
    };
    json!({
        "visible": s.visible,
        "area": s.area,
        "color": s.color,
        "background": layer(&s.background),
        "sprites": layer(&s.sprites),
    })
}

/// A layer's line for the lockstep report.
pub(crate) fn lines(
    s: &gp_interp::screen::Screen,
    frames: &[(u64, u64)],
    from: u64,
) -> Vec<String> {
    let sc = gp_interp::screen::score(s, frames, from);
    sc.says.iter().map(|l| format!("screen: {l}")).collect()
}
