//! Sealed runs, kept apart from every tool a worker runs (docs/sealed.md).
//!
//! A sealed run is a play kept under `$GP_HOME/sealed/GAME/`. It measures
//! the model on play no worker has seen, so no tool reads it. [`Guard`] is
//! the one place that refuses it: installed at start, it is asked by the
//! store's recording lookup and by the input-log readers (the clip set
//! reader among them), so any command that opens a recording or a log that
//! plays a sealed run is refused with the same message. [`refuse_clip_in`]
//! stays for what an input alone doesn't show (a `sealed*` session, a
//! folder under `$GP_HOME/sealed`). A play overlaps a sealed run when it is
//! the same input from power-on into a press of the sealed run's own frames
//! ([`overlaps`]). Another game's sealed runs never block a play: a sealed
//! run applies only to plays of the same game ([`applies`]).
//!
//! Only the coordinator's `gp sealed score` and `gp sealed leaks` read what
//! the guard refuses, and `score` writes what it finds under
//! `$GP_HOME/sealed/GAME/` alone.
//!
//! Ported from game-printer's sealed.rs. Games are named against the games
//! checkout (each game's roms.toml) instead of the old `sources/` registry,
//! a game's corpus is its package's, and the input readers that refused a
//! file by path (`read_input`, `read_baseline_input`) are not here: no
//! command of gameprinter's reads an input log file but through a clip set
//! or the mechanic page.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use clap::Subcommand;
use gp_evidence::guard::Of;
use gp_evidence::registry::Registry;
use gp_evidence::{InputLog, Paths};
use serde_json::json;

use crate::Outcome;

/// The folder under a game's sealed runs that `gp sealed score` writes
/// to; the runs are never read from it.
const SCORES: &str = "scores";
/// The ledger `gp sealed score` adds a line to for each score.
const LEDGER: &str = "ledger.txt";
/// The folder under a game's sealed runs that the page's sealed clips were
/// kept in, as a session for its numbering.
pub(crate) const SEALED_ROW: &str = "sealed-run";

#[derive(Subcommand)]
pub enum SealedCmd {
    /// Score a sealed run on a model the way `gp clip score` scores a clip
    /// (straight through and with resync, with the original's random draws
    /// when the mapping sets `score_draws`). It is for the coordinator
    /// alone; a worker never runs it, since the run's frames and scores
    /// would tell it what the sealed run holds. Everything it writes stays
    /// under $GP_HOME/sealed/GAME/, the scores in scores/RUN/ and a line
    /// (the model's hash, both percentages and the longest stretch) added
    /// to ledger.txt.
    Score {
        /// The game: its folder's name in the games checkout, or a source
        /// ID its roms.toml lists (donkey-kong-oe). Its sealed runs are
        /// under $GP_HOME/sealed/GAME/.
        game: String,
        /// The sealed run: N for clip N the page sealed
        /// (sealed-run/clips/NNN), SESSION:N for a clip of another folder
        /// under the game's sealed runs, or a log's name there.
        run: String,
        /// The model folder (model.toml and mapping.toml); by default the
        /// game's own.
        #[arg(long)]
        model: Option<PathBuf>,
        /// The ROM; by default the one in $GP_ROM_DIR the package names.
        #[arg(long)]
        rom: Option<PathBuf>,
        /// The game's built module. Needed for a model with hooks.
        #[arg(long)]
        module: Option<PathBuf>,
        /// The model's lifted assets (default: $GP_HOME/work/assets/SOURCE).
        #[arg(long)]
        assets: Option<PathBuf>,
        /// The games checkout (default: $GP_GAMES_DIR, else a `games`
        /// folder at or beside the current folder or one above it).
        #[arg(long)]
        games: Option<PathBuf>,
    },
    /// List every stored recording, mechanic-session clip and corpus clip of a
    /// game that plays one of its sealed runs, the ones every other tool
    /// refuses. Only the coordinator runs this command. It reads
    /// $GP_HOME/sealed itself, but prints only each leak's ID and the name of
    /// the sealed run it matches, never frames or presses.
    Leaks {
        /// The game, as for `score`.
        game: String,
        /// The games checkout, as for `score`.
        #[arg(long)]
        games: Option<PathBuf>,
    },
}

pub(crate) fn run(cmd: &SealedCmd, paths: &Paths) -> Result<Outcome> {
    match cmd {
        SealedCmd::Score {
            game,
            run,
            model,
            rom,
            module,
            assets,
            games,
        } => score(
            paths,
            game,
            run,
            &crate::clip_score::Open {
                module: module.as_deref(),
                model: model.as_deref(),
                assets: assets.as_deref(),
                rom: rom.as_deref(),
            },
            games.as_deref(),
        ),
        SealedCmd::Leaks { game, games } => leaks(paths, game, games.as_deref()),
    }
}

/// The package folder of `game` in the games checkout.
fn game_dir(game: &str, games: Option<&Path>) -> Result<PathBuf> {
    let checkout = crate::roms_cmd::games_dir(games)
        .context("no games checkout: pass --games or set GP_GAMES_DIR")?;
    crate::labels::game_folder(&checkout, game)
        .with_context(|| format!("no game {game} in {}", checkout.display()))
}

/// One thing that plays a sealed run: what it is, its ID and the run.
struct Leak {
    kind: &'static str,
    id: String,
    run: String,
}

/// Every stored recording, session clip and corpus clip of `game` that
/// plays one of its sealed runs. Recordings are read without the guard,
/// since finding what it refuses is the point; nothing of a run but its
/// name is printed.
fn leaks(paths: &Paths, game: &str, games: Option<&Path>) -> Result<Outcome> {
    let home = &paths.home;
    let runs = runs(home, game);
    let mut found: Vec<Leak> = Vec::new();
    let mut push = |kind, id: String, r: &Run| {
        found.push(Leak {
            kind,
            id,
            run: name_of(home, &r.file),
        });
    };
    let mut checked = [0usize; 3];
    if !runs.is_empty() {
        // Stored recordings of the game ([`applies`]).
        if let Ok(store) = gp_evidence::Store::open_existing(&paths.store) {
            let registry = quiet_registry();
            let listing = gp_evidence::recordings::list_checked(&store)?;
            for e in gp_evidence::recordings::unique(listing.entries) {
                let Ok(m) = gp_evidence::recordings::load_manifest_unguarded(&store, &e.id) else {
                    continue;
                };
                let play = ident(registry.as_ref(), &of_recording(&store, &m), []);
                let of_game: Vec<&Run> = runs.iter().filter(|r| applies(&play, &r.ident)).collect();
                if of_game.is_empty() {
                    continue;
                }
                let Some(k) = recording_keys(&store, &m) else {
                    continue;
                };
                checked[0] += 1;
                if let Some(r) = matching(of_game, &k, 0) {
                    push("recording", e.id.to_hex()[..12].to_owned(), r);
                }
            }
        }
        // The game's mechanic sessions: $GP_HOME/mechanics/GAME/SESSION/clips.
        let mut logs = Vec::new();
        if let Ok(rd) = fs::read_dir(home.join("mechanics").join(game)) {
            for s in rd.flatten() {
                let Ok(cd) = fs::read_dir(s.path().join("clips")) else {
                    continue;
                };
                for c in cd.flatten() {
                    let p = c.path();
                    if p.extension().is_some_and(|x| x == "log") {
                        logs.push(p);
                    }
                }
            }
        }
        logs.sort();
        for p in logs {
            let Ok((log, start)) = full_frames(home, &p) else {
                continue;
            };
            checked[1] += 1;
            if let Some(r) = matching(&runs, &keys(&log), start) {
                let session = p
                    .parent()
                    .and_then(Path::parent)
                    .and_then(Path::file_name)
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let n = p.file_stem().map(|n| n.to_string_lossy().into_owned());
                push(
                    "session clip",
                    format!("{session}:{}", n.unwrap_or_default()),
                    r,
                );
            }
        }
        // The game's corpus: its package's plan, or an older clips.toml,
        // read without the guard's filter.
        if let Ok(dir) = game_dir(game, games) {
            let set = match crate::plan::load(&dir)? {
                Some(p) => Some(p.set),
                None => {
                    let f = dir.join(crate::clipset::FILE);
                    match fs::read_to_string(&f) {
                        Ok(text) => Some(crate::clipset::parse(&text, &f)?),
                        Err(_) => None,
                    }
                }
            };
            for r in set.iter().flat_map(|s| &s.clips) {
                let Ok(Some((log, start))) = crate::clipset::recipe_log(&dir, r) else {
                    continue;
                };
                checked[2] += 1;
                if let Some(s) = matching(&runs, &keys(&log), start) {
                    push("corpus clip", r.id.clone(), s);
                }
            }
        }
    }
    let mut text = format!(
        "{game}: {} sealed run{}; checked {} recordings, {} session clips, {} corpus clips; {} play a sealed run",
        runs.len(),
        if runs.len() == 1 { "" } else { "s" },
        checked[0],
        checked[1],
        checked[2],
        found.len(),
    );
    for l in &found {
        text.push_str(&format!("\n  {} {} plays {}", l.kind, l.id, l.run));
    }
    Ok(Outcome::ok(
        json!({
            "game": game,
            "sealed_runs": runs.len(),
            "checked": {"recordings": checked[0], "session_clips": checked[1], "corpus_clips": checked[2]},
            "leaks": found.iter().map(|l| json!({"kind": l.kind, "id": l.id, "sealed_run": l.run})).collect::<Vec<_>>(),
        }),
        text,
    ))
}

/// An input frame as one number: the buttons, and the reset in bit 32.
pub(crate) fn key(buttons: u32, reset: bool) -> u64 {
    u64::from(buttons) | (u64::from(reset) << 32)
}

/// The keys of an input log's frames.
pub(crate) fn keys(log: &InputLog) -> Vec<u64> {
    log.frames
        .iter()
        .map(|f| key(f.buttons_word(), f.reset))
        .collect()
}

/// A clip's log from power-on and its start: the `.log`, with the path
/// and start frame (and its own frames) from the `.toml` beside it.
pub(crate) fn full_frames(home: &Path, log: &Path) -> Result<(InputLog, usize)> {
    let t: toml::Table = match fs::read_to_string(log.with_extension("toml")) {
        Ok(text) => toml::from_str(&text)
            .with_context(|| format!("reading {}", log.with_extension("toml").display()))?,
        Err(_) => toml::Table::new(),
    };
    let path = t.get("path").and_then(|v| v.as_str()).map(str::to_owned);
    let start = t.get("start").and_then(|v| v.as_integer()).unwrap_or(0) as usize;
    let frames = t
        .get("frames")
        .and_then(|v| v.as_integer())
        .map(|f| f.max(0) as usize);
    let own = InputLog::parse(
        &fs::read_to_string(log).with_context(|| format!("reading {}", log.display()))?,
    )
    .map_err(|e| anyhow::anyhow!("{}: {e}", log.display()))?;
    let (mut full, start) = match &path {
        Some(sha) => {
            let mut p = crate::mechanic::paths::read_path(home, sha)?;
            if start > p.frames.len() {
                bail!("path {sha} has {} frames, not {start}", p.frames.len());
            }
            p.frames.truncate(start);
            p.controllers = p.controllers.max(own.controllers);
            let mut own = own.frames;
            if let Some(f) = frames {
                own.truncate(f);
            }
            p.frames.extend(own);
            (p, start)
        }
        None => {
            let start = start.min(own.frames.len());
            (own, start)
        }
    };
    if path.is_none()
        && let Some(f) = frames
    {
        full.frames.truncate(start + f);
    }
    Ok((full, start))
}

/// One sealed run: its file, its frames from power-on, its start and the
/// game it is of.
pub(crate) struct Run {
    pub file: PathBuf,
    keys: Vec<u64>,
    start: usize,
    ident: Ident,
}

/// Who a play or a sealed run is of, resolved against the games' roms.toml.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Ident {
    /// Its source ID, when a game's roms.toml lists it.
    id: Option<String>,
    /// The name it goes by: a source ID, a game's or a sealed folder's name.
    name: Option<String>,
    /// The headerless SHA-256s of the ROMs it plays on.
    roms: std::collections::BTreeSet<String>,
}

/// `of` resolved against the registry, with `roms` added: a ROM the
/// registry lists names its game, and a registered game adds its ROM.
pub(crate) fn ident(
    registry: Option<&Registry>,
    of: &Of,
    roms: impl IntoIterator<Item = String>,
) -> Ident {
    let mut roms: std::collections::BTreeSet<String> =
        roms.into_iter().chain(of.rom.clone()).collect();
    let id = of
        .game
        .as_deref()
        .and_then(|g| registry?.by_id(g))
        .or_else(|| roms.iter().find_map(|r| registry?.by_sha256(r)))
        .map(|s| s.id.clone());
    if let Some(s) = id.as_deref().and_then(|i| registry?.by_id(i)) {
        roms.insert(s.sha256.clone());
    }
    Ident {
        id,
        name: of.game.clone(),
        roms,
    }
}

/// Whether the sealed run `run` applies to the play `play`: the two are of
/// the same game. By source ID when both games are registered; by ROM hash
/// when either isn't and both ROMs are known; by name when neither knows a
/// ROM. When nothing tells the two apart (a play that names no game and no
/// ROM, or a sealed run of an unregistered game with no ROM beside it), the
/// run applies, so a sealed run is never read for want of a name.
pub(crate) fn applies(play: &Ident, run: &Ident) -> bool {
    if let (Some(a), Some(b)) = (&play.id, &run.id) {
        return a == b;
    }
    if !play.roms.is_empty() && !run.roms.is_empty() {
        return !play.roms.is_disjoint(&run.roms);
    }
    if let (Some(a), Some(b)) = (&play.name, &run.name) {
        return a == b;
    }
    true
}

fn sealed_root(home: &Path) -> PathBuf {
    home.join("sealed")
}

/// The ROMs a sealed run is of: the `rom_sha256` of the `.toml` beside it,
/// else every `.nes` in the nearest folder holding one, from the run's
/// own up to its game's folder (`top`), by headerless SHA-256.
fn run_roms(top: &Path, log: &Path) -> Vec<String> {
    let given = fs::read_to_string(log.with_extension("toml"))
        .ok()
        .and_then(|t| toml::from_str::<toml::Table>(&t).ok())
        .and_then(|t| t.get("rom_sha256")?.as_str().map(str::to_owned));
    if let Some(r) = given {
        return vec![r];
    }
    let mut dir = log.parent();
    while let Some(d) = dir.filter(|d| d.starts_with(top)) {
        let mut found: Vec<String> = fs::read_dir(d)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("nes")))
            .filter_map(|p| fs::read(p).ok())
            .filter_map(|b| gp_evidence::source::identify_nes(&b).ok())
            .map(|i| i.sha256)
            .collect();
        if !found.is_empty() {
            found.sort();
            return found;
        }
        dir = d.parent();
    }
    Vec::new()
}

/// The sealed runs filed under `$GP_HOME/sealed/GAME/`: each `.log` there,
/// with the `.toml` beside it saying where it starts. A log that can't be
/// read is left out.
fn filed_under(home: &Path, game: &str, registry: Option<&Registry>) -> Vec<Run> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                if p.file_name().is_some_and(|n| n != SCORES) {
                    walk(&p, out);
                }
            } else if p.extension().is_some_and(|x| x == "log") {
                out.push(p);
            }
        }
    }
    if game.is_empty() || game.contains('/') || game.contains("..") {
        return Vec::new();
    }
    let top = sealed_root(home).join(game);
    let mut files = Vec::new();
    walk(&top, &mut files);
    files.sort();
    files
        .into_iter()
        .filter_map(|f| {
            let (log, start) = full_frames(home, &f).ok()?;
            let ident = ident(registry, &Of::game(game), run_roms(&top, &f));
            Some(Run {
                keys: keys(&log),
                start,
                file: f,
                ident,
            })
        })
        .collect()
}

/// Every sealed run of every game.
fn all_runs(home: &Path, registry: Option<&Registry>) -> Vec<Run> {
    games(home)
        .iter()
        .flat_map(|g| filed_under(home, g, registry))
        .collect()
}

/// The sealed runs of the game named `game` (a source ID or a game's
/// folder name), wherever they are filed ([`applies`]).
pub(crate) fn runs(home: &Path, game: &str) -> Vec<Run> {
    if game.is_empty() || game.contains('/') || game.contains("..") {
        return Vec::new();
    }
    runs_for(home, &Of::game(game))
}

/// The sealed runs that apply to a play of `of` ([`applies`]).
pub(crate) fn runs_for(home: &Path, of: &Of) -> Vec<Run> {
    if games(home).is_empty() {
        return Vec::new();
    }
    let registry = quiet_registry();
    let play = ident(registry.as_ref(), of, []);
    all_runs(home, registry.as_ref())
        .into_iter()
        .filter(|r| applies(&play, &r.ident))
        .collect()
}

/// Whether a play from power-on (`c`, its own frames from `cs`) plays a
/// sealed run's frames (`s`, its own from `ss`): the two are the same
/// from power-on into the sealed run's own frames, and the shared part of
/// those holds a press. Two plays that only wait through the same title
/// screen, a clip cut from an idle stretch, or a clip cut later from the
/// same play after the sealed run ended, don't overlap.
pub(crate) fn overlaps(c: &[u64], cs: usize, s: &[u64], ss: usize) -> bool {
    let n = c.len().min(s.len());
    let d = (0..n).find(|&i| c[i] != s[i]).unwrap_or(n);
    let lo = cs.max(ss);
    if lo >= d {
        return false;
    }
    c[lo..d].iter().any(|&k| k != 0)
}

fn refusal(what: &str, why: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "refused: {what} is sealed ({why}). Sealed runs are scored only by the coordinator, with gp sealed score"
    )
}

/// The sealed run's name for a message: its file under `$GP_HOME/sealed`.
fn name_of(home: &Path, f: &Path) -> String {
    f.strip_prefix(sealed_root(home))
        .unwrap_or(f)
        .display()
        .to_string()
}

/// The games with a folder under `$GP_HOME/sealed`.
fn games(home: &Path) -> Vec<String> {
    let Ok(rd) = fs::read_dir(sealed_root(home)) else {
        return Vec::new();
    };
    let mut out: Vec<String> = rd
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().to_str().map(str::to_owned))
        .collect();
    out.sort();
    out
}

/// The sealed run that frames from power-on (their own from `start`)
/// play, if any.
fn matching<'a>(
    runs: impl IntoIterator<Item = &'a Run>,
    c: &[u64],
    start: usize,
) -> Option<&'a Run> {
    runs.into_iter()
        .find(|r| overlaps(c, start, &r.keys, r.start))
}

/// What a recording is of: an NES recording its ROM, a model's recording
/// its model's source.
fn of_recording(store: &gp_evidence::Store, m: &gp_evidence::RecordingManifest) -> Of {
    use gp_evidence::manifest::SourceRef;
    match &m.source {
        SourceRef::NesRom { sha256, .. } => Of::rom(sha256),
        SourceRef::Model { .. } => gp_interp::recording::bundle(store, m)
            .ok()
            .and_then(|b| gp_model::Model::parse(&b.model).ok())
            .and_then(|m| m.model.source)
            .map_or_else(Of::default, |g| Of::game(&g)),
    }
}

/// The games checkout's roms.toml files as one registry, read quietly.
fn quiet_registry() -> Option<Registry> {
    Registry::load_games(&crate::roms_cmd::games_dir(None)?).ok()
}

/// A recording's input log, read without the guard.
fn recording_keys(
    store: &gp_evidence::Store,
    m: &gp_evidence::RecordingManifest,
) -> Option<Vec<u64>> {
    let text = String::from_utf8(store.get(&m.input_log).ok()?).ok()?;
    Some(keys(&InputLog::parse(&text).ok()?))
}

/// The guard every reader of a recording or an input log passes through
/// ([`gp_evidence::guard`]): it refuses a recording or a log that plays a
/// sealed run of its own game, with the same message as every other
/// refusal here. A recording is its own from power-on, so one that runs
/// through a sealed run's presses is refused whatever part a tool reads.
pub(crate) struct Guard {
    home: PathBuf,
    registry: std::sync::OnceLock<Option<Registry>>,
    all: std::sync::OnceLock<Vec<Run>>,
}

impl Guard {
    pub(crate) fn new(home: &Path) -> Self {
        Self {
            home: home.to_owned(),
            registry: std::sync::OnceLock::new(),
            all: std::sync::OnceLock::new(),
        }
    }

    fn registry(&self) -> Option<&Registry> {
        self.registry.get_or_init(quiet_registry).as_ref()
    }

    /// Refuses frames from power-on of `of` that play a sealed run of its
    /// game; every sealed run is read once.
    fn refuse(&self, what: &str, of: &Of, c: &[u64], start: usize) -> Result<(), String> {
        let play = ident(self.registry(), of, []);
        let all = self
            .all
            .get_or_init(|| all_runs(&self.home, self.registry()));
        refuse_keys_in(
            &self.home,
            all.iter().filter(|r| applies(&play, &r.ident)),
            what,
            c,
            start,
        )
        .map_err(|e| e.to_string())
    }
}

impl gp_evidence::guard::Guard for Guard {
    fn recording(
        &self,
        store: &gp_evidence::Store,
        id: &gp_evidence::Hash,
        m: &gp_evidence::RecordingManifest,
    ) -> std::result::Result<(), String> {
        // Nothing sealed at all: no game to look up, no log to read.
        if games(&self.home).is_empty() {
            return Ok(());
        }
        let Some(k) = recording_keys(store, m) else {
            return Ok(());
        };
        let hex = id.to_hex();
        self.refuse(
            &format!("the recording {}", &hex[..12]),
            &of_recording(store, m),
            &k,
            0,
        )
    }

    fn input(
        &self,
        what: &str,
        of: &Of,
        log: &InputLog,
        start: usize,
    ) -> std::result::Result<(), String> {
        if games(&self.home).is_empty() {
            return Ok(());
        }
        self.refuse(what, of, &keys(log), start)
    }
}

/// Installs the guard for this process (`main` does, before any command).
pub(crate) fn install(home: &Path) {
    gp_evidence::guard::install(Box::new(Guard::new(home)));
}

fn refuse_keys_in<'a>(
    home: &Path,
    runs: impl IntoIterator<Item = &'a Run>,
    what: &str,
    c: &[u64],
    start: usize,
) -> Result<()> {
    match matching(runs, c, start) {
        Some(r) => Err(refusal(
            what,
            &format!(
                "its path, start and inputs match the sealed run {}",
                name_of(home, &r.file)
            ),
        )),
        None => Ok(()),
    }
}

/// Whether `p` is under `$GP_HOME/sealed`.
fn under_sealed(home: &Path, p: &Path) -> bool {
    let canon = |x: &Path| x.canonicalize().unwrap_or_else(|_| x.to_owned());
    canon(p).starts_with(canon(&sealed_root(home)))
}

/// Refuses clip `n` of a session folder against a game's sealed runs read
/// once ([`runs`]): a `sealed*` session, a folder under `$GP_HOME/sealed`,
/// or a clip whose path, start and inputs play a sealed run's.
pub(crate) fn refuse_clip_in(home: &Path, runs: &[Run], dir: &Path, n: u64) -> Result<()> {
    let s = dir
        .file_name()
        .map(|x| x.to_string_lossy().into_owned())
        .unwrap_or_default();
    let what = format!("clip {n} of {s}");
    if under_sealed(home, dir) {
        return Err(refusal(&what, "it is kept under $GP_HOME/sealed"));
    }
    if s.starts_with("sealed") {
        return Err(refusal(&what, &format!("it is a clip of the session {s}")));
    }
    let (log, start) = full_frames(home, &dir.join(format!("clips/{n:03}.log")))?;
    refuse_keys_in(home, runs, &what, &keys(&log), start)
}

/// The sealed logs of a game, each by its presses and where they start
/// ([`clip_key`]): the logs at the top of its sealed folder and the
/// page's sealed clips.
pub(crate) fn sealed_logs(home: &Path, game: &str) -> std::collections::HashSet<blake3::Hash> {
    let top = sealed_root(home).join(game);
    [top.clone(), top.join(SEALED_ROW).join("clips")]
        .iter()
        .filter_map(|d| fs::read_dir(d).ok())
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "log"))
        .filter_map(|p| clip_key(&p))
        .collect()
}

/// A clip by its presses and where they start: its `.log` with the path
/// and start frame from the `.toml` beside it.
pub(crate) fn clip_key(log: &Path) -> Option<blake3::Hash> {
    let b = fs::read(log).ok()?;
    let t: toml::Table = fs::read_to_string(log.with_extension("toml"))
        .ok()
        .and_then(|t| toml::from_str(&t).ok())
        .unwrap_or_default();
    let mut h = blake3::Hasher::new();
    h.update(&b);
    for k in ["path", "start"] {
        h.update(
            t.get(k)
                .map(|v| v.to_string())
                .unwrap_or_default()
                .as_bytes(),
        );
    }
    Some(h.finalize())
}

/// The log of a sealed run named on the command line.
fn run_file(home: &Path, game: &str, run: &str) -> Result<PathBuf> {
    let top = sealed_root(home).join(game);
    let f = match run.split_once(':') {
        Some((s, n)) => {
            let n: u64 = n.parse().with_context(|| format!("{run}: SESSION:N"))?;
            top.join(s).join(format!("clips/{n:03}.log"))
        }
        None => match run.parse::<u64>() {
            Ok(n) => top.join(format!("{SEALED_ROW}/clips/{n:03}.log")),
            Err(_) => {
                let f = top.join(run);
                if f.extension().is_some_and(|x| x == "log") {
                    f
                } else {
                    f.with_extension("log")
                }
            }
        },
    };
    if run.contains("..") || game.contains("..") || game.contains('/') || !f.is_file() {
        bail!("no sealed run {run} of {game}");
    }
    Ok(f)
}

/// Now in UTC, as `YYYY-MM-DDTHH:MM:SS.ffffffZ`, the old ledger's form.
fn now_rfc3339() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let (secs, micros) = (now.as_secs(), now.subsec_micros());
    // The civil date from days since 1970 (Howard Hinnant's algorithm).
    let z = (secs / 86400) as i64 + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    let s = secs % 86400;
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{micros:06}Z",
        s / 3600,
        s / 60 % 60,
        s % 60
    )
}

fn score(
    paths: &Paths,
    game: &str,
    run: &str,
    how: &crate::clip_score::Open,
    games: Option<&Path>,
) -> Result<Outcome> {
    let home = &paths.home;
    let file = run_file(home, game, run)?;
    let (log, start) = full_frames(home, &file)?;
    let key = crate::clipset::file_key(run);
    let top = sealed_root(home).join(game);
    let out = top.join(SCORES).join(&key);
    let dir = game_dir(game, games)?;
    let mut ctx = crate::clip_score::Ctx::open(paths, &dir, how)?;
    // Nothing of a sealed run's play goes into the shared start cache.
    ctx.starts = false;
    let s = crate::clip_score::score_one(&ctx, &format!("sealed run {run}"), &log, start, &out)?;
    let model_file = how.model.unwrap_or(&dir).join(gp_model::MODEL_FILE);
    let model_text = fs::read_to_string(&model_file)
        .with_context(|| format!("reading {}", model_file.display()))?;
    let model_hash = crate::corpus_score::model_key(&model_text);
    let st = &s.json["stretches"];
    let line = format!(
        "{} {run} model {} straight {} resync {} longest {} s\n",
        now_rfc3339(),
        &model_hash[..model_hash.len().min(12)],
        crate::clip_score::percent(&s.json["straight"]),
        crate::clip_score::percent(&s.json["resync"]),
        gp_interp::lockstep::seconds(st["longest_any"].as_u64().unwrap_or(0)),
    );
    let ledger = top.join(LEDGER);
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&ledger)
        .with_context(|| format!("opening {}", ledger.display()))?;
    std::io::Write::write_all(&mut f, line.as_bytes())?;
    Ok(Outcome::ok(
        json!({"run": run, "model": model_hash, "score": s.json, "ledger": ledger}),
        format!("{}\nadded a line to {}", s.text, ledger.display()),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gp_evidence::input_log::generate;

    fn log_of(frames: usize, spans: &[&str]) -> InputLog {
        generate(frames, spans).unwrap()
    }

    #[test]
    fn a_play_overlaps_only_through_a_press_of_the_runs_own_frames() {
        let s = keys(&log_of(20, &["right@10-15"]));
        // The same play from power-on into the run's own presses.
        assert!(overlaps(&s, 0, &s, 5));
        // Only the idle lead-in in common.
        let c = keys(&log_of(20, &["left@10-15"]));
        assert!(!overlaps(&c, 0, &s, 5));
        // Cut after the sealed run's presses ended.
        assert!(!overlaps(&s, 16, &s, 5));
    }

    #[test]
    fn a_run_applies_by_id_then_rom_then_name() {
        let a = |id: Option<&str>, name: Option<&str>, roms: &[&str]| Ident {
            id: id.map(str::to_owned),
            name: name.map(str::to_owned),
            roms: roms.iter().map(|r| (*r).to_owned()).collect(),
        };
        assert!(!applies(&a(Some("x"), None, &[]), &a(Some("y"), None, &[])));
        assert!(applies(&a(None, None, &["r"]), &a(None, None, &["r", "s"])));
        assert!(!applies(&a(None, Some("g"), &[]), &a(None, Some("h"), &[])));
        assert!(applies(&Ident::default(), &a(Some("y"), None, &[])));
    }

    /// A sealed run under $GP_HOME/sealed/GAME is found, a session clip
    /// that plays it is refused, and leaks names that clip and nothing of
    /// the run but its name.
    #[test]
    fn sealed_runs_are_found_refused_and_listed_as_leaks() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path().to_path_buf();
        let paths = Paths {
            home: home.clone(),
            store: home.join("store"),
            roms: home.join("roms"),
        };
        let sealed = home.join("sealed/g");
        fs::create_dir_all(&sealed).unwrap();
        let run = log_of(30, &["right@5-20"]);
        fs::write(sealed.join("run-1.log"), run.to_text()).unwrap();
        let session = home.join("mechanics/g/walk");
        fs::create_dir_all(session.join("clips")).unwrap();
        fs::write(session.join("clips/001.log"), run.to_text()).unwrap();
        fs::write(session.join("clips/001.toml"), "start = 8\n").unwrap();
        fs::write(
            session.join("clips/002.log"),
            log_of(30, &["left@5-20"]).to_text(),
        )
        .unwrap();
        let runs = runs(&home, "g");
        assert_eq!(runs.len(), 1);
        // The guard refuses the run's own presses for its game, and not
        // for another game's play.
        {
            use gp_evidence::guard::Guard as _;
            let g = Guard::new(&home);
            let e = g.input("the log", &Of::game("g"), &run, 8).unwrap_err();
            assert!(e.starts_with("refused: the log is sealed"), "{e}");
            assert!(g.input("the log", &Of::game("other"), &run, 8).is_ok());
        }
        assert!(refuse_clip_in(&home, &runs, &session, 1).is_err());
        assert!(refuse_clip_in(&home, &runs, &session, 2).is_ok());
        let o = leaks(&paths, "g", Some(&home.join("no-games"))).unwrap();
        assert_eq!(o.json["sealed_runs"], 1);
        assert_eq!(o.json["checked"]["session_clips"], 2);
        assert_eq!(o.json["leaks"][0]["id"], "walk:001");
        assert_eq!(o.json["leaks"][0]["sealed_run"], "g/run-1.log");
        assert!(!o.text.contains("right"), "{}", o.text);
        assert!(sealed_logs(&home, "g").len() == 1);
        assert!(run_file(&home, "g", "run-1").is_ok());
        assert!(run_file(&home, "g", "../g/run-1").is_err());
    }
}
