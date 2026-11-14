//! Compiled modules kept on disk, so a gp call that opens a game's module
//! doesn't compile it again: compiling Donkey Kong's or Tapper's module
//! takes 0.4 to 0.7 s, a large share of scoring a short clip.
//!
//! An entry is keyed by the module's bytes and by everything wasmtime says
//! decides whether compiled code fits an engine (its full version, the
//! target and its CPU features, and the engine's settings:
//! `Engine::precompile_compatibility_hash`), hashed with BLAKE3. Its file
//! holds a magic line, that key, the BLAKE3 hash of the compiled code and
//! the code as `Module::serialize` wrote it. A file that is short, of
//! another key, of another hash or that wasmtime refuses is not used: the
//! module is compiled and the entry written again.
//!
//! The folder is one for the machine, like the tapes': `$GP_MODULE_CACHE_DIR`,
//! else `~/.gameprinter/work/compiled` from `$HOME`. Entries are written
//! whole through a temporary file and a rename, so several processes can
//! write the same entry at once, and the folder is held to a size cap
//! (`$GP_MODULE_CACHE_MB` megabytes, else 2 GB), least recently used first.

// The workspace forbids unsafe code; this crate denies it, and this one
// file allows it for `Module::deserialize` alone.
#![allow(unsafe_code)]

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};

use wasmtime::{Engine, Module};

const MAGIC: &[u8] = b"gp compiled module 1\n";

/// How long a temporary file may sit before the cap takes it as left by a
/// writer that died.
const STALE: Duration = Duration::from_secs(3600);

/// Where compiled modules are kept: `$GP_MODULE_CACHE_DIR`, else
/// `~/.gameprinter/work/compiled` from `$HOME`. `None` without either: the
/// module is compiled every time.
pub(crate) fn dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("GP_MODULE_CACHE_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(d));
    }
    let home = std::env::var_os("HOME").filter(|d| !d.is_empty())?;
    Some(
        PathBuf::from(home)
            .join(".gameprinter")
            .join("work")
            .join("compiled"),
    )
}

/// The folder's size cap: `$GP_MODULE_CACHE_MB` megabytes, else 2 GB.
/// Donkey Kong's compiled module is about 14 MB.
fn cap_bytes() -> u64 {
    std::env::var("GP_MODULE_CACHE_MB")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(2048)
        * 1024
        * 1024
}

/// A [`Hasher`] that feeds BLAKE3, for wasmtime's compatibility hash.
struct Blake(blake3::Hasher);

impl Hasher for Blake {
    fn write(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }

    fn finish(&self) -> u64 {
        let h = self.0.finalize();
        u64::from_le_bytes(*h.as_bytes().first_chunk().expect("32 bytes"))
    }
}

/// The entry's key: the format, the engine's compatibility and the
/// module's bytes.
pub(crate) fn key(engine: &Engine, wasm: &[u8]) -> [u8; 32] {
    let mut h = Blake(blake3::Hasher::new());
    h.0.update(MAGIC);
    engine.precompile_compatibility_hash().hash(&mut h);
    h.0.update(&(wasm.len() as u64).to_le_bytes());
    h.0.update(wasm);
    *h.0.finalize().as_bytes()
}

/// The entry's file in `dir`.
pub(crate) fn file(dir: &Path, key: &[u8; 32]) -> PathBuf {
    dir.join(format!("{}.cwasm", blake3::Hash::from_bytes(*key).to_hex()))
}

/// The compiled module in `file`, if it is there, whole and of `key`; it
/// is then marked used, so the cap removes it last.
pub(crate) fn read(engine: &Engine, file: &Path, key: &[u8; 32]) -> Option<Module> {
    let bytes = std::fs::read(file).ok()?;
    let rest = bytes.strip_prefix(MAGIC)?;
    let (k, rest) = rest.split_first_chunk::<32>()?;
    let (sum, code) = rest.split_first_chunk::<32>()?;
    if k != key || blake3::hash(code).as_bytes() != sum {
        return None;
    }
    // SAFETY: `Module::deserialize` trusts its bytes to be what
    // `Module::serialize` wrote, unchanged. They are here: the file is in
    // the user's own folder, written only by `write` below (whole, through
    // a rename); its key is the hash of this module's bytes and of this
    // engine's compatibility, and the code's BLAKE3 hash matches the one
    // written beside it, so a short, damaged or mixed-up file never gets
    // here. Anyone who could write a file that passes these checks could
    // as well replace gp itself. Code from another wasmtime version or
    // other settings has another key, and wasmtime refuses it besides.
    let module = unsafe { Module::deserialize(engine, code) }.ok()?;
    touch(file);
    Some(module)
}

/// Writes `module` as the entry `key` in `dir`, whole or not at all, then
/// holds the folder to its cap. An entry that can't be written only means
/// the next call compiles again.
pub(crate) fn write(dir: &Path, key: &[u8; 32], module: &Module) {
    static WRITES: AtomicUsize = AtomicUsize::new(0);
    let Ok(code) = module.serialize() else { return };
    let mut bytes = Vec::with_capacity(MAGIC.len() + 64 + code.len());
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(key);
    bytes.extend_from_slice(blake3::hash(&code).as_bytes());
    bytes.extend_from_slice(&code);
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let file = file(dir, key);
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = dir.join(format!(
        ".{name}.tmp-{}-{}",
        std::process::id(),
        WRITES.fetch_add(1, Ordering::Relaxed)
    ));
    if std::fs::write(&tmp, &bytes).is_ok() && std::fs::rename(&tmp, &file).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    evict(dir, cap_bytes());
}

/// Marks a file used now, for the cap's least recently used order.
fn touch(file: &Path) {
    if let Ok(f) = std::fs::File::options().append(true).open(file) {
        let _ = f.set_modified(SystemTime::now());
    }
}

/// Removes the least recently used entries in `dir` until it is under
/// `cap` bytes. A temporary file is left to its writer, unless it is older
/// than [`STALE`].
pub(crate) fn evict(dir: &Path, cap: u64) {
    let now = SystemTime::now();
    let mut files: Vec<(SystemTime, u64, PathBuf)> = Vec::new();
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let Ok(m) = e.metadata() else { continue };
        if !m.is_file() {
            continue;
        }
        let at = m.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        if e.file_name().to_string_lossy().starts_with('.') {
            if now.duration_since(at).is_ok_and(|d| d > STALE) {
                let _ = std::fs::remove_file(e.path());
            }
            continue;
        }
        files.push((at, m.len(), e.path()));
    }
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    if total <= cap {
        return;
    }
    files.sort();
    for (_, len, p) in files {
        if total <= cap {
            break;
        }
        if std::fs::remove_file(&p).is_ok() {
            total -= len;
        }
    }
}
