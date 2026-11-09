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
