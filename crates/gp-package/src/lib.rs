//! Packages (docs/packages.md): the one format for a model version and a
//! theme version. A package is its files, each stored once as a blob named
//! by the BLAKE3 hash of its bytes, and a manifest naming them by path,
//! hash and size. The package id is the BLAKE3 hash of the manifest's
//! bytes, so one id pins every byte of every file.
//!
//! A local store and every mirror hold the same tree of keys, so a mirror
//! is a copy of the local store's files and any static host serves it:
//!
//! | Key | What |
//! |---|---|
//! | `blobs/<hash>` | A file's bytes, as they are |
//! | `packages/<id>` | A manifest |
//! | `packages/<id>.sig` | The manifest's SSH signature, when it was signed |
//! | `index/themes/<game>.toml` | A copy of a game's themes index ([`index`]) |
//!
//! Keys are written once: a key that is there already holds the same
//! bytes, since its name is their hash.
//!
//! This crate depends on blake3 alone, so the runtimes (gp-web and the
//! generated Bevy games, through gp-present) read packages with the code gp
//! writes them with.

pub mod index;
pub mod registry;
pub mod sig;
mod store;

pub use store::Store;

/// The manifest's first line.
pub const HEADER: &str = "gp-package 1";

/// The BLAKE3 hash of some bytes, as 64 lowercase hex digits.
pub fn hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

/// Whether a string is a hash as packages write them.
pub fn is_hash(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Checks bytes against the hash they were named by.
pub fn check(want: &str, bytes: &[u8]) -> Result<(), String> {
    let got = hash(bytes);
    if got == want {
        Ok(())
    } else {
        Err(format!(
            "hash mismatch: wanted {want}, the bytes hash to {got}"
        ))
    }
}

/// A blob's key in a store or mirror.
pub fn blob_key(hash: &str) -> String {
    format!("blobs/{hash}")
}

/// A manifest's key.
pub fn package_key(id: &str) -> String {
    format!("packages/{id}")
}

/// A manifest's signature's key.
pub fn signature_key(id: &str) -> String {
    format!("packages/{id}.sig")
}

/// A game's themes index's key on a mirror.
pub fn themes_index_key(game: &str) -> String {
    format!("index/themes/{game}.toml")
}

/// A file a manifest names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct File {
    /// Relative, with `/` between parts; never absolute and never with
    /// `.`, `..` or an empty part.
    pub path: String,
    pub hash: String,
    pub size: u64,
}

/// A package's manifest: what kind of package it is (`model`, `theme`),
/// its name, and its files sorted by path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub kind: String,
    pub name: String,
    pub files: Vec<File>,
}

/// Whether a path is one a manifest may hold.
pub fn valid_path(p: &str) -> bool {
    !p.is_empty()
        && !p.starts_with('/')
        && !p.contains('\\')
        && !p.chars().any(|c| c.is_control())
        && p.split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

fn valid_word(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

impl Manifest {
    pub fn new(kind: &str, name: &str) -> Self {
        Self {
            kind: kind.to_owned(),
            name: name.to_owned(),
            files: Vec::new(),
        }
    }

    /// Adds a file by its bytes and returns its hash. A path given twice
    /// is refused.
    pub fn add(&mut self, path: &str, bytes: &[u8]) -> Result<String, String> {
        if !valid_path(path) {
            return Err(format!("`{path}` isn't a path a package may hold"));
        }
        let at = match self.files.binary_search_by(|f| f.path.as_str().cmp(path)) {
            Ok(_) => return Err(format!("`{path}` is in the package twice")),
            Err(at) => at,
        };
        let h = hash(bytes);
        self.files.insert(
            at,
            File {
                path: path.to_owned(),
                hash: h.clone(),
                size: bytes.len() as u64,
            },
        );
        Ok(h)
    }

    /// A file by its path.
    pub fn get(&self, path: &str) -> Option<&File> {
        self.files.iter().find(|f| f.path == path)
    }

    /// The files under a folder (`look`), with the folder taken off their
    /// paths.
    pub fn under<'a>(&'a self, folder: &'a str) -> impl Iterator<Item = (&'a str, &'a File)> + 'a {
        self.files.iter().filter_map(move |f| {
            let rest = f.path.strip_prefix(folder)?.strip_prefix('/')?;
            Some((rest, f))
        })
    }

    /// The manifest's text: the header, `kind`, `name`, then a line a file,
    /// `file HASH SIZE PATH`, in path order. These bytes are what the id
    /// hashes, so the same files always give the same id.
    pub fn write(&self) -> String {
        let mut s = format!("{HEADER}\nkind {}\nname {}\n", self.kind, self.name);
        for f in &self.files {
            s.push_str(&format!("file {} {} {}\n", f.hash, f.size, f.path));
        }
        s
    }

    /// The package id: the hash of the manifest's text.
    pub fn id(&self) -> String {
        hash(self.write().as_bytes())
    }

    /// Reads a manifest. It must be exactly what [`Manifest::write`]
    /// writes for it (files sorted, no path twice, nothing else), so a
    /// manifest has one text and one id.
    pub fn read(text: &str) -> Result<Self, String> {
        let mut lines = text.split('\n');
        if lines.next() != Some(HEADER) {
            return Err(format!("not a package manifest (no `{HEADER}` line)"));
        }
        let kind = lines
            .next()
            .and_then(|l| l.strip_prefix("kind "))
            .ok_or("no `kind` line")?;
        let name = lines
            .next()
            .and_then(|l| l.strip_prefix("name "))
            .ok_or("no `name` line")?;
        let good_name = match name.split_once('/') {
            Some((ns, n)) => valid_word(ns) && valid_word(n),
            None => valid_word(name),
        };
        if !valid_word(kind) || !good_name {
            return Err(format!("`{kind}` `{name}` isn't a kind and a name"));
        }
        let mut m = Manifest::new(kind, name);
        for line in lines {
            if line.is_empty() {
                continue;
            }
            let rest = line
                .strip_prefix("file ")
                .ok_or_else(|| format!("`{line}` isn't a file line"))?;
            let mut it = rest.splitn(3, ' ');
            let (h, size, path) = (
                it.next().unwrap_or(""),
                it.next().unwrap_or(""),
                it.next().unwrap_or(""),
            );
            if !is_hash(h) {
                return Err(format!("`{h}` isn't a hash"));
            }
            let size: u64 = size.parse().map_err(|_| format!("`{size}` isn't a size"))?;
            if !valid_path(path) {
                return Err(format!("`{path}` isn't a path a package may hold"));
            }
            if m.files.last().is_some_and(|l| l.path.as_str() >= path) {
                return Err(format!("`{path}` is out of order or there twice"));
            }
            m.files.push(File {
                path: path.to_owned(),
                hash: h.to_owned(),
                size,
            });
        }
        if m.write() != text {
            return Err("the manifest isn't written as gp writes one".into());
        }
        Ok(m)
    }

    /// Reads a manifest fetched by its id, checking the bytes hash to it.
    pub fn read_checked(id: &str, bytes: &[u8]) -> Result<Self, String> {
        check(id, bytes).map_err(|e| format!("package {id}: {e}"))?;
        let text = std::str::from_utf8(bytes).map_err(|_| format!("package {id}: not text"))?;
        Self::read(text)
    }

    /// The total size of its files.
    pub fn size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }
}

/// A file's bytes, fetched by its hash, checked against it.
pub fn checked_blob(f: &File, bytes: Vec<u8>) -> Result<Vec<u8>, String> {
    if bytes.len() as u64 != f.size {
        return Err(format!(
            "{}: {} bytes where the manifest says {}",
            f.path,
            bytes.len(),
            f.size
        ));
    }
    check(&f.hash, &bytes).map_err(|e| format!("{}: {e}", f.path))?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_manifest_round_trips_and_its_id_is_its_hash() {
        let mut m = Manifest::new("theme", "night");
        m.add("theme/theme.toml", b"format = \"0.1.3\"\n").unwrap();
        m.add("look/look.txt", b"gp-look 1\nid night\n").unwrap();
        m.add("checks.json", b"{}").unwrap();
        assert_eq!(
            m.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(),
            ["checks.json", "look/look.txt", "theme/theme.toml"]
        );
        let text = m.write();
        let back = Manifest::read(&text).unwrap();
        assert_eq!(back, m);
        assert_eq!(m.id(), hash(text.as_bytes()));
        assert_eq!(
            back.under("look").map(|(p, _)| p).collect::<Vec<_>>(),
            ["look.txt"]
        );
        // The same files in another order give the same id.
        let mut n = Manifest::new("theme", "night");
        n.add("checks.json", b"{}").unwrap();
        n.add("theme/theme.toml", b"format = \"0.1.3\"\n").unwrap();
        n.add("look/look.txt", b"gp-look 1\nid night\n").unwrap();
        assert_eq!(n.id(), m.id());
        assert!(Manifest::read_checked(&m.id(), text.as_bytes()).is_ok());
        assert!(Manifest::read_checked(&m.id(), text.replace("night", "nite").as_bytes()).is_err());
    }

    #[test]
    fn bad_paths_and_texts_are_refused() {
        let mut m = Manifest::new("theme", "x");
        for p in ["", "/abs", "a/../b", "a//b", "./a", "a\\b", "a\nb"] {
            assert!(m.add(p, b"").is_err(), "{p:?}");
        }
        m.add("a", b"1").unwrap();
        assert!(m.add("a", b"2").is_err());
        let good = m.write();
        assert!(Manifest::read(&good).is_ok());
        // Out of order, a stray line, a short hash, extra spacing.
        let mut two = Manifest::new("theme", "x");
        two.add("a", b"1").unwrap();
        two.add("b", b"2").unwrap();
        let t = two.write();
        let lines: Vec<&str> = t.lines().collect();
        let swapped = format!(
            "{}\n{}\n{}\n{}\n{}\n",
            lines[0], lines[1], lines[2], lines[4], lines[3]
        );
        assert!(Manifest::read(&swapped).is_err());
        assert!(Manifest::read(&format!("{good}junk\n")).is_err());
        assert!(Manifest::read(&good.replace(&m.files[0].hash, "abc")).is_err());
        assert!(Manifest::read(&good.replace("kind theme", "kind  theme")).is_err());
        assert!(Manifest::read("nope").is_err());
    }

    #[test]
    fn a_blob_is_checked_by_size_and_hash() {
        let mut m = Manifest::new("theme", "x");
        m.add("a.png", b"pixels").unwrap();
        let f = &m.files[0];
        assert!(checked_blob(f, b"pixels".to_vec()).is_ok());
        assert!(
            checked_blob(f, b"pixelz".to_vec())
                .unwrap_err()
                .contains("hash mismatch")
        );
        assert!(checked_blob(f, b"pix".to_vec()).is_err());
    }
}
