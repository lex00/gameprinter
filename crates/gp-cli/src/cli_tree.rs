//! `gp self commands`: every command gp has, read from its own clap tree
//! with the walk `gp self docs` uses, as JSON. `gp mcp`'s gp_commands and
//! gp_run read it from the binary serving them, so the list an agent sees
//! is always the commands this gp has, never a hand-kept copy.

use std::any::TypeId;
use std::path::PathBuf;

use anyhow::Result;
use clap::{ArgAction, CommandFactory};
use serde_json::{Value, json};

use crate::{Cli, Outcome};

/// A value's kind, from its parser's type.
fn value_type(arg: &clap::Arg) -> &'static str {
    let t = arg.get_value_parser().type_id();
    let ints = [
        TypeId::of::<u8>(),
        TypeId::of::<u16>(),
        TypeId::of::<u32>(),
        TypeId::of::<u64>(),
        TypeId::of::<usize>(),
        TypeId::of::<i8>(),
        TypeId::of::<i16>(),
        TypeId::of::<i32>(),
        TypeId::of::<i64>(),
        TypeId::of::<isize>(),
    ];
    if ints.iter().any(|i| t == *i) {
        "integer"
    } else if t == TypeId::of::<f32>() || t == TypeId::of::<f64>() {
        "number"
    } else if t == TypeId::of::<PathBuf>() {
        "path"
    } else if t == TypeId::of::<bool>() {
        "boolean"
    } else {
        "string"
    }
}

/// One argument or flag.
fn arg_json(arg: &clap::Arg) -> Value {
    let kind = match arg.get_action() {
        ArgAction::SetTrue | ArgAction::SetFalse => "switch",
        ArgAction::Count => "count",
        _ => "value",
    };
    let max = arg.get_num_args().map_or(1, |r| r.max_values());
    let append = matches!(arg.get_action(), ArgAction::Append);
    let name = match arg.get_long() {
        Some(l) => l.to_owned(),
        None => arg.get_id().as_str().replace('_', "-"),
    };
    let help = arg
        .get_long_help()
        .or(arg.get_help())
        .map(|h| h.to_string())
        .unwrap_or_default();
    let choices: Vec<String> = arg
        .get_possible_values()
        .iter()
        .filter(|p| !p.is_hide_set())
        .map(|p| p.get_name().to_owned())
        .collect();
    // A switch's default is always false, so only a value's is said.
    let default: Vec<String> = arg
        .get_default_values()
        .iter()
        .filter(|_| kind == "value")
        .map(|d| d.to_string_lossy().into_owned())
        .collect();
    json!({
        "name": name,
        "long": arg.get_long(),
        "short": arg.get_short().map(String::from),
        "position": arg.get_index(),
        "kind": kind,
        "type": if kind == "value" { value_type(arg) } else { "boolean" },
        // Given more than once (a repeated flag, or a list positional).
        "repeat": kind == "value" && (append || max > 1),
        // Several values after one flag (`--flag A B`) rather than the flag
        // again for each.
        "values_per_flag": if kind == "value" && !append && arg.get_long().is_some() && max > 1 && max < usize::MAX { json!(max) } else { Value::Null },
        "required": arg.is_required_set(),
        "value_name": arg.get_value_names().and_then(|v| v.first()).map(|v| v.to_string()),
        "choices": choices,
        "default": default,
        "help": help.trim(),
    })
}

/// A command's arguments a person gives: not help, version or hidden ones.
fn own_args(cmd: &clap::Command) -> Vec<&clap::Arg> {
    cmd.get_arguments()
        .filter(|a| !a.is_hide_set() && !a.is_global_set())
        .filter(|a| {
            !matches!(
                a.get_action(),
                ArgAction::Help | ArgAction::HelpShort | ArgAction::HelpLong | ArgAction::Version
            )
        })
        .collect()
}

/// Whether a command runs by itself: a leaf, or one whose subcommand is
/// optional and that takes arguments of its own (`gp bench FILES`).
fn runnable(cmd: &clap::Command, leaf: bool) -> bool {
    leaf || (!cmd.is_subcommand_required_set() && !own_args(cmd).is_empty())
}

/// Every runnable command's words, in the tree's order.
#[cfg(test)]
fn runnable_paths(cli: &clap::Command) -> Vec<String> {
    let mut out = Vec::new();
    crate::cli_doc::walk(cli, "", &mut |w, c, leaf| {
        if !w.is_empty() && runnable(c, leaf) {
            out.push(w.to_owned());
        }
    });
    out
}

/// The tree as JSON: `commands` (each runnable command with its help and
/// arguments) and `groups` (each command that has subcommands).
pub(crate) fn tree(cli: &clap::Command) -> Value {
    // Built, so each positional has its index.
    let mut cli = cli.clone();
    cli.build();
    let cli = &cli;
    let mut commands = Vec::new();
    let mut groups = Vec::new();
    crate::cli_doc::walk(cli, "", &mut |w, c, leaf| {
        if w.is_empty() {
            return;
        }
        let about = c.get_about().map(|a| a.to_string()).unwrap_or_default();
        let help = c
            .get_long_about()
            .map(|a| a.to_string())
            .unwrap_or_else(|| about.clone());
        if !leaf {
            groups.push(json!({ "path": w, "about": about.trim() }));
        }
        if runnable(c, leaf) {
            commands.push(json!({
                "path": w,
                "about": about.trim(),
                "help": help.trim(),
                "args": own_args(c).into_iter().map(arg_json).collect::<Vec<_>>(),
                // gp mcp speaks JSON-RPC on stdout, so it takes no --json.
                "json": w != "mcp",
            }));
        }
    });
    json!({ "commands": commands, "groups": groups })
}

pub(crate) fn run() -> Result<Outcome> {
    let cli = Cli::command();
    let tree = tree(&cli);
    let text = tree["commands"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| {
            let about = c["about"].as_str().unwrap_or("");
            let first = about.lines().next().unwrap_or("");
            format!("gp {}: {first}", c["path"].as_str().unwrap_or(""))
        })
        .collect::<Vec<_>>()
        .join("\n");
    Ok(Outcome::ok(tree, text))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// gp_run's policy (crates/gp-mcp/src/policy.rs) holds a row for every
    /// command, and no row for a command that doesn't exist. A new command
    /// fails here until it is classified there.
    #[test]
    fn every_command_has_a_rule_in_gp_run_s_policy() {
        let paths = runnable_paths(&Cli::command());
        let missing: Vec<&String> = paths
            .iter()
            .filter(|p| gp_mcp::policy::row(p).is_none())
            .collect();
        assert!(
            missing.is_empty(),
            "commands with no rule in crates/gp-mcp/src/policy.rs TABLE (read what each writes and add a row): {missing:?}"
        );
        let stale: Vec<&str> = gp_mcp::policy::TABLE
            .iter()
            .map(|r| r.path)
            .filter(|p| !paths.iter().any(|q| q == p))
            .collect();
        assert!(
            stale.is_empty(),
            "rows in crates/gp-mcp/src/policy.rs TABLE that name no command: {stale:?}"
        );
    }

    #[test]
    fn the_tree_gives_arguments_with_their_kinds() {
        let tree = tree(&Cli::command());
        let commands = tree["commands"].as_array().unwrap();
        let find = |p: &str| {
            commands
                .iter()
                .find(|c| c["path"] == p)
                .unwrap_or_else(|| panic!("{p} is in the tree"))
        };
        let gate = find("model gate");
        let args = gate["args"].as_array().unwrap();
        let arg = |n: &str| {
            args.iter()
                .find(|a| a["name"] == n)
                .unwrap_or_else(|| panic!("model gate has {n}"))
        };
        assert_eq!(arg("dry-run")["kind"], "switch");
        assert_eq!(arg("game")["position"], 1);
        assert_eq!(arg("game")["required"], true);
        assert_eq!(arg("patch")["kind"], "value");
        assert_eq!(arg("dry-run")["default"], json!([]));
        // A command's own --version isn't taken for clap's.
        let publish = find("registry publish");
        assert!(
            publish["args"]
                .as_array()
                .unwrap()
                .iter()
                .any(|a| a["name"] == "version" && a["kind"] == "value")
        );
        assert!(
            !args
                .iter()
                .any(|a| a["name"] == "json" || a["name"] == "help"),
            "the global and help flags aren't a command's own"
        );
        assert_eq!(find("mcp")["json"], false);
        // A command that runs by itself and has subcommands is in both.
        assert!(commands.iter().any(|c| c["path"] == "bench"));
        assert!(
            tree["groups"]
                .as_array()
                .unwrap()
                .iter()
                .any(|g| g["path"] == "bench")
        );
    }
}
