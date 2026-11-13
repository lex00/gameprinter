//! `gp mechanic triage`: the tools try each quest log row and each lead
//! first, so the person's queue holds only play the tools can't get.
//!
//! What a row has to show is written in the game's package folder as
//! `triage.toml` (the format is below): a test on RAM that comes to hold
//! when a clip shows it. Each tool tries in turn, with a budget:
//!
//! | Tool | What it does |
//! |---|---|
//! | `corpus` | Replays the corpus plan's clips (the same play once for all clips that share it) and looks for the test inside each clip |
//! | `script` | Plays the row's `try` steps (`gp mechanic probe`'s steps) from its `from` start |
//! | `autoplay` | Runs gp-arcade's autoplay example (`--search`) from power-on once, and from each row's near point (a save point) on, and looks in its logs (arcade only) |
//! | `search` | Seeded presses from the row's near points, as `gp capture dense`'s generator makes them, kept when the test holds |
//! | `demo` | `gp mechanic demo` with Laya (NES only, when Laya answers) |
//!
//! A run that shows the row is cut around the frame the test holds and
//! saved into the row's session as a clip made by the tools (`by =
//! "tools"`, and `tool`), a path and the presses after it like any clip,
//! so `gp clip score` and the gate score it as they score the rest. The
//! row gets a `triage` table on the quest log: `by` is "tools" or "you",
//! `why` says why the tools couldn't get it, `tried` what each tool did,
//! and for "you" a `request`: the start (a path and a frame, from a corpus
//! clip or the tools' own runs), what to do and when to stop, which the
//! page's PLAY FROM HERE loads, recording, at that start.
//!
//! `triage.toml`:
//!
//! ```toml
//! [watch]                  # names for RAM addresses, with probes.toml's
//! lives = 0xE4A7
//!
//! [[item]]
//! row = "die"              # the quest log row (made, as "next", if new)
//! goal = "lives == 2"      # comes to hold (it doesn't at the clip's start)
//! hold = 1                 # frames in a row it must hold
//! near = "mode == 0"       # where the hard stretch begins: the start point
//! slots = { at = 0xE2C2, stride = 4, count = 8 }  # `@K` in a test: byte K of any slot
//! from = "tapper.l4.001@120"  # the script's start (power-on, ID, ID@F, ID@end, SESSION:N@F)
//! try = ["T 30", ". 60"]   # scripted presses
//! where = "..."; do = "..."; stop = "..."   # the person's request
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use gp_evidence::Paths;
use gp_evidence::input_log::{InputFrame, InputLog};
use serde::Deserialize;
use serde_json::{Value, json};

use super::paths::{clip_fields, full_log, put_path};
use super::probe::{self, Board, Cond, Names};
use super::session::{Session, queue_file, read_queue, write_queue};
use crate::Outcome;
use crate::original::{OriginalRuntime, Platform};

/// The tools, in the order they try.
pub(crate) const TOOLS: &[&str] = &["corpus", "script", "autoplay", "search", "demo"];

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Plan {
    #[serde(default)]
    watch: BTreeMap<String, u16>,
    #[serde(default)]
    item: Vec<Item>,
}

#[derive(Deserialize, Clone, Copy, Debug)]
#[serde(deny_unknown_fields)]
struct Slots {
    at: u16,
    stride: u16,
    count: u16,
}

#[derive(Deserialize, Clone, Default)]
#[serde(deny_unknown_fields)]
struct Item {
    row: String,
    /// The row's line when the triage adds it.
    #[serde(default)]
    note: String,
    #[serde(default)]
    goal: String,
    /// RAM bytes any change of which shows it (a lead's).
    #[serde(default)]
    changes: Vec<String>,
    /// Routines (CPU addresses) any of which running shows it (a lead's;
    /// the NES only).
    #[serde(default)]
    routines: Vec<String>,
    #[serde(default = "one")]
    hold: usize,
    #[serde(default)]
    near: Option<String>,
    #[serde(default)]
    slots: Option<Slots>,
    #[serde(default)]
    from: Option<String>,
    #[serde(default, rename = "try")]
    tries: Vec<String>,
    /// The tools to try (default every one).
    #[serde(default)]
    tools: Vec<String>,
    #[serde(default = "before")]
    before: usize,
    #[serde(default = "after")]
    after: usize,
    #[serde(default, rename = "where")]
    place: String,
    #[serde(default, rename = "do")]
    todo: String,
    #[serde(default)]
    stop: String,
}

fn one() -> usize {
    1
}
fn before() -> usize {
    90
}
fn after() -> usize {
    120
}

/// A test on the original: fixed addresses, and `@K` bytes of any slot.
struct Test {
    fixed: Cond,
    slot: Option<(Slots, Cond)>,
}

impl Test {
    fn parse(names: &Names, text: &str, slots: Option<Slots>) -> Result<Self> {
        let words: Vec<&str> = text.split_whitespace().collect();
        let (mut fixed, mut slot) = (Vec::new(), Vec::new());
        for part in words.split(|w| *w == "and") {
            if part.first().is_some_and(|w| w.starts_with('@')) {
                slot.push(part.to_vec());
            } else {
                fixed.push(part.to_vec());
            }
        }
        let join = |ps: &[Vec<&str>]| -> Vec<String> {
            let mut out = Vec::new();
            for (i, p) in ps.iter().enumerate() {
                if i > 0 {
                    out.push("and".to_owned());
                }
                out.extend(p.iter().map(|w| (*w).to_owned()));
            }
            out
        };
        let fixed_words = join(&fixed);
        let fixed = if fixed_words.is_empty() {
            Cond(Vec::new())
        } else {
            probe::cond(
                names,
                &fixed_words.iter().map(String::as_str).collect::<Vec<_>>(),
            )?
        };
        let slot = if slot.is_empty() {
            None
        } else {
            let s = slots.with_context(|| format!("{text}: an @K test needs the item's slots"))?;
            let words: Vec<String> = join(&slot)
                .into_iter()
                .map(|w| match w.strip_prefix('@') {
                    Some(k) => format!("0x{:x}", probe::num(k).unwrap_or(0)),
                    None => w,
                })
                .collect();
            let local = Names {
                watch: BTreeMap::new(),
                space: 0x1_0000,
            };
            Some((
                s,
                probe::cond(
                    &local,
                    &words.iter().map(String::as_str).collect::<Vec<_>>(),
                )?,
            ))
        };
        Ok(Self { fixed, slot })
    }

    fn holds(&self, o: &OriginalRuntime) -> bool {
        self.which(o).is_some()
    }

    /// Whether it holds, and the matching slot's bytes (none without slots).
    fn which(&self, o: &OriginalRuntime) -> Option<Vec<u8>> {
        let byte = |a: u16| probe::peek(o, a).unwrap_or(0);
        if !self.fixed.holds_by(byte) {
            return None;
        }
        match &self.slot {
            None => Some(Vec::new()),
            Some((s, c)) => (0..s.count).find_map(|i| {
                let base = s.at.wrapping_add(i * s.stride);
                c.holds_by(|k| byte(base.wrapping_add(k)))
                    .then(|| (0..s.stride).map(|k| byte(base.wrapping_add(k))).collect())
            }),
        }
    }
}

/// Frames the near test must hold for a near point.
const NEAR_HOLD: usize = 30;

/// What a row is watched for while a run plays.
struct Goal {
    test: Option<Test>,
    changes: Vec<u16>,
    routines: BTreeSet<u16>,
    hold: usize,
    near: Option<Test>,
}

/// One watcher's state over a run.
#[derive(Default, Clone)]
struct Watch {
    was: bool,
    run: usize,
    prev: Vec<u8>,
    /// The first frame the goal came to hold (counted from power-on: the
    /// frame after which it held), inside the window.
    hit: Option<usize>,
    /// The first frame the near test had held [`NEAR_HOLD`] frames.
    near: Option<usize>,
    near_run: usize,
    /// The matching slot's bytes at the hit.
    slot: Vec<u8>,
}

impl Goal {
    /// Looks at the original after frame `f`; `inside` is whether `f` is
    /// in the window a hit may come from.
    fn look(
        &self,
        o: &OriginalRuntime,
        ran: &BTreeSet<u16>,
        w: &mut Watch,
        f: usize,
        inside: bool,
    ) {
        // The near point: where its test has held a second (a test that
        // holds on power-on's first frames, before the game sets RAM up,
        // isn't a place to start from).
        if w.near.is_none() && inside {
            if self.near.as_ref().is_some_and(|n| n.holds(o)) {
                w.near_run += 1;
                if w.near_run >= NEAR_HOLD {
                    w.near = Some(f);
                }
            } else {
                w.near_run = 0;
            }
        }
        if w.hit.is_some() {
            return;
        }
        if let Some(t) = &self.test {
            let slot = t.which(o);
            let now = slot.is_some();
            w.run = if now { w.run + 1 } else { 0 };
            if !now {
                w.was = true;
            }
            // It must come to hold: false first, then true `hold` frames.
            if inside && now && w.was && w.run >= self.hold {
                w.hit = Some(f + 1 - self.hold);
                w.slot = slot.unwrap_or_default();
            }
        } else if !self.routines.is_empty() {
            if inside && ran.iter().any(|pc| self.routines.contains(pc)) {
                w.hit = Some(f);
            }
        } else if !self.changes.is_empty() {
            let now: Vec<u8> = self
                .changes
                .iter()
                .map(|&a| probe::peek(o, a).unwrap_or(0))
                .collect();
            if inside && !w.prev.is_empty() && now != w.prev {
                w.hit = Some(f);
            }
            w.prev = now;
        }
    }
}

/// One frame on the original; on the NES, with `targets`, the addresses
/// among them the CPU ran are put in `ran`.
fn step_seeing(
    o: &mut OriginalRuntime,
    f: &InputFrame,
    targets: &BTreeSet<u16>,
    ran: &mut BTreeSet<u16>,
) -> Result<()> {
    ran.clear();
    if !targets.is_empty()
        && let OriginalRuntime::Nes(n) = o
    {
        let mut map = gp_nes::coverage::CodeMap::new();
        let input = gp_runtime::InputFrame {
            buttons: f.buttons_word(),
            axes: Vec::new(),
            reset: f.reset,
        };
        n.step_each(&input, &mut map, &mut |pc, _, _| {
            if targets.contains(&pc) {
                ran.insert(pc);
            }
        })
        .map_err(anyhow::Error::msg)?;
        return Ok(());
    }
    probe::step(o, f)
}

/// A play of the corpus: frames from power-on, and each clip of it as a
/// window a hit may come from (the clip's own frames).
struct Run {
    frames: Vec<InputFrame>,
    windows: Vec<(String, usize, usize)>,
}

/// Plays `frames` from power-on once, watching every goal in each window.
/// Returns, per goal, per window: its watch.
fn scan(
    rom: &[u8],
    frames: &[InputFrame],
    windows: &[(String, usize, usize)],
    goals: &[&Goal],
    deadline: Instant,
) -> Result<(Vec<Vec<Watch>>, usize)> {
    let mut o = OriginalRuntime::of_rom(rom)?;
    o.set_video(false);
    let mut w = vec![vec![Watch::default(); windows.len()]; goals.len()];
    let last = windows
        .iter()
        .map(|x| x.2)
        .max()
        .unwrap_or(0)
        .min(frames.len());
    let first = windows.iter().map(|x| x.1).min().unwrap_or(0);
    let targets: BTreeSet<u16> = goals
        .iter()
        .flat_map(|g| g.routines.iter().copied())
        .collect();
    let mut ran = BTreeSet::new();
    let mut played = 0;
    for (i, f) in frames[..last].iter().enumerate() {
        // Before the first window, no goal needs the code that ran.
        let none = BTreeSet::new();
        step_seeing(
            &mut o,
            f,
            if i + 2 >= first { &targets } else { &none },
            &mut ran,
        )?;
        let n = i + 1;
        if n < first.saturating_sub(1) {
            continue;
        }
        for (g, goal) in goals.iter().enumerate() {
            for (k, win) in windows.iter().enumerate() {
                if n + 1 < win.1 || n > win.2 {
                    continue;
                }
                goal.look(&o, &ran, &mut w[g][k], n, n > win.1);
            }
        }
        played = n;
        if n.is_multiple_of(500) && Instant::now() > deadline {
            break;
        }
    }
    Ok((w, played))
}

/// What the triage was asked.
pub(crate) struct Args<'a> {
    pub game: &'a str,
    pub games: Option<&'a Path>,
    /// Only these rows (default every row and lead the plan has).
    pub rows: &'a [String],
    /// Seconds each tool may spend on a row.
    pub budget: u64,
    /// How many of the leads to triage (NES only).
    pub leads: usize,
    /// gp-arcade's autoplay example; by default found beside gp's build.
    pub autoplay: Option<&'a Path>,
    /// Frames autoplay plays from power-on.
    pub autoplay_frames: usize,
    pub decide: &'a str,
    /// Write nothing: say what would land where.
    pub dry_run: bool,
}

/// Where the triage keeps its own runs.
fn runs_dir(home: &Path, game: &str) -> PathBuf {
    home.join("work").join("triage").join(game)
}

/// A start named as `from` is: power-on, a corpus clip `ID`, `ID@F`,
/// `ID@end`, or a session clip `SESSION:N` with the same endings. The
/// frames from power-on to it.
fn start_of(
    home: &Path,
    game_dir: &Path,
    game: &str,
    set: &crate::clipset::ClipSet,
    from: &str,
) -> Result<Vec<InputFrame>> {
    if from == "power-on" || from.is_empty() {
        return Ok(Vec::new());
    }
    let (id, at) = match from.rsplit_once('@') {
        Some((i, a)) => (i, Some(a)),
        None => (from, None),
    };
    let (log, start) = if let Some((s, n)) = id.split_once(':') {
        let dir = home.join("mechanics").join(game).join(s);
        let (text, start) = full_log(home, &dir, n.parse()?)?;
        (
            InputLog::parse(&text).map_err(|e| anyhow::anyhow!("{from}: {e}"))?,
            start,
        )
    } else {
        let r = set
            .clips
            .iter()
            .find(|r| r.id == id)
            .with_context(|| format!("no corpus clip {id}"))?;
        crate::clipset::recipe_log(game_dir, r)?
            .with_context(|| format!("corpus clip {id} has no presses from power-on"))?
    };
    let n = match at {
        None => start,
        Some("end") => log.frames.len(),
        Some(a) => start + a.parse::<usize>().with_context(|| format!("{from}: @F"))?,
    };
    if n > log.frames.len() {
        bail!("{from}: the clip has {} frames", log.frames.len());
    }
    Ok(log.frames[..n].to_vec())
}

/// The autoplay example's binary: given, or beside gp's own build.
fn autoplay_bin(given: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = given {
        return Some(p.to_path_buf());
    }
    if let Some(p) = std::env::var_os("GP_AUTOPLAY") {
        return Some(PathBuf::from(p));
    }
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    [
        dir.join("examples/autoplay"),
        dir.join("../release/examples/autoplay"),
    ]
    .into_iter()
    .find(|p| p.is_file())
}

/// Runs autoplay with `--search` into the triage's folder, once for the same
/// asking: from power-on, or on from `prefix` (a save point's frames).
fn autoplay(
    bin: &Path,
    rom_file: &Path,
    dir: &Path,
    board: &Board,
    prefix: &[InputFrame],
    frames: usize,
    secs: u64,
) -> Result<Vec<InputFrame>> {
    fs::create_dir_all(dir)?;
    let mut key_log = board.log(1);
    key_log.frames = prefix.to_vec();
    let key = gp_evidence::Hash::of(format!("{}\n{frames}", key_log.to_text()).as_bytes()).to_hex()
        [..16]
        .to_owned();
    let out = dir.join(format!("autoplay-{key}"));
    let input = out.with_extension("input");
    if !input.is_file() {
        let mut cmd = std::process::Command::new(bin);
        cmd.arg("--rom")
            .arg(rom_file)
            .arg("--out")
            .arg(&out)
            .arg("--search")
            .arg("--frames")
            .arg((prefix.len() + frames).to_string());
        if !prefix.is_empty() {
            let from = out.with_extension("from.input");
            // The oracle's spelling of the save point's frames.
            fs::write(&from, key_log.to_text())?;
            cmd.arg("--from").arg(&from);
        }
        cmd.stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let mut child = cmd
            .spawn()
            .with_context(|| format!("running {}", bin.display()))?;
        let until = Instant::now() + Duration::from_secs(secs);
        loop {
            if child.try_wait()?.is_some() {
                break;
            }
            if Instant::now() > until {
                let _ = child.kill();
                let _ = child.wait();
                break;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }
    // A run stopped early leaves its partial log.
    let file = if input.is_file() {
        input
    } else {
        let p = dir.join(format!("autoplay-{key}-partial.input"));
        if !p.is_file() {
            bail!("autoplay wrote no log in {secs} s");
        }
        p
    };
    let log = InputLog::parse(&fs::read_to_string(&file)?)
        .map_err(|e| anyhow::anyhow!("{}: {e}", file.display()))?;
    Ok(log.frames)
}

/// The buttons the search presses: one controller byte each.
fn chords(board: &Board) -> Vec<u8> {
    if board.arcade.is_none() {
        return gp_explore::presses::CHORDS.to_vec();
    }
    let k = |c: u8| board.keys.iter().find(|x| x.0 == c).map_or(0, |x| x.1);
    let (r, l, d, u, t) = (k(b'R'), k(b'L'), k(b'D'), k(b'U'), k(b'T'));
    vec![0, r, l, d, u, t, t, t, r | t, l | t]
}

/// The presses that reached a goal and the frame they reached it on.
type Found = (Vec<InputFrame>, usize);

/// Seeded presses from `start`, `tries` runs of up to `frames` each, until
/// the goal holds or the time runs out. The presses that got it.
fn search(
    rom: &[u8],
    board: &Board,
    start: &[InputFrame],
    goal: &Goal,
    frames: usize,
    deadline: Instant,
    seed: u64,
) -> Result<(Option<Found>, usize)> {
    let mut o0 = OriginalRuntime::of_rom(rom)?;
    o0.set_video(false);
    for f in start {
        probe::step(&mut o0, f)?;
    }
    let mut cs = chords(board);
    // From power-on the game has to be started: Start now and then.
    if start.is_empty() && board.arcade.is_none() {
        cs.push(gp_evidence::input_log::button::START);
    }
    let mut rng = gp_explore::presses::Rng::new(seed);
    let mut tries = 0;
    while Instant::now() < deadline {
        tries += 1;
        let mut o = o0.clone();
        let mut w = Watch::default();
        let mut ran = BTreeSet::new();
        goal.look(&o, &ran, &mut w, start.len(), false);
        let mut own = Vec::new();
        while own.len() < frames {
            let p = cs[rng.below(cs.len())];
            let hold = rng.within(4, 24);
            for _ in 0..hold {
                let f = probe::press(u16::from(p));
                step_seeing(&mut o, &f, &goal.routines, &mut ran)?;
                own.push(f);
                goal.look(&o, &ran, &mut w, start.len() + own.len(), true);
                if w.hit.is_some() {
                    break;
                }
            }
            if let Some(h) = w.hit {
                return Ok((Some((own, h)), tries));
            }
        }
    }
    Ok((None, tries))
}

/// Whether Laya (or any decision server) answers.
fn answers(url: &str) -> bool {
    let hostport = url
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap_or("");
    hostport
        .parse::<std::net::SocketAddr>()
        .ok()
        .is_some_and(|a| {
            std::net::TcpStream::connect_timeout(&a, Duration::from_millis(300)).is_ok()
        })
}

/// A hit found: the run's frames and the frame it held on.
struct Hit {
    tool: &'static str,
    run: String,
    frames: Vec<InputFrame>,
    at: usize,
    /// The earliest the clip may start (a corpus clip's own start).
    floor: usize,
    /// The slot that matched, its bytes.
    slot: Vec<u8>,
}

/// The triage of one game: every item of its triage.toml, and its leads.
pub(crate) fn run(paths: &Paths, a: &Args) -> Result<Outcome> {
    let home = &paths.home;
    let games = crate::roms_cmd::games_dir(a.games)
        .context("no games checkout found: pass --games or set GP_GAMES_DIR")?;
    let game_dir = crate::labels::game_folder(&games, a.game)
        .with_context(|| format!("no game {} in {}", a.game, games.display()))?;
    let (rom, set_id, _) = super::server::game_rom(paths, Some(&games), a.game)?;
    // The romset as a file, for autoplay: the bytes the page plays.
    let rom_file = match &rom.system {
        Some(sys) => {
            let d = runs_dir(home, a.game);
            fs::create_dir_all(&d)?;
            let f = d.join(format!("{sys}.zip"));
            if fs::read(&f).ok().as_deref() != Some(rom.bytes.as_slice()) {
                fs::write(&f, &rom.bytes)?;
            }
            Some(f)
        }
        _ => None,
    };
    let board = Board::of_rom(&rom.bytes)?;
    let platform = Platform::of_file(&rom.bytes);
    let plan_file = game_dir.join("triage.toml");
    let mut plan: Plan = match fs::read_to_string(&plan_file) {
        Ok(t) => toml::from_str(&t).with_context(|| plan_file.display().to_string())?,
        Err(_) => Plan::default(),
    };
    // probes.toml's names, and the plan's over them.
    let mut watch = probe::load(&game_dir, None)
        .map(|(_, f)| f.watch)
        .unwrap_or_default();
    watch.extend(std::mem::take(&mut plan.watch));
    let names = Names {
        watch,
        space: if board.arcade.is_some() {
            0x1_0000
        } else {
            0x800
        },
    };
    let set = crate::clipset::read_game(&game_dir)?;

    // The leads, as items whose test is a change of the RAM bytes no clip
    // wrote (the NES only: they come from stage 0's ROM dataset).
    let mut lead_note = String::new();
    if a.leads > 0 {
        if platform == Platform::Nes {
            match crate::leads::leads(home, &rom.bytes, a.game) {
                Ok((all, _)) => {
                    let mut skipped = 0;
                    for (i, l) in all
                        .iter()
                        .filter(|l| l["kind"] == json!("code"))
                        .take(a.leads)
                        .enumerate()
                    {
                        let ram: Vec<String> = l["ram"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|x| {
                                x.as_str()
                                    .map(|s| format!("0x{}", s.trim_start_matches('$')))
                            })
                            .collect();
                        // The lead's routines (CPU addresses): any of them
                        // running shows it.
                        let routines: Vec<String> = l["routines"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|x| x.as_str())
                            .filter(|r| u16::from_str_radix(r, 16).is_ok())
                            .map(str::to_owned)
                            .collect();
                        if ram.is_empty() && routines.is_empty() {
                            skipped += 1;
                            continue;
                        }
                        let gate = l["gate"].as_str().unwrap_or("").to_lowercase();
                        let row = format!(
                            "lead-{}",
                            gate.trim_start_matches('~')
                                .replace(|c: char| !c.is_ascii_alphanumeric(), "")
                        );
                        let row = if row == "lead-" {
                            format!("lead-{}", i + 1)
                        } else {
                            row
                        };
                        plan.item.push(Item {
                            row,
                            note: l["lead"].as_str().unwrap_or("").to_owned(),
                            changes: if routines.is_empty() { ram } else { Vec::new() },
                            routines,
                            hold: 1,
                            before: 90,
                            // Stage 0 reached these with seeded presses from
                            // power-on, 3600 frames a run: the search does the same.
                            after: 1710,
                            from: Some("power-on".into()),
                            todo: "Play on from here and do what the game hasn't been seen to do yet: a new place, a new enemy, a new way to lose or score".into(),
                            stop: "After a few seconds of anything new, or after 30 seconds".into(),
                            ..Item::default()
                        });
                    }
                    if skipped > 0 {
                        lead_note = format!(
                            "{skipped} lead(s) name no routine or RAM byte, so no tool can tell when a run shows them; left out. "
                        );
                    }
                }
                Err(e) => lead_note = format!("No leads: {e:#}. "),
            }
        } else {
            lead_note =
                "No leads: they come from stage 0's ROM dataset, which only an NES game has. "
                    .into();
        }
    }
    let items: Vec<Item> = plan
        .item
        .into_iter()
        .filter(|i| a.rows.is_empty() || a.rows.contains(&i.row))
        .collect();
    if items.is_empty() {
        bail!(
            "nothing to triage: {} has no [[item]] for {}",
            plan_file.display(),
            if a.rows.is_empty() {
                "any row".to_owned()
            } else {
                a.rows.join(", ")
            }
        );
    }
    let goals: Vec<Goal> = items
        .iter()
        .map(|i| -> Result<Goal> {
            let changes = i
                .changes
                .iter()
                .map(|c| probe::num(c).map(|v| v as u16))
                .collect::<Result<Vec<_>>>()?;
            let routines = i
                .routines
                .iter()
                .map(|r| {
                    u16::from_str_radix(r.trim_start_matches('$'), 16)
                        .with_context(|| format!("{}: {r} isn't a CPU address", i.row))
                })
                .collect::<Result<BTreeSet<_>>>()?;
            Ok(Goal {
                routines,
                test: if i.goal.is_empty() {
                    None
                } else {
                    Some(Test::parse(&names, &i.goal, i.slots)?)
                },
                changes,
                hold: i.hold.max(1),
                near: i
                    .near
                    .as_deref()
                    .map(|n| Test::parse(&names, n, i.slots))
                    .transpose()?,
            })
        })
        .collect::<Result<_>>()
        .with_context(|| plan_file.display().to_string())?;
    for i in &items {
        if let Some(t) = i.tools.iter().find(|t| !TOOLS.contains(&t.as_str())) {
            bail!("{}: no tool {t}: the tools are {}", i.row, TOOLS.join(", "));
        }
    }
    let uses = |i: &Item, t: &str| i.tools.is_empty() || i.tools.iter().any(|x| x == t);

    let mut hits: Vec<Option<Hit>> = (0..items.len()).map(|_| None).collect();
    let mut tried: Vec<Vec<Value>> = vec![Vec::new(); items.len()];
    // Where each row's near test first held, the shortest way there: the
    // frames from power-on and what they are.
    let mut nears: Vec<Option<(Vec<InputFrame>, String)>> =
        (0..items.len()).map(|_| None).collect();
    let budget = Duration::from_secs(a.budget.max(1));
    let started = Instant::now();

    // 1. The corpus: each play once, every clip of it as a window.
    let mut runs: Vec<Run> = Vec::new();
    let mut logs: Vec<(String, Vec<InputFrame>, usize, usize)> = Vec::new();
    for r in &set.clips {
        if let Ok(Some((log, start))) = crate::clipset::recipe_log(&game_dir, r) {
            let end = log.frames.len();
            logs.push((r.id.clone(), log.frames, start, end));
        }
    }
    logs.sort_by_key(|l| std::cmp::Reverse(l.1.len()));
    for (id, frames, s, e) in logs {
        let same = |x: &InputFrame, y: &InputFrame| x.pads == y.pads;
        if let Some(run) = runs.iter_mut().find(|r| {
            r.frames.len() >= e
                && r.frames[..e]
                    .iter()
                    .zip(&frames[..e])
                    .all(|(x, y)| same(x, y))
        }) {
            run.windows.push((id, s, e));
        } else {
            runs.push(Run {
                frames,
                windows: vec![(id, s, e)],
            });
        }
    }
    let corpus_items: Vec<usize> = (0..items.len())
        .filter(|&k| uses(&items[k], "corpus"))
        .collect();
    let mut corpus_frames = 0;
    let mut corpus_cut = false;
    let corpus_deadline =
        Instant::now() + budget * u32::try_from(corpus_items.len().max(1)).unwrap_or(1);
    for run in &runs {
        let gs: Vec<&Goal> = corpus_items.iter().map(|&k| &goals[k]).collect();
        let (w, played) = scan(&rom.bytes, &run.frames, &run.windows, &gs, corpus_deadline)?;
        let end = run.windows.iter().map(|x| x.2).max().unwrap_or(0);
        corpus_frames += played;
        corpus_cut |= played < end;
        for (j, &k) in corpus_items.iter().enumerate() {
            for (wi, win) in run.windows.iter().enumerate() {
                let ww = &w[j][wi];
                if let Some(n) = ww.near
                    && nears[k].as_ref().is_none_or(|x| x.0.len() > n)
                {
                    nears[k] = Some((run.frames[..n].to_vec(), format!("{}@{}", win.0, n - win.1)));
                }
                // The earliest hit from power-on: the shortest way there.
                if let Some(h) = ww.hit
                    && hits[k].as_ref().is_none_or(|x| x.at > h)
                {
                    hits[k] = Some(Hit {
                        tool: "corpus",
                        run: win.0.clone(),
                        frames: run.frames[..win.2].to_vec(),
                        at: h,
                        floor: win.1,
                        slot: ww.slot.clone(),
                    });
                }
            }
        }
    }
    for &k in &corpus_items {
        tried[k].push(json!({"tool": "corpus", "said": match &hits[k] {
            Some(h) => format!("corpus clip {} shows it {} frames in", h.run, h.at - h.floor),
            None => format!(
                "none of the {} corpus clips shows it ({} plays, {} frames{})",
                set.clips.len(),
                runs.len(),
                corpus_frames,
                if corpus_cut { ", stopped at the budget before the end of some" } else { "" }
            ),
        }}));
    }

    // 2. Scripted presses.
    for k in 0..items.len() {
        let i = &items[k];
        if hits[k].is_some() || i.tries.is_empty() || !uses(i, "script") {
            continue;
        }
        let from = i.from.clone().unwrap_or_else(|| "power-on".into());
        let said = (|| -> Result<String> {
            let base = start_of(home, &game_dir, a.game, &set, &from)?;
            let mut o = OriginalRuntime::of_rom(&rom.bytes)?;
            o.set_video(false);
            for f in &base {
                probe::step(&mut o, f)?;
            }
            let played = probe::play(
                o,
                &board,
                &names,
                &probe::Layout::sparse(BTreeSet::new()),
                &[],
                &[],
                &i.tries,
                None,
            )?;
            let mut frames = base.clone();
            frames.extend(played.inputs);
            let end = frames.len() + i.after;
            frames.resize(end, probe::press(0));
            let (w, _) = scan(
                &rom.bytes,
                &frames,
                &[("script".into(), base.len(), end)],
                &[&goals[k]],
                Instant::now() + budget,
            )?;
            Ok(match w[0][0].hit {
                Some(h) => {
                    hits[k] = Some(Hit {
                        tool: "script",
                        run: format!("the steps from {from}"),
                        frames,
                        at: h,
                        floor: base.len(),
                        slot: w[0][0].slot.clone(),
                    });
                    format!("the steps from {from} show it")
                }
                None => format!(
                    "the {} steps from {from} never showed it{}",
                    i.tries.len(),
                    played
                        .unreached
                        .map(|u| format!(" (the step `{u}` ran out of frames)"))
                        .unwrap_or_default()
                ),
            })
        })()
        .unwrap_or_else(|e| format!("the steps couldn't run: {e:#}"));
        tried[k].push(json!({"tool": "script", "said": said}));
    }

    // 3. Autoplay, arcade only: from power-on once, then from each row's
    // near point.
    let bin = autoplay_bin(a.autoplay);
    let arcade_tools = platform == Platform::Arcade;
    let ap_items: Vec<usize> = (0..items.len())
        .filter(|&k| hits[k].is_none() && uses(&items[k], "autoplay"))
        .collect();
    if arcade_tools && !ap_items.is_empty() {
        match (&bin, &rom_file) {
            (Some(bin), Some(rf)) => {
                let dir = runs_dir(home, a.game);
                let secs = a.budget.max(30) * 4;
                match autoplay(bin, rf, &dir, &board, &[], a.autoplay_frames, secs) {
                    Ok(frames) => {
                        let end = frames.len();
                        let gs: Vec<&Goal> = ap_items.iter().map(|&k| &goals[k]).collect();
                        let (w, _) = scan(
                            &rom.bytes,
                            &frames,
                            &[("autoplay".into(), 0, end)],
                            &gs,
                            Instant::now() + budget * 4,
                        )?;
                        for (j, &k) in ap_items.iter().enumerate() {
                            if let Some(n) = w[j][0].near
                                && nears[k].as_ref().is_none_or(|x| x.0.len() > n)
                            {
                                nears[k] =
                                    Some((frames[..n].to_vec(), format!("the autoplay run@{n}")));
                            }
                            match w[j][0].hit {
                                Some(h) => {
                                    hits[k] = Some(Hit { tool: "autoplay", run: "autoplay from power-on".into(), frames: frames.clone(), at: h, floor: 0, slot: w[j][0].slot.clone() });
                                    tried[k].push(json!({"tool": "autoplay", "said": format!("autoplay's search from power-on shows it at frame {h}")}));
                                }
                                None => tried[k].push(json!({"tool": "autoplay", "said": format!("autoplay's search played {end} frames from power-on and never showed it")})),
                            }
                        }
                    }
                    Err(e) => {
                        for &k in &ap_items {
                            tried[k].push(json!({"tool": "autoplay", "said": format!("autoplay didn't run: {e:#}")}));
                        }
                    }
                }
                // From each near point still missing, on with the search.
                for &k in &ap_items {
                    if hits[k].is_some() {
                        continue;
                    }
                    let Some((pre, label)) = nears[k].clone() else {
                        continue;
                    };
                    let more = (i64::try_from(items[k].after).unwrap_or(120) * 20) as usize;
                    match autoplay(bin, rf, &dir, &board, &pre, more.max(1800), a.budget.max(30) * 2) {
                        Ok(frames) => {
                            let end = frames.len();
                            let (w, _) = scan(&rom.bytes, &frames, &[("autoplay".into(), pre.len(), end)], &[&goals[k]], Instant::now() + budget)?;
                            match w[0][0].hit {
                                Some(h) => {
                                    hits[k] = Some(Hit { tool: "autoplay", run: format!("autoplay from {label}"), frames, at: h, floor: pre.len(), slot: w[0][0].slot.clone() });
                                    tried[k].push(json!({"tool": "autoplay", "said": format!("autoplay's search from {label} shows it")}));
                                }
                                None => tried[k].push(json!({"tool": "autoplay", "said": format!("autoplay's search from {label} played {} frames on and never showed it", end - pre.len())})),
                            }
                        }
                        Err(e) => tried[k].push(json!({"tool": "autoplay", "said": format!("autoplay from {label} didn't run: {e:#}")})),
                    }
                }
            }
            _ => {
                for &k in &ap_items {
                    tried[k].push(json!({"tool": "autoplay", "said": "no autoplay binary (cargo build --release -p gp-arcade --example autoplay, or GP_AUTOPLAY) or no romset file"}));
                }
            }
        }
    }

    // 4. Seeded presses from the near point (or the row's start).
    for k in 0..items.len() {
        let i = &items[k];
        if hits[k].is_some() || !uses(i, "search") {
            continue;
        }
        let start = match (&nears[k], &i.from) {
            (Some((f, l)), _) => Ok((f.clone(), l.clone())),
            (None, Some(from)) => {
                start_of(home, &game_dir, a.game, &set, from).map(|f| (f, from.clone()))
            }
            (None, None) => Err(anyhow::anyhow!(
                "no start: neither a near point in any run nor a from"
            )),
        };
        let said = match start {
            Ok((frames, label)) => {
                let len = (i.before + i.after).max(300) * 2;
                match search(
                    &rom.bytes,
                    &board,
                    &frames,
                    &goals[k],
                    len,
                    Instant::now() + budget,
                    0x9e37 + k as u64,
                ) {
                    Ok((Some((own, h)), n)) => {
                        let floor = frames.len();
                        let mut all = frames;
                        all.extend(own);
                        hits[k] = Some(Hit {
                            tool: "search",
                            run: format!("seeded presses from {label}"),
                            frames: all,
                            at: h,
                            floor,
                            slot: Vec::new(),
                        });
                        format!("seeded presses from {label} show it on try {n}")
                    }
                    Ok((None, n)) => format!(
                        "{n} tries of seeded presses ({len} frames each) from {label} in {} s never showed it",
                        budget.as_secs()
                    ),
                    Err(e) => format!("the search couldn't run: {e:#}"),
                }
            }
            Err(e) => format!("{e:#}"),
        };
        tried[k].push(json!({"tool": "search", "said": said}));
    }

    // 5. The decision model, NES only.
    if platform == Platform::Nes {
        let up = answers(a.decide);
        for k in 0..items.len() {
            if hits[k].is_some() || !uses(&items[k], "demo") {
                continue;
            }
            tried[k].push(json!({"tool": "demo", "said": if up {
                "Laya answers, but gp mechanic demo starts from a clip of the session and measures jumps: it has no test for this row".to_owned()
            } else {
                format!("Laya isn't answering at {} (just laya serves it)", a.decide)
            }}));
        }
    }

    // What lands where.
    let qf = queue_file(paths, a.game)?;
    let mut q = read_queue(&qf);
    let today = crate::game_new::today();
    let mut made = Vec::new();
    let mut yours = Vec::new();
    for (k, i) in items.iter().enumerate() {
        let mut t = json!({"at": today, "tried": tried[k], "goal": if i.goal.is_empty() { format!("a change of {}", i.changes.join(", ")) } else { i.goal.clone() }});
        if let Some(h) = &hits[k] {
            let start = h.at.saturating_sub(i.before).max(h.floor).min(h.at);
            let end = (h.at + i.hold + i.after).min(h.frames.len());
            t["by"] = json!("tools");
            t["tool"] = json!(h.tool);
            t["why"] = json!(format!("{} got it: {}", h.tool, h.run));
            if !h.slot.is_empty() {
                t["slot"] = json!(
                    h.slot
                        .iter()
                        .map(|b| format!("{b:02X}"))
                        .collect::<Vec<_>>()
                        .join(" ")
                );
            }
            if !a.dry_run {
                let n = save_clip(
                    paths,
                    a.game,
                    &rom.sha256,
                    &board,
                    &i.row,
                    &h.frames[..start],
                    &h.frames[start..end],
                    &format!(
                        "made by the tools ({}): {}, the test `{}` holds {} frames in",
                        h.tool,
                        h.run,
                        t["goal"].as_str().unwrap_or(""),
                        h.at - start
                    ),
                    h.tool,
                )?;
                t["clips"] = json!([n]);
            }
            made.push(format!(
                "{}: {} ({}, frame {}{})",
                i.row,
                h.tool,
                h.run,
                h.at,
                if h.slot.is_empty() {
                    String::new()
                } else {
                    format!(", slot {}", t["slot"].as_str().unwrap_or(""))
                }
            ));
        } else {
            let why = tried[k]
                .iter()
                .filter_map(|x| x["said"].as_str())
                .collect::<Vec<_>>()
                .join("; ");
            t["by"] = json!("you");
            t["why"] = json!(why);
            let (frames, label) = match (&nears[k], &i.from) {
                (Some((f, l)), _) => (f.clone(), l.clone()),
                (None, Some(from)) => (
                    start_of(home, &game_dir, a.game, &set, from).unwrap_or_default(),
                    from.clone(),
                ),
                (None, None) => (Vec::new(), "power-on".to_owned()),
            };
            let mut log = board.log(1);
            log.frames = frames;
            let n = log.frames.len();
            let path = if a.dry_run || n == 0 {
                None
            } else {
                Some(put_path(home, &log.to_text())?.0)
            };
            t["request"] = json!({
                "from": label,
                "path": path,
                "frame": n,
                "where": if i.place.is_empty() { format!("{label}, {:.0} s from power-on", n as f64 / fps(platform)) } else { i.place.clone() },
                "do": if i.todo.is_empty() { i.note.clone() } else { i.todo.clone() },
                "stop": if i.stop.is_empty() { "Once it has happened, then R to stop recording".to_owned() } else { i.stop.clone() },
            });
            yours.push(format!("{}: from {label}. Tools: {why}", i.row));
        }
        if !q.iter().any(|r| r["mechanic"] == json!(i.row)) {
            q.push(json!({"mechanic": i.row, "status": "next", "note": i.note}));
            if !a.dry_run {
                Session::create(paths, a.game, &i.row, Some(&i.note), &rom.sha256)?;
            }
        }
        if let Some(r) = q.iter_mut().find(|r| r["mechanic"] == json!(i.row)) {
            r["triage"] = t;
        }
    }
    if !a.dry_run {
        write_queue(&qf, &q)?;
    }
    let mut text = format!(
        "{} ({set_id}): {} made by the tools, {} need you, in {:.0} s. {lead_note}\n",
        a.game,
        made.len(),
        yours.len(),
        started.elapsed().as_secs_f64()
    );
    text.push_str("Made by the tools:\n");
    for m in &made {
        text.push_str(&format!("  {m}\n"));
    }
    text.push_str("Needs you:\n");
    for y in &yours {
        text.push_str(&format!("  {y}\n"));
    }
    Ok(Outcome::ok(
        json!({"game": a.game, "set": set_id, "made": made, "needs_you": yours, "queue": q}),
        text,
    ))
}

fn fps(p: Platform) -> f64 {
    match p {
        Platform::Nes => 60.0988,
        Platform::Arcade => 30.0,
    }
}

/// Saves a run the tools made as a clip of the row's session: the frames
/// before its start as a path, its own after.
#[allow(clippy::too_many_arguments)]
fn save_clip(
    paths: &Paths,
    game: &str,
    rom_sha: &str,
    board: &Board,
    row: &str,
    before: &[InputFrame],
    own: &[InputFrame],
    note: &str,
    tool: &str,
) -> Result<u64> {
    let mut prefix = board.log(1);
    prefix.frames = before.to_vec();
    let (sha, _) = put_path(&paths.home, &prefix.to_text())?;
    let mut mine = board.log(1);
    mine.frames = own.to_vec();
    let (mut t, log) = clip_fields(&sha, before.len(), &mine);
    t.insert("tool".into(), tool.into());
    let s = Session::create(paths, game, row, None, rom_sha)?;
    // The same run saved again by a later triage gives the clip there.
    for c in s.clips()? {
        if c["path"].as_str() == Some(sha.as_str())
            && c["by"] == json!("tools")
            && let Some(n) = c["n"].as_u64()
            && fs::read_to_string(s.dir.join(format!("clips/{n:03}.log"))).is_ok_and(|l| l == log)
        {
            return Ok(n);
        }
    }
    s.add_clip_meta(t, &log, note, "tools")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> Names {
        Names {
            watch: [("lives".to_owned(), 0xE4A7u16)].into_iter().collect(),
            space: 0x1_0000,
        }
    }

    /// A test splits into fixed tests and `@K` tests of any slot, and an
    /// `@K` test without slots is refused.
    #[test]
    fn a_test_has_fixed_and_slot_parts() {
        let slots = Slots {
            at: 0xE2C2,
            stride: 4,
            count: 8,
        };
        let t = Test::parse(
            &names(),
            "lives == 2 and @0 == 0xC5 and @3 < 0x30",
            Some(slots),
        )
        .unwrap();
        assert_eq!(
            t.fixed.0.iter().map(|a| a.addr).collect::<Vec<_>>(),
            [0xE4A7]
        );
        let (s, c) = t.slot.unwrap();
        assert_eq!(s.at, 0xE2C2);
        assert_eq!(c.0.iter().map(|a| a.addr).collect::<Vec<_>>(), [0, 3]);
        // Slot 1 drinking at x 0x20 holds; at x 0x40 it doesn't.
        let ram = |x: u8| move |k: u16| [0xC5, 0, 0x23, x][usize::from(k)];
        assert!(c.holds_by(ram(0x20)));
        assert!(!c.holds_by(ram(0x40)));
        assert!(Test::parse(&names(), "@0 == 1", None).is_err());
        assert!(Test::parse(&names(), "nobody == 1", None).is_err());
    }

    /// A plan reads its items with their defaults.
    #[test]
    fn a_plan_reads() {
        let p: Plan = toml::from_str(
            "[watch]\nmode = 0xE3BD\n[[item]]\nrow = \"die\"\ngoal = \"mode == 0\"\nwhere = \"the saloon\"\ndo = \"lose a life\"\n",
        )
        .unwrap();
        assert_eq!(p.watch["mode"], 0xE3BD);
        let i = &p.item[0];
        assert_eq!((i.hold, i.before, i.after), (1, 90, 120));
        assert_eq!(
            (i.place.as_str(), i.todo.as_str()),
            ("the saloon", "lose a life")
        );
        assert!(toml::from_str::<Plan>("[[item]]\nrow = \"x\"\nwhat = 1\n").is_err());
    }
}
