//! The words every agent reads, as MCP prompts and resources (docs/mcp.md,
//! "Prompts and resources"): `/gp:port`'s text as the prompt `port`, the
//! porter's brief as `porter`, and each skill as a prompt of its name and a
//! resource `gp://skills/NAME`. They are the gp plugin's own files, built
//! into the server, so Codex, Cursor, Gemini CLI and VS Code get the same
//! text Claude Code gets from the plugin; gp-cli's
//! `mcp_keeps_up::tests::the_prompts_are_the_plugins_files` fails when a
//! file in plugin/ has no prompt or a prompt names no file.

use serde_json::{Value, json};

/// What a prompt's argument is.
pub struct Arg {
    pub name: &'static str,
    pub description: &'static str,
}

/// One prompt and resource: a file of the plugin's.
pub struct Source {
    /// The prompt's name.
    pub name: &'static str,
    pub title: &'static str,
    /// The file, from the repo's root, as the plugin has it.
    pub path: &'static str,
    /// The file's text.
    pub text: &'static str,
    /// The resource's URI.
    pub uri: &'static str,
    /// The prompt's arguments, all optional.
    pub args: &'static [Arg],
}

/// Every prompt, in the order a client lists them.
pub const SOURCES: &[Source] = &[
    Source {
        name: "port",
        title: "Port a game from a ROM you own",
        path: "plugin/commands/port.md",
        text: include_str!("../../../plugin/commands/port.md"),
        uri: "gp://commands/port",
        args: &[Arg {
            name: "target",
            description: "The game's name, or the path of the ROM file to port",
        }],
    },
    Source {
        name: "porter",
        title: "The porter's brief: one step of a port",
        path: "plugin/agents/porter.md",
        text: include_str!("../../../plugin/agents/porter.md"),
        uri: "gp://agents/porter",
        args: &[
            Arg {
                name: "game",
                description: "The game the step is for",
            },
            Arg {
                name: "row",
                description: "The quest-log row the step works on",
            },
            Arg {
                name: "step",
                description: "The step's id, from next (s and 12 hex digits)",
            },
        ],
    },
    Source {
        name: "element-extraction",
        title: "Skill: extract one element, code first",
        path: "plugin/skills/element-extraction/SKILL.md",
        text: include_str!("../../../.claude/skills/element-extraction/SKILL.md"),
        uri: "gp://skills/element-extraction",
        args: &[],
    },
    Source {
        name: "level-design",
        title: "Skill: design a stage from a game's level kit",
        path: "plugin/skills/level-design/SKILL.md",
        text: include_str!("../../../.claude/skills/level-design/SKILL.md"),
        uri: "gp://skills/level-design",
        args: &[],
    },
    Source {
        name: "theme-generation",
        title: "Skill: make a theme of your own for a game",
        path: "plugin/skills/theme-generation/SKILL.md",
        text: include_str!("../../../.claude/skills/theme-generation/SKILL.md"),
        uri: "gp://skills/theme-generation",
        args: &[],
    },
];

/// A file's front matter (between the first two `---` lines) and the body
/// after it.
fn split(text: &str) -> (&str, &str) {
    let Some(rest) = text.strip_prefix("---\n") else {
        return ("", text);
    };
    match rest.find("\n---\n") {
        Some(i) => (&rest[..i], rest[i + 5..].trim_start_matches('\n')),
        None => ("", text),
    }
}

/// The front matter's `description:`, unquoted.
pub fn description(s: &Source) -> String {
    let (front, _) = split(s.text);
    front
        .lines()
        .find_map(|l| l.strip_prefix("description:"))
        .map(|d| d.trim().trim_matches('"').to_owned())
        .unwrap_or_default()
}

/// The text without its front matter.
pub fn body(s: &Source) -> &'static str {
    split(s.text).1
}

pub fn find(name: &str) -> Option<&'static Source> {
    SOURCES.iter().find(|s| s.name == name)
}

/// `prompts/list`'s result.
pub fn list() -> Value {
    let prompts: Vec<Value> = SOURCES
        .iter()
        .map(|s| {
            let args: Vec<Value> = s
                .args
                .iter()
                .map(|a| json!({"name": a.name, "description": a.description, "required": false}))
                .collect();
            json!({
                "name": s.name,
                "title": s.title,
                "description": description(s),
                "arguments": args,
            })
        })
        .collect();
    json!({ "prompts": prompts })
}

/// The prompt's text with its arguments: `/gp:port`'s `$ARGUMENTS` is the
/// `target`; the porter's step is named in a last line.
pub fn text(s: &Source, args: &serde_json::Map<String, Value>) -> String {
    let arg = |n: &str| {
        args.get(n)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
    };
    let mut t = body(s).to_owned();
    if t.contains("$ARGUMENTS") {
        t = t.replace(
            "$ARGUMENTS",
            arg("target").unwrap_or("the game the person names"),
        );
    }
    if s.name == "porter" {
        let named: Vec<String> = ["game", "row", "step"]
            .iter()
            .filter_map(|n| arg(n).map(|v| format!("{n} {v}")))
            .collect();
        if !named.is_empty() {
            t.push_str(&format!("\nYou were handed: {}.\n", named.join(", ")));
        }
    }
    t
}

/// `prompts/get`'s result, or None for no such prompt.
pub fn get(name: &str, args: &serde_json::Map<String, Value>) -> Option<Value> {
    let s = find(name)?;
    Some(json!({
        "description": description(s),
        "messages": [{"role": "user", "content": {"type": "text", "text": text(s, args)}}],
    }))
}

/// `resources/list`'s result.
pub fn resources() -> Value {
    let r: Vec<Value> = SOURCES
        .iter()
        .map(|s| {
            json!({
                "uri": s.uri,
                "name": s.name,
                "title": s.title,
                "description": description(s),
                "mimeType": "text/markdown",
            })
        })
        .collect();
    json!({ "resources": r })
}

/// `resources/read`'s result, or None for no such resource.
pub fn read(uri: &str) -> Option<Value> {
    let s = SOURCES.iter().find(|s| s.uri == uri)?;
    Some(json!({
        "contents": [{"uri": s.uri, "mimeType": "text/markdown", "text": s.text}],
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_prompt_is_its_file_without_the_front_matter() {
        for s in SOURCES {
            assert!(!description(s).is_empty(), "{}: no description", s.path);
            assert!(!body(s).starts_with("---"), "{}", s.path);
            assert!(read(s.uri).is_some());
        }
        let none = serde_json::Map::new();
        let port = find("port").unwrap();
        assert!(!text(port, &none).contains("$ARGUMENTS"));
        let mut a = serde_json::Map::new();
        a.insert("target".into(), json!("~/roms/x.nes"));
        assert!(text(port, &a).contains("~/roms/x.nes"));
        let mut a = serde_json::Map::new();
        a.insert("game".into(), json!("rooftop"));
        a.insert("step".into(), json!("s0123456789ab"));
        let t = text(find("porter").unwrap(), &a);
        assert!(
            t.ends_with("You were handed: game rooftop, step s0123456789ab.\n"),
            "{t}"
        );
        assert!(get("nope", &none).is_none());
        assert_eq!(list()["prompts"].as_array().unwrap().len(), SOURCES.len());
    }
}
