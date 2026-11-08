//! The Rust protocol in this crate must match wit/runtime/runtime.wit: the
//! same `runtime` methods with the same parameters, results where the WIT
//! has results, and the same record fields, variant cases and flags.
//!
//! The methods are compared with the source text of the `Runtime` trait.
//! The records are compared with the struct literals and matches below,
//! which stop compiling when a Rust field or case is added or renamed.

use std::path::Path;

use gp_runtime::{
    Capabilities, Entry, Event, InputFrame, LoadHow, LoadNote, PatchResult, Rect, Until, Value,
    WIT_PACKAGE,
};
use wit_parser::{FunctionKind, Resolve, Type, TypeDefKind, TypeId};

fn resolve() -> (Resolve, wit_parser::PackageId) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../wit/runtime");
    let mut resolve = Resolve::default();
    let (pkg, _) = resolve.push_dir(&dir).expect("wit/runtime resolves");
    (resolve, pkg)
}

fn type_named(resolve: &Resolve, pkg: wit_parser::PackageId, name: &str) -> TypeId {
    let types = resolve.packages[pkg].interfaces["types"];
    resolve.interfaces[types].types[name]
}

fn snake(kebab: &str) -> String {
    kebab.replace('-', "_")
}

/// One trait method as the source declares it.
#[derive(Debug, PartialEq)]
struct Method {
    name: String,
    params: Vec<String>,
    fallible: bool,
}

/// The methods of `pub trait Runtime` in src/lib.rs, from its text.
fn trait_methods() -> Vec<Method> {
    let source = include_str!("../src/lib.rs");
    let start = source.find("pub trait Runtime").expect("the trait");
    let body = &source[start..];
    let end = body.find("\n}\n").expect("the end of the trait");
    let code: String = body[..end]
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join(" ");
    code.split(" fn ")
        .skip(1)
        .map(|decl| {
            let open = decl.find('(').expect("parameters");
            let close = decl.find(')').expect("parameters");
            let params = decl[open + 1..close]
                .split(',')
                .map(str::trim)
                .filter(|p| !p.is_empty() && !p.ends_with("self"))
                .map(|p| p.split(':').next().unwrap().trim().to_owned())
                .collect();
            let ret = decl[close..].split(';').next().unwrap();
            Method {
                name: decl[..open].trim().to_owned(),
                params,
                fallible: ret.contains("-> Result<"),
            }
        })
        .collect()
}

#[test]
fn package_matches() {
    let (resolve, pkg) = resolve();
    assert_eq!(resolve.packages[pkg].name.to_string(), WIT_PACKAGE);
}

#[test]
fn runtime_methods_match_the_trait() {
    let (resolve, pkg) = resolve();
    let session = resolve.packages[pkg].interfaces["session"];
    let wit: Vec<Method> = resolve.interfaces[session]
        .functions
        .values()
        .filter(|f| matches!(f.kind, FunctionKind::Method(_)))
        .map(|f| Method {
            name: snake(f.item_name()),
            params: f
                .params
                .iter()
                .filter(|p| p.name != "self")
                .map(|p| snake(&p.name))
                .collect(),
            fallible: matches!(
                f.result,
                Some(Type::Id(id)) if matches!(resolve.types[id].kind, TypeDefKind::Result(_))
            ),
        })
        .collect();
    assert_eq!(trait_methods(), wit);
}

fn record_fields(name: &str) -> Vec<String> {
    let (resolve, pkg) = resolve();
    match &resolve.types[type_named(&resolve, pkg, name)].kind {
        TypeDefKind::Record(r) => r.fields.iter().map(|f| snake(&f.name)).collect(),
        other => panic!("{name} is a {}", other.as_str()),
    }
}

fn variant_cases(name: &str) -> Vec<String> {
    let (resolve, pkg) = resolve();
    match &resolve.types[type_named(&resolve, pkg, name)].kind {
        TypeDefKind::Variant(v) => v.cases.iter().map(|c| c.name.clone()).collect(),
        other => panic!("{name} is a {}", other.as_str()),
    }
}

fn enum_cases(name: &str) -> Vec<String> {
    let (resolve, pkg) = resolve();
    match &resolve.types[type_named(&resolve, pkg, name)].kind {
        TypeDefKind::Enum(e) => e.cases.iter().map(|c| c.name.clone()).collect(),
        other => panic!("{name} is a {}", other.as_str()),
    }
}

#[test]
fn records_match() {
    // Each literal names every field, so adding or renaming a Rust field
    // breaks the build, and the list beside it is checked against the WIT.
    let _ = InputFrame {
        buttons: 0,
        axes: vec![],
        reset: false,
    };
    assert_eq!(record_fields("input-frame"), ["buttons", "axes", "reset"]);
    let _ = Rect {
        x: 0,
        y: 0,
        w: 0,
        h: 0,
    };
    assert_eq!(record_fields("rect"), ["x", "y", "w", "h"]);
    let entry = Entry {
        path: String::new(),
        value: Value::Int(0),
    };
    assert_eq!(record_fields("entry"), ["path", "value"]);
    let event = Event {
        frame: 0,
        kind: String::new(),
        entity: None,
        data: vec![entry],
    };
    assert_eq!(record_fields("event"), ["frame", "kind", "entity", "data"]);
    let _ = Until {
        met: false,
        events: vec![event],
    };
    assert_eq!(record_fields("until"), ["met", "events"]);
    let _ = LoadNote {
        path: String::new(),
        how: LoadHow::Loaded,
        why: String::new(),
    };
    assert_eq!(record_fields("load-note"), ["path", "how", "why"]);
}

#[test]
fn variants_and_flags_match() {
    let value_case = |v: &Value| match v {
        Value::Int(_) => "int",
        Value::Fixed(_) => "fixed",
        Value::Flag(_) => "flag",
        Value::Text(_) => "text",
        Value::Entity(_) => "entity",
        Value::Vec2(..) => "vec2",
    };
    let values = [
        Value::Int(0),
        Value::Fixed(0),
        Value::Flag(false),
        Value::Text(String::new()),
        Value::Entity(0),
        Value::Vec2(0, 0),
    ];
    assert_eq!(
        values.iter().map(value_case).collect::<Vec<_>>(),
        variant_cases("value")
    );
    let patch_case = |p: &PatchResult| match p {
        PatchResult::Applied => "applied",
        PatchResult::NeedsReload => "needs-reload",
        PatchResult::Rejected(_) => "rejected",
    };
    let patches = [
        PatchResult::Applied,
        PatchResult::NeedsReload,
        PatchResult::Rejected(String::new()),
    ];
    assert_eq!(
        patches.iter().map(patch_case).collect::<Vec<_>>(),
        variant_cases("patch-result")
    );

    let how_case = |h: &LoadHow| match h {
        LoadHow::Loaded => "loaded",
        LoadHow::Approximated => "approximated",
        LoadHow::Missing => "missing",
    };
    let hows = [LoadHow::Loaded, LoadHow::Approximated, LoadHow::Missing];
    assert_eq!(
        hows.iter().map(how_case).collect::<Vec<_>>(),
        enum_cases("load-how")
    );
    assert!(hows.iter().all(|h| h.name() == how_case(h)));

    let (resolve, pkg) = resolve();
    let TypeDefKind::Flags(flags) = &resolve.types[type_named(&resolve, pkg, "capabilities")].kind
    else {
        panic!("capabilities isn't a flags type");
    };
    let wit: Vec<&str> = flags.flags.iter().map(|f| f.name.as_str()).collect();
    let rust: Vec<&str> = Capabilities::ALL.iter().map(|(name, _)| *name).collect();
    assert_eq!(rust, wit);
}
