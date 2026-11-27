//! The hint a command prints first, on stderr: which workflow it belongs
//! to, which step, the skill or MCP tool that drives it, and what usually
//! comes next, so an agent running one command knows where it is. stdout
//! and `--json` are left as they are. `GP_NO_HINTS=1` turns it off.
//!
//! The workflows are the skills in .claude/skills and the site's pages
//! (site/src/content/docs). Each one's steps are listed once in its
//! [`Flow`]; a row names a command, its flow, its step and what comes next.
//! A command no workflow uses has no row and prints nothing. The tests
//! check every row's command against the clap tree and every MCP tool
//! against gp-mcp's list, so a renamed command or tool fails them.

/// One workflow and its steps, in order.
pub struct Flow {
    pub name: &'static str,
    /// The skill that drives it, under .claude/skills.
    pub skill: Option<&'static str>,
    /// The MCP tools (gp mcp) that drive it.
    pub mcp: &'static [&'static str],
    /// The page that describes it, when no skill does.
    pub doc: Option<&'static str>,
    pub steps: &'static [&'static str],
}

/// Porting a game from a ROM: the checklist in bring.md.
pub const PORT: Flow = Flow {
    name: "porting a game",
    skill: None,
    mcp: &["next"],
    doc: Some("site page Bring in a game, bring.md"),
    steps: &[
        "copy the ROM in and identify it",
        "compare revisions",
        "start the game",
        "check the skeleton",
        "land the skeleton",
        "verify the source",
        "find a movie that syncs",
        "name the stage byte",
        "find the player's bytes",
        "set the scope",
        "record the first clips",
        "find the random draws",
        "score a clip",
        "measure what a clip shows",
        "model each mechanic",
        "land each change",
        "ask for clips no sweep can make",
        "check the picture",
        "record the held-out runs",
        "score the held-out runs",
    ],
};

/// One element from a ROM into the model, code first.
pub const ELEMENT: Flow = Flow {
    name: "element extraction",
    skill: Some("element-extraction"),
    mcp: &[],
    doc: None,
    steps: &[
        "element to assets",
        "assets to code",
        "code to inputs",
        "inputs to the model",
        "branches to clips",
        "clips to differences",
        "differences to the answer",
        "answer to the fix",
    ],
};

/// A large-art theme, the theme-generation skill's numbered steps. Its
/// steps change as the theme tools grow: edit this list and the rows
/// that name THEME below.
pub const THEME: Flow = Flow {
    name: "the theme procedure",
    skill: Some("theme-generation"),
    mcp: &["next"],
    doc: None,
    steps: &[
        "the game is ready to skin",
        "every stage and room on record",
        "the kit lists every picture to draw",
        "words for every frame",
        "grids and prompts",
        "paid art, after the person's yes",
        "fit and write the rules",
        "compare with the original, then show the person",
        "ship it",
    ],
};

/// A new stage by hand from a game's level kit.
pub const LEVEL: Flow = Flow {
    name: "level design",
    skill: Some("level-design"),
    mcp: &["template_fork", "gp_run"],
    doc: None,
    steps: &[
        "read the kit",
        "lay the stage out on a grid",
        "check the reach by hand",
        "verify",
        "theme it",
    ],
};

/// A registry and a page of the person's own (host_your_own's steps).
pub const HOST: Flow = Flow {
    name: "hosting your own",
    skill: None,
    mcp: &["host_your_own"],
    doc: Some("site page Host your game or theme, host-your-own.md"),
    steps: &[
        "make a public GitHub repo for the registry",
        "make it a registry",
        "grant your namespace",
        "publish the theme and list it",
        "check it from outside",
        "open the public page with your registry",
        "build and deploy a page of your own",
    ],
};

/// A game of the person's own, from nothing or a template (the new-game
/// copy's steps that `next` gives).
pub const NEW: Flow = Flow {
    name: "a game of your own",
    skill: None,
    mcp: &["next", "template_fork"],
    doc: Some("site page Make your own game, make.md"),
    steps: &[
        "start from nothing or a template",
        "name it",
        "see it play",
        "change a stage",
        "give it a look",
        "play and share",
    ],
};

/// A fix to a published game, sent back through the gate (improve.md).
pub const IMPROVE: Flow = Flow {
    name: "improving a game",
    skill: None,
    mcp: &[],
    doc: Some("site page Improve a game, improve.md"),
    steps: &[
        "make the change in your fork",
        "run the gate with --dry-run",
        "open a pull request with the gate's result",
    ],
};

/// One command's place in one workflow.
pub struct Row {
    /// The command's path, as `gp self commands` names it.
    pub cmd: &'static str,
    pub flow: &'static Flow,
    /// The step's number in the flow; 0 for a command used at any step.
    /// `last` makes it a range.
    pub step: u8,
    pub last: u8,
    /// What usually comes next. Empty: the flow's next step.
    pub next: &'static str,
    /// MCP tools for this command beyond the flow's own.
    pub mcp: &'static [&'static str],
}

const fn r(cmd: &'static str, flow: &'static Flow, step: u8, next: &'static str) -> Row {
    Row {
        cmd,
        flow,
        step,
        last: step,
        next,
        mcp: &[],
    }
}

const fn steps(
    cmd: &'static str,
    flow: &'static Flow,
    step: u8,
    last: u8,
    next: &'static str,
) -> Row {
    Row {
        cmd,
        flow,
        step,
        last,
        next,
        mcp: &[],
    }
}

const fn tool(
    cmd: &'static str,
    flow: &'static Flow,
    step: u8,
    next: &'static str,
    mcp: &'static [&'static str],
) -> Row {
    Row {
        cmd,
        flow,
        step,
        last: step,
        next,
        mcp,
    }
}

/// Every hint. A command's first row is the one it is mostly run for;
/// its other rows are said as "also".
pub const ROWS: &[Row] = &[
    // Porting a game.
    r(
        "progress",
        &PORT,
        0,
        "the step it names; gp progress --next again after it",
    ),
    r(
        "roms add",
        &PORT,
        1,
        "gp roms scan names the file by its hash",
    ),
    r(
        "roms scan",
        &PORT,
        1,
        "gp roms diff if there are several revisions, else gp game new GAME --rom ROM (step 3)",
    ),
    r(
        "roms diff",
        &PORT,
        2,
        "gp game new GAME --rom ROM with the chosen revision",
    ),
    r(
        "game new",
        &PORT,
        3,
        "gp model check, gp template check and gp package scan (step 4)",
    ),
    r(
        "game new",
        &NEW,
        1,
        "name it, then gp template build DIR to see it play (step 3)",
    ),
    r(
        "model check",
        &PORT,
        4,
        "gp model gate GAME --patch FILE -m MSG lands it (step 5)",
    ),
    r("model check", &LEVEL, 4, "gp level measure --template T"),
    r(
        "template check",
        &PORT,
        4,
        "gp model gate GAME --patch FILE -m MSG lands it (step 5)",
    ),
    r(
        "template check",
        &LEVEL,
        4,
        "gp level measure --template T, then gp level difficulty",
    ),
    r(
        "package scan",
        &PORT,
        4,
        "gp model gate GAME --patch FILE -m MSG lands it (step 5)",
    ),
    r("model gate", &PORT, 5, "a movie that syncs (step 7)"),
    r(
        "model gate",
        &PORT,
        16,
        "the next quest log row, from gp mechanic queue GAME",
    ),
    r(
        "model gate",
        &ELEMENT,
        8,
        "the done checks: gp picture, gp model fmt --check",
    ),
    r(
        "model gate",
        &IMPROVE,
        2,
        "on PASS, a pull request with the gate's result",
    ),
    r(
        "movie import",
        &PORT,
        7,
        "gp movie diagnose, then the stage byte and gp mechanic harvest (step 8)",
    ),
    r(
        "movie diagnose",
        &PORT,
        7,
        "the stage byte in observe.toml, then gp mechanic harvest GAME RECORDING --add (step 8)",
    ),
    r(
        "mechanic harvest",
        &PORT,
        8,
        "a walk clip, then gp model blind for the player's bytes (step 9)",
    ),
    r(
        "model blind",
        &PORT,
        9,
        "the player's x and y into mapping.toml, then its [scope] (step 10)",
    ),
    r(
        "labels lookup",
        &PORT,
        9,
        "the player's x and y into mapping.toml, then its [scope] (step 10)",
    ),
    r(
        "labels propose",
        &PORT,
        9,
        "the player's x and y into mapping.toml, then its [scope] (step 10)",
    ),
    r(
        "labels show",
        &ELEMENT,
        1,
        "gp element inputs --run GAME --clip ID for its routines and inputs (steps 2 and 3)",
    ),
    r(
        "mechanic start",
        &PORT,
        9,
        "record on the CLIPS tab and hand off, then gp corpus import GAME --write (step 11)",
    ),
    r("mechanic start", &PORT, 11, "gp corpus import GAME --write"),
    r(
        "corpus import",
        &PORT,
        11,
        "gp interp draws --find for the random draws (step 12), then gp clip score (step 13)",
    ),
    r(
        "clip presses",
        &PORT,
        11,
        "gp interp draws --find for the random draws (step 12), then gp clip score (step 13)",
    ),
    r(
        "clip log",
        &PORT,
        12,
        "gp interp draws --find ROM --input LOG",
    ),
    r(
        "interp draws",
        &PORT,
        12,
        "the proposed [[draws]] into mapping.toml, then gp clip score GAME SESSION:N (step 13)",
    ),
    r(
        "clip score",
        &PORT,
        13,
        "gp mechanic probe GAME --import SESSION:N keeps its measures (step 14)",
    ),
    r(
        "clip score",
        &ELEMENT,
        6,
        "gp solve GAME --run RUN --missing, gp interp moments (step 7)",
    ),
    r("clip screen", &PORT, 13, "gp clip score GAME SESSION:N"),
    r(
        "corpus score",
        &PORT,
        13,
        "gp mechanic probe to keep a clip's measures (step 14)",
    ),
    r(
        "corpus score",
        &ELEMENT,
        6,
        "gp solve GAME --run RUN --missing, gp interp moments (step 7)",
    ),
    r(
        "corpus score",
        &THEME,
        1,
        "gp theme look sheet --game GAME (step 2)",
    ),
    r(
        "mechanic probe",
        &PORT,
        14,
        "each mechanic with the element-extraction skill (step 15)",
    ),
    r(
        "mechanic probe",
        &ELEMENT,
        5,
        "gp clip score GAME SESSION:N or gp corpus score GAME --only ID (step 6)",
    ),
    r(
        "mechanic queue",
        &PORT,
        15,
        "the next row's clips; land each change with gp model gate (step 16)",
    ),
    r(
        "mechanic leads",
        &PORT,
        17,
        "the person records the rows it adds on the CLIPS tab; then gp picture GAME --run RUN (step 18)",
    ),
    r(
        "mechanic sweep",
        &PORT,
        17,
        "gp corpus import GAME --write, then gp picture GAME --run RUN (step 18)",
    ),
    r(
        "picture",
        &PORT,
        18,
        "the held-out runs (step 19), then gp sealed score (step 20)",
    ),
    r(
        "picture",
        &THEME,
        1,
        "gp theme look sheet --game GAME (step 2)",
    ),
    r(
        "sealed score",
        &PORT,
        20,
        "gp progress --next for what is left",
    ),
    // Element extraction.
    steps(
        "element explain",
        &ELEMENT,
        1,
        3,
        "compare the inputs with the model's rule: gp model mechanics (step 4)",
    ),
    r(
        "element inputs",
        &ELEMENT,
        3,
        "compare the inputs with the model's rule: gp model check, gp model mechanics (step 4)",
    ),
    r(
        "element inputs",
        &PORT,
        15,
        "gp model refine for the numbers; land with gp model gate (step 16)",
    ),
    r(
        "agent brief",
        &ELEMENT,
        3,
        "the commands it writes out, then gp model mechanics (step 4)",
    ),
    r(
        "code listing",
        &ELEMENT,
        2,
        "gp element inputs for what each routine reads (step 3)",
    ),
    r(
        "code tables",
        &ELEMENT,
        2,
        "gp element inputs for what each routine reads (step 3)",
    ),
    r(
        "model mechanics",
        &ELEMENT,
        4,
        "clips that take every branch: gp capture dense, gp capture carry, gp mechanic probe (step 5)",
    ),
    r(
        "capture dense",
        &ELEMENT,
        5,
        "gp clip score or gp corpus score --only ID (step 6)",
    ),
    r(
        "capture carry",
        &ELEMENT,
        5,
        "gp clip score or gp corpus score --only ID (step 6)",
    ),
    r(
        "interp scenarios",
        &ELEMENT,
        5,
        "gp clip score or gp corpus score --only ID (step 6)",
    ),
    r(
        "interp compare",
        &ELEMENT,
        6,
        "gp solve GAME --run RUN --missing, gp interp moments (step 7)",
    ),
    r(
        "interp slots",
        &ELEMENT,
        6,
        "gp solve GAME --run RUN --missing, gp interp moments (step 7)",
    ),
    r(
        "agent score",
        &ELEMENT,
        6,
        "gp solve GAME --run RUN --missing, gp interp moments (step 7)",
    ),
    r(
        "solve",
        &ELEMENT,
        7,
        "a fix that reads the missing input, with the code cited: gp model refine (step 8)",
    ),
    tool(
        "interp moments",
        &ELEMENT,
        7,
        "gp model refine --lockstep on that clip and moment (step 8)",
        &["moments"],
    ),
    tool(
        "model refine",
        &ELEMENT,
        8,
        "score the fix, then land it with gp model gate GAME --patch FILE -m MSG",
        &["model_refine"],
    ),
    r(
        "model refine",
        &PORT,
        15,
        "land each change with gp model gate (step 16)",
    ),
    r(
        "model refine",
        &IMPROVE,
        1,
        "gp model questions for what it couldn't fix, then the gate with --dry-run (step 2)",
    ),
    r(
        "agent attempt",
        &ELEMENT,
        8,
        "gp agent score for the try, then land it with gp model gate",
    ),
    r(
        "model fmt",
        &ELEMENT,
        8,
        "the done checks: gp picture GAME --run RUN, and the notes log's row",
    ),
    // Improving a game.
    tool(
        "model questions",
        &IMPROVE,
        1,
        "copy the numbers you want into your fork, then the gate with --dry-run (step 2)",
        &["model_questions"],
    ),
    r(
        "model propose",
        &IMPROVE,
        1,
        "gp model refine writes the values a failing check backs up",
    ),
    tool(
        "validate",
        &IMPROVE,
        1,
        "gp model refine for what fails, then the gate with --dry-run (step 2)",
        &["validate"],
    ),
    r(
        "model derive",
        &IMPROVE,
        1,
        "answer where the facts and their evidence differ, then the gate with --dry-run (step 2)",
    ),
    r(
        "game kit",
        &IMPROVE,
        1,
        "gp game build GAME --kit KIT -o MODULE.wasm",
    ),
    r(
        "game build",
        &IMPROVE,
        1,
        "gp model refine GAME --lockstep --model COPY --module MODULE.wasm",
    ),
    // The theme procedure.
    r(
        "theme look sheet",
        &THEME,
        2,
        "a clip for any room missing from names.toml, then gp theme kit GAME (step 3)",
    ),
    r(
        "theme kit",
        &THEME,
        3,
        "words.toml, one phrase per frame and a deny list (step 4), then gp theme sheet (step 5)",
    ),
    r(
        "theme sheet",
        &THEME,
        5,
        "paid art after the person's yes (step 6), then gp theme fit (step 7)",
    ),
    r(
        "theme look new",
        &THEME,
        7,
        "gp theme fit into it, then gp theme look check THEME --game GAME",
    ),
    r(
        "theme look new",
        &NEW,
        5,
        "gp theme look check THEME --game GAME",
    ),
    r(
        "theme fit",
        &THEME,
        7,
        "gp theme look check THEME --game GAME until it reports no problems",
    ),
    r(
        "theme look cut",
        &THEME,
        7,
        "its pic line in theme/art.txt, then gp theme look check THEME --game GAME",
    ),
    r(
        "theme look check",
        &THEME,
        7,
        "gp theme look play THEME --game GAME --shots DIR --against Original (step 8)",
    ),
    r(
        "theme picture propose",
        &THEME,
        7,
        "the person's ACCEPT or REJECT on the page; gp theme picture list reads it",
    ),
    r(
        "theme picture list",
        &THEME,
        7,
        "after an ACCEPT, gp theme look check again",
    ),
    r(
        "theme picture accept",
        &THEME,
        7,
        "gp theme look check again",
    ),
    r(
        "theme picture reject",
        &THEME,
        7,
        "read what the person wrote and propose again",
    ),
    r("theme picture undo", &THEME, 7, "gp theme look check again"),
    r(
        "theme sound propose",
        &THEME,
        7,
        "the person's ACCEPT or REJECT on the page; gp theme sound list reads it",
    ),
    r(
        "theme sound list",
        &THEME,
        7,
        "after an ACCEPT, gp theme look check again",
    ),
    r("theme sound accept", &THEME, 7, "gp theme look check again"),
    r(
        "theme sound reject",
        &THEME,
        7,
        "read what the person wrote and propose again",
    ),
    r("theme sound undo", &THEME, 7, "gp theme look check again"),
    r(
        "theme look play",
        &THEME,
        8,
        "the theme into GAME/themes/NAME in the games repo, then gp registry publish (step 9)",
    ),
    r(
        "theme look take",
        &THEME,
        8,
        "git diff in the games checkout into a patch, then gp model gate GAME --patch FILE --dry-run",
    ),
    // Hosting your own, and shipping a theme.
    r(
        "registry host",
        &HOST,
        0,
        "the first step it shows as not done",
    ),
    r(
        "registry init",
        &HOST,
        2,
        "gp registry grant for your namespace (step 3)",
    ),
    r(
        "registry grant",
        &HOST,
        3,
        "gp registry publish, then gp registry reindex --keep NAME@VERSION (step 4)",
    ),
    r(
        "registry publish",
        &HOST,
        4,
        "gp registry reindex --keep NAME@VERSION",
    ),
    r(
        "registry publish",
        &THEME,
        9,
        "gp registry reindex, then gp registry check --for GAME --theme NAME",
    ),
    r(
        "registry reindex",
        &HOST,
        4,
        "gp registry check RAW_URL --for GAME --theme NAME (step 5)",
    ),
    r(
        "registry reindex",
        &THEME,
        9,
        "gp registry check --for GAME --theme NAME",
    ),
    r(
        "registry check",
        &HOST,
        5,
        "open the public page with r= set to your registry (step 6)",
    ),
    r(
        "registry check",
        &THEME,
        9,
        "the theme is offered, or the reason it isn't",
    ),
    r(
        "registry yank",
        &THEME,
        9,
        "gp registry reindex --keep --drop NAME@VERSION takes it off the list",
    ),
    // Level design.
    r(
        "level kit",
        &LEVEL,
        1,
        "read GAME/level-kit/level-kit.md top to bottom, then lay the stage out (step 2)",
    ),
    r("level list", &LEVEL, 1, "gp level show COPY --level main"),
    r(
        "level show",
        &LEVEL,
        1,
        "gp level edit COPY --op ... (step 2)",
    ),
    r("level show", &NEW, 4, "gp level edit COPY --op ..."),
    r(
        "level render",
        &LEVEL,
        2,
        "show the picture to the person; gp level verify COPY (step 4)",
    ),
    r(
        "level edit",
        &LEVEL,
        2,
        "gp level render COPY, then gp level verify COPY (step 4); gp level undo takes it back",
    ),
    r("level edit", &NEW, 4, "gp level verify COPY"),
    r(
        "level undo",
        &LEVEL,
        2,
        "another gp level edit, or gp level verify COPY (step 4)",
    ),
    r(
        "template derive",
        &LEVEL,
        2,
        "the stage's slots in your own numbers, then gp model check T (step 4)",
    ),
    r(
        "level verify",
        &LEVEL,
        4,
        "show the person; then theme it with the theme-generation skill (step 5)",
    ),
    r("level verify", &NEW, 4, "give it a look (step 5)"),
    r(
        "level measure",
        &LEVEL,
        4,
        "move what an out-of-tolerance measure names and measure again; then gp level difficulty",
    ),
    r("level difficulty", &LEVEL, 4, "theme it (step 5)"),
    r("template build", &LEVEL, 4, "gp template check T"),
    r(
        "template build",
        &NEW,
        3,
        "play it, then change a stage with gp level show and gp level edit (step 4)",
    ),
    r(
        "interp baselines",
        &LEVEL,
        4,
        "with --write only when you meant to change how the game plays",
    ),
    // A game of your own.
    r("game share", &NEW, 6, ""),
];

/// A step, or a range of them, in words.
fn step_words(row: &Row) -> String {
    let n = row.flow.steps.len();
    if row.step == 0 {
        format!("any step of {n}")
    } else if row.last > row.step {
        format!("steps {} to {} of {n}", row.step, row.last)
    } else {
        format!(
            "step {} of {n}, {}",
            row.step,
            row.flow.steps[usize::from(row.step) - 1]
        )
    }
}

/// Who drives a row's step: its skill, MCP tools and page.
fn drivers(row: &Row) -> String {
    let mut parts = Vec::new();
    if let Some(s) = row.flow.skill {
        parts.push(format!("skill {s}"));
    }
    let tools: Vec<&str> = row.flow.mcp.iter().chain(row.mcp).copied().collect();
    if !tools.is_empty() {
        parts.push(format!("MCP {}", tools.join(", ")));
    }
    if let Some(d) = row.flow.doc {
        parts.push(d.to_owned());
    }
    parts.join("; ")
}

fn next_words(row: &Row) -> String {
    if !row.next.is_empty() {
        return row.next.to_owned();
    }
    match row.flow.steps.get(usize::from(row.last)) {
        Some(s) if row.step > 0 => format!("step {}, {s}", row.last + 1),
        _ => "the workflow's last step is done".to_owned(),
    }
}

/// The hint for a command path (`theme sheet`), one or two lines, or
/// None for a command no workflow uses.
pub fn hint(cmd: &str) -> Option<String> {
    let mut rows = ROWS.iter().filter(|r| r.cmd == cmd);
    let first = rows.next()?;
    let mut out = format!(
        "hint: gp {cmd} belongs to {}, {} ({}). Next: {}.",
        first.flow.name,
        step_words(first),
        drivers(first),
        next_words(first)
    );
    let also: Vec<String> = rows
        .map(|r| {
            let at = if r.step == 0 {
                "any step".to_owned()
            } else if r.last > r.step {
                format!("steps {} to {}", r.step, r.last)
            } else {
                format!("step {}", r.step)
            };
            format!("{} {at}", r.flow.name)
        })
        .collect();
    if !also.is_empty() {
        out.push_str(&format!("\nhint: also {}.", also.join("; ")));
    }
    Some(out)
}

/// Prints a command's hint on stderr, unless GP_NO_HINTS is set.
pub fn print(cmd: &str) {
    if std::env::var("GP_NO_HINTS").is_ok_and(|v| !v.is_empty() && v != "0") {
        return;
    }
    if let Some(h) = hint(cmd) {
        eprintln!("{h}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn paths(c: &clap::Command, prefix: &str, out: &mut BTreeSet<String>) {
        for s in c.get_subcommands() {
            let p = if prefix.is_empty() {
                s.get_name().to_owned()
            } else {
                format!("{prefix} {}", s.get_name())
            };
            paths(s, &p, out);
            out.insert(p);
        }
    }

    #[test]
    fn every_command_in_the_table_is_in_the_clap_tree() {
        let mut all = BTreeSet::new();
        paths(
            &<crate::Cli as clap::CommandFactory>::command(),
            "",
            &mut all,
        );
        let missing: Vec<&str> = ROWS
            .iter()
            .map(|r| r.cmd)
            .filter(|c| !all.contains(*c))
            .collect();
        assert!(
            missing.is_empty(),
            "hints name commands gp lacks: {missing:?}"
        );
    }

    #[test]
    fn every_mcp_tool_cited_is_served() {
        let served: BTreeSet<&str> = gp_mcp::tools::list().iter().map(|t| t.name).collect();
        let flows = [&PORT, &ELEMENT, &THEME, &LEVEL, &HOST, &NEW, &IMPROVE];
        let cited = flows
            .iter()
            .flat_map(|f| f.mcp.iter())
            .chain(ROWS.iter().flat_map(|r| r.mcp.iter()));
        let missing: Vec<&&str> = cited.filter(|t| !served.contains(**t)).collect();
        assert!(
            missing.is_empty(),
            "hints cite MCP tools gp mcp lacks: {missing:?}"
        );
    }

    #[test]
    fn every_skill_cited_is_in_the_repo() {
        let skills = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.claude/skills");
        for f in [&PORT, &ELEMENT, &THEME, &LEVEL, &HOST, &NEW, &IMPROVE] {
            if let Some(s) = f.skill {
                assert!(skills.join(s).join("SKILL.md").is_file(), "no skill {s}");
            }
        }
    }

    #[test]
    fn steps_are_in_range_and_rows_are_once_per_flow() {
        let mut seen = BTreeSet::new();
        for r in ROWS {
            let n = r.flow.steps.len() as u8;
            assert!(
                r.step <= n && r.last <= n && r.last >= r.step,
                "{} step {}",
                r.cmd,
                r.step
            );
            assert!(
                seen.insert((r.cmd, r.flow.name, r.step)),
                "{} twice in {}",
                r.cmd,
                r.flow.name
            );
        }
    }

    #[test]
    fn hints_are_short_plain_ascii() {
        for r in ROWS {
            let h = hint(r.cmd).unwrap();
            assert!(h.lines().count() <= 2, "{h}");
            assert!(h.is_ascii(), "{h}");
        }
        assert!(hint("noop").is_none());
        assert_eq!(
            hint("theme sheet").unwrap(),
            "hint: gp theme sheet belongs to the theme procedure, step 5 of 9, grids and prompts (skill theme-generation; MCP next). Next: paid art after the person's yes (step 6), then gp theme fit (step 7)."
        );
    }
}
