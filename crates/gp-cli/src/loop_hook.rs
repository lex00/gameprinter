//! `gp loop hook EVENT`: the hooks of the gp plugin (plugin/, docs/plugin.md)
//! for Claude Code. Each reads the hook's JSON on stdin and prints the
//! hook's answer on stdout, or nothing to let the agent go on.
//!
//! | Event | What it does |
//! |---|---|
//! | `session-start` | Puts the loop's state in the session: gp's and the plugin's versions, whether the runner works the rows, the open yeses, and `gp progress --next` |
//! | `pre-tool-use` | Refuses Edit, Write, MultiEdit and NotebookEdit on a file in a game's folder of a games checkout: a game changes through gp's tools and its gate |
//! | `stop` | Refuses the first stop while a yes the person gave hasn't been used, or a card raised in the last half hour waits for them |
//!
//! A worker the runner started (`GP_STEP`) gets none of these: its tools
//! are scoped by the runner and its step ends with its report.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result};
use clap::ValueEnum;
use serde_json::{Value, json};

use crate::Outcome;

/// How long a card the agent raised holds its stop: it waits with `next`
/// that long at most, then may stop and say the card is up.
const CARD_HOLDS_STOP_MS: u64 = 30 * 60 * 1000;

/// The hook events the plugin's hooks/hooks.json runs this for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum HookEvent {
    SessionStart,
    PreToolUse,
    Stop,
}

impl HookEvent {
    /// Claude Code's name for the event.
    pub fn claude_name(self) -> &'static str {
        match self {
            HookEvent::SessionStart => "SessionStart",
            HookEvent::PreToolUse => "PreToolUse",
            HookEvent::Stop => "Stop",
        }
    }
}

/// What a hook decides from, gathered once so the decisions test without a
/// home of the person's.
pub struct Facts {
    /// The step of a worker the runner started (`GP_STEP`).
    pub step: Option<String>,
    /// `$GP_GAMES_DIR`.
    pub games: Option<PathBuf>,
    /// The asks on the person's page that are no runner worker's, newest
    /// first (gp_mcp::asks).
    pub asks: Vec<Value>,
    pub now_ms: u64,
}

impl Facts {
    fn gather(home: &Path) -> Facts {
        let workers: std::collections::BTreeSet<String> = gp_mcp::steps::workers(home)
            .iter()
            .filter_map(|w| w["step"]["id"].as_str().map(str::to_owned))
            .collect();
        let asks = gp_mcp::asks::all(home)
            .into_iter()
            .filter(|a| !a["step"].as_str().is_some_and(|s| workers.contains(s)))
            .collect();
        Facts {
            step: std::env::var(gp_mcp::steps::STEP_ENV)
                .ok()
                .filter(|s| !s.trim().is_empty()),
            games: std::env::var_os("GP_GAMES_DIR")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from),
            asks,
            now_ms: gp_mcp::asks::now_ms(),
        }
    }
}

/// The game folder `file` is in, when it is a game's folder (it holds
/// roms.toml) of a games checkout (its parent is `$GP_GAMES_DIR` or a git
/// work tree's root).
pub fn game_folder_of(file: &Path, games: Option<&Path>) -> Option<PathBuf> {
    let d = file
        .ancestors()
        .skip(1)
        .find(|d| d.join("roms.toml").is_file())?;
    let parent = d.parent()?;
    let checkout = parent.join(".git").exists() || games.is_some_and(|g| same_dir(g, parent));
    checkout.then(|| d.to_path_buf())
}

fn same_dir(a: &Path, b: &Path) -> bool {
    let c = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    c(a) == c(b)
}

/// PreToolUse: a hand edit in a game's folder is refused.
pub fn pre_tool_use(input: &Value, f: &Facts) -> Option<Value> {
    if f.step.is_some() {
        return None;
    }
    let tool = input["tool_name"].as_str()?;
    if !matches!(tool, "Edit" | "Write" | "MultiEdit" | "NotebookEdit") {
        return None;
    }
    let ti = &input["tool_input"];
    let p = ti["file_path"].as_str().or(ti["notebook_path"].as_str())?;
    let mut path = PathBuf::from(p);
    if path.is_relative()
        && let Some(cwd) = input["cwd"].as_str()
    {
        path = Path::new(cwd).join(path);
    }
    let game = game_folder_of(&path, f.games.as_deref())?;
    let name = game
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    Some(json!({
        "hookSpecificOutput": {
            "hookEventName": HookEvent::PreToolUse.claude_name(),
            "permissionDecision": "deny",
            "permissionDecisionReason": format!(
                "{} is in {name}'s folder of the games checkout, which changes only through gp: \
                 model_edit, contract_edit and assets_place for a copy, or a patch made in a \
                 worktree of the games repo and landed with gp model gate {name} (gp_run), \
                 which puts a card on the person's page for their yes.",
                path.display()
            ),
        }
    }))
}

/// Stop: the first stop is refused while a yes waits to be used or a card
/// the agent just raised waits for the person.
pub fn stop(input: &Value, f: &Facts) -> Option<Value> {
    if f.step.is_some() || input["stop_hook_active"].as_bool() == Some(true) {
        return None;
    }
    let what = |a: &Value| {
        let c = a["command"].as_str().unwrap_or("?");
        let says = a["says"].as_str().unwrap_or("");
        let id = a["id"].as_str().unwrap_or("?");
        if says.is_empty() {
            format!("gp {c} (ask {id})")
        } else {
            format!("gp {c} (ask {id}: {says})")
        }
    };
    if let Some(a) = f.asks.iter().find(|a| gp_mcp::asks::is_yes(a, f.now_ms)) {
        return Some(json!({
            "decision": "block",
            "reason": format!(
                "The person said yes on their page to {} and it hasn't run. Call gp_run again with the same command, arguments and ask_id, then stop.",
                what(a)
            ),
        }));
    }
    let fresh = f.asks.iter().find(|a| {
        gp_mcp::asks::is_open(a, f.now_ms)
            && f.now_ms.saturating_sub(a["made_ms"].as_u64().unwrap_or(0)) < CARD_HOLDS_STOP_MS
    });
    fresh.map(|a| {
        json!({
            "decision": "block",
            "reason": format!(
                "A card waits for the person's yes on their page: {}. Wait for it with the next tool (wait_s 300), and run the call again with its ask_id on a yes; or tell the person in one line that the card is up and stop.",
                what(a)
            ),
        })
    })
}

/// The plugin's version, from the plugin.json beside its hooks
/// (`$CLAUDE_PLUGIN_ROOT/.claude-plugin/plugin.json`).
fn plugin_version(root: &Path) -> Option<String> {
    let t = std::fs::read_to_string(root.join(".claude-plugin").join("plugin.json")).ok()?;
    let v: Value = serde_json::from_str(&t).ok()?;
    v["version"].as_str().map(str::to_owned)
}

/// The line on gp's and the plugin's versions: they come out together, so
/// a difference says which to update.
pub fn versions_line(plugin: Option<&str>) -> String {
    let gp = env!("CARGO_PKG_VERSION");
    match plugin {
        Some(p) if p != gp => format!(
            "Game Printer: gp is {gp} and the gp plugin is {p}; they are released together. Update the one that is older (brew upgrade gp, or /plugin update gp) so everyone porting a ROM has the same tools."
        ),
        Some(_) => format!("Game Printer: gp {gp} with the gp plugin of the same release."),
        None => format!("Game Printer: gp {gp}."),
    }
}

/// SessionStart: the loop's state, as context for the session.
fn session_start(home: &Path, f: &Facts) -> Value {
    let mut lines = vec![versions_line(
        std::env::var_os("CLAUDE_PLUGIN_ROOT")
            .and_then(|r| plugin_version(Path::new(&r)))
            .as_deref(),
    )];
    if let Some(s) = &f.step {
        lines.push(format!(
            "You are a worker of the loop on step {s}: do that step and nothing else, then stop with a report."
        ));
    } else {
        let st = gp_mcp::steps::settings(home);
        if st.runner.on && gp_mcp::steps::why_no_runner(home).is_none() {
            lines.push("The loop's runner is on: its workers do the quest log's rows. Your job is the setup: run gp loop health (gp_run) now and after anything changes, fix each PROBLEM it lists, and put what the person reports into the quest log as a row.".into());
        } else {
            lines.push("No runner works the rows: call the next tool now and after each step, tell the person where things stand, and stop and ask before anything in waits_for_yes.".into());
        }
        let yes: Vec<&Value> = f
            .asks
            .iter()
            .filter(|a| gp_mcp::asks::is_yes(a, f.now_ms))
            .collect();
        for a in yes {
            lines.push(format!(
                "A yes waits to be used: gp {} (ask {}). Run it with gp_run and its ask_id.",
                a["command"].as_str().unwrap_or("?"),
                a["id"].as_str().unwrap_or("?")
            ));
        }
        if let Some(next) = progress_next(home) {
            lines.push(format!("gp progress --next:\n{next}"));
        }
    }
    json!({
        "hookSpecificOutput": {
            "hookEventName": HookEvent::SessionStart.claude_name(),
            "additionalContext": lines.join("\n"),
        }
    })
}

/// `gp progress --next`'s words, its first 40 lines.
fn progress_next(home: &Path) -> Option<String> {
    let me = std::env::current_exe().ok()?;
    let out = Command::new(me)
        .args(["progress", "--next"])
        .env("GP_HOME", home)
        .env("GP_NO_HINTS", "1")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let t = String::from_utf8_lossy(&out.stdout);
    let t: Vec<&str> = t.lines().take(40).collect();
    (out.status.success() && !t.is_empty()).then(|| t.join("\n"))
}

pub(crate) fn run(event: HookEvent, paths: &gp_evidence::Paths) -> Result<Outcome> {
    let mut raw = String::new();
    if event != HookEvent::SessionStart || !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut raw)
            .context("reading the hook's input")?;
    }
    let input: Value = if raw.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(&raw).context("the hook's input isn't JSON")?
    };
    let f = Facts::gather(&paths.home);
    let answer = match event {
        HookEvent::SessionStart => Some(session_start(&paths.home, &f)),
        HookEvent::PreToolUse => pre_tool_use(&input, &f),
        HookEvent::Stop => stop(&input, &f),
    };
    // The hook's answer is the whole of stdout, as Claude Code reads it.
    let text = answer.as_ref().map_or_else(String::new, Value::to_string);
    Ok(Outcome::ok(answer.unwrap_or(Value::Null), text))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(asks: Vec<Value>) -> Facts {
        Facts {
            step: None,
            games: None,
            asks,
            now_ms: 10 * CARD_HOLDS_STOP_MS,
        }
    }

    fn edit(path: &Path) -> Value {
        json!({"tool_name": "Edit", "tool_input": {"file_path": path.display().to_string()}})
    }

    #[test]
    fn an_edit_in_a_games_checkouts_game_folder_is_refused() {
        let t = tempfile::tempdir().unwrap();
        let games = t.path().join("games");
        let game = games.join("rooftop");
        std::fs::create_dir_all(games.join(".git")).unwrap();
        std::fs::create_dir_all(game.join("model")).unwrap();
        std::fs::write(game.join("roms.toml"), "").unwrap();
        let f = facts(vec![]);
        let v = pre_tool_use(&edit(&game.join("model/model.toml")), &f).unwrap();
        assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "deny");
        assert!(
            v["hookSpecificOutput"]["permissionDecisionReason"]
                .as_str()
                .unwrap()
                .contains("gp model gate rooftop")
        );
        // A relative path is read from the session's folder.
        let rel = json!({"tool_name": "Write", "cwd": games.display().to_string(),
            "tool_input": {"file_path": "rooftop/notes.md"}});
        assert!(pre_tool_use(&rel, &f).is_some());
        // Outside the game's folder, another tool, or a worker: let through.
        assert!(pre_tool_use(&edit(&games.join("README.md")), &f).is_none());
        let read = json!({"tool_name": "Read", "tool_input": {"file_path": game.join("roms.toml").display().to_string()}});
        assert!(pre_tool_use(&read, &f).is_none());
        let w = Facts {
            step: Some("s0123456789ab".into()),
            ..facts(vec![])
        };
        assert!(pre_tool_use(&edit(&game.join("model/model.toml")), &w).is_none());
        // A copy of a game outside any checkout is the person's to edit.
        let copy = t.path().join("copies").join("rooftop");
        std::fs::create_dir_all(&copy).unwrap();
        std::fs::write(copy.join("roms.toml"), "").unwrap();
        assert!(pre_tool_use(&edit(&copy.join("model.toml")), &f).is_none());
        // Unless it is $GP_GAMES_DIR's.
        let g = Facts {
            games: Some(t.path().join("copies")),
            ..facts(vec![])
        };
        assert!(pre_tool_use(&edit(&copy.join("model.toml")), &g).is_some());
    }

    #[test]
    fn stop_waits_for_a_fresh_card_and_an_unused_yes_once() {
        let now = 10 * CARD_HOLDS_STOP_MS;
        let open = |made: u64| json!({"id": "a1", "command": "model gate", "state": "open", "made_ms": made});
        let stop_in = json!({"stop_hook_active": false});
        // A card made a minute ago holds the stop.
        let v = stop(&stop_in, &facts(vec![open(now - 60_000)])).unwrap();
        assert_eq!(v["decision"], "block");
        assert!(v["reason"].as_str().unwrap().contains("next tool"));
        // An hour-old card doesn't: the agent said it is up and may stop.
        assert!(stop(&stop_in, &facts(vec![open(now - 2 * CARD_HOLDS_STOP_MS)])).is_none());
        // A yes not used holds it, however old the card.
        let yes = json!({"id": "a2", "command": "tool gate", "state": "yes", "made_ms": 1, "answered_ms": now - 1000});
        let v = stop(&stop_in, &facts(vec![yes.clone()])).unwrap();
        assert!(v["reason"].as_str().unwrap().contains("ask_id"));
        // The second stop in a row goes, and a worker's always does.
        assert!(
            stop(
                &json!({"stop_hook_active": true}),
                &facts(vec![yes.clone()])
            )
            .is_none()
        );
        let w = Facts {
            step: Some("s0123456789ab".into()),
            ..facts(vec![yes])
        };
        assert!(stop(&stop_in, &w).is_none());
        assert!(stop(&stop_in, &facts(vec![])).is_none());
    }

    #[test]
    fn the_versions_line_says_which_to_update() {
        let gp = env!("CARGO_PKG_VERSION");
        assert!(versions_line(Some(gp)).contains("same release"));
        assert!(versions_line(Some("0.0.0")).contains("Update the one that is older"));
    }

    /// The plugin in the repo: released with gp (the same version), its
    /// hooks the events this command answers, its skills the repo's.
    #[test]
    fn the_plugin_is_released_with_gp_and_runs_these_hooks() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let plugin = repo.join("plugin");
        assert_eq!(
            plugin_version(&plugin).as_deref(),
            Some(env!("CARGO_PKG_VERSION")),
            "plugin/.claude-plugin/plugin.json's version is gp-cli's: bump them together"
        );
        let market: Value = serde_json::from_str(
            &std::fs::read_to_string(repo.join(".claude-plugin/marketplace.json")).unwrap(),
        )
        .unwrap();
        let entry = &market["plugins"][0];
        assert_eq!(entry["name"], "gp");
        assert_eq!(entry["source"], "./plugin");
        assert_eq!(entry["version"], env!("CARGO_PKG_VERSION"));
        let hooks: Value = serde_json::from_str(
            &std::fs::read_to_string(plugin.join("hooks/hooks.json")).unwrap(),
        )
        .unwrap();
        for e in HookEvent::value_variants() {
            let name = e.to_possible_value().unwrap().get_name().to_owned();
            let cmd = hooks["hooks"][e.claude_name()][0]["hooks"][0]["command"]
                .as_str()
                .unwrap_or_else(|| panic!("hooks.json runs nothing on {}", e.claude_name()));
            assert!(
                cmd.ends_with(&format!("loop hook {name}")),
                "{}: {cmd}",
                e.claude_name()
            );
        }
        let skills = repo.join(".claude/skills");
        for s in std::fs::read_dir(&skills).unwrap().flatten() {
            assert!(
                plugin
                    .join("skills")
                    .join(s.file_name())
                    .join("SKILL.md")
                    .is_file(),
                "the plugin has every skill of .claude/skills: {:?}",
                s.file_name()
            );
        }
        for f in ["agents/porter.md", "commands/port.md", "bin/gp"] {
            assert!(plugin.join(f).is_file(), "plugin/{f}");
        }
        // Its MCP server is gp's, named gp, started through bin/gp.
        let manifest: Value = serde_json::from_str(
            &std::fs::read_to_string(plugin.join(".claude-plugin/plugin.json")).unwrap(),
        )
        .unwrap();
        let mcp = &manifest["mcpServers"]["gp"];
        assert_eq!(mcp["args"][0], "${CLAUDE_PLUGIN_ROOT}/bin/gp");
        assert_eq!(mcp["args"][1], "mcp");
        assert!(repo.join("AGENTS.md").is_file());
    }
}
