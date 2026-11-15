//! A folder registry's whole round trip, with real `ssh-keygen` keys, and
//! each way a reader must refuse it.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::*;

const NOW: &str = "2026-11-08T12:00:00Z";
const LATER: &str = "2026-11-15T12:00:00Z";
const AFTER: &str = "2026-11-16T12:00:00Z";

struct Key {
    path: PathBuf,
    public: String,
}

fn key(dir: &Path, name: &str) -> Key {
    let path = dir.join(name);
    let ok = Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-C", name, "-f"])
        .arg(&path)
        .status()
        .unwrap()
        .success();
    assert!(ok);
    let public = std::fs::read_to_string(path.with_extension("pub"))
        .unwrap()
        .trim()
        .to_owned();
    Key { path, public }
}

fn ssh_sign(key: &Path, data: &[u8]) -> Result<Vec<u8>, String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let f = dir.path().join("d");
    std::fs::write(&f, data).map_err(|e| e.to_string())?;
    let o = Command::new("ssh-keygen")
        .args(["-q", "-Y", "sign", "-n", NAMESPACE, "-f"])
        .arg(key)
        .arg(&f)
        .output()
        .map_err(|e| e.to_string())?;
    if !o.status.success() {
        return Err(String::from_utf8_lossy(&o.stderr).into_owned());
    }
    std::fs::read(f.with_extension("sig")).map_err(|e| e.to_string())
}

/// `f` with a signer made from `k`.
fn with<T>(k: &Key, f: impl FnOnce(&Signer) -> T) -> T {
    let path = k.path.clone();
    let sign = move |d: &[u8]| ssh_sign(&path, d);
    f(&Signer {
        public: k.public.clone(),
        sign: &sign,
    })
}

struct Fixture {
    _dir: tempfile::TempDir,
    root_dir: PathBuf,
    root: Key,
    ns: Key,
    other: Key,
    id: String,
}

impl Fixture {
    fn target(&self) -> Folder {
        Folder(self.root_dir.clone())
    }
    fn rewrite(&self, key: &str, f: impl FnOnce(String) -> String) {
        let p = self.root_dir.join(key);
        let t = std::fs::read_to_string(&p).unwrap();
        std::fs::write(&p, f(t)).unwrap();
    }
}

fn files() -> BTreeMap<String, Vec<u8>> {
    BTreeMap::from([
        ("model.toml".to_owned(), b"name = \"x\"\n".to_vec()),
        ("themes/a.toml".to_owned(), b"a = 1\n".to_vec()),
    ])
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let k = dir.path().join("keys");
    std::fs::create_dir(&k).unwrap();
    let (root, index, ns, other) = (
        key(&k, "root"),
        key(&k, "index"),
        key(&k, "ns"),
        key(&k, "other"),
    );
    let root_dir = dir.path().join("reg");
    std::fs::create_dir(&root_dir).unwrap();
    let t = Folder(root_dir.clone());
    with(&root, |r| {
        init(
            &t,
            "gp-test",
            r,
            std::slice::from_ref(&index.public),
            &[("lex00".to_owned(), vec![ns.public.clone()])],
            "2026-11-08",
        )
    })
    .unwrap();
    let id = with(&ns, |s| {
        publish(
            &t,
            &Publish {
                name: "lex00/game",
                version: "1.0.0",
                kind: "game",
                title: "Game",
                for_games: &[],
                published: NOW,
            },
            &files(),
            s,
        )
    })
    .unwrap();
    with(&ns, |s| {
        publish(
            &t,
            &Publish {
                name: "lex00/night",
                version: "0.1.0",
                kind: "content",
                title: "Night",
                for_games: &["lex00/game".to_owned()],
                published: NOW,
            },
            &files(),
            s,
        )
    })
    .unwrap();
    let versions = [
        ("lex00/game".to_owned(), "1.0.0".to_owned()),
        ("lex00/night".to_owned(), "0.1.0".to_owned()),
    ];
    with(&index, |s| reindex(&t, &versions, s, NOW, LATER)).unwrap();
    Fixture {
        _dir: dir,
        root_dir,
        root,
        ns,
        other,
        id,
    }
}

#[test]
fn a_package_goes_round_and_the_lists_name_it() {
    let f = fixture();
    let t = f.target();
    let r = Reader::open(&t, &f.root.public).unwrap();
    let (m, got) = r.fetch("lex00/game", "1.0.0", &f.id).unwrap();
    assert_eq!(m.id(), f.id);
    assert_eq!(got, files());
    let c = r.catalog(NOW).unwrap();
    let names: Vec<&str> = c.package.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["lex00/game", "lex00/night"]);
    let g = r.by_game("lex00/game", NOW).unwrap();
    assert_eq!(g.line[0].package, "lex00/night");
    assert!(r.yanks(NOW).unwrap().yank.is_empty());
}

#[test]
fn a_wrong_root_is_refused() {
    let f = fixture();
    let t = f.target();
    let e = Reader::open(&t, &f.other.public).err().unwrap();
    assert!(e.contains("different key"), "{e}");
}

#[test]
fn an_edited_registry_file_is_refused() {
    let f = fixture();
    f.rewrite("registry.toml", |t| t.replace("gp-test", "gp-evil"));
    let t = f.target();
    let e = Reader::open(&t, &f.root.public).err().unwrap();
    assert!(e.contains("doesn't match"), "{e}");
}

#[test]
fn a_version_signed_by_a_key_not_in_the_namespace_is_refused() {
    let f = fixture();
    let t = f.target();
    let key = "index/lex00/game/1.0.0.toml";
    let bytes = std::fs::read(f.root_dir.join(key)).unwrap();
    let forged = ssh_sign(&f.other.path, &bytes).unwrap();
    std::fs::write(f.root_dir.join(format!("{key}.sig")), forged).unwrap();
    let r = Reader::open(&t, &f.root.public).unwrap();
    let e = r.fetch("lex00/game", "1.0.0", &f.id).err().unwrap();
    assert!(e.contains("different key"), "{e}");
}

#[test]
fn an_id_the_version_doesnt_name_is_refused() {
    let f = fixture();
    let t = f.target();
    let r = Reader::open(&t, &f.root.public).unwrap();
    let wrong = crate::hash(b"other");
    let e = r.fetch("lex00/game", "1.0.0", &wrong).err().unwrap();
    assert!(e.contains("not the id asked for"), "{e}");
}

#[test]
fn a_changed_blob_is_refused() {
    let f = fixture();
    let t = f.target();
    let hash = crate::hash(b"a = 1\n");
    std::fs::write(f.root_dir.join(blob_key(&hash)), b"a = 2\n").unwrap();
    let r = Reader::open(&t, &f.root.public).unwrap();
    assert!(r.fetch("lex00/game", "1.0.0", &f.id).is_err());
}

#[test]
fn a_changed_manifest_is_refused() {
    let f = fixture();
    let t = f.target();
    f.rewrite(&package_key(&f.id), |t| format!("{t}\n"));
    let r = Reader::open(&t, &f.root.public).unwrap();
    assert!(r.fetch("lex00/game", "1.0.0", &f.id).is_err());
}

#[test]
fn an_expired_list_is_refused_and_a_package_still_fetches() {
    let f = fixture();
    let t = f.target();
    let r = Reader::open(&t, &f.root.public).unwrap();
    let e = r.catalog(AFTER).err().unwrap();
    assert!(e.contains("expired"), "{e}");
    assert!(r.by_game("lex00/game", AFTER).is_err());
    assert!(r.yanks(AFTER).is_err());
    assert!(r.fetch("lex00/game", "1.0.0", &f.id).is_ok());
}

#[test]
fn an_edited_expiry_is_refused() {
    let f = fixture();
    f.rewrite("catalog.toml", |t| t.replace(LATER, "2030-01-01T00:00:00Z"));
    let t = f.target();
    let r = Reader::open(&t, &f.root.public).unwrap();
    assert!(r.catalog(AFTER).unwrap_err().contains("doesn't match"));
}

#[test]
fn a_version_is_published_once() {
    let f = fixture();
    let t = f.target();
    let e = with(&f.ns, |s| {
        publish(
            &t,
            &Publish {
                name: "lex00/game",
                version: "1.0.0",
                kind: "game",
                title: "Game",
                for_games: &[],
                published: NOW,
            },
            &files(),
            s,
        )
    })
    .unwrap_err();
    assert!(e.contains("already published"), "{e}");
}

#[test]
fn an_attestation_is_written_once_signed_and_listed() {
    let f = fixture();
    let t = f.target();
    let text = "x = 1\n";
    let key = with(&f.ns, |s| attest(&t, &f.id, text, s)).unwrap();
    assert!(key.starts_with(&format!("attest/{}/", f.id)));
    let body = t.read(&key).unwrap().unwrap();
    assert_eq!(body, text.as_bytes());
    let sig = t.read(&format!("{key}.sig")).unwrap().unwrap();
    crate::sig::verify(&f.ns.public, NAMESPACE, &sig, &body).unwrap();
    let hash = key.rsplit('/').next().unwrap().trim_end_matches(".toml");
    let list = t.read(&format!("attest/{}/list", f.id)).unwrap().unwrap();
    assert_eq!(String::from_utf8(list).unwrap(), format!("{hash}\n"));
    // The same record can't be written twice; a package not here can't be attested.
    assert!(with(&f.ns, |s| attest(&t, &f.id, text, s)).is_err());
    assert!(with(&f.ns, |s| attest(&t, "missing", "y = 1\n", s)).is_err());
}

/// A registry made with no key at all.
fn unsigned_fixture() -> (tempfile::TempDir, String) {
    let d = tempfile::tempdir().unwrap();
    let t = Folder(d.path().to_owned());
    let ns = [("lex00".to_owned(), Vec::new())];
    let none = Signer::none();
    init(&t, "open", &none, &[], &ns, "2026-11-08").unwrap();
    let id = publish(
        &t,
        &Publish {
            name: "lex00/game",
            version: "1.0.0",
            kind: "game",
            title: "Game",
            for_games: &[],
            published: NOW,
        },
        &files(),
        &none,
    )
    .unwrap();
    let versions = [("lex00/game".to_owned(), "1.0.0".to_owned())];
    reindex(&t, &versions, &none, NOW, LATER).unwrap();
    (d, id)
}

#[test]
fn a_registry_with_no_keys_is_read_with_no_signatures_and_no_expiry() {
    let (d, id) = unsigned_fixture();
    let t = Folder(d.path().to_owned());
    let has_sig = walk(d.path()).iter().any(|p| p.ends_with(".sig"));
    assert!(!has_sig, "no signature file is written");
    let r = Reader::unsigned(&t).unwrap();
    let (_, got) = r.fetch("lex00/game", "1.0.0", &id).unwrap();
    assert_eq!(got, files());
    assert_eq!(r.catalog(AFTER).unwrap().package.len(), 1, "long expired");
    assert!(r.yanks(AFTER).unwrap().yank.is_empty());
}

#[test]
fn hashes_are_still_checked_with_no_signatures() {
    let (d, id) = unsigned_fixture();
    let t = Folder(d.path().to_owned());
    let r = Reader::unsigned(&t).unwrap();
    let (m, _) = r.fetch("lex00/game", "1.0.0", &id).unwrap();
    let blob = d.path().join(blob_key(&m.files[0].hash));
    let mut bytes = std::fs::read(&blob).unwrap();
    bytes[0] ^= 1;
    std::fs::write(&blob, bytes).unwrap();
    assert!(r.fetch("lex00/game", "1.0.0", &id).is_err());
}

#[test]
fn a_changed_manifest_is_refused_with_no_signatures() {
    let (d, id) = unsigned_fixture();
    let t = Folder(d.path().to_owned());
    let r = Reader::unsigned(&t).unwrap();
    let at = d.path().join(package_key(&id));
    let text = std::fs::read_to_string(&at).unwrap();
    std::fs::write(&at, text + "\n").unwrap();
    assert!(r.manifest("lex00/game", "1.0.0", &id).is_err());
}

#[test]
fn a_reader_with_a_root_refuses_a_registry_with_no_signatures() {
    let f = fixture();
    let (d, _) = unsigned_fixture();
    let t = Folder(d.path().to_owned());
    assert!(Reader::open(&t, &f.root.public).is_err());
}

fn walk(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else {
            out.push(p.display().to_string());
        }
    }
    out
}

#[test]
fn a_namespace_is_granted_as_init_writes_it_and_only_once() {
    let (d, _) = unsigned_fixture();
    let t = Folder(d.path().to_owned());
    let none = Signer::none();
    grant(&t, "friend", &[], &none, "2026-11-14").unwrap();
    // The same two files init writes for a registry made with both names.
    let made = tempfile::tempdir().unwrap();
    let m = Folder(made.path().to_owned());
    let ns = [
        ("lex00".to_owned(), Vec::new()),
        ("friend".to_owned(), Vec::new()),
    ];
    init(&m, "open", &none, &[], &ns, "2026-11-14").unwrap();
    for key in ["registry.toml", "namespaces/friend.toml"] {
        assert_eq!(t.read(key).unwrap(), m.read(key).unwrap(), "{key}");
    }
    let text = String::from_utf8(t.read("namespaces/friend.toml").unwrap().unwrap()).unwrap();
    assert_eq!(
        text,
        "format = \"1\"\nnamespace = \"friend\"\nstatus = \"granted\"\nkeys = []\n"
    );
    let before = t.read("registry.toml").unwrap();
    let e = grant(&t, "friend", &[], &none, "2026-11-14").unwrap_err();
    assert!(e.contains("already granted"), "{e}");
    let e = grant(&t, "lex00", &[], &none, "2026-11-14").unwrap_err();
    assert!(e.contains("already granted"), "{e}");
    assert_eq!(t.read("registry.toml").unwrap(), before);
    assert!(grant(&t, "Not/Ok", &[], &none, "2026-11-14").is_err());
    let names: Vec<String> = grants(&t).unwrap().into_iter().map(|(n, _)| n).collect();
    assert_eq!(names, ["lex00", "friend"]);
    // A reader now accepts a package in the namespace.
    let r = Reader::unsigned(&t).unwrap();
    assert_eq!(r.namespace("friend").unwrap().status, "granted");
}

#[test]
fn a_signed_registry_grants_only_with_its_root_and_a_key() {
    let f = fixture();
    let t = f.target();
    let none = Signer::none();
    let e = grant(
        &t,
        "friend",
        std::slice::from_ref(&f.other.public),
        &none,
        "2026-11-14",
    )
    .unwrap_err();
    assert!(e.contains("--root-key"), "{e}");
    let e = with(&f.other, |s| {
        grant(
            &t,
            "friend",
            std::slice::from_ref(&f.other.public),
            s,
            "2026-11-14",
        )
    })
    .unwrap_err();
    assert!(e.contains("isn't the one"), "{e}");
    let e = with(&f.root, |s| grant(&t, "friend", &[], s, "2026-11-14")).unwrap_err();
    assert!(e.contains("--key"), "{e}");
    with(&f.root, |s| {
        grant(
            &t,
            "friend",
            std::slice::from_ref(&f.other.public),
            s,
            "2026-11-14",
        )
    })
    .unwrap();
    let r = Reader::open(&t, &f.root.public).unwrap();
    assert_eq!(r.file.namespaces, ["lex00", "friend"]);
    assert_eq!(r.namespace("friend").unwrap().keys[0].key, f.other.public);
}

#[test]
fn a_yank_is_added_to_yanks_toml_once_and_can_be_taken_back() {
    let (d, id) = unsigned_fixture();
    let t = Folder(d.path().to_owned());
    let none = Signer::none();
    let y = yank(
        &t,
        "lex00/game",
        "1.0.0",
        "made up for a test",
        "security",
        &none,
        NOW,
        LATER,
    )
    .unwrap();
    assert_eq!(y.id, id);
    let text = String::from_utf8(t.read("yanks.toml").unwrap().unwrap()).unwrap();
    assert_eq!(
        text,
        format!(
            "format = \"1\"\nregistry = \"open\"\nsigned = \"{NOW}\"\nexpires = \"{LATER}\"\n\n[[yank]]\nname = \"lex00/game\"\nversion = \"1.0.0\"\nid = \"{id}\"\nreason = \"made up for a test\"\nseverity = \"security\"\n"
        )
    );
    let r = Reader::unsigned(&t).unwrap();
    assert_eq!(r.yanks(NOW).unwrap().yank, std::slice::from_ref(&y));
    let e = yank(
        &t,
        "lex00/game",
        "1.0.0",
        "again",
        "notice",
        &none,
        NOW,
        LATER,
    )
    .unwrap_err();
    assert!(e.contains("already yanked"), "{e}");
    let e = yank(&t, "lex00/game", "9.9.9", "r", "notice", &none, NOW, LATER).unwrap_err();
    assert!(e.contains("isn't published"), "{e}");
    // A reindex keeps what yanks.toml lists.
    let versions = [("lex00/game".to_owned(), "1.0.0".to_owned())];
    reindex(&t, &versions, &none, NOW, LATER).unwrap();
    assert_eq!(r.yanks(NOW).unwrap().yank, std::slice::from_ref(&y));
    assert_eq!(
        unyank(&t, "lex00/game", "1.0.0", &none, NOW, LATER).unwrap(),
        y
    );
    let text = String::from_utf8(t.read("yanks.toml").unwrap().unwrap()).unwrap();
    assert!(text.ends_with("yank = []\n"), "{text}");
    assert!(unyank(&t, "lex00/game", "1.0.0", &none, NOW, LATER).is_err());
}

#[test]
fn a_signed_registry_yanks_only_with_a_key() {
    let f = fixture();
    let t = f.target();
    let e = yank(
        &t,
        "lex00/game",
        "1.0.0",
        "r",
        "security",
        &Signer::none(),
        NOW,
        LATER,
    )
    .unwrap_err();
    assert!(e.contains("--key"), "{e}");
}

#[test]
fn listed_names_what_the_lists_hold_and_would_list_agrees_with_reindex() {
    let (d, _) = unsigned_fixture();
    let t = Folder(d.path().to_owned());
    let none = Signer::none();
    let games = ["lex00/game".to_owned()];
    for v in ["0.1.0", "0.2.0"] {
        let p = Publish {
            name: "lex00/night",
            version: v,
            kind: "content",
            title: "Night",
            for_games: &games,
            published: NOW,
        };
        publish(&t, &p, &files(), &none).unwrap();
    }
    let versions = [
        ("lex00/game".to_owned(), "1.0.0".to_owned()),
        ("lex00/night".to_owned(), "0.1.0".to_owned()),
        ("lex00/night".to_owned(), "0.2.0".to_owned()),
    ];
    let would = would_list(&t, &versions).unwrap();
    reindex(&t, &versions, &none, NOW, LATER).unwrap();
    let now = listed(&t).unwrap();
    assert_eq!(now, would);
    let pairs: Vec<(String, bool)> = now
        .iter()
        .map(|l| (format!("{}@{}", l.name, l.version), l.by_game))
        .collect();
    assert_eq!(
        pairs,
        [
            ("lex00/night@0.1.0".to_owned(), true),
            ("lex00/night@0.2.0".to_owned(), true),
            ("lex00/game@1.0.0".to_owned(), false),
        ]
    );
    // Reindexing what is listed rewrites the same lists.
    let again: Vec<(String, String)> = now
        .iter()
        .map(|l| (l.name.clone(), l.version.clone()))
        .collect();
    let before = t.read("by-game/lex00/game.toml").unwrap();
    reindex(&t, &again, &none, NOW, LATER).unwrap();
    assert_eq!(t.read("by-game/lex00/game.toml").unwrap(), before);
}

/// The by-game line of the made-up content package, as a reader reads it.
fn night_line(t: &Folder) -> ByGameLine {
    let r = Reader::unsigned(t).unwrap();
    r.by_game("lex00/game", NOW).unwrap().line.remove(0)
}

fn unsigned_with_content() -> tempfile::TempDir {
    let (d, _) = unsigned_fixture();
    let t = Folder(d.path().to_owned());
    let none = Signer::none();
    publish(
        &t,
        &Publish {
            name: "lex00/night",
            version: "0.1.0",
            kind: "content",
            title: "Night",
            for_games: &["lex00/game".to_owned()],
            published: NOW,
        },
        &files(),
        &none,
    )
    .unwrap();
    let versions = [
        ("lex00/game".to_owned(), "1.0.0".to_owned()),
        ("lex00/night".to_owned(), "0.1.0".to_owned()),
    ];
    reindex(&t, &versions, &none, NOW, LATER).unwrap();
    d
}

#[test]
fn a_recheck_is_recorded_in_the_by_game_line_and_a_reindex_keeps_it() {
    let d = unsigned_with_content();
    let t = Folder(d.path().to_owned());
    let none = Signer::none();
    assert_eq!(night_line(&t).recheck, None);
    let text = String::from_utf8(t.read("by-game/lex00/game.toml").unwrap().unwrap()).unwrap();
    assert!(!text.contains("recheck"), "no key until a recheck: {text}");
    let failed = Recheck {
        date: "2026-11-14".into(),
        result: "fail".into(),
        reason: "made up: its pictures don't fit".into(),
    };
    let l = record_recheck(
        &t,
        "lex00/game",
        "lex00/night",
        "0.1.0",
        failed.clone(),
        &none,
        NOW,
        LATER,
    )
    .unwrap();
    assert_eq!(l.recheck.as_ref(), Some(&failed));
    assert!(failed.failed());
    let text = String::from_utf8(t.read("by-game/lex00/game.toml").unwrap().unwrap()).unwrap();
    assert!(
        text.contains(
            "[line.recheck]\ndate = \"2026-11-14\"\nresult = \"fail\"\nreason = \"made up: its pictures don't fit\"\n"
        ),
        "{text}"
    );
    assert_eq!(night_line(&t).recheck, Some(failed.clone()));
    // A reindex of the same versions keeps it.
    let versions = [
        ("lex00/game".to_owned(), "1.0.0".to_owned()),
        ("lex00/night".to_owned(), "0.1.0".to_owned()),
    ];
    reindex(&t, &versions, &none, NOW, LATER).unwrap();
    assert_eq!(night_line(&t).recheck, Some(failed));
    // A later pass replaces it.
    let passed = Recheck {
        date: "2026-11-15".into(),
        result: "pass".into(),
        reason: "made up".into(),
    };
    record_recheck(
        &t,
        "lex00/game",
        "lex00/night",
        "0.1.0",
        passed.clone(),
        &none,
        NOW,
        LATER,
    )
    .unwrap();
    assert_eq!(night_line(&t).recheck, Some(passed.clone()));
    // Only pass or fail, and only a version the list names.
    let odd = Recheck {
        result: "maybe".into(),
        ..passed.clone()
    };
    let e = record_recheck(
        &t,
        "lex00/game",
        "lex00/night",
        "0.1.0",
        odd,
        &none,
        NOW,
        LATER,
    )
    .unwrap_err();
    assert!(e.contains("pass or fail"), "{e}");
    let e = record_recheck(
        &t,
        "lex00/game",
        "lex00/night",
        "9.0.0",
        passed,
        &none,
        NOW,
        LATER,
    )
    .unwrap_err();
    assert!(e.contains("doesn't list lex00/night@9.0.0"), "{e}");
}

#[test]
fn a_recheck_on_a_signed_registry_needs_the_signing_key_and_is_signed() {
    let f = fixture();
    let t = f.target();
    let failed = Recheck {
        date: "2026-11-14".into(),
        result: "fail".into(),
        reason: "made up".into(),
    };
    let e = record_recheck(
        &t,
        "lex00/game",
        "lex00/night",
        "0.1.0",
        failed.clone(),
        &Signer::none(),
        NOW,
        LATER,
    )
    .unwrap_err();
    assert!(e.contains("--key"), "{e}");
    // Signed by another key than the registry's signing key: the reader
    // with the pinned root refuses the file, so the field is covered.
    with(&f.other, |s| {
        record_recheck(
            &t,
            "lex00/game",
            "lex00/night",
            "0.1.0",
            failed.clone(),
            s,
            NOW,
            LATER,
        )
    })
    .unwrap();
    let r = Reader::open(&t, &f.root.public).unwrap();
    assert!(r.by_game("lex00/game", NOW).is_err());
}

/// A reader from before the key (a by-game line with only the four keys it
/// knew) reads a line that has a recheck, and today's reader passes over a
/// key it doesn't know.
#[test]
fn an_older_reader_passes_over_the_recheck() {
    #[derive(Deserialize)]
    #[allow(dead_code)]
    struct OldLine {
        package: String,
        version: String,
        id: String,
        title: String,
    }
    #[derive(Deserialize)]
    #[allow(dead_code)]
    struct OldByGame {
        format: String,
        registry: String,
        signed: String,
        expires: String,
        line: Vec<OldLine>,
    }
    let text = "format = \"1\"\nregistry = \"r\"\nsigned = \"a\"\nexpires = \"b\"\n\n[[line]]\npackage = \"x/y\"\nversion = \"1\"\nid = \"i\"\ntitle = \"Y\"\n\n[line.recheck]\ndate = \"2026-11-14\"\nresult = \"fail\"\nreason = \"r\"\nlater = 1\n";
    let old: OldByGame = toml::from_str(text).unwrap();
    assert_eq!(old.line[0].package, "x/y");
    let now: ByGame = toml::from_str(&format!("{text}\n[[line]]\npackage = \"x/z\"\nversion = \"1\"\nid = \"j\"\ntitle = \"Z\"\nfrom_a_newer_gp = true\n")).unwrap();
    assert_eq!(now.line[0].recheck.as_ref().unwrap().result, "fail");
    assert_eq!(now.line[1].recheck, None);
}
