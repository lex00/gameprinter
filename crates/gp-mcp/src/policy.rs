//! What `gp_run` may run, as one table: every gp command by its path, the
//! road it belongs to and the rule that governs running it from a tool.
//! gp-cli's tests walk the whole command tree and fail when a command has
//! no row here or a row names no command, so a new command can't reach an
//! agent unclassified. Each row was set by reading what the command writes
//! (the audit's slice files have one entry per command), not by its name.
//!
//! The rules, from the owner's standing rules:
//! - read: writes nothing outside the home's work and cache areas.
//! - write: writes the person's games, working copies or home. A write
//!   that changes or removes a file already in the games checkout (more
//!   than adding lines to it) is put back and held for the person's yes
//!   (crate::held), unless the command is one of [`SELF_GUARDED`].
//! - ask: publishes, uploads, pushes or lands a change through a gate;
//!   spends money or reads a secret or answer key; or downloads a lot.
//!   It runs only with the person's yes, unless one of its dry-run flags
//!   is given: their YES on the page's card (crate::asks) while gp serve
//!   runs for the home, else the call's `confirmed_by_person`. Starting
//!   paid agents (`capture background --agents N` above 0) spends.
//! - refuse: serves until stopped, waits with no bound, opens another
//!   program for the person, or runs any program it is given.
//!
//! A command missing from the table (only possible with a gp other than
//! the one the tests ran on) gets a rule from its words: anything that
//! sounds like publishing, fetching, generating, a gate or a server needs
//! the person's yes or is refused.

use serde_json::{Value, json};

/// Where a command sits, from PRODUCT-REVIEW.md section 3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Road {
    /// On a person's road: porting, a theme, a new game.
    Journey,
    /// A stage of the port the tools run without a person.
    Stage,
    /// Developer and maintenance tools: kept, not for a person's road.
    Developer,
    /// Off the target: kept for now, likely to go.
    OffTarget,
}

impl Road {
    pub fn name(self) -> &'static str {
        match self {
            Road::Journey => "journey",
            Road::Stage => "port stage",
            Road::Developer => "developer",
            Road::OffTarget => "off target",
        }
    }

    /// The words an agent sees beside a command it shouldn't steer a
    /// person to.
    pub fn mark(self) -> Option<&'static str> {
        match self {
            Road::Developer => Some("developer tool: not for a person's journey"),
            Road::OffTarget => Some("off the target: kept for now, not for a person's journey"),
            _ => None,
        }
    }
}

/// Why a command needs the person's yes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ask {
    /// Publishes, uploads, pushes, or lands a change through a gate.
    Publish,
    /// Spends money, calls a paid service, or reads a secret or answer key.
    Spend,
    /// Downloads a lot.
    Download,
    /// Changes or removes data already in the person's games.
    Replace,
}

impl Ask {
    pub fn name(self) -> &'static str {
        match self {
            Ask::Publish => "publishes",
            Ask::Spend => "spends",
            Ask::Download => "downloads",
            Ask::Replace => "replaces",
        }
    }

    /// What the agent says to the person before asking again.
    pub fn before(self) -> &'static str {
        match self {
            Ask::Publish => {
                "it makes something public or lands a change others get. Tell the person exactly what it will publish or land and where, and ask them"
            }
            Ask::Spend => {
                "it spends money, calls a paid service or reads a secret. Give the person the cost estimate first (run it with its dry-run flag, which prints one where it exists) and ask them"
            }
            Ask::Download => {
                "it downloads a lot. Tell the person what it downloads and about how large it is, and ask them"
            }
            Ask::Replace => {
                "it changes or removes data already in the person's games. Tell the person exactly what it changes (the plan in this reply lists it) and ask them"
            }
        }
    }
}

/// What a rule falls back to when its condition isn't met.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Base {
    Read,
    Write,
}

/// The rule for one command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    /// Writes nothing outside the home's work and cache areas.
    Read,
    /// Writes the person's games, working copies or home.
    Write,
    /// Needs the person's yes, unless one of the flags (a dry run) is given.
    Ask(Ask, &'static [&'static str]),
    /// Needs the person's yes when one of the arguments is given; else
    /// the base.
    AskWith(Ask, &'static [&'static str], Base),
    /// Refused; the text says what to use instead.
    Refuse(&'static str),
    /// Refused when the argument is given; else the base.
    RefuseWith(&'static str, &'static str, Base),
    /// Refused unless one of the flags is given; else the base.
    RefuseUnless(&'static [&'static str], &'static str, Base),
}

impl Rule {
    /// The rule's short name, as `gp_commands` lists it.
    pub fn name(self) -> &'static str {
        match self {
            Rule::Read => "read",
            Rule::Write => "write",
            Rule::Ask(..) => "ask",
            Rule::AskWith(_, _, Base::Read) => "read; ask with an argument",
            Rule::AskWith(_, _, Base::Write) => "write; ask with an argument",
            Rule::Refuse(_) => "refused",
            Rule::RefuseWith(_, _, Base::Read) => "read; refused with an argument",
            Rule::RefuseWith(_, _, Base::Write) => "write; refused with an argument",
            Rule::RefuseUnless(_, _, Base::Read) => "read; refused without an argument",
            Rule::RefuseUnless(_, _, Base::Write) => "write; refused without an argument",
        }
    }

    /// Its row in docs/mcp.md's table of rules ("Every command"): `ask` and
    /// `refused` take the rules that ask or refuse only with an argument.
    pub fn kind(self) -> &'static str {
        match self {
            Rule::Read => "read",
            Rule::Write => "write",
            Rule::Ask(..) | Rule::AskWith(..) => "ask",
            Rule::Refuse(_) | Rule::RefuseWith(..) | Rule::RefuseUnless(..) => "refused",
        }
    }

    /// The rule in a sentence, for one command's page.
    pub fn words(self) -> String {
        match self {
            Rule::Read => "Reads: writes nothing outside the home's work and cache areas. Runs at once.".into(),
            Rule::Write => "Writes the person's games, copies or home. Runs at once; the reply says what it wrote when the command reports it.".into(),
            Rule::Ask(a, []) => format!("Needs the person's yes: it {}. Runs only with confirmed_by_person.", a.name()),
            Rule::Ask(a, free) => format!(
                "Needs the person's yes: it {}. Runs only with confirmed_by_person, except as a dry run ({}), which runs at once.",
                a.name(),
                flags(free)
            ),
            Rule::AskWith(a, args, _) => format!(
                "Runs at once, but with {} {} it {} and needs the person's yes (confirmed_by_person).",
                args.iter()
                    .map(|x| format!("`{x}`"))
                    .collect::<Vec<_>>()
                    .join(" or "),
                if args.iter().all(|x| COUNTS.contains(x)) {
                    "above 0"
                } else {
                    "given"
                },
                a.name()
            ),
            Rule::Refuse(instead) => format!("Refused from a tool: {instead}"),
            Rule::RefuseWith(arg, instead, _) => {
                format!("Runs at once, but refused with `{arg}`: {instead}")
            }
            Rule::RefuseUnless(free, instead, _) => {
                format!("Runs at once with {}; refused without: {instead}", flags(free))
            }
        }
    }
}

fn flags(free: &[&str]) -> String {
    free.iter()
        .map(|f| format!("--{f}"))
        .collect::<Vec<_>>()
        .join(" or ")
}

/// When a command may start Laya, the decision server, on port 7720,
/// which a tool may not do: it then runs only while Laya already answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Laya {
    No,
    Always,
    /// Only with this flag.
    With(&'static str),
    /// Unless this flag is given.
    Unless(&'static str),
}

/// One row of the table.
#[derive(Debug, Clone, Copy)]
pub struct Row {
    pub path: &'static str,
    pub road: Road,
    pub rule: Rule,
    pub laya: Laya,
}

const fn r(path: &'static str, road: Road, rule: Rule) -> Row {
    Row {
        path,
        road,
        rule,
        laya: Laya::No,
    }
}

const fn laya(path: &'static str, road: Road, rule: Rule, laya: Laya) -> Row {
    Row {
        path,
        road,
        rule,
        laya,
    }
}

use Road::{Developer as D, Journey as J, OffTarget as O, Stage as S};
use Rule::{Read, Write};

const PUBLISH: Rule = Rule::Ask(Ask::Publish, &[]);
const DOWNLOAD: Rule = Rule::Ask(Ask::Download, &[]);
const DRY_RUN: &[&str] = &["dry-run"];
/// Arguments that count something: given as 0 they are not given.
const COUNTS: &[&str] = &["agents"];

/// Only the person decides a picture the agent proposes for their theme.
const PERSON_DECIDES: &str = "only the person decides a proposed picture, with ACCEPT, REJECT or PUT THE OLD ONE BACK on their page. Propose with theme picture propose, wait with next (wait_s) until waits_for_yes shows it answered, and read their answer with theme picture list.";
/// Only the person decides a sound change the agent proposes for their copy.
const PERSON_DECIDES_SOUND: &str = "only the person decides a proposed sound change, with ACCEPT, REJECT or PUT THE OLD ONE BACK on their page. Propose with sound_edit (or theme sound propose), wait with next (wait_s) until waits_for_yes shows it answered, and read their answer with theme sound list.";
/// Only the person decides a sprite edit the agent proposes for their copy.
const PERSON_DECIDES_SPRITE: &str = crate::sprite_proposal::PERSON_DECIDES;
/// Only the person changes the loop's rules (docs/loop.md).
const PERSON_SETS_LOOP: &str = "only the person changes the loop's settings (gp loop set), marks a step wrong (gp loop wrong, or WRONG on their page) or resumes a halted loop (gp loop resume), in their own terminal. Work as the open steps say: next and gp loop steps list them.";
const SERVE: &str = "it serves a page until stopped. The person starts `gp serve` themselves and opens http://127.0.0.1:7710/; for a local preview use build_publish.";

/// Every gp command, with its road and rule. Sorted by path.
pub const TABLE: &[Row] = &[
    // Making and playing games (the public set).
    r("assets find", J, Write),
    r("assets hash", S, Write),
    r("assets lift", J, Write),
    r(
        "assets open",
        O,
        Rule::Refuse(
            "it opens a pixel editor on the person's screen and waits for its saves. Use assets_export and assets_edit (import) instead.",
        ),
    ),
    r("assets pack", J, Read),
    r(
        "assets proposal accept",
        J,
        Rule::Refuse(PERSON_DECIDES_SPRITE),
    ),
    r("assets proposal list", J, Read),
    r(
        "assets proposal reject",
        J,
        Rule::Refuse(PERSON_DECIDES_SPRITE),
    ),
    r(
        "assets proposal undo",
        J,
        Rule::Refuse(PERSON_DECIDES_SPRITE),
    ),
    r("assets stub", J, Write),
    r("corpus bump", J, Write),
    r("corpus diff", J, Read),
    // Changes a clip already in the corpus only with --replace, removes
    // one only with --remove (it holds the others and says so), and those
    // need the person's yes; adding clips runs at once. SELF_GUARDED.
    r(
        "corpus import",
        J,
        Rule::AskWith(Ask::Replace, &["replace", "remove"], Base::Write),
    ),
    r("corpus init", J, Write),
    r("corpus reduce", S, Write),
    r("corpus score", J, Write),
    r("corpus show", J, Read),
    r("corpus verify", J, Write),
    r("game attest", J, PUBLISH),
    r("game build", J, Write),
    r("game fetch", J, DOWNLOAD),
    r("game hash", D, Read),
    r("game kit", J, Write),
    r("game new", J, Write),
    r("game score", J, Write),
    r("game share", J, Write),
    r("game test", J, Read),
    r("model blind", S, Write),
    r("model check", J, Read),
    r("model derive", S, Write),
    r("model diff", J, Read),
    r("model fmt", J, Write),
    r(
        "model gate",
        J,
        Rule::Ask(Ask::Publish, &["dry-run", "score-only"]),
    ),
    r("model mechanics", S, Read),
    r("model mutants", D, Write),
    r("model propose", S, Write),
    r("model questions", J, Write),
    r("model refine", J, Write),
    r("model schema", D, Read),
    r("model static", S, Write),
    r("package hooks", D, Read),
    r("package scan", J, Read),
    r("registry check", J, Read),
    r("registry grant", J, Rule::Ask(Ask::Publish, DRY_RUN)),
    r("registry grants", J, Read),
    // Reads the checkout and asks git what origin holds; writes nothing.
    r("registry host", J, Read),
    r("registry init", J, Write),
    r("registry publish", J, Rule::Ask(Ask::Publish, DRY_RUN)),
    r("registry reindex", J, Rule::Ask(Ask::Publish, DRY_RUN)),
    r("registry serve", J, Rule::Refuse(SERVE)),
    // Pushes to the person's fork on GitHub and opens a pull request on
    // the registry's repo: others see it. --dry-run makes the commits in a
    // clone under the work area and sends nothing.
    r("registry submit", J, Rule::Ask(Ask::Publish, DRY_RUN)),
    r("registry yank", J, Rule::Ask(Ask::Publish, DRY_RUN)),
    r("roms add", J, Write),
    r("roms clean", J, Write),
    r("roms diff", J, Read),
    r("roms scan", J, Read),
    r("roms sets", J, Read),
    r("serve", J, Rule::Refuse(SERVE)),
    r("template build", J, Write),
    r("template check", J, Read),
    r("template cites", D, Read),
    r("template derive", J, Write),
    r("template extract", J, Write),
    r("template font", D, Read),
    r("template pack", J, Write),
    r("template served", J, Read),
    r("theme check", J, Read),
    r("theme fetch", O, DOWNLOAD),
    r("theme fit", J, Write),
    r("theme generate", J, Rule::Ask(Ask::Spend, DRY_RUN)),
    r("theme kit", J, Write),
    r("theme list", O, Read),
    r("theme look check", J, Read),
    r("theme look cells", J, Read),
    r("theme look new", J, Write),
    r(
        "theme look play",
        J,
        Rule::RefuseUnless(
            &["background", "shots"],
            "without --background or --shots it serves until stopped. Give --background (it serves from a process of its own for --minutes, 60 by default, and returns the link and the PID) or --shots DIR (pictures in a folder the person named).",
            Base::Write,
        ),
    ),
    r("theme look sheet", J, Write),
    r("theme paint", J, Write),
    r("theme picture accept", J, Rule::Refuse(PERSON_DECIDES)),
    r("theme picture list", J, Read),
    r("theme picture propose", J, Read),
    r("theme picture reject", J, Rule::Refuse(PERSON_DECIDES)),
    r("theme picture undo", J, Rule::Refuse(PERSON_DECIDES)),
    r("theme publish", O, Rule::Ask(Ask::Publish, DRY_RUN)),
    r("theme recheck", O, Rule::Ask(Ask::Publish, DRY_RUN)),
    r("theme schema", D, Read),
    r("theme sheet", J, Write),
    r("theme sound accept", J, Rule::Refuse(PERSON_DECIDES_SOUND)),
    r("theme sound list", J, Read),
    r("theme sound propose", J, Read),
    r("theme sound reject", J, Rule::Refuse(PERSON_DECIDES_SOUND)),
    r("theme sound undo", J, Rule::Refuse(PERSON_DECIDES_SOUND)),
    r("theme style list", J, Read),
    r("theme style new", J, Write),
    r("theme style show", J, Read),
    r("theme video", J, Write),
    // Research, tests and the workflow's own tools (the lab set).
    r("agent approve", S, Write),
    r("agent attempt", S, Write),
    r("agent bench", D, Write),
    r("agent brief", S, Read),
    r("agent clips", S, Read),
    r("agent done", S, Write),
    r("agent integrate", D, Rule::Ask(Ask::Publish, DRY_RUN)),
    r("agent note", S, Write),
    r("agent score", S, Write),
    r("agent state", S, Read),
    r("agent tell", S, Write),
    r("agent tells", S, Read),
    r("author", J, Write),
    r("baseline check", D, Read),
    r("bench", D, Read),
    r("bench clips", D, Write),
    r("bench report", D, Read),
    r("bench scratch", D, Write),
    r(
        "capture background",
        S,
        Rule::AskWith(Ask::Spend, &["agents"], Base::Write),
    ),
    r("capture carry", S, Read),
    r("capture dense", S, Write),
    r("capture events", S, Read),
    r("capture fidelity", S, Write),
    r("capture impact", S, Read),
    r("capture list", S, Read),
    r("capture regenerate", S, Write),
    r("capture schema", D, Read),
    r("capture verify", S, Write),
    r("catalog", S, Read),
    r("chunks", J, Write),
    r("clip log", S, Write),
    r("clip presses", J, Write),
    r("clip score", S, Read),
    r("clip screen", S, Read),
    r("code listing", S, Write),
    r("code tables", S, Read),
    r("coverage", D, Read),
    r("coverage check", D, Read),
    r("cycles chain", S, Write),
    r("cycles check", S, Read),
    r("cycles readset", S, Read),
    r("cycles slice", S, Read),
    r("cycles split", S, Write),
    r("dataset gaps", S, Read),
    r(
        "dataset grade",
        D,
        Rule::AskWith(Ask::Spend, &["key"], Base::Read),
    ),
    r("dataset rom", S, Write),
    r("element explain", S, Read),
    r("element inputs", S, Read),
    r("env", D, Read),
    r("events", S, Read),
    r("explore", S, Write),
    r("farm", S, Write),
    r("farm-topdown", S, Write),
    r("findings confirm", S, Write),
    r("findings cut", S, Write),
    r("findings ingest", S, Write),
    r("findings list", S, Read),
    r("gaps all", S, Write),
    r("gaps changes", S, Write),
    r("gaps drawn", S, Write),
    r("gaps interactive", S, Write),
    r("host record", J, Write),
    r("host scenarios", J, Read),
    r("host search", J, Read),
    r("host serve", J, Rule::Refuse(SERVE)),
    r("host speed", D, Read),
    r("input-log check", S, Read),
    r("input-log fork", S, Write),
    r("input-log generate", S, Write),
    r("interp baselines", J, Write),
    r("interp compare", S, Read),
    r("interp draws", S, Write),
    r("interp moments", S, Read),
    r("interp odds", S, Read),
    r("interp reach", J, Write),
    r("interp run", J, Write),
    r("interp scenarios", S, Write),
    r("interp slots", S, Write),
    r("interp translate", S, Write),
    r("labels check", S, Read),
    r("labels lookup", S, Read),
    r("labels propose", S, Write),
    r("labels show", S, Read),
    r("level difficulty", J, Read),
    // A level edit writes only a copy in the home's work area: the command
    // itself refuses a games checkout's original, a folder in a git work
    // tree and a published package, so the refusal holds by hand too.
    r("level edit", J, Write),
    r("level kit", J, Write),
    r("level list", J, Read),
    r("level measure", J, Read),
    r("level render", J, Read),
    r("level show", J, Read),
    r("level undo", J, Write),
    r("level verify", J, Read),
    r("localize", S, Write),
    r(
        "mcp",
        J,
        Rule::Refuse("it is this server; it is already running."),
    ),
    r("mechanic autoplay", S, Write),
    r("mechanic busy", J, Write),
    r("mechanic clip-save", S, Write),
    r("mechanic compare", J, Write),
    r("mechanic contacts", S, Write),
    laya("mechanic demo", S, Write, Laya::Always),
    r("mechanic harvest", S, Write),
    r("mechanic import", D, Write),
    r("mechanic leads", S, Write),
    r("mechanic log", J, Write),
    r("mechanic pin", J, Write),
    r("mechanic probe", S, Write),
    r("mechanic problem", J, Write),
    r("mechanic queue", J, Write),
    r("mechanic screen", J, Write),
    laya("mechanic say", J, Write, Laya::With("judge")),
    r("mechanic show", J, Read),
    r(
        "mechanic start",
        J,
        Rule::Refuse(
            "it serves the game's page until stopped. Ask the person to open the page (`gp serve`, then the game's CLIPS tab), and use next with wait_s to hear what they hand you.",
        ),
    ),
    r("mechanic sweep", S, Write),
    laya("mechanic triage", S, Write, Laya::Unless("dry-run")),
    r("mechanic video", S, Write),
    r(
        "mechanic wait",
        J,
        Rule::Refuse(
            "it waits with no time limit. Use next with wait_s (at most 30 s), or mechanic show to read the session now.",
        ),
    ),
    r(
        "mechanic watch",
        J,
        Rule::Refuse(
            "it serves the watch panel until stopped. Use mechanic log, pin and problem; the person sees them on the page they already have open.",
        ),
    ),
    r("metrics check", D, Read),
    r("metrics record", D, Write),
    r("metrics report", D, Write),
    r(
        "metrics time",
        D,
        Rule::Refuse("it runs any program it is given. Run the gp command itself with gp_run."),
    ),
    r("movie diagnose", J, Read),
    r("movie hashes", D, Read),
    r("movie import", J, Write),
    r("noop", D, Read),
    r("oracle check", D, Read),
    r("oracle diff", D, Read),
    r("oracle golden", D, Write),
    r("oracle record", D, Write),
    r("oracle triage", D, Write),
    r("picture", S, Write),
    r("progress", J, Write),
    r("query", S, Read),
    r("reach", S, Write),
    r("record", S, Write),
    r("recordings list", S, Read),
    r("recordings show", S, Read),
    r("replay", S, Read),
    r("rule jump", S, Write),
    r("rule lane", S, Write),
    r("run-until", S, Read),
    r("runs list", D, Read),
    r("scorecard", D, Read),
    r("sealed leaks", J, Read),
    r("sealed score", J, Write),
    r("self check", J, Read),
    r("self commands", D, Read),
    r("self docs", D, Write),
    r("self features", D, Read),
    r("self path", D, Read),
    r("self publish", D, PUBLISH),
    r("self schemas", D, Write),
    r("self shell", D, Write),
    r("solve", S, Read),
    r("sound banks", J, Read),
    r("sound capture", S, Write),
    r("sound compare", S, Read),
    r("sound decode", S, Write),
    r("sound drivers", S, Read),
    r("sound fidelity", S, Write),
    r("sound render", J, Write),
    r("sprites capture", S, Write),
    r("sprites schema", D, Read),
    r("store gc", D, Write),
    r("store get", D, Write),
    r("store put", D, Write),
    r("store reindex", D, Read),
    r("store stats", D, Read),
    r("store unreferenced", D, Read),
    r("store verify", D, Read),
    r("testroms built", D, Read),
    r("testroms fetch", D, DOWNLOAD),
    r("testroms run", D, Rule::Ask(Ask::Download, &["offline"])),
    r("tool gate", D, Rule::Ask(Ask::Publish, DRY_RUN)),
    r("trace diff", D, Read),
    r("trace fields", D, Read),
    r("trace frame", D, Read),
    r("trace instructions", D, Write),
    r("trace verify", D, Read),
    r("track", S, Read),
    r("validate", J, Read),
    r("video", D, Write),
    r("watch detail", S, Read),
    r(
        "watch events",
        S,
        Rule::RefuseWith(
            "follow",
            "it follows the session until stopped. Use the watch_events tool, whose wait_ms waits for new events.",
            Base::Read,
        ),
    ),
    r("watch explore", S, Write),
    r("watch list", S, Read),
    r("watch point", S, Read),
    r("watch rerun", S, Write),
    r("watch start", S, Write),
    r("worker env", D, Read),
    r("loop resume", S, Rule::Refuse(PERSON_SETS_LOOP)),
    r("loop set", S, Rule::Refuse(PERSON_SETS_LOOP)),
    r("loop steps", S, Read),
    r("loop health", S, Read),
    r("loop wrong", S, Rule::Refuse(PERSON_SETS_LOOP)),
];

/// The commands known to run longer than gp_run's usual limit
/// (`run::DEFAULT_TIMEOUT`, 300 s), with the seconds a call that gives no
/// `timeout_s` gets instead. Measured on 2026-11-14: `corpus score` on
/// an arcade game took 444 s in a debug build; a gate runs a whole check and a
/// corpus, 30 to 60 minutes. Every path is a row of [`TABLE`] (a test
/// holds it), and none is past `run::MAX_TIMEOUT`.
pub const LONG: &[(&str, u64)] = &[
    // Whole corpora: every clip through the module, or the ROM again.
    ("baseline check", 1800),
    ("capture verify", 1800),
    ("corpus score", 1800),
    ("corpus verify", 1800),
    ("game score", 1800),
    ("interp baselines", 1800),
    ("sealed score", 1800),
    ("agent score", 1800),
    // The model's stages, clip by clip against the original.
    ("model refine", 1800),
    ("model static", 1800),
    ("mechanic triage", 1800),
    // Builds: a module in the sandbox, a worktree's gp and preview.
    ("game build", 900),
    ("game share", 1800),
    ("game test", 900),
    ("agent attempt", 1800),
    // The gates (their dry runs and score-only runs are free).
    ("model gate", 3600),
    ("tool gate", 3600),
];

/// The seconds a call of `path` runs when it gives no `timeout_s`: its
/// [`LONG`] entry, or None for gp_run's usual limit.
pub fn long_timeout_s(path: &str) -> Option<u64> {
    LONG.iter().find(|(p, _)| *p == path).map(|(_, s)| *s)
}

/// A command's row in the table.
pub fn row(path: &str) -> Option<&'static Row> {
    TABLE.iter().find(|r| r.path == path)
}

/// A command's rule: its row, or for a command the table doesn't hold, one
/// read from its path and help, failing safe.
pub fn row_or_guess(path: &str, help: &str) -> Row {
    if let Some(r) = row(path) {
        return *r;
    }
    let words = format!("{path} {help}").to_lowercase();
    let has = |w: &[&str]| w.iter().any(|w| words.contains(w));
    let rule = if has(&["serve", "until stopped", "follow"]) {
        Rule::Refuse(
            "gp_run doesn't know this command, and its words say it may run until stopped.",
        )
    } else if has(&[
        "publish", "push", "upload", "gate", "attest", "registry", "git:", "s3://",
    ]) {
        Rule::Ask(Ask::Publish, DRY_RUN)
    } else if has(&["generate", "runway", "secret", "paid", "credits", "key"]) {
        Rule::Ask(Ask::Spend, DRY_RUN)
    } else if has(&["fetch", "download"]) {
        Rule::Ask(Ask::Download, &[])
    } else {
        Rule::Write
    };
    Row {
        path: "",
        road: Road::Developer,
        rule,
        laya: Laya::No,
    }
}

/// The writes that guard data already in the person's games themselves,
/// so gp_run's net over the games checkout ([`crate::held`]) leaves them
/// be: each changes or removes what is there only when an argument names
/// it, and its rule asks for the person's yes on that argument. Its plan
/// (the call without `--write`, or with `--dry-run`) lists in `replaces`
/// what the call would change, which the ask shows. A command joins only
/// with a test in gp-cli that it holds what isn't named.
pub const SELF_GUARDED: &[&str] = &["corpus import"];

/// Whether gp_run's net over the games checkout watches a call of `path`.
pub fn netted(path: &str) -> bool {
    !SELF_GUARDED.contains(&path)
}

/// What gp_run does with a call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Run it; `writes` when it may write the person's things.
    Run { writes: bool },
    /// Run it as a write under the net: a change it makes to a file
    /// already in the games checkout (more than lines added) is put back
    /// and held until the person's yes on the ask that shows it
    /// ([`crate::held`]); new files and added lines stay.
    RunHeld,
    /// Run it only with the person's yes.
    NeedsYes(Ask),
    /// Never from a tool.
    Refused(String),
}

/// The decision for a call of `row`'s command: `given` says whether a
/// flag or argument (by its name, `dry-run`) is in the call, and `values`
/// are every value the call passes.
pub fn decide(row: &Row, given: &dyn Fn(&str) -> bool, values: &[(String, String)]) -> Decision {
    // Ports 7710 (gp serve) and 7720 (Laya) belong to the person's own
    // servers.
    for (name, v) in values {
        if (name == "port" || name.ends_with("-port")) && (v == "7710" || v == "7720") {
            return Decision::Refused(format!(
                "port {v} belongs to the person's own server (7710 is gp serve's, 7720 is Laya's); a tool never binds it"
            ));
        }
    }
    // The rules for a write that changes data already in the games
    // checkout live here: one that guards itself (SELF_GUARDED) asks on
    // the arguments that name what it changes; every other write runs
    // under the net, which holds such a change for the person's yes. The
    // check that the call is a step `next` gave (issue #43) is one more
    // input to the decision, applied to what this returns: decide_step.
    let base = |b: Base| match b {
        Base::Read => Decision::Run { writes: false },
        Base::Write if netted(row.path) => Decision::RunHeld,
        Base::Write => Decision::Run { writes: true },
    };
    // gp model gate --import takes the sessions' new clips into the corpus
    // with no yes (owner, 2026-11-19): a clip only added changes nothing
    // already held, and the gate fails the import if any clip scores worse.
    // With --replaces or --removes it changes clips already there, and asks.
    let keeps_clips = row.path == "model gate"
        && given("import")
        && !["replaces", "removes", "patch"].iter().any(|f| given(f));
    let decision = match row.rule {
        _ if keeps_clips => Decision::Run { writes: true },
        Rule::Read => base(Base::Read),
        Rule::Write => base(Base::Write),
        Rule::Ask(_, free) if free.iter().any(|f| given(f)) => base(Base::Read),
        Rule::Ask(a, _) => Decision::NeedsYes(a),
        Rule::AskWith(a, args, _)
            if args.iter().any(|arg| {
                given(arg)
                    && !(COUNTS.contains(arg)
                        && values
                            .iter()
                            .filter(|(n, _)| n == arg)
                            .all(|(_, v)| v == "0"))
            }) =>
        {
            Decision::NeedsYes(a)
        }
        Rule::AskWith(_, _, b) => base(b),
        Rule::Refuse(why) => Decision::Refused(why.to_owned()),
        Rule::RefuseWith(arg, why, _) if given(arg) => Decision::Refused(why.to_owned()),
        Rule::RefuseWith(_, _, b) => base(b),
        Rule::RefuseUnless(free, _, b) if free.iter().any(|f| given(f)) => base(b),
        Rule::RefuseUnless(_, why, _) => Decision::Refused(why.to_owned()),
    };
    // A target in git or a bucket publishes, whatever the command.
    if matches!(decision, Decision::Run { .. } | Decision::RunHeld)
        && values
            .iter()
            .any(|(_, v)| v.starts_with("git:") || v.starts_with("s3://"))
    {
        return Decision::NeedsYes(Ask::Publish);
    }
    decision
}

/// What the loop knows when gp_run checks a call against its steps
/// (docs/loop.md, crate::steps): the step the call cites, the steps open
/// now and those the runner's live workers hold (`{id, who, subject, says,
/// commands}` each), why the loop is halted, and the step a worker's gp
/// was started for (`GP_STEP`).
#[derive(Debug, Default)]
pub struct Steps<'a> {
    pub cited: Option<&'a str>,
    pub open: &'a [Value],
    pub held: &'a [Value],
    pub halted: Option<&'a str>,
    pub bound: Option<&'a str>,
}

/// A step in a sentence: its id, who, subject, words and commands.
pub fn step_words(s: &Value) -> String {
    let cmds: Vec<&str> = s["commands"]
        .as_array()
        .map(|c| c.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    format!(
        "{} ({}, {}): {}{}",
        s["id"].as_str().unwrap_or("?"),
        s["who"].as_str().unwrap_or("?"),
        s["subject"].as_str().unwrap_or("the home"),
        s["says"].as_str().unwrap_or("").trim_end_matches('.'),
        if cmds.is_empty() {
            ", runs no gp command".to_owned()
        } else {
            format!(", runs {}", cmds.join(", "))
        }
    )
}

fn steps_named(list: &[&Value]) -> String {
    if list.is_empty() {
        return "none".into();
    }
    let mut w: Vec<String> = list.iter().take(8).map(|s| step_words(s)).collect();
    if list.len() > 8 {
        w.push(format!("and {} more (gp loop steps)", list.len() - 8));
    }
    w.join("; ")
}

/// The loop's check on a decision (issue #43): a call that writes, lands
/// or publishes runs only as a step the loop gave, cited by its id, whose
/// commands include the call's. Reads, and refusals already made, pass as
/// they are. While the loop is halted nothing is a step, but a worker
/// holding one may still write (not land or publish) to finish it.
pub fn decide_step(decision: Decision, path: &str, st: &Steps) -> Decision {
    if !matches!(
        decision,
        Decision::Run { writes: true } | Decision::RunHeld | Decision::NeedsYes(_)
    ) {
        return decision;
    }
    let lands = matches!(decision, Decision::NeedsYes(_));
    let open: Vec<&Value> = st.open.iter().collect();
    // "*" in a step's commands: any plain write (a check's files, the
    // item's record); a landing, a publish or a spend still has to be
    // named.
    let runs_it = |s: &&Value| {
        s["commands"]
            .as_array()
            .is_some_and(|c| c.iter().any(|x| x == path) || (!lands && c.iter().any(|x| x == "*")))
    };
    let these: Vec<&Value> = open.iter().copied().filter(runs_it).collect();
    let cited = st.cited.or(st.bound);
    if let (Some(b), Some(c)) = (st.bound, st.cited)
        && b != c
    {
        return Decision::Refused(format!(
            "this worker was started for step {b}, and a call may cite no other (it cited {c})"
        ));
    }
    let found = cited.and_then(|id| {
        open.iter()
            .copied()
            .chain(st.held.iter())
            .find(|s| s["id"] == json!(id))
    });
    if let Some(why) = st.halted {
        let held = cited.is_some_and(|id| st.held.iter().any(|s| s["id"] == json!(id)));
        if lands || !held {
            return Decision::Refused(format!(
                "the loop is halted: {why}. Nothing {} until a tool-gate landing clears it (docs/loop.md, \"Wrong\")",
                if lands {
                    "lands or publishes"
                } else {
                    "writes as a step"
                }
            ));
        }
    }
    let Some(id) = cited else {
        return Decision::Refused(format!(
            "gp {path} writes, lands or publishes, so gp_run runs it only as a step the loop gave: call again with step set to the id of an open step whose commands include {path}. Open steps that run it: {}. All open steps: {}",
            steps_named(&these),
            steps_named(&open)
        ));
    };
    let Some(s) = found else {
        return Decision::Refused(format!(
            "step {id} isn't open now: the facts it was decided from changed, or it was never a step (next and gp loop steps give the open ones). Open steps that run gp {path}: {}. All open steps: {}",
            steps_named(&these),
            steps_named(&open)
        ));
    };
    if !runs_it(&s) {
        return Decision::Refused(format!(
            "step {} doesn't run gp {path}. Open steps that run it: {}",
            step_words(s),
            steps_named(&these)
        ));
    }
    decision
}

/// The rule for the `sound_edit` tool's direct changes (`write` or
/// `undo`), which change the person's sound without a proposal: their
/// yes. While their page is open the yes is their ACCEPT on a proposal's
/// card, so a direct change is refused there and the agent proposes; with
/// no page open it runs on the agent's confirmed_by_person, as gp_run's
/// asks do. A proposal (the tool's default) is free, as
/// `theme sound propose` is.
pub fn direct_sound_edit(page_open: bool, confirmed_by_person: bool) -> Decision {
    if page_open {
        Decision::Refused("a direct change to the person's sound needs their yes, and with their page open that is their ACCEPT on a proposal's card. Leave write and undo out: sound_edit then proposes the change, the page shows it with the sound now and the new one, ACCEPT and REJECT, and next (wait_s) tells you their answer. To put back a change they accepted, they press PUT THE OLD ONE BACK; or propose the change back.".into())
    } else if confirmed_by_person {
        Decision::Run { writes: true }
    } else {
        Decision::Refused("a direct change to the person's sound needs their yes. Leave write and undo out to propose it instead (it waits on their page, gp serve, with ACCEPT and REJECT). No page is open for this home, so if they say yes to this exact change here, call again with confirmed_by_person: true.".into())
    }
}

/// Whether a call of `row`'s command may start Laya.
pub fn may_start_laya(row: &Row, given: &dyn Fn(&str) -> bool) -> bool {
    match row.laya {
        Laya::No => false,
        Laya::Always => true,
        Laya::With(f) => given(f),
        Laya::Unless(f) => !given(f),
    }
}

/// The rules in words, as `gp_commands` prints them, with how many
/// commands each holds.
pub fn summary() -> Value {
    // A rule with a condition counts where the condition takes it: a
    // command that needs a yes with one argument is under ask.
    let count = |f: &dyn Fn(&Rule) -> bool| TABLE.iter().filter(|r| f(&r.rule)).count();
    json!({
        "read": {
            "commands": count(&|r| matches!(r, Rule::Read)),
            "means": "writes nothing outside the home's work and cache areas; runs at once",
        },
        "write": {
            "commands": count(&|r| matches!(r, Rule::Write)),
            "means": "writes the person's games, working copies or home; runs at once, and the reply says what it wrote when the command reports it. A change it makes to a file already in the games checkout (more than lines added) is put back and held: the reply is refused with would_change, diff and ask_id, and the held change goes in on the person's yes (the card, or with no page open confirmed_by_person on a later call after you showed them the change), without running the command again",
        },
        "ask": {
            "commands": count(&|r| matches!(r, Rule::Ask(..) | Rule::AskWith(..))),
            "means": "publishes, uploads, pushes or lands a change through a gate; spends money, calls a paid service or reads a secret or answer key; or downloads a lot. A refusal puts the ask on the person's page as a card with YES and NO (ask_id); call again with ask_id once next shows it answered. With no page open, confirmed_by_person: true runs it, which you may set only after the person said yes to that exact action in this conversation. Its dry-run flag, where it has one, runs at once. Some need a yes only with one argument: their rule says which",
        },
        "refused": {
            "commands": count(&|r| matches!(r, Rule::Refuse(_) | Rule::RefuseWith(..) | Rule::RefuseUnless(..))),
            "means": "serves until stopped, waits with no bound, opens a program on the person's screen, or runs any program: refused from a tool, and the refusal names what to use instead. Some are refused only with one argument: their rule says which. Ports 7710 and 7720 are never bound from a tool",
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// docs/mcp.md's table of rules gives how many commands each rule has,
    /// counted from this table, so it can't go stale.
    #[test]
    fn the_docs_count_the_commands_of_each_rule() {
        let doc = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/mcp.md"),
        )
        .unwrap();
        let mut wrong = Vec::new();
        for kind in ["read", "write", "ask", "refused"] {
            let rows: Vec<&str> = TABLE
                .iter()
                .filter(|r| r.rule.kind() == kind)
                .map(|r| r.path)
                .collect();
            let line = format!("| {kind} | {} |", rows.len());
            if !doc.lines().any(|l| l.starts_with(&line)) {
                wrong.push(format!("`{line}` ({})", rows.join(", ")));
            }
        }
        assert!(
            wrong.is_empty(),
            "docs/mcp.md's table under \"Every command\" should have rows starting:\n{}",
            wrong.join("\n")
        );
    }

    fn given<'a>(names: &'a [&'a str]) -> impl Fn(&str) -> bool + 'a {
        move |n| names.contains(&n)
    }

    #[test]
    fn the_table_is_sorted_with_no_repeats() {
        // Sorted within each set, so a row is easy to find; no path twice.
        let mut seen = std::collections::BTreeSet::new();
        for r in TABLE {
            assert!(seen.insert(r.path), "{} is in the table twice", r.path);
        }
    }

    #[test]
    fn publishing_needs_a_yes_and_a_dry_run_doesnt() {
        let gate = row("model gate").unwrap();
        assert_eq!(
            decide(gate, &given(&[]), &[]),
            Decision::NeedsYes(Ask::Publish)
        );
        assert_eq!(
            decide(gate, &given(&["dry-run"]), &[]),
            Decision::Run { writes: false }
        );
        // Taking the sessions' new clips in needs no yes; changing or
        // removing clips already in the corpus does.
        assert_eq!(
            decide(gate, &given(&["import"]), &[]),
            Decision::Run { writes: true }
        );
        for with in ["replaces", "removes"] {
            assert_eq!(
                decide(gate, &given(&["import", with]), &[]),
                Decision::NeedsYes(Ask::Publish),
                "{with}"
            );
        }
        let generate = row("theme generate").unwrap();
        assert_eq!(
            decide(generate, &given(&[]), &[]),
            Decision::NeedsYes(Ask::Spend)
        );
        for p in [
            "registry publish",
            "registry submit",
            "self publish",
            "tool gate",
            "game attest",
        ] {
            assert_eq!(
                decide(row(p).unwrap(), &given(&[]), &[]),
                Decision::NeedsYes(Ask::Publish),
                "{p}"
            );
        }
        for p in ["testroms fetch", "game fetch", "theme fetch"] {
            assert_eq!(
                decide(row(p).unwrap(), &given(&[]), &[]),
                Decision::NeedsYes(Ask::Download),
                "{p}"
            );
        }
    }

    #[test]
    fn a_theme_plays_in_the_background_or_as_pictures_and_never_publishes() {
        let play = row("theme look play").unwrap();
        assert_eq!(play.road, Road::Journey);
        // In the foreground it serves until stopped.
        assert!(matches!(
            decide(play, &given(&[]), &[]),
            Decision::Refused(why) if why.contains("--background")
        ));
        // Serving on a local port for a while, or pictures in a folder the
        // person named: runs at once, with no yes, as a write.
        for f in ["background", "shots"] {
            assert_eq!(decide(play, &given(&[f]), &[]), Decision::RunHeld, "{f}");
        }
        // The person's own ports stay theirs.
        let port = [("port".to_owned(), "7710".to_owned())];
        assert!(matches!(
            decide(play, &given(&["background", "port"]), &port),
            Decision::Refused(_)
        ));
    }

    #[test]
    fn a_proposed_picture_is_proposed_freely_and_decided_only_by_the_person() {
        // Proposing writes only a pending file in the home's work area,
        // and listing reads the answers: both run at once.
        for p in ["theme picture propose", "theme picture list"] {
            assert_eq!(
                decide(row(p).unwrap(), &given(&[]), &[]),
                Decision::Run { writes: false },
                "{p}"
            );
        }
        // Applying is the person's ACCEPT on the page, never the agent's.
        for p in [
            "theme picture accept",
            "theme picture reject",
            "theme picture undo",
        ] {
            match decide(row(p).unwrap(), &given(&[]), &[]) {
                Decision::Refused(why) => {
                    assert!(why.contains("only the person decides"), "{p}: {why}")
                }
                d => panic!("{p} isn't refused: {d:?}"),
            }
            // A yes from the conversation doesn't change it.
            assert!(matches!(
                decide(row(p).unwrap(), &given(&["words"]), &[]),
                Decision::Refused(_)
            ));
        }
    }

    #[test]
    fn a_proposed_sound_is_proposed_freely_and_decided_only_by_the_person() {
        // Proposing writes only a pending file in the home's work area,
        // and listing reads the answers: both run at once.
        for p in ["theme sound propose", "theme sound list"] {
            assert_eq!(
                decide(row(p).unwrap(), &given(&[]), &[]),
                Decision::Run { writes: false },
                "{p}"
            );
        }
        // Applying is the person's ACCEPT on the page, never the agent's.
        for p in [
            "theme sound accept",
            "theme sound reject",
            "theme sound undo",
        ] {
            match decide(row(p).unwrap(), &given(&[]), &[]) {
                Decision::Refused(why) => {
                    assert!(why.contains("only the person decides"), "{p}: {why}")
                }
                d => panic!("{p} isn't refused: {d:?}"),
            }
            assert!(matches!(
                decide(row(p).unwrap(), &given(&["words"]), &[]),
                Decision::Refused(_)
            ));
        }
        // sound_edit's direct write (and undo) stays for callers that
        // had it, with the person's yes: never while their page is open,
        // where the card is the yes, and with no page only on their word.
        for confirmed in [false, true] {
            assert!(matches!(
                direct_sound_edit(true, confirmed),
                Decision::Refused(why) if why.contains("ACCEPT")
            ));
        }
        assert!(matches!(
            direct_sound_edit(false, false),
            Decision::Refused(why) if why.contains("confirmed_by_person")
        ));
        assert_eq!(
            direct_sound_edit(false, true),
            Decision::Run { writes: true }
        );
    }

    #[test]
    fn an_answer_key_needs_a_yes() {
        let grade = row("dataset grade").unwrap();
        assert_eq!(
            decide(grade, &given(&[]), &[]),
            Decision::Run { writes: false }
        );
        assert_eq!(
            decide(grade, &given(&["key"]), &[]),
            Decision::NeedsYes(Ask::Spend)
        );
    }

    /// A write runs under the net, but one that guards itself asks on
    /// the arguments that name what it changes, and adds at once.
    #[test]
    fn a_write_runs_under_the_net_and_corpus_import_asks_to_change_a_clip() {
        assert_eq!(
            decide(row("model fmt").unwrap(), &given(&[]), &[]),
            Decision::RunHeld
        );
        let import = row("corpus import").unwrap();
        assert_eq!(
            decide(import, &given(&["write"]), &[]),
            Decision::Run { writes: true }
        );
        for f in ["replace", "remove"] {
            assert_eq!(
                decide(import, &given(&["write", f]), &[]),
                Decision::NeedsYes(Ask::Replace),
                "{f}"
            );
        }
        for p in SELF_GUARDED {
            assert!(
                matches!(
                    row(p).map(|r| r.rule),
                    Some(Rule::AskWith(Ask::Replace, _, Base::Write))
                ),
                "{p} guards itself, so its rule asks on what names a change"
            );
        }
    }

    #[test]
    fn paid_clip_agents_need_a_yes() {
        let bg = row("capture background").unwrap();
        let vals = |n: &str| vec![("agents".to_owned(), n.to_owned())];
        assert_eq!(decide(bg, &given(&[]), &[]), Decision::RunHeld);
        assert_eq!(
            decide(bg, &given(&["agents"]), &vals("0")),
            Decision::RunHeld
        );
        assert_eq!(
            decide(bg, &given(&["agents"]), &vals("2")),
            Decision::NeedsYes(Ask::Spend)
        );
    }

    #[test]
    fn servers_are_refused_and_name_what_to_use() {
        for p in [
            "serve",
            "mcp",
            "host serve",
            "registry serve",
            "mechanic start",
            "mechanic watch",
            "mechanic wait",
        ] {
            assert!(
                matches!(
                    decide(row(p).unwrap(), &given(&[]), &[]),
                    Decision::Refused(_)
                ),
                "{p}"
            );
        }
        let events = row("watch events").unwrap();
        assert_eq!(
            decide(events, &given(&[]), &[]),
            Decision::Run { writes: false }
        );
        let Decision::Refused(why) = decide(events, &given(&["follow"]), &[]) else {
            panic!("--follow runs until stopped")
        };
        assert!(why.contains("watch_events"), "{why}");
    }

    #[test]
    fn hosting_your_own_asks_before_each_write_to_the_outside_world() {
        // host_your_own's road (docs/host-your-own.md): host reads, and every
        // command that writes the person's registry repo needs their yes.
        let host = row("registry host").unwrap();
        assert_eq!(host.rule, Rule::Read);
        let dir = [("dir".to_owned(), "/home/acme/acme-registry".to_owned())];
        assert_eq!(
            decide(host, &given(&["dir", "theme"]), &dir),
            Decision::Run { writes: false }
        );
        let git = [(
            "target".to_owned(),
            "git:/home/acme/acme-registry".to_owned(),
        )];
        for p in [
            "registry init",
            "registry grant",
            "registry publish",
            "registry reindex",
        ] {
            assert_eq!(
                decide(row(p).unwrap(), &given(&["dry-run"]), &git),
                Decision::NeedsYes(Ask::Publish),
                "{p} on a git: target"
            );
        }
    }

    #[test]
    fn the_person_s_ports_and_published_targets() {
        let read = row("env").unwrap();
        let port = [("port".to_owned(), "7720".to_owned())];
        assert!(matches!(
            decide(read, &given(&["port"]), &port),
            Decision::Refused(_)
        ));
        let git = [("target".to_owned(), "git:../registry".to_owned())];
        assert_eq!(
            decide(row("registry init").unwrap(), &given(&["target"]), &git),
            Decision::NeedsYes(Ask::Publish)
        );
    }

    #[test]
    fn laya_is_only_for_the_calls_that_may_start_it() {
        let say = row("mechanic say").unwrap();
        assert!(!may_start_laya(say, &given(&[])));
        assert!(may_start_laya(say, &given(&["judge"])));
        let triage = row("mechanic triage").unwrap();
        assert!(may_start_laya(triage, &given(&[])));
        assert!(!may_start_laya(triage, &given(&["dry-run"])));
        assert!(!may_start_laya(row("farm").unwrap(), &given(&[])));
    }

    #[test]
    fn explaining_a_spot_runs_freely() {
        // A person's "what is this?" on the page: the agent answers it at
        // once, with nothing written but the home's caches.
        let r = row("element explain").unwrap();
        assert_eq!(r.rule, Rule::Read);
        assert_eq!(
            decide(r, &given(&["run", "clip", "frame", "at"]), &[]),
            Decision::Run { writes: false }
        );
    }

    #[test]
    fn an_unknown_command_fails_safe() {
        for (path, help) in [
            ("theme push", "Push a theme"),
            ("model upload", "Upload it"),
            ("roms fetch-all", "Download every ROM"),
            ("art generate", "Make art"),
            ("page serve", "Serve the page"),
            ("tool gate2", "Land a patch"),
        ] {
            let row = row_or_guess(path, help);
            assert!(
                !matches!(row.rule, Rule::Read | Rule::Write),
                "{path} must need a yes or be refused, got {:?}",
                row.rule
            );
            assert_eq!(row.road, Road::Developer);
        }
        assert_eq!(row_or_guess("model look", "Show it").rule, Rule::Write);
    }

    #[test]
    fn a_level_is_read_freely_and_edited_as_a_write_to_the_copy() {
        for p in ["level list", "level show", "level render", "level verify"] {
            let r = row(p).unwrap();
            assert_eq!(r.road, Road::Journey, "{p}");
            assert_eq!(
                decide(r, &given(&[]), &[]),
                Decision::Run { writes: false },
                "{p}"
            );
        }
        for p in ["level edit", "level undo"] {
            let r = row(p).unwrap();
            assert_eq!(r.road, Road::Journey, "{p}");
            assert_eq!(decide(r, &given(&[]), &[]), Decision::RunHeld, "{p}");
        }
        // A copy in git or a bucket would publish: that needs a yes.
        let git = [("game".to_owned(), "git:../games".to_owned())];
        assert_eq!(
            decide(row("level edit").unwrap(), &given(&["game"]), &git),
            Decision::NeedsYes(Ask::Publish)
        );
        // The person's own ports stay theirs, even for a read.
        let port = [("port".to_owned(), "7710".to_owned())];
        assert!(matches!(
            decide(row("level render").unwrap(), &given(&["port"]), &port),
            Decision::Refused(_)
        ));
    }

    #[test]
    fn a_star_step_runs_any_plain_write_but_no_landing() {
        let open = [
            json!({"id": "s444444444444", "who": "agent", "subject": "g", "says": "Work on a row.", "commands": ["model gate", "*"]}),
        ];
        let st = Steps {
            cited: Some("s444444444444"),
            open: &open,
            ..Steps::default()
        };
        let w = Decision::Run { writes: true };
        assert_eq!(decide_step(w.clone(), "picture", &st), w);
        let land = Decision::NeedsYes(Ask::Publish);
        assert_eq!(decide_step(land.clone(), "model gate", &st), land);
        assert!(matches!(
            decide_step(land, "registry publish", &st),
            Decision::Refused(_)
        ));
    }

    #[test]
    fn a_write_runs_only_as_an_open_step_that_runs_its_command() {
        let open = [
            json!({"id": "s111111111111", "who": "agent", "subject": "g", "says": "Add the clips.", "commands": ["corpus import"]}),
            json!({"id": "s222222222222", "who": "person", "subject": "g", "says": "Press YES.", "commands": []}),
        ];
        let held = [
            json!({"id": "s333333333333", "who": "agent", "subject": "h", "says": "Fix it.", "commands": ["mechanic say"]}),
        ];
        let w = Decision::Run { writes: true };
        let st = |cited| Steps {
            cited,
            open: &open,
            held: &held,
            ..Steps::default()
        };
        // Reads are free, with or without a step.
        let read = Decision::Run { writes: false };
        assert_eq!(decide_step(read.clone(), "progress", &st(None)), read);
        // No step: refused, naming the open steps and the one that runs it.
        let Decision::Refused(why) = decide_step(w.clone(), "corpus import", &st(None)) else {
            panic!("a write with no step runs")
        };
        assert!(why.contains("call again with step set to"), "{why}");
        assert!(
            why.contains("s111111111111 (agent, g): Add the clips, runs corpus import"),
            "{why}"
        );
        // The step that runs it: as decided.
        assert_eq!(
            decide_step(w.clone(), "corpus import", &st(Some("s111111111111"))),
            w
        );
        assert_eq!(
            decide_step(
                Decision::RunHeld,
                "corpus import",
                &st(Some("s111111111111"))
            ),
            Decision::RunHeld
        );
        // A step that runs something else, or one not open.
        assert!(matches!(
            decide_step(w.clone(), "corpus import", &st(Some("s222222222222"))),
            Decision::Refused(r) if r.contains("doesn't run gp corpus import")
        ));
        assert!(matches!(
            decide_step(w.clone(), "corpus import", &st(Some("s999999999999"))),
            Decision::Refused(r) if r.contains("isn't open now")
        ));
        // A landing needs its step too; a live worker's step still counts.
        assert!(matches!(
            decide_step(Decision::NeedsYes(Ask::Publish), "model gate", &st(None)),
            Decision::Refused(_)
        ));
        assert_eq!(
            decide_step(w.clone(), "mechanic say", &st(Some("s333333333333"))),
            w
        );
        // A worker's gp may cite no step but its own, and cites it unasked.
        let bound = Steps {
            bound: Some("s111111111111"),
            ..st(None)
        };
        assert_eq!(decide_step(w.clone(), "corpus import", &bound), w);
        let other = Steps {
            cited: Some("s333333333333"),
            ..bound
        };
        assert!(matches!(
            decide_step(w.clone(), "mechanic say", &other),
            Decision::Refused(r) if r.contains("started for step s111111111111")
        ));
    }

    #[test]
    fn a_halted_loop_runs_no_step_but_lets_a_worker_finish_its_writes() {
        let open = [
            json!({"id": "s111111111111", "who": "agent", "subject": "g", "says": "Add.", "commands": ["corpus import"]}),
        ];
        let held = [
            json!({"id": "s333333333333", "who": "agent", "subject": "h", "says": "Fix.", "commands": ["mechanic say", "model gate"]}),
        ];
        let st = |cited| Steps {
            cited,
            open: &open,
            held: &held,
            halted: Some("step s111111111111 was marked wrong: it adds the wrong clips"),
            bound: None,
        };
        let w = Decision::Run { writes: true };
        assert!(matches!(
            decide_step(w.clone(), "corpus import", &st(Some("s111111111111"))),
            Decision::Refused(r) if r.starts_with("the loop is halted: step s111111111111 was marked wrong")
        ));
        assert_eq!(
            decide_step(w.clone(), "mechanic say", &st(Some("s333333333333"))),
            w
        );
        assert!(matches!(
            decide_step(Decision::NeedsYes(Ask::Publish), "model gate", &st(Some("s333333333333"))),
            Decision::Refused(r) if r.contains("Nothing lands or publishes")
        ));
    }
}
