//! The tool's own tests read no ROM and no games checkout (row
//! test-folders-read-no-rom). A test of the tool runs on a made-up fixture
//! (made-up bytes, a made-up romset, Moss Hop) and always runs; a test of
//! a game against its ROM belongs in the game's folder, run from there with
//! gp. So no file under a `crates/*/tests` folder names the romset finder
//! or one of the variables that point at this machine's ROMs or games,
//! except as the variable a child gp's `.env(..)` or `.env_remove(..)` pins
//! to a scratch folder, which is how a test keeps the child off them.
//! gate_mosshop.rs is left to another row.

use std::path::{Path, PathBuf};

/// The words, built so this file doesn't hold them whole.
fn words() -> [String; 4] {
    [
        format!("local_{}", "romset"),
        format!("GP_{}_DIR", "GAME"),
        format!("GP_{}_DIR", "GAMES"),
        format!("GP_{}_DIR", "ROM"),
    ]
}

const LEFT_TO_ANOTHER_ROW: &[&str] = &["gate_mosshop.rs"];

/// Each place in `text` that names a word, as `LINE: WORD`, but a GP_
/// variable passed straight to `.env("` or `.env_remove("`.
fn named(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for word in words() {
        for (at, _) in text.match_indices(word.as_str()) {
            let before = &text[..at];
            let pinned = word.starts_with("GP_")
                && (before.ends_with(".env(\"") || before.ends_with(".env_remove(\""));
            if !pinned {
                let line = before.matches('\n').count() + 1;
                out.push(format!("{line}: {word}"));
            }
        }
    }
    out
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

#[test]
fn no_test_folder_reads_a_rom_or_a_games_checkout() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut files = Vec::new();
    for c in std::fs::read_dir(&crates).unwrap().flatten() {
        rust_files(&c.path().join("tests"), &mut files);
    }
    files.sort();
    assert!(files.len() > 10, "only {} test files found", files.len());
    let mut found = Vec::new();
    for f in &files {
        let name = f.file_name().unwrap().to_string_lossy();
        if LEFT_TO_ANOTHER_ROW.contains(&name.as_ref()) {
            continue;
        }
        let text = std::fs::read_to_string(f).unwrap();
        let rel = f.strip_prefix(&crates).unwrap_or(f).display().to_string();
        found.extend(
            named(&text)
                .into_iter()
                .map(|n| format!("crates/{rel}:{n}")),
        );
    }
    assert!(
        found.is_empty(),
        "a test of the tool reads no ROM or games checkout: give it a made-up fixture, or move a test of a game to the game's folder:\n{}",
        found.join("\n")
    );
}

#[test]
fn a_pinned_variable_passes_and_a_read_or_a_mention_fails() {
    let [finder, game, games, roms] = words();
    let pinned =
        format!("cmd.env(\"{roms}\", home.join(\"roms\"))\n    .env_remove(\"{games}\");\n");
    assert!(named(&pinned).is_empty(), "{:?}", named(&pinned));
    let read = format!("let d = std::env::var_os(\"{roms}\");\n");
    assert_eq!(named(&read), [format!("1: {roms}")]);
    let doc = format!("//! The game is ${game}.\n//! Its set is from {finder}.\n");
    assert_eq!(named(&doc), [format!("2: {finder}"), format!("1: {game}")]);
}
