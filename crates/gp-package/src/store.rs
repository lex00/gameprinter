//! A store on disk: the key tree every mirror holds ([`crate`]), under one
//! folder. The local store is `$GP_STORE` (beside the evidence store's
//! `objects/`); a mirror on disk, or a folder a static host serves, is the
//! same layout.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::{Manifest, blob_key, check, checked_blob, package_key};

static TMP: AtomicU64 = AtomicU64::new(0);

/// A store in a folder.
#[derive(Debug, Clone)]
pub struct Store {
    pub root: PathBuf,
}

fn io_err(p: &Path, e: io::Error) -> String {
    format!("{}: {e}", p.display())
}

impl Store {
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// A key's file.
    pub fn path(&self, key: &str) -> PathBuf {
        self.root.join(key)
    }

    pub fn has(&self, key: &str) -> bool {
        self.path(key).is_file()
    }

    /// Writes a key once: staged beside it and renamed into place, so a
    /// crash never leaves part of a file under the key. A key that is
    /// there already is left as it is.
    pub fn put(&self, key: &str, bytes: &[u8]) -> Result<bool, String> {
        let p = self.path(key);
        if p.is_file() {
            return Ok(false);
        }
        self.replace(key, bytes)?;
        Ok(true)
    }

    /// Writes a key whether or not it is there: for the one key that
    /// changes, a game's index copy.
    pub fn replace(&self, key: &str, bytes: &[u8]) -> Result<(), String> {
        let p = self.path(key);
        let dir = p.parent().unwrap_or(&self.root);
        fs::create_dir_all(dir).map_err(|e| io_err(dir, e))?;
        let tmp = dir.join(format!(
            ".tmp-{}-{}",
            std::process::id(),
            TMP.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&tmp, bytes).map_err(|e| io_err(&tmp, e))?;
        fs::rename(&tmp, &p).map_err(|e| io_err(&p, e))
    }

    pub fn get(&self, key: &str) -> Result<Vec<u8>, String> {
        let p = self.path(key);
        fs::read(&p).map_err(|e| io_err(&p, e))
    }

    /// Stores a blob and returns its hash.
    pub fn put_blob(&self, bytes: &[u8]) -> Result<String, String> {
        let h = crate::hash(bytes);
        self.put(&blob_key(&h), bytes)?;
        Ok(h)
    }

    /// A blob by its hash, checked.
    pub fn blob(&self, hash: &str) -> Result<Vec<u8>, String> {
        let b = self.get(&blob_key(hash))?;
        check(hash, &b).map_err(|e| format!("blob {hash}: {e}"))?;
        Ok(b)
    }

    /// Stores a package: each file's blob, then the manifest last, so a
    /// manifest in the store always has its files. Returns the id.
    pub fn put_package(
        &self,
        m: &Manifest,
        files: &BTreeMap<String, Vec<u8>>,
    ) -> Result<String, String> {
        for f in &m.files {
            let b = files
                .get(&f.path)
                .ok_or_else(|| format!("{}: no bytes for it", f.path))?;
            check(&f.hash, b).map_err(|e| format!("{}: {e}", f.path))?;
            self.put(&blob_key(&f.hash), b)?;
        }
        let id = m.id();
        self.put(&package_key(&id), m.write().as_bytes())?;
        Ok(id)
    }

    /// A package's manifest by its id, checked.
    pub fn package(&self, id: &str) -> Result<Manifest, String> {
        Manifest::read_checked(id, &self.get(&package_key(id))?)
    }

    /// A package's files, each checked against the manifest.
    pub fn files(&self, m: &Manifest) -> Result<BTreeMap<String, Vec<u8>>, String> {
        let mut out = BTreeMap::new();
        for f in &m.files {
            let b = self.get(&blob_key(&f.hash))?;
            out.insert(f.path.clone(), checked_blob(f, b)?);
        }
        Ok(out)
    }

    /// Writes a package's files under a folder, each checked first.
    pub fn unpack(&self, id: &str, out: &Path) -> Result<Manifest, String> {
        let m = self.package(id)?;
        for (path, bytes) in self.files(&m)? {
            let p = out.join(&path);
            if let Some(d) = p.parent() {
                fs::create_dir_all(d).map_err(|e| io_err(d, e))?;
            }
            fs::write(&p, bytes).map_err(|e| io_err(&p, e))?;
        }
        Ok(m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_package_round_trips_through_a_store_and_a_bad_blob_is_refused() {
        let d = tempfile::tempdir().unwrap();
        let s = Store::at(d.path());
        let mut m = Manifest::new("theme", "night");
        let mut files = BTreeMap::new();
        for (p, b) in [
            ("theme/theme.toml", b"id = \"night\"\n".to_vec()),
            ("theme/a.pix", b"pix".to_vec()),
            // The same bytes twice are one blob.
            ("look/a.png", b"pix".to_vec()),
        ] {
            m.add(p, &b).unwrap();
            files.insert(p.to_owned(), b);
        }
        let id = s.put_package(&m, &files).unwrap();
        assert_eq!(id, m.id());
        assert_eq!(fs::read_dir(d.path().join("blobs")).unwrap().count(), 2);
        assert_eq!(s.package(&id).unwrap(), m);
        let out = d.path().join("out");
        s.unpack(&id, &out).unwrap();
        assert_eq!(fs::read(out.join("look/a.png")).unwrap(), b"pix");
        // Written once: the key is kept.
        assert!(!s.put(&package_key(&id), b"other").unwrap());
        // A blob changed on disk is refused by its hash.
        let h = &m.get("theme/a.pix").unwrap().hash;
        fs::write(s.path(&blob_key(h)), b"pox").unwrap();
        assert!(s.blob(h).unwrap_err().contains("hash mismatch"));
        assert!(s.unpack(&id, &out).is_err());
        // A manifest changed on disk is refused by its id.
        fs::write(
            s.path(&package_key(&id)),
            m.write().replace("night", "nite"),
        )
        .unwrap();
        assert!(s.package(&id).unwrap_err().contains("hash mismatch"));
    }
}
