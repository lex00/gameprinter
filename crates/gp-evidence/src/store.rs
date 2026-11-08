//! The content-addressed evidence store: a directory of immutable objects,
//! each named by the BLAKE3 hash of its contents.
//!
//! Layout: `objects/<first 2 hex chars>/<remaining 62>`, with writes staged in
//! `tmp/` and renamed into place so a crash never leaves a partial object.
//!
//! `lock` is an empty file for an advisory lock. A command takes it shared
//! before its first write (an object or a recordings log line) and keeps it
//! until it exits; `gp store gc` takes it exclusively, so it never deletes
//! while a recording is half written.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;

use crate::hash::Hash;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("store I/O at {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("object {0} is not in the store")]
    Missing(Hash),
    #[error("no evidence store at {0}; check $GP_STORE")]
    NoStore(PathBuf),
    #[error("object {expected} is corrupt: its contents hash to {actual}")]
    Corrupt { expected: Hash, actual: Hash },
}

fn io_at(path: &Path) -> impl FnOnce(io::Error) -> StoreError + '_ {
    move |source| StoreError::Io {
        path: path.to_path_buf(),
        source,
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct StoreStats {
    pub objects: u64,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct VerifyReport {
    pub checked: u64,
    pub corrupt: Vec<Hash>,
}

pub struct Store {
    root: PathBuf,
    lock: Mutex<Lock>,
}

/// What this process holds of the store's lock file.
enum Lock {
    None,
    /// Held while writing; other writers hold it too.
    Shared(#[allow(dead_code)] File),
    /// Held by `gp store gc`; nothing else holds it.
    Exclusive(#[allow(dead_code)] File),
}

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

impl Store {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, StoreError> {
        let root = root.into();
        for dir in [root.join("objects"), root.join("tmp")] {
            fs::create_dir_all(&dir).map_err(io_at(&dir))?;
        }
        Ok(Self::at(root))
    }

    /// Opens a store that must already exist, creating and writing nothing.
    /// For checks that only read: a mistyped path is an error, not a new,
    /// empty store that checks nothing.
    pub fn open_existing(root: impl Into<PathBuf>) -> Result<Self, StoreError> {
        let root = root.into();
        // `just setup` makes the folder; objects/ appears with the first object.
        if !root.is_dir() {
            return Err(StoreError::NoStore(root));
        }
        Ok(Self::at(root))
    }

    fn at(root: PathBuf) -> Self {
        Self {
            root,
            lock: Mutex::new(Lock::None),
        }
    }

    fn lock_file(&self) -> Result<File, StoreError> {
        let path = self.root.join("lock");
        fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(io_at(&path))
    }

    /// Takes the store's lock shared, once for this `Store`, waiting while
    /// `gp store gc` holds it. Every write calls it, so a command holds the
    /// lock from its first write until it exits. Where the platform has no
    /// file locks (WASI), it does nothing.
    pub fn lock_for_writing(&self) -> Result<(), StoreError> {
        let mut held = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        if !matches!(*held, Lock::None) {
            return Ok(());
        }
        let file = self.lock_file()?;
        let locked = match file.try_lock_shared() {
            Ok(()) => Ok(()),
            Err(fs::TryLockError::WouldBlock) => {
                eprintln!(
                    "gp: waiting for `gp store gc` to finish with {}",
                    self.root.display()
                );
                file.lock_shared()
            }
            Err(fs::TryLockError::Error(e)) => Err(e),
        };
        match locked {
            Ok(()) => *held = Lock::Shared(file),
            Err(e) if e.kind() == io::ErrorKind::Unsupported => {}
            Err(e) => return Err(io_at(&self.root.join("lock"))(e)),
        }
        Ok(())
    }

    /// Takes the store's lock exclusively without waiting, for `gp store
    /// gc`. False if another process holds it, which means a command that
    /// writes, such as `gp record`, is running. Writes through this `Store`
    /// still work while it's held.
    pub fn try_lock_exclusive(&self) -> Result<bool, StoreError> {
        let mut held = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        if matches!(*held, Lock::Exclusive(_)) {
            return Ok(true);
        }
        let file = self.lock_file()?;
        // A shared lock held by this `Store` is a different open file, so
        // it would block the exclusive one: let it go first.
        *held = Lock::None;
        match file.try_lock() {
            Ok(()) => {
                *held = Lock::Exclusive(file);
                Ok(true)
            }
            Err(fs::TryLockError::WouldBlock) => Ok(false),
            Err(fs::TryLockError::Error(e)) if e.kind() == io::ErrorKind::Unsupported => Ok(true),
            Err(fs::TryLockError::Error(e)) => Err(io_at(&self.root.join("lock"))(e)),
        }
    }

    /// Deletes an object, for `gp store gc`, which holds the exclusive lock.
    /// Returns its size, or `None` if it was already gone.
    pub fn delete(&self, hash: &Hash) -> Result<Option<u64>, StoreError> {
        let path = self.path_of(hash);
        let bytes = match fs::metadata(&path) {
            Ok(m) => m.len(),
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(io_at(&path)(e)),
        };
        fs::remove_file(&path).map_err(io_at(&path))?;
        Ok(Some(bytes))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn path_of(&self, hash: &Hash) -> PathBuf {
        let hex = hash.to_hex();
        self.root.join("objects").join(&hex[..2]).join(&hex[2..])
    }

    pub fn contains(&self, hash: &Hash) -> bool {
        self.path_of(hash).is_file()
    }

    pub fn put_bytes(&self, data: &[u8]) -> Result<Hash, StoreError> {
        self.put_reader(data).map(|(hash, _)| hash)
    }

    /// Streams `reader` into the store, hashing as it goes. Returns the hash
    /// and the number of bytes written. Storing an existing object is a no-op.
    pub fn put_reader(&self, mut reader: impl Read) -> Result<(Hash, u64), StoreError> {
        self.lock_for_writing()?;
        let tmp = self.root.join("tmp").join(format!(
            "{}-{}",
            std::process::id(),
            TMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = File::create(&tmp).map_err(io_at(&tmp))?;
        let mut hasher = blake3::Hasher::new();
        let mut buf = vec![0u8; 64 * 1024];
        let mut len = 0u64;
        loop {
            let n = reader.read(&mut buf).map_err(io_at(&tmp))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            file.write_all(&buf[..n]).map_err(io_at(&tmp))?;
            len += n as u64;
        }
        file.sync_all().map_err(io_at(&tmp))?;
        drop(file);

        let hash = Hash::from_blake3(hasher.finalize());
        let dest = self.path_of(&hash);
        if dest.is_file() {
            fs::remove_file(&tmp).map_err(io_at(&tmp))?;
        } else {
            let parent = dest.parent().expect("object paths have a parent");
            fs::create_dir_all(parent).map_err(io_at(parent))?;
            fs::rename(&tmp, &dest).map_err(io_at(&dest))?;
        }
        Ok((hash, len))
    }

    /// Reads an object and checks that its contents still match its name.
    pub fn get(&self, hash: &Hash) -> Result<Vec<u8>, StoreError> {
        let path = self.path_of(hash);
        let data = match fs::read(&path) {
            Ok(d) => d,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Err(StoreError::Missing(*hash));
            }
            Err(e) => return Err(io_at(&path)(e)),
        };
        let actual = Hash::of(&data);
        if actual != *hash {
            return Err(StoreError::Corrupt {
                expected: *hash,
                actual,
            });
        }
        Ok(data)
    }

    pub fn hashes(&self) -> Result<Vec<Hash>, StoreError> {
        let objects = self.root.join("objects");
        let mut out = Vec::new();
        for shard in fs::read_dir(&objects).map_err(io_at(&objects))? {
            let shard = shard.map_err(io_at(&objects))?.path();
            if !shard.is_dir() {
                continue;
            }
            let prefix = shard
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_owned();
            for entry in fs::read_dir(&shard).map_err(io_at(&shard))? {
                let entry = entry.map_err(io_at(&shard))?;
                let name = entry.file_name();
                if let Ok(hash) = format!("{prefix}{}", name.to_string_lossy()).parse() {
                    out.push(hash);
                }
            }
        }
        out.sort();
        Ok(out)
    }

    pub fn stats(&self) -> Result<StoreStats, StoreError> {
        let mut stats = StoreStats {
            objects: 0,
            bytes: 0,
        };
        for hash in self.hashes()? {
            let path = self.path_of(&hash);
            let meta = fs::metadata(&path).map_err(io_at(&path))?;
            stats.objects += 1;
            stats.bytes += meta.len();
        }
        Ok(stats)
    }

    pub fn verify_all(&self) -> Result<VerifyReport, StoreError> {
        let mut report = VerifyReport {
            checked: 0,
            corrupt: Vec::new(),
        };
        for hash in self.hashes()? {
            report.checked += 1;
            match self.get(&hash) {
                Ok(_) => {}
                Err(StoreError::Corrupt { .. }) => report.corrupt.push(hash),
                Err(e) => return Err(e),
            }
        }
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, Store) {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path().join("store")).unwrap();
        (tmp, store)
    }

    #[test]
    fn put_then_get() {
        let (_tmp, s) = store();
        let h = s.put_bytes(b"frame trace chunk").unwrap();
        assert_eq!(h, Hash::of(b"frame trace chunk"));
        assert_eq!(s.get(&h).unwrap(), b"frame trace chunk");
        assert!(s.contains(&h));
    }

    #[test]
    fn storing_twice_keeps_one_object() {
        let (_tmp, s) = store();
        s.put_bytes(b"same").unwrap();
        s.put_bytes(b"same").unwrap();
        assert_eq!(
            s.stats().unwrap(),
            StoreStats {
                objects: 1,
                bytes: 4
            }
        );
        assert_eq!(fs::read_dir(s.root().join("tmp")).unwrap().count(), 0);
    }

    #[test]
    fn missing_object() {
        let (_tmp, s) = store();
        let h = Hash::of(b"never stored");
        assert!(matches!(s.get(&h), Err(StoreError::Missing(_))));
    }

    #[test]
    fn detects_corruption() {
        let (_tmp, s) = store();
        let h = s.put_bytes(b"original").unwrap();
        fs::write(s.path_of(&h), b"tampered").unwrap();
        assert!(matches!(s.get(&h), Err(StoreError::Corrupt { .. })));
        let report = s.verify_all().unwrap();
        assert_eq!(report.checked, 1);
        assert_eq!(report.corrupt, vec![h]);
    }

    #[test]
    fn a_writer_keeps_gc_out() {
        let (_tmp, writer) = store();
        let gc = Store::open_existing(writer.root()).unwrap();
        assert!(gc.try_lock_exclusive().unwrap(), "no writer yet");
        // Exclusive here, so this `Store` may still write.
        let h = gc.put_bytes(b"x").unwrap();
        assert_eq!(gc.delete(&h).unwrap(), Some(1));
        assert_eq!(gc.delete(&h).unwrap(), None);
        drop(gc);
        writer.put_bytes(b"y").unwrap();
        let gc = Store::open_existing(writer.root()).unwrap();
        assert!(!gc.try_lock_exclusive().unwrap(), "a writer holds the lock");
        drop(writer);
        assert!(gc.try_lock_exclusive().unwrap());
    }

    #[test]
    fn streams_large_input() {
        let (_tmp, s) = store();
        let data = vec![7u8; 300_000];
        let (h, len) = s.put_reader(&data[..]).unwrap();
        assert_eq!(len, 300_000);
        assert_eq!(h, Hash::of(&data));
    }
}
