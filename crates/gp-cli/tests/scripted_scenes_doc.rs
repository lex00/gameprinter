//! The scripted scenes design, docs/scripted-scenes.md, held to what the
//! quest-log row scripted-scenes-design asked of it: every part of the
//! design has its section, the model paths it adds are paths gp-model
//! parses, the build is laid out as rows that each have a measure, every
//! link goes to a file that exists, and no ported game is named (the names
//! scripts/check/no-game-names.sh lists).

use std::fs;
use std::path::{Path, PathBuf};

/// The sections the design must have, by their `## ` headings.
const PARTS: [&str; 7] = [
    "Finding a scene",
    "The steps",
    "The scene player",
    "Checking a scene",
    "The MCP tool",
    "What the model gains",
    "Build order",
];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The doc's `## ` sections, each heading with the text under it.
fn sections(doc: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut fenced = false;
    for line in doc.lines() {
        if line.starts_with("```") {
            fenced = !fenced;
        }
        if !fenced && let Some(h) = line.strip_prefix("## ") {
            out.push((h.trim().to_string(), String::new()));
        } else if let Some((_, text)) = out.last_mut() {
            text.push_str(line);
            text.push('\n');
        }
    }
    out
}

fn section<'a>(secs: &'a [(String, String)], name: &str) -> &'a str {
    secs.iter()
        .find(|(h, _)| h == name)
        .map(|(_, t)| t.as_str())
        .unwrap_or("")
}

/// Every `code span` of a text.
fn spans(text: &str) -> Vec<&str> {
    text.split('`').skip(1).step_by(2).collect()
}

/// The ported games' names, from the line `games=(...)` of the name check.
fn ported_games() -> Vec<String> {
    let script = fs::read_to_string(root().join("scripts/check/no-game-names.sh")).unwrap();
    let line = script
        .lines()
        .find_map(|l| l.trim().strip_prefix("games=("))
        .expect("the name check lists the games as games=(...)");
    line.trim_end_matches(')')
        .split_whitespace()
        .map(str::to_lowercase)
        .collect()
}

#[test]
fn the_scripted_scenes_design_has_its_parts() {
    let path = root().join("docs/scripted-scenes.md");
    let doc = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let secs = sections(&doc);
    let mut wrong = Vec::new();

    for part in PARTS {
        if section(&secs, part).trim().is_empty() {
            wrong.push(format!("no section `## {part}` with text under it"));
        }
    }

    // The model paths it adds, each a model path under `scenes`.
    let gains = section(&secs, "What the model gains");
    let paths: Vec<&str> = spans(gains)
        .into_iter()
        .filter(|s| s.starts_with("scenes."))
        .collect();
    if paths.len() < 5 {
        wrong.push(format!(
            "What the model gains names {} paths under `scenes.`, not at least 5",
            paths.len()
        ));
    }
    for p in &paths {
        if let Err(e) = gp_model::Path::parse(p) {
            wrong.push(format!("`{p}` isn't a model path: {e}"));
        }
    }

    // The MCP tool's request and its reply.
    let mcp = section(&secs, "The MCP tool").to_lowercase();
    for word in ["request", "reply"] {
        if !mcp.contains(word) {
            wrong.push(format!("The MCP tool doesn't give its {word}"));
        }
    }

    // The build order: a table of rows, each with a measure that runs.
    let build = section(&secs, "Build order");
    let rows: Vec<Vec<&str>> = build
        .lines()
        .filter(|l| l.starts_with('|'))
        .map(|l| {
            l.trim()
                .trim_matches('|')
                .split('|')
                .map(str::trim)
                .collect()
        })
        .collect();
    let measure = rows
        .first()
        .and_then(|h| h.iter().position(|c| *c == "Measure"));
    match measure {
        None => wrong.push("Build order has no table with a Measure column".into()),
        Some(m) => {
            let rows: Vec<&Vec<&str>> = rows
                .iter()
                .skip(1)
                .filter(|r| !r.iter().all(|c| c.chars().all(|ch| ch == '-')))
                .collect();
            if rows.len() < 4 {
                wrong.push(format!(
                    "Build order has {} rows, not at least 4",
                    rows.len()
                ));
            }
            for r in rows {
                let cell = r.get(m).copied().unwrap_or("");
                if !(cell.contains("cargo test") || cell.contains("gp ")) {
                    wrong.push(format!(
                        "the build row {} has no measure that runs (cargo test or gp)",
                        r[0]
                    ));
                }
            }
        }
    }

    // Links to files of the repo resolve.
    let docs = root().join("docs");
    for target in doc.split("](").skip(1).filter_map(|s| s.split(')').next()) {
        if target.starts_with("http") || target.starts_with('#') {
            continue;
        }
        let file = target.split('#').next().unwrap_or(target);
        if !docs.join(file).exists() {
            wrong.push(format!("the link {target} goes to no file"));
        }
    }

    // No ported game: a name's hyphen also matches a space, an underscore
    // or nothing, as the name check reads it.
    let lower = doc.to_lowercase();
    for game in ported_games() {
        for sep in ["-", " ", "_", ""] {
            let name = game.replace('-', sep);
            if lower.contains(&name) {
                wrong.push(format!("the doc names the ported game {name}"));
            }
        }
    }

    assert!(
        wrong.is_empty(),
        "docs/scripted-scenes.md:\n{}",
        wrong.join("\n")
    );
}
