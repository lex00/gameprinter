// Each hook's code, as a hash: the items of its hook crate that the hook
// can reach, so a change to a hook crate names the hooks it can move
// (docs/gate.md, "Affected clips"). In game-printer build.rs included this
// file to hash every template's hook crate into the gp binary; in
// gameprinter a game's hook crate is its package's ext/, outside gp's
// build, so affected.rs reads it with this at run time. It uses std, syn,
// quote, proc-macro2 and blake3 only.
//
// A crate is read from its src/lib.rs, following `mod x;` to x.rs or
// x/mod.rs. Each fn, const, static, type, struct, enum, union and trait
// is a unit named by its name (an enum by its variants' too), and each
// item of an `impl` is a unit owned by the type the impl is for. A unit
// reaches every unit named by an identifier in its tokens (its own name
// aside), and every unit owned by a type it names: a method runs on a
// value of its type, which some reached code made by naming the type.
// Names resolve as Rust resolves them, by module: a bare name reaches the
// unit of that name in its own module and what the module's `use` items
// import under it (any module's, after a glob of the crate's own), and a
// path through one of the crate's modules (`fire::step`, `super::p`)
// reaches that module's unit. Modules are told apart by their last name.
// A hook is the type its `exts()` registers (`Box::new(Fire)`),
// and its code is every unit reached from that type, with the units every
// hook shares: `exts()` itself, each `use`, `mod`, macro and other item,
// the crate's attributes, its Cargo.toml and every file under the crate
// but its .rs files. Names are matched by text, so a hook reaches more
// than it calls, never less. When anything can't be read so (a file that
// doesn't parse, a `#[path]`, an `exts()` this doesn't follow), every hook
// is the whole crate.

/// Each hook's hash by the name the crate's `exts()` registers it under;
/// `Err` says why the crate couldn't be read by hook.
#[allow(dead_code)]
pub fn hook_hashes(
    dir: &std::path::Path,
) -> Result<std::collections::BTreeMap<String, String>, String> {
    let c = closures(dir)?;
    let mut out = std::collections::BTreeMap::new();
    for (hook, (ty, seen)) in &c.hooks {
        // Each unit's text and the names it resolves to, which follow the
        // imports of its module.
        let mut parts: Vec<String> = seen
            .iter()
            .map(|&i| {
                let u = &c.units[i];
                let refs: Vec<&str> = u.refs.iter().map(String::as_str).collect();
                format!("{}\n{}", u.text, refs.join(" "))
            })
            .collect();
        parts.sort();
        let mut h = blake3::Hasher::new();
        h.update(format!("hook {hook} {ty}\n").as_bytes());
        for p in c.shared.iter().chain(&parts) {
            h.update(&(p.len() as u64).to_le_bytes());
            h.update(p.as_bytes());
        }
        out.insert(hook.clone(), h.finalize().to_hex().to_string());
    }
    Ok(out)
}

/// A hook crate read by unit: its units, what every hook shares, and each
/// hook's type and the units it reaches.
#[allow(dead_code)]
struct Closures {
    units: Vec<Unit>,
    shared: Vec<String>,
    hooks: std::collections::BTreeMap<String, (String, std::collections::BTreeSet<usize>)>,
    /// How each hook reached each unit: the unit that named it (none for
    /// the hook's type) and the name.
    via: std::collections::BTreeMap<
        String,
        std::collections::BTreeMap<usize, (Option<usize>, String)>,
    >,
}

#[allow(dead_code)]
fn closures(dir: &std::path::Path) -> Result<Closures, String> {
    use std::collections::{BTreeMap, BTreeSet};
    let src = dir.join("src");
    let mut units: Vec<Unit> = Vec::new();
    let mut shared: Vec<String> = Vec::new();
    let mut roots: Option<Vec<(String, String)>> = None;
    let mut uses: std::collections::BTreeMap<String, Imports> = Default::default();
    let mut heads: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    let mut r = Read {
        units: &mut units,
        shared: &mut shared,
        roots: &mut roots,
        uses: &mut uses,
        heads: &mut heads,
    };
    read_module(&src.join("lib.rs"), &src, "", &mut r)?;
    for u in &mut units {
        if let Some(h) = heads.get(&u.at) {
            u.text = format!("{}\n{}", h.join("\n"), u.text);
        }
    }
    let roots = roots.ok_or("lib.rs has no fn exts()")?;
    // Every file under the crate but its Rust sources, and its manifest:
    // data a hook may include.
    let mut other = Vec::new();
    files(dir, dir, &mut other);
    for (rel, bytes) in other {
        if !rel.ends_with(".rs") {
            shared.push(format!("file {rel} {}", blake3::hash(&bytes).to_hex()));
        }
    }
    shared.sort();
    // The crate's modules by their last name.
    let mut modules: BTreeSet<String> = BTreeSet::new();
    for u in &units {
        modules.insert(last(&u.at).to_owned());
    }
    for m in uses.keys() {
        modules.insert(last(m).to_owned());
    }
    let none = Imports::default();
    for u in &mut units {
        let mut r = BTreeSet::new();
        let imports = uses.get(&u.at).unwrap_or(&none);
        let cx = Names {
            at: &u.at,
            modules: &modules,
            imports,
            uses: &uses,
        };
        refs_of(u.ts.clone(), &cx, &mut r);
        refs_of(u.head.clone(), &cx, &mut r);
        let m = last(&u.at);
        for n in &u.names {
            r.remove(&format!("{m}::{n}"));
        }
        u.refs = r;
    }
    let mut by_name: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, u) in units.iter().enumerate() {
        if let Some(o) = &u.owner {
            by_name.entry(format!("owner {o}")).or_default().push(i);
            continue;
        }
        let m = last(&u.at);
        for n in &u.names {
            by_name.entry(n.clone()).or_default().push(i);
            by_name.entry(format!("{m}::{n}")).or_default().push(i);
        }
    }
    let mut hooks = BTreeMap::new();
    let mut via = BTreeMap::new();
    for (hook, ty) in roots {
        let mut seen: BTreeSet<usize> = BTreeSet::new();
        let mut names: BTreeSet<String> = BTreeSet::new();
        let mut how: BTreeMap<usize, (Option<usize>, String)> = BTreeMap::new();
        let mut todo: Vec<(Option<usize>, String)> = vec![(None, ty.clone())];
        while let Some((from, n)) = todo.pop() {
            if !names.insert(n.clone()) {
                continue;
            }
            // A type's methods, by its last name.
            let own = format!("owner {}", n.rsplit("::").next().unwrap_or(&n));
            for k in [&n, &own] {
                for &i in by_name.get(k.as_str()).map_or(&[][..], Vec::as_slice) {
                    if seen.insert(i) {
                        how.insert(i, (from, n.clone()));
                        todo.extend(units[i].refs.iter().map(|r| (Some(i), r.clone())));
                    }
                }
            }
        }
        hooks.insert(hook.clone(), (ty, seen));
        via.insert(hook, how);
    }
    Ok(Closures {
        units,
        shared,
        hooks,
        via,
    })
}

/// One unit of a hook crate: the names it defines, the type it belongs
/// to, the identifiers it names, and its text (with where it is).
#[allow(dead_code)]
struct Unit {
    at: String,
    names: Vec<String>,
    owner: Option<String>,
    ts: proc_macro2::TokenStream,
    /// Its impl's header (the type, the trait), whose names it reaches.
    head: proc_macro2::TokenStream,
    refs: std::collections::BTreeSet<String>,
    text: String,
}

/// What reading a crate gathers.
#[allow(dead_code)]
struct Read<'a> {
    units: &'a mut Vec<Unit>,
    shared: &'a mut Vec<String>,
    roots: &'a mut Option<Vec<(String, String)>>,
    /// Each module's imports, by its path.
    uses: &'a mut std::collections::BTreeMap<String, Imports>,
    /// Each module's `mod` declaration with its attributes, by its path:
    /// it goes with each of the module's units.
    heads: &'a mut std::collections::BTreeMap<String, Vec<String>>,
}

/// What a module's `use` items bring in: each name and the units it
/// stands for (`module::name`, by the module's last name), and the first
/// segments of its globs' paths.
#[allow(dead_code)]
#[derive(Default)]
struct Imports {
    names: std::collections::BTreeMap<String, Vec<String>>,
    globs: Vec<String>,
}

/// A module's path's last name, `""` for the crate's root.
#[allow(dead_code)]
fn last(at: &str) -> &str {
    at.rsplit("::").next().unwrap_or("")
}

/// The module a path's segments lead to from module `at`, by its last
/// name: `crate::a::b` is `b`, `super` from `x::y` is `x`.
#[allow(dead_code)]
fn module_of(prefix: &[String], at: &str) -> String {
    let mut path: Vec<&str> = at.split("::").filter(|s| !s.is_empty()).collect();
    for (i, seg) in prefix.iter().enumerate() {
        match seg.as_str() {
            "crate" if i == 0 => path.clear(),
            "self" => {}
            "super" => {
                path.pop();
            }
            s => path.push(s),
        }
    }
    path.last().map_or(String::new(), |s| (*s).to_owned())
}

#[allow(dead_code)]
fn use_tree(t: &syn::UseTree, prefix: &mut Vec<String>, at: &str, out: &mut Imports) {
    use syn::UseTree as U;
    match t {
        U::Path(p) => {
            prefix.push(p.ident.to_string());
            use_tree(&p.tree, prefix, at, out);
            prefix.pop();
        }
        U::Name(n) => {
            let name = n.ident.to_string();
            let m = module_of(prefix, at);
            out.names
                .entry(name.clone())
                .or_default()
                .push(format!("{m}::{name}"));
        }
        U::Rename(r) => {
            let m = module_of(prefix, at);
            out.names
                .entry(r.rename.to_string())
                .or_default()
                .push(format!("{m}::{}", r.ident));
        }
        U::Glob(_) => out.globs.push(prefix.first().cloned().unwrap_or_default()),
        U::Group(g) => {
            for t in &g.items {
                use_tree(t, prefix, at, out);
            }
        }
    }
}

/// What resolving a unit's names needs.
#[allow(dead_code)]
struct Names<'a> {
    at: &'a str,
    modules: &'a std::collections::BTreeSet<String>,
    imports: &'a Imports,
    uses: &'a std::collections::BTreeMap<String, Imports>,
}

/// The names `ts` refers to, resolved as [`Names`] says: `m::f` for the
/// unit `f` of module `m` (by its last name), a bare `f` for any module's.
#[allow(dead_code)]
fn refs_of(
    ts: proc_macro2::TokenStream,
    cx: &Names<'_>,
    out: &mut std::collections::BTreeSet<String>,
) {
    use proc_macro2::TokenTree as T;
    let v: Vec<T> = ts.into_iter().collect();
    let sep = |i: usize| matches!((v.get(i), v.get(i + 1)), (Some(T::Punct(a)), Some(T::Punct(b))) if a.as_char() == ':' && b.as_char() == ':');
    let ident = |i: usize| match v.get(i) {
        Some(T::Ident(x)) => Some(x.to_string().trim_start_matches("r#").to_owned()),
        _ => None,
    };
    let glob = cx
        .imports
        .globs
        .iter()
        .any(|g| matches!(g.as_str(), "crate" | "self" | "super") || cx.modules.contains(g));
    let mut i = 0;
    while i < v.len() {
        if let T::Group(g) = &v[i] {
            refs_of(g.stream(), cx, out);
            i += 1;
            continue;
        }
        let Some(a) = ident(i) else {
            i += 1;
            continue;
        };
        // A path: follow its module segments to the name after them.
        let mut prefix = vec![a.clone()];
        let mut j = i;
        while sep(j + 1)
            && let Some(b) = ident(j + 3)
        {
            let head = prefix.last().map(String::as_str).unwrap_or("");
            let is_module = matches!(head, "crate" | "self" | "super") || cx.modules.contains(head);
            if !is_module {
                break;
            }
            prefix.push(b);
            j += 3;
        }
        if prefix.len() > 1 {
            let name = prefix.pop().unwrap_or_default();
            let m = module_of(&prefix, cx.at);
            out.insert(format!("{m}::{name}"));
            // A name the module only imports.
            for (at, imp) in cx.uses {
                if last(at) == m
                    && let Some(t) = imp.names.get(&name)
                {
                    out.extend(t.iter().cloned());
                }
            }
            i = j + 1;
            continue;
        }
        if !matches!(a.as_str(), "crate" | "self" | "super" | "Self") {
            out.insert(format!("{}::{a}", last(cx.at)));
            if let Some(t) = cx.imports.names.get(&a) {
                out.extend(t.iter().cloned());
            }
            if glob {
                out.insert(a);
            }
        }
        i += 1;
    }
}

#[allow(dead_code)]
fn files(root: &std::path::Path, dir: &std::path::Path, out: &mut Vec<(String, Vec<u8>)>) {
    let mut entries: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .map(|r| r.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    entries.sort();
    for p in entries {
        let rel = p
            .strip_prefix(root)
            .map(|r| r.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        if rel == "target" || rel.starts_with('.') {
            continue;
        }
        if p.is_dir() {
            files(root, &p, out);
        } else if let Ok(b) = std::fs::read(&p) {
            out.push((rel, b));
        }
    }
}

#[allow(dead_code)]
fn idents(ts: proc_macro2::TokenStream, out: &mut std::collections::BTreeSet<String>) {
    for t in ts {
        match t {
            proc_macro2::TokenTree::Ident(i) => {
                out.insert(i.to_string().trim_start_matches("r#").to_owned());
            }
            proc_macro2::TokenTree::Group(g) => idents(g.stream(), out),
            _ => {}
        }
    }
}

/// An item's tokens without its doc comments (`#[doc = ".."]` and
/// `#![doc = ".."]`), which change nothing it does.
#[allow(dead_code)]
fn tokens(t: &impl quote::ToTokens) -> proc_macro2::TokenStream {
    let mut ts = proc_macro2::TokenStream::new();
    t.to_tokens(&mut ts);
    undoc(ts)
}

#[allow(dead_code)]
fn undoc(ts: proc_macro2::TokenStream) -> proc_macro2::TokenStream {
    use proc_macro2::{Delimiter, Group, TokenTree as T};
    let v: Vec<T> = ts.into_iter().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < v.len() {
        let doc = |g: &T| match g {
            T::Group(g) if g.delimiter() == Delimiter::Bracket => matches!(
                g.stream().into_iter().next(),
                Some(T::Ident(d)) if d == "doc"
            ),
            _ => false,
        };
        match &v[i] {
            T::Punct(p) if p.as_char() == '#' => {
                let bang = matches!(v.get(i + 1), Some(T::Punct(b)) if b.as_char() == '!');
                let at = if bang { i + 2 } else { i + 1 };
                if v.get(at).is_some_and(doc) {
                    i = at + 1;
                    continue;
                }
                out.push(v[i].clone());
            }
            T::Group(g) => {
                let mut n = Group::new(g.delimiter(), undoc(g.stream()));
                n.set_span(g.span());
                out.push(T::Group(n));
            }
            t => out.push(t.clone()),
        }
        i += 1;
    }
    out.into_iter().collect()
}

/// The last identifier of a type's path: `Fire` for `fire::Fire`.
#[allow(dead_code)]
fn type_name(t: &syn::Type) -> Option<String> {
    match t {
        syn::Type::Path(p) => p.path.segments.last().map(|s| s.ident.to_string()),
        _ => None,
    }
}

#[allow(dead_code)]
fn unit(
    at: &str,
    names: Vec<String>,
    owner: Option<String>,
    head: &str,
    t: &impl quote::ToTokens,
) -> Unit {
    let ts = tokens(t);
    Unit {
        text: format!("{at} {head} {ts}"),
        at: at.to_owned(),
        names,
        owner,
        ts,
        head: Default::default(),
        refs: Default::default(),
    }
}

/// Reads the module in `file` (its items' path `at`), whose child modules
/// are in `dir`.
#[allow(dead_code)]
fn read_module(
    file: &std::path::Path,
    dir: &std::path::Path,
    at: &str,
    r: &mut Read<'_>,
) -> Result<(), String> {
    let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let f = syn::parse_file(&text).map_err(|e| format!("{}: {e}", file.display()))?;
    for a in f.attrs.iter().filter(|a| !a.path().is_ident("doc")) {
        r.shared.push(format!("{at} attr {}", tokens(a)));
    }
    read_items(&f.items, dir, at, r)
}

#[allow(dead_code)]
fn read_items(
    items: &[syn::Item],
    dir: &std::path::Path,
    at: &str,
    r: &mut Read<'_>,
) -> Result<(), String> {
    use syn::Item;
    for item in items {
        let mut one = |names: Vec<String>, what: &str, t: &dyn quote::ToTokens| {
            let mut ts = proc_macro2::TokenStream::new();
            t.to_tokens(&mut ts);
            r.units.push(unit(at, names, None, what, &ts));
        };
        match item {
            Item::Fn(f) => {
                let name = f.sig.ident.to_string();
                if at.is_empty() && name == "exts" {
                    let roots = registered(f)?;
                    // A unit that names every hook's type, for code that
                    // calls exts() itself.
                    let mut u = unit(at, vec![name], None, "fn", f);
                    u.head = tokens(f);
                    r.units.push(u);
                    *r.roots = Some(roots);
                    continue;
                }
                one(vec![name], "fn", f);
            }
            Item::Const(c) => one(vec![c.ident.to_string()], "const", c),
            Item::Static(x) => one(vec![x.ident.to_string()], "static", x),
            Item::Type(t) => one(vec![t.ident.to_string()], "type", t),
            Item::Struct(x) => one(vec![x.ident.to_string()], "struct", x),
            Item::Union(u) => one(vec![u.ident.to_string()], "union", u),
            Item::Trait(t) => one(vec![t.ident.to_string()], "trait", t),
            Item::Enum(e) => {
                let mut names = vec![e.ident.to_string()];
                names.extend(e.variants.iter().map(|v| v.ident.to_string()));
                one(names, "enum", e);
            }
            Item::Use(u) => {
                // What it imports is in each unit's names (`refs`), which
                // its hash covers.
                use_tree(
                    &u.tree,
                    &mut Vec::new(),
                    at,
                    r.uses.entry(at.to_owned()).or_default(),
                );
            }
            Item::Impl(i) => {
                let Some(owner) = type_name(&i.self_ty) else {
                    r.shared.push(format!("{at} impl {}", tokens(i)));
                    continue;
                };
                // The impl's header goes with each of its items, so a
                // change to it changes them all.
                let mut head = i.clone();
                head.items.clear();
                let text = format!("impl {}", tokens(&head));
                let mut hr = tokens(&i.self_ty);
                if let Some((_, p, _)) = &i.trait_ {
                    hr.extend(tokens(p));
                }
                hr.extend(tokens(&i.generics));
                for it in &i.items {
                    let name = match it {
                        syn::ImplItem::Fn(f) => f.sig.ident.to_string(),
                        syn::ImplItem::Const(c) => c.ident.to_string(),
                        syn::ImplItem::Type(t) => t.ident.to_string(),
                        _ => String::new(),
                    };
                    let mut u = unit(at, vec![name], Some(owner.clone()), &text, it);
                    u.head = hr.clone();
                    r.units.push(u);
                }
            }
            Item::Mod(m) => {
                if m.attrs.iter().any(|a| a.path().is_ident("path")) {
                    return Err(format!("module {} has a #[path]", m.ident));
                }
                // A test module isn't in the build.
                if m.attrs.iter().any(|a| {
                    a.path().is_ident("cfg") && tokens(&a.meta).to_string() == "cfg (test)"
                }) {
                    continue;
                }
                let name = m.ident.to_string();
                let inner = if at.is_empty() {
                    name.clone()
                } else {
                    format!("{at}::{name}")
                };
                r.heads.entry(inner.clone()).or_default().push(format!(
                    "mod {} {}",
                    name,
                    m.attrs
                        .iter()
                        .map(|a| tokens(a).to_string())
                        .collect::<Vec<_>>()
                        .join(" ")
                ));
                match &m.content {
                    Some((_, items)) => read_items(items, &dir.join(&name), &inner, r)?,
                    None => {
                        let one = dir.join(format!("{name}.rs"));
                        let file = if one.is_file() {
                            one
                        } else {
                            dir.join(&name).join("mod.rs")
                        };
                        read_module(&file, &dir.join(&name), &inner, r)?;
                    }
                }
            }
            other => r.shared.push(format!("{at} item {}", tokens(other))),
        }
    }
    Ok(())
}

/// The hooks `exts()` registers: each `X.insert("name".into(), Box::new(T)
/// [as Box<dyn Ext>])` as the name and the type `T` (`m::T`, `T {..}` or
/// `T::new(..)` too). Any other statement but the map's `let` and its
/// return makes the crate unreadable by hook.
#[allow(dead_code)]
fn registered(f: &syn::ItemFn) -> Result<Vec<(String, String)>, String> {
    use syn::{Expr, Lit, Stmt};
    fn lit(e: &Expr) -> Option<String> {
        match e {
            Expr::Lit(l) => match &l.lit {
                Lit::Str(s) => Some(s.value()),
                _ => None,
            },
            Expr::MethodCall(m) if m.args.is_empty() => lit(&m.receiver),
            Expr::Call(c) if c.args.len() == 1 => lit(&c.args[0]),
            _ => None,
        }
    }
    fn ty(e: &Expr) -> Option<String> {
        match e {
            Expr::Cast(c) => ty(&c.expr),
            Expr::Paren(p) => ty(&p.expr),
            Expr::Call(c) => {
                let Expr::Path(p) = &*c.func else { return None };
                let segs: Vec<String> = p
                    .path
                    .segments
                    .iter()
                    .map(|s| s.ident.to_string())
                    .collect();
                if segs.len() >= 2 && segs[segs.len() - 2] == "Box" && segs[segs.len() - 1] == "new"
                {
                    c.args.first().and_then(ty)
                } else if segs.len() >= 2 {
                    // `T::new(..)`.
                    Some(segs[segs.len() - 2].clone())
                } else {
                    None
                }
            }
            Expr::Path(p) => p.path.segments.last().map(|s| s.ident.to_string()),
            Expr::Struct(s) => s.path.segments.last().map(|s| s.ident.to_string()),
            _ => None,
        }
    }
    let mut out = Vec::new();
    let n = f.block.stmts.len();
    for (i, s) in f.block.stmts.iter().enumerate() {
        match s {
            Stmt::Local(_) if i == 0 => {}
            Stmt::Expr(Expr::Path(_), None) if i + 1 == n => {}
            Stmt::Expr(Expr::MethodCall(m), _) if m.method == "insert" && m.args.len() == 2 => {
                match (lit(&m.args[0]), ty(&m.args[1])) {
                    (Some(name), Some(t)) => out.push((name, t)),
                    _ => return Err(format!("exts(): can't read `{}`", tokens(m))),
                }
            }
            other => return Err(format!("exts(): can't read `{}`", tokens(other))),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crate_of(files: &[(&str, &str)]) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
        for (f, t) in files {
            let p = d.path().join("src").join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, t).unwrap();
        }
        d
    }

    const LIB: &str = "mod a;\nmod b;\nuse gp_sim::{Ext, Exts};\n\
        pub fn exts() -> Exts {\n    let mut out = Exts::new();\n\
        out.insert(\"h_a\".into(), Box::new(a::A) as Box<dyn Ext>);\n\
        out.insert(\"h_b\".into(), Box::new(b::B));\n    out\n}\n\
        pub(crate) fn shared_helper() -> i64 { 1 }\n";

    fn hooks(a: &str, b: &str, lib: &str) -> std::collections::BTreeMap<String, String> {
        let d = crate_of(&[("lib.rs", lib), ("a.rs", a), ("b.rs", b)]);
        hook_hashes(d.path()).unwrap()
    }

    /// A change to code only one hook reaches moves that hook's hash
    /// alone; one to code both reach, or to what they share, moves both;
    /// a test or a comment moves neither.
    #[test]
    fn a_change_moves_the_hooks_that_reach_it() {
        let a = "pub struct A;\nimpl gp_sim::Ext for A { fn step(&self, cx: &mut Cx) { own_a(cx); } }\n\
                 fn own_a(cx: &mut Cx) { let _ = 1; }\n";
        let b = "pub struct B;\nimpl gp_sim::Ext for B { fn step(&self, cx: &mut Cx) { let _ = crate::shared_helper(); } }\n\
                 #[cfg(test)]\nmod tests { #[test] fn t() {} }\n";
        let base = hooks(a, b, LIB);
        assert_eq!(base.keys().collect::<Vec<_>>(), ["h_a", "h_b"]);
        let changed = |x: &std::collections::BTreeMap<String, String>| -> Vec<String> {
            base.iter()
                .filter(|(k, v)| x[*k] != **v)
                .map(|(k, _)| k.clone())
                .collect()
        };
        // A's own helper.
        assert_eq!(
            changed(&hooks(&a.replace("let _ = 1", "let _ = 2"), b, LIB)),
            ["h_a"]
        );
        // The helper B calls, in lib.rs.
        assert_eq!(
            changed(&hooks(a, b, &LIB.replace("{ 1 }", "{ 2 }"))),
            ["h_b"]
        );
        // A `use` written another way changes nothing.
        assert!(
            changed(&hooks(
                a,
                b,
                &LIB.replace("use gp_sim::{Ext, Exts};", "use gp_sim::{Exts, Ext};")
            ))
            .is_empty()
        );
        // A name a module imports from elsewhere moves the hook that uses
        // it, and A's own `own_a` isn't B's `pick`.
        let b3 = format!(
            "use crate::shared_helper as pick;\n{}",
            b.replace("crate::shared_helper()", "pick()")
        );
        let base3 = hooks(a, &b3, LIB);
        let moved = hooks(
            a,
            &b3.replace(
                "use crate::shared_helper as pick",
                "use crate::a::own_a as pick",
            ),
            LIB,
        );
        assert_eq!(base3["h_a"], moved["h_a"]);
        assert_ne!(base3["h_b"], moved["h_b"]);
        // B's test, and a comment.
        assert!(
            changed(&hooks(
                a,
                &b.replace("fn t() {}", "fn t() { let _ = 3; }"),
                LIB
            ))
            .is_empty()
        );
        assert!(changed(&hooks(&format!("// note\n{a}"), b, LIB)).is_empty());
        // A helper of A that B starts to call reaches B too.
        let b2 = b.replace("crate::shared_helper()", "crate::a::own_a(cx)");
        let base2 = hooks(a, &b2, LIB);
        let moved = hooks(&a.replace("let _ = 1", "let _ = 2"), &b2, LIB);
        assert!(base2.iter().all(|(k, v)| moved[k] != *v));
    }

    /// What can't be read by hook makes the crate unreadable, so every hook
    /// is the whole crate.
    #[test]
    fn a_crate_it_cannot_follow_is_an_error() {
        let a = "pub struct A;\n";
        let b = "pub struct B;\n";
        let d = crate_of(&[
            (
                "lib.rs",
                &LIB.replace("out.insert(\"h_b\"", "if true { }\n out.insert(\"h_b\""),
            ),
            ("a.rs", a),
            ("b.rs", b),
        ]);
        assert!(hook_hashes(d.path()).is_err());
        let d = crate_of(&[
            (
                "lib.rs",
                &LIB.replace("mod a;", "#[path = \"z.rs\"] mod a;"),
            ),
            ("a.rs", a),
            ("b.rs", b),
        ]);
        assert!(hook_hashes(d.path()).is_err());
        let d = crate_of(&[("lib.rs", "pub fn exts( {"), ("a.rs", a), ("b.rs", b)]);
        assert!(hook_hashes(d.path()).is_err());
    }

    /// Prints which hooks of the hook crates in `GP_HOOK_CRATES` (folders,
    /// `:`-separated, in pairs: before and after) differ.
    #[test]
    #[ignore]
    fn hooks_that_differ_between_two_crates() {
        let v = std::env::var("GP_HOOK_CRATES").unwrap();
        let dirs: Vec<&str> = v.split(':').collect();
        for p in dirs.chunks(2) {
            let (a, b) = (
                hook_hashes(std::path::Path::new(p[0])),
                hook_hashes(std::path::Path::new(p[1])),
            );
            match (a, b) {
                (Ok(a), Ok(b)) => {
                    let d: Vec<&String> = b.keys().filter(|k| a.get(*k) != b.get(*k)).collect();
                    println!(
                        "{} -> {}: {} of {} hooks differ: {:?}",
                        p[0],
                        p[1],
                        d.len(),
                        b.len(),
                        d
                    );
                    let (ca, cb) = (
                        closures(std::path::Path::new(p[0])).unwrap(),
                        closures(std::path::Path::new(p[1])).unwrap(),
                    );
                    let sa: std::collections::BTreeSet<&String> = ca.shared.iter().collect();
                    for x in cb.shared.iter().filter(|x| !sa.contains(x)) {
                        println!("  shared changed: {}", &x[..x.len().min(100)]);
                    }
                    // Why each hook named in GP_HOOK_WANT reaches the changed units.
                    let ta0: std::collections::BTreeSet<&str> =
                        ca.units.iter().map(|u| u.text.as_str()).collect();
                    for h in std::env::var("GP_HOOK_WANT")
                        .unwrap_or_default()
                        .split(',')
                        .filter(|w| !w.is_empty())
                    {
                        let Some(how) = cb.via.get(h) else { continue };
                        for (&i, _) in how
                            .iter()
                            .filter(|(i, _)| !ta0.contains(cb.units[**i].text.as_str()))
                        {
                            let mut path = Vec::new();
                            let mut at = Some(i);
                            while let Some(u) = at {
                                let (f, n) = &how[&u];
                                path.push(format!(
                                    "{}:{:?}/{} (by {n})",
                                    cb.units[u].at,
                                    cb.units[u].owner,
                                    cb.units[u].names.join(",")
                                ));
                                at = *f;
                            }
                            println!("  {h} reaches a changed unit: {}", path.join(" <- "));
                        }
                    }
                    let ta: std::collections::BTreeSet<&str> =
                        ca.units.iter().map(|u| u.text.as_str()).collect();
                    let _ = &ta;
                    for (h, (_, seen)) in &cb.hooks {
                        let ch: Vec<String> = seen
                            .iter()
                            .filter(|&&i| !ta.contains(cb.units[i].text.as_str()))
                            .map(|&i| {
                                format!(
                                    "{}:{:?}/{:?}",
                                    cb.units[i].text.split(' ').next().unwrap_or(""),
                                    cb.units[i].owner,
                                    cb.units[i].names
                                )
                            })
                            .collect();
                        println!("  {h}: {} units, changed {:?}", seen.len(), ch);
                    }
                }
                (a, b) => println!("{} -> {}: {:?} {:?}", p[0], p[1], a.err(), b.err()),
            }
        }
    }
}
