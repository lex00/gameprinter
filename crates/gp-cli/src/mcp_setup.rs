//! `gp mcp setup`: how to connect an agent to gp mcp, per harness (Claude
//! Code, Codex, Cursor, Gemini CLI, VS Code), and with `--write` the
//! connection made after showing what it writes. `gp self check`'s Agent
//! line reads the same [`harnesses`] (docs/mcp.md, "Registering it with an
//! agent").

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use clap::{Args, ValueEnum};
use serde_json::{Value, json};

use crate::Outcome;

#[derive(Clone, Copy, PartialEq, Eq, Debug, ValueEnum)]
pub enum Harness {
    Claude,
    Codex,
    Cursor,
    Gemini,
    Vscode,
}

impl Harness {
    const ALL: [Harness; 5] = [
        Harness::Claude,
        Harness::Codex,
        Harness::Cursor,
        Harness::Gemini,
        Harness::Vscode,
    ];

    fn key(self) -> &'static str {
        match self {
            Harness::Claude => "claude",
            Harness::Codex => "codex",
            Harness::Cursor => "cursor",
            Harness::Gemini => "gemini",
            Harness::Vscode => "vscode",
        }
    }

    /// Its program, when it is on PATH.
    fn program(self) -> &'static str {
        match self {
            Harness::Vscode => "code",
            h => h.key(),
        }
    }

    pub(crate) fn words(self) -> &'static str {
        match self {
            Harness::Claude => "Claude Code",
            Harness::Codex => "Codex",
            Harness::Cursor => "Cursor",
            Harness::Gemini => "Gemini CLI",
            Harness::Vscode => "VS Code",
        }
    }

    /// Its own folder in the user's home (VS Code's user settings folder).
    fn dir(self, user: &Path) -> PathBuf {
        match self {
            Harness::Claude => user.join(".claude"),
            Harness::Codex => user.join(".codex"),
            Harness::Cursor => user.join(".cursor"),
            Harness::Gemini => user.join(".gemini"),
            Harness::Vscode if cfg!(target_os = "macos") => {
                user.join("Library/Application Support/Code/User")
            }
            Harness::Vscode if cfg!(windows) => user.join("AppData/Roaming/Code/User"),
            Harness::Vscode => user.join(".config/Code/User"),
        }
    }

    /// The config file `--write` writes (Claude Code's is written by its
    /// own `claude mcp add`).
    fn config(self, user: &Path) -> PathBuf {
        match self {
            Harness::Claude => user.join(".claude.json"),
            Harness::Codex => user.join(".codex").join("config.toml"),
            Harness::Cursor => user.join(".cursor").join("mcp.json"),
            Harness::Gemini => user.join(".gemini").join("settings.json"),
            Harness::Vscode => self.dir(user).join("mcp.json"),
        }
    }

    /// The key of a JSON config's servers (VS Code's is `servers`).
    fn servers_key(self) -> &'static str {
        if self == Harness::Vscode {
            "servers"
        } else {
            "mcpServers"
        }
    }

    /// The server's entry in a JSON config.
    fn json_entry(self, gp: &str) -> Value {
        if self == Harness::Vscode {
            json!({"type": "stdio", "command": gp, "args": ["mcp"]})
        } else {
            json!({"command": gp, "args": ["mcp"]})
        }
    }
}

#[derive(Args)]
pub struct SetupArgs {
    /// The agent to connect (default: each one found on this machine, or
    /// every one when none is).
    #[arg(long, value_enum)]
    harness: Option<Harness>,
    /// Write the connection into the agent's config (for Claude Code, run
    /// `claude mcp add`) after printing what it writes. Needs --harness.
    #[arg(long, requires = "harness")]
    write: bool,
}

/// What this machine has of one harness.
pub(crate) struct Found {
    pub harness: Harness,
    /// Its program on PATH, or its folder in the home.
    pub here: bool,
    /// gp is in its config already (for Claude Code, the gp plugin or a
    /// server named gp).
    pub connected: bool,
    /// What was found, in words.
    pub found: String,
}

/// The user's home folder.
pub(crate) fn user_home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn read_json(p: &Path) -> Value {
    std::fs::read_to_string(p)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Value::Null)
}

/// Whether the gp plugin is installed in Claude Code (its
/// plugins/installed_plugins.json names gp@ a marketplace).
fn plugin_installed(user: &Path) -> bool {
    let v = read_json(&user.join(".claude/plugins/installed_plugins.json"));
    let Some(plugins) = v["plugins"].as_object() else {
        return false;
    };
    plugins.keys().any(|k| k.starts_with("gp@"))
}

/// Whether `h`'s config names a server gp.
fn in_config(h: Harness, user: &Path) -> bool {
    let p = h.config(user);
    if h != Harness::Codex {
        return read_json(&p)[h.servers_key()]["gp"].is_object();
    }
    let Ok(text) = std::fs::read_to_string(&p) else {
        return false;
    };
    let Ok(table) = text.parse::<toml::Table>() else {
        return false;
    };
    let gp = table.get("mcp_servers").and_then(|s| s.get("gp"));
    gp.is_some_and(toml::Value::is_table)
}

/// What one harness has here, in words.
fn found_words(h: Harness, user: &Path, program: bool, plugin: bool, config: bool) -> String {
    let mut parts = vec![if program {
        format!("{} on PATH", h.program())
    } else {
        format!("{} is here", h.dir(user).display())
    }];
    if h == Harness::Claude {
        let p = if plugin { "installed" } else { "not installed" };
        parts.push(format!("the gp plugin {p}"));
    }
    if config {
        parts.push(format!("gp in {}", h.config(user).display()));
    } else if h != Harness::Claude {
        parts.push("gp not in its MCP servers".into());
    }
    parts.join(", ")
}

/// The harnesses this machine has, with `on_path` finding a program.
pub(crate) fn harnesses(user: &Path, on_path: &dyn Fn(&str) -> bool) -> Vec<Found> {
    let mut out = Vec::new();
    for h in Harness::ALL {
        let program = on_path(h.program());
        let here = program || h.dir(user).is_dir();
        let plugin = h == Harness::Claude && plugin_installed(user);
        let config = in_config(h, user);
        let found = if here {
            found_words(h, user, program, plugin, config)
        } else {
            "not found".to_owned()
        };
        out.push(Found {
            harness: h,
            here,
            connected: plugin || config,
            found,
        });
    }
    out
}

/// The gp a config should start: gp on PATH, else the published build in
/// ~/.gameprinter/bin, else this one (the order of plugin/bin/gp).
pub(crate) fn gp_command() -> String {
    let published = user_home()
        .map(|u| u.join(".gameprinter/bin/gp"))
        .filter(|p| p.is_file());
    crate::self_check::on_path("gp")
        .or(published)
        .or_else(|| std::env::current_exe().ok())
        .map_or_else(|| "gp".into(), |p| p.display().to_string())
}

/// The one line that connects `h`, for `gp self check` and the setup.
pub(crate) fn line(h: Harness, gp: &str) -> String {
    match h {
        Harness::Claude => format!(
            "in Claude Code: /plugin marketplace add lex00/gameprinter, then /plugin install gp@gameprinter (or: claude mcp add --scope user gp -- {gp} mcp)"
        ),
        Harness::Codex => format!(
            "add to ~/.codex/config.toml: [mcp_servers.gp] command = \"{gp}\" args = [\"mcp\"] (gp mcp setup --harness codex --write does it)"
        ),
        Harness::Cursor | Harness::Gemini | Harness::Vscode => {
            let file = match h {
                Harness::Cursor => "~/.cursor/mcp.json",
                Harness::Gemini => "~/.gemini/settings.json",
                _ => "VS Code's user mcp.json",
            };
            let entry = json!({h.servers_key(): {"gp": h.json_entry(gp)}});
            format!(
                "add to {file}: {entry} (gp mcp setup --harness {} --write does it)",
                h.key()
            )
        }
    }
}

/// What `h`'s setup writes, as text: Claude Code's command, Codex's TOML
/// table, the others' JSON entry.
fn snippet(h: Harness, gp: &str) -> String {
    match h {
        Harness::Claude => format!("claude mcp add --scope user gp -- {gp} mcp"),
        Harness::Codex => format!("[mcp_servers.gp]\ncommand = {gp:?}\nargs = [\"mcp\"]\n"),
        Harness::Cursor | Harness::Gemini | Harness::Vscode => {
            let entry = json!({h.servers_key(): {"gp": h.json_entry(gp)}});
            serde_json::to_string_pretty(&entry).unwrap_or_default()
        }
    }
}

/// Runs Claude Code's own `claude mcp add` for gp.
fn write_claude(gp: &str) -> Result<String> {
    let Some(claude) = crate::self_check::on_path("claude") else {
        bail!("claude isn't on PATH: install Claude Code, then run this again");
    };
    let st = Command::new(claude)
        .args(["mcp", "add", "--scope", "user", "gp", "--", gp, "mcp"])
        .stdin(Stdio::null())
        .status()
        .context("running claude mcp add")?;
    if !st.success() {
        bail!("claude mcp add failed ({st})");
    }
    Ok("claude mcp add added gp to Claude Code's user servers".into())
}

/// Adds `[mcp_servers.gp]` at the end of Codex's config, `p`.
fn write_codex(p: &Path, gp: &str) -> Result<String> {
    let mut text = std::fs::read_to_string(p).unwrap_or_default();
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    if !text.is_empty() {
        text.push('\n');
    }
    text.push_str(&snippet(Harness::Codex, gp));
    if let Err(e) = text.parse::<toml::Table>() {
        bail!("{} would not read as TOML: {e}", p.display());
    }
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(p, text).with_context(|| p.display().to_string())?;
    Ok(format!("added [mcp_servers.gp] to {}", p.display()))
}

/// Adds the server gp to `h`'s JSON config, `p` (mcpServers.gp, or VS
/// Code's servers.gp), keeping what it has.
fn write_json(h: Harness, p: &Path, gp: &str) -> Result<String> {
    let key = h.servers_key();
    let mut v = json!({});
    if p.is_file() {
        let t = std::fs::read_to_string(p)?;
        let Ok(read) = serde_json::from_str::<Value>(&t) else {
            bail!("{} isn't JSON", p.display());
        };
        v = read;
    }
    let Some(obj) = v.as_object_mut() else {
        bail!("{} isn't a JSON object", p.display());
    };
    let servers = obj.entry(key).or_insert_with(|| json!({}));
    let Some(servers) = servers.as_object_mut() else {
        bail!("{}'s {key} isn't an object", p.display());
    };
    servers.insert("gp".into(), h.json_entry(gp));
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    let text = serde_json::to_string_pretty(&v)? + "\n";
    std::fs::write(p, text).with_context(|| p.display().to_string())?;
    Ok(format!("added {key}.gp to {}", p.display()))
}

/// Writes `h`'s connection under `user`, returning what changed.
fn write(h: Harness, user: &Path, gp: &str) -> Result<String> {
    let p = h.config(user);
    if in_config(h, user) {
        let had = p.display();
        return Ok(format!("{had} already names a server gp; nothing written"));
    }
    match h {
        Harness::Claude => write_claude(gp),
        Harness::Codex => write_codex(&p, gp),
        Harness::Cursor | Harness::Gemini | Harness::Vscode => write_json(h, &p, gp),
    }
}

/// The setup for `user`'s home, starting `gp`, with `on_path` finding a
/// program.
fn setup_in(
    args: &SetupArgs,
    user: &Path,
    gp: &str,
    on_path: &dyn Fn(&str) -> bool,
) -> Result<Outcome> {
    let found = harnesses(user, on_path);
    let any_here = found.iter().any(|f| f.here);
    let picked: Vec<&Found> = found
        .iter()
        .filter(|f| match args.harness {
            Some(h) => f.harness == h,
            None => f.here || !any_here,
        })
        .collect();
    let mut text = String::new();
    let mut list = Vec::new();
    for f in &picked {
        let h = f.harness;
        let _ = writeln!(text, "{}: {}", h.words(), f.found);
        let mut wrote = None;
        if f.connected {
            let _ = writeln!(text, "  gp is connected here already");
        } else {
            let _ = writeln!(text, "  to connect it: {}", line(h, gp));
        }
        if args.write && !f.connected {
            let into = match h {
                Harness::Claude => "Claude Code (it runs claude mcp add)".to_owned(),
                _ => h.config(user).display().to_string(),
            };
            let _ = writeln!(text, "  writes into {into}:");
            for l in snippet(h, gp).lines() {
                let _ = writeln!(text, "    {l}");
            }
            let w = write(h, user, gp)?;
            let again = h.words();
            let _ = writeln!(text, "  {w}; a new {again} session has the gp tools");
            wrote = Some(w);
        }
        list.push(json!({
            "harness": h.key(), "here": f.here, "connected": f.connected, "found": f.found,
            "line": line(h, gp), "writes": snippet(h, gp), "config": h.config(user),
            "wrote": wrote,
        }));
    }
    if !args.write && picked.iter().any(|f| !f.connected) {
        let _ = writeln!(text, "--harness NAME --write makes the connection.");
    }
    Ok(Outcome::ok(
        json!({ "gp": gp, "harnesses": list }),
        text.trim_end().to_owned(),
    ))
}

pub(crate) fn run(args: &SetupArgs) -> Result<Outcome> {
    let Some(user) = user_home() else {
        bail!("no HOME: gp mcp setup reads the agents' configs there");
    };
    let on_path = |p: &str| crate::self_check::on_path(p).is_some();
    setup_in(args, &user, &gp_command(), &on_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(h: Option<Harness>, write: bool) -> SetupArgs {
        SetupArgs { harness: h, write }
    }

    fn names(o: &Outcome) -> Vec<String> {
        let hs = o.json["harnesses"].as_array().unwrap();
        hs.iter().map(|h| h["harness"].to_string()).collect()
    }

    #[test]
    fn setup_says_the_line_for_each_harness_found_and_writes_it() {
        let t = tempfile::tempdir().unwrap();
        let user = t.path();
        let none = |_: &str| false;
        // Nothing here: every one, each with its line.
        let o = setup_in(&args(None, false), user, "/bin/gp", &none).unwrap();
        assert_eq!(names(&o).len(), 5);
        let plugin = "/plugin install gp@gameprinter";
        assert!(o.text.contains(plugin), "{}", o.text);
        // Codex and Cursor here: only those, and --write connects them.
        std::fs::create_dir_all(user.join(".codex")).unwrap();
        std::fs::write(user.join(".codex/config.toml"), "model = \"x\"").unwrap();
        std::fs::create_dir_all(user.join(".cursor")).unwrap();
        let other = r#"{"mcpServers": {"other": {}}}"#;
        std::fs::write(user.join(".cursor/mcp.json"), other).unwrap();
        let o = setup_in(&args(None, false), user, "/bin/gp", &none).unwrap();
        assert_eq!(names(&o), ["\"codex\"", "\"cursor\""]);
        for h in [Harness::Codex, Harness::Cursor] {
            let o = setup_in(&args(Some(h), true), user, "/bin/gp", &none).unwrap();
            assert!(o.text.contains("writes into"), "{}", o.text);
            assert!(in_config(h, user), "{h:?}");
        }
        let codex = std::fs::read_to_string(user.join(".codex/config.toml")).unwrap();
        let codex: toml::Table = codex.parse().unwrap();
        assert_eq!(codex["model"].as_str(), Some("x"));
        let command = codex["mcp_servers"]["gp"]["command"].as_str();
        assert_eq!(command, Some("/bin/gp"));
        let cursor = read_json(&user.join(".cursor/mcp.json"));
        assert!(cursor["mcpServers"]["other"].is_object());
        assert_eq!(cursor["mcpServers"]["gp"]["args"][0], "mcp");
        // Connected now: said so, and nothing written again.
        let codex = Some(Harness::Codex);
        let o = setup_in(&args(codex, true), user, "/bin/gp", &none).unwrap();
        assert!(o.text.contains("connected here already"), "{}", o.text);
        assert!(o.json["harnesses"][0]["wrote"].is_null());
    }

    /// Gemini CLI and VS Code are found by their folders and connected by
    /// their own JSON: mcpServers.gp in Gemini's settings.json (the rest
    /// kept), servers.gp of type stdio in VS Code's mcp.json.
    #[test]
    fn gemini_cli_and_vs_code_are_connected_in_their_own_json() {
        let t = tempfile::tempdir().unwrap();
        let user = t.path();
        let none = |_: &str| false;
        std::fs::create_dir_all(user.join(".gemini")).unwrap();
        let theme = r#"{"theme": "x"}"#;
        std::fs::write(user.join(".gemini/settings.json"), theme).unwrap();
        std::fs::create_dir_all(Harness::Vscode.dir(user)).unwrap();
        let o = setup_in(&args(None, false), user, "/bin/gp", &none).unwrap();
        assert_eq!(names(&o), ["\"gemini\"", "\"vscode\""]);
        assert!(o.text.contains("Gemini CLI"), "{}", o.text);
        assert!(o.text.contains("--harness vscode --write"), "{}", o.text);
        for h in [Harness::Gemini, Harness::Vscode] {
            setup_in(&args(Some(h), true), user, "/bin/gp", &none).unwrap();
            assert!(in_config(h, user), "{h:?}");
        }
        let gemini = read_json(&user.join(".gemini/settings.json"));
        assert_eq!(gemini["theme"], "x");
        assert_eq!(gemini["mcpServers"]["gp"]["command"], "/bin/gp");
        let code = read_json(&Harness::Vscode.config(user));
        assert_eq!(code["servers"]["gp"]["type"], "stdio");
        assert_eq!(code["servers"]["gp"]["args"][0], "mcp");
        // VS Code on PATH as `code` counts as here.
        let code_on_path = |p: &str| p == "code";
        let f = &harnesses(tempfile::tempdir().unwrap().path(), &code_on_path)[4];
        assert!(f.here && f.found.contains("code on PATH"), "{}", f.found);
    }

    #[test]
    fn the_gp_plugin_counts_as_connected() {
        let t = tempfile::tempdir().unwrap();
        let user = t.path();
        let claude = |p: &str| p == "claude";
        let f = &harnesses(user, &claude)[0];
        assert!(f.here && !f.connected, "{}", f.found);
        assert!(f.found.contains("the gp plugin not installed"));
        std::fs::create_dir_all(user.join(".claude/plugins")).unwrap();
        let installed = r#"{"version": 2, "plugins": {"gp@gameprinter": [{"scope": "user"}]}}"#;
        let file = user.join(".claude/plugins/installed_plugins.json");
        std::fs::write(file, installed).unwrap();
        let f = &harnesses(user, &claude)[0];
        assert!(f.connected, "{}", f.found);
        assert!(f.found.contains("the gp plugin installed"));
    }

    #[test]
    fn gp_mcp_setup_parses() {
        use clap::Parser as _;
        for a in [
            vec!["gp", "mcp", "setup"],
            vec!["gp", "mcp", "setup", "--harness", "codex", "--write"],
            vec!["gp", "mcp", "--repo", "x"],
        ] {
            if let Err(e) = crate::Cli::try_parse_from(&a) {
                panic!("{a:?}: {e}");
            }
        }
        let alone = ["gp", "mcp", "setup", "--write"];
        assert!(crate::Cli::try_parse_from(alone).is_err());
    }
}
