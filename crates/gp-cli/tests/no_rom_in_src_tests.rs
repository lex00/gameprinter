//! The test modules under crates/*/src read no ROM and no games checkout
//! of this machine (row src-tests-read-no-rom). A test there that wants a
//! ROM folder, a game's folder or a games checkout makes one of its own and
//! points the variable at it; one that reads the variable as the shell left
//! it, or asks the arcade crate's local-romset finder or test_* helpers for the
//! person's set, plays this machine's ROMs where they are and skips where
//! they aren't, so `just check` never runs it. Those are a game's checks,
//! run from the game's folder.
//!
//! The rule, held to every `#[cfg(test)]` item (a `mod tests { .. }`, a
//! `mod tests;` file, a test helper fn) of every crates/*/src file:
//!
//! - a variable's name (the ROM folder's, the game folder's, the games
//!   checkout's) appears only as the first argument of `set_var(`,
//!   `remove_var(`, a child's `.env(` or `.env_remove(`;
//! - the local-romset finder is called only after the same test item set the ROM
//!   folder's variable (or the set's own) to a folder of its own;
//! - the arcade crate's test_* helpers that find the person's game and set
//!   are not called at all.
//!
//! Comments are not read. The names are put together here, so this file
//! names none of them itself.

use std::path::{Path, PathBuf};

fn vars() -> Vec<String> {
    ["ROM_DIR", "GAME_DIR", "GAMES_DIR"]
        .iter()
        .map(|s| format!("GP_{s}"))
        .collect()
}

fn romset_fn() -> String {
    format!("local_{}", "romset")
}

/// The arcade crate's helpers that read the game folder's variable and the
/// person's ROM folder.
fn helpers() -> Vec<String> {
    [
        "game",
        "set",
        "romset",
        "romset_path",
        "oracle_set_path",
        "replays",
        "input_log",
    ]
    .iter()
    .map(|s| format!("test_{s}("))
    .chain(
        ["TEST_GAME_ENV", "GAMES_DIR_ENV"]
            .iter()
            .map(|s| s.to_string()),
    )
    .collect()
}

fn ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// `text` with every comment blanked to spaces (newlines kept), and a mask
/// of which bytes are inside a string or char literal.
fn lex(text: &str) -> (Vec<u8>, Vec<bool>) {
    let b = text.as_bytes();
    let mut out = b.to_vec();
    let mut in_str = vec![false; b.len()];
    let mut i = 0;
    while i < b.len() {
        if b[i..].starts_with(b"//") {
            while i < b.len() && b[i] != b'\n' {
                out[i] = b' ';
                i += 1;
            }
        } else if b[i..].starts_with(b"/*") {
            let mut depth = 0;
            while i < b.len() {
                if b[i..].starts_with(b"/*") {
                    depth += 1;
                    out[i] = b' ';
                    out[i + 1] = b' ';
                    i += 2;
                } else if b[i..].starts_with(b"*/") {
                    depth -= 1;
                    out[i] = b' ';
                    out[i + 1] = b' ';
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    if b[i] != b'\n' {
                        out[i] = b' ';
                    }
                    i += 1;
                }
            }
        } else if b[i] == b'r'
            && (i == 0 || !ident(b[i - 1]) || (b[i - 1] == b'b' && (i < 2 || !ident(b[i - 2]))))
            && b[i + 1..].iter().take_while(|&&c| c == b'#').count() + i + 1 < b.len()
            && b[i + 1 + b[i + 1..].iter().take_while(|&&c| c == b'#').count()] == b'"'
        {
            let hashes = b[i + 1..].iter().take_while(|&&c| c == b'#').count();
            let close: Vec<u8> = std::iter::once(b'"')
                .chain(std::iter::repeat_n(b'#', hashes))
                .collect();
            i += 2 + hashes;
            while i < b.len() && !b[i..].starts_with(&close) {
                in_str[i] = true;
                i += 1;
            }
            i += close.len();
        } else if b[i] == b'"' {
            i += 1;
            while i < b.len() && b[i] != b'"' {
                in_str[i] = true;
                if b[i] == b'\\' && i + 1 < b.len() {
                    in_str[i + 1] = true;
                    i += 1;
                }
                i += 1;
            }
            i += 1;
        } else if b[i] == b'\'' {
            // A char literal ('x', '\n', '{'), or a lifetime ('a).
            let end = if b.get(i + 1) == Some(&b'\\') {
                b[i + 2..]
                    .iter()
                    .position(|&c| c == b'\'')
                    .map(|p| i + 2 + p)
            } else if b.get(i + 2) == Some(&b'\'') {
                Some(i + 2)
            } else {
                // A multi-byte char: up to 4 bytes then a quote.
                (2..6).map(|k| i + k).find(|&k| {
                    b.get(k) == Some(&b'\'') && !b[i + 1..k].iter().all(|c| c.is_ascii())
                })
            };
            match end {
                Some(e) => {
                    for m in &mut in_str[i + 1..e] {
                        *m = true;
                    }
                    i = e + 1;
                }
                None => i += 1,
            }
        } else {
            i += 1;
        }
    }
    (out, in_str)
}

/// The byte ranges of `code` (comments blanked) that are `#[cfg(test)]`
/// items with a body, and the names of `#[cfg(test)] mod NAME;` files.
fn test_items(code: &[u8], in_str: &[bool]) -> (Vec<(usize, usize)>, Vec<String>) {
    let text = String::from_utf8_lossy(code);
    let mut ranges = Vec::new();
    let mut files = Vec::new();
    let mut from = 0;
    while let Some(p) = text[from..].find("#[cfg(") {
        let at = from + p;
        from = at + 6;
        if in_str[at] {
            continue;
        }
        let Some(close) = text[at..].find(']') else {
            break;
        };
        let attr = &text[at..at + close];
        let has_test = attr
            .split(|c: char| !(c.is_alphanumeric() || c == '_'))
            .any(|w| w == "test");
        if !has_test || attr.contains("not(test") {
            continue;
        }
        // The item after it: its body's braces, or `mod NAME;`.
        let mut i = at + close + 1;
        let mut depth = 0usize;
        let mut start = None;
        while i < code.len() {
            if !in_str[i] {
                match code[i] {
                    b'{' => {
                        if start.is_none() {
                            start = Some(i);
                        }
                        depth += 1;
                    }
                    // A field or an arm, not an item: the end of what
                    // holds it.
                    b'}' if start.is_none() => break,
                    b',' if start.is_none()
                        && !text[at + close + 1..i]
                            .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                            .any(|w| w == "fn") =>
                    {
                        break;
                    }
                    b'}' => {
                        depth = depth.saturating_sub(1);
                        if depth == 0 && start.is_some() {
                            break;
                        }
                    }
                    b';' if start.is_none() => {
                        let head = text[at + close + 1..i].trim();
                        let words: Vec<&str> = head.split_whitespace().collect();
                        if let Some(k) = words.iter().position(|w| *w == "mod")
                            && let Some(name) = words.get(k + 1)
                        {
                            files.push(name.to_string());
                        }
                        break;
                    }
                    _ => {}
                }
            }
            i += 1;
        }
        if let Some(s) = start {
            ranges.push((s, i.min(code.len())));
            from = i.max(from);
        }
    }
    (ranges, files)
}

/// What `code[range]` breaks of the rule, as `LINE: WHAT` with lines of
/// the whole file.
fn breaks(code: &[u8], in_str: &[bool], range: (usize, usize)) -> Vec<String> {
    let text = String::from_utf8_lossy(code);
    let line = |at: usize| text[..at].matches('\n').count() + 1;
    let body = &text[range.0..range.1];
    let mut out = Vec::new();
    let pins = ["set_var(", "remove_var(", ".env(", ".env_remove("];
    let reads = ["var(", "var_os(", "env!(", "vars_os("];
    let mut set_rom = Vec::new();
    for v in vars() {
        let quoted = format!("\"{v}\"");
        for (p, _) in body.match_indices(&quoted) {
            let at = range.0 + p;
            if in_str[at] {
                continue;
            }
            let before = text[..at].trim_end();
            if let Some(a) = pins.iter().find(|a| before.ends_with(*a)) {
                if v.ends_with("ROM_DIR") && a.ends_with("set_var(") {
                    set_rom.push(at);
                }
                continue;
            }
            // Named as text (an error it says, a list it holds, a JSON
            // key): not a read.
            if reads.iter().any(|r| before.ends_with(r)) {
                out.push(format!("{}: reads {v}", line(at)));
            }
        }
    }
    for (p, _) in body.match_indices("set_var(\"GP_ARCADE_") {
        set_rom.push(range.0 + p);
    }
    for (p, _) in body.match_indices(&format!("{}(", romset_fn())) {
        let at = range.0 + p;
        let word_start = at == 0 || {
            let c = code[at - 1];
            !(c.is_ascii_alphanumeric() || c == b'_')
        };
        if in_str[at] || !word_start {
            continue;
        }
        if !set_rom.iter().any(|&s| s < at) {
            out.push(format!(
                "{}: calls {} on this machine's ROM folder",
                line(at),
                romset_fn()
            ));
        }
    }
    for h in helpers() {
        for (p, _) in body.match_indices(&h) {
            let at = range.0 + p;
            let word_start = at == 0 || {
                let c = code[at - 1];
                !(c.is_ascii_alphanumeric() || c == b'_')
            };
            if in_str[at] || !word_start {
                continue;
            }
            out.push(format!(
                "{}: calls {} (the person's game and set)",
                line(at),
                h.trim_end_matches('(')
            ));
        }
    }
    out.sort();
    out
}

/// Every break of the rule in the file at `path` (whose test-module files
/// are read as wholly test code), as `PATH:LINE: WHAT`.
fn check_file(path: &Path, whole: bool, shown: &str, out: &mut Vec<String>) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    let (code, in_str) = lex(&text);
    let (ranges, files) = test_items(&code, &in_str);
    let ranges = if whole { vec![(0, code.len())] } else { ranges };
    for r in ranges {
        for b in breaks(&code, &in_str, r) {
            out.push(format!("{shown}:{b}"));
        }
    }
    // `#[cfg(test)] mod NAME;`: NAME.rs or NAME/mod.rs beside a mod.rs,
    // lib.rs or main.rs, else in the folder named for this file.
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let dir = path.parent().unwrap_or(Path::new("."));
    let dir = if ["mod", "lib", "main"].contains(&stem) {
        dir.to_path_buf()
    } else {
        dir.join(stem)
    };
    for name in files {
        for child in [
            dir.join(format!("{name}.rs")),
            dir.join(&name).join("mod.rs"),
        ] {
            if child.is_file() {
                let shown_child = child
                    .strip_prefix(repo())
                    .unwrap_or(&child)
                    .display()
                    .to_string();
                check_file(&child, true, &shown_child, out);
            }
        }
    }
}

/// The files whose test modules read this machine's ROMs on purpose:
/// gp-nes's ROM-tier boot check (registered_roms_boot_as_their_files_do),
/// which returns at once unless GP_CHECK_ROM_BOOT is set, as only `just
/// check-roms` sets it.
const KEPT: &[&str] = &["crates/gp-nes/src/board.rs"];

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

#[test]
fn no_test_module_under_src_reads_this_machines_roms_or_games() {
    let root = repo();
    let mut files = Vec::new();
    for c in std::fs::read_dir(root.join("crates")).unwrap().flatten() {
        rust_files(&c.path().join("src"), &mut files);
    }
    files.sort();
    assert!(files.len() > 100, "found only {} files", files.len());
    let mut out = Vec::new();
    for f in &files {
        let shown = f.strip_prefix(&root).unwrap_or(f).display().to_string();
        check_file(f, false, &shown, &mut out);
    }
    // Kept, read on purpose: the ROM tier's own check, which runs only when
    // `just check-roms` sets its switch and is told the ROM folder.
    out.retain(|b| !KEPT.iter().any(|k| b.starts_with(&format!("{k}:"))));
    out.sort();
    out.dedup();
    assert!(
        out.is_empty(),
        "{} places in test modules under crates/*/src read this machine's ROM \
         folder or games checkout (make the folder in the test, or move the \
         game's check to the game's folder):\n{}",
        out.len(),
        out.join("\n")
    );
}

/// The rule on made-up text: a pin to the test's own folder passes, a read
/// fails, and so do the local-romset finder before a pin, a test_* helper, and the
/// same outside a test item (not read).
#[test]
fn the_rule_holds_on_made_up_text() {
    let v = &vars()[0];
    let g = &vars()[1];
    let rs = romset_fn();
    let check = |src: &str| {
        let (code, in_str) = lex(src);
        let (ranges, _) = test_items(&code, &in_str);
        ranges
            .into_iter()
            .flat_map(|r| breaks(&code, &in_str, r))
            .collect::<Vec<_>>()
    };
    let pinned = format!(
        "#[cfg(test)]\nmod tests {{\n    #[test]\n    fn t() {{\n        let d = tmp();\n        // {v} is read by {rs}()\n        std::env::set_var(\"{v}\", &d);\n        let x = '{{';\n        let s = r#\"{{\"{v}\": 1}}\"#;\n        Command::new(\"gp\").env(\"{g}\", &d).env_remove(\n            \"{v}\");\n        assert!(e.contains(\"{g}\"));\n        for k in [\"{v}\", \"HOME\"] {{}}\n        let _ = {rs}(\"rooftop\");\n    }}\n}}\n"
    );
    assert_eq!(check(&pinned), Vec::<String>::new());

    let read = format!(
        "fn live() {{ std::env::var(\"{v}\"); }}\n#[cfg(test)]\nmod tests {{\n    fn t() {{\n        let a = std::env::var_os(\"{g}\");\n        let b = {rs}(\"rooftop\");\n        std::env::set_var(\"{v}\", tmp());\n        let c = gp_arcade::romset::test_oracle_set_path();\n    }}\n}}\n"
    );
    assert_eq!(
        check(&read),
        vec![
            format!("5: reads {g}"),
            format!("6: calls {rs} on this machine's ROM folder"),
            "8: calls test_oracle_set_path (the person's game and set)".to_string(),
        ]
    );

    let helper = format!(
        "#[cfg(all(test, feature = \"x\"))]\nfn dir() -> Option<PathBuf> {{\n    std::env::var_os(\"{g}\").map(PathBuf::from)\n}}\n#[cfg(not(test))]\nfn live() {{ std::env::var(\"{v}\"); }}\n"
    );
    assert_eq!(check(&helper), vec![format!("3: reads {g}")]);
}
