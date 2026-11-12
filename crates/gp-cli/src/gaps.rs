//! `gp gaps`: what a recording of the original shows that the model
//! doesn't account for (docs/gaps.md). Detect and catalog only; nothing
//! here changes a model. The detectors are `gp-gaps`; this module builds
//! their per-frame facts from a recording (its frame trace, its event
//! stream and, with the ROM, a replay with the code observer
//! `gp_coverage::activity`) and the template's model and state mapping,
//! prints the ranked lists, and writes them under `$GP_HOME/reports/gaps/`
//! for the scorecard.
//!
//! The facts are the NES's: OAM sprites, nametable cells, APU writes and
//! 6502 code. A game whose package names an arcade platform is refused in
//! words; it needs gp-arcade's sprite-list and tilemap adapter and its
//! memory log (port wave 6).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use gp_evidence::Paths;
use gp_gaps::{
    CellChange, Code, Ent, FrameFacts, Known, Object, RamLabel, RamWrite, RomLabel, Run,
};
use gp_interp::mapping::{Field, Mapping};
use gp_runtime::Value;
use gp_runtime::select::{Condition, Index, Selector};
use serde_json::json;

use crate::Outcome;
use crate::open::Opened;
use crate::recordings::parse_frames;

#[derive(Subcommand)]
pub enum GapsCmd {
    /// State that changes during play that the model doesn't account for:
    /// RAM, sprites, background, sound and code, each ranked.
    Changes(GapsArgs),
    /// Every background cell that changes during play and every handler
    /// routine that fires, with its context, clustered into interactive
    /// element kinds (docs/formats/interactive.md).
    Interactive(GapsArgs),
    /// The model's sprite boxes against the original's sprites for the
    /// same objects: offsets and size differences per entity kind.
    Drawn(GapsArgs),
    /// All three from one read of the recording.
    All(GapsArgs),
}

#[derive(Args, Clone)]
pub struct GapsArgs {
    /// The template: a directory holding model.toml and mapping.toml (a
    /// game's package folder), or a name under `--dir`.
    pub template: String,
    /// The recording's ID, or a unique prefix of at least 4 characters.
    #[arg(long)]
    pub recording: String,
    /// Which frames: `N`, `A..B` (B excluded), `A..=B` or `A..`.
    #[arg(long, default_value = "1..")]
    pub frames: String,
    /// Windows to explain, as for `--frames` (repeat for more): each lists
    /// what the original did in those frames that the model doesn't
    /// account for (the interactions, cell changes, sound requests,
    /// counters, sprites, and the routines and RAM rare elsewhere). The
    /// detectors still read every frame of `--frames`, so rare means rare in
    /// the whole recording (docs/show.md).
    #[arg(long)]
    pub focus: Vec<String>,
    /// The game's labels file (docs/formats/labels.md), instead of the one
    /// for the mapping's source.
    #[arg(long)]
    pub labels: Option<PathBuf>,
    /// An ld65 debug file whose labels name the routines, for a game whose
    /// labels file has RAM labels only (Super Mario Bros.'s `just
    /// smb-listing`). Used for boards with 32KB of PRG or less.
    #[arg(long)]
    pub listing: Option<PathBuf>,
    /// The model's lifted assets, for the drawn-box check (default
    /// $GP_HOME/work/assets/<source>).
    #[arg(long)]
    pub assets: Option<PathBuf>,
    /// A labelled reference set to score the detectors against
    /// (docs/gaps.md, "Scoring"), instead of the one kept for the
    /// template in crates/gp-gaps/truth/. Scoring needs `gaps all` and the
    /// code signals.
    #[arg(long)]
    pub truth: Option<PathBuf>,
    /// Skip the replay with the code observer, which needs the ROM.
    #[arg(long)]
    pub no_code: bool,
    /// Threads for the replay.
    #[arg(long, default_value_t = 8)]
    pub threads: usize,
    /// Items printed per place.
    #[arg(long, default_value_t = 10)]
    pub top: usize,
    /// Frames the sprite table lags the game's RAM (as for gp track).
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u64).range(0..=1))]
    pub oam_lag: u64,
    /// The folder holding the templates (default: `templates`, then the
    /// games checkout, $GP_GAMES_DIR).
    #[arg(long)]
    pub dir: Option<PathBuf>,
    /// Write the reports here instead of $GP_HOME/reports/gaps/<template>.
    #[arg(long)]
    pub out: Option<PathBuf>,
}

pub(crate) fn run(cmd: &GapsCmd, paths: &Paths) -> Result<Outcome> {
    let (args, which) = match cmd {
        GapsCmd::Changes(a) => (a, [true, false, false]),
        GapsCmd::Interactive(a) => (a, [false, true, false]),
        GapsCmd::Drawn(a) => (a, [false, false, true]),
        GapsCmd::All(a) => (a, [true, true, true]),
    };
    let t = Template::open(args.dir.as_deref(), &args.template)?;
    let built = build(args, &t, paths)?;
    let out_dir = args
        .out
        .clone()
        .unwrap_or_else(|| paths.reports_dir().join("gaps").join(&t.id));
    std::fs::create_dir_all(&out_dir).with_context(|| out_dir.display().to_string())?;
    let mut text = String::new();
    let mut js = serde_json::Map::new();
    let _ = writeln!(
        text,
        "{} on {}: frames {}..{} ({} in play){}",
        t.id,
        built.short,
        built.range.start,
        built.range.end,
        built.run.play_frames(),
        if built.run.has_code() {
            String::new()
        } else {
            format!(", no code signals ({})", built.no_code_why)
        }
    );
    let n = args.top;
    let meta = json!({"template": t.id, "recording": built.id, "frames": [built.range.start, built.range.end]});
    let save = |name: &str, v: &serde_json::Value| -> Result<()> {
        let mut v = v.clone();
        if let Some(o) = v.as_object_mut() {
            o.insert("about".into(), meta.clone());
        }
        let text = serde_json::to_string_pretty(&v)?;
        let f = out_dir.join(format!("{name}.json"));
        std::fs::write(&f, &text).with_context(|| f.display().to_string())?;
        // Each recording's reports are also kept apart, so a later run on
        // another recording doesn't hide them.
        if args.out.is_none() {
            let own = out_dir.join(&built.id[..12]);
            std::fs::create_dir_all(&own)?;
            std::fs::write(own.join(format!("{name}.json")), &text)?;
        }
        Ok(())
    };
    let mut changes_r = None;
    let mut catalog_r = None;
    if which[0] {
        let r = gp_gaps::changes::detect(&built.run);
        changes_text(&mut text, &r, n);
        let v = serde_json::to_value(&r)?;
        save("changes", &v)?;
        js.insert("changes".into(), v);
        changes_r = Some(r);
    }
    if which[1] {
        let (cat, interactions) = gp_gaps::interactive::catalog(&built.run);
        interactive_text(&mut text, &cat, n);
        let mut v = serde_json::to_value(&cat)?;
        if let Some(o) = v.as_object_mut() {
            o.insert("game".into(), json!(t.source));
        }
        save("interactive", &v)?;
        save("interactions", &json!({"interactions": interactions}))?;
        js.insert("interactive".into(), v);
        catalog_r = Some(cat);
    }
    if which[2] {
        match model_boxes(args, &t, &built.run.kinds, paths) {
            Ok(boxes) => {
                let r = gp_gaps::drawn::check(&built.run, &boxes);
                drawn_text(&mut text, &r, n);
                let v = serde_json::to_value(&r)?;
                save("drawn", &v)?;
                js.insert("drawn".into(), v);
            }
            Err(e) => {
                let _ = writeln!(text, "\ndrawn: can't build the model's boxes: {e:#}");
                js.insert("drawn".into(), json!({"error": format!("{e:#}")}));
            }
        }
    }
    // Detection quality: the findings against the game's labelled
    // reference set, when it has one.
    let truth_text = match &args.truth {
        Some(p) => Some(std::fs::read_to_string(p).with_context(|| p.display().to_string())?),
        None => gp_gaps::score::builtin(&t.id).map(str::to_owned),
    };
    if let Some(tt) = truth_text
        && which[0]
        && which[1]
        && built.run.has_code()
    {
        let truth = gp_gaps::score::Truth::parse(&tt).map_err(anyhow::Error::msg)?;
        let r = changes_r
            .take()
            .unwrap_or_else(|| gp_gaps::changes::detect(&built.run));
        let cat = match catalog_r.take() {
            Some(c) => c,
            None => gp_gaps::interactive::catalog(&built.run).0,
        };
        let sc = gp_gaps::score::score(&built.run, &truth, &r, &cat);
        score_text(&mut text, &sc);
        let v = serde_json::to_value(&sc)?;
        save("score", &v)?;
        js.insert("score".into(), v);
    }
    if !args.focus.is_empty() {
        let mut windows = Vec::new();
        for spec in &args.focus {
            let r = parse_frames(spec, built.range.end.saturating_sub(1))?;
            windows.push((r.start, r.end - 1));
        }
        let fs = focus_of(&built.run, &windows);
        for f in &fs {
            let _ = writeln!(
                text,
                "\nfocus {}..={} ({} in play):",
                f.from, f.to, f.play_frames
            );
            let lines = f.lines();
            if lines.is_empty() {
                let _ = writeln!(text, "  nothing the model doesn't account for");
            }
            for l in lines {
                let _ = writeln!(text, "  {l}");
            }
        }
        let v = serde_json::to_value(&fs)?;
        save("focus", &json!({"windows": v}))?;
        js.insert("focus".into(), v);
    }
    let _ = writeln!(text, "\nreports: {}", out_dir.display());
    Ok(Outcome::ok(
        serde_json::Value::Object(js),
        text.trim_end().to_owned(),
    ))
}

/// The model.toml in `dir`.
pub(crate) fn load_model(dir: &Path) -> Result<gp_model::Model> {
    let file = dir.join(gp_model::MODEL_FILE);
    let text =
        std::fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
    gp_model::Model::parse(&text).with_context(|| file.display().to_string())
}

/// A template's model and mapping.
pub(crate) struct Template {
    pub id: String,
    pub dir: PathBuf,
    pub model: gp_model::Model,
    pub mapping: Mapping,
    pub source: String,
}

impl Template {
    /// `name` as a folder, else under `base`, else under `templates` and
    /// the games checkout. A game whose package names an arcade platform is
    /// refused in words.
    pub fn open(base: Option<&Path>, name: &str) -> Result<Self> {
        let bases: Vec<PathBuf> = match base {
            Some(b) => vec![b.to_path_buf()],
            None => std::iter::once(PathBuf::from("templates"))
                .chain(crate::roms_cmd::games_dir(None))
                .collect(),
        };
        let dir = std::iter::once(PathBuf::from(name))
            .chain(bases.iter().map(|b| b.join(name)))
            .find(|p| p.join("model.toml").is_file())
            .with_context(|| format!("no template `{name}` with a model.toml"))?;
        crate::track::nes_package(&dir, "gp gaps")?;
        let id = std::fs::canonicalize(&dir)
            .unwrap_or_else(|_| dir.clone())
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let model = load_model(&dir)?;
        let mtext = std::fs::read_to_string(dir.join(gp_interp::mapping::MAPPING_FILE))
            .with_context(|| {
                format!(
                    "{} has no mapping.toml; gp gaps reads the original through it",
                    dir.display()
                )
            })?;
        let mapping = Mapping::parse(&mtext).map_err(anyhow::Error::msg)?;
        Ok(Self {
            id,
            source: mapping.source.clone(),
            dir,
            model,
            mapping,
        })
    }
}

pub(crate) struct Built {
    pub run: Run,
    pub range: std::ops::Range<u64>,
    pub short: String,
    pub id: String,
    pub no_code_why: String,
}

/// What the original did in each window (`from`, `to`, both included),
/// from one run of the whole-recording detectors.
pub(crate) fn focus_of(run: &Run, windows: &[(u64, u64)]) -> Vec<gp_gaps::focus::Focus> {
    let (cat, interactions) = gp_gaps::interactive::catalog(run);
    let (_, requests) = gp_gaps::sound::detect(run);
    let found = gp_gaps::focus::Found {
        catalog: &cat,
        interactions: &interactions,
        requests: &requests,
    };
    windows
        .iter()
        .map(|&(a, b)| gp_gaps::focus::window(run, &found, a, b))
        .collect()
}

impl GapsArgs {
    /// The arguments `gp sound capture` builds a run with: the sound
    /// requests come from the same detector.
    pub(crate) fn for_sound(
        template: &str,
        recording: &str,
        frames: &str,
        dir: Option<&Path>,
        listing: Option<PathBuf>,
    ) -> Self {
        Self {
            template: template.to_owned(),
            recording: recording.to_owned(),
            frames: frames.to_owned(),
            labels: None,
            listing,
            assets: None,
            truth: None,
            no_code: false,
            threads: 8,
            top: 10,
            oam_lag: 1,
            focus: Vec::new(),
            dir: dir.map(Path::to_path_buf),
            out: None,
        }
    }
}

/// The per-frame facts `gp gaps` reads, and why there are no code signals
/// (empty when there are).
pub(crate) fn build_run(args: &GapsArgs, t: &Template, paths: &Paths) -> Result<(Run, String)> {
    let b = build(args, t, paths)?;
    Ok((b.run, b.no_code_why))
}

fn whole(v: &Value) -> Option<i64> {
    match v {
        Value::Int(n) => Some(*n),
        Value::Fixed(raw) => Some(raw >> 16),
        Value::Flag(b) => Some(i64::from(*b)),
        _ => None,
    }
}

pub(crate) fn read(f: &Field, ram: &[u8], slot: u16) -> Option<i64> {
    // A field read from the sprite table, the nametables or a machine's
    // state has no RAM bytes to read here. (game-printer panicked on one,
    // Donkey Kong's `counters.frame_odd`.)
    if !f.index
        && (f.machine.is_some()
            || f.oam_tile.is_some()
            || !f.oam.is_empty()
            || !f.nametable.is_empty())
    {
        return None;
    }
    let bytes: Vec<u8> = f
        .slot_addresses(slot)
        .into_iter()
        .map(|a| ram[usize::from(a & 0x7FF)])
        .collect();
    whole(&f.slot_value(slot, &bytes))
}

fn eval(c: &Condition, ram: &[u8]) -> bool {
    c.eval(&mut |s: &Selector| match s.segments.as_slice() {
        [seg] if seg.name == "ram" => match seg.index {
            Some(Index::At(n)) => Ok(i64::from(ram[(n as usize) & 0x7FF])),
            _ => Err("not a byte".into()),
        },
        _ => Err("only ram[N]".into()),
    })
    .unwrap_or(false)
}

/// Every identifier in `text`.
fn tokens(text: &str, out: &mut BTreeSet<String>) {
    for t in text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')) {
        if t.len() >= 3
            && t.chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        {
            out.insert(t.to_owned());
        }
    }
}

fn read_tree(dir: &Path, out: &mut BTreeSet<String>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            if p.file_name().is_some_and(|n| n != "target") {
                read_tree(&p, out);
            }
        } else if p.extension().is_some_and(|x| x == "rs" || x == "toml")
            && let Ok(t) = std::fs::read_to_string(&p)
        {
            tokens(&t, out);
        }
    }
}

/// What the model already accounts for: mapped RAM, and every name its
/// files, mechanic records and hooks cite.
fn known(t: &Template) -> Known {
    let mut k = Known {
        oam_page: 2,
        ..Known::default()
    };
    let m = &t.mapping;
    for f in &m.fields {
        k.mapped.extend(f.ram.iter().chain(&f.frac).copied());
    }
    if let Some(s) = &m.slots {
        for i in 0..s.count {
            k.mapped.insert(s.active_at(i));
        }
        let mut bases = Vec::new();
        bases.extend(s.id);
        bases.extend(s.state);
        for b in bases {
            for i in 0..s.count {
                k.mapped.insert(b + i);
            }
        }
        for f in s
            .fields
            .iter()
            .chain(s.kinds.iter().flat_map(|k| &k.fields))
        {
            for i in 0..s.count {
                k.mapped.extend(f.slot_addresses(i));
            }
        }
        k.mapped.extend(s.clock);
    }
    for file in [
        "model.toml",
        "mapping.toml",
        "fidelity.toml",
        "observe.toml",
    ] {
        if let Ok(text) = std::fs::read_to_string(t.dir.join(file)) {
            tokens(&text, &mut k.cited);
        }
    }
    read_tree(&t.dir.join("ext"), &mut k.cited);
    let (exts, _) = gp_exts::registry(&t.model.model.name);
    let cat = gp_interp::mechanics::catalog(&t.model, &exts);
    if let Ok(text) = serde_json::to_string(&cat) {
        tokens(&text, &mut k.cited);
    }
    for mech in &cat.mechanics {
        let writes = mech.writes.iter().any(|w| w.contains("tile"));
        let about = mech.about.to_ascii_lowercase();
        if writes || about.contains("set_tile") || about.contains("changes a tile") {
            k.tile_mechanics.push(mech.name.to_owned());
        }
    }
    // A hook changes level cells through `ExtCx::set_tile`.
    if let Ok(rd) = std::fs::read_dir(t.dir.join("ext/src")) {
        for e in rd.flatten() {
            if std::fs::read_to_string(e.path()).is_ok_and(|s| s.contains("set_tile(")) {
                k.tile_mechanics.push(format!(
                    "ext hook code in {}",
                    e.file_name().to_string_lossy()
                ));
            }
        }
    }
    if let Some(h) = t.model.present.as_ref().and_then(|p| p.hud.as_ref()) {
        let rows = match h.size {
            Some([_, rows]) => rows,
            None => h.field.iter().map(|f| f.at[1] + 1).max().unwrap_or(0),
        };
        k.hud_rows = rows.min(255) as u8;
    }
    let text = std::fs::read_to_string(t.dir.join("model.toml")).unwrap_or_default();
    k.sound = text.contains("[sound") || text.contains("[audio");
    k
}

/// A labels file, schema 0 (docs/formats/labels.md): the parts gp gaps
/// reads.
#[derive(serde::Deserialize)]
struct LabelsFile {
    schema: u32,
    #[serde(default)]
    label: Vec<LabelEntry>,
    #[serde(default)]
    routine: Vec<RomEntry>,
    #[serde(default)]
    data: Vec<RomEntry>,
}

#[derive(serde::Deserialize)]
struct LabelEntry {
    name: String,
    addr: u16,
    size: u16,
}

#[derive(serde::Deserialize)]
struct RomEntry {
    name: String,
    addr: u16,
    prg: u32,
    size: u32,
}

fn parse_labels(text: &str) -> Result<LabelsFile> {
    let file: LabelsFile = toml::from_str(text)?;
    anyhow::ensure!(file.schema == 0, "labels schema {} isn't 0", file.schema);
    Ok(file)
}

/// The template's labels file: `labels.toml` in its folder, else the one
/// this repo keeps for its source (`games/<source>/labels.toml`, Moss
/// Hop's), else the one beside the roms.toml in the games checkout that
/// lists the source. The old tool kept a table of files per game,
/// generated under $GP_HOME for the games gameprinter doesn't carry.
fn labels_file(t: &Template) -> Option<PathBuf> {
    const FILE: &str = "labels.toml";
    let own = t.dir.join(FILE);
    if own.is_file() {
        return Some(own);
    }
    let plain = !t.source.is_empty() && !t.source.contains('/') && !t.source.contains("..");
    let has = |d: &Path| d.join("games").join(&t.source).join(FILE).is_file();
    let repo = std::env::current_dir()
        .ok()
        .and_then(|h| h.ancestors().find(|d| has(d)).map(Path::to_path_buf))
        .or_else(|| {
            let built = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
            has(&built).then_some(built)
        });
    if plain && let Some(r) = repo {
        return Some(r.join("games").join(&t.source).join(FILE));
    }
    let games = crate::roms_cmd::games_dir(None)?;
    std::fs::read_dir(&games)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|d| d.join(FILE).is_file())
        .find(|d| {
            std::fs::read_to_string(d.join("roms.toml"))
                .ok()
                .and_then(|text| text.parse::<toml::Table>().ok())
                .and_then(|t| t.get("source").and_then(toml::Value::as_array).cloned())
                .is_some_and(|sources| {
                    sources.iter().any(|s| {
                        s.get("id").and_then(toml::Value::as_str) == Some(t.source.as_str())
                    })
                })
        })
        .map(|d| d.join(FILE))
}

/// RAM and ROM labels: the labels file, and for routines a listing's
/// symbols when the file has none.
fn labels(
    args: &GapsArgs,
    t: &Template,
    paths: &Paths,
    prg_len: usize,
    k: &mut Known,
    constants: &mut Vec<gp_gaps::Constant>,
) -> Vec<String> {
    let mut notes = Vec::new();
    let file = match &args.labels {
        Some(f) => Some(f.clone()),
        None => labels_file(t),
    };
    let labels_dir = file
        .as_ref()
        .filter(|f| f.is_file())
        .and_then(|f| f.parent().map(Path::to_path_buf));
    if let Some(f) = file.filter(|f| f.is_file()) {
        match std::fs::read_to_string(&f)
            .map_err(anyhow::Error::from)
            .and_then(|s| parse_labels(&s))
        {
            Ok(l) => {
                k.ram = l
                    .label
                    .iter()
                    .map(|x| RamLabel {
                        name: x.name.clone(),
                        addr: x.addr,
                        size: x.size,
                    })
                    .collect();
                k.rom = l
                    .routine
                    .iter()
                    .map(|x| RomLabel {
                        name: x.name.clone(),
                        prg: x.prg,
                        addr: x.addr,
                        size: x.size,
                    })
                    .collect();
                k.data = l
                    .data
                    .iter()
                    .map(|x| RomLabel {
                        name: x.name.clone(),
                        prg: x.prg,
                        addr: x.addr,
                        size: x.size,
                    })
                    .collect();
            }
            Err(e) => notes.push(format!("labels {}: {e:#}", f.display())),
        }
    } else {
        notes.push(format!("no labels file for {}", t.source));
    }
    // A listing: the one given, else an ld65 debug file or an assembler
    // listing beside the labels file (in its folder, or its build/ or
    // listing/ folder), as the labels scripts leave them.
    let beside = labels_dir.as_ref().and_then(|d| {
        [d.clone(), d.join("listing"), d.join("build")]
            .into_iter()
            .filter_map(|d| std::fs::read_dir(d).ok())
            .flat_map(|rd| rd.flatten().map(|e| e.path()).collect::<Vec<_>>())
            .filter(|p| p.extension().is_some_and(|x| x == "dbg" || x == "lst"))
            .min_by_key(|p| p.extension().is_some_and(|x| x == "lst"))
    });
    let listing = args.listing.clone().or(beside).or_else(|| {
        (t.source == "smb1-ntsc-prg0").then(|| paths.home.join("work/smbdis/listing/smbdis.dbg"))
    });
    let asm_listing = listing
        .as_ref()
        .is_some_and(|p| p.extension().is_some_and(|x| x == "lst"));
    if let Some(text) = listing
        .filter(|p| p.is_file())
        .and_then(|l| std::fs::read_to_string(l).ok())
    {
        if asm_listing {
            *constants = asm_listing_constants(&text);
        } else {
            *constants = listing_constants(&text);
        }
        if k.rom.is_empty() && !asm_listing {
            if prg_len == 0 || prg_len > 0x8000 {
                notes.push(
                    "a listing's code labels are used only for boards of 32KB PRG or less".into(),
                );
            } else {
                k.rom = listing_symbols(&text, prg_len);
            }
        }
    }
    k.rom.sort_by_key(|l| l.prg);
    k.rom.dedup_by_key(|l| l.prg);
    // Sizes from a listing are the gap to the next label.
    for i in 0..k.rom.len() {
        if k.rom[i].size == 0 {
            let next = k.rom.get(i + 1).map_or(prg_len as u32, |n| n.prg);
            k.rom[i].size = next.saturating_sub(k.rom[i].prg).max(1);
        }
    }
    notes
}

/// The code labels of an ld65 debug file (`sym` lines of type `lab` in
/// PRG ROM), for a board that maps all its PRG at once: CPU $8000 is PRG
/// offset 0 for 32KB, and 16KB is mirrored at $C000.
pub(crate) fn listing_symbols(text: &str, prg_len: usize) -> Vec<RomLabel> {
    let mut out = Vec::new();
    for line in text.lines() {
        let Some(rest) = line.strip_prefix("sym\t") else {
            continue;
        };
        let mut name = None;
        let mut val = None;
        let mut lab = false;
        for kv in rest.split(',') {
            match kv.split_once('=') {
                Some(("name", v)) => name = Some(v.trim_matches('"').to_owned()),
                Some(("val", v)) => val = u32::from_str_radix(v.trim_start_matches("0x"), 16).ok(),
                Some(("type", v)) => lab = v == "lab",
                _ => {}
            }
        }
        if let (Some(name), Some(v), true) = (name, val, lab)
            && (0x8000..=0xFFFF).contains(&v)
        {
            let prg = (v - 0x8000) % prg_len.max(1) as u32;
            out.push(RomLabel {
                name,
                prg,
                addr: v as u16,
                size: 0,
            });
        }
    }
    out
}

/// The named constants of an ld65 debug file (`sym` lines of type `equ`),
/// each with the symbols its source references within three lines after
/// each use of it, found through the file's `line` table.
pub(crate) fn listing_constants(text: &str) -> Vec<gp_gaps::Constant> {
    let mut lines: std::collections::HashMap<u32, u32> = Default::default();
    let mut syms: Vec<(String, Option<i64>, bool, Vec<u32>)> = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("line\t") {
            let (mut id, mut at) = (None, None);
            for kv in rest.split(',') {
                match kv.split_once('=') {
                    Some(("id", v)) => id = v.parse().ok(),
                    Some(("line", v)) => at = v.parse().ok(),
                    _ => {}
                }
            }
            if let (Some(id), Some(at)) = (id, at) {
                lines.insert(id, at);
            }
            continue;
        }
        let Some(rest) = line.strip_prefix("sym\t") else {
            continue;
        };
        let (mut name, mut val, mut equ, mut refs) = (None, None, false, Vec::new());
        for kv in rest.split(',') {
            match kv.split_once('=') {
                Some(("name", v)) => name = Some(v.trim_matches('"').to_owned()),
                Some(("val", v)) => val = i64::from_str_radix(v.trim_start_matches("0x"), 16).ok(),
                Some(("type", v)) => equ = v == "equ",
                Some(("ref", v)) => refs = v.split('+').filter_map(|r| r.parse().ok()).collect(),
                _ => {}
            }
        }
        if let Some(n) = name {
            syms.push((n, val, equ, refs));
        }
    }
    let mut by_line: BTreeMap<u32, Vec<&str>> = BTreeMap::new();
    for (n, _, _, refs) in &syms {
        for r in refs {
            if let Some(&l) = lines.get(r) {
                by_line.entry(l).or_default().push(n);
            }
        }
    }
    syms.iter()
        .filter_map(|(n, v, equ, refs)| {
            let v = (*equ).then_some((*v)?)?;
            let mut near = BTreeSet::new();
            if gp_gaps::sound::sound_name(n) {
                for r in refs {
                    let Some(&l) = lines.get(r) else { continue };
                    for (_, names) in by_line.range(l..=l + 3) {
                        near.extend(names.iter().filter(|m| *m != n).map(|m| (*m).to_owned()));
                    }
                }
            }
            Some(gp_gaps::Constant {
                name: n.clone(),
                value: v,
                near: near.into_iter().collect(),
            })
        })
        .collect()
}

/// The named constants of an assembler listing that keeps each source line
/// after a fixed-width column of address and bytes (asm6f's `.lst`): every
/// `Name = value` line, each with the names its source uses within three
/// lines after each use of it (`lda #Sfx_Bump` / `sta SoundQueue`).
pub(crate) fn asm_listing_constants(text: &str) -> Vec<gp_gaps::Constant> {
    let source: Vec<&str> = text
        .lines()
        .map(|l| l.get(32..).unwrap_or("").trim())
        .collect();
    let ident = |s: &str| -> Vec<String> {
        let mut out = BTreeSet::new();
        // Leave the comment out.
        let code = s.split(';').next().unwrap_or("");
        tokens(code, &mut out);
        out.into_iter().collect()
    };
    let mut defs: Vec<(String, i64)> = Vec::new();
    for s in &source {
        let Some((name, value)) = s.split_once('=') else {
            continue;
        };
        let name = name.trim();
        if name.is_empty()
            || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            || !name
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        {
            continue;
        }
        let v = value.split(';').next().unwrap_or("").trim();
        let parsed = if let Some(h) = v.strip_prefix('$') {
            i64::from_str_radix(h, 16).ok()
        } else if let Some(b) = v.strip_prefix('%') {
            i64::from_str_radix(b, 2).ok()
        } else {
            v.parse().ok()
        };
        if let Some(n) = parsed {
            defs.push((name.to_owned(), n));
        }
    }
    let wanted: BTreeSet<String> = defs
        .iter()
        .filter(|(n, _)| gp_gaps::sound::sound_name(n))
        .map(|(n, _)| n.clone())
        .collect();
    let mut near: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (i, s) in source.iter().enumerate() {
        if s.contains('=') {
            continue;
        }
        for id in ident(s) {
            if wanted.contains(&id) {
                let e = near.entry(id.clone()).or_default();
                for t in source.iter().skip(i).take(4) {
                    e.extend(ident(t).into_iter().filter(|x| *x != id));
                }
            }
        }
    }
    defs.into_iter()
        .map(|(name, value)| gp_gaps::Constant {
            near: near
                .get(&name)
                .map(|s| s.iter().cloned().collect())
                .unwrap_or_default(),
            name,
            value,
        })
        .collect()
}

/// The vram address's physical cell under `mirroring`.
pub(crate) fn vram_cell(addr: u16, mirroring: u8, tables: usize) -> Option<(u8, u8, u8)> {
    if !(0x2000..0x3F00).contains(&addr) {
        return None;
    }
    let a = (addr - 0x2000) & 0xFFF;
    let logical = (a >> 10) as u8;
    let within = a & 0x3FF;
    if within >= 0x3C0 {
        return None;
    }
    let phys = match mirroring {
        0 => logical & 1,
        1 => logical >> 1,
        2 => 0,
        3 => 1,
        _ => logical,
    };
    if usize::from(phys) >= tables {
        return None;
    }
    let (row, col) = (within / 32, within % 32);
    Some((phys, (col / 2) as u8, (row / 2) as u8))
}

pub(crate) fn build(args: &GapsArgs, t: &Template, paths: &Paths) -> Result<Built> {
    let opened = Opened::open(paths, &args.recording)?;
    crate::track::nes_recording(&opened, "gp gaps")?;
    let manifest = &opened.manifest;
    if manifest.trace.is_none() {
        bail!(
            "{} has no frame trace; gp gaps reads states from it",
            opened.short()
        );
    }
    let range = parse_frames(&args.frames, manifest.frames)?;
    let range = range.start.max(1)
        ..range
            .end
            .min(manifest.frames.saturating_sub(args.oam_lag) + 1);
    if range.is_empty() {
        bail!("--frames {} leaves no frame", args.frames);
    }
    let mut run = Run {
        known: known(t),
        ..Run::default()
    };

    // The ROM, for the code observer and the static cross-check.
    let mut no_code_why = String::new();
    let rom = if args.no_code {
        no_code_why = "--no-code".into();
        None
    } else {
        match opened.rom(None, paths) {
            Ok(r) => Some(r),
            Err(e) => {
                no_code_why = format!("{e:#}");
                None
            }
        }
    };
    let prg_len = rom
        .as_ref()
        .map_or(0, |r| r.image.header.prg_rom_bytes as usize);
    if let Some(r) = &rom {
        let start = 16 + r.image.trainer_bytes as usize;
        run.prg = r.bytes[start..start + prg_len].to_vec();
    }
    let mut known = std::mem::take(&mut run.known);
    let notes = labels(args, t, paths, prg_len, &mut known, &mut run.constants);
    for n in notes {
        eprintln!("gp: {n}");
    }
    run.known = known;

    let activity = match &rom {
        Some(r) => {
            let mut marks = gp_coverage::Bits::new(prg_len);
            for l in &run.known.rom {
                if (l.prg as usize) < prg_len {
                    marks.set(l.prg as usize);
                }
            }
            let named: BTreeSet<u32> = run
                .known
                .rom
                .iter()
                .filter(|l| gp_gaps::sound::sound_name(&l.name))
                .map(|l| l.prg)
                .collect();
            let record = |watch: &BTreeSet<u32>, calls_only: bool| {
                gp_coverage::activity::record(
                    &opened.store,
                    manifest,
                    &r.bytes,
                    &marks,
                    watch,
                    calls_only,
                    args.threads.max(1),
                )
            };
            match record(&named, false).and_then(|mut a| {
                run.calls = a
                    .calls
                    .iter()
                    .map(|(t, st)| gp_gaps::CallStat {
                        target: *t,
                        calls: st.calls,
                        callers: st.callers.len(),
                        distinct_a: st.distinct_a(),
                        reaches_apu: st.reaches_apu,
                        engine: st.is_engine(),
                    })
                    .collect();
                run.watched = gp_gaps::sound::candidates(&run);
                // Play-sound candidates found by their calls need a second
                // pass to list each call.
                let extra: BTreeSet<u32> = run
                    .watched
                    .iter()
                    .copied()
                    .filter(|t| !named.contains(t))
                    .collect();
                if !extra.is_empty() {
                    let b = record(&extra, true)?;
                    for (fa, fb) in a.frames.iter_mut().zip(b.frames) {
                        fa.calls.extend(fb.calls);
                    }
                }
                Ok(a)
            }) {
                Ok(a) => {
                    run.executed = (0..prg_len.div_ceil(64))
                        .map(|w| {
                            (0..64).fold(0u64, |acc, b| {
                                let o = w * 64 + b;
                                acc | (u64::from(o < prg_len && a.starts.contains(o)) << b)
                            })
                        })
                        .collect();
                    Some(a)
                }
                Err(e) => {
                    no_code_why = format!("the replay failed: {e}");
                    None
                }
            }
        }
        None => None,
    };

    // Names the facts use.
    let m = &t.mapping;
    run.kinds = t.model.entities.keys().cloned().collect();
    let kind_of = |name: &str| run.kinds.iter().position(|k| k == name).map(|i| i as u16);
    let player_name = m
        .fields
        .iter()
        .find_map(|f| {
            let rest = f.path.strip_prefix("entities.")?;
            let (name, field) = rest.split_once("[0].")?;
            (field == "x").then(|| name.to_owned())
        })
        .unwrap_or_default();
    let field = |path: &str| m.fields.iter().find(|f| f.path == path);
    let px = field(&format!("entities.{player_name}[0].x"));
    let py = field(&format!("entities.{player_name}[0].y"));
    let cam = m
        .fields
        .iter()
        .find(|f| f.path.starts_with("cameras.") && f.path.ends_with(".x"));
    let cam_y = cam.and_then(|c| field(&c.path.replace(".x", ".y")));
    let counters: Vec<&Field> = m
        .fields
        .iter()
        .filter(|f| f.path.starts_with("counters."))
        .collect();
    run.counters = counters
        .iter()
        .map(|f| f.path.trim_start_matches("counters.").to_owned())
        .collect();
    // Each counter's display scale: a mapping that reads a counter's
    // displayed digits, with no scale of its own, where the model's HUD
    // shows the counter divided by `divide` beside the layer's fixed
    // digits (a score whose ones are always 0), is in units of `divide`;
    // the counters are reported in the units the player sees.
    let hud_divide = |name: &str| {
        t.model
            .present
            .as_ref()
            .and_then(|p| p.hud.as_ref())
            .and_then(|h| h.field.iter().find(|f| f.counter.as_deref() == Some(name)))
            .and_then(|f| f.divide)
            .filter(|d| *d > 1)
    };
    let counter_scale: Vec<i64> = counters
        .iter()
        .map(|f| {
            let name = f.path.trim_start_matches("counters.");
            match hud_divide(name) {
                Some(d)
                    if f.digits
                        && f.scale.is_none()
                        && f.ram.len() == hud_digits(&t.model, name) =>
                {
                    i64::from(d)
                }
                _ => 1,
            }
        })
        .collect();
    run.counter_scale = counter_scale.clone();
    let start = m
        .start
        .as_deref()
        .map(Condition::parse)
        .transpose()
        .map_err(anyhow::Error::msg)?;
    let moving = m
        .slots
        .as_ref()
        .and_then(|s| s.moving.as_deref())
        .map(Condition::parse)
        .transpose()
        .map_err(anyhow::Error::msg)?;
    let player_kind = kind_of(&player_name);
    let slot_kinds: BTreeMap<u8, Option<u16>> = m
        .slots
        .iter()
        .flat_map(|s| s.kinds.iter())
        .flat_map(|k| k.ids.iter().map(move |id| (*id, k.entity.clone())))
        .map(|(id, e)| (id, kind_of(&e)))
        .collect();

    // Events: APU writes, and the sprite DMA page.
    let mut apu: BTreeMap<u64, Vec<(u16, u8)>> = BTreeMap::new();
    let mut dma_pages: BTreeMap<u8, u64> = BTreeMap::new();
    if let Some(e) = &manifest.events {
        let first = (range.start / u64::from(e.chunk_frames)) as usize;
        let last = (range.end / u64::from(e.chunk_frames)) as usize;
        for i in first..=last.min(e.chunks.len().saturating_sub(1)) {
            let bytes = opened.store.get(&e.chunks[i])?;
            let chunk = gp_evidence::events::decode_chunk_covering(
                &bytes,
                gp_evidence::chunk::chunk_range(i, e.chunk_frames, manifest.frames),
            )?;
            for ev in chunk.events {
                match ev.kind {
                    gp_evidence::events::EventKind::ApuWrite => {
                        apu.entry(ev.frame).or_default().push((ev.addr, ev.value));
                    }
                    gp_evidence::events::EventKind::PpuWrite if ev.addr == 0x4014 => {
                        *dma_pages.entry(ev.value).or_default() += 1;
                    }
                    _ => {}
                }
            }
        }
    }
    if let Some((&page, _)) = dma_pages.iter().max_by_key(|(_, n)| **n) {
        run.known.oam_page = page;
    }

    let log = gp_evidence::InputLog::parse(
        &String::from_utf8(opened.store.get(&manifest.input_log)?)
            .context("the recording's input log isn't text")?,
    )?;
    let table = gp_nes::recording::field_table(&opened.store, manifest)?;
    let lag = args.oam_lag;
    let mut held: std::collections::VecDeque<(u64, Vec<u8>, Vec<u8>, u8)> = Default::default();
    let mut prev_ram: Option<Vec<u8>> = None;
    let mut prev_nt: Option<Vec<u8>> = None;
    let mut failed = None;
    let slots = m.slots.clone();
    let end = (range.end - 1 + lag).min(manifest.frames);
    let ok = gp_nes::recording::scan_trace(
        &opened.store,
        manifest,
        (range.start - 1)..=end,
        |frame, state| {
            let fields = match table.split(state) {
                Ok(f) => f,
                Err(e) => {
                    failed = Some(e.to_string());
                    return false;
                }
            };
            let get = |name: &str| fields.iter().find(|(n, _)| *n == name).map(|(_, b)| *b);
            let (Some(ram), Some(_)) = (get("ram"), get("oam")) else {
                failed = Some("the trace has no `ram` and `oam` fields".into());
                return false;
            };
            let nt = get("nametables").unwrap_or(&[]);
            let mirroring = get("mapper.mirroring").map_or(1, |b| b[0]);
            if frame == range.start - 1 {
                prev_ram = Some(ram.to_vec());
                prev_nt = Some(nt.to_vec());
                return true;
            }
            held.push_back((frame, ram.to_vec(), nt.to_vec(), mirroring));
            if frame < range.start + lag {
                return true;
            }
            let (f, ram, nt, mirroring) = held.pop_front().expect("pushed above");
            if f >= range.end {
                return false;
            }
            // The sprites and scroll drawn from frame f's RAM.
            let oam = get("oam").unwrap_or(&[]);
            let t_reg = get("ppu.t").map_or(0, |b| u16::from_le_bytes([b[0], b[1]]));
            let fine = get("ppu.x").map_or(0, |b| u16::from(b[0] & 7));
            let scroll_x = (t_reg & 31) * 8 + fine + ((t_reg >> 10) & 1) * 256;
            let scroll_y =
                ((t_reg >> 5) & 31) * 8 + ((t_reg >> 12) & 7) + ((t_reg >> 11) & 1) * 240;
            let tall = get("ppu.ctrl").is_some_and(|b| b[0] & 0x20 != 0);
            let input = &log.frames[(f - 1) as usize];
            // In play: the player has control (the mapping's start
            // condition) or the game moves its objects (`slots.moving`).
            let play = match (&start, &moving) {
                (None, None) => input.polled,
                (a, b) => {
                    a.as_ref().is_some_and(|c| eval(c, &ram))
                        || b.as_ref().is_some_and(|c| eval(c, &ram))
                }
            };
            let pr = prev_ram.replace(ram.clone()).unwrap_or_else(|| ram.clone());
            let ram_changed: Vec<u16> = (0..ram.len().min(0x800))
                .filter(|&i| ram[i] != pr[i])
                .map(|i| i as u16)
                .collect();
            let camera = [
                cam.and_then(|c| read(c, &ram, 0)).unwrap_or(0) as i32,
                cam_y.and_then(|c| read(c, &ram, 0)).unwrap_or(0) as i32,
            ];
            let mut ents = Vec::new();
            if let (Some(x), Some(y)) = (px, py)
                && let (Some(x), Some(y)) = (read(x, &ram, 0), read(y, &ram, 0))
            {
                ents.push(Ent {
                    kind: player_kind,
                    player: true,
                    slot: None,
                    id: None,
                    x: x as i32,
                    y: y as i32,
                });
            }
            if let Some(s) = &slots {
                let fx = s.fields.iter().find(|f| f.path == "x");
                let fy = s.fields.iter().find(|f| f.path == "y");
                for i in 0..s.count {
                    let active = s.flag_of(i, |a| Ok(ram[usize::from(a)])).unwrap_or(0);
                    if active == 0 {
                        continue;
                    }
                    if let (Some(st), Some(ends)) = (s.state, s.ends)
                        && ram[usize::from(st + i)] & ends != 0
                    {
                        continue;
                    }
                    let id = s.id.map_or(0, |at| ram[usize::from(at + i)]);
                    let (Some(fx), Some(fy)) = (fx, fy) else {
                        continue;
                    };
                    let (Some(x), Some(y)) = (read(fx, &ram, i), read(fy, &ram, i)) else {
                        continue;
                    };
                    ents.push(Ent {
                        kind: slot_kinds.get(&id).copied().flatten(),
                        player: false,
                        slot: Some(i as u8),
                        id: Some(id),
                        x: x as i32,
                        y: y as i32,
                    });
                }
            }
            let counters = counters
                .iter()
                .zip(&counter_scale)
                .map(|(c, k)| read(c, &ram, 0).unwrap_or(0) * k)
                .collect();
            let clusters = gp_track::sprites::clusters(oam, tall);
            let h: i16 = if tall { 16 } else { 8 };
            let objects = clusters
                .into_iter()
                .map(|c| {
                    let sprites = (0..64u8)
                        .filter(|&i| {
                            let s = &oam[usize::from(i) * 4..usize::from(i) * 4 + 4];
                            let (x, y) = (i16::from(s[3]), i16::from(s[0]) + 1);
                            s[0] < 0xEF
                                && s[2] & 3 == c.palette
                                && x >= c.x0
                                && x + 8 <= c.x1
                                && y >= c.y0
                                && y + h <= c.y1
                        })
                        .collect();
                    Object { c, sprites }
                })
                .collect();
            let code = activity.as_ref().and_then(|a| a.frames.get(f as usize));
            let mut cells = Vec::new();
            let pn = prev_nt.replace(nt.clone()).unwrap_or_else(|| nt.clone());
            let tables = nt.len() / 1024;
            if pn.len() == nt.len() {
                let mut writers: BTreeMap<(u8, u8, u8), BTreeSet<u32>> = BTreeMap::new();
                if let Some(c) = code {
                    for w in &c.vram {
                        if let Some(cell) = vram_cell(w.addr, mirroring, tables) {
                            writers.entry(cell).or_default().insert(w.routine);
                        }
                    }
                }
                for p in 0..tables {
                    let base = p * 1024;
                    for cy in 0..15usize {
                        for cx in 0..16usize {
                            let at = |r: usize, c: usize| base + (cy * 2 + r) * 32 + cx * 2 + c;
                            let idx = [at(0, 0), at(0, 1), at(1, 0), at(1, 1)];
                            if idx.iter().all(|&i| nt[i] == pn[i]) {
                                continue;
                            }
                            let key = (p as u8, cx as u8, cy as u8);
                            cells.push(CellChange {
                                nt: p as u8,
                                cx: cx as u8,
                                cy: cy as u8,
                                before: idx.map(|i| pn[i]),
                                after: idx.map(|i| nt[i]),
                                writers: writers
                                    .remove(&key)
                                    .unwrap_or_default()
                                    .into_iter()
                                    .collect(),
                            });
                        }
                    }
                }
            }
            let code = code.map(|c| {
                let mut apu_r: Vec<u32> = c.apu.iter().map(|w| w.routine).collect();
                apu_r.sort_unstable();
                apu_r.dedup();
                Code {
                    entered: c.entered.clone(),
                    ram: c
                        .ram
                        .iter()
                        .map(|w| RamWrite {
                            addr: w.addr,
                            value: w.value,
                            engine: w.engine,
                            pc: w.pc,
                            routine: w.routine,
                        })
                        .collect(),
                    apu: apu_r,
                    apu_pc: {
                        let mut v: Vec<u32> = c.apu.iter().map(|w| w.pc).collect();
                        v.sort_unstable();
                        v.dedup();
                        v
                    },
                    calls: c.calls.clone(),
                }
            });
            run.frames.push(FrameFacts {
                frame: f,
                play,
                pad: (input.buttons_word() & 0xFF) as u8,
                camera,
                scroll: [scroll_x, scroll_y],
                mirroring,
                ram_changed,
                counters,
                ents,
                objects,
                cells,
                apu: apu.remove(&f).unwrap_or_default(),
                code,
            });
            true
        },
    )
    .map_err(|e| anyhow::anyhow!("{e}"))?;
    if let Some(e) = failed {
        bail!("{e}");
    }
    if !ok {
        bail!("the recording has no frame trace");
    }
    let player_ram: Vec<u16> = [px, py]
        .into_iter()
        .flatten()
        .flat_map(|f| f.ram.iter().chain(&f.frac).copied())
        .collect();
    // How many frames the mapping calls play that are a screen with the
    // player still or not moved by its code (the old tool logged it at
    // info, which it didn't print).
    let _still = run.refine_play(&player_ram);
    Ok(Built {
        run,
        range,
        short: opened.short(),
        id: opened.entry.id.to_hex(),
        no_code_why,
    })
}

/// How many digits the model's HUD shows a counter with (0 for none).
fn hud_digits(m: &gp_model::Model, name: &str) -> usize {
    m.present
        .as_ref()
        .and_then(|p| p.hud.as_ref())
        .and_then(|h| h.field.iter().find(|f| f.counter.as_deref() == Some(name)))
        .and_then(|f| f.digits)
        .map_or(0, |d| d as usize)
}

/// How the model draws each entity kind (by `kinds`) in the state its
/// lives start in: its body's size, its base picture's and, for a frame laid out from the position, where the
/// picture's top left is from it, from the presentation the generator
/// compiles.
fn model_boxes(
    args: &GapsArgs,
    t: &Template,
    kinds: &[String],
    paths: &Paths,
) -> Result<Vec<Option<gp_gaps::drawn::ModelBox>>> {
    let dir = match &args.assets {
        Some(a) => a.clone(),
        None => paths.home.join("work/assets").join(&t.source),
    };
    // Only the compiled spec and the presentation are read, with no hook
    // run, so a model with hooks needs no module here.
    let assets = gp_interp::Lifted::read_dir(&dir, &t.model).map_err(|e| {
        anyhow::anyhow!(
            "{e}\n(lift the model's assets with gp assets lift, or name them with --assets)"
        )
    })?;
    let spec = gp_interp::compile::compile(&t.model, &assets).map_err(anyhow::Error::msg)?;
    let art = gp_look::present::art(&t.model, &spec, &dir).map_err(anyhow::Error::msg)?;
    let p = &art.present;
    Ok(kinds
        .iter()
        .map(|name| {
            let k = spec.kinds.iter().position(|s| &s.name == name)?;
            // The look of the state its lives start in, else the first.
            let looks = p.kinds.get(k)?;
            let first = spec.kinds[k].machine.as_ref().map_or(0, |m| m.initial);
            let look = looks
                .get(first)
                .and_then(Option::as_ref)
                .or_else(|| looks.iter().flatten().next())?;
            let pic = *p.frames.get(look.base.id as usize)?.first()?;
            let size = p.pictures.get(pic as usize)?.size;
            let body = spec.kinds[k].size;
            Some(gp_gaps::drawn::ModelBox {
                body: [body[0].floor() as i32, body[1].floor() as i32],
                picture: [size[0] as i32, size[1] as i32],
                origin: p.origins.get(look.base.id as usize).copied().flatten(),
            })
        })
        .collect())
}

fn list<T: std::fmt::Debug>(v: &[T]) -> String {
    v.iter()
        .map(|x| format!("{x:?}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn pairs(v: &[(String, u64)]) -> String {
    v.iter()
        .map(|(n, c)| format!("{n} {c}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn changes_text(t: &mut String, r: &gp_gaps::changes::Report, n: usize) {
    let _ = writeln!(
        t,
        "\nchanges: ram {}, sprites {}, background {}, sound {}, code {}",
        r.ram.len(),
        r.oam.len(),
        r.background.len(),
        r.counts()[3].1,
        r.code.len()
    );
    let _ = writeln!(t, "RAM (unexplained bytes, by play changes):");
    for i in r.ram.iter().take(n) {
        let _ = writeln!(
            t,
            "  {:<28} {} changes in play ({} all, lift {:.2}){} writers: {} samples {}",
            i.name,
            i.play_changes,
            i.changes,
            i.input_lift,
            if i.labelled { "" } else { " unlabelled" },
            pairs(&i.writers),
            list(&i.samples)
        );
    }
    let _ = writeln!(t, "sprites (objects no model entity is near):");
    for i in r.oam.iter().take(n) {
        let _ = writeln!(
            t,
            "  {:<28} {} tracks, {} frames, {}x{}, near a cell change {:.2}, slot ids {:?}, writers: {} samples {}",
            i.name,
            i.tracks,
            i.frames,
            i.size[0],
            i.size[1],
            i.near_cell_change,
            i.slot_ids,
            pairs(&i.writers),
            list(&i.samples)
        );
    }
    let _ = writeln!(
        t,
        "background (cells changed in play; the model's tile-changing mechanics: {}):",
        if r.model_tile_mechanics.is_empty() {
            "none".to_owned()
        } else {
            r.model_tile_mechanics.join(", ")
        }
    );
    for i in r.background.iter().take(n) {
        let _ = writeln!(
            t,
            "  {} -> {}  {} times at {} cells, writers: {} samples {}",
            i.before,
            i.after,
            i.count,
            i.cells,
            pairs(&i.writers),
            list(&i.samples)
        );
    }
    let _ = writeln!(
        t,
        "sound requests (effects the model has no counterpart for; the model has {}):",
        if r.model_sound { "sound" } else { "none" }
    );
    if r.sound_interfaces.is_empty() {
        let _ = writeln!(t, "  no sound-request interface found");
    }
    for i in &r.sound_interfaces {
        let _ = writeln!(
            t,
            "  via {} {} ({}): {} requests; {}",
            i.kind,
            i.name,
            i.addr.map_or_else(String::new, |a| format!("${a:04X}")),
            i.requests,
            i.why
        );
    }
    for i in r.sound.iter().take(n) {
        let counters: Vec<String> = i
            .counters
            .iter()
            .map(|(c, d, sh)| format!("{c} {d:+} {sh:.2}"))
            .collect();
        let spawns: Vec<String> = i
            .spawns
            .iter()
            .map(|(x, sh)| format!("{x} {sh:.2}"))
            .collect();
        let _ = writeln!(
            t,
            "  {:<28} x{} ({} in play) via {} id {:#04x}; requested by {}; counters [{}] spawns [{}] cell change {:.2}; suggests {}; samples {}",
            i.name,
            i.count,
            i.play_count,
            i.via,
            i.id,
            pairs(&i.requesters),
            counters.join(", "),
            spawns.join(", "),
            i.cell_change,
            if i.suggests.is_empty() {
                "-"
            } else {
                &i.suggests
            },
            list(&i.samples)
        );
    }
    let _ = writeln!(t, "APU by channel:");
    for i in r.apu.iter().take(n) {
        let with: Vec<String> = i
            .with
            .iter()
            .map(|(e, l, k)| format!("{e} x{l} ({k})"))
            .collect();
        let _ = writeln!(
            t,
            "  {:<8} {} writes, {} notes ({} in play), with: {}; writers: {}",
            i.channel,
            i.writes,
            i.triggers,
            i.play_triggers,
            with.join(", "),
            pairs(&i.writers)
        );
    }
    let _ = writeln!(
        t,
        "code (routines run in play that the model doesn't cite):"
    );
    for i in r.code.iter().take(n) {
        let _ = writeln!(
            t,
            "  {:<28} {} frames, {} unexplained changes, samples {}",
            i.name,
            i.frames,
            i.caused,
            list(&i.samples)
        );
    }
}

fn interactive_text(t: &mut String, c: &gp_gaps::interactive::Catalog, n: usize) {
    let _ = writeln!(
        t,
        "\ninteractive: {} interactions in {} kinds ({} more kinds of scenery nothing touched); {} handler routines",
        c.interactions,
        c.kinds.len(),
        c.scenery.len(),
        c.handlers.len()
    );
    for k in c.kinds.iter().take(n) {
        let counters: Vec<String> = k
            .effects
            .counters
            .iter()
            .map(|(c, (d, m))| format!("{c} {d:+} ({m})"))
            .collect();
        let _ = writeln!(
            t,
            "  {} x{}: trigger {} {:.2}, v {:?}, routine {} {:.2}; effects counters [{}] spawns [{}] sound [{}] requests [{}]; result {} (reverted {}, after {} frames); limits {} cells, max {} hits, interval {}; evidence {}{}",
            k.kind,
            k.count,
            k.trigger.relation,
            k.trigger.share,
            k.trigger.player_v,
            k.trigger.routine.as_deref().unwrap_or("-"),
            k.trigger.routine_share,
            counters.join(", "),
            pairs(&k.effects.spawned),
            pairs(&k.effects.sound),
            pairs(&k.effects.requests),
            k.result.after.as_deref().unwrap_or("-"),
            k.result.reverted,
            k.result.revert_frames,
            k.limits.cells,
            k.limits.max_hits,
            k.limits.interval,
            list(&k.evidence),
            match &k.code {
                Some(s) => format!(
                    "; code: {} targets ran, unexercised [{}], compares {:02x?}",
                    s.exercised,
                    s.unexercised.join(", "),
                    s.compares
                ),
                None => String::new(),
            }
        );
    }
    let hs: Vec<String> = c
        .handlers
        .iter()
        .take(n)
        .map(|(r, f, w)| format!("{r} {w}/{f}"))
        .collect();
    let _ = writeln!(
        t,
        "  handlers (with a cell change/fired): {}",
        hs.join(", ")
    );
}

fn score_text(t: &mut String, s: &gp_gaps::score::Score) {
    let _ = writeln!(
        t,
        "\ndetection quality (precision/recall against the reference set):"
    );
    for d in &s.detectors {
        let _ = writeln!(
            t,
            "  {:<12} {:.2}/{:.2}  ({} found right, {} not in the set, {} exercised){}{}",
            d.detector,
            d.precision,
            d.recall,
            d.true_positives,
            d.false_positives,
            d.exercised,
            if d.false_found.is_empty() {
                String::new()
            } else {
                format!("; not in the set: {}", d.false_found.join(", "))
            },
            if d.missed.is_empty() {
                String::new()
            } else {
                format!("; missed: {}", d.missed.join(", "))
            },
        );
    }
    let _ = writeln!(
        t,
        "  effects named {:.2}, classed {:.2}; RAM bookkeeping in the top {}: {}",
        s.effects_named,
        s.effects_classed,
        gp_gaps::score::RAM_TOP,
        s.ram_bookkeeping
    );
    if !s.unknown.is_empty() {
        let _ = writeln!(t, "  names the labels don't have: {}", s.unknown.join(", "));
    }
}

fn drawn_text(t: &mut String, r: &gp_gaps::drawn::Report, n: usize) {
    let _ = writeln!(
        t,
        "\ndrawn: {} kinds checked, {} with a systematic difference",
        r.kinds.len(),
        r.off()
    );
    for k in r.kinds.iter().take(n) {
        let _ = writeln!(
            t,
            "  {:<24} {}/{} frames matched; median dx {:+} dy {:+} dw {:+} dh {:+} (held on {:.2}/{:.2}/{:.2}/{:.2}); mean dy {:+.2} sd {:.2}{} samples {}",
            k.kind,
            k.matched,
            k.frames,
            k.median[0],
            k.median[1],
            k.median[2],
            k.median[3],
            k.at_median[0],
            k.at_median[1],
            k.at_median[2],
            k.at_median[3],
            k.dy[0],
            k.dy[1],
            if k.systematic.is_empty() {
                String::new()
            } else {
                format!(" SYSTEMATIC {}", k.systematic.join(" "))
            },
            list(&k.samples)
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listing_symbols_are_code_labels_in_prg() {
        let text = "sym\tid=0,name=\"Start\",addrsize=absolute,scope=0,def=1,val=0x8000,seg=6,type=lab\n\
                    sym\tid=1,name=\"Table\",addrsize=absolute,scope=0,def=2,val=0x8123,seg=6,type=lab\n\
                    sym\tid=2,name=\"Ram\",addrsize=zeropage,scope=0,def=3,val=0x10,type=lab\n\
                    sym\tid=3,name=\"K\",addrsize=absolute,scope=0,def=4,val=0x9000,type=equ\n";
        let l = listing_symbols(text, 0x8000);
        let names: Vec<(&str, u32)> = l.iter().map(|x| (x.name.as_str(), x.prg)).collect();
        assert_eq!(names, vec![("Start", 0), ("Table", 0x123)]);
    }

    #[test]
    fn listing_constants_know_the_names_used_after_them() {
        // Source lines 10 and 11: `lda #Sfx_Ping` then `sta SoundQueue`.
        let text = "line\tid=0,file=0,line=10\n\
                    line\tid=1,file=0,line=11\n\
                    line\tid=2,file=0,line=40\n\
                    sym\tid=0,name=\"Sfx_Ping\",addrsize=zeropage,val=0x4,ref=0,type=equ\n\
                    sym\tid=1,name=\"SoundQueue\",addrsize=zeropage,val=0xF0,ref=1+2,type=equ\n\
                    sym\tid=2,name=\"Start\",addrsize=absolute,val=0x8000,type=lab\n";
        let c = listing_constants(text);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].name, "Sfx_Ping");
        assert_eq!(c[0].value, 4);
        assert_eq!(c[0].near, vec!["SoundQueue".to_owned()]);
        // Only sound-named constants look for their neighbors.
        assert!(c[1].near.is_empty());
    }

    #[test]
    fn asm_listing_constants_read_the_source_column() {
        // A made-up listing: 32 columns of address and bytes, then source.
        let pad = |prefix: &str, src: &str| format!("{prefix:<32}{src}\n");
        let text = [
            pad("00000", "Sfx_Ping = $04"),
            pad("00000", "Music_Theme = %00000010"),
            pad("00000", "Speed = 12 ; not a sound"),
            pad("08000 A9 04", "\tLDA #Sfx_Ping"),
            pad("08002 8D 02 06", "\tSTA SoundQueue ; ask"),
        ]
        .concat();
        let c = asm_listing_constants(&text);
        let get = |n: &str| c.iter().find(|x| x.name == n).unwrap();
        assert_eq!(get("Sfx_Ping").value, 4);
        assert_eq!(get("Music_Theme").value, 2);
        assert_eq!(get("Speed").value, 12);
        assert!(get("Sfx_Ping").near.contains(&"SoundQueue".to_owned()));
        assert!(get("Speed").near.is_empty());
    }

    #[test]
    fn vram_addresses_fold_by_mirroring() {
        // $2400 is the second logical table: physical 1 with vertical
        // mirroring (0), physical 0 with horizontal (1).
        assert_eq!(vram_cell(0x2400 + 32 * 2 + 4, 0, 2), Some((1, 2, 1)));
        assert_eq!(vram_cell(0x2400, 1, 2), Some((0, 0, 0)));
        assert_eq!(vram_cell(0x2800, 1, 2), Some((1, 0, 0)));
        // Attribute bytes aren't cells.
        assert_eq!(vram_cell(0x23C0, 1, 2), None);
    }

    #[test]
    fn identifiers_are_tokens() {
        let mut s = BTreeSet::new();
        tokens(
            "quote = \"Enemy_X_Position:\" at = SMBDIS.ASM:12 x1",
            &mut s,
        );
        assert!(s.contains("Enemy_X_Position"));
        assert!(s.contains("SMBDIS"));
        assert!(!s.contains("12"));
    }
}
