//! A registry: a tree of keys any folder or bucket can serve. Every key is
//! written once except the list files, which the operator rewrites and
//! re-signs. A reader pins only the operator's root key.
//!
//! Exact versions and ids only: there is no resolver and no dependencies.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::sig::{NAMESPACE, verify};
use crate::{Manifest, blob_key, check, package_key, signature_key};

/// The format every registry file carries.
pub const FORMAT: &str = "1";

/// A place a registry's keys live.
pub trait Target {
    /// The key's bytes, or None when there is no such key.
    fn read(&self, key: &str) -> Result<Option<Vec<u8>>, String>;
    /// Writes a key once. False when the key already holds other bytes'
    /// claim: nothing is overwritten.
    fn write_once(&self, key: &str, bytes: &[u8]) -> Result<bool, String>;
    /// Writes a list file, replacing what is there.
    fn replace(&self, key: &str, bytes: &[u8]) -> Result<(), String>;
}

/// A registry in a folder.
pub struct Folder(pub std::path::PathBuf);

impl Target for Folder {
    fn read(&self, key: &str) -> Result<Option<Vec<u8>>, String> {
        match std::fs::read(self.0.join(key)) {
            Ok(b) => Ok(Some(b)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("{key}: {e}")),
        }
    }

    fn write_once(&self, key: &str, bytes: &[u8]) -> Result<bool, String> {
        use std::io::Write as _;
        let path = self.0.join(key);
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p).map_err(|e| format!("{key}: {e}"))?;
        }
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut f) => f
                .write_all(bytes)
                .map(|()| true)
                .map_err(|e| format!("{key}: {e}")),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
            Err(e) => Err(format!("{key}: {e}")),
        }
    }

    fn replace(&self, key: &str, bytes: &[u8]) -> Result<(), String> {
        let path = self.0.join(key);
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p).map_err(|e| format!("{key}: {e}"))?;
        }
        std::fs::write(&path, bytes).map_err(|e| format!("{key}: {e}"))
    }
}

/// How a signer signs bytes.
pub type SignFn = dyn Fn(&[u8]) -> Result<Vec<u8>, String>;

/// Signs bytes as one key: the public key and how to sign with it.
pub struct Signer<'a> {
    pub public: String,
    pub sign: &'a SignFn,
}

/// `registry.toml`, signed by the root.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RegistryFile {
    pub format: String,
    pub name: String,
    pub root: String,
    /// The online keys that sign the list files.
    pub signing: Vec<String>,
    pub namespaces: Vec<String>,
    #[serde(default)]
    pub mirrors: Vec<String>,
}

/// One signer key of a namespace.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NamespaceKey {
    pub key: String,
    pub added: String,
}

/// `namespaces/<ns>.toml`, signed by the root.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NamespaceFile {
    pub format: String,
    pub namespace: String,
    pub status: String,
    pub keys: Vec<NamespaceKey>,
}

/// `index/<ns>/<name>/<version>.toml`, signed by a key of the namespace.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VersionFile {
    pub format: String,
    pub name: String,
    pub version: String,
    pub id: String,
    pub kind: String,
    pub title: String,
    pub size: u64,
    pub published: String,
    pub signer: String,
    /// For content: the games each section is for.
    #[serde(default)]
    pub for_games: Vec<String>,
}

/// One line of `catalog.toml`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CatalogEntry {
    pub name: String,
    pub kind: String,
    pub title: String,
    pub latest: String,
    pub id: String,
    #[serde(default)]
    pub for_games: Vec<String>,
}

/// `catalog.toml`, signed by a signing key, with an expiry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Catalog {
    pub format: String,
    pub registry: String,
    pub signed: String,
    pub expires: String,
    #[serde(default)]
    pub package: Vec<CatalogEntry>,
}

/// One content package offered for a game.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ByGameLine {
    pub package: String,
    pub version: String,
    pub id: String,
    pub title: String,
}

/// `by-game/<ns>/<game>.toml`, signed by a signing key, with an expiry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ByGame {
    pub format: String,
    pub registry: String,
    pub signed: String,
    pub expires: String,
    #[serde(default)]
    pub line: Vec<ByGameLine>,
}

/// One withdrawn version.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Yank {
    pub name: String,
    pub version: String,
    pub id: String,
    pub reason: String,
    pub severity: String,
}

/// `yanks.toml`, signed by a signing key, with an expiry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Yanks {
    pub format: String,
    pub registry: String,
    pub signed: String,
    pub expires: String,
    #[serde(default)]
    pub yank: Vec<Yank>,
}

fn to_text<T: Serialize>(v: &T) -> Result<String, String> {
    toml::to_string(v).map_err(|e| e.to_string())
}

fn sig_key(key: &str) -> String {
    format!("{key}.sig")
}

/// A package name's two parts, refusing anything else.
fn split_name(name: &str) -> Result<(&str, &str), String> {
    let (ns, n) = name
        .split_once('/')
        .ok_or_else(|| format!("{name:?} isn't namespace/name"))?;
    let good = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.')
    };
    if !good(ns) || !good(n) || n.contains('/') {
        return Err(format!("{name:?} isn't namespace/name"));
    }
    Ok((ns, n))
}

fn version_key(name: &str, version: &str) -> Result<String, String> {
    let (ns, n) = split_name(name)?;
    if version.is_empty() || version.contains('/') {
        return Err(format!("{version:?} isn't a version"));
    }
    Ok(format!("index/{ns}/{n}/{version}.toml"))
}

/// Writes a key and its signature, write-once.
fn put_signed(t: &dyn Target, key: &str, bytes: &[u8], s: &Signer) -> Result<(), String> {
    if !t.write_once(key, bytes)? {
        return Err(format!("{key} is already published"));
    }
    let sig = (s.sign)(bytes)?;
    if !t.write_once(&sig_key(key), &sig)? {
        return Err(format!("{} is already published", sig_key(key)));
    }
    Ok(())
}

/// Rewrites a list file and its signature.
fn replace_signed(t: &dyn Target, key: &str, bytes: &[u8], s: &Signer) -> Result<(), String> {
    let sig = (s.sign)(bytes)?;
    t.replace(key, bytes)?;
    t.replace(&sig_key(key), &sig)
}

/// An attestation's key: its text is named by its own hash.
fn attest_key(id: &str, hash: &str) -> String {
    format!("attest/{id}/{hash}.toml")
}

/// The list of a package's attestations.
fn attest_list_key(id: &str) -> String {
    format!("attest/{id}/list")
}

/// Writes one attestation about package `id` with its signature and adds it
/// to the package's list. Refuses a package the registry doesn't have and
/// an attestation already there, so a record can't be replaced.
pub fn attest(t: &dyn Target, id: &str, text: &str, s: &Signer) -> Result<String, String> {
    must_read(t, &package_key(id))?;
    let hash = blake3::hash(text.as_bytes()).to_hex().to_string();
    let key = attest_key(id, &hash);
    put_signed(t, &key, text.as_bytes(), s)?;
    let list = attest_list_key(id);
    let mut lines = match t.read(&list)? {
        Some(b) => String::from_utf8(b).map_err(|e| e.to_string())?,
        None => String::new(),
    };
    lines.push_str(&hash);
    lines.push('\n');
    t.replace(&list, lines.as_bytes())?;
    Ok(key)
}

/// Makes a registry: `registry.toml` and each namespace file, root-signed.
pub fn init(
    t: &dyn Target,
    name: &str,
    root: &Signer,
    signing: &[String],
    namespaces: &[(String, Vec<String>)],
    today: &str,
) -> Result<(), String> {
    let file = RegistryFile {
        format: FORMAT.into(),
        name: name.into(),
        root: root.public.clone(),
        signing: signing.to_vec(),
        namespaces: namespaces.iter().map(|(n, _)| n.clone()).collect(),
        mirrors: Vec::new(),
    };
    put_signed(t, "registry.toml", to_text(&file)?.as_bytes(), root)?;
    for (ns, keys) in namespaces {
        let f = NamespaceFile {
            format: FORMAT.into(),
            namespace: ns.clone(),
            status: "granted".into(),
            keys: keys
                .iter()
                .map(|k| NamespaceKey {
                    key: k.clone(),
                    added: today.into(),
                })
                .collect(),
        };
        put_signed(
            t,
            &format!("namespaces/{ns}.toml"),
            to_text(&f)?.as_bytes(),
            root,
        )?;
    }
    Ok(())
}

/// What a publish needs beyond the files.
pub struct Publish<'a> {
    pub name: &'a str,
    pub version: &'a str,
    pub kind: &'a str,
    pub title: &'a str,
    pub for_games: &'a [String],
    pub published: &'a str,
}

/// Publishes a package: its blobs, its signed manifest, then its signed
/// version file, last, so a reader never sees a version with parts missing.
pub fn publish(
    t: &dyn Target,
    p: &Publish,
    files: &BTreeMap<String, Vec<u8>>,
    s: &Signer,
) -> Result<String, String> {
    let key = version_key(p.name, p.version)?;
    if p.kind != "game" && p.kind != "content" {
        return Err(format!("{:?} isn't game or content", p.kind));
    }
    if t.read(&key)?.is_some() {
        return Err(format!("{}@{} is already published", p.name, p.version));
    }
    let mut m = Manifest::new(p.kind, p.name);
    for (path, bytes) in files {
        let hash = m.add(path, bytes)?;
        t.write_once(&blob_key(&hash), bytes)?;
    }
    let id = m.id();
    let text = m.write();
    if t.write_once(&package_key(&id), text.as_bytes())? {
        let sig = (s.sign)(text.as_bytes())?;
        t.write_once(&signature_key(&id), &sig)?;
    }
    let v = VersionFile {
        format: FORMAT.into(),
        name: p.name.into(),
        version: p.version.into(),
        id: id.clone(),
        kind: p.kind.into(),
        title: p.title.into(),
        size: m.size(),
        published: p.published.into(),
        signer: s.public.clone(),
        for_games: p.for_games.to_vec(),
    };
    put_signed(t, &key, to_text(&v)?.as_bytes(), s)?;
    Ok(id)
}

/// Rewrites `catalog.toml`, `by-game/` and `yanks.toml` from the signed
/// version files named in `versions` (`name@version`), signed by `s`.
pub fn reindex(
    t: &dyn Target,
    versions: &[(String, String)],
    s: &Signer,
    signed: &str,
    expires: &str,
) -> Result<(), String> {
    let reg = read_registry_unchecked(t)?;
    let mut catalog: BTreeMap<String, CatalogEntry> = BTreeMap::new();
    let mut by_game: BTreeMap<String, Vec<ByGameLine>> = BTreeMap::new();
    for (name, version) in versions {
        let key = version_key(name, version)?;
        let bytes = t
            .read(&key)?
            .ok_or_else(|| format!("{name}@{version} isn't published"))?;
        let v: VersionFile = parse(&key, &bytes)?;
        catalog.insert(
            name.clone(),
            CatalogEntry {
                name: name.clone(),
                kind: v.kind.clone(),
                title: v.title.clone(),
                latest: v.version.clone(),
                id: v.id.clone(),
                for_games: v.for_games.clone(),
            },
        );
        for g in &v.for_games {
            by_game.entry(g.clone()).or_default().push(ByGameLine {
                package: name.clone(),
                version: v.version.clone(),
                id: v.id.clone(),
                title: v.title.clone(),
            });
        }
    }
    let c = Catalog {
        format: FORMAT.into(),
        registry: reg.name.clone(),
        signed: signed.into(),
        expires: expires.into(),
        package: catalog.into_values().collect(),
    };
    replace_signed(t, "catalog.toml", to_text(&c)?.as_bytes(), s)?;
    for (game, line) in by_game {
        let (ns, n) = split_name(&game)?;
        let f = ByGame {
            format: FORMAT.into(),
            registry: reg.name.clone(),
            signed: signed.into(),
            expires: expires.into(),
            line,
        };
        replace_signed(
            t,
            &format!("by-game/{ns}/{n}.toml"),
            to_text(&f)?.as_bytes(),
            s,
        )?;
    }
    let existing: Vec<Yank> = match t.read("yanks.toml")? {
        Some(b) => parse::<Yanks>("yanks.toml", &b)?.yank,
        None => Vec::new(),
    };
    let y = Yanks {
        format: FORMAT.into(),
        registry: reg.name,
        signed: signed.into(),
        expires: expires.into(),
        yank: existing,
    };
    replace_signed(t, "yanks.toml", to_text(&y)?.as_bytes(), s)
}

fn parse<'a, T: Deserialize<'a>>(key: &str, bytes: &'a [u8]) -> Result<T, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| format!("{key} isn't text"))?;
    toml::from_str(text).map_err(|e| format!("{key}: {e}"))
}

fn must_read(t: &dyn Target, key: &str) -> Result<Vec<u8>, String> {
    t.read(key)?.ok_or_else(|| format!("{key} isn't there"))
}

fn read_registry_unchecked(t: &dyn Target) -> Result<RegistryFile, String> {
    parse("registry.toml", &must_read(t, "registry.toml")?)
}

/// Reads a signed key, checked against one of `keys`.
fn checked(t: &dyn Target, key: &str, keys: &[String]) -> Result<Vec<u8>, String> {
    let bytes = must_read(t, key)?;
    let sig = must_read(t, &sig_key(key))?;
    let mut last = String::from("no key may sign it");
    for k in keys {
        match verify(k, NAMESPACE, &sig, &bytes) {
            Ok(()) => return Ok(bytes),
            Err(e) => last = e,
        }
    }
    Err(format!("{key}: {last}"))
}

/// A list file's expiry against now. Both are RFC 3339 UTC, the same width,
/// so they compare as text.
fn fresh(key: &str, expires: &str, now: &str) -> Result<(), String> {
    if expires <= now {
        return Err(format!(
            "{key} expired at {expires}: the source hasn't been refreshed"
        ));
    }
    Ok(())
}

/// A registry read through its pinned root.
pub struct Reader<'a> {
    pub target: &'a dyn Target,
    pub file: RegistryFile,
}

impl<'a> Reader<'a> {
    /// Checks `registry.toml` against the pinned root, by the root's public key.
    pub fn open(target: &'a dyn Target, root: &str) -> Result<Self, String> {
        let bytes = checked(target, "registry.toml", &[root.to_owned()])?;
        let file: RegistryFile = parse("registry.toml", &bytes)?;
        if crate::sig::key_blob(&file.root)? != crate::sig::key_blob(root)? {
            return Err("registry.toml names another root key".into());
        }
        Ok(Self { target, file })
    }

    /// A namespace's file, checked against the root.
    pub fn namespace(&self, ns: &str) -> Result<NamespaceFile, String> {
        let key = format!("namespaces/{ns}.toml");
        let f: NamespaceFile = parse(
            &key,
            &checked(self.target, &key, std::slice::from_ref(&self.file.root))?,
        )?;
        if f.namespace != ns {
            return Err(format!("{key} is the file of {:?}", f.namespace));
        }
        Ok(f)
    }

    /// A version file, checked against its namespace's keys.
    pub fn version(&self, name: &str, version: &str) -> Result<VersionFile, String> {
        let (ns, _) = split_name(name)?;
        let keys: Vec<String> = self
            .namespace(ns)?
            .keys
            .into_iter()
            .map(|k| k.key)
            .collect();
        let key = version_key(name, version)?;
        let v: VersionFile = parse(&key, &checked(self.target, &key, &keys)?)?;
        if v.name != name || v.version != version {
            return Err(format!("{key} describes {}@{}", v.name, v.version));
        }
        Ok(v)
    }

    /// The catalog, checked against the signing keys and its expiry.
    pub fn catalog(&self, now: &str) -> Result<Catalog, String> {
        let c: Catalog = parse(
            "catalog.toml",
            &checked(self.target, "catalog.toml", &self.file.signing)?,
        )?;
        fresh("catalog.toml", &c.expires, now)?;
        Ok(c)
    }

    /// A game's content, checked against the signing keys and its expiry.
    pub fn by_game(&self, game: &str, now: &str) -> Result<ByGame, String> {
        let (ns, n) = split_name(game)?;
        let key = format!("by-game/{ns}/{n}.toml");
        let f: ByGame = parse(&key, &checked(self.target, &key, &self.file.signing)?)?;
        fresh(&key, &f.expires, now)?;
        Ok(f)
    }

    /// The withdrawn versions, checked against the signing keys and expiry.
    pub fn yanks(&self, now: &str) -> Result<Yanks, String> {
        let y: Yanks = parse(
            "yanks.toml",
            &checked(self.target, "yanks.toml", &self.file.signing)?,
        )?;
        fresh("yanks.toml", &y.expires, now)?;
        Ok(y)
    }

    /// A package's manifest by its exact id: the version file must name that
    /// id, and the manifest must hash to it and be signed by the namespace.
    pub fn manifest(&self, name: &str, version: &str, id: &str) -> Result<Manifest, String> {
        let (ns, _) = split_name(name)?;
        let v = self.version(name, version)?;
        if v.id != id {
            return Err(format!(
                "{name}@{version} is {}, not the id asked for, {id}",
                v.id
            ));
        }
        let keys: Vec<String> = self
            .namespace(ns)?
            .keys
            .into_iter()
            .map(|k| k.key)
            .collect();
        let text = must_read(self.target, &package_key(id))?;
        check(id, &text)?;
        let sig = must_read(self.target, &signature_key(id))?;
        let mut signed = Err(String::from("no key may sign it"));
        for k in &keys {
            signed = verify(k, NAMESPACE, &sig, &text);
            if signed.is_ok() {
                break;
            }
        }
        signed.map_err(|e| format!("the manifest of {id}: {e}"))?;
        let m = Manifest::read_checked(id, &text)?;
        if m.name != name {
            return Err(format!(
                "the manifest of {id} is for {}, not {name}",
                m.name
            ));
        }
        Ok(m)
    }

    /// A package by its exact id: its [`Reader::manifest`], and every file
    /// must match that manifest.
    pub fn fetch(
        &self,
        name: &str,
        version: &str,
        id: &str,
    ) -> Result<(Manifest, BTreeMap<String, Vec<u8>>), String> {
        let m = self.manifest(name, version, id)?;
        let mut files = BTreeMap::new();
        for f in &m.files {
            let bytes = must_read(self.target, &blob_key(&f.hash))?;
            files.insert(f.path.clone(), crate::checked_blob(f, bytes)?);
        }
        Ok((m, files))
    }
}

#[cfg(test)]
mod tests;
