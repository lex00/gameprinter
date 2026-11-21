//! The checks that keep gp's MCP server current with gp (docs/mcp.md,
//! "Keeping up"). They run with the tests in `just check` and fail on
//! drift, each saying what to change: the server's instructions name
//! every named tool and no gone one, every gp command the instructions,
//! the tools' descriptions and a worker's brief name is one gp has (with
//! its flags), every `mcp__gp__` tool anything in the repo names is one the
//! server has, docs/mcp.md's table has a row for every tool, and a plugin's
//! manifest carries gp's version. gp_run's policy holding a row for every
//! command is cli_tree.rs's check.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use clap::CommandFactory;
use serde_json::Value;

/// Every named tool's name.
pub(crate) fn tool_names() -> BTreeSet<&'static str> {
    gp_mcp::tools::list().iter().map(|t| t.name).collect()
}

/// Every argument name any named tool takes, from their schemas.
fn arg_names() -> BTreeSet<String> {
    fn walk(v: &Value, out: &mut BTreeSet<String>) {
        match v {
            Value::Object(o) => {
                if let Some(Value::Object(p)) = o.get("properties") {
                    out.extend(p.keys().cloned());
                }
                o.values().for_each(|x| walk(x, out));
            }
            Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
            _ => {}
        }
    }
    let mut out = BTreeSet::new();
    for t in gp_mcp::tools::list() {
        walk(&t.input_schema, &mut out);
    }
    out
}

/// A text's words: runs of letters, digits and `_`.
fn words(text: &str) -> BTreeSet<&str> {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
        .collect()
}

/// The snake_case words of a text (`run_preview`, `wait_s`), not
/// `mcp__gp__` names.
fn snake_words(text: &str) -> BTreeSet<&str> {
    words(text)
        .into_iter()
        .filter(|w| {
            w.contains('_')
                && !w.contains("__")
                && !w.starts_with('_')
                && !w.ends_with('_')
                && w.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        })
        .collect()
}

/// The words that follow a bare `gp` in prose and aren't a command's
/// ("the gp this server runs", "this same gp with --json").
const PROSE: &[&str] = &[
    "a", "an", "and", "as", "at", "binary", "by", "command", "commands", "does", "for", "from",
    "has", "in", "is", "it", "its", "needs", "reads", "takes", "writes", "itself", "of", "on",
    "or", "runs", "serving", "that", "the", "this", "to", "with",
];

/// A word as a command's: without the quotes and brackets around it, the
/// punctuation after it or a possessive `'s`.
fn clean(w: &str) -> &str {
    let w = w.trim_start_matches(['(', '"', '`', '\'']);
    let w = w.trim_end_matches([',', '.', ';', ':', ')', '"', '`', '\'']);
    w.strip_suffix("'s").unwrap_or(w)
}

/// Whether a raw word ends the phrase it is in.
fn ends_phrase(w: &str) -> bool {
    w.ends_with([',', '.', ';', ':', ')'])
}

fn is_word(w: &str) -> bool {
    w.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && w.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// The gp commands a text names that gp hasn't, each as the text has it.
/// After a `gp`, the words must lead through gp's command tree: a group's
/// next word one of its subcommands (or no command's word at all: a
/// GAME, the end of the phrase), and a first word a command's or plain
/// English. Each `--flag` in the phrase after a command must be one of its
/// own or a global flag. After `gp_run`, a first word that is no command's
/// is the prose's ("gp_run runs reads"), and the rest is held as after
/// `gp`.
pub(crate) fn unknown_commands(text: &str) -> Vec<String> {
    let mut root = crate::Cli::command();
    root.build();
    let globals: BTreeSet<String> = root
        .get_arguments()
        .filter_map(|a| a.get_long().map(str::to_owned))
        .chain(["help".to_owned(), "version".to_owned()])
        .collect();
    let raw: Vec<&str> = text.split_whitespace().collect();
    let mut out = Vec::new();
    for (i, w) in raw.iter().enumerate() {
        let lead = w.trim_start_matches(['(', '"', '`']);
        let run = match lead {
            "gp" => false,
            "gp_run" => true,
            _ => continue,
        };
        let mut at = &root;
        let mut path: Vec<&str> = Vec::new();
        let mut j = i + 1;
        while let Some(x) = raw.get(j) {
            let c = clean(x);
            // "gp_run runs reads" is prose, though gp has a runs group.
            if !is_word(c) || path.is_empty() && PROSE.contains(&c) {
                break;
            }
            match at.find_subcommand(c).filter(|s| s.get_name() != "help") {
                Some(s) => {
                    at = s;
                    path.push(c);
                    j += 1;
                    if ends_phrase(x) || !at.has_subcommands() {
                        break;
                    }
                }
                None => {
                    // A group that runs only through its subcommands.
                    let group_alone = at.has_subcommands()
                        && at.get_arguments().all(|a| {
                            a.is_global_set() || a.get_long().is_some_and(|l| globals.contains(l))
                        });
                    let wrong =
                        !PROSE.contains(&c) && if path.is_empty() { !run } else { group_alone };
                    if wrong {
                        let mut p = path.clone();
                        p.push(c);
                        out.push(format!("gp {}", p.join(" ")));
                    }
                    break;
                }
            }
        }
        if path.is_empty() {
            continue;
        }
        // The flags in the rest of the phrase, up to the next gp.
        if !ends_phrase(raw[j - 1]) {
            for x in &raw[j..] {
                let lead = x.trim_start_matches(['(', '"', '`']);
                if lead == "gp" || lead == "gp_run" {
                    break;
                }
                let f = clean(x);
                if let Some(long) = f.strip_prefix("--") {
                    let long = long.split('=').next().unwrap_or(long);
                    let own = at.get_arguments().any(|a| a.get_long() == Some(long));
                    if !long.is_empty() && !own && !globals.contains(long) {
                        out.push(format!("gp {} --{long}", path.join(" ")));
                    }
                }
                if ends_phrase(x) {
                    break;
                }
            }
        }
    }
    out
}

/// The repo this crate is in.
fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The repo's files git tracks, by path from its root.
fn tracked() -> Vec<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo())
        .args(["ls-files", "-z"])
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git ls-files in {}", repo().display());
    String::from_utf8_lossy(&out.stdout)
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The tools a text names as `mcp__gp__NAME` (`mcp__gp__*` names none).
fn mcp_names(text: &str) -> BTreeSet<&str> {
    text.match_indices("mcp__gp__")
        .map(|(i, m)| {
            let rest = &text[i + m.len()..];
            let n = rest
                .find(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'))
                .unwrap_or(rest.len());
            &rest[..n]
        })
        .filter(|n| !n.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The instructions a client reads on connecting name every named
    /// tool, and every snake_case word in them is a tool, an argument a
    /// tool takes or a field a tool's description gives
    /// (`waits_for_yes`): a tool renamed or gone fails here.
    #[test]
    fn the_instructions_name_every_tool_and_no_gone_one() {
        let text = gp_mcp::server::INSTRUCTIONS;
        let have = words(text);
        let tools = tool_names();
        let missing: Vec<&&str> = tools.iter().filter(|t| !have.contains(**t)).collect();
        assert!(
            missing.is_empty(),
            "tools the server's instructions (crates/gp-mcp/src/server.rs INSTRUCTIONS) don't name: say when to use each: {missing:?}"
        );
        let args = arg_names();
        let described: BTreeSet<&str> = gp_mcp::tools::list()
            .iter()
            .flat_map(|t| words(t.description))
            .collect();
        let gone: Vec<&str> = snake_words(text)
            .into_iter()
            .filter(|w| !tools.contains(w) && !args.contains(*w) && !described.contains(w))
            .collect();
        assert!(
            gone.is_empty(),
            "words in the server's instructions that are no tool, no tool's argument and no field a tool gives: {gone:?}"
        );
    }

    /// A tool's description names other tools by name (`assets_edit
    /// import`, `watch_events`): a snake_case word that begins as a tool's
    /// name does is a tool or an argument.
    #[test]
    fn the_tools_descriptions_name_only_tools_there_are() {
        let tools = tool_names();
        let args = arg_names();
        let heads: BTreeSet<&str> = tools
            .iter()
            .filter_map(|t| t.split_once('_').map(|(h, _)| h))
            .collect();
        let mut gone = Vec::new();
        for t in gp_mcp::tools::list() {
            for w in snake_words(t.description) {
                let head = w.split('_').next().unwrap_or(w);
                if heads.contains(head) && !tools.contains(w) && !args.contains(w) {
                    gone.push(format!("{}: {w}", t.name));
                }
            }
        }
        assert!(
            gone.is_empty(),
            "tool descriptions (crates/gp-mcp/src/tools.rs) naming no tool: {gone:?}"
        );
    }

    #[test]
    fn a_text_names_the_commands_gp_has() {
        assert!(unknown_commands("run gp mechanic wait GAME, then gp loop health").is_empty());
        assert!(unknown_commands("the gp this server runs, with gp serve's runner").is_empty());
        assert!(unknown_commands("gp_run runs reads; gp_run mechanic queue with game").is_empty());
        assert!(unknown_commands("gp tool gate --patch FILE -m MESSAGE.").is_empty());
        assert_eq!(
            unknown_commands("gp mechanic qeue GAME"),
            ["gp mechanic qeue"]
        );
        assert_eq!(unknown_commands("then gp frobnicate it"), ["gp frobnicate"]);
        assert_eq!(
            unknown_commands("gp_run mechanic qeue with"),
            ["gp mechanic qeue"]
        );
        assert_eq!(
            unknown_commands("gp tool gate --pach FILE"),
            ["gp tool gate --pach"]
        );
    }

    /// Every `gp ...` the instructions and the tools' descriptions name is a
    /// command gp has, with flags it takes: a command renamed or removed
    /// fails here until the words follow it.
    #[test]
    fn the_server_names_only_commands_gp_has() {
        let mut bad: Vec<String> = unknown_commands(gp_mcp::server::INSTRUCTIONS)
            .into_iter()
            .map(|c| format!("instructions: {c}"))
            .collect();
        for t in gp_mcp::tools::list() {
            bad.extend(
                unknown_commands(t.description)
                    .into_iter()
                    .map(|c| format!("{}: {c}", t.name)),
            );
        }
        assert!(
            bad.is_empty(),
            "gp commands the server names that gp hasn't (crates/gp-mcp/src/server.rs, tools.rs): {bad:?}"
        );
    }

    /// The commands a quest-log row's step runs are gp's: one gone would
    /// drop from the step's commands without a word.
    #[test]
    fn a_rows_step_runs_commands_gp_has() {
        let cli = format!("{}{}", crate::steps::ROW_CLI, crate::steps::TOOL_ROW_CLI);
        assert_eq!(unknown_commands(&cli), Vec::<String>::new(), "{cli}");
        let named = cli.matches("gp ").count();
        assert_eq!(crate::steps::commands_of(&cli).len(), named, "{cli}");
    }

    /// Every `mcp__gp__NAME` anywhere in the repo (a worker's allowed tools,
    /// a skill, a plugin's hooks and commands, the docs) is a tool the
    /// server has.
    #[test]
    fn everything_names_only_tools_the_server_has() {
        let tools = tool_names();
        let text = |p: &str| {
            [
                ".rs", ".md", ".json", ".toml", ".sh", ".js", ".mjs", ".ts", ".py", ".yml", ".yaml",
            ]
            .iter()
            .any(|e| p.ends_with(e))
        };
        let mut gone = Vec::new();
        let mut seen = 0;
        for p in tracked().iter().filter(|p| text(p)) {
            let Ok(body) = std::fs::read_to_string(repo().join(p)) else {
                continue;
            };
            for n in mcp_names(&body) {
                seen += 1;
                if !tools.contains(n) {
                    gone.push(format!("{p}: mcp__gp__{n}"));
                }
            }
        }
        // The runner's worker names three at least.
        assert!(seen >= 3, "the scan found the names it checks");
        assert!(
            gone.is_empty(),
            "names of gp's MCP tools that the server hasn't: {gone:?}"
        );
    }

    /// docs/mcp.md's table of the tools has a row for each, and no row
    /// for a tool gone.
    #[test]
    fn the_docs_table_has_every_tool() {
        let doc = std::fs::read_to_string(repo().join("docs/mcp.md")).unwrap();
        let table: Vec<&str> = doc
            .lines()
            .skip_while(|l| !l.starts_with("| Roadmap name | MCP name |"))
            .take_while(|l| l.starts_with('|'))
            .collect();
        assert!(!table.is_empty(), "docs/mcp.md has the table of the tools");
        let mut rows = BTreeSet::new();
        for line in table {
            let cell = line.split('|').nth(2).unwrap_or("");
            rows.extend(cell.split('`').skip(1).step_by(2).map(str::to_owned));
        }
        let tools: BTreeSet<String> = tool_names().into_iter().map(str::to_owned).collect();
        let missing: Vec<&String> = tools.difference(&rows).collect();
        let gone: Vec<&String> = rows.difference(&tools).collect();
        assert!(
            missing.is_empty() && gone.is_empty(),
            "docs/mcp.md's table of the tools: no row for {missing:?}; rows for no tool {gone:?}"
        );
    }

    /// A plugin's manifest (`.claude-plugin/plugin.json`) carries gp's
    /// version, the one `gp mcp` says, so a plugin and the gp it runs
    /// move together.
    #[test]
    fn a_plugin_carries_gps_version() {
        for p in tracked()
            .iter()
            .filter(|p| p.ends_with(".claude-plugin/plugin.json"))
        {
            let m: Value = serde_json::from_str(&std::fs::read_to_string(repo().join(p)).unwrap())
                .unwrap_or_else(|e| panic!("{p}: {e}"));
            assert_eq!(
                m["version"],
                env!("CARGO_PKG_VERSION"),
                "{p}'s version is gp's (crates/gp-cli/Cargo.toml)"
            );
        }
    }
}
