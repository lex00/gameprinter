//! The hook crates the tool's own builds run, by model. It holds only the
//! demo hook: a game's hook crate lives with the game, and the page's build
//! is given one as a build input in phase 2.1.

mod demo;
#[cfg(test)]
mod hashing;

use gp_sim::Exts;

include!(concat!(env!("OUT_DIR"), "/hashes.rs"));

/// One model's hooks: the model's `model.name`, the key of its crate in
/// the build's hash table (the template directory's name), and the crate's
/// registry.
pub struct Entry {
    pub model: &'static str,
    pub key: &'static str,
    pub exts: fn() -> Exts,
}

/// Every model with hooks.
pub const ENTRIES: &[Entry] = &[Entry {
    model: "hooks_demo",
    key: "demo",
    exts: demo::exts,
}];

/// A model's hooks and the label recordings store for them.
pub struct Hooks {
    pub exts: Exts,
    /// The crate's path, its version and the first 12 hex digits of its
    /// code's hash, such as `templates/smb1-ntsc-prg0/ext 0.1.0 1f2e3d4c5b6a`.
    pub label: String,
}

/// The hooks registered for a model, by its `model.name`.
pub fn for_model(model: &str) -> Option<Hooks> {
    let e = ENTRIES.iter().find(|e| e.model == model)?;
    let label = CRATES
        .iter()
        .find(|c| c.0 == e.key)
        .map(|(_, path, version, hash)| format!("{path} {version} {}", &hash[..12]))
        .unwrap_or_else(|| {
            panic!(
                "gp-exts: model `{}` has no hook crate `{}` hashed (crates/gp-exts/src/lib.rs)",
                e.model, e.key
            )
        });
    Some(Hooks {
        exts: (e.exts)(),
        label,
    })
}

/// A model's hooks and their label, or no hooks and an empty label for a
/// model with no hook crate: what the interpreter in gp runs and what its
/// recordings store.
pub fn registry(model: &str) -> (Exts, String) {
    match for_model(model) {
        Some(h) => (h.exts, h.label),
        None => (Exts::new(), String::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_demo_hooks_are_registered_with_a_label() {
        let h = for_model("hooks_demo").unwrap();
        assert!(h.exts.contains_key("demo_turn"));
        assert!(h.label.starts_with("gp-exts demo "), "{}", h.label);
        assert_eq!(h.label.split(' ').next_back().unwrap().len(), 12);
        assert!(for_model("mosshop").is_none());
    }

    /// Every row names a crate build.rs hashed, and every hook crate under
    /// templates/ has a row.
    #[test]
    fn rows_and_hook_crates_match() {
        for e in ENTRIES {
            assert!(
                CRATES.iter().any(|c| c.0 == e.key),
                "{} has no crate",
                e.key
            );
        }
        for c in CRATES {
            assert!(ENTRIES.iter().any(|e| e.key == c.0), "{} has no row", c.1);
        }
        // A template's row names the model its model.toml names, or the
        // model of a template derived from it (template.toml's [derive]).
        let templates = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates");
        let name_of = |dir: &std::path::Path| {
            let text = std::fs::read_to_string(dir.join("model.toml")).unwrap();
            text.lines()
                .find_map(|l| l.strip_prefix("name = "))
                .map(|v| v.trim_matches('"').to_owned())
                .unwrap()
        };
        for e in ENTRIES.iter().filter(|e| e.key != "demo") {
            let own = name_of(&templates.join(e.key));
            let derived = std::fs::read_dir(&templates).unwrap().flatten().any(|d| {
                let manifest =
                    std::fs::read_to_string(d.path().join("template.toml")).unwrap_or_default();
                manifest.contains(&format!("from = \"../{}\"", e.key))
                    && name_of(&d.path()) == e.model
            });
            assert!(
                own == e.model || derived,
                "templates/{}: {} isn't {}",
                e.key,
                own,
                e.model
            );
        }
    }

    #[test]
    fn the_hash_covers_code_and_nothing_else() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_owned();
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"x\"\nversion = \"0.2.0\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("src/lib.rs"), "pub fn a() {}\n").unwrap();
        let hash = || hashing::hook_hash(&dir, &hashing::hook_files(&dir));
        let first = hash();
        std::fs::write(dir.join(".DS_Store"), "junk").unwrap();
        std::fs::write(dir.join("src/.lib.rs.swp"), "junk").unwrap();
        std::fs::write(dir.join("notes.txt"), "junk").unwrap();
        assert_eq!(hash(), first, "untracked files leave the hash alone");
        std::fs::write(dir.join("src/lib.rs"), "pub fn b() {}\n").unwrap();
        assert_ne!(hash(), first, "a code change changes it");
        assert_eq!(
            hashing::package_version(
                "[package]\nversion.workspace = true\n[dependencies.y]\nversion = \"9\"\n",
                "[workspace.package]\nversion = \"0.1.0\"\n"
            ),
            "0.1.0"
        );
    }
}
