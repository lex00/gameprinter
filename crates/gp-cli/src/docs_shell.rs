//! `gp self shell`: the runtime shell's generated files (docs/shell.md).
//! From wit/shell/storage.wit it writes the storage interface's bindings,
//! a Rust trait (crates/gp-present/src/shell/storage.rs) and the
//! JavaScript host's check (scripts/web/page/storage-wit.js); from
//! `gp_present::shell` it writes the web preview's copy of the key table
//! and the shell's policy (scripts/web/page/shell-keys.js). A test fails
//! while any of them is stale; `just docs` writes them.
//!
//! Ported from game-printer's docs_shell.rs, where it was `gp docs shell`;
//! gameprinter keeps the generated-docs commands under `gp self`.

use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context, Result, bail};
use wit_parser::{Docs, Resolve, Type, TypeDefKind};

/// The files written, relative to the repo, with their text.
pub(crate) fn files(repo: &Path) -> Result<Vec<(&'static str, String)>> {
    let wit = repo.join("wit/shell");
    let mut resolve = Resolve::default();
    let (pkg, _) = resolve
        .push_dir(&wit)
        .map_err(|e| anyhow::anyhow!("{}: {e:#}", wit.display()))?;
    let package = resolve.packages[pkg].name.to_string();
    let iface = *resolve.packages[pkg]
        .interfaces
        .get("storage")
        .context("wit/shell has no `storage` interface")?;
    Ok(vec![
        (
            "crates/gp-present/src/shell/storage.rs",
            rust_bindings(&resolve, iface, &package)?,
        ),
        (
            "scripts/web/page/storage-wit.js",
            js_bindings(&resolve, iface, &package)?,
        ),
        ("scripts/web/page/shell-keys.js", js_keys()),
    ])
}

/// Writes the files, or with `check` writes nothing and names the stale.
pub(crate) fn run(repo: &Path, check: bool) -> Result<crate::Outcome> {
    let mut stale = Vec::new();
    for (rel, text) in files(repo)? {
        let path = repo.join(rel);
        if std::fs::read_to_string(&path).ok().as_deref() == Some(text.as_str()) {
            continue;
        }
        if !check {
            std::fs::write(&path, &text).with_context(|| format!("writing {}", path.display()))?;
        }
        stale.push(rel);
    }
    let text = match (check, stale.is_empty()) {
        (_, true) => "the shell's generated files are current".to_owned(),
        (true, false) => format!("stale: {}; run `just docs`", stale.join(", ")),
        (false, false) => format!("wrote {}", stale.join(", ")),
    };
    Ok(crate::Outcome {
        json: serde_json::json!({ "stale": stale, "written": !check }),
        text,
        exit: u8::from(check && !stale.is_empty()),
    })
}

fn snake(kebab: &str) -> String {
    kebab.replace('-', "_")
}

fn camel(kebab: &str) -> String {
    let mut out = String::new();
    let mut up = false;
    for c in kebab.chars() {
        if c == '-' {
            up = true;
        } else if up {
            out.extend(c.to_uppercase());
            up = false;
        } else {
            out.push(c);
        }
    }
    out
}

fn upper_camel(kebab: &str) -> String {
    let c = camel(kebab);
    let mut chars = c.chars();
    chars
        .next()
        .map(|f| f.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

fn doc_lines(docs: &Docs, indent: &str, mark: &str) -> String {
    docs.contents
        .as_deref()
        .map(|d| {
            d.lines()
                .map(|l| {
                    if l.is_empty() {
                        format!("{indent}{mark}\n")
                    } else {
                        format!("{indent}{mark} {l}\n")
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

/// A WIT type in Rust: borrowed as a parameter, owned as a result.
fn rust_type(resolve: &Resolve, ty: &Type, param: bool) -> Result<String> {
    Ok(match ty {
        Type::Bool => "bool".into(),
        Type::U8 => "u8".into(),
        Type::U32 => "u32".into(),
        Type::U64 => "u64".into(),
        Type::String if param => "&str".into(),
        Type::String => "String".into(),
        Type::Id(id) => {
            let def = &resolve.types[*id];
            match &def.kind {
                TypeDefKind::List(Type::U8) if param => "&[u8]".into(),
                TypeDefKind::List(t) => format!("Vec<{}>", rust_type(resolve, t, false)?),
                TypeDefKind::Option(t) => format!("Option<{}>", rust_type(resolve, t, false)?),
                TypeDefKind::Result(r) => format!(
                    "Result<{}, {}>",
                    r.ok.as_ref()
                        .map(|t| rust_type(resolve, t, false))
                        .transpose()?
                        .unwrap_or_else(|| "()".into()),
                    r.err
                        .as_ref()
                        .map(|t| rust_type(resolve, t, false))
                        .transpose()?
                        .unwrap_or_else(|| "()".into()),
                ),
                TypeDefKind::Record(_) => {
                    upper_camel(def.name.as_deref().context("a record with no name")?)
                }
                k => bail!("the shell's WIT generator has no Rust for {k:?}"),
            }
        }
        t => bail!("the shell's WIT generator has no Rust for {t:?}"),
    })
}

/// A WIT type as the JavaScript host's promise gives it, in JSDoc.
fn js_type(resolve: &Resolve, ty: &Type) -> Result<String> {
    Ok(match ty {
        Type::Bool => "boolean".into(),
        Type::U8 | Type::U32 | Type::U64 => "number".into(),
        Type::String => "string".into(),
        Type::Id(id) => {
            let def = &resolve.types[*id];
            match &def.kind {
                TypeDefKind::List(Type::U8) => "Uint8Array".into(),
                TypeDefKind::List(t) => format!("{}[]", js_type(resolve, t)?),
                TypeDefKind::Option(t) => format!("{} | null", js_type(resolve, t)?),
                // An error rejects the promise with an Error of its text.
                TypeDefKind::Result(r) => {
                    r.ok.as_ref()
                        .map(|t| js_type(resolve, t))
                        .transpose()?
                        .unwrap_or_else(|| "void".into())
                }
                TypeDefKind::Record(_) => {
                    upper_camel(def.name.as_deref().context("a record with no name")?)
                }
                k => bail!("the shell's WIT generator has no JavaScript for {k:?}"),
            }
        }
        t => bail!("the shell's WIT generator has no JavaScript for {t:?}"),
    })
}

fn rust_bindings(
    resolve: &Resolve,
    iface: wit_parser::InterfaceId,
    package: &str,
) -> Result<String> {
    let i = &resolve.interfaces[iface];
    let mut s = String::new();
    let _ = writeln!(
        s,
        "//! The shell's storage interface, `{package}`'s `storage`, as Rust:\n//! generated from wit/shell/storage.wit by `gp self shell`. Don't edit\n//! it; change the WIT and run `just docs`.\n"
    );
    let _ = writeln!(s, "/// The WIT package these bindings are of.");
    let _ = writeln!(s, "pub const WIT_PACKAGE: &str = {package:?};\n");
    for (name, id) in &i.types {
        let def = &resolve.types[*id];
        let TypeDefKind::Record(r) = &def.kind else {
            bail!("the shell's WIT generator has no Rust for the type `{name}`");
        };
        s.push_str(&doc_lines(&def.docs, "", "///"));
        let _ = writeln!(s, "#[derive(Debug, Clone, PartialEq, Eq)]");
        let _ = writeln!(s, "pub struct {} {{", upper_camel(name));
        for f in &r.fields {
            s.push_str(&doc_lines(&f.docs, "    ", "///"));
            let _ = writeln!(
                s,
                "    pub {}: {},",
                snake(&f.name),
                rust_type(resolve, &f.ty, false)?
            );
        }
        let _ = writeln!(s, "}}\n");
    }
    let _ = writeln!(
        s,
        "/// What a target keeps for one game. Each function is the WIT's, with\n/// its name in snake case."
    );
    let _ = writeln!(s, "pub trait Storage {{");
    for (n, (name, f)) in i.functions.iter().enumerate() {
        if n > 0 {
            s.push('\n');
        }
        s.push_str(&doc_lines(&f.docs, "    ", "///"));
        let params = f
            .params
            .iter()
            .map(|p| {
                Ok(format!(
                    ", {}: {}",
                    snake(&p.name),
                    rust_type(resolve, &p.ty, true)?
                ))
            })
            .collect::<Result<String>>()?;
        let result = f
            .result
            .as_ref()
            .map(|t| Ok::<_, anyhow::Error>(format!(" -> {}", rust_type(resolve, t, false)?)))
            .transpose()?
            .unwrap_or_default();
        let _ = writeln!(s, "    fn {}(&mut self{params}){result};", snake(name));
    }
    let _ = writeln!(s, "}}");
    Ok(s)
}

fn js_bindings(resolve: &Resolve, iface: wit_parser::InterfaceId, package: &str) -> Result<String> {
    let i = &resolve.interfaces[iface];
    let mut s = String::new();
    let _ = writeln!(
        s,
        "// The shell's storage interface, `{package}`'s `storage`, for a\n// JavaScript host: generated from wit/shell/storage.wit by `gp self\n// shell`. Don't edit it; change the WIT and run `just docs`.\n//\n// A host is an object with each function below, its name in camel case,\n// returning a promise: of the WIT's result, with `option` as the value or\n// null and `result` as its ok value or a rejection with an Error of its\n// error text.\n"
    );
    let _ = writeln!(s, "export const WIT_PACKAGE = {package:?};\n");
    for (name, id) in &i.types {
        let def = &resolve.types[*id];
        let TypeDefKind::Record(r) = &def.kind else {
            bail!("the shell's WIT generator has no JavaScript for the type `{name}`");
        };
        let _ = writeln!(s, "/**");
        s.push_str(&doc_lines(&def.docs, " ", "*"));
        let _ = writeln!(s, " * @typedef {{object}} {}", upper_camel(name));
        for f in &r.fields {
            let doc = f.docs.contents.as_deref().unwrap_or("").replace('\n', " ");
            let _ = writeln!(
                s,
                " * @property {{{}}} {}{}",
                js_type(resolve, &f.ty)?,
                camel(&f.name),
                if doc.is_empty() {
                    String::new()
                } else {
                    format!(" {doc}")
                }
            );
        }
        let _ = writeln!(s, " */\n");
    }
    let _ = writeln!(
        s,
        "/**\n * What a target keeps for one game.\n * @typedef {{object}} Storage"
    );
    for (name, f) in &i.functions {
        let params = f
            .params
            .iter()
            .map(|p| Ok(format!("{}: {}", camel(&p.name), js_type(resolve, &p.ty)?)))
            .collect::<Result<Vec<_>>>()?
            .join(", ");
        let result = f
            .result
            .as_ref()
            .map(|t| js_type(resolve, t))
            .transpose()?
            .unwrap_or_else(|| "void".into());
        let doc = f.docs.contents.as_deref().unwrap_or("").replace('\n', " ");
        let _ = writeln!(
            s,
            " * @property {{({params}) => Promise<{result}>}} {} {doc}",
            camel(name)
        );
    }
    let _ = writeln!(s, " */\n");
    let _ = writeln!(
        s,
        "// Each function's name and parameters, in the WIT's order."
    );
    let _ = writeln!(s, "export const STORAGE = [");
    for (name, f) in &i.functions {
        let params = f
            .params
            .iter()
            .map(|p| format!("{:?}", camel(&p.name)))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(s, "  {{ name: {:?}, params: [{params}] }},", camel(name));
    }
    let _ = writeln!(s, "];\n");
    let _ = writeln!(
        s,
        "// Throws unless `host` has every function of the interface, each taking\n// its parameters.\nexport function checkStorage(host) {{\n  for (const {{ name, params }} of STORAGE) {{\n    if (typeof host?.[name] !== \"function\") throw new Error(`storage host: no ${{name}}()`);\n    if (host[name].length !== params.length) {{\n      throw new Error(`storage host: ${{name}}() takes ${{host[name].length}} parameters, not ${{params.length}} (${{params.join(\", \")}})`);\n    }}\n  }}\n  return host;\n}}"
    );
    Ok(s)
}

/// The key table and the shell's policy for the web preview.
fn js_keys() -> String {
    use gp_present::shell::{self, keys};
    let list = |v: &[&str]| {
        v.iter()
            .map(|k| format!("{k:?}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut s = String::new();
    let _ = writeln!(
        s,
        "// The runtime shell's keys and gamepad buttons and its policy, as every\n// target reads them (docs/shell.md): generated from gp_present::shell by\n// `gp self shell`. Don't edit it; change the table and run `just docs`.\n//\n// A key is its KeyboardEvent.code; `standard` is each button's index in\n// the browser's standard gamepad layout. `on` says which targets have a\n// binding (both or web); the preview reads the ones it has. `needs`\n// says what of the shell's settings it needs: always, not-console-only (off\n// while Console only is on) or quick-buttons (Quick save buttons on).\n"
    );
    let _ = writeln!(s, "export const BINDINGS = [");
    for b in keys::BINDINGS {
        let standard = b
            .buttons
            .iter()
            .map(|x| keys::standard_index(x).map_or("null".into(), |i| i.to_string()))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(
            s,
            "  {{ action: {:?}, keys: [{}], shift: {}, buttons: [{}], standard: [{standard}], on: {:?}, when: {:?}, needs: {:?} }},",
            b.action,
            list(b.keys),
            b.shift,
            list(b.buttons),
            b.on.name(),
            b.when.name(),
            b.needs.name()
        );
    }
    let _ = writeln!(s, "];\n");
    let _ = writeln!(s, "// Controller 1's and 2's buttons by bit, A to Right.");
    let _ = writeln!(s, "export const NES = [{}];", list(&keys::NES));
    let _ = writeln!(s, "export const NES2 = [{}];", list(&keys::NES2));
    let _ = writeln!(
        s,
        "// The settings screen's keys, in its key numbers' order."
    );
    let _ = writeln!(s, "export const SCREEN = [{}];\n", list(&keys::SCREEN));
    let _ = writeln!(
        s,
        "// The rewind ring: a state every `every` frames, `seconds` of them.\nexport const REWIND = {{ every: {}, seconds: {} }};",
        shell::REWIND_EVERY,
        shell::REWIND_SECONDS
    );
    let _ = writeln!(
        s,
        "// The slots the number keys reach, where a slot is kept, the stamp of\n// when it was saved beside it, and the autosave's slot.\nexport const QUICK_SLOTS = {};\nexport const slotName = (n) => `{}`;\nexport const whenName = (n) => `{}`;\nexport const AUTOSAVE_SLOT = {};",
        shell::QUICK_SLOTS,
        shell::slot_name(usize::MAX).replace(&usize::MAX.to_string(), "${n}"),
        shell::slots::when_name(usize::MAX).replace(&usize::MAX.to_string(), "${n}"),
        shell::AUTOSAVE_SLOT
    );
    let _ = writeln!(
        s,
        "// The shell's own settings, kept through storage as this file\n// (gp_present::shell::Settings).\nexport const SHELL_SETTINGS = {:?};\nexport const exportName = (game, n) => `{}`;",
        shell::slots::SETTINGS_NAME,
        shell::export_name("${game}", usize::MAX).replace(&usize::MAX.to_string(), "${n}")
    );
    let _ = writeln!(
        s,
        "// The sentences a refused save gets (gp_sim::save).\nexport const REFUSED = {{ otherBuild: {:?}, otherGame: {:?}, damaged: {:?} }};",
        gp_sim::save::OTHER_BUILD,
        gp_sim::save::OTHER_GAME,
        gp_sim::save::NOT_A_SAVE
    );
    s
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    /// The generated files are what `gp self shell` writes now.
    #[test]
    fn the_shell_s_generated_files_are_current() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for (rel, text) in super::files(&repo).unwrap() {
            let have = std::fs::read_to_string(repo.join(rel)).unwrap_or_default();
            assert!(have == text, "{rel} is out of date; run `just docs`");
        }
    }

    #[test]
    fn names_are_cased_for_each_language() {
        assert_eq!(super::camel("file-name"), "fileName");
        assert_eq!(super::upper_camel("picked"), "Picked");
        assert_eq!(super::snake("file-name"), "file_name");
    }
}
