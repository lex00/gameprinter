//! `gp video`: replays a recording with video rendering on and writes each
//! frame's picture to an animated PNG, with the frame number, the input and
//! the frame's events drawn in a band below it (docs/video.md).
//!
//! The replay is checked as it goes: each frame's hashes against the
//! recording's frame hashes, whether the game polled the controller against
//! the input log, and the frame's events against the event stream, which
//! was recorded headless. A frame that differs gets a red border and `DIFF`,
//! and the command exits 1 unless another runtime made the recording.
//!
//! Videos of commercial games are private, like their recordings, so the
//! command refuses to write inside a git work tree. By default it writes to
//! `$GP_HOME/videos/`.
//!
//! Ported from game-printer's video.rs; the animated PNG writer is
//! crate::theme_apng, which `gp theme video` already used. On an arcade
//! recording ([`render_arcade`]) the frames come from gp-arcade: the
//! board's picture (512 by 480, so `--scale` is 1 by default there), its
//! state checked against the recording's hashes and its events against the
//! stored stream, and the band shows the board's controls and events. The
//! board polls no controller, so there are no lag frames to mark. Themes
//! are the NES's and refused there. No run metrics are recorded.

mod font;

use std::fs;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use gp_evidence::Paths;
use gp_evidence::events::{Event, EventKind};
use gp_evidence::input_log::button;
use gp_evidence::manifest::Region;
use gp_nes::video::{HEIGHT, WIDTH};
use serde_json::json;

use crate::Outcome;
use crate::open::Opened;

/// What `gp video` was asked for.
pub(crate) struct Request<'a> {
    pub id: &'a str,
    pub out: Option<&'a Path>,
    pub frames: &'a str,
    pub scale: Option<u8>,
    pub overlay: bool,
    pub force: bool,
    pub rom: Option<&'a Path>,
    /// Theme directories, bottom first.
    pub themes: &'a [PathBuf],
}

const WHITE: [u8; 3] = [0xff, 0xff, 0xff];
const DIM: [u8; 3] = [0x50, 0x50, 0x50];
const BAND: [u8; 3] = [0x14, 0x14, 0x14];
const ORANGE: [u8; 3] = [0xff, 0xa0, 0x20];
const RED: [u8; 3] = [0xff, 0x30, 0x30];
const YELLOW: [u8; 3] = [0xff, 0xe0, 0x40];
const MAGENTA: [u8; 3] = [0xff, 0x50, 0xff];
const CYAN: [u8; 3] = [0x40, 0xe0, 0xff];

/// One line of overlay text, in pixels.
const LINE: usize = font::H + 2;

/// The frame delay as a fraction of a second that fits APNG's 16-bit
/// fields: NTSC runs at 39375000/655171 (60.0988) frames a second, PAL at
/// 50.0070.
fn frame_delay(region: Region) -> (u16, u16) {
    match region {
        Region::Ntsc => (1000, 60099),
        Region::Pal => (1000, 50007),
    }
}

/// An RGB image at one pixel per NES pixel.
struct Canvas {
    w: usize,
    h: usize,
    rgb: Vec<u8>,
}

impl Canvas {
    fn new(w: usize, h: usize) -> Self {
        Self {
            w,
            h,
            rgb: vec![0; w * h * 3],
        }
    }

    fn set(&mut self, x: usize, y: usize, c: [u8; 3]) {
        if x < self.w && y < self.h {
            let i = (y * self.w + x) * 3;
            self.rgb[i..i + 3].copy_from_slice(&c);
        }
    }

    fn fill(&mut self, x: usize, y: usize, w: usize, h: usize, c: [u8; 3]) {
        for yy in y..(y + h).min(self.h) {
            for xx in x..(x + w).min(self.w) {
                self.set(xx, yy, c);
            }
        }
    }

    /// Draws `text` from `x`, returning where the next character goes.
    fn text(&mut self, x: usize, y: usize, text: &str, c: [u8; 3]) -> usize {
        let mut x = x;
        for ch in text.chars() {
            for (row, bits) in font::glyph(ch).iter().enumerate() {
                for col in 0..font::W {
                    if bits & (1 << (font::W - 1 - col)) != 0 {
                        self.set(x + col, y + row, c);
                    }
                }
            }
            x += font::ADVANCE;
        }
        x
    }

    /// The image with each pixel repeated `scale` times both ways.
    fn scaled(&self, scale: usize, out: &mut Vec<u8>) {
        out.clear();
        let row_bytes = self.w * scale * 3;
        for y in 0..self.h {
            let start = out.len();
            for px in self.rgb[y * self.w * 3..(y + 1) * self.w * 3]
                .as_chunks::<3>()
                .0
                .iter()
            {
                for _ in 0..scale {
                    out.extend_from_slice(px);
                }
            }
            for _ in 1..scale {
                out.extend_from_within(start..start + row_bytes);
            }
        }
    }
}

/// What the overlay shows for one frame.
struct Overlay<'a> {
    frame: u64,
    pads: [u8; 4],
    controllers: u8,
    lag: bool,
    diff: bool,
    events: &'a [Event],
}

/// Lines of text in the band: the frame and port 1, port 2 when the log
/// has it, and the events.
fn band_lines(controllers: u8) -> usize {
    if controllers >= 2 { 3 } else { 2 }
}

fn band_height(controllers: u8) -> usize {
    band_lines(controllers) * LINE + 2
}

const BUTTONS: [(&str, u8); 8] = [
    ("U", button::UP),
    ("D", button::DOWN),
    ("L", button::LEFT),
    ("R", button::RIGHT),
    ("SE", button::SELECT),
    ("ST", button::START),
    ("B", button::B),
    ("A", button::A),
];

fn draw_pad(c: &mut Canvas, x: usize, y: usize, port: usize, pad: u8) -> usize {
    let mut x = c.text(x, y, &format!("P{port} "), WHITE);
    for (label, bit) in BUTTONS {
        let color = if pad & bit != 0 { WHITE } else { DIM };
        x = c.text(x, y, label, color) + font::ADVANCE;
    }
    x
}

fn draw_overlay(c: &mut Canvas, o: &Overlay<'_>) {
    // Event markers at the edges of the picture, on the scanline where
    // each happened: sprite 0 hits and IRQs on the left, bank switches on
    // the right.
    for e in o.events {
        let y = usize::from(e.scanline);
        if y >= HEIGHT {
            continue;
        }
        match e.kind {
            EventKind::Sprite0Hit => c.fill(0, y, 4, 1, YELLOW),
            EventKind::Irq => c.fill(0, y, 4, 1, MAGENTA),
            EventKind::BankSwitch => c.fill(WIDTH - 4, y, 4, 1, CYAN),
            _ => {}
        }
    }
    if o.diff {
        c.fill(0, 0, WIDTH, 1, RED);
        c.fill(0, HEIGHT - 1, WIDTH, 1, RED);
        c.fill(0, 0, 1, HEIGHT, RED);
        c.fill(WIDTH - 1, 0, 1, HEIGHT, RED);
    }
    let top = HEIGHT;
    c.fill(0, top, WIDTH, band_height(o.controllers), BAND);
    let mut y = top + 2;
    let x = c.text(2, y, &format!("F {:06}", o.frame), WHITE) + font::ADVANCE;
    let x = draw_pad(c, x, y, 1, o.pads[0]);
    let x = if o.lag {
        c.text(x, y, "LAG", ORANGE) + font::ADVANCE
    } else {
        x + 4 * font::ADVANCE
    };
    if o.diff {
        c.text(x, y, "DIFF", RED);
    }
    if o.controllers >= 2 {
        y += LINE;
        draw_pad(c, 2 + 9 * font::ADVANCE, y, 2, o.pads[1]);
    }
    y += LINE;
    let count = |k: EventKind| o.events.iter().filter(|e| e.kind == k).count();
    let mut x = 2;
    for (label, kind, color) in [
        ("NMI", EventKind::Nmi, WHITE),
        ("IRQ", EventKind::Irq, MAGENTA),
        ("S0", EventKind::Sprite0Hit, YELLOW),
        ("BNK", EventKind::BankSwitch, CYAN),
        ("PPU", EventKind::PpuWrite, WHITE),
        ("APU", EventKind::ApuWrite, WHITE),
    ] {
        x = c.text(x, y, label, color) + font::ADVANCE;
        x = c.text(x, y, &count(kind).to_string(), WHITE) + font::ADVANCE;
    }
}

/// Where the video goes: as given, or `$GP_HOME/videos/<id>.png`. Refuses a
/// path inside a git work tree, another format, and an existing file
/// unless `force`.
fn output_path(req: &Request<'_>, paths: &Paths, id: &str) -> Result<PathBuf> {
    let out = match req.out {
        Some(p) => std::path::absolute(p).with_context(|| format!("resolving {}", p.display()))?,
        None => paths.home.join("videos").join(format!("{}.png", &id[..12])),
    };
    match out.extension().and_then(|e| e.to_str()) {
        Some("png" | "apng") => {}
        _ => bail!(
            "{} isn't a .png or .apng file; gp video writes animated PNG (docs/video.md)",
            out.display()
        ),
    }
    if let Some(repo) = gp_evidence::paths::enclosing_git_work_tree(&out) {
        bail!(
            "{} is inside the git work tree at {}; videos stay outside any repo, like recordings (by default in $GP_HOME/videos)",
            out.display(),
            repo.display()
        );
    }
    if out.exists() && !req.force {
        bail!(
            "{} already exists; pass --force to replace it",
            out.display()
        );
    }
    Ok(out)
}

/// How to draw a video.
pub(crate) struct Style<'a> {
    pub scale: u8,
    pub overlay: bool,
    /// A theme stack drawn over the console's picture.
    pub theme: Option<&'a gp_theme::Resolved>,
}

/// What [`render`] wrote.
pub(crate) struct Rendered {
    pub frames: u64,
    pub bytes: u64,
    pub width: usize,
    pub height: usize,
    pub frames_per_second: f64,
    /// Whether another runtime made the recording, so it was replayed from
    /// power-on.
    pub stale: bool,
    /// The first frame that differs from the recording, and how.
    pub first_diff: Option<(u64, &'static str)>,
}

/// Replays `frames` of a recording with video rendering on and writes
/// them to `out` as an animated PNG, checking each frame against the
/// recording. Writes to a temporary file beside `out` and renames it, so
/// `out` is never left half written.
pub(crate) fn render(
    store: &gp_evidence::Store,
    manifest: &gp_evidence::manifest::RecordingManifest,
    rom: &[u8],
    frames: std::ops::Range<u64>,
    out: &Path,
    style: &Style<'_>,
) -> Result<Rendered> {
    let theme_overlay = style
        .theme
        .and_then(|t| t.overlay(WIDTH as u32, HEIGHT as u32));
    let emitters = style.theme.map(|t| t.emitters()).unwrap_or_default();
    let range = frames;
    if range.start == 0 {
        bail!("frame 0 is power-on, which has no picture; start --frames at 1");
    }
    let scale = usize::from(style.scale);
    let mut cursor = gp_nes::seek::open(store, manifest, rom, range.start - 1)
        .with_context(|| format!("opening the recording at frame {}", range.start - 1))?;
    let stale = cursor.stale;
    let recorded_events = gp_nes::recording::stored_events(store, manifest, range.clone())?;
    let mut by_frame: Vec<Vec<Event>> = vec![Vec::new(); (range.end - range.start) as usize];
    for e in recorded_events.iter().flatten() {
        by_frame[(e.frame - range.start) as usize].push(*e);
    }
    let any_p2 = range
        .clone()
        .any(|f| cursor.input(f - 1).is_some_and(|i| i.pads[1] != 0));
    let controllers = if any_p2 { 2 } else { 1 };
    {
        let nes = cursor.nes_mut();
        nes.set_video(true);
        nes.set_events(true);
    }

    let (w, h) = if style.overlay {
        (WIDTH, HEIGHT + band_height(controllers))
    } else {
        (WIDTH, HEIGHT)
    };
    let dir = out.parent().context("the output path has no folder")?;
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let partial = dir.join(format!(
        ".{}.partial-{}",
        out.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id()
    ));
    let file =
        fs::File::create(&partial).with_context(|| format!("writing {}", partial.display()))?;
    let frames = range.end - range.start;
    let started = Instant::now();
    let written = (|| -> Result<Option<(u64, &'static str)>> {
        let mut apng = crate::theme_apng::Apng::new(
            BufWriter::new(file),
            w * scale,
            h * scale,
            u32::try_from(frames).context("too many frames for one video")?,
            frame_delay(manifest.power_on.region),
        )?;
        let mut picture = Box::new([0u8; WIDTH * HEIGHT * 3]);
        let mut canvas = Canvas::new(w, h);
        let mut scaled = Vec::new();
        let mut first_diff = None;
        for frame in range.clone() {
            let input = cursor
                .input(frame - 1)
                .expect("the range is inside the recording");
            let polled = cursor.step()?.polled;
            let events = cursor.nes_mut().take_events();
            let what = if polled != input.polled {
                Some("polled")
            } else if gp_nes::seek::matches(cursor.nes(), cursor.hashes())? == Some(false) {
                Some("state")
            } else if recorded_events.is_some()
                && by_frame[(frame - range.start) as usize] != events
            {
                Some("events")
            } else {
                None
            };
            if let (Some(what), None) = (what, first_diff) {
                first_diff = Some((frame, what));
            }
            cursor.nes().frame_rgb(&mut picture);
            if let Some(t) = style.theme {
                theme_picture(
                    t,
                    &mut picture[..],
                    frame,
                    theme_overlay.as_ref(),
                    &emitters,
                );
            }
            canvas.rgb[..picture.len()].copy_from_slice(&picture[..]);
            if style.overlay {
                draw_overlay(
                    &mut canvas,
                    &Overlay {
                        frame,
                        pads: input.pads,
                        controllers,
                        lag: !polled,
                        diff: what.is_some(),
                        events: &events,
                    },
                );
            }
            canvas.scaled(scale, &mut scaled);
            apng.frame(&scaled)?;
        }
        apng.finish()?;
        Ok(first_diff)
    })();
    let first_diff = match written {
        Ok(d) => d,
        Err(e) => {
            let _ = fs::remove_file(&partial);
            return Err(e);
        }
    };
    fs::rename(&partial, out).with_context(|| format!("writing {}", out.display()))?;
    let secs = started.elapsed().as_secs_f64();
    Ok(Rendered {
        frames,
        bytes: fs::metadata(out)?.len(),
        width: w * scale,
        height: h * scale,
        frames_per_second: frames as f64 / secs.max(1e-9),
        stale,
        first_diff,
    })
}

/// The arcade board's frame rate as APNG's delay: 30 frames a second.
const ARCADE_DELAY: (u16, u16) = (1, 30);

/// The board's controls, bit 0 first, as the band labels them.
const ARCADE_BUTTONS: [&str; 8] = ["C", "ST", "R", "L", "D", "U", "T", "TL"];

fn draw_arcade_overlay(c: &mut Canvas, frame: u64, buttons: u32, diff: bool, events: &[Event]) {
    let (w, h) = (gp_arcade::video::WIDTH, gp_arcade::video::HEIGHT);
    // The main CPU's interrupts on the left at their line, the sound
    // CPU's on the right.
    for e in events {
        let y = usize::from(e.scanline);
        if y >= h {
            continue;
        }
        match e.kind {
            EventKind::Irq => c.fill(0, y, 6, 1, MAGENTA),
            EventKind::SoundIrq => c.fill(w - 6, y, 6, 1, CYAN),
            _ => {}
        }
    }
    if diff {
        c.fill(0, 0, w, 2, RED);
        c.fill(0, h - 2, w, 2, RED);
        c.fill(0, 0, 2, h, RED);
        c.fill(w - 2, 0, 2, h, RED);
    }
    c.fill(0, h, w, band_height(1), BAND);
    let y = h + 2;
    let mut x = c.text(2, y, &format!("F {frame:06}"), WHITE) + font::ADVANCE;
    for (k, label) in ARCADE_BUTTONS.iter().enumerate() {
        let color = if buttons & (1 << k) != 0 { WHITE } else { DIM };
        x = c.text(x, y, label, color) + font::ADVANCE;
    }
    if diff {
        c.text(x + font::ADVANCE, y, "DIFF", RED);
    }
    let y = y + LINE;
    let count = |k: EventKind| events.iter().filter(|e| e.kind == k).count();
    let mut x = 2;
    for (label, kind, color) in [
        ("IRQ", EventKind::Irq, MAGENTA),
        ("SIRQ", EventKind::SoundIrq, CYAN),
        ("LATCH", EventKind::SoundLatch, YELLOW),
        ("AY", EventKind::AyWrite, WHITE),
        ("PORT", EventKind::PortRead, WHITE),
        ("WD", EventKind::Watchdog, WHITE),
    ] {
        x = c.text(x, y, label, color) + font::ADVANCE;
        x = c.text(x, y, &count(kind).to_string(), WHITE) + font::ADVANCE;
    }
}

/// [`render`] for an arcade recording: the frames replayed on gp-arcade
/// from the nearest checkpoint, each frame's picture as the board drew it,
/// its state checked against the recording's hashes and its events against
/// the stored stream.
pub(crate) fn render_arcade(
    store: &gp_evidence::Store,
    manifest: &gp_evidence::manifest::RecordingManifest,
    set: &gp_arcade::Romset,
    range: std::ops::Range<u64>,
    out: &Path,
    style: &Style<'_>,
) -> Result<Rendered> {
    use gp_arcade::video::{HEIGHT as AH, WIDTH as AW};
    if range.start == 0 {
        bail!("frame 0 is power-on, which has no picture; start --frames at 1");
    }
    let scale = usize::from(style.scale);
    let mut cursor = crate::arcade_rec::open(store, manifest, set, range.start - 1)
        .with_context(|| format!("opening the recording at frame {}", range.start - 1))?;
    let stale = cursor.stale;
    let recorded = crate::arcade_rec::stored_events(store, manifest, range.clone())?;
    let mut by_frame: Vec<Vec<Event>> = vec![Vec::new(); (range.end - range.start) as usize];
    for e in recorded.iter().flatten() {
        by_frame[(e.frame - range.start) as usize].push(*e);
    }
    cursor.tapper_mut().set_events(true);
    let (w, h) = if style.overlay {
        (AW, AH + band_height(1))
    } else {
        (AW, AH)
    };
    let dir = out.parent().context("the output path has no folder")?;
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let partial = dir.join(format!(
        ".{}.partial-{}",
        out.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id()
    ));
    let file =
        fs::File::create(&partial).with_context(|| format!("writing {}", partial.display()))?;
    let frames = range.end - range.start;
    let started = Instant::now();
    let written = (|| -> Result<Option<(u64, &'static str)>> {
        let mut apng = crate::theme_apng::Apng::new(
            BufWriter::new(file),
            w * scale,
            h * scale,
            u32::try_from(frames).context("too many frames for one video")?,
            ARCADE_DELAY,
        )?;
        let mut canvas = Canvas::new(w, h);
        let mut scaled = Vec::new();
        let mut first_diff = None;
        for frame in range.clone() {
            let input = cursor
                .input(frame - 1)
                .expect("the range is inside the recording");
            cursor.step()?;
            let events: Vec<Event> = cursor
                .tapper_mut()
                .take_events()
                .iter()
                .map(|e| crate::arcade_rec::stored(frame, e))
                .collect();
            let what = if !cursor.matches_recording() {
                Some("state")
            } else if recorded.is_some() && by_frame[(frame - range.start) as usize] != events {
                Some("events")
            } else {
                None
            };
            if let (Some(what), None) = (what, first_diff) {
                first_diff = Some((frame, what));
            }
            for (i, px) in cursor.tapper().picture().iter().enumerate() {
                canvas.rgb[i * 3..i * 3 + 3].copy_from_slice(&[
                    (px >> 16) as u8,
                    (px >> 8) as u8,
                    *px as u8,
                ]);
            }
            if style.overlay {
                draw_arcade_overlay(
                    &mut canvas,
                    frame,
                    input.buttons_word(),
                    what.is_some(),
                    &events,
                );
            }
            canvas.scaled(scale, &mut scaled);
            apng.frame(&scaled)?;
        }
        apng.finish()?;
        Ok(first_diff)
    })();
    let first_diff = match written {
        Ok(d) => d,
        Err(e) => {
            let _ = fs::remove_file(&partial);
            return Err(e);
        }
    };
    fs::rename(&partial, out).with_context(|| format!("writing {}", out.display()))?;
    let secs = started.elapsed().as_secs_f64();
    Ok(Rendered {
        frames,
        bytes: fs::metadata(out)?.len(),
        width: w * scale,
        height: h * scale,
        frames_per_second: frames as f64 / secs.max(1e-9),
        stale,
        first_diff,
    })
}

/// A theme on the console's picture: `*` palette rules (the pixels carry no
/// roles), screen particles, then the overlay. Presentation only: the
/// emulator's state was hashed before this runs.
fn theme_picture(
    theme: &gp_theme::Resolved,
    rgb: &mut [u8],
    frame: u64,
    overlay: Option<&gp_theme::pix::Rgba>,
    emitters: &[(Option<String>, gp_theme::particles::Emitter)],
) {
    use gp_theme::color::Rgb;
    let mut img = gp_theme::pix::Rgba::new(WIDTH as u32, HEIGHT as u32);
    for (p, c) in img.pixels.iter_mut().zip(rgb.as_chunks::<3>().0) {
        let Rgb([r, g, b]) = theme.color(None, 0, Rgb(*c));
        *p = [r, g, b, 255];
    }
    let dots = emitters
        .iter()
        .filter(|(role, _)| role.is_none())
        .flat_map(|(_, e)| gp_theme::particles::dots(e, frame, [WIDTH as f32, HEIGHT as f32], &[]))
        .collect();
    let out = gp_theme::compose::frame(&gp_theme::compose::Scene {
        view: [WIDTH as u32, HEIGHT as u32],
        clear: [0, 0, 0],
        level: Some(&img),
        camera: [0, 0],
        sprites: Vec::new(),
        dots,
        overlay,
    });
    for (c, p) in rgb.as_chunks_mut::<3>().0.iter_mut().zip(&out.pixels) {
        *c = [p[0], p[1], p[2]];
    }
}

pub(crate) fn run(req: &Request<'_>, paths: &Paths) -> Result<Outcome> {
    let opened = Opened::open(paths, req.id)?;
    let manifest = &opened.manifest;
    let range = crate::recordings::parse_frames(req.frames, manifest.frames)?;
    if range.start == 0 {
        bail!("frame 0 is power-on, which has no picture; start --frames at 1");
    }
    let out = output_path(req, paths, &opened.entry.id.to_hex())?;
    if opened.is_arcade() {
        if !req.themes.is_empty() {
            bail!(
                "--theme draws on the NES's picture; an arcade recording's video has no theme yet"
            );
        }
        let set = opened.romset(req.rom, paths)?;
        let r = render_arcade(
            &opened.store,
            manifest,
            &set,
            range.clone(),
            &out,
            &Style {
                scale: req.scale.unwrap_or(1),
                overlay: req.overlay,
                theme: None,
            },
        )?;
        return Ok(outcome(req, &opened, &out, range, &r));
    }
    let rom = opened.rom(req.rom, paths)?.bytes;
    let mut theme = gp_theme::Resolved::none();
    for d in req.themes {
        theme
            .layers
            .push(gp_theme::stack::load(d, gp_theme::Scope::Personal).map_err(anyhow::Error::msg)?);
    }
    let r = render(
        &opened.store,
        manifest,
        &rom,
        range.clone(),
        &out,
        &Style {
            scale: req.scale.unwrap_or(2),
            overlay: req.overlay,
            theme: (!theme.is_empty()).then_some(&theme),
        },
    )?;
    Ok(outcome(req, &opened, &out, range, &r))
}

/// What `gp video` says of a render.
fn outcome(
    req: &Request<'_>,
    opened: &Opened,
    out: &Path,
    range: std::ops::Range<u64>,
    r: &Rendered,
) -> Outcome {
    let (frames, bytes, fps) = (r.frames, r.bytes, r.frames_per_second);
    let (last, matched) = (range.end - 1, r.first_diff.is_none());
    let mut text = format!(
        "wrote frames {} to {last} of {} to {}: {frames} frames, {bytes} bytes, {}x{} pixels, {fps:.0} frames/s",
        range.start,
        opened.short(),
        out.display(),
        r.width,
        r.height,
    );
    match (r.first_diff, r.stale) {
        (None, _) => text.push_str(
            "\nevery frame matched the recording: hashes, lag frames and events (rendered with video on, recorded headless)",
        ),
        (Some((frame, what)), true) => text.push_str(&format!(
            "\nanother runtime made this recording, so it was replayed from power-on; frame {frame} differs ({what}), marked DIFF"
        )),
        (Some((frame, what)), false) => text.push_str(&format!(
            "\nframe {frame} differs from the recording ({what}); every frame that differs is marked DIFF"
        )),
    }
    Outcome {
        json: json!({
            "recording": opened.entry.id,
            "out": out,
            "first": range.start,
            "last": last,
            "frames": frames,
            "bytes": bytes,
            "width": r.width,
            "height": r.height,
            "overlay": req.overlay,
            "frames_per_second": fps,
            "stale": r.stale,
            "matched": matched,
            "divergence": r.first_diff.map(|(frame, what)| json!({ "frame": frame, "what": what })),
        }),
        text,
        exit: u8::from(!matched && !r.stale),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaling_repeats_pixels_both_ways() {
        let mut c = Canvas::new(2, 1);
        c.set(1, 0, [1, 2, 3]);
        let mut out = Vec::new();
        c.scaled(2, &mut out);
        assert_eq!(
            out,
            [
                0, 0, 0, 0, 0, 0, 1, 2, 3, 1, 2, 3, //
                0, 0, 0, 0, 0, 0, 1, 2, 3, 1, 2, 3,
            ]
        );
    }
}
