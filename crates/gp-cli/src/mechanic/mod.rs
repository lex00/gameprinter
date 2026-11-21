//! `gp mechanic`: one mechanic of one game, worked on by a person and their
//! agent. The person plays the game in the browser, records short clips of
//! the mechanic with a note each and hands them off; the agent reads the
//! session with `gp mechanic show`, answers with `gp mechanic say`, and asks
//! for the next clip.
//!
//! A session is a folder, `$GP_HOME/mechanics/GAME/MECHANIC/`:
//!
//! | File | What it holds |
//! |---|---|
//! | `session.toml` | The game, the mechanic, the person's words about it, and the ROM's SHA-256 |
//! | `clips/NNN.log` | A clip's own inputs (or, written the old way, its input log from power-on) |
//! | `clips/NNN.toml` | Its path and start frame, its frames, the state hash there, who made it and the note |
//! | `talk.ndjson` | The conversation, one line a turn: who, the words, the clips it hands off |
//! | `busy.json` | What the agent said it is doing, until its next turn |
//! | `watch/` | What the agent is doing, for the page's watch panel: its log, pins and problem table ([`watch`]) |
//!
//! Beside the sessions, `$GP_HOME/mechanics/GAME/queue.json` is the game's
//! quest log (seeded by `gp game new` from [`CHECKLISTS`]) and `current`
//! names the mechanic open in the page. Paths, the plays clips start from,
//! are kept once under `$GP_HOME/work/paths/` ([`paths`]).
//!
//! Clips are recipes, never pictures: anyone with the ROM plays them again.
//! `gp mechanic start` serves the page (`mechanic/` in the repo) on this
//! machine only, with the player module and the ROM ([`server`]). An arcade
//! game's logs, paths and clips are arcade input logs (`gp-arcade-input 0`,
//! the MAME oracle's spelling, read by the same `InputLog`), and their
//! starts are replayed on gp-arcade ([`paths::original_at`]).
//!
//! Ported from game-printer's mechanic.rs: the core loop, `probe`
//! ([`probe`], the game's probes.toml), the page's three tabs (the rebuild
//! reviewed beside the original and played live, [`live`]), the store of
//! sealed runs the page writes, and the decision model at the controller
//! ([`demo`]). `video`, `contacts`, `compare` and `clip-save` are
//! [`evidence`]. The corpus import is `gp corpus import`
//! (crate::corpus_import), and sealed runs are scored by `gp sealed`
//! (crate::sealed).

use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context as _, Result, bail};
use clap::Subcommand;
use gp_evidence::Paths;
use serde::Deserialize;
use serde_json::{Value, json};

mod autoplay;
mod demo;
pub(crate) mod detect;
mod evidence;
pub(crate) mod http;
mod import;
pub(crate) mod live;
pub(crate) mod paths;
pub(crate) mod probe;
pub(crate) mod propose;
pub(crate) mod review;
pub(crate) mod scores;
pub(crate) mod server;
pub(crate) mod session;
mod triage;
pub(crate) mod watch;
pub(crate) mod watch_page;

use crate::Outcome;
pub(crate) use session::name_ok;
use session::{Session, current, queue_file, read_queue, show_text, write_queue};

#[derive(Subcommand)]
pub enum MechanicCmd {
    /// Open a mechanic's session and serve its page on 127.0.0.1: the game
    /// to play, record clips of the mechanic in and hand them to your agent.
    /// Makes the session if it is new. Runs until stopped.
    Start {
        /// The game, as its package folder in the games checkout is named.
        game: String,
        /// The mechanic, one word or a few with dashes (jump). By default the
        /// one open last, else the first quest log row not done.
        mechanic: Option<String>,
        /// What the mechanic is, in your words, for a new session.
        #[arg(long)]
        prompt: Option<String>,
        /// gp serve's port (7710; started when none answers there); with
        /// --own-server, the page's own port (0 picks a free one; 7712 by
        /// default).
        #[arg(long)]
        port: Option<u16>,
        /// Serve the page on a server of its own, as before gp serve took
        /// it in, rather than as a tab of gp serve's page.
        #[arg(long)]
        own_server: bool,
        /// The games checkout (default: $GP_GAMES_DIR, else a `games` folder
        /// at or beside the current folder or one above it)
        #[arg(long)]
        games: Option<PathBuf>,
        /// The game's built module (`gp game build`), for reviewing clips on
        /// the model (default: $GP_HOME/work/modules/GAME.wasm if it's there)
        #[arg(long)]
        module: Option<PathBuf>,
        /// The ROM set to play, for a game with several in its roms.toml:
        /// its id (such as `game-rev-b`) or a file of it. Kept for the game, as the
        /// page's set picker keeps it (default: the set chosen last, else
        /// the package's scored set).
        #[arg(long)]
        rom: Option<String>,
        /// Stay on this build. By default the server restarts into
        /// $GP_HOME/bin/gp when a new one is published (by the gate or `gp
        /// self publish`), between requests, on the same port and mechanic.
        #[arg(long)]
        no_follow: bool,
    },
    /// The watch panel on a page of its own, on 127.0.0.1: what your agent is
    /// doing (its log, pins and problem table) and a box for your notes, for
    /// work with no game to play in the mechanic page, such as a new
    /// platform's emulator. Makes the session if it is new. Needs no ROM.
    /// Runs until stopped.
    Watch {
        /// A name for the work's game (its folder's name).
        game: String,
        /// What is being worked on (emulator).
        topic: String,
        /// What the work is, in your words, for a new session.
        #[arg(long)]
        prompt: Option<String>,
        /// The editor's port (`gp serve`, started when none answers there);
        /// with --own-server, the panel's own port (0 picks a free one).
        #[arg(long)]
        port: Option<u16>,
        /// Serve the panel on a server of its own rather than in the editor.
        #[arg(long)]
        own_server: bool,
    },
    /// Say what you're doing while it's your turn ("measuring the jump"):
    /// the person's page shows it until your next turn.
    Busy {
        game: String,
        mechanic: String,
        text: String,
    },
    /// Add a line to the log the person watches beside the game: what you're
    /// doing now. The newest line is your status.
    Log {
        game: String,
        mechanic: String,
        text: String,
    },
    /// Pin a picture or a clip (png, jpg, gif, webp, mp4, webm) for the person
    /// to see beside the game: a sheet of frames, the original against the
    /// model. It's copied into the session. The newest few show; `--keep`
    /// keeps one at the top. `--drop N` takes pin N off instead.
    Pin {
        game: String,
        mechanic: String,
        /// The file to pin.
        #[arg(required_unless_present = "drop")]
        file: Option<PathBuf>,
        /// What it shows, in a line.
        #[arg(long, default_value = "")]
        title: String,
        /// Keep it at the top.
        #[arg(long)]
        keep: bool,
        /// Take pin N off the list.
        #[arg(long, conflicts_with_all = ["file", "keep"])]
        drop: Option<u64>,
    },
    /// Add a row to the problem table the person watches: where, what is
    /// wrong, the change that fixes it and its state (open, working, fixed,
    /// wontfix). With `--n`, changes the fields given of row N.
    Problem {
        game: String,
        mechanic: String,
        /// The row to change.
        #[arg(long)]
        n: Option<u64>,
        /// Where it shows ("25m, the second ladder").
        #[arg(long = "where")]
        place: Option<String>,
        /// What is wrong.
        #[arg(long)]
        what: Option<String>,
        /// The change that fixes it.
        #[arg(long)]
        fix: Option<String>,
        /// open, working, fixed or wontfix (a new row is open).
        #[arg(long)]
        state: Option<String>,
    },
    /// Add a quest-log row the loop found it needs, with no card: status
    /// next, its note "Proposed by the loop: WHAT (from FROM)". A worker
    /// that finds a problem outside its own row proposes it here, with its
    /// test (`--goal`, and the request the triage gives the person), which
    /// goes in the game's triage.toml in the same call so the triage tries
    /// the row first. A row the quest log has (any status) is left as it is.
    Propose {
        game: String,
        /// The new row's name: lower-case letters, digits and dashes.
        mechanic: String,
        /// What is wrong or missing, in one line.
        #[arg(long)]
        what: String,
        /// Where it came from: the command, clip and frame, or the step.
        #[arg(long)]
        from: String,
        /// games (the game's folder, the default) or tools (gameprinter).
        #[arg(long, value_parser = ["games", "tools"])]
        repo: Option<String>,
        /// The row's triage test: a test on RAM that comes to hold when a
        /// clip shows it (triage.toml's goal, `lives == 2`).
        #[arg(long)]
        goal: Option<String>,
        /// Where the hard stretch begins (triage.toml's near).
        #[arg(long, requires = "goal")]
        near: Option<String>,
        /// The script's start (triage.toml's from: power-on, ID, ID@F).
        #[arg(long = "start", requires = "goal")]
        start: Option<String>,
        /// The person's request: where to play.
        #[arg(long = "where", requires = "goal")]
        place: Option<String>,
        /// The person's request: what to do.
        #[arg(long = "do", requires = "goal")]
        todo: Option<String>,
        /// The person's request: when to stop.
        #[arg(long, requires = "goal")]
        stop: Option<String>,
        /// The games checkout (default $GP_GAMES_DIR).
        #[arg(long)]
        games: Option<PathBuf>,
    },
    /// Lock a look the owner said yes to: the row's clip (a corpus clip, or
    /// clip N once it is in the corpus), its frames and look (the game's
    /// own) are measured on the game as it is and written to the game's
    /// approved.toml, each frame's pixels apart from the original. The
    /// model gate then fails a change that draws one of those frames
    /// further apart. Run it on the owner's yes only, and again to move
    /// the lock (docs/gate.md, "Approved looks").
    Approve {
        game: String,
        /// The row whose clip the owner approved.
        mechanic: String,
        /// The game's built module. Needed for a model with hooks.
        #[arg(long)]
        module: Option<PathBuf>,
        /// The games checkout (default $GP_GAMES_DIR).
        #[arg(long)]
        games: Option<PathBuf>,
    },
    /// Wait until it is the agent's turn (the person handed off a clip or
    /// answered), then print the session as `show` does. Run it in the
    /// background after each turn you give the person. Without a mechanic,
    /// waits on the one the person has open, and also returns when they pick
    /// another from the quest log.
    Wait {
        game: String,
        mechanic: Option<String>,
    },
    /// Show what a session holds, for the agent: the mechanic and its clips
    /// with their notes, the conversation and whose turn it is.
    Show { game: String, mechanic: String },
    /// Set what the game's screen page shows (gp serve's
    /// /mechanic/GAME/screen.html: the original and the model) and print
    /// what it shows, so the person sees what you talk about. The parts
    /// left out stay as they are; with none it only reads. The same as the
    /// screen_show tool.
    Screen {
        game: String,
        /// A clip of the mechanic's session to show, stood still at --frame.
        #[arg(long)]
        clip: Option<u64>,
        /// The clip's mechanic; the one open on the game's page by default.
        #[arg(long)]
        mechanic: Option<String>,
        /// The clip's frame, from 0 at its start.
        #[arg(long)]
        frame: Option<u64>,
        /// Back to live play, which the person plays.
        #[arg(long, conflicts_with_all = ["clip", "frame"])]
        live: bool,
        /// `game` for the game's own pictures, or a theme's id.
        #[arg(long)]
        pictures: Option<String>,
        /// side, top, diff, or solo (the model alone).
        #[arg(long)]
        view: Option<String>,
        /// The model's opacity over the original in the top view, 0 to 1.
        #[arg(long)]
        opacity: Option<f64>,
    },
    /// Say something to the person in a session: an answer, what is now
    /// known, a question or the clip you want next. It shows over their game,
    /// which pauses until they answer. With `--choice` they pick one with the
    /// controller. A clip replays under the words when `--show` is given.
    Say {
        game: String,
        mechanic: String,
        text: String,
        /// A choice they can pick (2 to 4 of them).
        #[arg(long = "choice")]
        choices: Vec<String>,
        /// A clip to replay in the window while they answer.
        #[arg(long)]
        show: Option<u64>,
        /// The frame of the shown clip the words are about: the screen page
        /// opens the clip there on both screens.
        #[arg(long, requires = "show")]
        frame: Option<u64>,
        /// A clip to compare the shown one with: the page plays both from
        /// their starts, in step, as an overlay, side by side, or a diff.
        #[arg(long, requires = "show")]
        against: Option<u64>,
        /// Ask the decision model too: the shown clip is replayed and its
        /// jumps measured, and the model picks among the choices from
        /// those numbers. Its pick shows beside the choices, to confirm or
        /// dispute.
        #[arg(long, requires = "show")]
        judge: bool,
        /// Draw the measurements over the shown clip as it plays (its
        /// clips/NNN.measure.json, from `gp mechanic contacts`).
        #[arg(long, requires = "show")]
        measure: bool,
        /// The decision server for `--judge` (Laya; gp serve starts it here when none answers).
        /// Default: $GP_LAYA_URL when it is set (the address gp serve and gp_run
        /// keep Laya at), else http://127.0.0.1:7720.
        #[arg(
            long,
            env = "GP_LAYA_URL",
            hide_env_values = true,
            default_value = "http://127.0.0.1:7720"
        )]
        decide: String,
        /// The games checkout, for `--judge` (default $GP_GAMES_DIR).
        #[arg(long)]
        games: Option<PathBuf>,
        /// A fact worked out from the measurements ("71 px is 5 above the
        /// standing jump's 66"), shown to the person under the words and
        /// given to the model.
        #[arg(long = "fact")]
        facts: Vec<String>,
    },
    /// Make a clip of the mechanic with a decision model at the controller:
    /// from a clip's start in the session, every few frames the model gets
    /// the player's state and the hint and picks the buttons to hold, until
    /// the mechanic shows (for a jump, `--times` jumps up and back down) or
    /// the frames run out. The page shows it as it plays. The clip goes
    /// into the session, made by the model, with each decision's
    /// probabilities beside it. The player's place is read from the game's
    /// state mapping; on an arcade game the model picks among the board's
    /// chords and plays its frames without counting jumps.
    Demo {
        game: String,
        mechanic: String,
        /// What to show, in words the model reads.
        #[arg(long)]
        hint: String,
        /// A Jev-compatible decision server (Laya; gp serve starts it here when none answers).
        /// Default: $GP_LAYA_URL when it is set (the address gp serve and gp_run
        /// keep Laya at), else http://127.0.0.1:7720.
        #[arg(
            long,
            env = "GP_LAYA_URL",
            hide_env_values = true,
            default_value = "http://127.0.0.1:7720"
        )]
        decide: String,
        /// How many times the mechanic should show before the clip ends.
        #[arg(long, default_value_t = 2)]
        times: u32,
        /// One step of the mechanic, in order ("a short hop", "a full-height
        /// jump"): the model is told which step it is on, and each jump
        /// done moves to the next. A step that starts "walk" or "run" is
        /// done when the player moves across at that pace instead. With
        /// steps, `--times` is the count of jump steps.
        #[arg(long = "step")]
        steps: Vec<String>,
        /// The most frames the clip runs after its start.
        #[arg(long, default_value_t = 900)]
        frames: usize,
        /// Frames each decision holds its buttons for.
        #[arg(long, default_value_t = 8)]
        every: usize,
        /// The clip whose start the demo starts from (default the
        /// session's newest).
        #[arg(long)]
        from: Option<u64>,
        /// The games checkout (default $GP_GAMES_DIR).
        #[arg(long)]
        games: Option<PathBuf>,
    },
    /// An old home's mechanic work for GAME, brought into this home: its
    /// sessions (clips, conversation, watch files), the quest log, the
    /// gate's scores and the sealed runs. The old name is --old-name, else
    /// the games checkout's corpus.toml `[import] prefix` (the old source id
    /// its clips were imported under), else GAME. Nothing here is overwritten: a session this
    /// home has but never used takes the old one's place; one with clips or
    /// conversation of its own is kept and listed.
    Import {
        game: String,
        /// The old home (game-printer's was ~/.game-printer).
        #[arg(long)]
        from: PathBuf,
        /// The game's name in the old home.
        #[arg(long)]
        old_name: Option<String>,
        /// The games checkout, for the corpus's import prefix (default: as
        /// `mechanic start` finds it).
        #[arg(long)]
        games: Option<PathBuf>,
        /// List what would be copied, and write nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Run the game's probe recipes, from its package folder's probes.toml.
    /// Each starts from power-on, a stored path or a mechanic clip at a
    /// frame, plays its steps (presses, a walk to a RAM value, presses held
    /// until a RAM test holds), and takes its measures from RAM after each
    /// frame. A sweep plays one cell a value of its variable. Each measure
    /// with an expected value is checked, and a probe that names the clips it
    /// reproduces is compared with them, input for input from power-on.
    /// `--save` writes a cell with no saved clip into its session.
    /// `--import SESSION:CLIP` prints a saved clip as a probe to start from.
    Probe {
        game: String,
        /// Probes to run, by id (default: every one).
        ids: Vec<String>,
        /// A probes file instead of the package folder's probes.toml.
        #[arg(long)]
        file: Option<PathBuf>,
        /// The ROM; by default the one in $GP_ROM_DIR the game's roms.toml
        /// names.
        #[arg(long)]
        rom: Option<PathBuf>,
        /// The games checkout (default: $GP_GAMES_DIR, else a `games` folder
        /// at or beside the current folder or one above it)
        #[arg(long)]
        games: Option<PathBuf>,
        /// Save each cell that has no clip yet into its session.
        #[arg(long)]
        save: bool,
        /// Print the clip SESSION:CLIP as a probe recipe instead.
        #[arg(long, conflicts_with_all = ["save", "ids"])]
        import: Option<String>,
    },
    /// The game's quest log, shown beside the game: each mechanic with its
    /// status (done, asking you, agent working, in line, next) and one plain
    /// line. With `--status`, adds the mechanic or changes it; without,
    /// prints the quest log. A row whose clips are in the corpus takes a
    /// line of scores from the gate's status.json, and keeps its typed
    /// status; the typed note is shown under it.
    Queue {
        game: String,
        mechanic: Option<String>,
        #[arg(long, requires = "mechanic", value_parser = ["done", "asking you", "agent working", "in line", "next"])]
        status: Option<String>,
        /// One plain line: what is known, or what it's waiting on.
        #[arg(long, requires = "status")]
        note: Option<String>,
        /// Who works the row while it is "agent working": a worker's name
        /// (default: $GP_WORKER). The page shows it on the row and keeps
        /// the rows being worked at the top. Another status clears it.
        #[arg(long, requires = "status")]
        worker: Option<String>,
        /// Something measured, exact (repeat for each). Replaces the list.
        #[arg(long, requires = "mechanic")]
        known: Vec<String>,
        /// The agent's reading of a measurement, a name or a cause the
        /// tool didn't find itself: worth a check by eye.
        #[arg(long = "read", requires = "mechanic")]
        reads: Vec<String>,
        /// What isn't found yet, and why.
        #[arg(long = "open", requires = "mechanic")]
        opens: Vec<String>,
        /// Mark the row out of the corpus's scope, with why.
        #[arg(
            long,
            value_name = "WHY",
            requires = "mechanic",
            conflicts_with = "in_scope"
        )]
        out_of_scope: Option<String>,
        /// Take the out-of-scope mark off the row.
        #[arg(long, requires = "mechanic")]
        in_scope: bool,
        /// Where the row's work is: games (the game's folder, the default)
        /// or tools (gameprinter itself). The loop's runner gives the
        /// row's worker a worktree of that repo.
        #[arg(long, requires = "mechanic", value_parser = ["games", "tools"])]
        repo: Option<String>,
        /// The clip that shows what the person is to judge: N (clip N of
        /// this row), ROW/N (clip N of another row) or a corpus clip's id
        /// (tapper.l1.001). The row and its card get a LOOK AT IT button
        /// that sets it up on the screens, the original beside the model.
        #[arg(long, requires = "mechanic", conflicts_with = "no_clip")]
        clip: Option<String>,
        /// The clip's frames in question, from the clip's start: A-B (played
        /// in a loop) or A (stood still there).
        #[arg(long, requires = "clip")]
        frames: Option<String>,
        /// The pictures to judge it in: a theme of the game (last-call), or
        /// game for the game's own.
        #[arg(long, requires = "clip")]
        look: Option<String>,
        /// Take the clip off the row.
        #[arg(long, requires = "mechanic")]
        no_clip: bool,
        /// The row's target, a measure on a line of the game's scoreboard:
        /// LINE state|background|sprites|own at least N (level 1 sprites at
        /// least 330 of 336; the demo state at least 99.5%). gp serve's
        /// scoreboard closes the row when the measure gets there, and
        /// --status done is refused before.
        #[arg(long, requires = "mechanic", conflicts_with_all = ["no_target", "measure_needed"])]
        target: Option<String>,
        /// Take the target off the row (the person's, not a worker's).
        #[arg(long, requires = "mechanic", conflicts_with = "measure_needed")]
        no_target: bool,
        /// For a row with no measure yet: what would measure it (a line
        /// the scoreboard lacks, a clip still to record).
        #[arg(long, value_name = "WORDS", requires = "mechanic")]
        measure_needed: Option<String>,
    },
    /// A sweep of one mechanic: a grid of clips from one spot, each played
    /// on the original and on a rebuilt model in lockstep. `jump` (the
    /// jumpathon) varies the takeoff speed, how long A is held and the
    /// steering in the air, from open flat ground found blind. The grid
    /// says which cells the model gets wrong and what parts first.
    Sweep {
        /// The mechanic to sweep: jump.
        #[arg(value_parser = ["jump"])]
        kind: String,
        game: String,
        /// The model folder to score (model.toml and mapping.toml); by
        /// default the game's package folder.
        #[arg(long)]
        model: Option<PathBuf>,
        /// The ROM; by default the one in $GP_ROM_DIR the package names.
        #[arg(long)]
        rom: Option<PathBuf>,
        /// Leave out writing the cells as clips of a jumpathon session.
        #[arg(long)]
        no_clips: bool,
        /// Sweep along a path through a level instead (an input log from
        /// power-on, such as a farm clip or a blind path). Every place the
        /// player stands, a 16 px column and a height, on the path or
        /// reached by probe moves chained from places already stood on, gets
        /// a standing full jump and a hop from 0, 4, 8 and 12 px into the
        /// column and from the next column's edge, and running jumps forward
        /// and back, with the head hits, the sprites and the code each runs
        /// for the first time. Level x is the x byte and its high byte, found
        /// on the path, checked against the scroll.
        #[arg(long)]
        along: Option<PathBuf>,
        /// Instead of sweeping, fit the jump blind and write a scratch
        /// model: the player's bytes from blind discovery, jumps from the
        /// game's jumpathon clips, its farm runs and its blind paths (never a
        /// recording), a third held out. The model given (or the template) is
        /// copied to this folder with only its jump facts replaced.
        #[arg(long, conflicts_with = "along")]
        fit_blind: Option<PathBuf>,
        /// The game's built module. Needed for a model with hooks.
        #[arg(long)]
        module: Option<PathBuf>,
    },
    /// Mechanics from a long recording in the store (a TAS run): it is
    /// replayed once and every detector runs over it; each kind of event
    /// gets an example clip per level (a session `harvest-LEVEL`) and the
    /// kinds are listed as candidate mechanics, matched to queue rows.
    /// `--add` notes the matches on their rows and adds the rest as "next".
    Harvest {
        game: String,
        recording: String,
        #[arg(long, default_value_t = 25)]
        top: usize,
        #[arg(long)]
        add: bool,
    },
    /// The tools try each quest log row and each lead first, with a
    /// budget, so the queue holds only play the tools can't get: the
    /// corpus's clips, the row's scripted presses, the game's auto-player
    /// with its search (from power-on and from save points), seeded
    /// presses from the row's near point, and the demo with Laya. What a
    /// row has to show is its test in the package's triage.toml. A run
    /// that shows it is saved into the row's session as a clip made by the
    /// tools and the row is marked "made by the tools"; else "needs you",
    /// with why, and a request: a start point (a path and a frame), what
    /// to do and when to stop, which the page's PLAY FROM HERE loads.
    Triage {
        game: String,
        /// Only these rows (default every item of triage.toml).
        rows: Vec<String>,
        /// Seconds the game's auto-player may run before it counts as
        /// hung (four times this from power-on, twice from a save point);
        /// a run it cuts isn't used. No other tool stops on time.
        #[arg(long, default_value_t = 60)]
        budget: u64,
        /// Runs of seeded presses the search makes for a row (the same on
        /// every machine).
        #[arg(long, default_value_t = 300)]
        tries: usize,
        /// How many leads (gp mechanic leads) to triage too.
        #[arg(long, default_value_t = 5)]
        leads: usize,
        /// An autoplay binary to run instead of gp's own (default:
        /// $GP_AUTOPLAY, else `gp mechanic autoplay` from this gp).
        #[arg(long)]
        autoplay: Option<PathBuf>,
        /// Frames autoplay plays from power-on.
        #[arg(long, default_value_t = 12000)]
        autoplay_frames: usize,
        /// The decision server for the demo.
        /// Default: $GP_LAYA_URL when it is set (the address gp serve and gp_run
        /// keep Laya at), else http://127.0.0.1:7720.
        #[arg(
            long,
            env = "GP_LAYA_URL",
            hide_env_values = true,
            default_value = "http://127.0.0.1:7720"
        )]
        decide: String,
        /// The games checkout (default $GP_GAMES_DIR).
        #[arg(long)]
        games: Option<PathBuf>,
        /// Say what would land where, and write nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Runs the game's own auto-player: the game whose romset `--rom ZIP`
    /// is (its identity in a game's roms.toml) names one in its folder's
    /// checks.toml `[autoplay]`, a cargo package gp builds and runs or a
    /// program, and gets the flags as given (`--rom ZIP --help` lists
    /// them). A game without one has none, and this says so. `gp mechanic
    /// triage` runs it.
    #[command(disable_help_flag = true)]
    Autoplay {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Find where the player's head hit something in a clip and what changed
    /// then, read from RAM and sprites. A rise that turns over by 3 px a frame
    /// at once, where gravity turns it 1 px at a time, is a hit. The bytes that
    /// had held still and changed at the hit, and the sprites that appeared
    /// after it, are what the hit did. Uses the ROM dataset's player bytes and
    /// names, or the game's mapping; nothing about any one game. On arcade the
    /// board's work RAM and sprite list.
    Contacts {
        game: String,
        mechanic: String,
        clip: u64,
        /// The ROM set to play, kept for the game (as `gp mechanic start
        /// --rom`).
        #[arg(long)]
        rom: Option<PathBuf>,
    },
    /// Make a clip's video, the evidence a finding cites: the clip's start
    /// is checked against the state hash it was saved with, its inputs from
    /// power-on are stored as a recording, and `gp video` draws its frames
    /// (checking each as it draws) into $GP_HOME/videos.
    Video {
        game: String,
        mechanic: String,
        clip: u64,
        /// The ROM set to play, kept for the game (as `gp mechanic start
        /// --rom`).
        #[arg(long)]
        rom: Option<PathBuf>,
    },
    /// Answer a question that the measurements settle, with no model and no
    /// judgement: the tallest jump of clip NEW against that of clip OLD,
    /// measured from RAM (on an arcade game, whose player doesn't jump, how
    /// far across each took the player). The person sees both clips
    /// together and only checks that the measured jumps are the ones in the
    /// clips.
    Compare {
        game: String,
        mechanic: String,
        new: u64,
        old: u64,
    },
    /// Save a clip in the snapshot format from an input log that runs from
    /// power-on: the frames before `--start` are stored once as a path, the
    /// state there is hashed (and checked on load), and the clip keeps only
    /// its own `--frames` frames. `--as N` writes it over clip N (to convert
    /// a clip saved the old way); else it's the session's next clip.
    ClipSave {
        game: String,
        mechanic: String,
        /// An input log from power-on.
        #[arg(long)]
        log: PathBuf,
        /// The frame the clip starts at.
        #[arg(long)]
        start: usize,
        /// The clip's length in frames; by default the rest of the log.
        #[arg(long)]
        frames: Option<usize>,
        /// A note for the clip.
        #[arg(long, default_value = "")]
        note: String,
        /// Who made it: player or agent.
        #[arg(long, default_value = "agent")]
        by: String,
        /// Write over this clip instead of adding one.
        #[arg(long = "as")]
        replace: Option<u64>,
    },
    /// New mechanics to look at: every clip of every mechanic of GAME is
    /// replayed, and the routines and RAM bytes the ROM dataset says stage
    /// 0's runs reached but no clip did are grouped by the routine (one the
    /// clips run) they branch off from. Each queue row's open items are
    /// leads too. Best first; `--add` puts the top ones in the queue.
    Leads {
        game: String,
        /// How many to print.
        #[arg(long, default_value_t = 10)]
        top: usize,
        /// Add the top leads to the queue as "next".
        #[arg(long)]
        add: bool,
    },
}

pub(crate) fn run(cmd: &MechanicCmd, paths: &Paths) -> Result<Outcome> {
    match cmd {
        MechanicCmd::Start {
            game,
            mechanic,
            prompt,
            port,
            own_server: true,
            games,
            module,
            rom,
            no_follow,
        } => {
            if let Some(r) = rom {
                server::choose_rom(paths, games.as_deref(), game, r)?;
            }
            server::start(
                paths,
                &server::Start {
                    game,
                    mechanic: mechanic.as_deref(),
                    prompt: prompt.as_deref(),
                    port: port.unwrap_or(7712),
                    games: games.as_deref(),
                    module: module.as_deref(),
                    follow: !*no_follow,
                },
            )
        }
        MechanicCmd::Start {
            game,
            mechanic,
            prompt,
            port,
            games,
            rom,
            ..
        } => in_editor(
            paths,
            games.as_deref(),
            port.unwrap_or(crate::serve::DEFAULT_PORT),
            &format!("/api/mechanic/{game}/open"),
            &json!({
                "mechanic": mechanic,
                "prompt": prompt,
                "reload": match rom {
                    Some(r) => {
                        server::choose_rom(paths, games.as_deref(), game, r)?;
                        true
                    }
                    None => false,
                },
            }),
            |v| {
                let m = v["mechanic"].as_str().unwrap_or("").to_owned();
                (
                    format!("mechanic/{game}/?tab=clips"),
                    format!(
                        "{game} / {m} in gp serve's page, its CLIPS tab.\nSession: {}\n{}",
                        v["session"].as_str().unwrap_or(""),
                        server::agent_words(game, &m)
                    ),
                )
            },
        ),
        MechanicCmd::Watch {
            game,
            topic,
            prompt,
            port,
            own_server: true,
        } => watch_page::run(paths, game, topic, prompt.as_deref(), port.unwrap_or(7711)),
        MechanicCmd::Watch {
            game,
            topic,
            prompt,
            port,
            ..
        } => {
            // The session first, with the person's words when it is new;
            // gp serve's watch page opens it.
            Session::create(paths, game, topic, prompt.as_deref(), "")?;
            in_editor(
                paths,
                None,
                port.unwrap_or(crate::serve::DEFAULT_PORT),
                &format!("/watch/{game}/{topic}/api/watch"),
                &Value::Null,
                |_| {
                    (
                        format!("watch/{game}/{topic}/"),
                        format!(
                            "{game} / {topic}: your agent's log, from gp serve. The agent writes here with gp mechanic log, pin and problem {game} {topic}, and reads your notes with gp mechanic show or wait {game} {topic}."
                        ),
                    )
                },
            )
        }
        MechanicCmd::Import {
            game,
            from,
            old_name,
            games,
            dry_run,
        } => import::run(
            paths,
            &import::Import {
                game,
                from,
                old_name: old_name.as_deref(),
                games: games.as_deref(),
                dry_run: *dry_run,
            },
        ),
        MechanicCmd::Busy {
            game,
            mechanic,
            text,
        } => {
            Session::open_or_make(paths, game, mechanic)?.set_busy(text)?;
            Ok(Outcome::ok(json!({}), format!("busy: {text}")))
        }
        MechanicCmd::Log {
            game,
            mechanic,
            text,
        } => {
            let line = watch::log(&Session::open_or_make(paths, game, mechanic)?.dir, text)?;
            Ok(Outcome::ok(
                line.clone(),
                format!("{} {}", line["at"].as_str().unwrap_or(""), text.trim()),
            ))
        }
        MechanicCmd::Pin {
            game,
            mechanic,
            file,
            title,
            keep,
            drop,
        } => {
            let dir = Session::open(paths, game, mechanic)?.dir;
            if let Some(n) = drop {
                watch::unpin(&dir, *n)?;
                return Ok(Outcome::ok(
                    json!({"dropped": n}),
                    format!("pin {n} taken off"),
                ));
            }
            let Some(file) = file else {
                bail!("give a file to pin, or --drop N");
            };
            let row = watch::pin(&dir, file, title, *keep)?;
            Ok(Outcome::ok(
                row.clone(),
                format!("pin {}: {}", row["n"], row["file"].as_str().unwrap_or("")),
            ))
        }
        MechanicCmd::Problem {
            game,
            mechanic,
            n,
            place,
            what,
            fix,
            state,
        } => {
            let row = watch::problem(
                &Session::open(paths, game, mechanic)?.dir,
                &watch::Problem {
                    n: *n,
                    place: place.as_deref(),
                    what: what.as_deref(),
                    fix: fix.as_deref(),
                    state: state.as_deref(),
                },
            )?;
            Ok(Outcome::ok(
                row.clone(),
                format!(
                    "problem {} ({}): {}: {}",
                    row["n"],
                    row["state"].as_str().unwrap_or(""),
                    row["where"].as_str().unwrap_or(""),
                    row["what"].as_str().unwrap_or("")
                ),
            ))
        }
        MechanicCmd::Approve {
            game,
            mechanic,
            module,
            games,
        } => crate::approved::approve(paths, game, mechanic, games.as_deref(), module.as_deref()),
        MechanicCmd::Propose {
            game,
            mechanic,
            what,
            from,
            repo,
            goal,
            near,
            start,
            place,
            todo,
            stop,
            games,
        } => {
            let dir = crate::roms_cmd::games_dir(games.as_deref())
                .and_then(|g| crate::labels::game_folder(&g, game));
            let v = propose::propose(
                paths,
                dir.as_deref(),
                &propose::Proposal {
                    game,
                    row: mechanic,
                    from,
                    what,
                    repo: repo.as_deref(),
                    test: goal.as_deref().map(|goal| propose::Test {
                        goal,
                        near: near.as_deref(),
                        from: start.as_deref(),
                        place: place.as_deref(),
                        todo: todo.as_deref(),
                        stop: stop.as_deref(),
                    }),
                    clips: &[],
                },
            )?;
            let text = propose::words(&v);
            Ok(Outcome::ok(v, text))
        }
        MechanicCmd::Probe {
            game,
            ids,
            file,
            rom,
            games,
            save,
            import,
        } => match import {
            Some(i) => {
                let (s, n) = i
                    .split_once(':')
                    .ok_or_else(|| anyhow::anyhow!("--import is SESSION:CLIP, as jump:3"))?;
                probe::import(paths, game, s, n.parse()?)
            }
            None => probe::run(
                paths,
                &probe::Args {
                    game,
                    ids,
                    file: file.as_deref(),
                    rom: rom.as_deref(),
                    games: games.as_deref(),
                    save: *save,
                },
            ),
        },
        MechanicCmd::Wait { game, mechanic } => wait(paths, game, mechanic.as_deref()),
        MechanicCmd::Screen {
            game,
            clip,
            mechanic,
            frame,
            live,
            pictures,
            view,
            opacity,
        } => {
            let a = gp_mcp::screen::ShowArgs {
                game: game.clone(),
                clip: *clip,
                mechanic: mechanic.clone(),
                frame: *frame,
                live: *live,
                pictures: pictures.clone(),
                view: view.clone(),
                opacity: *opacity,
            };
            let v = gp_mcp::screen::show_at(&paths.home, &a, std::time::Duration::from_secs(4))
                .map_err(anyhow::Error::msg)?;
            let line = v["said"].as_str().unwrap_or("").to_owned();
            Ok(Outcome::ok(v, line))
        }
        MechanicCmd::Show { game, mechanic } => {
            let v = match Session::exists(paths, game, mechanic)? {
                false => match Session::row_state(paths, game, mechanic)? {
                    Some(v) => v,
                    None => Session::open(paths, game, mechanic)?.state()?,
                },
                true => Session::open(paths, game, mechanic)?.state()?,
            };
            Ok(Outcome::ok(v.clone(), show_text(&v)))
        }
        MechanicCmd::Say {
            game,
            mechanic,
            text,
            choices,
            show,
            frame,
            against,
            judge,
            measure,
            decide,
            games,
            facts,
        } => {
            if choices.len() == 1 || choices.len() > 4 {
                bail!("give 2 to 4 choices, or none");
            }
            let s = Session::open(paths, game, mechanic)?;
            for c in show.iter().chain(against) {
                if !s.dir.join(format!("clips/{c:03}.toml")).is_file() {
                    bail!("no clip {c} in this session");
                }
            }
            let mut turn = json!({"by": "agent", "text": text, "choices": choices,
                "show": show, "against": against, "facts": facts});
            if *measure {
                turn["measure"] = json!(true);
            }
            if let Some(f) = frame {
                turn["frame"] = json!(f);
            }
            let mut line = String::new();
            if *judge {
                if choices.is_empty() {
                    bail!("--judge picks among choices: give them with --choice");
                }
                let clip = show.context("--judge needs --show")?;
                let j = demo::judge(
                    paths,
                    &s,
                    game,
                    games.as_deref(),
                    clip,
                    text,
                    choices,
                    facts,
                    decide,
                )?;
                line = format!(
                    " (Laya picks {}, {:.0}% sure)",
                    j["choice"].as_str().unwrap_or("?"),
                    j["confidence"].as_f64().unwrap_or(0.0) * 100.0
                );
                turn["judge"] = j;
            }
            let n = s.turn(turn)?;
            Ok(Outcome::ok(
                json!({"turn": n}),
                format!("said (turn {n}){line}"),
            ))
        }
        MechanicCmd::Demo {
            game,
            mechanic,
            hint,
            decide,
            times,
            steps,
            frames,
            every,
            from,
            games,
        } => demo::demo(
            paths,
            &demo::Demo {
                game,
                mechanic,
                hint,
                decide,
                times: *times,
                steps,
                frames: *frames,
                every: (*every).max(1),
                from: *from,
                games: games.as_deref(),
            },
        ),
        MechanicCmd::Queue {
            game,
            mechanic,
            status,
            note,
            known,
            reads,
            opens,
            out_of_scope,
            in_scope,
            worker,
            repo,
            clip,
            frames,
            look,
            no_clip,
            target,
            no_target,
            measure_needed,
        } => {
            let clip = match clip {
                Some(c) => Some(row_clip(c, frames.as_deref(), look.as_deref())?),
                None => None,
            };
            if *no_target && std::env::var("GP_STEP").is_ok_and(|s| !s.is_empty()) {
                bail!(
                    "a worker doesn't take a row's target off: the measure closes the row. If the target is wrong, set a better one (--target) and say why in --note"
                );
            }
            let games = crate::roms_cmd::games_dir(None);
            // A row's status set from outside the loop's steps while the
            // runner goes is a hand fix: the loop files it (crate::trouble).
            if let (Some(m), Some(st)) = (mechanic.as_deref(), status.as_deref()) {
                crate::trouble::status_by_hand(paths, game, m, st);
            }
            queue(
                paths,
                game,
                mechanic.as_deref(),
                &RowChange {
                    status: status.as_deref(),
                    note: note.as_deref(),
                    worker: worker
                        .clone()
                        .or_else(|| std::env::var("GP_WORKER").ok().filter(|w| !w.is_empty()))
                        .as_deref(),
                    known,
                    reads,
                    opens,
                    out_of_scope: out_of_scope.as_deref(),
                    in_scope: *in_scope,
                    repo: repo.as_deref(),
                    clip: clip.as_ref(),
                    no_clip: *no_clip,
                    target: target.as_deref(),
                    no_target: *no_target,
                    measure_needed: measure_needed.as_deref(),
                    games: games.as_deref(),
                    under_step: std::env::var("GP_STEP").is_ok_and(|s| !s.is_empty()),
                },
            )
        }
        MechanicCmd::Sweep {
            kind: _,
            game,
            model,
            rom,
            no_clips: _,
            along: _,
            fit_blind: Some(out),
            ..
        } => crate::sweep::fit_blind(paths, game, model.as_deref(), rom.as_deref(), out),
        MechanicCmd::Sweep {
            kind: _,
            game,
            model,
            rom,
            no_clips,
            along,
            fit_blind: None,
            module,
        } => crate::sweep::jump(
            paths,
            &crate::sweep::Args {
                game,
                model: model.as_deref(),
                rom: rom.as_deref(),
                clips: !no_clips,
                along: along.as_deref(),
                module: module.as_deref(),
            },
        ),
        MechanicCmd::Harvest {
            game,
            recording,
            top,
            add,
        } => {
            let (v, text) = crate::harvest::harvest(paths, game, recording, *add, *top)?;
            Ok(Outcome::ok(v, text))
        }
        MechanicCmd::Triage {
            game,
            rows,
            budget,
            tries,
            leads,
            autoplay,
            autoplay_frames,
            decide,
            games,
            dry_run,
        } => triage::run(
            paths,
            &triage::Args {
                game,
                games: games.as_deref(),
                rows,
                budget: *budget,
                tries: *tries,
                leads: *leads,
                autoplay: autoplay.as_deref(),
                autoplay_frames: *autoplay_frames,
                decide,
                dry_run: *dry_run,
            },
        ),
        MechanicCmd::Contacts {
            game,
            mechanic,
            clip,
            rom,
        } => evidence::contacts(paths, game, mechanic, *clip, rom.as_deref()),
        MechanicCmd::Video {
            game,
            mechanic,
            clip,
            rom,
        } => evidence::clip_video(paths, game, mechanic, *clip, rom.as_deref()),
        MechanicCmd::Compare {
            game,
            mechanic,
            new,
            old,
        } => evidence::compare(paths, game, mechanic, *new, *old),
        MechanicCmd::ClipSave {
            game,
            mechanic,
            log,
            start,
            frames,
            note,
            by,
            replace,
        } => evidence::clip_save(
            paths,
            &evidence::ClipSave {
                game,
                mechanic,
                log,
                start: *start,
                frames: *frames,
                note,
                by,
                replace: *replace,
            },
        ),
        MechanicCmd::Autoplay { args } => autoplay::run(args),
        MechanicCmd::Leads { game, top, add } => {
            if !name_ok(game) {
                bail!("a game name is lower-case letters, digits and dashes: {game}");
            }
            let games = crate::roms_cmd::games_dir(None)
                .context("no games checkout found: set GP_GAMES_DIR")?;
            let game_dir = crate::labels::game_folder(&games, game)
                .with_context(|| format!("no game {game} in {}", games.display()))?;
            let rom = crate::clip_score::rom_of(paths, &game_dir, None)?;
            let (all, summary) = crate::leads::leads(&paths.home, &rom, game)?;
            let shown: Vec<Value> = all.iter().take(*top).cloned().collect();
            let mut text = format!(
                "{} clips ({} frames) ran {} of {} routines ({} of {} code bytes); stage 0's runs wrote {} RAM bytes, {} of them no clip wrote.\n",
                summary["clips"],
                summary["frames"],
                summary["routines_in_clips"],
                summary["routines"],
                summary["code_bytes_in_clips"],
                summary["code_bytes"],
                summary["ram_written_by_stage0"],
                summary["ram_no_clip_wrote"]
            );
            for (n, l) in shown.iter().enumerate() {
                let _ = writeln!(
                    text,
                    "{}. [{}] {}",
                    n + 1,
                    l["kind"].as_str().unwrap_or(""),
                    l["lead"].as_str().unwrap_or("")
                );
            }
            if *add {
                let f = paths.home.join("mechanics").join(game).join("queue.json");
                let mut q = read_queue(&f);
                let mut k = q.len();
                for l in shown.iter().filter(|l| l["kind"] != json!("open")) {
                    k += 1;
                    let name = format!("lead-{k}");
                    if q.iter().any(|r| r["mechanic"] == json!(name)) {
                        continue;
                    }
                    q.push(json!({"mechanic": name, "status": "next", "note": l["lead"]}));
                }
                fs::write(&f, serde_json::to_string_pretty(&q)?)?;
                text.push_str("added to the queue as next\n");
            }
            Ok(Outcome::ok(
                json!({"summary": summary, "leads": shown}),
                text,
            ))
        }
    }
}

/// `gp mechanic wait`: polls the session until it is the agent's turn.
fn wait(paths: &Paths, game: &str, mechanic: Option<&str>) -> Result<Outcome> {
    let open_now = || -> Result<String> {
        match mechanic {
            Some(m) => Ok(m.to_owned()),
            None => current(paths, game),
        }
    };
    let m0 = open_now()?;
    let s = Session::open(paths, game, &m0)?;
    let from = s.turns()?.len();
    loop {
        let m = open_now()?;
        if m != m0 {
            let v = Session::open(paths, game, &m)?.state()?;
            return Ok(Outcome::ok(v.clone(), show_text(&v)));
        }
        let v = s.state()?;
        let n = v["talk"].as_array().map_or(0, Vec::len);
        if n > from && v["waiting_for"] == "agent" {
            return Ok(Outcome::ok(v.clone(), show_text(&v)));
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
}

/// What `gp mechanic queue` changes on a row.
struct RowChange<'a> {
    status: Option<&'a str>,
    note: Option<&'a str>,
    worker: Option<&'a str>,
    known: &'a [String],
    reads: &'a [String],
    opens: &'a [String],
    out_of_scope: Option<&'a str>,
    in_scope: bool,
    repo: Option<&'a str>,
    /// The clip the person judges on (row_clip's object).
    clip: Option<&'a Value>,
    no_clip: bool,
    /// The row's target (crate::scoreboard::parse_target's words).
    target: Option<&'a str>,
    no_target: bool,
    /// What would measure a row with no target.
    measure_needed: Option<&'a str>,
    /// The games checkout, for the game's scoreboard.
    games: Option<&'a std::path::Path>,
    /// Run as a loop step's (GP_STEP): a row asks the person only about a
    /// look (worker_ask_refused).
    under_step: bool,
}

/// Why a worker may not leave row `r` asking the person, or None: only a
/// look asks the owner (a clip on the row for them to watch, or a card for
/// their YES, which the page shows on its own). A question about tools,
/// routing or how to do the work is the worker's to decide (owner,
/// ask-only-about-looks).
pub(crate) fn worker_ask_refused(r: &Value) -> Option<String> {
    if r["status"] != json!("asking you") || r.get("clip").is_some() {
        return None;
    }
    let note = r["note"].as_str().unwrap_or("");
    if note.to_lowercase().contains("card") || note.contains("YES") {
        return None;
    }
    Some("a worker asks the person only about a look: give the row the clip that shows it (--clip N --frames A-B --look THEME), or put the change on a card and say so in --note. A question about the tools, the routing or how to do the work is yours: pick, say why in --note, and go on (--status next, or carry on in this step)".to_owned())
}

/// A row's clip, as the screen page reads it: `{"n": N}` (with
/// `"mechanic"` for another row's) or `{"id": CORPUS-ID}`, with `from` and
/// `to` (frames from the clip's start) and `look` (a theme, or game).
fn row_clip(clip: &str, frames: Option<&str>, look: Option<&str>) -> Result<Value> {
    let clip = clip.trim();
    let mut v = if let Ok(n) = clip.parse::<u64>() {
        json!({"n": n})
    } else if let Some((m, n)) = clip.split_once('/')
        && let Ok(n) = n.parse::<u64>()
    {
        if !name_ok(m) {
            bail!("--clip ROW/N: {m} isn't a row's name");
        }
        json!({"mechanic": m, "n": n})
    } else if !clip.is_empty() && clip.chars().all(id_char) {
        json!({"id": clip})
    } else {
        bail!("--clip is N, ROW/N or a corpus clip's id (tapper.l1.001): {clip}");
    };
    if let Some(f) = frames {
        let num = |s: &str| s.trim().parse::<u64>().ok();
        let (a, b) = match f.split_once('-') {
            Some((a, b)) => (num(a), num(b)),
            None => (num(f), num(f)),
        };
        let (Some(a), Some(b)) = (a, b) else {
            bail!("--frames is A-B or A, frames from the clip's start: {f}");
        };
        if b < a {
            bail!("--frames {f}: the last frame is before the first");
        }
        v["from"] = json!(a);
        v["to"] = json!(b);
    }
    if let Some(l) = look.map(str::trim).filter(|l| !l.is_empty()) {
        if !name_ok(l) {
            bail!("--look is a theme's name or game: {l}");
        }
        v["look"] = json!(l);
    }
    Ok(v)
}

/// A letter of a corpus clip's id: tapper.l1.001.
fn id_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || "._-".contains(c)
}

/// A row's clip in words: "clip 1 of get-ready-screen, frames 200 to 230,
/// in last-call".
fn clip_words(cl: &Value) -> String {
    let mut s = match (cl["id"].as_str(), cl["n"].as_u64()) {
        (Some(id), _) => format!("corpus clip {id}"),
        (None, Some(n)) => match cl["mechanic"].as_str() {
            Some(m) => format!("clip {n} of {m}"),
            None => format!("clip {n}"),
        },
        _ => "a clip".to_owned(),
    };
    match (cl["from"].as_u64(), cl["to"].as_u64()) {
        (Some(a), Some(b)) if a == b => s.push_str(&format!(", frame {a}")),
        (Some(a), Some(b)) => s.push_str(&format!(", frames {a} to {b}")),
        _ => {}
    }
    if let Some(l) = cl["look"].as_str() {
        s.push_str(&format!(", in {l}"));
    }
    s
}

fn queue(paths: &Paths, game: &str, mechanic: Option<&str>, c: &RowChange) -> Result<Outcome> {
    let f = queue_file(paths, game)?;
    let mut q = read_queue(&f);
    let mut hint: Option<String> = None;
    let board = c
        .games
        .and_then(|g| crate::scoreboard::board(&paths.home, g, game));
    if let Some(t) = c.target {
        crate::scoreboard::check_target(board.as_ref(), game, t).map_err(anyhow::Error::msg)?;
    }
    if let Some(m) = mechanic
        && (c.status.is_some()
            || !c.known.is_empty()
            || !c.reads.is_empty()
            || !c.opens.is_empty()
            || c.out_of_scope.is_some()
            || c.repo.is_some()
            || c.clip.is_some()
            || c.no_clip
            || c.in_scope
            || c.target.is_some()
            || c.no_target
            || c.measure_needed.is_some())
    {
        if !name_ok(m) {
            bail!("a mechanic name is lower-case letters, digits and dashes: {m}");
        }
        if !q.iter().any(|r| r["mechanic"] == json!(m)) {
            if c.status.is_none() {
                bail!("{m} isn't in the quest log: give it a --status");
            }
            q.push(json!({"mechanic": m}));
        }
        let r = q
            .iter_mut()
            .find(|r| r["mechanic"] == json!(m))
            .expect("just added");
        if let Some(st) = c.status {
            // A row the person wrote on the page keeps their words (asked)
            // when the agent's note takes its place, so the page can say
            // who wrote which.
            if r["by"] == json!("person")
                && r.get("asked").is_none()
                && let Some(o) = r.as_object_mut()
            {
                let said = o.get("note").cloned().unwrap_or(json!(""));
                o.insert("asked".into(), said);
            }
            r["status"] = json!(st);
            r["note"] = json!(c.note.unwrap_or_default());
            if let Some(o) = r.as_object_mut() {
                match c.worker.filter(|_| st == "agent working") {
                    Some(w) => o.insert("worker".into(), json!(w)),
                    None => o.remove("worker"),
                };
            }
        }
        for (k, v) in [("known", c.known), ("read", c.reads), ("open", c.opens)] {
            if !v.is_empty() {
                r[k] = json!(v);
            }
        }
        if let Some(o) = r.as_object_mut() {
            match c.repo {
                Some("tools") => {
                    o.insert("repo".into(), json!("tools"));
                }
                Some(_) => {
                    o.remove("repo");
                }
                None => {}
            }
            if let Some(why) = c.out_of_scope {
                o.insert("scope".into(), json!("out"));
                o.insert("scope_why".into(), json!(why));
            }
            if c.in_scope {
                o.remove("scope");
                o.remove("scope_why");
            }
            if let Some(cl) = c.clip {
                o.insert("clip".into(), cl.clone());
            }
            if c.no_clip {
                o.remove("clip");
            }
            if let Some(t) = c.target {
                o.insert("target".into(), json!(t.trim()));
                o.remove("measure_needed");
            }
            if c.no_target {
                o.remove("target");
                o.remove("awaiting_measure");
            }
            if let Some(w) = c.measure_needed {
                o.insert("measure_needed".into(), json!(w.trim()));
            }
        }
        // A row with a target is done only when the measure gets there: a
        // worker's word doesn't close it.
        let r = q
            .iter()
            .find(|r| r["mechanic"] == json!(m))
            .expect("just changed");
        if c.status == Some("done")
            && let Some(t) = r["target"].as_str()
        {
            crate::scoreboard::may_close(board.as_ref(), t)
                .map_err(|e| anyhow::anyhow!("{m}: {e}"))?;
        }
        if r["status"] != json!("done")
            && r.get("target").is_none()
            && r.get("measure_needed").is_none()
        {
            hint = Some(format!(
                "{m} has no target: give it one on a line of the scoreboard (--target \"LINE sprites at least N of M\"), or say what would measure it (--measure-needed WORDS)"
            ));
        }
        // A question for the person with nothing to look at: say how to
        // give it one, since the page sets the clip up from the row.
        let r = q
            .iter()
            .find(|r| r["mechanic"] == json!(m))
            .expect("just changed");
        if c.under_step
            && let Some(why) = worker_ask_refused(r)
        {
            bail!("{m}: {why}");
        }
        if r["status"] == json!("asking you") && r.get("clip").is_none() {
            hint = Some(format!(
                "{m} asks the person without a clip: when they are to judge something they can see, give the row the clip that shows it (--clip N --frames A-B --look THEME), and the page sets it up on the screens"
            ));
        }
        write_queue(&f, &q)?;
    }
    let mut q = scores::scored_queue(&paths.home, game);
    // The target as this call's games checkout measures it.
    if c.games.is_some() {
        for r in &mut q {
            if let Some(t) = r["target"].as_str() {
                r["target_says"] = json!(crate::scoreboard::target_says(board.as_ref(), t));
            }
        }
    }
    let text = q
        .iter()
        .map(|r| {
            let mut l = format!(
                "{}: {}{}.",
                r["mechanic"].as_str().unwrap_or(""),
                r["status"].as_str().unwrap_or(""),
                r["worker"]
                    .as_str()
                    .map(|w| format!(" ({w})"))
                    .unwrap_or_default()
            );
            for k in ["scores", "note"] {
                if let Some(t) = r[k].as_str().filter(|t| !t.is_empty()) {
                    l.push_str(if k == "scores" || r["scores"].is_null() {
                        " "
                    } else {
                        "\n    "
                    });
                    l.push_str(t);
                }
            }
            if r["scope"] == json!("out") {
                l.push_str(&format!(
                    " (out of scope: {})",
                    r["scope_why"].as_str().unwrap_or("")
                ));
            }
            if let Some(cl) = r.get("clip") {
                l.push_str(&format!("\n    to judge on: {}", clip_words(cl)));
            }
            if let Some(t) = r["target_says"].as_str() {
                l.push_str(&format!("\n    target: {t}"));
            } else if let Some(w) = r["measure_needed"].as_str() {
                l.push_str(&format!("\n    no measure yet; what would measure it: {w}"));
            }
            l
        })
        .collect::<Vec<_>>()
        .join("\n");
    match hint {
        Some(h) => Ok(Outcome::ok(
            json!({"queue": q, "hint": h}),
            format!("{text}\nhint: {h}"),
        )),
        None => Ok(Outcome::ok(json!({"queue": q}), text)),
    }
}

/// Each genre's quest log rows, in order (checklists.toml).
/// Opens a view of the editor (`gp serve` on `port`, started when none
/// answers): asks `path` (a POST with `body`, else a GET), and says the
/// view's address and `words` from the answer.
fn in_editor(
    paths: &Paths,
    games: Option<&std::path::Path>,
    port: u16,
    path: &str,
    body: &Value,
    words: impl FnOnce(&Value) -> (String, String),
) -> Result<Outcome> {
    crate::serve::ensure(paths, games, port)?;
    let (status, text) = if body.is_null() {
        crate::serve::call(port, "GET", path, "")?
    } else {
        crate::serve::call(port, "POST", path, &body.to_string())?
    };
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if status != 200 {
        anyhow::bail!(
            "the editor said {status}: {}",
            v["error"].as_str().unwrap_or(&text)
        );
    }
    let (query, said) = words(&v);
    let url = format!("http://127.0.0.1:{port}/{query}");
    Ok(Outcome::ok(
        json!({"url": url, "answer": v}),
        format!("{url}\n{said}"),
    ))
}

pub(crate) const CHECKLISTS: &str = include_str!("checklists.toml");

/// One row of a genre's checklist.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub(crate) struct Row {
    pub row: String,
    pub what: String,
}

/// A genre's checklist from [`CHECKLISTS`].
pub(crate) fn checklist(genre: &str) -> Result<Vec<Row>> {
    let mut all: std::collections::BTreeMap<String, Vec<Row>> = toml::from_str(CHECKLISTS)?;
    match all.remove(genre) {
        Some(rows) => Ok(rows),
        None => bail!("no quest log checklist for the genre {genre}"),
    }
}

/// Seeds a game's quest log from its genre's checklist: each row it lacks
/// is added as "next" with its line and what would measure it (a line of
/// the game's scoreboard, once a clip shows it), and each row gets a session if it has
/// none. Rows already there are left as they are. Returns the quest log's
/// folder, the rows and how many were new.
pub(crate) fn seed_quest_log(
    paths: &Paths,
    game: &str,
    genre: &str,
    rom_sha: &str,
) -> Result<(PathBuf, Vec<Row>, usize)> {
    let rows = checklist(genre)?;
    let f = queue_file(paths, game)?;
    let mut q: Vec<Value> = read_queue(&f);
    let mut added = 0;
    for r in &rows {
        if !q.iter().any(|x| x["mechanic"] == json!(r.row)) {
            // No line can measure it before the first clip: the row says
            // what would, so it isn't closed on a worker's word alone.
            q.push(json!({
                "mechanic": r.row, "status": "next", "note": r.what,
                "measure_needed": format!(
                    "a line of {game}/{} on the first corpus clip that shows {}",
                    crate::scoreboard::FILE,
                    r.row
                ),
            }));
            added += 1;
        }
        Session::create(paths, game, &r.row, Some(&r.what), rom_sha)?;
    }
    write_queue(&f, &q)?;
    let dir = f.parent().map(PathBuf::from).unwrap_or_default();
    Ok((dir, rows, added))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_paths as paths;

    /// Every genre `gp game new` takes has a checklist of plain rows.
    #[test]
    fn every_genre_has_a_checklist() {
        let all: std::collections::BTreeMap<String, Vec<Row>> = toml::from_str(CHECKLISTS).unwrap();
        assert_eq!(
            all.keys().map(String::as_str).collect::<Vec<_>>(),
            {
                let mut g = crate::game_new::GENRES.to_vec();
                g.sort_unstable();
                g
            },
            "checklists.toml's genres are gp game new's"
        );
        for (g, rows) in &all {
            assert!(rows.len() >= 5, "{g}: {rows:?}");
            assert_eq!(rows[0].row, "title", "{g}");
            let mut seen = std::collections::BTreeSet::new();
            for r in rows {
                assert!(name_ok(&r.row), "{g}: {}", r.row);
                assert!(!r.what.is_empty(), "{g}: {}", r.row);
                assert!(seen.insert(&r.row), "{g}: {} twice", r.row);
            }
        }
        assert!(checklist("racing").is_err());
    }

    /// Seeding writes each row as next with a session, and seeding again
    /// adds nothing and keeps what changed.
    #[test]
    fn the_quest_log_is_seeded_once() {
        let d = tempfile::tempdir().unwrap();
        let p = paths(d.path());
        let (dir, rows, added) = seed_quest_log(&p, "test-hop", "platformer", "ab").unwrap();
        assert_eq!(added, rows.len());
        let q = read_queue(&dir.join("queue.json"));
        assert_eq!(q.len(), rows.len());
        assert_eq!(q[2]["mechanic"], "jump");
        assert_eq!(q[2]["status"], "next");
        assert!(
            q.iter().all(|r| r["measure_needed"]
                .as_str()
                .is_some_and(|w| w.contains("test-hop/scoreboard.toml"))),
            "every seeded row says what would measure it: {q:?}"
        );
        let s = Session::open(&p, "test-hop", "jump").unwrap();
        let v = s.state().unwrap();
        assert_eq!(v["prompt"], rows[2].what);
        assert_eq!(v["rom"], "ab");
        let mut q = q;
        q[2]["status"] = json!("done");
        write_queue(&dir.join("queue.json"), &q).unwrap();
        let (_, _, added) = seed_quest_log(&p, "test-hop", "platformer", "ab").unwrap();
        assert_eq!(added, 0);
        assert_eq!(read_queue(&dir.join("queue.json"))[2]["status"], "done");
    }

    /// A quest-log row with no session shows its note and writes nothing;
    /// a name that is neither a row nor a session is still refused.
    #[test]
    fn a_row_with_no_session_shows_its_note() {
        let d = tempfile::tempdir().unwrap();
        let p = paths(d.path());
        let dir = p.home.join("mechanics").join("test-hop");
        std::fs::create_dir_all(&dir).unwrap();
        write_queue(
            &dir.join("queue.json"),
            &[json!({"mechanic": "trouble-x", "status": "next", "note": "Proposed by the loop: x"})],
        )
        .unwrap();
        let v = Session::row_state(&p, "test-hop", "trouble-x")
            .unwrap()
            .unwrap();
        assert_eq!(v["prompt"], "Proposed by the loop: x");
        assert_eq!(v["waiting_for"], "agent");
        assert!(show_text(&v).contains("test-hop / trouble-x: Proposed by the loop: x"));
        assert!(!Session::exists(&p, "test-hop", "trouble-x").unwrap());
        assert!(
            Session::row_state(&p, "test-hop", "nothing")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn the_queue_command_adds_and_changes_rows() {
        let d = tempfile::tempdir().unwrap();
        let p = paths(d.path());
        let none: &[String] = &[];
        let plain = RowChange {
            status: None,
            note: None,
            worker: None,
            known: none,
            reads: none,
            opens: none,
            out_of_scope: None,
            in_scope: false,
            repo: None,
            clip: None,
            no_clip: false,
            target: None,
            no_target: false,
            measure_needed: None,
            games: None,
            under_step: false,
        };
        assert!(
            queue(&p, "test-hop", Some("hop"), &plain)
                .unwrap()
                .text
                .is_empty()
        );
        let known = vec!["66 px high".to_owned()];
        let o = queue(
            &p,
            "test-hop",
            Some("hop"),
            &RowChange {
                status: Some("agent working"),
                note: Some("the hop's height"),
                known: &known,
                ..plain
            },
        )
        .unwrap();
        assert_eq!(o.json["queue"][0]["known"][0], "66 px high");
        assert!(o.text.contains("hop: agent working."), "{}", o.text);
        assert!(
            queue(
                &p,
                "test-hop",
                Some("other"),
                &RowChange {
                    known: &known,
                    ..plain
                }
            )
            .is_err()
        );
        // A row the person wrote keeps their words once the agent's note
        // replaces them, and keeps the first words on later notes.
        let f = queue_file(&p, "test-hop").unwrap();
        let mut q = read_queue(&f);
        q.push(json!({"mechanic": "mine", "status": "next", "note": "the duck is slow", "by": "person"}));
        write_queue(&f, &q).unwrap();
        for note in ["measured the duck", "the duck is 2 frames late"] {
            let change = RowChange {
                status: Some("agent working"),
                note: Some(note),
                ..plain
            };
            queue(&p, "test-hop", Some("mine"), &change).unwrap();
        }
        let r = read_queue(&f)
            .into_iter()
            .find(|r| r["mechanic"] == "mine")
            .unwrap();
        assert_eq!(r["asked"], "the duck is slow");
        assert_eq!(r["note"], "the duck is 2 frames late");
        assert!(read_queue(&f)[0].get("asked").is_none());
    }

    /// A row's target is a measure on a scoreboard line: a target the
    /// board can't measure is refused, done is refused while the measure
    /// is short, and the board's measure closes the row once it gets there;
    /// a landing waits in line and goes back to next if the measure after
    /// it falls short.
    #[test]
    fn a_row_with_a_target_closes_on_its_measure() {
        let d = tempfile::tempdir().unwrap();
        let p = paths(d.path());
        let games = tempfile::tempdir().unwrap();
        let put = |f: &std::path::Path, s: &str| {
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, s).unwrap();
        };
        let g = games.path().join("test-hop");
        put(&g.join("model.toml"), "# a model\n");
        put(
            &g.join("corpus.toml"),
            "[corpus]\nformat = \"1.0.0\"\n\n[[clip]]\nid = \"hop.l1.001\"\nframes = 336\nlevel = \"field\"\n",
        );
        put(
            &g.join("scoreboard.toml"),
            "[[line]]\nname = \"Level 1\"\nclip = \"hop.l1.001\"\n",
        );
        // The gp that measures, as the board keys its lines.
        put(&p.home.join("bin/gp"), "a gp");
        let none: &[String] = &[];
        let plain = RowChange {
            status: None,
            note: None,
            worker: None,
            known: none,
            reads: none,
            opens: none,
            out_of_scope: None,
            in_scope: false,
            repo: None,
            clip: None,
            no_clip: false,
            target: None,
            no_target: false,
            measure_needed: None,
            games: Some(games.path()),
            under_step: false,
        };
        // A line the board lacks, and words that aren't a measure.
        for t in ["level 9 sprites at least 1", "make the mugs nicer"] {
            let c = RowChange {
                status: Some("next"),
                target: Some(t),
                ..plain
            };
            assert!(queue(&p, "test-hop", Some("mugs"), &c).is_err(), "{t}");
        }
        // No measure yet: done is refused, and the row says so.
        let target = "level 1 sprites at least 330 of 336";
        let set = RowChange {
            status: Some("agent working"),
            note: Some("the mugs' frames"),
            target: Some(target),
            ..plain
        };
        let o = queue(&p, "test-hop", Some("mugs"), &set).unwrap();
        assert!(
            o.text
                .contains(&format!("target: {target}: not measured yet")),
            "{}",
            o.text
        );
        let done = RowChange {
            status: Some("done"),
            note: Some("I fixed it"),
            ..plain
        };
        let Err(e) = queue(&p, "test-hop", Some("mugs"), &done) else {
            panic!("done before the target was met");
        };
        let e = e.to_string();
        assert!(e.contains("closes itself"), "{e}");
        // Measured short: still refused, and close_rows leaves it.
        let key = |home: &std::path::Path| {
            crate::scoreboard::key_of(home, &g, "test-hop", "hop.l1.001", &home.join("bin/gp"))
        };
        let measured = |n: u64| crate::scoreboard::Numbers {
            background: [336, 336],
            sprites: [n, 336],
            state: None,
            own: None,
        };
        crate::scoreboard::record(
            &p.home,
            "test-hop",
            "hop.l1.001",
            &key(&p.home),
            Ok(measured(322)),
        )
        .unwrap();
        assert!(queue(&p, "test-hop", Some("mugs"), &done).is_err());
        assert!(crate::scoreboard::close_rows(&p.home, games.path(), "test-hop").is_empty());
        // A landing waits in line; the measure after it falls short: next.
        let f = queue_file(&p, "test-hop").unwrap();
        let mut q = read_queue(&f);
        q[0]["status"] = json!("in line");
        q[0]["awaiting_measure"] = json!(1);
        write_queue(&f, &q).unwrap();
        crate::scoreboard::record(
            &p.home,
            "test-hop",
            "hop.l1.001",
            &key(&p.home),
            Ok(measured(325)),
        )
        .unwrap();
        assert_eq!(
            crate::scoreboard::close_rows(&p.home, games.path(), "test-hop"),
            vec!["mugs".to_owned()]
        );
        let r = &read_queue(&f)[0];
        assert_eq!(r["status"], "next");
        assert!(r.get("awaiting_measure").is_none());
        assert!(
            r["note"]
                .as_str()
                .unwrap()
                .contains("now sprites 325 of 336"),
            "{r}"
        );
        // Measured at the target: the board closes it.
        crate::scoreboard::record(
            &p.home,
            "test-hop",
            "hop.l1.001",
            &key(&p.home),
            Ok(measured(331)),
        )
        .unwrap();
        assert_eq!(
            crate::scoreboard::close_rows(&p.home, games.path(), "test-hop"),
            vec!["mugs".to_owned()]
        );
        let r = &read_queue(&f)[0];
        assert_eq!(r["status"], "done");
        assert!(
            r["note"].as_str().unwrap().starts_with("Met its target"),
            "{r}"
        );
        // A row with no target says what would measure it.
        let o = queue(
            &p,
            "test-hop",
            Some("feel"),
            &RowChange {
                status: Some("next"),
                ..plain
            },
        )
        .unwrap();
        assert!(
            o.json["hint"]
                .as_str()
                .unwrap()
                .contains("--measure-needed")
        );
        let o = queue(
            &p,
            "test-hop",
            Some("feel"),
            &RowChange {
                measure_needed: Some("a scoreboard line for the jump"),
                ..plain
            },
        )
        .unwrap();
        assert!(
            o.text
                .contains("what would measure it: a scoreboard line for the jump"),
            "{}",
            o.text
        );
    }

    /// A row asking the person carries the clip they judge on, which the
    /// page sets up; one without a clip gets a hint, and --no-clip takes it
    /// off.
    #[test]
    fn a_row_carries_the_clip_to_judge_on() {
        assert_eq!(
            row_clip("3", Some("200-230"), Some("last-call")).unwrap(),
            json!({"n": 3, "from": 200, "to": 230, "look": "last-call"})
        );
        assert_eq!(
            row_clip("get-ready-screen/1", Some("40"), None).unwrap(),
            json!({"mechanic": "get-ready-screen", "n": 1, "from": 40, "to": 40})
        );
        let id = row_clip("tapper.l1.001", None, None).unwrap();
        assert_eq!(id, json!({"id": "tapper.l1.001"}));
        assert!(row_clip("", None, None).is_err());
        assert!(row_clip("a b", None, None).is_err());
        assert!(row_clip("2", Some("9-3"), None).is_err());
        assert!(row_clip("2", Some("x"), None).is_err());
        let other = row_clip("get-ready-screen/1", Some("200-230"), Some("last-call")).unwrap();
        let words = "clip 1 of get-ready-screen, frames 200 to 230, in last-call";
        assert_eq!(clip_words(&other), words);
        assert_eq!(clip_words(&id), "corpus clip tapper.l1.001");
        let d = tempfile::tempdir().unwrap();
        let p = paths(d.path());
        let none: &[String] = &[];
        let plain = RowChange {
            status: None,
            note: None,
            worker: None,
            known: none,
            reads: none,
            opens: none,
            out_of_scope: None,
            in_scope: false,
            repo: None,
            clip: None,
            no_clip: false,
            target: None,
            no_target: false,
            measure_needed: None,
            games: None,
            under_step: false,
        };
        let asking = RowChange {
            status: Some("asking you"),
            note: Some("does the mug look right?"),
            ..plain
        };
        let o = queue(&p, "test-hop", Some("mug"), &asking).unwrap();
        assert!(
            o.json["hint"].as_str().unwrap().contains("--clip"),
            "{}",
            o.json
        );
        // Under a step, only a look asks the person: a clip or a card.
        let tools_q = json!({"status": "asking you", "note": "Should this row move to the tool's quest log?"});
        assert!(worker_ask_refused(&tools_q).is_some());
        for ok in [
            json!({"status": "asking you", "note": "Does the mug look right?", "clip": {"n": 1}}),
            json!({"status": "asking you", "note": "Its card is on your page: press YES"}),
            json!({"status": "next", "note": "Picked the tool's log?"}),
        ] {
            assert_eq!(worker_ask_refused(&ok), None, "{ok}");
        }
        let by_worker = RowChange {
            note: Some("Should this row move to the tool's quest log?"),
            under_step: true,
            ..asking
        };
        assert!(queue(&p, "test-hop", Some("tool-q"), &by_worker).is_err());
        let cl = row_clip("1", Some("10-20"), Some("game")).unwrap();
        let with_clip = RowChange {
            clip: Some(&cl),
            ..asking
        };
        let o = queue(&p, "test-hop", Some("mug"), &with_clip).unwrap();
        assert!(
            !o.json["hint"].as_str().unwrap_or("").contains("--clip"),
            "{}",
            o.json
        );
        assert_eq!(o.json["queue"][0]["clip"], cl);
        let words = "to judge on: clip 1, frames 10 to 20, in game";
        assert!(o.text.contains(words), "{}", o.text);
        // A later note keeps the clip; --no-clip takes it off.
        let again = RowChange {
            note: Some("again?"),
            ..asking
        };
        let o = queue(&p, "test-hop", Some("mug"), &again).unwrap();
        assert_eq!(o.json["queue"][0]["clip"], cl);
        let off = RowChange {
            no_clip: true,
            ..plain
        };
        let o = queue(&p, "test-hop", Some("mug"), &off).unwrap();
        assert!(o.json["queue"][0].get("clip").is_none());
    }
}
