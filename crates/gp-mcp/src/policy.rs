//! What `gp_run` may run, as one table: every gp command by its path, the
//! road it belongs to and the rule that governs running it from a tool.
//! gp-cli's tests walk the whole command tree and fail when a command has
//! no row here or a row names no command, so a new command can't reach an
//! agent unclassified. Each row was set by reading what the command writes
//! (the audit's slice files have one entry per command), not by its name.
//!
//! The rules, from the owner's standing rules:
//! - read: writes nothing outside the home's work and cache areas.
//! - write: writes the person's games, working copies or home.
//! - ask: publishes, uploads, pushes or lands a change through a gate;
//!   spends money or reads a secret or answer key; or downloads a lot.
//!   It runs only when the call carries `confirmed_by_person`, unless one
//!   of its dry-run flags is given.
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
}

impl Ask {
    pub fn name(self) -> &'static str {
        match self {
            Ask::Publish => "publishes",
            Ask::Spend => "spends",
            Ask::Download => "downloads",
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
    /// Needs the person's yes when the argument is given; else the base.
    AskWith(Ask, &'static str, Base),
    /// Refused; the text says what to use instead.
    Refuse(&'static str),
    /// Refused when the argument is given; else the base.
    RefuseWith(&'static str, &'static str, Base),
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
            Rule::AskWith(a, arg, _) => format!(
                "Runs at once, but with `{arg}` given it {} and needs the person's yes (confirmed_by_person).",
                a.name()
            ),
            Rule::Refuse(instead) => format!("Refused from a tool: {instead}"),
            Rule::RefuseWith(arg, instead, _) => {
                format!("Runs at once, but refused with `{arg}`: {instead}")
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
    r("assets stub", J, Write),
    r("corpus bump", J, Write),
    r("corpus diff", J, Read),
    r("corpus import", J, Write),
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
    r("registry init", J, Write),
    r("registry publish", J, PUBLISH),
    r("registry reindex", J, PUBLISH),
    r("registry serve", J, Rule::Refuse(SERVE)),
    r("roms add", J, Write),
    r("roms clean", J, Write),
    r("roms diff", J, Read),
    r("roms scan", J, Read),
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
    r("theme paint", J, Write),
    r("theme publish", O, Rule::Ask(Ask::Publish, DRY_RUN)),
    r("theme recheck", O, Rule::Ask(Ask::Publish, DRY_RUN)),
    r("theme schema", D, Read),
    r("theme sheet", J, Write),
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
    r("capture background", S, Write),
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
        Rule::AskWith(Ask::Spend, "key", Base::Read),
    ),
    r("dataset rom", S, Write),
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
    r("level kit", J, Write),
    r("level measure", J, Read),
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
];

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

/// What gp_run does with a call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Run it; `writes` when it may write the person's things.
    Run { writes: bool },
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
    let base = |b: Base| Decision::Run {
        writes: b == Base::Write,
    };
    let decision = match row.rule {
        Rule::Read => base(Base::Read),
        Rule::Write => base(Base::Write),
        Rule::Ask(_, free) if free.iter().any(|f| given(f)) => base(Base::Read),
        Rule::Ask(a, _) => Decision::NeedsYes(a),
        Rule::AskWith(a, arg, _) if given(arg) => Decision::NeedsYes(a),
        Rule::AskWith(_, _, b) => base(b),
        Rule::Refuse(why) => Decision::Refused(why.to_owned()),
        Rule::RefuseWith(arg, why, _) if given(arg) => Decision::Refused(why.to_owned()),
        Rule::RefuseWith(_, _, b) => base(b),
    };
    // A target in git or a bucket publishes, whatever the command.
    if let Decision::Run { .. } = decision
        && values
            .iter()
            .any(|(_, v)| v.starts_with("git:") || v.starts_with("s3://"))
    {
        return Decision::NeedsYes(Ask::Publish);
    }
    decision
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
            "means": "writes the person's games, working copies or home; runs at once, and the reply says what it wrote when the command reports it",
        },
        "ask": {
            "commands": count(&|r| matches!(r, Rule::Ask(..) | Rule::AskWith(..))),
            "means": "publishes, uploads, pushes or lands a change through a gate; spends money, calls a paid service or reads a secret or answer key; or downloads a lot. Runs only when the call has confirmed_by_person: true, which you may set only after the person said yes to that exact action in this conversation. Its dry-run flag, where it has one, runs at once. Some need a yes only with one argument: their rule says which",
        },
        "refused": {
            "commands": count(&|r| matches!(r, Rule::Refuse(_) | Rule::RefuseWith(..))),
            "means": "serves until stopped, waits with no bound, opens a program on the person's screen, or runs any program: refused from a tool, and the refusal names what to use instead. Some are refused only with one argument: their rule says which. Ports 7710 and 7720 are never bound from a tool",
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let generate = row("theme generate").unwrap();
        assert_eq!(
            decide(generate, &given(&[]), &[]),
            Decision::NeedsYes(Ask::Spend)
        );
        for p in [
            "registry publish",
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
}
