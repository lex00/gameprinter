//! `gp theme generate`: a theme's art from Runway's API, start to end
//! (docs/theme-kit.md, "Generating with Runway"). It reads the sheets
//! `gp theme sheet` writes (sheets.json), makes the style board and the
//! turnarounds on `gen4_image`, then every sheet on `gpt_image_2` and every
//! picture on `gen4_image` with those as references, downloads the results
//! into a run folder, fits them (`gp theme fit`), checks them (`gp theme
//! check --original`), makes the cells of a refused slot again on
//! `gen4_image_turbo` up to `--retries` rounds, and writes a contact sheet.
//!
//! Every call is in the run folder's ledger.jsonl before it is made and
//! again when it ends, keyed by its id and its request's hash, so a run
//! that stops (a kill, the budget, an error) continues where it stopped and
//! never pays for the same request twice. `--dry-run` prints every call
//! with its credits and makes none; a paid run asks on the terminal, or
//! needs `--yes`, and stops before it would pass `--budget`.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use gp_evidence::Paths;
use gp_theme::pix::Rgba;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::Outcome;
use crate::theme_runway::{self as rw, Client};

/// The options of `gp theme generate`.
#[derive(clap::Args, Debug, Clone)]
pub struct GenerateArgs {
    /// A template directory, or a template id under ./templates.
    pub game: PathBuf,
    /// The style card (themes/styles/NAME.toml or
    /// $GP_HOME/themes/styles/NAME.toml), passed to `gp theme sheet`.
    #[arg(long)]
    pub style: String,
    /// Only these calls: a reference's id (style, hero), a sheet's file,
    /// a slot id, or a cell (a single-cell call on gen4_image_turbo).
    /// Comma-separated or repeated. References they need and haven't got
    /// are added.
    #[arg(long, value_delimiter = ',')]
    pub only: Vec<String>,
    /// Stop before a call would take the run past this many credits
    /// (default: the estimate and a quarter more, for retries).
    #[arg(long, value_name = "CREDITS")]
    pub budget: Option<u32>,
    /// Print every call with its model and credits, and the total in
    /// dollars; make none.
    #[arg(long)]
    pub dry_run: bool,
    /// Make the paid calls without asking. Without it a paid run asks on
    /// the terminal, and refuses when there is none.
    #[arg(long)]
    pub yes: bool,
    /// The sheets folder `gp theme sheet` wrote (default: RUN/sheets,
    /// written by `gp theme sheet GAME --style NAME` when missing).
    #[arg(long, value_name = "DIR")]
    pub sheets: Option<PathBuf>,
    /// The theme's own title for its title screen, passed to `gp theme
    /// sheet`.
    #[arg(long)]
    pub logo: Option<String>,
    /// The run folder (default:
    /// $GP_HOME/work/theme-runs/GAME/STYLE). Running again with the same
    /// folder continues the run.
    #[arg(long, value_name = "DIR")]
    pub run: Option<PathBuf>,
    /// Tasks in flight at once. Runway's first tier runs 1 or 2 a model
    /// group; more wait as THROTTLED.
    #[arg(long, default_value_t = 2, value_parser = clap::value_parser!(u32).range(1..=8))]
    pub concurrency: u32,
    /// Rounds of single-cell calls for the cells of a slot the checks
    /// refuse, and tries of a call whose task fails for a reason that
    /// can pass.
    #[arg(long, default_value_t = 2)]
    pub retries: u32,
    /// gpt_image_2's quality: low, medium, high (1K or 2K: 1, 5 or 20
    /// credits).
    #[arg(long, default_value = "high", value_parser = ["low", "medium", "high"])]
    pub quality: String,
    /// Make the selected calls again even when the ledger has their
    /// results. They are paid again.
    #[arg(long)]
    pub again: bool,
    /// Generate and download only: no fit, check, retries or contact
    /// sheet.
    #[arg(long)]
    pub no_fit: bool,
    /// The ROM for the originality check's byte runs.
    #[arg(long)]
    pub rom: Option<PathBuf>,
    /// Send stage guides made from your ROM (sheets.json's rom_derived)
    /// with the picture calls, and ask gp theme sheet for them. Off by
    /// default: a guide from a ROM is the original's layout.
    #[arg(long)]
    pub rom_guides: bool,
    /// The API's address.
    #[arg(long, default_value = rw::API)]
    pub api: String,
    /// The first wait between polls of a task, in milliseconds; it grows
    /// by half each poll to 15 seconds.
    #[arg(long, default_value_t = 2000, hide = true)]
    pub poll_ms: u64,
}

/// Where a reference image comes from.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    /// Another call's result.
    Call(String),
    /// A file of the sheets folder (a grid, a guide).
    File(PathBuf),
}

/// One call to make.
#[derive(Debug, Clone, Serialize)]
pub struct Call {
    pub id: String,
    /// style-board, turnaround, sheet, picture or cell.
    pub kind: String,
    pub model: String,
    pub ratio: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    pub prompt: String,
    pub refs: Vec<(String, Source)>,
    /// Where the result goes (the entry's `out` in the sheets folder).
    pub out: PathBuf,
    /// A cell's result is pasted into its sheet's filled grid.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paste: Option<Paste>,
    pub credits: u32,
    pub slots: Vec<String>,
    pub cells: Vec<String>,
}

/// Where a single cell's picture goes: its sheet's `out`, at the cell's
/// `rect` in a grid of `grid` pixels.
#[derive(Debug, Clone, Serialize)]
pub struct Paste {
    pub sheet: PathBuf,
    pub rect: [u32; 4],
    pub grid: [u32; 2],
}

impl Call {
    /// The calls this one needs first.
    fn after(&self) -> Vec<String> {
        self.refs
            .iter()
            .filter_map(|(_, s)| match s {
                Source::Call(c) => Some(c.clone()),
                Source::File(_) => None,
            })
            .collect()
    }

    /// The request's identity: what is sent, with a call's reference as
    /// that call's key.
    fn hash(&self, keys: &BTreeMap<String, String>) -> String {
        let mut h = blake3::Hasher::new();
        for p in [&self.model, &self.ratio, &self.prompt] {
            h.update(p.as_bytes());
            h.update(&[0]);
        }
        h.update(self.quality.as_deref().unwrap_or("").as_bytes());
        h.update(self.background.as_deref().unwrap_or("").as_bytes());
        for (tag, s) in &self.refs {
            h.update(tag.as_bytes());
            match s {
                Source::Call(c) => h.update(keys.get(c).map_or("", String::as_str).as_bytes()),
                Source::File(f) => h.update(fs::read(f).unwrap_or_default().as_slice()),
            };
        }
        h.finalize().to_hex()[..16].to_owned()
    }
}

/// A cell of a sheet, as sheets.json (`gp-sheets 1`) gives it.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct Cell {
    pub cell: String,
    pub slot: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub frame_names: Vec<String>,
    /// The cell's inside in the grid, `[x, y, w, h]`.
    #[serde(default)]
    pub rect: [u32; 4],
    #[serde(default)]
    pub pose: String,
}

/// The manifest format this reads (docs/theme-kit.md, "sheets.json").
pub const SHEETS_FORMAT: &str = "gp-sheets 1";

/// What `gp theme sheet` wrote, read as calls: the refs in order (a ref
/// with `image` is the card's own picture, made by no call), the sheets
/// (each with its grid as `@grid` and its refs), then the pictures (with
/// `@guide`). A ref is tagged by its id with `-` as `_`.
#[derive(Debug, Clone, Default)]
pub struct Manifest {
    pub calls: Vec<Call>,
    /// Each cell, its sheet's call id and its grid's size.
    pub cells: BTreeMap<String, (Cell, String, [u32; 2])>,
    /// Slots drawn in code, with why.
    pub code: Vec<(String, String)>,
    /// Refs the style card already has: id -> picture.
    pub given: BTreeMap<String, PathBuf>,
}

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_owned)
}

fn list(v: &Value, k: &str) -> Vec<String> {
    v.get(k)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn safe(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// A ref's tag in prompts: its id with `-` as `_`.
pub fn tag(id: &str) -> String {
    id.replace('-', "_")
}

fn read_prompt(dir: &Path, file: &str) -> Result<String> {
    let p = dir.join(file);
    Ok(fs::read_to_string(&p)
        .with_context(|| format!("reading {}", p.display()))?
        .trim()
        .to_owned())
}

/// PNG width and height from its header.
fn png_size(f: &Path) -> Option<(u32, u32)> {
    let b = fs::read(f).ok()?;
    if b.len() < 24 || &b[..8] != b"\x89PNG\r\n\x1a\n" {
        return None;
    }
    let w = u32::from_be_bytes(b[16..20].try_into().ok()?);
    let h = u32::from_be_bytes(b[20..24].try_into().ok()?);
    Some((w, h))
}

impl Manifest {
    pub fn read(dir: &Path, quality: &str) -> Result<Self> {
        let file = dir.join("sheets.json");
        let v: Value = serde_json::from_str(
            &fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?,
        )
        .with_context(|| format!("{}", file.display()))?;
        let format = v["format"].as_str().unwrap_or("");
        if format != SHEETS_FORMAT {
            bail!(
                "{}: format `{format}`, and this gp reads `{SHEETS_FORMAT}`",
                file.display()
            );
        }
        let arr = |k: &str| {
            v.get(k)
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        };
        let mut m = Manifest::default();
        let q = |model: &str| (model == "gpt_image_2").then(|| quality.to_owned());
        // A ref named in `refs` -> where its picture comes from.
        let source = |m: &Manifest, id: &str| match m.given.get(id) {
            Some(p) => Source::File(p.clone()),
            None => Source::Call(format!("ref:{id}")),
        };
        for r in arr("refs") {
            let id = s(&r, "id").ok_or_else(|| anyhow!("a ref in sheets.json has no id"))?;
            if let Some(img) = s(&r, "image") {
                m.given.insert(id, PathBuf::from(img));
                continue;
            }
            let model = s(&r, "model").unwrap_or_else(|| "gen4_image".into());
            let refs = list(&r, "refs")
                .iter()
                .map(|t| (tag(t), source(&m, t)))
                .collect();
            m.calls.push(Call {
                id: format!("ref:{id}"),
                kind: s(&r, "kind").unwrap_or_else(|| "turnaround".into()),
                quality: q(&model),
                background: None,
                prompt: read_prompt(dir, &s(&r, "prompt_file").unwrap_or_default())?,
                refs,
                out: dir.join(s(&r, "out").unwrap_or_else(|| format!("refs/{id}.png"))),
                paste: None,
                credits: 0,
                slots: list(&r, "slots"),
                cells: Vec::new(),
                ratio: s(&r, "ratio").unwrap_or_else(|| "1920:1080".into()),
                model,
            });
        }
        for sh in arr("sheets") {
            let id = s(&sh, "id").ok_or_else(|| anyhow!("a sheet in sheets.json has no id"))?;
            let image = dir.join(s(&sh, "image").unwrap_or_else(|| format!("{id}.png")));
            let model = s(&sh, "model").unwrap_or_else(|| "gpt_image_2".into());
            let size: [u32; 2] = serde_json::from_value(sh["size"].clone())
                .ok()
                .or_else(|| png_size(&image).map(|(w, h)| [w, h]))
                .unwrap_or([1024, 1024]);
            let ratio =
                s(&sh, "ratio").unwrap_or_else(|| rw::nearest_ratio(&model, size[0], size[1]));
            let pfile = sh["prompts"][model.as_str()]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| anyhow!("{id}: no prompt for {model} in sheets.json"))?;
            let cells: Vec<Cell> = serde_json::from_value(sh["cells"].clone())
                .with_context(|| format!("the cells of {id} in sheets.json"))?;
            let call = format!("sheet:{id}");
            let mut slots: Vec<String> = cells.iter().map(|c| c.slot.clone()).collect();
            slots.dedup();
            let mut refs: Vec<(String, Source)> = vec![("grid".into(), Source::File(image))];
            refs.extend(list(&sh, "refs").iter().map(|t| (tag(t), source(&m, t))));
            for c in &cells {
                m.cells
                    .insert(c.cell.clone(), (c.clone(), call.clone(), size));
            }
            m.calls.push(Call {
                id: call,
                kind: "sheet".into(),
                quality: q(&model),
                background: (model == "gpt_image_2").then(|| "opaque".into()),
                prompt: read_prompt(dir, &pfile)?,
                refs,
                out: dir.join(s(&sh, "out").unwrap_or_else(|| format!("{id}.out.png"))),
                paste: None,
                credits: 0,
                cells: cells.iter().map(|c| c.cell.clone()).collect(),
                slots,
                model,
                ratio,
            });
        }
        for p in arr("pictures") {
            let slot =
                s(&p, "slot").ok_or_else(|| anyhow!("a picture in sheets.json names no slot"))?;
            let id = s(&p, "id").unwrap_or_else(|| slot.clone());
            let model = s(&p, "model").unwrap_or_else(|| "gen4_image".into());
            let guide = s(&p, "guide").map(|g| dir.join(g));
            let mut refs = Vec::new();
            for t in list(&p, "refs") {
                if t == "guide" {
                    match &guide {
                        Some(g) => refs.push(("guide".to_owned(), Source::File(g.clone()))),
                        None => bail!("{id}: refers to a guide sheets.json doesn't give"),
                    }
                } else {
                    refs.push((tag(&t), source(&m, &t)));
                }
            }
            m.calls.push(Call {
                id: format!("picture:{id}"),
                kind: "picture".into(),
                quality: q(&model),
                background: None,
                prompt: read_prompt(dir, &s(&p, "prompt_file").unwrap_or_default())?,
                refs,
                out: dir.join(s(&p, "out").unwrap_or_else(|| format!("pictures/{id}.out.png"))),
                paste: None,
                credits: 0,
                slots: vec![slot],
                cells: Vec::new(),
                ratio: s(&p, "ratio").unwrap_or_else(|| "1440:1080".into()),
                model,
            });
        }
        for c in arr("code") {
            m.code.push((
                s(&c, "slot").unwrap_or_default(),
                s(&c, "why").unwrap_or_default(),
            ));
        }
        for c in &mut m.calls {
            c.credits = rw::credits(&c.model, &c.ratio, c.quality.as_deref(), 1)?;
        }
        Ok(m)
    }

    /// A single-cell call for a cell, in round `round`, on
    /// gen4_image_turbo with its sheet's refs (at most 3, the grid left
    /// out), its result pasted into the sheet's filled grid.
    pub fn cell_call(&self, cell: &str, round: u32) -> Option<Call> {
        let (c, sheet, grid) = self.cells.get(cell)?;
        let sheet = self.calls.iter().find(|x| &x.id == sheet)?;
        let refs: Vec<(String, Source)> = sheet
            .refs
            .iter()
            .filter(|(t, _)| t != "grid")
            .take(3)
            .cloned()
            .collect();
        let tags: Vec<String> = refs.iter().map(|(t, _)| format!("@{t}")).collect();
        let prompt = format!(
            "One original game sprite: {}, {}, drawn like {}. The whole figure, centered, standing on the bottom edge, flat colors, a dark outline and at most three tones, on a flat magenta (#FF00FF) background. No text, no border.",
            if c.label.is_empty() {
                "a figure"
            } else {
                &c.label
            },
            if c.pose.is_empty() {
                "standing"
            } else {
                &c.pose
            },
            if tags.is_empty() {
                "the references".to_owned()
            } else {
                tags.join(" and ")
            }
        );
        let dir = sheet
            .out
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        Some(Call {
            id: if round == 0 {
                format!("cell:{cell}")
            } else {
                format!("cell:{cell}#r{round}")
            },
            kind: "cell".into(),
            model: "gen4_image_turbo".into(),
            ratio: "1024:1024".into(),
            quality: None,
            background: None,
            prompt,
            refs,
            out: dir
                .join("cells")
                .join(format!("{}.r{round}.png", safe(cell))),
            paste: Some(Paste {
                sheet: sheet.out.clone(),
                rect: c.rect,
                grid: *grid,
            }),
            credits: 2,
            slots: vec![c.slot.clone()],
            cells: vec![c.cell.clone()],
        })
    }
}

/// Pastes a cell's picture into its sheet's filled grid: scaled to fit
/// the cell's rect (the grid scaled to the filled grid's size), standing
/// on the rect's bottom, centered, on the key color.
pub fn paste(cell: &Path, p: &Paste) -> Result<()> {
    let mut sheet = Rgba::load(&p.sheet).map_err(|e| anyhow!("{}: {e}", p.sheet.display()))?;
    let img = Rgba::load(cell).map_err(|e| anyhow!("{}: {e}", cell.display()))?;
    let sx = f64::from(sheet.width) / f64::from(p.grid[0].max(1));
    let sy = f64::from(sheet.height) / f64::from(p.grid[1].max(1));
    let [x, y, w, h] = p.rect;
    let (rx, ry) = ((f64::from(x) * sx) as u32, (f64::from(y) * sy) as u32);
    let (rw_, rh) = (
        ((f64::from(w) * sx) as u32).max(1),
        ((f64::from(h) * sy) as u32).max(1),
    );
    let k = (f64::from(rw_) / f64::from(img.width.max(1)))
        .min(f64::from(rh) / f64::from(img.height.max(1)));
    let (iw, ih) = (
        ((f64::from(img.width) * k) as u32).max(1),
        ((f64::from(img.height) * k) as u32).max(1),
    );
    let small = crate::theme_paper::scale_to(&img, iw, ih);
    for yy in 0..rh {
        for xx in 0..rw_ {
            sheet.set(rx + xx, ry + yy, [255, 0, 255, 255]);
        }
    }
    let (ox, oy) = (rx + (rw_ - iw) / 2, ry + (rh - ih));
    for yy in 0..ih {
        for xx in 0..iw {
            let c = small.get(xx, yy);
            if c[3] >= 128 {
                sheet.set(ox + xx, oy + yy, [c[0], c[1], c[2], 255]);
            }
        }
    }
    fs::write(&p.sheet, sheet.png())?;
    Ok(())
}

/// A file that may never go to a service: anything under the lifted
/// assets, the ROMs, a theme kit's local part, or named like the
/// reference sheet. Only files of the sheets folder and the run's own
/// results go.
fn forbidden(f: &Path, paths: &Paths, allowed: &[&Path]) -> Option<String> {
    let real = fs::canonicalize(f).unwrap_or_else(|_| f.to_owned());
    let name = real
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let under = |d: &Path| {
        let d = fs::canonicalize(d).unwrap_or_else(|_| d.to_owned());
        real.starts_with(d)
    };
    if name.starts_with("reference") {
        return Some("it is named like the original's reference sheet".into());
    }
    for (d, what) in [
        (paths.home.join("work").join("assets"), "the lifted assets"),
        (paths.roms.clone(), "the ROMs"),
        (
            paths.home.join("work").join("theme-kits"),
            "a theme kit's local part",
        ),
    ] {
        if under(&d) {
            return Some(format!("it is under {what} ({})", d.display()));
        }
    }
    if !allowed.iter().any(|a| under(a)) {
        return Some("it is outside the sheets folder and the run folder".into());
    }
    None
}

/// Every problem the plan has that the API would refuse, or that the
/// rules refuse, before anything is paid.
fn validate(calls: &[Call], paths: &Paths, allowed: &[&Path]) -> Vec<String> {
    let mut errs = Vec::new();
    let ids: BTreeSet<&str> = calls.iter().map(|c| c.id.as_str()).collect();
    for c in calls {
        if !rw::ratios(&c.model).contains(&c.ratio.as_str()) {
            errs.push(format!("{}: {} has no ratio {}", c.id, c.model, c.ratio));
        }
        let len = if c.model == "gpt_image_2" {
            c.prompt.chars().count()
        } else {
            c.prompt.encode_utf16().count()
        };
        if c.prompt.trim().is_empty() {
            errs.push(format!("{}: an empty prompt", c.id));
        }
        if len > rw::prompt_limit(&c.model) {
            errs.push(format!(
                "{}: a prompt of {len} characters, {} takes {}",
                c.id,
                c.model,
                rw::prompt_limit(&c.model)
            ));
        }
        let (lo, hi) = rw::reference_limits(&c.model);
        if c.refs.len() < lo || c.refs.len() > hi {
            errs.push(format!(
                "{}: {} references, {} takes {lo} to {hi}",
                c.id,
                c.refs.len(),
                c.model
            ));
        }
        let mut tags = BTreeSet::new();
        for (t, s) in &c.refs {
            if !rw::valid_tag(t) {
                errs.push(format!("{}: the tag `{t}` isn't one the API takes (a letter, then letters, digits or _, 3 to 16)", c.id));
            }
            if !tags.insert(t) {
                errs.push(format!("{}: the tag `{t}` twice", c.id));
            }
            match s {
                Source::Call(r) if !ids.contains(r.as_str()) => {
                    errs.push(format!("{}: refers to {r}, which sheets.json hasn't", c.id));
                }
                Source::File(f) => {
                    if !f.is_file() {
                        errs.push(format!("{}: no file {}", c.id, f.display()));
                    } else if let Some(why) = forbidden(f, paths, allowed) {
                        errs.push(format!("{}: won't send {}: {why}", c.id, f.display()));
                    } else if png_size(f).is_none() {
                        errs.push(format!("{}: {} isn't a PNG", c.id, f.display()));
                    } else if data_uri_len(f) > DATA_URI_MAX {
                        errs.push(format!(
                            "{}: {} is over the API's 5 MB data URI",
                            c.id,
                            f.display()
                        ));
                    }
                }
                _ => {}
            }
        }
    }
    errs
}

const DATA_URI_MAX: usize = 5_000_000;

fn data_uri_len(f: &Path) -> usize {
    let n = fs::metadata(f).map(|m| m.len() as usize).unwrap_or(0);
    22 + n.div_ceil(3) * 4
}

fn data_uri(f: &Path) -> Result<String> {
    let b = fs::read(f)?;
    let kind = match b.get(..4) {
        Some([0x89, b'P', b'N', b'G']) => "image/png",
        Some([0xFF, 0xD8, 0xFF, _]) => "image/jpeg",
        Some(b"RIFF") => "image/webp",
        _ => bail!("{} isn't a PNG, JPEG or WebP", f.display()),
    };
    Ok(format!(
        "data:{kind};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(b)
    ))
}

/// One line of the ledger.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Entry {
    /// submitting, submitted, rejected, ended, downloaded, stopped.
    pub event: String,
    #[serde(default)]
    pub call: String,
    /// The call's id and its request's hash.
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub attempt: u32,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub prompt_sha256: String,
    #[serde(default)]
    pub credits_est: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// What the task cost (0 for a refund).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credits: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    /// The run's credits spent or held so far, with this line.
    #[serde(default)]
    pub spent: u32,
}

/// An attempt as the ledger leaves it.
#[derive(Debug, Clone, Default)]
struct Attempt {
    task: Option<String>,
    est: u32,
    status: Option<String>,
    credits: Option<u32>,
    failure_code: Option<String>,
    result: Option<String>,
    rejected: bool,
}

impl Attempt {
    /// Credits this attempt took or may have taken: its cost once it
    /// ended, else its estimate (in flight, or lost between the
    /// submitting line and the reply). A refused request took none.
    fn held(&self) -> u32 {
        if self.rejected {
            0
        } else if let Some(c) = self.credits {
            c
        } else {
            self.est
        }
    }
}

/// The ledger: every line, and the attempts by key.
pub struct Ledger {
    file: PathBuf,
    attempts: BTreeMap<String, BTreeMap<u32, Attempt>>,
    spent: u32,
}

impl Ledger {
    pub fn open(file: &Path) -> Result<Self> {
        let mut l = Ledger {
            file: file.to_owned(),
            attempts: BTreeMap::new(),
            spent: 0,
        };
        if let Ok(text) = fs::read_to_string(file) {
            for (n, line) in text.lines().enumerate() {
                if line.trim().is_empty() {
                    continue;
                }
                // A line cut short by a kill is the last; it is left out.
                let Ok(e) = serde_json::from_str::<Entry>(line) else {
                    eprintln!("ledger line {} unreadable, left out", n + 1);
                    continue;
                };
                l.apply(&e);
            }
        }
        Ok(l)
    }

    fn apply(&mut self, e: &Entry) {
        if e.key.is_empty() {
            return;
        }
        let a = self
            .attempts
            .entry(e.key.clone())
            .or_default()
            .entry(e.attempt)
            .or_default();
        let before = a.held();
        match e.event.as_str() {
            "submitting" => a.est = e.credits_est,
            "submitted" => a.task.clone_from(&e.task),
            "rejected" => a.rejected = true,
            "ended" => {
                a.status.clone_from(&e.status);
                a.credits = e.credits;
                a.failure_code.clone_from(&e.failure_code);
            }
            "downloaded" => a.result.clone_from(&e.result),
            _ => {}
        }
        let after = a.held();
        self.spent = self.spent.saturating_sub(before) + after;
    }

    /// Appends a line, synced to disk before the call it records is made.
    fn write(&mut self, mut e: Entry) -> Result<()> {
        self.apply(&e);
        e.spent = self.spent;
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.file)?;
        writeln!(f, "{}", serde_json::to_string(&e)?)?;
        f.sync_data()?;
        Ok(())
    }

    pub fn spent(&self) -> u32 {
        self.spent
    }

    fn attempts(&self, key: &str) -> Option<&BTreeMap<u32, Attempt>> {
        self.attempts.get(key)
    }
}

/// A call's state from the ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
enum State {
    /// Never made, or only refused.
    New,
    /// Its result is downloaded.
    Done,
    /// A task to poll (in flight, or succeeded and not downloaded).
    Live(String, u32),
    /// Failed this many times in a way that may pass.
    Failed(u32),
    /// Failed in a way that won't pass (a safety refusal, a bad input).
    Final(String),
}

fn state_of(l: &Ledger, key: &str, out: &Path) -> (State, u32) {
    let Some(at) = l.attempts(key) else {
        return (State::New, 0);
    };
    let next = at.keys().max().map_or(0, |m| m + 1);
    let mut fails = 0;
    let mut last = State::New;
    for (n, a) in at {
        if a.rejected {
            continue;
        }
        if a.result.is_some() && out.is_file() {
            return (State::Done, next);
        }
        match (a.status.as_deref(), &a.task) {
            (Some("SUCCEEDED"), Some(t)) | (None, Some(t)) => last = State::Live(t.clone(), *n),
            (Some(_), _) => {
                if rw::failure_retryable(a.failure_code.as_deref()) {
                    fails += 1;
                    last = State::Failed(fails);
                } else {
                    last = State::Final(a.failure_code.clone().unwrap_or_default());
                }
            }
            // Lost between the submitting line and the reply: maybe paid,
            // never known. Counted as held; it may be made again.
            (None, None) => {
                fails += 1;
                last = State::Failed(fails);
            }
        }
    }
    (last, next)
}

/// How a scheduling pass ended.
#[derive(Debug, Default, Serialize)]
pub struct Pass {
    pub done: Vec<String>,
    pub failed: Vec<(String, String)>,
    pub blocked: Vec<String>,
    pub stopped: Option<String>,
}

struct Live {
    call: usize,
    attempt: u32,
    task: String,
    next: Instant,
    delay: Duration,
    polls: u32,
}

/// Up to 50% more, from a hash: jitter with no clock or RNG.
fn jitter(d: Duration, seed: &str, n: u32) -> Duration {
    let h = blake3::hash(format!("{seed}/{n}").as_bytes());
    let f = f64::from(h.as_bytes()[0]) / 255.0 * 0.5;
    d.mul_f64(1.0 + f)
}

struct Runner<'a> {
    client: &'a Client,
    run: PathBuf,
    ledger: Ledger,
    cap: u32,
    concurrency: usize,
    retries: u32,
    poll: Duration,
    /// Result files of calls done, for references.
    results: BTreeMap<String, PathBuf>,
    /// Tasks of calls done, for a fresh URL when a file is too big.
    tasks: BTreeMap<String, String>,
    log: Vec<String>,
}

impl Runner<'_> {
    fn say(&mut self, line: String) {
        eprintln!("{line}");
        self.log.push(line);
    }

    fn entry(&self, c: &Call, key: &str, attempt: u32, event: &str) -> Entry {
        Entry {
            event: event.into(),
            call: c.id.clone(),
            key: key.into(),
            attempt,
            model: c.model.clone(),
            prompt_sha256: gp_theme::format::sha256_hex(c.prompt.as_bytes()),
            credits_est: c.credits,
            ..Default::default()
        }
    }

    /// A reference's URI: the result as a data URI when it fits, else a
    /// fresh URL of its task's output.
    fn reference(&self, s: &Source) -> Result<String> {
        match s {
            Source::File(f) => data_uri(f),
            Source::Call(id) => {
                let f = self
                    .results
                    .get(id)
                    .ok_or_else(|| anyhow!("{id} has no result yet"))?;
                if data_uri_len(f) <= DATA_URI_MAX {
                    return data_uri(f);
                }
                let task = self
                    .tasks
                    .get(id)
                    .ok_or_else(|| anyhow!("{id} has no task"))?;
                let r = self
                    .client
                    .request("GET", &format!("/v1/tasks/{task}"), None)?;
                r.json()["output"][0]
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| anyhow!("{id}'s task has no output to refer to"))
            }
        }
    }

    fn body(&self, c: &Call) -> Result<Value> {
        let mut refs = Vec::new();
        for (tag, s) in &c.refs {
            refs.push(json!({"uri": self.reference(s)?, "tag": tag}));
        }
        let mut b = json!({
            "model": c.model,
            "promptText": c.prompt,
            "ratio": c.ratio,
        });
        if !refs.is_empty() {
            b["referenceImages"] = Value::Array(refs);
        }
        if let Some(q) = &c.quality {
            b["quality"] = json!(q);
        }
        if let Some(bg) = &c.background {
            b["background"] = json!(bg);
        }
        Ok(b)
    }

    /// Downloads a succeeded task's output to the call's file, and pastes
    /// a cell's into its sheet.
    fn fetch(&mut self, c: &Call, key: &str, attempt: u32, task: &Value) -> Result<()> {
        let url = task["output"][0]
            .as_str()
            .ok_or_else(|| anyhow!("{}: the task succeeded with no output", c.id))?;
        let to = c.out.clone();
        if let Some(p) = to.parent() {
            fs::create_dir_all(p)?;
        }
        rw::download(url, &to)?;
        let bytes = fs::read(&to)?;
        if bytes.get(..4) != Some(&[0x89, b'P', b'N', b'G']) {
            self.say(format!(
                "{}: the result isn't a PNG; gp theme fit reads PNG only",
                c.id
            ));
        }
        if let Some(p) = &c.paste {
            paste(&to, p)?;
        }
        let mut e = self.entry(c, key, attempt, "downloaded");
        e.result = Some(c.out.display().to_string());
        e.note = Some(gp_theme::format::sha256_hex(&bytes));
        self.ledger.write(e)?;
        self.results.insert(c.id.clone(), to);
        Ok(())
    }

    /// Makes the calls, `concurrency` at a time, in order as their
    /// references are ready.
    fn pass(&mut self, calls: &[Call], again: &BTreeSet<String>) -> Result<Pass> {
        let mut out = Pass::default();
        let mut keys: BTreeMap<String, String> = BTreeMap::new();
        for c in calls {
            let k = format!("{}@{}", c.id, c.hash(&keys));
            keys.insert(c.id.clone(), k);
        }
        let mut queue: VecDeque<usize> = VecDeque::new();
        let mut live: Vec<Live> = Vec::new();
        let mut next_attempt: BTreeMap<usize, u32> = BTreeMap::new();
        let mut fails: BTreeMap<usize, u32> = BTreeMap::new();
        let mut finished: BTreeSet<String> = BTreeSet::new();
        let mut lost: BTreeSet<String> = BTreeSet::new();
        for (i, c) in calls.iter().enumerate() {
            let key = &keys[&c.id];
            let (st, next) = state_of(&self.ledger, key, &c.out);
            next_attempt.insert(i, next);
            match st {
                State::Done if !again.contains(&c.id) => {
                    self.results.insert(c.id.clone(), c.out.clone());
                    if let Some(t) = self
                        .ledger
                        .attempts(key)
                        .and_then(|a| a.values().rev().find_map(|x| x.task.clone()))
                    {
                        self.tasks.insert(c.id.clone(), t);
                    }
                    finished.insert(c.id.clone());
                }
                State::Live(t, attempt) if !again.contains(&c.id) => {
                    self.say(format!("{}: continuing task {t}", c.id));
                    live.push(Live {
                        call: i,
                        attempt,
                        task: t,
                        next: Instant::now(),
                        delay: self.poll,
                        polls: 0,
                    });
                }
                State::Final(code) if !again.contains(&c.id) => {
                    out.failed.push((c.id.clone(), code));
                    lost.insert(c.id.clone());
                }
                State::Failed(n) => {
                    fails.insert(i, n);
                    if n > self.retries && !again.contains(&c.id) {
                        out.failed.push((c.id.clone(), format!("failed {n} times")));
                        lost.insert(c.id.clone());
                    } else {
                        queue.push_back(i);
                    }
                }
                _ => queue.push_back(i),
            }
        }
        let mut hold = Instant::now();
        let mut refusals = 0u32;
        loop {
            // Start what can start.
            while out.stopped.is_none() && live.len() < self.concurrency && Instant::now() >= hold {
                let Some(pos) = queue.iter().position(|&i| {
                    calls[i]
                        .after()
                        .iter()
                        .all(|d| finished.contains(d) || lost.contains(d))
                }) else {
                    break;
                };
                let i = queue.remove(pos).expect("in the queue");
                let c = &calls[i];
                if let Some(d) = c.after().iter().find(|d| lost.contains(*d)) {
                    out.blocked.push(format!("{} (needs {d})", c.id));
                    lost.insert(c.id.clone());
                    continue;
                }
                if self.ledger.spent() + c.credits > self.cap {
                    let msg = format!(
                        "budget: {} would take the run to {} credits, past the cap of {}",
                        c.id,
                        self.ledger.spent() + c.credits,
                        self.cap
                    );
                    let mut e = self.entry(c, "", 0, "stopped");
                    e.note = Some(msg.clone());
                    self.ledger.write(e)?;
                    self.say(msg.clone());
                    out.stopped = Some(msg);
                    queue.push_front(i);
                    break;
                }
                let key = keys[&c.id].clone();
                let attempt = next_attempt[&i];
                next_attempt.insert(i, attempt + 1);
                let body = self.body(c)?;
                let req = self
                    .run
                    .join("requests")
                    .join(format!("{}-a{attempt}.json", safe(&c.id)));
                fs::create_dir_all(req.parent().expect("a folder"))?;
                fs::write(&req, serde_json::to_vec(&body)?)?;
                self.ledger
                    .write(self.entry(c, &key, attempt, "submitting"))?;
                let r = self
                    .client
                    .request("POST", "/v1/text_to_image", Some(&req))?;
                let id = r.json()["id"].as_str().map(str::to_owned);
                match (r.status, id) {
                    (200..=299, Some(task)) => {
                        refusals = 0;
                        let mut e = self.entry(c, &key, attempt, "submitted");
                        e.task = Some(task.clone());
                        self.ledger.write(e)?;
                        self.say(format!(
                            "{}: task {task} on {} ({} credits; {} of {} spent or held)",
                            c.id,
                            c.model,
                            c.credits,
                            self.ledger.spent(),
                            self.cap
                        ));
                        live.push(Live {
                            call: i,
                            attempt,
                            task,
                            next: Instant::now() + self.poll,
                            delay: self.poll,
                            polls: 0,
                        });
                    }
                    (st, _) => {
                        let mut e = self.entry(c, &key, attempt, "rejected");
                        e.status = Some(format!("HTTP {st}"));
                        e.note = Some(r.body.chars().take(300).collect());
                        self.ledger.write(e)?;
                        if st == 401 {
                            bail!(
                                "Runway refused the key (HTTP 401): {}",
                                r.body.chars().take(300).collect::<String>()
                            );
                        }
                        if r.retryable() {
                            refusals += 1;
                            if refusals > 8 {
                                let msg = format!(
                                    "the API refused {refusals} requests in a row (last HTTP {st}); run again later to continue"
                                );
                                self.say(msg.clone());
                                out.stopped = Some(msg);
                                queue.push_front(i);
                                break;
                            }
                            let wait =
                                jitter(self.poll * 2u32.pow(refusals.min(6)), &c.id, refusals);
                            self.say(format!(
                                "{}: HTTP {st}, trying again in {:.1} s",
                                c.id,
                                wait.as_secs_f64()
                            ));
                            hold = Instant::now() + wait;
                            queue.push_front(i);
                        } else {
                            out.failed
                                .push((c.id.clone(), format!("HTTP {st}: {}", r.body)));
                            lost.insert(c.id.clone());
                        }
                    }
                }
            }
            if live.is_empty() && (out.stopped.is_some() || queue.is_empty()) {
                break;
            }
            if live.is_empty()
                && !queue.iter().any(|&i| {
                    calls[i]
                        .after()
                        .iter()
                        .all(|d| finished.contains(d) || lost.contains(d))
                })
            {
                for i in queue.drain(..) {
                    out.blocked.push(calls[i].id.clone());
                }
                break;
            }
            // Poll what is due.
            let now = Instant::now();
            let mut k = 0;
            while k < live.len() {
                if live[k].next > now {
                    k += 1;
                    continue;
                }
                let (i, attempt, task) = (live[k].call, live[k].attempt, live[k].task.clone());
                let c = &calls[i];
                let key = keys[&c.id].clone();
                let r = self
                    .client
                    .request("GET", &format!("/v1/tasks/{task}"), None)?;
                let v = r.json();
                let status = v["status"].as_str().unwrap_or("").to_owned();
                let l = &mut live[k];
                l.polls += 1;
                if r.status == 401 {
                    bail!("Runway refused the key (HTTP 401): {}", r.body);
                }
                if !(200..=299).contains(&r.status)
                    || matches!(status.as_str(), "PENDING" | "THROTTLED" | "RUNNING" | "")
                {
                    if r.status == 404 {
                        // The task is gone: its cost is taken as its
                        // estimate, and the call may be made again.
                        let mut e = self.entry(c, &key, attempt, "ended");
                        e.task = Some(task.clone());
                        e.status = Some("LOST".into());
                        e.credits = Some(c.credits);
                        e.failure_code = Some("INTERNAL".into());
                        self.ledger.write(e)?;
                        live.remove(k);
                        let n = fails.entry(i).or_default();
                        *n += 1;
                        if *n <= self.retries {
                            queue.push_front(i);
                        } else {
                            out.failed.push((c.id.clone(), "its task was lost".into()));
                            lost.insert(c.id.clone());
                        }
                        continue;
                    }
                    let l = &mut live[k];
                    l.delay = (l.delay.mul_f64(1.5)).min(Duration::from_secs(15));
                    l.next = Instant::now() + jitter(l.delay, &l.task, l.polls);
                    k += 1;
                    continue;
                }
                live.remove(k);
                let cost = v["cost"]["credits"].as_u64().map(|x| x as u32);
                let code = v["failureCode"].as_str().map(str::to_owned);
                let mut e = self.entry(c, &key, attempt, "ended");
                e.task = Some(task.clone());
                e.status = Some(status.clone());
                e.failure_code.clone_from(&code);
                e.credits = Some(match (status.as_str(), cost) {
                    (_, Some(x)) => x,
                    ("SUCCEEDED", None) => c.credits,
                    (_, None) if rw::failure_charged(code.as_deref()) => c.credits,
                    _ => 0,
                });
                if status != "SUCCEEDED" {
                    e.note = v["failure"].as_str().map(|f| self.client.redact(f));
                }
                self.ledger.write(e)?;
                if status == "SUCCEEDED" {
                    match self.fetch(c, &key, attempt, &v) {
                        Ok(()) => {
                            self.tasks.insert(c.id.clone(), task.clone());
                            self.say(format!("{}: done, {}", c.id, c.out.display()));
                            out.done.push(c.id.clone());
                            finished.insert(c.id.clone());
                        }
                        Err(err) => {
                            out.failed
                                .push((c.id.clone(), self.client.redact(&format!("{err:#}"))));
                            lost.insert(c.id.clone());
                        }
                    }
                    continue;
                }
                let why = format!(
                    "{} {}: {}",
                    status,
                    code.clone().unwrap_or_default(),
                    v["failure"].as_str().unwrap_or("")
                );
                let n = fails.entry(i).or_default();
                *n += 1;
                if rw::failure_retryable(code.as_deref()) && *n <= self.retries {
                    self.say(format!(
                        "{}: {why}; trying again ({} of {})",
                        c.id, n, self.retries
                    ));
                    queue.push_front(i);
                } else {
                    self.say(format!("{}: {why}", c.id));
                    out.failed.push((c.id.clone(), why));
                    lost.insert(c.id.clone());
                }
            }
            // Wait for the next poll or the end of a hold.
            let mut until = live.iter().map(|l| l.next).min();
            if !queue.is_empty() && live.len() < self.concurrency && out.stopped.is_none() {
                until = Some(until.map_or(hold, |u| u.min(hold)));
            }
            if let Some(u) = until {
                let now = Instant::now();
                if u > now {
                    std::thread::sleep(u - now);
                }
            }
        }
        Ok(out)
    }
}

/// The calls `--only` selects, with the references they need and
/// haven't got.
fn select(m: &Manifest, only: &[String]) -> Result<Vec<Call>> {
    if only.is_empty() {
        return Ok(m.calls.clone());
    }
    let mut picked: BTreeSet<String> = BTreeSet::new();
    let mut cells = Vec::new();
    for o in only {
        let mut hit = false;
        for c in &m.calls {
            let bare = c.id.split_once(':').map_or(c.id.as_str(), |x| x.1);
            if c.id == *o || bare == o || c.slots.contains(o) {
                picked.insert(c.id.clone());
                hit = true;
            }
        }
        if !hit && m.cells.contains_key(o) {
            cells.push(o.clone());
            hit = true;
        }
        if !hit {
            bail!("--only {o}: no reference, sheet, slot or cell of that name in sheets.json");
        }
    }
    // The references the picked calls and cells need.
    let mut need: Vec<String> = picked.iter().cloned().collect();
    let mut extra: Vec<Call> = cells.iter().filter_map(|c| m.cell_call(c, 0)).collect();
    need.extend(extra.iter().flat_map(Call::after));
    let mut seen = BTreeSet::new();
    while let Some(id) = need.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        if let Some(c) = m.calls.iter().find(|c| c.id == id) {
            picked.insert(id);
            need.extend(c.after());
        }
    }
    let mut out: Vec<Call> = m
        .calls
        .iter()
        .filter(|c| picked.contains(&c.id))
        .cloned()
        .collect();
    out.append(&mut extra);
    Ok(out)
}

fn dollars(credits: u32) -> String {
    format!("${:.2}", f64::from(credits) * rw::DOLLARS_PER_CREDIT)
}

/// The estimate as text: a line a call, then the totals.
fn estimate(calls: &[(Call, bool)], paid: u32, cap: u32, retries: u32) -> (String, u32) {
    let mut t = String::new();
    t.push_str(&format!(
        "{:<44} {:<17} {:<10} {:<7} {:<40} {:>7}\n",
        "call", "model", "ratio", "quality", "references", "credits"
    ));
    let mut total = 0;
    let mut n = 0;
    for (c, done) in calls {
        let refs: Vec<&str> = c.refs.iter().map(|(t, _)| t.as_str()).collect();
        t.push_str(&format!(
            "{:<44} {:<17} {:<10} {:<7} {:<40} {:>7}{}\n",
            c.id,
            c.model,
            c.ratio,
            c.quality.as_deref().unwrap_or("-"),
            if refs.is_empty() {
                "-".to_owned()
            } else {
                refs.join(",")
            },
            c.credits,
            if *done {
                "  (in the ledger: not paid again)"
            } else {
                ""
            }
        ));
        if !done {
            total += c.credits;
            n += 1;
        }
    }
    t.push_str(&format!(
        "\n{n} calls to make, {total} credits ({}). In this run's ledger already: {paid} credits ({}).\nCap: {cap} credits ({}) with what the ledger holds. Retries: up to {retries} rounds of single-cell calls on gen4_image_turbo at 2 credits a refused cell, and up to {retries} more tries of a failed task (refunded unless refused for safety), all within the cap.",
        dollars(total),
        dollars(paid),
        dollars(cap),
    ));
    (t, total)
}

/// The run folder's lock: the process id of the run using it. A second
/// run on the same folder would pay for the same calls, so it is refused
/// while the first is alive.
fn lock(run: &Path) -> Result<()> {
    let f = run.join("run.lock");
    if let Ok(text) = fs::read_to_string(&f) {
        let pid = text.trim();
        if !pid.is_empty()
            && pid != "free"
            && pid != std::process::id().to_string()
            && Command::new("kill")
                .args(["-0", pid])
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
        {
            bail!(
                "another gp theme generate (process {pid}) is using {}; wait for it",
                run.display()
            );
        }
    }
    fs::write(&f, std::process::id().to_string())?;
    Ok(())
}

fn unlock(run: &Path) {
    let _ = fs::write(run.join("run.lock"), "free");
}

/// gp itself, for the steps that are other gp commands.
fn gp() -> Result<Command> {
    Ok(Command::new(std::env::current_exe()?))
}

/// The refused slots and the readability verdict of `gp theme check
/// --original --json`'s output.
pub fn refused_slots(check: &Value) -> (Vec<String>, String) {
    let slots = check["originality"]["scores"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|s| s["refused"].as_bool() == Some(true))
                .filter_map(|s| s["what"].as_str())
                .filter(|w| !w.starts_with("sound"))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let verdict = check["readability"]["verdict"]
        .as_str()
        .unwrap_or("unknown")
        .to_lowercase();
    (slots, verdict)
}

/// `gp theme generate`.
pub fn run(a: &GenerateArgs, paths: &Paths) -> Result<Outcome> {
    let tdir = crate::theme_kit::template_dir(&a.game)?;
    let (game, _, _) = crate::theme_kit::manifest(&tdir);
    let run = a.run.clone().unwrap_or_else(|| {
        paths
            .home
            .join("work")
            .join("theme-runs")
            .join(&game)
            .join(safe(&a.style))
    });
    fs::create_dir_all(&run)?;
    let sheets = match &a.sheets {
        Some(d) => d.clone(),
        None => {
            let d = run.join("sheets");
            if !d.join("sheets.json").is_file() {
                let o = gp()?
                    .args(["theme", "sheet"])
                    .arg(&tdir)
                    .args(["--style", &a.style, "-o"])
                    .arg(&d)
                    .args(a.logo.iter().flat_map(|l| ["--logo", l.as_str()]))
                    .args(a.rom_guides.then_some("--rom-guides"))
                    .output()?;
                if !o.status.success() {
                    bail!(
                        "gp theme sheet failed, so there are no sheets to make; pass --sheets DIR for sheets written already:\n{}",
                        String::from_utf8_lossy(&o.stderr).trim()
                    );
                }
            }
            d
        }
    };
    // A guide made from the ROM goes to a service only when asked for.
    if !a.rom_guides {
        let v: Value = serde_json::from_str(&fs::read_to_string(sheets.join("sheets.json"))?)?;
        let rom: Vec<String> = v["pictures"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|p| p["rom_derived"].as_bool() == Some(true) && p["guide"].is_string())
            .filter_map(|p| p["id"].as_str().map(str::to_owned))
            .collect();
        if !rom.is_empty() {
            bail!(
                "these pictures' guides are made from your ROM (rom_derived in sheets.json): {}. They go to a service only with --rom-guides; else write the sheets again without it",
                rom.join(", ")
            );
        }
    }
    let manifest = Manifest::read(&sheets, &a.quality)?;
    let calls = select(&manifest, &a.only)?;
    // The card's own pictures may go too: they are the maker's.
    let given: Vec<&Path> = manifest.given.values().map(PathBuf::as_path).collect();
    let mut allowed: Vec<&Path> = vec![&sheets, &run];
    allowed.extend(given);
    let errs = validate(&calls, paths, &allowed);
    let ledger = Ledger::open(&run.join("ledger.jsonl"))?;
    let again: BTreeSet<String> = if a.again {
        calls.iter().map(|c| c.id.clone()).collect()
    } else {
        BTreeSet::new()
    };
    let mut keys: BTreeMap<String, String> = BTreeMap::new();
    let mut marked = Vec::new();
    for c in &calls {
        let k = format!("{}@{}", c.id, c.hash(&keys));
        let (st, _) = state_of(&ledger, &k, &c.out);
        keys.insert(c.id.clone(), k);
        let done =
            !again.contains(&c.id) && matches!(st, State::Done | State::Live(..) | State::Final(_));
        marked.push((c.clone(), done));
    }
    let paid = ledger.spent();
    let todo: u32 = marked.iter().filter(|x| !x.1).map(|x| x.0.credits).sum();
    let cap = a.budget.unwrap_or(paid + todo + todo.div_ceil(4));
    let (mut text, total) = estimate(&marked, paid, cap, a.retries);
    if !manifest.code.is_empty() {
        text.push_str(&format!(
            "\nDrawn in code, no call: {}.",
            manifest
                .code
                .iter()
                .map(|(s, _)| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if !errs.is_empty() {
        text.push_str(&format!(
            "\n\nProblems the API or the rules would refuse ({}); no paid run starts until they are fixed:\n  {}",
            errs.len(),
            errs.join("\n  ")
        ));
    }
    let plan = json!({
        "game": game,
        "style": a.style,
        "sheets": sheets,
        "run": run,
        "calls": marked.iter().map(|(c, done)| json!({
            "id": c.id, "kind": c.kind, "model": c.model, "ratio": c.ratio,
            "quality": c.quality, "background": c.background,
            "references": c.refs.iter().map(|(t, s)| json!({"tag": t, "from": s})).collect::<Vec<_>>(),
            "prompt_sha256": gp_theme::format::sha256_hex(c.prompt.as_bytes()),
            "prompt_chars": c.prompt.chars().count(),
            "credits": c.credits, "in_ledger": done, "out": c.out,
        })).collect::<Vec<_>>(),
        "to_make": marked.iter().filter(|x| !x.1).count(),
        "credits": total,
        "dollars": f64::from(total) * rw::DOLLARS_PER_CREDIT,
        "ledger_credits": paid,
        "cap": cap,
        "retries": a.retries,
        "code": manifest.code.iter().map(|(s, w)| json!({"slot": s, "why": w})).collect::<Vec<_>>(),
        "problems": errs,
    });
    fs::write(run.join("plan.json"), serde_json::to_string_pretty(&plan)?)?;
    fs::write(run.join("estimate.txt"), format!("{text}\n"))?;
    if a.dry_run {
        return Ok(Outcome {
            json: plan,
            text: format!(
                "{text}\ndry run: no call made. The plan is in {}",
                run.join("plan.json").display()
            ),
            exit: u8::from(!errs.is_empty()),
        });
    }
    if !errs.is_empty() {
        bail!(
            "the plan has problems ({}), and nothing was sent:\n  {}",
            errs.len(),
            errs.join("\n  ")
        );
    }
    // A paid run from here on.
    if !a.yes && !std::io::stdin().is_terminal() {
        bail!(
            "a paid run needs --yes when not run from a terminal. Run with --dry-run first, show the estimate to the person paying, and pass --yes only after they say yes"
        );
    }
    let key = rw::load_key(&paths.home)?;
    let client = Client::new(&a.api, key);
    let res = paid_run(
        a, &tdir, &run, &sheets, &manifest, calls, ledger, cap, &text, total, &again, &client,
    );
    unlock(&run);
    res.map_err(|e| anyhow!("{}", client.redact(&format!("{e:#}"))))
}

#[allow(clippy::too_many_arguments)]
fn paid_run(
    a: &GenerateArgs,
    tdir: &Path,
    run: &Path,
    sheets: &Path,
    manifest: &Manifest,
    calls: Vec<Call>,
    ledger: Ledger,
    cap: u32,
    text: &str,
    total: u32,
    again: &BTreeSet<String>,
    client: &Client,
) -> Result<Outcome> {
    lock(run)?;
    // The balance, a free call, before asking.
    let org = client.request("GET", "/v1/organization", None)?;
    if org.status == 401 {
        bail!("Runway refused the key (HTTP 401): {}", org.body);
    }
    let balance = org.json()["creditBalance"].as_u64();
    if let Some(b) = balance
        && b < u64::from(total)
    {
        bail!("the account has {b} credits and the plan needs {total}; nothing was spent");
    }
    if !a.yes {
        eprintln!("{text}");
        if let Some(b) = balance {
            eprintln!("The account has {b} credits.");
        }
        eprint!("Spend up to {cap} credits ({})? Type yes: ", dollars(cap));
        let _ = std::io::stderr().flush();
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line)?;
        if line.trim() != "yes" {
            bail!("not confirmed; nothing was spent");
        }
    }
    let mut runner = Runner {
        client,
        run: run.to_owned(),
        ledger,
        cap,
        concurrency: a.concurrency as usize,
        retries: a.retries,
        poll: Duration::from_millis(a.poll_ms.max(1)),
        results: BTreeMap::new(),
        tasks: BTreeMap::new(),
        log: Vec::new(),
    };
    let first = runner.pass(&calls, again)?;
    let mut rounds = Vec::new();
    let mut post = json!(null);
    if !a.no_fit && first.stopped.is_none() {
        let theme = run.join("theme");
        for round in 0..=a.retries {
            let mut fit = gp()?;
            fit.args(["theme", "fit"]).arg(tdir);
            fit.arg("--sheets").arg(sheets).arg("-o").arg(&theme);
            fit.arg("--contact").arg(run.join("contact.png"));
            let fo = fit.output()?;
            if !fo.status.success() {
                let msg = format!(
                    "gp theme fit failed; the results are in {}: {}",
                    sheets.display(),
                    String::from_utf8_lossy(&fo.stderr).trim()
                );
                runner.say(msg.clone());
                post = json!({"fit": msg});
                break;
            }
            let mut ck = gp()?;
            ck.args(["--json", "theme", "check"])
                .arg(tdir)
                .arg("--theme")
                .arg(&theme)
                .arg("--original");
            if let Some(r) = &a.rom {
                ck.arg("--rom").arg(r);
            }
            let co = ck.output()?;
            let v: Value = serde_json::from_slice(&co.stdout).unwrap_or(Value::Null);
            let (slots, verdict) = refused_slots(&v);
            let cells: Vec<String> = manifest
                .cells
                .iter()
                .filter(|(_, (c, _, _))| slots.contains(&c.slot))
                .map(|(k, _)| k.clone())
                .collect();
            let pictures: Vec<&Call> = manifest
                .calls
                .iter()
                .filter(|c| c.kind == "picture" && c.slots.iter().any(|s| slots.contains(s)))
                .collect();
            rounds.push(json!({"round": round, "readability": verdict, "refused_slots": slots, "cells": cells}));
            runner.say(format!(
                "check round {round}: readability {verdict}, {} slots refused, {} cells to make again",
                slots.len(),
                cells.len()
            ));
            if cells.is_empty() && pictures.is_empty() {
                break;
            }
            if round == a.retries {
                runner.say(format!(
                    "after {} rounds these slots are still refused; a person picks: {}",
                    a.retries,
                    slots.join(", ")
                ));
                break;
            }
            let mut again_calls: Vec<Call> = cells
                .iter()
                .filter_map(|c| manifest.cell_call(c, round + 1))
                .collect();
            for p in pictures {
                let mut p = p.clone();
                p.id = format!("{}#r{}", p.id, round + 1);
                again_calls.push(p);
            }
            // Their references are done; the pass needs them as calls.
            let mut with_refs: Vec<Call> = manifest
                .calls
                .iter()
                .filter(|c| c.kind != "sheet" && c.kind != "picture")
                .cloned()
                .collect();
            with_refs.extend(again_calls.iter().cloned());
            let p = runner.pass(&with_refs, &BTreeSet::new())?;
            if p.stopped.is_some() {
                rounds.push(json!({"stopped": p.stopped}));
                break;
            }
        }
        if post.is_null() {
            runner.say(format!(
                "contact sheet: {}",
                run.join("contact.png").display()
            ));
        }
    }
    let spent = runner.ledger.spent();
    let text = format!(
        "{}\n{} done, {} failed, {} blocked; {spent} credits spent or held ({}) of a cap of {cap}{}\nledger: {}",
        runner.log.join("\n"),
        first.done.len(),
        first.failed.len(),
        first.blocked.len(),
        dollars(spent),
        first
            .stopped
            .as_ref()
            .map(|s| format!("\nstopped: {s}; run the same command again to continue"))
            .unwrap_or_default(),
        run.join("ledger.jsonl").display()
    );
    let exit = u8::from(first.stopped.is_some() || !first.failed.is_empty());
    Ok(Outcome {
        json: json!({
            "run": run,
            "pass": first,
            "rounds": rounds,
            "post": post,
            "credits": spent,
            "dollars": f64::from(spent) * rw::DOLLARS_PER_CREDIT,
            "cap": cap,
        }),
        text,
        exit,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sheets folder in `gp-sheets 1`, as gp theme sheet writes one.
    pub(crate) fn write_sheets(d: &Path, hero: &str, picture: &str) {
        for sub in ["refs", "pictures"] {
            fs::create_dir_all(d.join(sub)).unwrap();
        }
        fs::write(d.join("sheet-01.png"), Rgba::new(1024, 1024).png()).unwrap();
        fs::write(
            d.join("pictures/picture-stage.guide.png"),
            Rgba::new(256, 240).png(),
        )
        .unwrap();
        for (f, t) in [
            (
                "refs/style-board.gen4_image.txt",
                "A style board of folded paper.",
            ),
            ("refs/turn-hero.gen4_image.txt", hero),
            (
                "sheet-01.gpt_image_2.txt",
                "Fill each bordered cell of @grid with @turn_hero in @style_board.",
            ),
            ("sheet-01.gen4_image.txt", "Fill @grid with @turn_hero."),
            ("pictures/picture-stage.gen4_image.txt", picture),
        ] {
            fs::write(d.join(f), t).unwrap();
        }
        fs::write(
            d.join("sheets.json"),
            serde_json::to_string(&json!({
                "format": "gp-sheets 1", "game": "test", "style": null, "key": "#FF00FF",
                "refs": [
                    {"id": "style-board", "kind": "style-board", "slots": [], "model": "gen4_image", "ratio": "1920:1080",
                     "prompt_file": "refs/style-board.gen4_image.txt", "out": "refs/style-board.png", "refs": []},
                    {"id": "turn-hero", "kind": "turnaround", "subject": "hero", "slots": ["entity.player"], "model": "gen4_image",
                     "ratio": "1440:1080", "prompt_file": "refs/turn-hero.gen4_image.txt", "out": "refs/turn-hero.png", "refs": ["style-board"]}
                ],
                "sheets": [{"id": "sheet-01", "image": "sheet-01.png", "size": [1024, 1024], "scale": 16, "model": "gpt_image_2",
                    "prompts": {"gpt_image_2": "sheet-01.gpt_image_2.txt", "gen4_image": "sheet-01.gen4_image.txt"},
                    "out": "sheet-01.out.png", "refs": ["style-board", "turn-hero"],
                    "cells": [{"cell": "A01", "slot": "entity.player", "label": "hero", "frames": [0, 3], "frame_names": ["base", "clip1.0"],
                               "rect": [16, 48, 256, 256], "body": [32, 64, 224, 224], "size": [16, 16], "anchor": "bottom",
                               "baseline": 300, "pose": "standing"}]}],
                "pictures": [{"id": "picture-stage", "slot": "picture.stage.main", "size": [256, 240], "model": "gen4_image",
                    "ratio": "1440:1080", "prompt_file": "pictures/picture-stage.gen4_image.txt", "out": "pictures/picture-stage.out.png",
                    "guide": "pictures/picture-stage.guide.png", "refs": ["style-board", "guide"]}],
                "code": [{"slot": "font", "why": "8 by 8"}]
            }))
            .unwrap(),
        )
        .unwrap();
    }

    fn manifest_dir() -> tempfile::TempDir {
        let t = tempfile::tempdir().unwrap();
        write_sheets(
            t.path(),
            "A hero turnaround in @style_board.",
            "A stage in @style_board on @guide.",
        );
        t
    }

    #[test]
    fn the_manifest_reads_as_calls_with_credits() {
        let t = manifest_dir();
        let m = Manifest::read(t.path(), "high").unwrap();
        let ids: Vec<&str> = m.calls.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "ref:style-board",
                "ref:turn-hero",
                "sheet:sheet-01",
                "picture:picture-stage"
            ]
        );
        let credits: Vec<u32> = m.calls.iter().map(|c| c.credits).collect();
        assert_eq!(credits, [8, 8, 20, 8]);
        assert_eq!(m.calls[2].ratio, "1920:1920");
        let tags: Vec<&str> = m.calls[2].refs.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(tags, ["grid", "style_board", "turn_hero"]);
        assert_eq!(m.calls[2].after(), ["ref:style-board", "ref:turn-hero"]);
        assert_eq!(m.calls[2].out, t.path().join("sheet-01.out.png"));
        let tags: Vec<&str> = m.calls[3].refs.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(tags, ["style_board", "guide"]);
        assert_eq!(m.code, [("font".to_owned(), "8 by 8".to_owned())]);
        let cell = m.cell_call("A01", 1).unwrap();
        assert_eq!(cell.id, "cell:A01#r1");
        assert_eq!(cell.out, t.path().join("cells/A01.r1.png"));
        assert!(
            cell.prompt.contains("@style_board and @turn_hero"),
            "{}",
            cell.prompt
        );
        let p = cell.paste.unwrap();
        assert_eq!((p.rect, p.grid), ([16, 48, 256, 256], [1024, 1024]));
    }

    #[test]
    fn a_card_image_is_used_and_not_made() {
        let t = manifest_dir();
        let f = t.path().join("sheets.json");
        let mut v: Value = serde_json::from_str(&fs::read_to_string(&f).unwrap()).unwrap();
        v["refs"][1]["image"] = json!("/cards/hero.png");
        fs::write(&f, v.to_string()).unwrap();
        let m = Manifest::read(t.path(), "high").unwrap();
        assert!(m.calls.iter().all(|c| c.id != "ref:turn-hero"));
        assert!(
            m.calls[1]
                .refs
                .contains(&("turn_hero".into(), Source::File("/cards/hero.png".into())))
        );
    }

    #[test]
    fn a_cell_pastes_into_its_filled_grid() {
        let t = tempfile::tempdir().unwrap();
        let sheet = t.path().join("s.png");
        fs::write(&sheet, Rgba::new(512, 512).png()).unwrap();
        let mut cell = Rgba::new(10, 10);
        cell.pixels.fill([1, 2, 3, 255]);
        fs::write(t.path().join("c.png"), cell.png()).unwrap();
        paste(
            &t.path().join("c.png"),
            &Paste {
                sheet: sheet.clone(),
                rect: [100, 100, 200, 100],
                grid: [1024, 1024],
            },
        )
        .unwrap();
        let s = Rgba::load(&sheet).unwrap();
        // The rect at half size is 50,50 100x50: the figure 50x50, centered.
        assert_eq!(s.get(50, 50), [255, 0, 255, 255]);
        assert_eq!(s.get(75, 50), [1, 2, 3, 255]);
        assert_eq!(s.get(124, 99), [1, 2, 3, 255]);
        assert_eq!(s.get(126, 99), [255, 0, 255, 255]);
        assert_eq!(s.get(10, 10), [0, 0, 0, 0]);
    }

    #[test]
    fn only_adds_what_its_calls_need() {
        let t = manifest_dir();
        let m = Manifest::read(t.path(), "high").unwrap();
        let ids = |o: &[&str]| -> Vec<String> {
            select(&m, &o.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>())
                .unwrap()
                .into_iter()
                .map(|c| c.id)
                .collect()
        };
        assert_eq!(ids(&["style-board"]), ["ref:style-board"]);
        assert_eq!(
            ids(&["entity.player"]),
            ["ref:style-board", "ref:turn-hero", "sheet:sheet-01"]
        );
        assert_eq!(
            ids(&["A01"]),
            ["ref:style-board", "ref:turn-hero", "cell:A01"]
        );
        assert!(select(&m, &["nothing".into()]).is_err());
    }

    #[test]
    fn validation_refuses_before_paying() {
        let t = manifest_dir();
        let home = tempfile::tempdir().unwrap();
        let paths = Paths {
            home: home.path().to_owned(),
            store: home.path().join("store"),
            roms: home.path().join("roms"),
        };
        let m = Manifest::read(t.path(), "high").unwrap();
        assert!(validate(&m.calls, &paths, &[t.path()]).is_empty());
        let mut bad = m.calls.clone();
        bad[0].ratio = "7:3".into();
        bad[1].prompt = "x".repeat(1001);
        bad[2]
            .refs
            .push(("no".into(), Source::Call("ref:none".into())));
        bad[2].refs.push((
            "turn_the_one_to_rescue".into(),
            Source::Call("ref:style-board".into()),
        ));
        let kits = home.path().join("work/theme-kits/game");
        fs::create_dir_all(&kits).unwrap();
        fs::write(kits.join("guide.png"), Rgba::new(8, 8).png()).unwrap();
        bad[3]
            .refs
            .push(("guide".into(), Source::File(kits.join("guide.png"))));
        let errs = validate(&bad, &paths, &[t.path(), kits.as_path()]).join("\n");
        assert!(errs.contains("no ratio 7:3"), "{errs}");
        assert!(errs.contains("1001 characters"), "{errs}");
        assert!(errs.contains("tag `no`"), "{errs}");
        assert!(errs.contains("refers to ref:none"), "{errs}");
        assert!(errs.contains("tag `turn_the_one_to_rescue`"), "{errs}");
        assert!(errs.contains("a theme kit's local part"), "{errs}");
    }

    #[test]
    fn the_ledger_keeps_a_running_total_and_states() {
        let t = tempfile::tempdir().unwrap();
        let f = t.path().join("ledger.jsonl");
        let mut l = Ledger::open(&f).unwrap();
        let e = |event: &str, attempt: u32| Entry {
            event: event.into(),
            call: "c".into(),
            key: "c@1".into(),
            attempt,
            credits_est: 20,
            ..Default::default()
        };
        l.write(e("submitting", 0)).unwrap();
        assert_eq!(l.spent(), 20);
        l.write(Entry {
            task: Some("t0".into()),
            ..e("submitted", 0)
        })
        .unwrap();
        l.write(Entry {
            status: Some("FAILED".into()),
            credits: Some(0),
            failure_code: Some("INTERNAL".into()),
            ..e("ended", 0)
        })
        .unwrap();
        assert_eq!(l.spent(), 0, "a refund");
        l.write(e("submitting", 1)).unwrap();
        l.write(Entry {
            status: Some("HTTP 429".into()),
            ..e("rejected", 1)
        })
        .unwrap();
        assert_eq!(l.spent(), 0, "a refused request costs nothing");
        l.write(e("submitting", 2)).unwrap();
        l.write(Entry {
            task: Some("t2".into()),
            ..e("submitted", 2)
        })
        .unwrap();
        let l = Ledger::open(&f).unwrap();
        assert_eq!(l.spent(), 20);
        assert_eq!(
            state_of(&l, "c@1", &t.path().join("x.png")).0,
            State::Live("t2".into(), 2)
        );
        let last: Entry =
            serde_json::from_str(fs::read_to_string(&f).unwrap().lines().last().unwrap()).unwrap();
        assert_eq!(last.spent, 20);
    }

    #[test]
    fn refused_slots_come_from_the_check() {
        let v = json!({"readability": {"verdict": "Warn"}, "originality": {"scores": [
            {"what": "entity.player", "value": 0.8, "judged": true, "refused": true},
            {"what": "entity.drum", "value": 0.2, "judged": true, "refused": false},
            {"what": "sound jump", "value": 0.9, "judged": true, "refused": true}
        ]}});
        assert_eq!(
            refused_slots(&v),
            (vec!["entity.player".to_owned()], "warn".to_owned())
        );
    }
}
