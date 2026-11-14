//! `gp clip screen`: the screen measure run on a corpus clip's original
//! against itself, the check that a measure, its sprite mask and a score
//! tape agree with the emulator before any model is held to them.
//!
//! The original is put at the clip's start (from the start cache `gp clip
//! score` keeps, made here when it isn't there), then plays the clip
//! twice: once live, writing a score tape (crate::tape), and once from
//! that tape. The live run is what this checks, so it never comes from a
//! kept tape. Each
//! frame's two pictures go through the screen measure
//! (gp_interp::screen) with the platform's options, and every frame must
//! measure 0 on both layers. It also counts the pixels the sprite mask
//! puts in the sprite layer, and with `--mask` writes one frame's picture
//! with its sprite pixels in red beside it, for a person to look at. The
//! picture shows the game's art, so it goes outside any repo.

use std::path::Path;

use anyhow::{Context as _, Result, bail};
use gp_evidence::Paths;
use gp_present::raster::Frame;
use gp_runtime::Runtime as _;
use serde_json::json;

use crate::Outcome;
use crate::original::OriginalRuntime;
use crate::screen::Shows as _;
use crate::tape::Original;

pub(crate) fn run(
    paths: &Paths,
    game: &Path,
    clip: &str,
    rom: Option<&Path>,
    mask: Option<(&Path, Option<u64>)>,
) -> Result<Outcome> {
    let c = crate::clip_score::corpus_clip(game, clip)?;
    let (log, start) = c.log()?;
    let bytes = crate::clip_score::rom_of(paths, game, rom)?;
    let inputs = crate::run::frames(&log);
    let mut o = to_start(paths, &bytes, &log, &inputs, start)?;
    o.set_video(true);
    let opts = crate::screen::options_for(o.platform());
    let own = &inputs[start..];

    let mut live = Original::recording(o.fork());
    let mut first: Vec<Frame> = Vec::with_capacity(own.len());
    for i in own {
        live.step(std::slice::from_ref(i))
            .map_err(anyhow::Error::msg)?;
        first.push(live.shown());
    }
    if live.missed() {
        bail!("the live run asked something a tape can't keep");
    }
    let tape = live.into_tape().context("the tape is still shared")?;
    let mut again = Original::replay(tape);
    let mut screen = gp_interp::screen::Screen::new(&opts);
    let mut sprite_px = 0u64;
    let mut frames_with_sprites = 0u64;
    for (k, i) in own.iter().enumerate() {
        again
            .step(std::slice::from_ref(i))
            .map_err(anyhow::Error::msg)?;
        let shown = again.shown();
        let n = first[k].sprite.iter().filter(|s| **s).count() as u64;
        sprite_px += n;
        frames_with_sprites += u64::from(n > 0);
        screen.push(&first[k], &shown);
    }
    if again.missed() {
        bail!("the tape's replay missed a frame or a picture");
    }
    let worst = screen
        .layers()
        .iter()
        .flat_map(|l| l.frames.iter().map(|f| f.distance(&l.name)))
        .max()
        .unwrap_or(0);
    let perfect = worst == 0;
    let mut lines = vec![
        format!(
            "{clip}: {} frames from frame {start}, the original live against its own tape, lag {}",
            own.len(),
            opts.lag
        ),
        format!(
            "screen: background {} of {} frames exact, sprites {} of {} exact; largest distance {worst}",
            screen.background.within,
            screen.background.frames.len(),
            screen.sprites.within,
            screen.sprites.frames.len(),
        ),
        format!("sprite mask: {sprite_px} pixels over {frames_with_sprites} frames with a sprite"),
    ];
    let mut j = json!({
        "clip": clip,
        "start": start,
        "frames": own.len(),
        "perfect": perfect,
        "largest_distance": worst,
        "sprite_pixels": sprite_px,
        "frames_with_sprites": frames_with_sprites,
    });
    if let Some((file, at)) = mask {
        let k = match at {
            Some(f) => usize::try_from(
                f.checked_sub(start as u64)
                    .context("--at is before the clip")?,
            )?,
            // The frame with the most sprite pixels.
            None => (0..first.len())
                .max_by_key(|&k| first[k].sprite.iter().filter(|s| **s).count())
                .unwrap_or(0),
        };
        let f = first.get(k).context("--at is past the clip")?;
        let file = std::path::absolute(file)?;
        if let Some(repo) = gp_evidence::paths::enclosing_git_work_tree(&file) {
            bail!(
                "{} is inside the git work tree at {}; pictures of the game stay outside any repo",
                file.display(),
                repo.display()
            );
        }
        write_mask(&file, f)?;
        lines.push(format!(
            "wrote {}: frame {} and its sprite mask in red",
            file.display(),
            start + k
        ));
        j["mask"] = json!({"file": file, "frame": start + k});
    }
    Ok(Outcome {
        json: j,
        text: lines.join("\n"),
        exit: u8::from(!perfect),
    })
}

/// The original at the clip's start, video off. Played from the start
/// cache (`clip::cache`, the snapshot `gp clip score` keeps
/// [`crate::clip::cache::START_LEAD`] frames before a clip's start, made
/// here when it isn't there), then those last frames: the run up to the
/// clip isn't what this checks, the clip is. From power-on with the cache
/// off (`GP_START_CACHE=off`) or for a clip that starts too early.
fn to_start(
    paths: &Paths,
    rom: &[u8],
    log: &gp_evidence::InputLog,
    inputs: &[gp_runtime::InputFrame],
    start: usize,
) -> Result<OriginalRuntime> {
    use crate::clip::cache;
    use gp_evidence::manifest::PowerOn;
    let k = start.saturating_sub(cache::START_LEAD);
    // The original stands after `at` of the clip's inputs.
    let (mut o, at) = if cache::starts_on() && k > 0 && k <= log.frames.len() {
        let key = cache::start_key(rom, &PowerOn::default(), &log.frames, k);
        match cache::read_start(&paths.home, &key, rom, PowerOn::default()) {
            Some(saved) => (OriginalRuntime::from_saved(rom, &saved)?, k),
            None => {
                let mut o = OriginalRuntime::of_rom(rom)?;
                o.step(&inputs[..k]).map_err(anyhow::Error::msg)?;
                cache::write_start_of(&paths.home, &key, &o)?;
                (o, k)
            }
        }
    } else {
        (OriginalRuntime::of_rom(rom)?, 0)
    };
    o.step(&inputs[at..start]).map_err(anyhow::Error::msg)?;
    Ok(o)
}

/// The picture on the left, the same with its sprite pixels in red on the
/// right.
fn write_mask(file: &Path, f: &Frame) -> Result<()> {
    let (w, h) = (f.w as usize, f.h as usize);
    let mut rgb = vec![0u8; w * 2 * h * 3];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let c = f.rgb[i];
            let m = if f.sprite[i] {
                [255, 0, 0]
            } else {
                [c[0] / 3, c[1] / 3, c[2] / 3]
            };
            for (panel, p) in [c, m].into_iter().enumerate() {
                let at = (y * w * 2 + panel * w + x) * 3;
                rgb[at..at + 3].copy_from_slice(&p);
            }
        }
    }
    if let Some(d) = file.parent() {
        std::fs::create_dir_all(d)?;
    }
    let out = std::fs::File::create(file).with_context(|| file.display().to_string())?;
    let mut e = png::Encoder::new(std::io::BufWriter::new(out), (w * 2) as u32, h as u32);
    e.set_color(png::ColorType::Rgb);
    e.set_depth(png::BitDepth::Eight);
    e.write_header()?.write_image_data(&rgb)?;
    Ok(())
}
